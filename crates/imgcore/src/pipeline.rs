use image::{imageops::FilterType, DynamicImage};

use crate::decode::decode;
use crate::encode::{encode, EncodeParams, Pixels};
use crate::error::{Error, Result};
use crate::types::*;

const DEFAULT_MIN_QUALITY: u8 = 40;
const MAX_SHRINK_ROUNDS: usize = 8;
const MIN_DIMENSION: u32 = 8;

/// 处理单张图片：解码 → 裁剪 → 缩放 → 编码（按压缩方式）
pub fn process(input: &[u8], opts: &ProcessOptions) -> Result<ProcessOutput> {
    let decoded = decode(input)?;
    let source_name = decoded.format_name;
    let mut warnings = Vec::new();
    if source_name == "gif" {
        warnings.push("GIF 动图只处理第一帧".to_string());
    }

    let mut img = decoded.image;
    let (orig_w, orig_h) = (img.width(), img.height());

    if let Some(crop) = &opts.crop {
        img = apply_crop(img, crop)?;
    }
    if let Some(resize) = &opts.resize {
        img = apply_resize(img, resize)?;
    }

    let source_fmt = OutputFormat::from_extension(source_name);
    let out_fmt = opts.format.or(source_fmt).unwrap_or(OutputFormat::Png);
    let geometry_changed = (img.width(), img.height()) != (orig_w, orig_h);
    let same_as_source = !geometry_changed && source_fmt == Some(out_fmt);
    let background = opts.background.unwrap_or([255, 255, 255]);

    let original_report = |data_len: u64| ProcessReport {
        format: out_fmt,
        width: img.width(),
        height: img.height(),
        original_format: source_name.to_string(),
        original_width: orig_w,
        original_height: orig_h,
        original_bytes: input.len() as u64,
        output_bytes: data_len,
        quality: None,
        kept_original: false,
        target_met: None,
        warnings: Vec::new(),
    };

    let px = Pixels::from_image(&img);
    if out_fmt == OutputFormat::Jpeg && px.has_alpha {
        warnings.push("JPEG 不支持透明，透明区域已填充背景色".to_string());
    }

    let (data, quality, width, height, target_met) = match &opts.mode {
        CompressMode::Quality { quality } => {
            let q = (*quality).clamp(1, 100);
            let data = encode(&px, out_fmt, EncodeParams { quality: Some(q), fast: false, background })?;
            (data, Some(q), px.width, px.height, None)
        }
        CompressMode::Lossless => {
            let q = match out_fmt {
                OutputFormat::Png | OutputFormat::Webp => None,
                OutputFormat::Jpeg => {
                    warnings.push("JPEG 无法无损重编码，已使用质量 95".to_string());
                    Some(95)
                }
                OutputFormat::Avif => {
                    warnings.push("AVIF 无损暂不支持，已使用质量 100".to_string());
                    Some(100)
                }
            };
            let data = encode(&px, out_fmt, EncodeParams { quality: q, fast: false, background })?;
            (data, q, px.width, px.height, None)
        }
        CompressMode::TargetSize { max_bytes, min_quality } => {
            if *max_bytes == 0 {
                return Err(Error::InvalidArgument("目标体积必须大于 0".into()));
            }
            if same_as_source && input.len() as u64 <= *max_bytes {
                let mut report = original_report(input.len() as u64);
                report.kept_original = true;
                report.target_met = Some(true);
                report.warnings = warnings;
                return Ok(ProcessOutput { data: input.to_vec(), report });
            }
            let min_q = min_quality.unwrap_or(DEFAULT_MIN_QUALITY).clamp(1, 94);
            let found = search_target_size(img.clone(), px, out_fmt, *max_bytes, min_q, background, &mut warnings)?;
            (found.data, Some(found.quality), found.width, found.height, Some(found.met))
        }
    };

    if opts.keep_original_if_larger && same_as_source && data.len() >= input.len() {
        let mut report = original_report(input.len() as u64);
        report.kept_original = true;
        report.target_met = target_met.map(|_| false);
        warnings.push("压缩后没有变小，已保留原图".to_string());
        report.warnings = warnings;
        return Ok(ProcessOutput { data: input.to_vec(), report });
    }

    let report = ProcessReport {
        format: out_fmt,
        width,
        height,
        original_format: source_name.to_string(),
        original_width: orig_w,
        original_height: orig_h,
        original_bytes: input.len() as u64,
        output_bytes: data.len() as u64,
        quality,
        kept_original: false,
        target_met,
        warnings,
    };
    Ok(ProcessOutput { data, report })
}

struct SearchResult {
    data: Vec<u8>,
    quality: u8,
    width: u32,
    height: u32,
    met: bool,
}

/// 目标体积搜索：每轮在 [min_q, 95] 二分查找满足体积的最高质量；
/// 质量降到下限仍超标时，按体积比例缩小分辨率后进入下一轮。
fn search_target_size(
    mut img: DynamicImage,
    mut px: Pixels,
    fmt: OutputFormat,
    max_bytes: u64,
    min_q: u8,
    background: [u8; 3],
    warnings: &mut Vec<String>,
) -> Result<SearchResult> {
    let mut smallest: Option<SearchResult> = None;

    for round in 0..MAX_SHRINK_ROUNDS {
        if round > 0 {
            px = Pixels::from_image(&img);
        }
        let enc = |q: u8| encode(&px, fmt, EncodeParams { quality: Some(q), fast: true, background });

        let top = enc(95)?;
        if top.len() as u64 <= max_bytes {
            return Ok(SearchResult { data: top, quality: 95, width: px.width, height: px.height, met: true });
        }

        let (mut lo, mut hi) = (min_q, 94u8);
        let mut best: Option<(u8, Vec<u8>)> = None;
        let mut round_smallest = top;
        while lo <= hi {
            let mid = lo + (hi - lo) / 2;
            let data = enc(mid)?;
            if data.len() as u64 <= max_bytes {
                best = Some((mid, data));
                lo = mid + 1;
            } else {
                if data.len() < round_smallest.len() {
                    round_smallest = data;
                }
                if mid == 0 {
                    break;
                }
                hi = mid - 1;
            }
        }
        if let Some((q, data)) = best {
            return Ok(SearchResult { data, quality: q, width: px.width, height: px.height, met: true });
        }

        let ratio = max_bytes as f64 / round_smallest.len() as f64;
        let replace = smallest.as_ref().map(|s| round_smallest.len() < s.data.len()).unwrap_or(true);
        let (cur_w, cur_h) = (px.width, px.height);
        if replace {
            smallest = Some(SearchResult { data: round_smallest, quality: min_q, width: cur_w, height: cur_h, met: false });
        }

        // 体积大致与像素数成正比，按面积比开方得到边长缩放系数，再留一点余量
        let scale = (ratio.sqrt() * 0.92).clamp(0.25, 0.9);
        let nw = ((cur_w as f64) * scale).round() as u32;
        let nh = ((cur_h as f64) * scale).round() as u32;
        if nw < MIN_DIMENSION || nh < MIN_DIMENSION {
            break;
        }
        img = img.resize_exact(nw, nh, FilterType::Lanczos3);
    }

    let result = smallest.ok_or_else(|| Error::Encode("目标体积搜索失败".into()))?;
    warnings.push(format!(
        "已缩小到 {}x{}、质量 {}，仍无法达到目标体积",
        result.width, result.height, result.quality
    ));
    Ok(result)
}

pub fn apply_crop(img: DynamicImage, crop: &CropSpec) -> Result<DynamicImage> {
    let (w, h) = (img.width(), img.height());
    let (x, y, cw, ch) = match *crop {
        CropSpec::Rect { x, y, width, height } => {
            if x >= w || y >= h {
                return Err(Error::InvalidArgument(format!("裁剪起点 ({x},{y}) 超出图片范围 {w}x{h}")));
            }
            (x, y, width.min(w - x), height.min(h - y))
        }
        CropSpec::Aspect { width: aw, height: ah } => {
            if aw == 0 || ah == 0 {
                return Err(Error::InvalidArgument("宽高比必须大于 0".into()));
            }
            let target = aw as f64 / ah as f64;
            let current = w as f64 / h as f64;
            if current > target {
                let cw = ((h as f64) * target).round() as u32;
                ((w - cw) / 2, 0, cw, h)
            } else {
                let ch = ((w as f64) / target).round() as u32;
                (0, (h - ch) / 2, w, ch)
            }
        }
    };
    if cw == 0 || ch == 0 {
        return Err(Error::InvalidArgument("裁剪区域为空".into()));
    }
    if (x, y, cw, ch) == (0, 0, w, h) {
        return Ok(img);
    }
    Ok(img.crop_imm(x, y, cw, ch))
}

pub fn apply_resize(img: DynamicImage, spec: &ResizeSpec) -> Result<DynamicImage> {
    let (w, h) = (img.width() as f64, img.height() as f64);
    let filter = FilterType::Lanczos3;
    match (spec.width, spec.height) {
        (None, None) => Ok(img),
        (Some(0), _) | (_, Some(0)) => Err(Error::InvalidArgument("缩放尺寸必须大于 0".into())),
        (Some(tw), None) => {
            let scale = tw as f64 / w;
            scale_by(img, scale, spec.without_enlargement, filter)
        }
        (None, Some(th)) => {
            let scale = th as f64 / h;
            scale_by(img, scale, spec.without_enlargement, filter)
        }
        (Some(tw), Some(th)) => match spec.fit {
            ResizeFit::Contain => {
                let scale = (tw as f64 / w).min(th as f64 / h);
                scale_by(img, scale, spec.without_enlargement, filter)
            }
            ResizeFit::Fill => {
                if spec.without_enlargement && tw as f64 >= w && th as f64 >= h {
                    return Ok(img);
                }
                Ok(img.resize_exact(tw, th, filter))
            }
            ResizeFit::Cover => {
                let scale = (tw as f64 / w).max(th as f64 / h);
                if spec.without_enlargement && scale > 1.0 {
                    // 不放大时只按目标宽高比居中裁剪
                    return apply_crop(img, &CropSpec::Aspect { width: tw, height: th });
                }
                let resized = scale_by(img, scale, false, filter)?;
                let (rw, rh) = (resized.width(), resized.height());
                let (cw, ch) = (tw.min(rw), th.min(rh));
                Ok(resized.crop_imm((rw - cw) / 2, (rh - ch) / 2, cw, ch))
            }
        },
    }
}

fn scale_by(img: DynamicImage, scale: f64, without_enlargement: bool, filter: FilterType) -> Result<DynamicImage> {
    if (without_enlargement && scale >= 1.0) || (scale - 1.0).abs() < 1e-9 {
        return Ok(img);
    }
    let nw = ((img.width() as f64) * scale).round().max(1.0) as u32;
    let nh = ((img.height() as f64) * scale).round().max(1.0) as u32;
    Ok(img.resize_exact(nw, nh, filter))
}
