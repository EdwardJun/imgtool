//! 智能裁剪的文件级任务：识别（或使用给定的框）→ 逐个渲染为正方形 → 写入 `<原名>_icons/`

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::decode::decode;
use crate::detect::{detect_regions, render_square, DetectOptions, Region, SquareOptions};
use crate::encode::{encode, EncodeParams, Pixels};
use crate::error::{Error, Result};
use crate::project::output::{resolve_conflict, write_atomic};
use crate::project::preset::{ConflictPolicy, OutputPolicy};
use crate::types::OutputFormat;

#[derive(Debug, Clone)]
pub struct SplitJob {
    pub input: PathBuf,
    /// 为空时自动识别
    pub regions: Option<Vec<Region>>,
    pub detect: DetectOptions,
    pub square: SquareOptions,
    /// 按顺序给每个图标命名（不含扩展名），不足的用 icon_01…
    pub names: Vec<String>,
    /// 默认 `<原图目录>/<原名>_icons`
    pub out_dir: Option<PathBuf>,
    pub format: OutputFormat,
    pub on_conflict: ConflictPolicy,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitFile {
    pub path: String,
    pub name: String,
    pub region: Region,
    pub bytes: u64,
    pub written: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitResult {
    pub input: String,
    pub out_dir: String,
    pub size: u32,
    /// "alpha" / "color" / "manual"
    pub mode: String,
    pub files: Vec<SplitFile>,
}

/// 自动识别时允许导出的最多图标数
pub const MAX_AUTO_REGIONS: usize = 100;

pub fn default_out_dir(input: &Path) -> PathBuf {
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
    input.parent().unwrap_or_else(|| Path::new(".")).join(format!("{stem}_icons"))
}

/// 文件名只保留字母数字、中文、- 和 _
pub fn sanitize_name(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let s = s.trim_matches('_').chars().take(60).collect::<String>();
    if s.is_empty() {
        "icon".into()
    } else {
        s
    }
}

pub fn split_file(job: &SplitJob) -> Result<SplitResult> {
    let data = std::fs::read(&job.input)?;
    let img = decode(&data)?.image;
    let (regions, mode, detected_bg) = match &job.regions {
        Some(r) => (r.clone(), "manual".to_string(), None),
        None => {
            let d = detect_regions(&img, &job.detect);
            // 照片、纹理类图片会识别出大量碎块，自动模式下拒绝，避免一次写出成百上千个文件
            if d.regions.len() > MAX_AUTO_REGIONS {
                return Err(Error::InvalidArgument(format!(
                    "识别到 {} 个以上区域，这张图可能不是图标拼图（自动模式最多 {MAX_AUTO_REGIONS} 个）；可调大合并间距/最小尺寸，或手动指定框",
                    d.regions.len()
                )));
            }
            (d.regions, d.mode, d.background)
        }
    };
    if regions.is_empty() {
        return Err(Error::InvalidArgument("没有识别到图标，可调整合并间距、容差或背景色后重试".into()));
    }
    for r in &regions {
        if r.width == 0 || r.height == 0 || r.x >= img.width() || r.y >= img.height() {
            return Err(Error::InvalidArgument(format!("裁剪框超出图片范围: {r:?}")));
        }
    }
    // 按背景色识别的图（如白底）用同一背景色填充画布，避免出现"透明底上的白块"
    let mut square = job.square.clone();
    if square.background.is_none() && (detected_bg.is_some() || !job.format.supports_alpha()) {
        square.background = Some(detected_bg.unwrap_or([255, 255, 255]));
    }

    let out_dir = job.out_dir.clone().unwrap_or_else(|| default_out_dir(&job.input));
    let policy = OutputPolicy::default();
    let mut files = Vec::with_capacity(regions.len());
    let mut used = std::collections::HashSet::new();
    for (i, region) in regions.iter().enumerate() {
        let mut name = job.names.get(i).map(|n| sanitize_name(n)).unwrap_or_else(|| format!("icon_{:02}", i + 1));
        // 同一批里重名时追加序号
        if !used.insert(name.clone()) {
            name = format!("{name}_{:02}", i + 1);
            used.insert(name.clone());
        }
        let tile = render_square(&img, region, &square);
        let px = Pixels::from_image(&tile);
        let quality = match job.format {
            OutputFormat::Png => None,
            _ => Some(90),
        };
        let bytes = encode(&px, job.format, EncodeParams { quality, fast: false, background: square.background.unwrap_or([255, 255, 255]) })?;
        let planned = out_dir.join(format!("{name}.{}", job.format.extension()));
        let (path, written) = if job.dry_run {
            (planned, false)
        } else {
            let target = resolve_conflict(planned, &job.input, &policy, job.on_conflict)?;
            write_atomic(&target, &bytes)?;
            (target, true)
        };
        files.push(SplitFile { path: path.display().to_string(), name, region: *region, bytes: bytes.len() as u64, written });
    }
    Ok(SplitResult { input: job.input.display().to_string(), out_dir: out_dir.display().to_string(), size: square.size, mode, files })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba, RgbaImage};

    /// 透明底上 3 个实心方块
    fn write_sheet(path: &Path, bg: Rgba<u8>) {
        let mut img = RgbaImage::from_pixel(400, 150, bg);
        for i in 0..3u32 {
            for y in 40..100 {
                for x in (40 + i * 120)..(100 + i * 120) {
                    img.put_pixel(x, y, Rgba([200, 30, 30, 255]));
                }
            }
        }
        DynamicImage::ImageRgba8(img).save(path).unwrap();
    }

    fn job(input: PathBuf) -> SplitJob {
        SplitJob {
            input,
            regions: None,
            detect: DetectOptions::default(),
            square: SquareOptions { size: 128, ..Default::default() },
            names: vec![],
            out_dir: None,
            format: OutputFormat::Png,
            on_conflict: ConflictPolicy::Rename,
            dry_run: false,
        }
    }

    #[test]
    fn splits_into_squares_in_icons_dir() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("nav.png");
        write_sheet(&input, Rgba([0, 0, 0, 0]));
        let r = split_file(&job(input.clone())).unwrap();
        assert_eq!(r.files.len(), 3);
        assert_eq!(r.out_dir, dir.path().join("nav_icons").display().to_string());
        for f in &r.files {
            let img = image::open(&f.path).unwrap().to_rgba8();
            assert_eq!(img.dimensions(), (128, 128));
            assert_eq!(img.get_pixel(0, 0)[3], 0, "透明底保留透明");
            assert_eq!(img.get_pixel(64, 64)[3], 255);
        }
        assert!(r.files[0].path.ends_with("icon_01.png"));

        // 再导一次：重名自动加序号
        let again = split_file(&job(input)).unwrap();
        assert!(again.files[0].path.ends_with("icon_01-1.png"), "{}", again.files[0].path);
    }

    #[test]
    fn custom_names_and_white_background_fill() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("sheet.png");
        write_sheet(&input, Rgba([255, 255, 255, 255]));
        let mut j = job(input);
        j.names = vec!["home".into(), "发现 页".into(), "home".into()];
        let r = split_file(&j).unwrap();
        let names: Vec<&str> = r.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["home", "发现_页", "home_03"]);
        let img = image::open(&r.files[0].path).unwrap().to_rgba8();
        assert_eq!(*img.get_pixel(0, 0), Rgba([255, 255, 255, 255]), "白底图用白色填充");
    }

    #[test]
    fn manual_regions_and_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.png");
        write_sheet(&input, Rgba([0, 0, 0, 0]));
        let mut j = job(input);
        j.regions = Some(vec![Region { x: 30, y: 30, width: 80, height: 80, kind: crate::detect::RegionKind::Icon }]);
        j.dry_run = true;
        let r = split_file(&j).unwrap();
        assert_eq!(r.mode, "manual");
        assert_eq!(r.files.len(), 1);
        assert!(!r.files[0].written && !Path::new(&r.files[0].path).exists());

        j.regions = Some(vec![Region { x: 9999, y: 0, width: 10, height: 10, kind: crate::detect::RegionKind::Icon }]);
        assert!(split_file(&j).is_err());
    }

    #[test]
    fn rejects_photo_like_images_in_auto_mode() {
        // 稀疏的随机噪点：会被识别成大量小区域
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("noise.png");
        let mut seed = 7u32;
        let img = RgbaImage::from_fn(600, 600, |x, y| {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            if x % 30 < 8 && y % 30 < 8 && (seed >> 16).is_multiple_of(2) {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([0, 0, 0, 0])
            }
        });
        DynamicImage::ImageRgba8(img).save(&input).unwrap();
        let mut j = job(input);
        j.detect.gap = Some(2);
        let err = split_file(&j).unwrap_err().to_string();
        assert!(err.contains("不是图标拼图"), "{err}");
        assert!(!dir.path().join("noise_icons").exists());
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize_name("../a b"), "a_b");
        assert_eq!(sanitize_name("  "), "icon");
        assert_eq!(sanitize_name("消息"), "消息");
    }
}
