use std::io::Cursor;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};

use crate::error::{Error, Result};
use crate::types::ImageInfo;

/// 通过文件头魔数识别格式名，包含本工具不能解码的格式（avif/heic/svg）
pub fn sniff_format(data: &[u8]) -> Option<&'static str> {
    if data.len() >= 12 && &data[4..8] == b"ftyp" {
        let brand = &data[8..12];
        return match brand {
            b"avif" | b"avis" => Some("avif"),
            b"heic" | b"heix" | b"hevc" | b"heim" | b"heis" | b"mif1" | b"msf1" => Some("heic"),
            _ => None,
        };
    }
    let head = &data[..data.len().min(256)];
    let text = String::from_utf8_lossy(head);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with("<svg") || (trimmed.starts_with("<?xml") && text.contains("<svg")) {
        return Some("svg");
    }
    image::guess_format(data).ok().map(format_name)
}

pub fn format_name(f: ImageFormat) -> &'static str {
    match f {
        ImageFormat::Jpeg => "jpeg",
        ImageFormat::Png => "png",
        ImageFormat::WebP => "webp",
        ImageFormat::Gif => "gif",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Tiff => "tiff",
        ImageFormat::Ico => "ico",
        ImageFormat::Avif => "avif",
        _ => "unknown",
    }
}

fn is_decodable(name: &str) -> bool {
    matches!(name, "jpeg" | "png" | "webp" | "gif" | "bmp" | "tiff" | "ico")
}

/// 读取基础信息，只解析文件头
pub fn probe(data: &[u8]) -> Result<ImageInfo> {
    let name = sniff_format(data).ok_or(Error::UnknownFormat)?;
    let decodable = is_decodable(name);
    let (width, height) = if decodable {
        let reader = ImageReader::new(Cursor::new(data))
            .with_guessed_format()
            .map_err(Error::Io)?;
        match reader.into_decoder() {
            Ok(mut decoder) => {
                let (w, h) = decoder.dimensions();
                // EXIF 方向为 90/270 度时宽高互换，报告旋正后的尺寸
                let swap = decoder
                    .orientation()
                    .map(|o| {
                        use image::metadata::Orientation::*;
                        matches!(o, Rotate90 | Rotate270 | Rotate90FlipH | Rotate270FlipH)
                    })
                    .unwrap_or(false);
                if swap {
                    (Some(h), Some(w))
                } else {
                    (Some(w), Some(h))
                }
            }
            Err(_) => (None, None),
        }
    } else {
        (None, None)
    };
    Ok(ImageInfo {
        format: name.to_string(),
        width,
        height,
        bytes: data.len() as u64,
        decodable,
    })
}

pub struct Decoded {
    pub image: DynamicImage,
    pub format_name: &'static str,
    pub source_format: Option<ImageFormat>,
}

/// 完整解码并按 EXIF 方向旋正
pub fn decode(data: &[u8]) -> Result<Decoded> {
    let name = sniff_format(data).ok_or(Error::UnknownFormat)?;
    if !is_decodable(name) {
        return Err(Error::Unsupported(name.to_string()));
    }
    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(Error::Io)?;
    let source_format = reader.format();
    let mut decoder = reader
        .into_decoder()
        .map_err(|e| Error::Decode(e.to_string()))?;
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder).map_err(|e| Error::Decode(e.to_string()))?;
    if let Some(o) = orientation {
        image.apply_orientation(o);
    }
    Ok(Decoded {
        image,
        format_name: name,
        source_format,
    })
}
