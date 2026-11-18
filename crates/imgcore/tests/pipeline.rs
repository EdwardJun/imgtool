use std::io::Cursor;

use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use imgcore::*;

/// 生成带渐变和噪声的"照片"，让有损编码体积对质量敏感
fn sample_image(w: u32, h: u32, alpha: bool) -> DynamicImage {
    let mut seed = 12345u32;
    let img = RgbaImage::from_fn(w, h, |x, y| {
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        let n = ((seed >> 16) & 0x3f) as u8;
        let a = if alpha && x < w / 4 { 0 } else { 255 };
        Rgba([(x * 255 / w) as u8 ^ n, (y * 255 / h) as u8, n.wrapping_mul(3), a])
    });
    DynamicImage::ImageRgba8(img)
}

fn encode_as(img: &DynamicImage, fmt: ImageFormat) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    match fmt {
        ImageFormat::Jpeg => DynamicImage::ImageRgb8(img.to_rgb8()).write_to(&mut buf, fmt).unwrap(),
        _ => img.write_to(&mut buf, fmt).unwrap(),
    }
    buf.into_inner()
}

fn decoded_dims(data: &[u8]) -> (u32, u32) {
    let img = image::load_from_memory(data).unwrap();
    (img.width(), img.height())
}

#[test]
fn quality_mode_shrinks_png() {
    let src = encode_as(&sample_image(400, 300, true), ImageFormat::Png);
    let out = process(&src, &ProcessOptions { mode: CompressMode::Quality { quality: 60 }, ..Default::default() }).unwrap();
    assert_eq!(out.report.format, OutputFormat::Png);
    assert!(out.report.output_bytes < out.report.original_bytes, "{:?}", out.report);
    assert_eq!(decoded_dims(&out.data), (400, 300));
}

#[test]
fn converts_to_every_format() {
    let src = encode_as(&sample_image(160, 120, true), ImageFormat::Png);
    for fmt in OutputFormat::ALL {
        let opts = ProcessOptions { format: Some(fmt), mode: CompressMode::Quality { quality: 70 }, ..Default::default() };
        let out = process(&src, &opts).unwrap();
        assert_eq!(out.report.format, fmt);
        let sniffed = imgcore::decode::sniff_format(&out.data).unwrap();
        assert_eq!(sniffed, match fmt {
            OutputFormat::Jpeg => "jpeg",
            OutputFormat::Png => "png",
            OutputFormat::Webp => "webp",
            OutputFormat::Avif => "avif",
        });
        if fmt == OutputFormat::Jpeg {
            assert!(out.report.warnings.iter().any(|w| w.contains("透明")));
        }
    }
}

#[test]
fn target_size_is_met_for_jpeg() {
    let src = encode_as(&sample_image(1200, 900, false), ImageFormat::Jpeg);
    let max = 60 * 1024;
    let opts = ProcessOptions { mode: CompressMode::TargetSize { max_bytes: max, min_quality: None }, ..Default::default() };
    let out = process(&src, &opts).unwrap();
    assert_eq!(out.report.target_met, Some(true), "{:?}", out.report);
    assert!(out.data.len() as u64 <= max);
}

#[test]
fn target_size_shrinks_resolution_when_needed() {
    let src = encode_as(&sample_image(800, 600, false), ImageFormat::Png);
    let max = 8 * 1024;
    let opts = ProcessOptions {
        format: Some(OutputFormat::Webp),
        mode: CompressMode::TargetSize { max_bytes: max, min_quality: Some(50) },
        ..Default::default()
    };
    let out = process(&src, &opts).unwrap();
    assert_eq!(out.report.target_met, Some(true), "{:?}", out.report);
    assert!(out.report.width < 800);
    assert!(out.data.len() as u64 <= max);
    // 缩小时保持宽高比
    let ratio = out.report.width as f64 / out.report.height as f64;
    assert!((ratio - 4.0 / 3.0).abs() < 0.02);
}

#[test]
fn crop_then_resize() {
    let src = encode_as(&sample_image(400, 300, false), ImageFormat::Png);
    let opts = ProcessOptions {
        crop: Some(CropSpec::Aspect { width: 1, height: 1 }),
        resize: Some(ResizeSpec { width: Some(100), height: None, fit: ResizeFit::Contain, without_enlargement: true }),
        mode: CompressMode::Lossless,
        ..Default::default()
    };
    let out = process(&src, &opts).unwrap();
    assert_eq!(decoded_dims(&out.data), (100, 100));

    let rect = ProcessOptions {
        crop: Some(CropSpec::Rect { x: 350, y: 250, width: 500, height: 500 }),
        mode: CompressMode::Lossless,
        ..Default::default()
    };
    // 超出边界的部分被截断
    assert_eq!(decoded_dims(&process(&src, &rect).unwrap().data), (50, 50));
}

#[test]
fn cover_fit_produces_exact_box() {
    let src = encode_as(&sample_image(400, 300, false), ImageFormat::Png);
    let opts = ProcessOptions {
        resize: Some(ResizeSpec { width: Some(200), height: Some(200), fit: ResizeFit::Cover, without_enlargement: true }),
        format: Some(OutputFormat::Webp),
        ..Default::default()
    };
    assert_eq!(decoded_dims(&process(&src, &opts).unwrap().data), (200, 200));
}

#[test]
fn keeps_original_when_not_smaller() {
    // 先用低质量压一遍，再用高质量压，结果必然更大 → 保留原图
    let src = encode_as(&sample_image(300, 200, false), ImageFormat::Jpeg);
    let low = process(&src, &ProcessOptions { mode: CompressMode::Quality { quality: 30 }, ..Default::default() }).unwrap();
    let again = process(&low.data, &ProcessOptions { mode: CompressMode::Quality { quality: 95 }, ..Default::default() }).unwrap();
    assert!(again.report.kept_original);
    assert_eq!(again.data, low.data);
}

#[test]
fn svg_input_renders_to_png_and_reports_info() {
    let svg = br##"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#2050e0"/></svg>"##;
    let info = probe(svg).unwrap();
    assert_eq!(info.format, "svg");
    assert!(info.decodable);
    assert_eq!((info.width, info.height), (Some(40), Some(20)));

    // 保持原格式 → PNG，按声明尺寸
    let out = process(svg, &ProcessOptions::default()).unwrap();
    assert_eq!(out.report.format, OutputFormat::Png);
    assert_eq!((out.report.width, out.report.height), (40, 20));
    assert_eq!(out.report.original_format, "svg");
    assert!(!out.report.kept_original);

    // 放大时按目标尺寸渲染：边缘依然锐利（没有插值出来的中间色）
    let big = process(
        svg,
        &ProcessOptions { resize: Some(ResizeSpec { width: Some(400), height: None, fit: ResizeFit::Contain, without_enlargement: false }), ..Default::default() },
    )
    .unwrap();
    assert_eq!((big.report.width, big.report.height), (400, 200));
    let img = image::load_from_memory(&big.data).unwrap().to_rgba8();
    assert!(img.pixels().all(|p| p.0 == [0x20, 0x50, 0xe0, 255]));

    // 没有声明尺寸：长边 512
    let icon = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 12"><rect width="24" height="12"/></svg>"#;
    let out = process(icon, &ProcessOptions { format: Some(OutputFormat::Webp), ..Default::default() }).unwrap();
    assert_eq!((out.report.width, out.report.height), (512, 256));

    // 不是合法 SVG
    assert!(matches!(process(b"<svg", &ProcessOptions::default()), Err(Error::Decode(_))));

    let png = encode_as(&sample_image(64, 32, false), ImageFormat::Png);
    let info = probe(&png).unwrap();
    assert_eq!((info.width, info.height), (Some(64), Some(32)));
}

#[test]
fn remove_background_from_jpeg_outputs_transparent_png() {
    // 白底 JPEG，中间一块蓝色方块
    let img = DynamicImage::ImageRgba8(RgbaImage::from_fn(120, 80, |x, y| {
        if (40..80).contains(&x) && (20..60).contains(&y) {
            Rgba([30, 60, 200, 255])
        } else {
            Rgba([255, 255, 255, 255])
        }
    }));
    let src = encode_as(&img, ImageFormat::Jpeg);
    let opts = ProcessOptions {
        remove_background: Some(BackgroundRemoval { trim: true, ..Default::default() }),
        mode: CompressMode::Lossless,
        ..Default::default()
    };
    let out = process(&src, &opts).unwrap();
    assert_eq!(out.report.format, OutputFormat::Png, "JPEG 源默认改为 PNG 以保留透明");
    assert!(!out.report.kept_original);
    let stats = out.report.background.as_ref().unwrap();
    assert!(stats.color.iter().all(|&c| c > 245), "{:?}", stats.color);
    let decoded = image::load_from_memory(&out.data).unwrap().to_rgba8();
    // trim 后只剩蓝色方块（JPEG 边缘有少量噪点，允许 ±3px）
    assert!((40..=46).contains(&decoded.width()) && (40..=46).contains(&decoded.height()), "{:?}", decoded.dimensions());
    let c = decoded.get_pixel(decoded.width() / 2, decoded.height() / 2);
    assert_eq!(c[3], 255);
}

#[test]
fn remove_background_warns_on_busy_background() {
    let src = encode_as(&sample_image(200, 150, false), ImageFormat::Png);
    let opts = ProcessOptions { remove_background: Some(BackgroundRemoval::default()), ..Default::default() };
    let out = process(&src, &opts).unwrap();
    assert!(out.report.warnings.iter().any(|w| w.contains("不是纯色")), "{:?}", out.report.warnings);
}

#[test]
fn scan_flags_large_images_and_respects_gitignore() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::create_dir_all(root.join("dist")).unwrap();
    std::fs::write(root.join(".gitignore"), "dist/\n").unwrap();
    std::fs::write(root.join("assets/big.png"), encode_as(&sample_image(600, 600, false), ImageFormat::Png)).unwrap();
    std::fs::write(root.join("assets/small.png"), encode_as(&sample_image(16, 16, false), ImageFormat::Png)).unwrap();
    std::fs::write(root.join("dist/ignored.png"), encode_as(&sample_image(16, 16, false), ImageFormat::Png)).unwrap();

    let opts = project::ScanOptions {
        policy: project::ScanPolicy { warn_kb: 100, warn_dimension: 500, exclude: vec![] },
        ..Default::default()
    };
    let report = project::scan(root, &opts).unwrap();
    assert_eq!(report.total_images, 2);
    assert_eq!(report.images[0].path, "assets/big.png");
    assert_eq!(report.images[0].issues.len(), 2, "{:?}", report.images[0]);
    assert_eq!(report.flagged_images, 1);
}
