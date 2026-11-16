//! 生成图片前后的通用处理（桌面端与 MCP 共用）：
//! - `prepare_reference`：上传参考图前按需压缩（长边 ≤ 2048，体积 ≤ 1.5MB 时原样上传）
//! - `finalize`：生成结果按需去背景、调整到最终尺寸、转换格式

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::background::BackgroundRemoval;
use crate::types::{CompressMode, OutputFormat, ProcessOptions, ResizeFit, ResizeSpec};

/// 上传前把长边限制在这个尺寸以内
pub const MAX_REFERENCE_EDGE: u32 = 2048;
/// 原图不超过这个体积、尺寸也不超限时直接上传原文件
pub const PASSTHROUGH_BYTES: u64 = 1536 * 1024;

/// 生成后在本地调整到的最终尺寸（生成接口只支持固定几档尺寸）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct FinalSize {
    pub width: u32,
    pub height: u32,
    /// cover：精确尺寸、居中裁剪（默认）；contain：等比缩放不超过；fill：拉伸
    #[serde(default = "default_fit")]
    pub fit: ResizeFit,
}

fn default_fit() -> ResizeFit {
    ResizeFit::Cover
}

pub struct Finalized {
    pub data: Vec<u8>,
    /// 实际格式名：jpeg/png/webp/avif
    pub ext: String,
    pub width: u32,
    pub height: u32,
    pub warnings: Vec<String>,
}

/// 生成结果按需去背景、调整尺寸、转换格式；格式相同且无需处理时保留原始字节
pub fn finalize(data: Vec<u8>, format: OutputFormat, remove_background: Option<BackgroundRemoval>, final_size: Option<FinalSize>) -> Result<Finalized, String> {
    let info = crate::probe(&data).map_err(|e| format!("生成结果不是可识别的图片: {e}"))?;
    let (gw, gh) = (info.width.unwrap_or(0), info.height.unwrap_or(0));
    // 已经是目标尺寸就不再缩放
    let final_size = final_size.filter(|f| f.width > 0 && f.height > 0 && (f.width, f.height) != (gw, gh));
    if remove_background.is_none() && final_size.is_none() && OutputFormat::from_extension(&info.format) == Some(format) {
        return Ok(Finalized { data, ext: info.format, width: gw, height: gh, warnings: vec![] });
    }
    let mut warnings = Vec::new();
    if let Some(f) = &final_size {
        let sx = f.width as f64 / gw.max(1) as f64;
        let sy = f.height as f64 / gh.max(1) as f64;
        let scale = match f.fit {
            ResizeFit::Contain => sx.min(sy),
            ResizeFit::Cover | ResizeFit::Fill => sx.max(sy),
        };
        if scale > 1.05 {
            warnings.push(format!("生成图 {gw}×{gh} 小于最终尺寸 {}×{}，已放大，可能变模糊；可提高质量档位", f.width, f.height));
        }
    }
    let resize = final_size.map(|f| ResizeSpec { width: Some(f.width), height: Some(f.height), fit: f.fit, without_enlargement: false });
    // JPEG 不支持透明，去背景时改存 PNG
    let format = if remove_background.is_some() && !format.supports_alpha() { OutputFormat::Png } else { format };
    let mode = if format == OutputFormat::Png { CompressMode::Lossless } else { CompressMode::Quality { quality: 92 } };
    let out = crate::process(
        &data,
        &ProcessOptions { format: Some(format), mode, remove_background, resize, keep_original_if_larger: false, ..Default::default() },
    )
    .map_err(|e| e.to_string())?;
    warnings.extend(out.report.warnings);
    Ok(Finalized { data: out.data, ext: format.extension().to_string(), width: out.report.width, height: out.report.height, warnings })
}

pub struct Prepared {
    pub data: Vec<u8>,
    pub mime: &'static str,
    pub filename: String,
    /// 是否经过压缩
    pub compressed: bool,
}

fn mime_of(format: &str) -> Option<&'static str> {
    match format {
        "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// 读取并按需压缩参考图：长边 ≤ 2048；有透明像素存 PNG，否则存质量 90 的 JPEG
pub fn prepare_reference(path: &Path) -> Result<Prepared, String> {
    let data = std::fs::read(path).map_err(|e| format!("读取参考图 {} 失败: {e}", path.display()))?;
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("ref").to_string();
    prepare_reference_bytes(data, &stem).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn prepare_reference_bytes(data: Vec<u8>, stem: &str) -> Result<Prepared, String> {
    let info = crate::probe(&data).map_err(|e| e.to_string())?;
    if !info.decodable {
        return Err(format!("暂不支持 {} 格式作为参考图", info.format));
    }
    let small = info.width.unwrap_or(u32::MAX).max(info.height.unwrap_or(u32::MAX)) <= MAX_REFERENCE_EDGE;
    if let (true, true, Some(mime)) = (small, (data.len() as u64) <= PASSTHROUGH_BYTES, mime_of(&info.format)) {
        let ext = if info.format == "jpeg" { "jpg" } else { info.format.as_str() };
        return Ok(Prepared { data, mime, filename: format!("{stem}.{ext}"), compressed: false });
    }
    let img = crate::decode::decode(&data).map_err(|e| e.to_string())?.image;
    let has_alpha = img.color().has_alpha() && img.to_rgba8().pixels().any(|p| p[3] < 255);
    let (format, mode) = if has_alpha { (OutputFormat::Png, CompressMode::Lossless) } else { (OutputFormat::Jpeg, CompressMode::Quality { quality: 90 }) };
    let out = crate::process(
        &data,
        &ProcessOptions {
            format: Some(format),
            mode,
            resize: Some(ResizeSpec {
                width: Some(MAX_REFERENCE_EDGE),
                height: Some(MAX_REFERENCE_EDGE),
                fit: ResizeFit::Contain,
                without_enlargement: true,
            }),
            keep_original_if_larger: false,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(Prepared {
        data: out.data,
        mime: if has_alpha { "image/png" } else { "image/jpeg" },
        filename: format!("{stem}.{}", if has_alpha { "png" } else { "jpg" }),
        compressed: true,
    })
}

/// 生成预览缩略图（JPEG/PNG 字节 + MIME），供 Agent 直接查看
pub fn thumbnail(data: &[u8], max_edge: u32) -> Result<(Vec<u8>, &'static str), String> {
    let img = crate::decode::decode(data).map_err(|e| e.to_string())?.image;
    let has_alpha = img.color().has_alpha() && img.to_rgba8().pixels().any(|p| p[3] < 255);
    let format = if has_alpha { OutputFormat::Png } else { OutputFormat::Jpeg };
    let out = crate::process(
        data,
        &ProcessOptions {
            format: Some(format),
            mode: CompressMode::Quality { quality: 80 },
            resize: Some(ResizeSpec { width: Some(max_edge), height: Some(max_edge), fit: ResizeFit::Contain, without_enlargement: true }),
            keep_original_if_larger: false,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    Ok((out.data, format.mime()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, alpha: bool) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, if alpha && x < 4 { 0 } else { 255 }]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    #[test]
    fn thumbnail_limits_edge_and_keeps_alpha_as_png() {
        let (t, mime) = thumbnail(&png(1200, 600, false), 300).unwrap();
        assert_eq!(mime, "image/jpeg");
        let img = image::load_from_memory(&t).unwrap();
        assert_eq!((img.width(), img.height()), (300, 150));
        let (_, mime) = thumbnail(&png(100, 100, true), 300).unwrap();
        assert_eq!(mime, "image/png");
    }

    #[test]
    fn finalize_passthrough_and_resize() {
        let src = png(64, 48, false);
        let same = finalize(src.clone(), OutputFormat::Png, None, None).unwrap();
        assert_eq!(same.data, src);
        let r = finalize(src, OutputFormat::Webp, None, Some(FinalSize { width: 32, height: 32, fit: ResizeFit::Cover })).unwrap();
        assert_eq!((r.width, r.height, r.ext.as_str()), (32, 32, "webp"));
    }
}
