//! SVG 导出：清洗后保存 `<name>.svg`，并渲染成多个尺寸的 `<name>-<size>.<ext>`。
//! 桌面端和 MCP 共用。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::encode::{encode, EncodeParams, Pixels};
use crate::error::{Error, Result};
use crate::project::output::{resolve_conflict, write_atomic};
use crate::project::preset::{ConflictPolicy, OutputPolicy};
use crate::project::split::sanitize_name;
use crate::svg::{render_square_sizes, sanitize, SanitizeOptions};
use crate::types::OutputFormat;

/// 默认导出尺寸
pub const DEFAULT_SIZES: &[u32] = &[64, 128, 256];
/// 单次最多导出的尺寸数
pub const MAX_SIZES: usize = 12;

#[derive(Debug, Clone)]
pub struct SvgExportJob {
    pub svg: Vec<u8>,
    /// 文件名（不含扩展名），会清洗成安全文件名
    pub name: String,
    pub out_dir: PathBuf,
    /// 正方形边长列表，空表示 [`DEFAULT_SIZES`]
    pub sizes: Vec<u32>,
    pub format: OutputFormat,
    /// 位图质量；PNG 为 None 时无损
    pub quality: Option<u8>,
    /// 线条 / 填充颜色（SVG 使用 currentColor 时生效）
    pub color: Option<String>,
    /// 同时保存清洗后的 .svg
    pub save_svg: bool,
    /// 来源文件（避免覆盖），没有时为空
    pub source: Option<PathBuf>,
    pub on_conflict: ConflictPolicy,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SvgExportFile {
    pub path: String,
    /// "svg" 或位图格式名
    pub format: String,
    /// 位图边长；svg 文件为空
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    pub bytes: u64,
    pub written: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SvgExportResult {
    pub name: String,
    /// 清洗后的 SVG 文本
    pub svg: String,
    pub files: Vec<SvgExportFile>,
    /// 清洗时去掉的元素 / 属性
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// 颜色只接受 #rgb / #rrggbb / #rrggbbaa、CSS 颜色名和 currentColor，防止注入属性
pub fn validate_color(c: &str) -> Result<String> {
    let c = c.trim();
    let hex = c.strip_prefix('#').is_some_and(|h| matches!(h.len(), 3 | 4 | 6 | 8) && h.chars().all(|x| x.is_ascii_hexdigit()));
    let named = !c.is_empty() && c.len() <= 30 && c.chars().all(|x| x.is_ascii_alphabetic());
    if hex || named {
        Ok(c.to_string())
    } else {
        Err(Error::InvalidArgument(format!("颜色格式无效: {c}（示例 #4F46E5）")))
    }
}

/// 去重并校验尺寸列表，空列表用默认值
pub fn size_list(sizes: &[u32]) -> Result<Vec<u32>> {
    let src = if sizes.is_empty() { DEFAULT_SIZES } else { sizes };
    let mut out: Vec<u32> = Vec::new();
    for &s in src {
        if !(8..=4096).contains(&s) {
            return Err(Error::InvalidArgument(format!("输出尺寸 {s} 超出范围 8-4096")));
        }
        if !out.contains(&s) {
            out.push(s);
        }
    }
    if out.len() > MAX_SIZES {
        return Err(Error::InvalidArgument(format!("最多 {MAX_SIZES} 个尺寸")));
    }
    Ok(out)
}

pub fn export_svg(job: &SvgExportJob) -> Result<SvgExportResult> {
    let text = std::str::from_utf8(&job.svg).map_err(|_| Error::Decode("SVG 必须是 UTF-8 文本".into()))?;
    let color = job.color.as_deref().map(validate_color).transpose()?;
    let cleaned = sanitize(text, &SanitizeOptions { color: color.clone() })?;
    let mut warnings = Vec::new();
    if color.is_some() && !cleaned.svg.contains("currentColor") {
        warnings.push("SVG 没有使用 currentColor，颜色设置不生效（需把 stroke/fill 写成 currentColor）".into());
    }
    if !cleaned.removed.is_empty() {
        warnings.push(format!("已移除不安全或不支持的内容：{}", cleaned.removed.join("、")));
    }
    let sizes = size_list(&job.sizes)?;
    let rendered = render_square_sizes(cleaned.svg.as_bytes(), &sizes)?;

    let name = sanitize_name(&job.name);
    let policy = OutputPolicy::default();
    let source = job.source.clone().unwrap_or_else(|| job.out_dir.join(".imgtool-no-source"));
    let mut files = Vec::new();
    let mut write = |file_name: String, bytes: &[u8], format: String, size: Option<u32>| -> Result<()> {
        let planned = job.out_dir.join(file_name);
        let (path, written) = if job.dry_run {
            (planned, false)
        } else {
            let target = resolve_conflict(planned, &source, &policy, job.on_conflict)?;
            write_atomic(&target, bytes)?;
            (target, true)
        };
        files.push(SvgExportFile { path: path.display().to_string(), format, size, bytes: bytes.len() as u64, written });
        Ok(())
    };
    if job.save_svg {
        write(format!("{name}.svg"), cleaned.svg.as_bytes(), "svg".into(), None)?;
    }
    let quality = match job.format {
        OutputFormat::Png => job.quality,
        _ => Some(job.quality.unwrap_or(90)),
    };
    for r in rendered {
        let px = Pixels::from_image(&image::DynamicImage::ImageRgba8(r.image));
        let bytes = encode(&px, job.format, EncodeParams { quality, fast: false, background: [255, 255, 255] })?;
        write(format!("{name}-{}.{}", r.size, job.format.extension()), &bytes, job.format.extension().into(), Some(r.size))?;
    }
    Ok(SvgExportResult { name, svg: cleaned.svg, files, removed: cleaned.removed, warnings })
}

/// 一套图标中单个图标的导出结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IconSetItem {
    /// 图标名称（用户输入）
    pub label: String,
    #[serde(flatten)]
    pub result: Option<SvgExportResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 导出一套图标：`items` 为 (名称, 文件名, SVG)。同一套里文件名重复时追加序号；
/// 单个图标失败不影响其他图标。`template` 里的 svg / name 字段会被忽略。
pub fn export_icon_set(items: &[(String, String, String)], template: &SvgExportJob) -> Vec<IconSetItem> {
    let mut used = std::collections::HashSet::new();
    items
        .iter()
        .enumerate()
        .map(|(i, (label, stem, svg))| {
            let mut name = sanitize_name(stem);
            if !used.insert(name.clone()) {
                name = format!("{name}-{}", i + 1);
                used.insert(name.clone());
            }
            let job = SvgExportJob { svg: svg.as_bytes().to_vec(), name, ..template.clone() };
            match export_svg(&job) {
                Ok(r) => IconSetItem { label: label.clone(), result: Some(r), error: None },
                Err(e) => IconSetItem { label: label.clone(), result: None, error: Some(e.to_string()) },
            }
        })
        .collect()
}

/// 读取项目内的 .svg 文件作为导出来源
pub fn read_svg_file(path: &Path) -> Result<Vec<u8>> {
    let data = std::fs::read(path)?;
    if crate::decode::sniff_format(&data) != Some("svg") {
        return Err(Error::InvalidArgument(format!("{} 不是 SVG 文件", path.display())));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="8"/><script>x</script></svg>"#;

    fn job(dir: &Path) -> SvgExportJob {
        SvgExportJob {
            svg: ICON.as_bytes().to_vec(),
            name: "我的 icon".into(),
            out_dir: dir.to_path_buf(),
            sizes: vec![],
            format: OutputFormat::Png,
            quality: None,
            color: Some("#4F46E5".into()),
            save_svg: true,
            source: None,
            on_conflict: ConflictPolicy::Error,
            dry_run: false,
        }
    }

    #[test]
    fn exports_svg_and_default_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let r = export_svg(&job(dir.path())).unwrap();
        let names: Vec<String> = r.files.iter().map(|f| Path::new(&f.path).file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["我的_icon.svg", "我的_icon-64.png", "我的_icon-128.png", "我的_icon-256.png"]);
        let saved = std::fs::read_to_string(dir.path().join("我的_icon.svg")).unwrap();
        assert!(saved.contains(r##"color="#4F46E5""##) && !saved.contains("script"));
        let png = image::open(dir.path().join("我的_icon-128.png")).unwrap().to_rgba8();
        assert_eq!(png.dimensions(), (128, 128));
        assert!(png.pixels().any(|p| p.0 == [0x4F, 0x46, 0xE5, 255]), "按颜色渲染");
        assert!(r.warnings.iter().any(|w| w.contains("<script>")));
        // 再导一次：默认报错不覆盖
        assert!(export_svg(&job(dir.path())).is_err());
    }

    #[test]
    fn exports_icon_set_with_dedup_and_partial_failure() {
        let dir = tempfile::tempdir().unwrap();
        let items = vec![
            ("首页".to_string(), "home".to_string(), ICON.to_string()),
            ("主页".to_string(), "home".to_string(), ICON.to_string()),
            ("坏的".to_string(), "bad".to_string(), "<svg".to_string()),
        ];
        let mut t = job(dir.path());
        t.sizes = vec![32];
        let r = export_icon_set(&items, &t);
        assert_eq!(r[0].result.as_ref().unwrap().name, "home");
        assert_eq!(r[1].result.as_ref().unwrap().name, "home-2");
        assert!(r[2].result.is_none() && r[2].error.is_some());
        assert!(dir.path().join("home-2-32.png").exists());
    }

    #[test]
    fn validates_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let mut j = job(dir.path());
        j.color = Some(r#"red" onload="x"#.into());
        assert!(export_svg(&j).is_err(), "颜色注入");
        let mut j = job(dir.path());
        j.sizes = vec![4];
        assert!(export_svg(&j).is_err());
        let mut j = job(dir.path());
        j.svg = b"<html/>".to_vec();
        assert!(export_svg(&j).is_err());
        let mut j = job(dir.path());
        j.dry_run = true;
        j.format = OutputFormat::Webp;
        j.sizes = vec![32, 32, 48];
        let r = export_svg(&j).unwrap();
        assert_eq!(r.files.len(), 3, "尺寸去重");
        assert!(r.files.iter().all(|f| !f.written && !Path::new(&f.path).exists()));
    }
}
