//! 把 Agent 对话里的图片（地址 / base64 / 本机文件）存进项目，供其他工具按路径处理

use std::path::{Path, PathBuf};

use base64::Engine;

use imgcore::project::{resolve_conflict, sanitize_name, write_atomic, ConflictPolicy, OutputPolicy};

/// 单张图片上限
pub const MAX_IMPORT_BYTES: usize = 30 * 1024 * 1024;
pub const INBOX_DIR: &str = ".imgtool/inbox";

/// 解析 base64，兼容 `data:image/png;base64,...` 前缀和换行
pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let s = input.trim();
    let payload = match s.strip_prefix("data:") {
        Some(rest) => rest.split_once(',').map(|(_, d)| d).ok_or("data URL 缺少逗号分隔的数据部分")?,
        None => s,
    };
    let cleaned: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() / 4 * 3 > MAX_IMPORT_BYTES + 4 {
        return Err(format!("图片超过 {}MB", MAX_IMPORT_BYTES / 1024 / 1024));
    }
    base64::engine::general_purpose::STANDARD
        .decode(&cleaned)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(&cleaned))
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(cleaned.trim_end_matches('=')))
        .map_err(|e| format!("base64 解码失败: {e}"))
}

/// 校验是可处理的图片，按识别出的格式命名后写入目录；返回 (路径, 图片信息)
pub fn save_image(dir: &Path, stem_hint: Option<&str>, data: &[u8]) -> Result<(PathBuf, imgcore::ImageInfo), String> {
    if data.is_empty() {
        return Err("图片为空".into());
    }
    if data.len() > MAX_IMPORT_BYTES {
        return Err(format!("图片超过 {}MB", MAX_IMPORT_BYTES / 1024 / 1024));
    }
    let info = imgcore::probe(data).map_err(|_| "不是可识别的图片（支持 jpeg/png/webp/gif/bmp/tiff/ico）".to_string())?;
    if !info.decodable {
        return Err(format!("暂不支持 {} 格式（支持 jpeg/png/webp/gif/bmp/tiff/ico）", info.format));
    }
    let ext = if info.format == "jpeg" { "jpg" } else { info.format.as_str() };
    let stem = stem_hint
        .map(|s| Path::new(s).file_stem().and_then(|x| x.to_str()).unwrap_or(s).to_string())
        .map(|s| sanitize_name(&s))
        .filter(|s| s != "icon")
        .unwrap_or_else(|| {
            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            format!("image-{ts}")
        });
    let target = resolve_conflict(dir.join(format!("{stem}.{ext}")), Path::new(""), &OutputPolicy::default(), ConflictPolicy::Rename)
        .map_err(|e| e.to_string())?;
    write_atomic(&target, data).map_err(|e| e.to_string())?;
    let mut info = info;
    info.bytes = data.len() as u64;
    Ok((target, info))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 10, 10]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    #[test]
    fn decodes_plain_and_data_url() {
        let data = png();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
        assert_eq!(decode_base64(&b64).unwrap(), data);
        assert_eq!(decode_base64(&format!("data:image/png;base64,{b64}")).unwrap(), data);
        let wrapped: String = b64.chars().collect::<Vec<_>>().chunks(10).map(|c| c.iter().collect::<String>() + "\n").collect();
        assert_eq!(decode_base64(&wrapped).unwrap(), data);
        assert!(decode_base64("data:image/png;base64").is_err());
        assert!(decode_base64("!!!").is_err());
    }

    #[test]
    fn saves_with_detected_extension_and_rejects_non_images() {
        let dir = tempfile::tempdir().unwrap();
        // 文件名给的是 .jpg，实际是 PNG → 按实际格式存
        let (p, info) = save_image(dir.path(), Some("设计稿 v1.jpg"), &png()).unwrap();
        assert_eq!(p, dir.path().join("设计稿_v1.png"));
        assert_eq!((info.width, info.height), (Some(8), Some(4)));
        let (p2, _) = save_image(dir.path(), Some("设计稿 v1.jpg"), &png()).unwrap();
        assert_eq!(p2, dir.path().join("设计稿_v1-1.png"), "重名自动加序号");
        assert!(save_image(dir.path(), None, b"not an image").is_err());
        assert!(save_image(dir.path(), None, br#"<svg xmlns="http://www.w3.org/2000/svg"></svg>"#).unwrap_err().contains("svg"));
    }
}
