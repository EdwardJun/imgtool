//! 各格式编码器。所有编码器接收同一份预处理好的像素，便于目标体积搜索时反复编码。

use std::panic::{catch_unwind, AssertUnwindSafe};

use image::{DynamicImage, ImageEncoder};

use crate::error::{Error, Result};
use crate::types::OutputFormat;

/// 预处理后的像素，供编码器复用
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    /// RGBA8，长度 = width * height * 4
    pub rgba: Vec<u8>,
    /// 是否真的存在半透明/透明像素
    pub has_alpha: bool,
}

impl Pixels {
    pub fn from_image(img: &DynamicImage) -> Self {
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        let rgba = rgba.into_raw();
        let has_alpha = img.color().has_alpha() && rgba.as_chunks::<4>().0.iter().any(|p| p[3] != 255);
        Self { width, height, rgba, has_alpha }
    }

    /// 去掉 alpha 通道，透明部分按 alpha 与背景色混合
    pub fn to_rgb(&self, background: [u8; 3]) -> Vec<u8> {
        let mut out = Vec::with_capacity((self.width * self.height * 3) as usize);
        for p in self.rgba.as_chunks::<4>().0 {
            let a = p[3] as u32;
            if a == 255 {
                out.extend_from_slice(&p[..3]);
            } else {
                for c in 0..3 {
                    let v = (p[c] as u32 * a + background[c] as u32 * (255 - a) + 127) / 255;
                    out.push(v as u8);
                }
            }
        }
        out
    }
}

/// 编码参数：quality 为 None 表示无损
#[derive(Debug, Clone, Copy)]
pub struct EncodeParams {
    pub quality: Option<u8>,
    /// 目标体积搜索时使用更快的参数
    pub fast: bool,
    pub background: [u8; 3],
}

pub fn encode(px: &Pixels, format: OutputFormat, params: EncodeParams) -> Result<Vec<u8>> {
    match format {
        OutputFormat::Jpeg => encode_jpeg(px, params.quality.unwrap_or(95), params.background),
        OutputFormat::Png => encode_png(px, params.quality, params.fast),
        OutputFormat::Webp => encode_webp(px, params.quality),
        OutputFormat::Avif => encode_avif(px, params.quality.unwrap_or(100), params.fast),
    }
}

fn encode_jpeg(px: &Pixels, quality: u8, background: [u8; 3]) -> Result<Vec<u8>> {
    let rgb = px.to_rgb(background);
    let (w, h) = (px.width as usize, px.height as usize);
    // mozjpeg 出错时会 unwind，必须用 catch_unwind 包住
    let result = catch_unwind(AssertUnwindSafe(|| -> std::io::Result<Vec<u8>> {
        let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
        comp.set_size(w, h);
        comp.set_quality(quality.clamp(1, 100) as f32);
        comp.set_progressive_mode();
        comp.set_optimize_coding(true);
        if quality >= 90 {
            // 高质量时关闭色度抽样，避免红色/文字边缘发糊
            comp.set_chroma_sampling_pixel_sizes((1, 1), (1, 1));
        }
        let mut started = comp.start_compress(Vec::with_capacity(w * h / 4))?;
        started.write_scanlines(&rgb)?;
        started.finish()
    }));
    match result {
        Ok(Ok(data)) => Ok(data),
        Ok(Err(e)) => Err(Error::Encode(format!("JPEG: {e}"))),
        Err(_) => Err(Error::Encode("JPEG 编码器内部错误".into())),
    }
}

/// PNG 质量到调色板颜色数的映射：质量越低颜色越少
pub fn png_colors_for_quality(quality: u8) -> usize {
    let q = quality.clamp(1, 94) as f64 / 95.0;
    ((q * q * 256.0).round() as usize).clamp(2, 256)
}

fn encode_png(px: &Pixels, quality: Option<u8>, fast: bool) -> Result<Vec<u8>> {
    let raw = match quality {
        Some(q) if q < 95 => quantize_png(px, png_colors_for_quality(q))?,
        _ => lossless_png(px)?,
    };
    let mut opts = oxipng::Options::from_preset(if fast { 1 } else { 2 });
    opts.strip = oxipng::StripChunks::Safe;
    oxipng::optimize_from_memory(&raw, &opts).map_err(|e| Error::Encode(format!("PNG 优化: {e}")))
}

fn lossless_png(px: &Pixels) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        &mut buf,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    );
    if px.has_alpha {
        encoder.write_image(&px.rgba, px.width, px.height, image::ExtendedColorType::Rgba8)
    } else {
        let rgb: Vec<u8> = px.rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        encoder.write_image(&rgb, px.width, px.height, image::ExtendedColorType::Rgb8)
    }
    .map_err(|e| Error::Encode(format!("PNG: {e}")))?;
    Ok(buf)
}

/// 有损 PNG：NeuQuant 量化成调色板图
fn quantize_png(px: &Pixels, colors: usize) -> Result<Vec<u8>> {
    let pixel_count = (px.width as u64) * (px.height as u64);
    // 大图提高采样步长以控制耗时
    let samplefac = if pixel_count > 4_000_000 { 20 } else { 10 };
    let nq = color_quant::NeuQuant::new(samplefac, colors, &px.rgba);
    let indices: Vec<u8> = px.rgba.as_chunks::<4>().0.iter().map(|p| nq.index_of(p) as u8).collect();

    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, px.width, px.height);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        enc.set_palette(nq.color_map_rgb());
        if px.has_alpha {
            enc.set_trns(nq.color_map_alpha());
        }
        let mut writer = enc.write_header().map_err(|e| Error::Encode(format!("PNG: {e}")))?;
        writer
            .write_image_data(&indices)
            .map_err(|e| Error::Encode(format!("PNG: {e}")))?;
    }
    Ok(buf)
}

fn encode_webp(px: &Pixels, quality: Option<u8>) -> Result<Vec<u8>> {
    let rgb;
    let encoder = if px.has_alpha {
        webp::Encoder::from_rgba(&px.rgba, px.width, px.height)
    } else {
        rgb = px.to_rgb([255, 255, 255]);
        webp::Encoder::from_rgb(&rgb, px.width, px.height)
    };
    let mut config = webp::WebPConfig::new().map_err(|_| Error::Encode("WebP 配置初始化失败".into()))?;
    config.method = 4;
    match quality {
        Some(q) => {
            config.lossless = 0;
            config.quality = q.clamp(1, 100) as f32;
            config.alpha_compression = 1;
        }
        None => {
            config.lossless = 1;
            config.quality = 75.0;
            config.exact = 0;
        }
    }
    let mem = encoder
        .encode_advanced(&config)
        .map_err(|e| Error::Encode(format!("WebP: {e:?}")))?;
    Ok(mem.to_vec())
}

fn encode_avif(px: &Pixels, quality: u8, fast: bool) -> Result<Vec<u8>> {
    let q = quality.clamp(1, 100) as f32;
    let enc = ravif::Encoder::new()
        .with_quality(q)
        .with_alpha_quality(q)
        .with_speed(if fast { 9 } else { 6 });
    let (w, h) = (px.width as usize, px.height as usize);
    let result = if px.has_alpha {
        let buf: Vec<ravif::RGBA8> = px
            .rgba
            .as_chunks::<4>().0.iter()
            .map(|p| ravif::RGBA8::new(p[0], p[1], p[2], p[3]))
            .collect();
        enc.encode_rgba(ravif::Img::new(buf.as_slice(), w, h))
    } else {
        let buf: Vec<ravif::RGB8> = px
            .rgba
            .as_chunks::<4>().0.iter()
            .map(|p| ravif::RGB8::new(p[0], p[1], p[2]))
            .collect();
        enc.encode_rgb(ravif::Img::new(buf.as_slice(), w, h))
    };
    result
        .map(|r| r.avif_file)
        .map_err(|e| Error::Encode(format!("AVIF: {e}")))
}
