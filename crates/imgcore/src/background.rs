//! 去除背景（纯色/近纯色背景抠图）：
//! 1. 背景色：手动指定，或从图片四条边自动识别出现最多的颜色
//! 2. 从四条边开始洪水填充（或全图匹配），颜色距离 ≤ 容差的像素变为全透明，
//!    容差 ~ 容差+柔和度 之间按距离线性过渡为半透明，保证边缘平滑
//! 3. 去色边：半透明像素反混合掉背景色，避免白底抠图后留下白边
//! 4. 可选裁掉四周的透明空白

use std::collections::{HashMap, VecDeque};

use image::{DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BackgroundRemoval {
    /// 背景色 [r, g, b]；为空时自动识别边缘颜色
    #[serde(default)]
    pub color: Option<[u8; 3]>,
    /// 容差 0-100（颜色距离百分比），越大去除得越多，默认 12
    #[serde(default = "default_tolerance")]
    pub tolerance: u8,
    /// 边缘柔和度 0-100，容差之外这一段距离内的像素做半透明过渡，默认 8
    #[serde(default = "default_softness")]
    pub softness: u8,
    /// 只去除与图片边缘相连的背景（主体内部的同色区域保留），默认 true
    #[serde(default = "default_true")]
    pub contiguous: bool,
    /// 去色边：去掉半透明边缘残留的背景色，默认 true
    #[serde(default = "default_true")]
    pub decontaminate: bool,
    /// 裁掉四周透明空白
    #[serde(default)]
    pub trim: bool,
    /// 裁剪时保留的边距（像素）
    #[serde(default)]
    pub trim_padding: u32,
}

fn default_tolerance() -> u8 {
    12
}
fn default_softness() -> u8 {
    8
}
fn default_true() -> bool {
    true
}

impl Default for BackgroundRemoval {
    fn default() -> Self {
        Self {
            color: None,
            tolerance: default_tolerance(),
            softness: default_softness(),
            contiguous: true,
            decontaminate: true,
            trim: false,
            trim_padding: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemovalStats {
    /// 实际使用的背景色
    pub color: [u8; 3],
    /// 边缘像素中与背景色匹配的比例（0-1），偏低说明背景不是纯色
    pub border_match: f64,
    /// 变为透明或半透明的像素比例（0-1）
    pub removed_ratio: f64,
}

/// RGB 欧氏距离，换算为 0-100
fn distance(a: [u8; 3], b: [u8; 3]) -> f32 {
    let d: f32 = (0..3).map(|i| (a[i] as f32 - b[i] as f32).powi(2)).sum();
    d.sqrt() / 441.672_96 * 100.0
}

fn border_pixels(img: &RgbaImage) -> Vec<[u8; 4]> {
    let (w, h) = img.dimensions();
    let mut out = Vec::with_capacity(((w + h) * 2) as usize);
    for x in 0..w {
        out.push(img.get_pixel(x, 0).0);
        if h > 1 {
            out.push(img.get_pixel(x, h - 1).0);
        }
    }
    for y in 1..h.saturating_sub(1) {
        out.push(img.get_pixel(0, y).0);
        if w > 1 {
            out.push(img.get_pixel(w - 1, y).0);
        }
    }
    out
}

/// 边缘最常见的颜色（按 16 级量化分桶，再对桶内像素取平均）；透明像素不参与。
/// 边缘全透明时（如四周留了透明边的图标），改为统计紧贴透明区域的第一圈不透明像素
pub fn detect_background(img: &RgbaImage) -> [u8; 3] {
    let border = border_pixels(img);
    let samples: Vec<[u8; 4]> = if border.iter().any(|p| p[3] >= 128) { border } else { opaque_rim(img) };
    let mut buckets: HashMap<(u8, u8, u8), (u64, [u64; 3])> = HashMap::new();
    for p in samples {
        if p[3] < 128 {
            continue;
        }
        let key = (p[0] >> 4, p[1] >> 4, p[2] >> 4);
        let e = buckets.entry(key).or_insert((0, [0; 3]));
        e.0 += 1;
        for (sum, &v) in e.1.iter_mut().zip(&p[..3]) {
            *sum += v as u64;
        }
    }
    buckets
        .values()
        .max_by_key(|(n, _)| *n)
        .map(|(n, sum)| [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8])
        .unwrap_or([255, 255, 255])
}

/// 从四条边向内扫描，取每行/每列遇到的第一个不透明像素
fn opaque_rim(img: &RgbaImage) -> Vec<[u8; 4]> {
    let (w, h) = img.dimensions();
    let mut out = Vec::new();
    for y in 0..h {
        if let Some(x) = (0..w).find(|&x| img.get_pixel(x, y)[3] >= 128) {
            out.push(img.get_pixel(x, y).0);
            let xr = (0..w).rev().find(|&x| img.get_pixel(x, y)[3] >= 128).unwrap_or(x);
            out.push(img.get_pixel(xr, y).0);
        }
    }
    for x in 0..w {
        if let Some(y) = (0..h).find(|&y| img.get_pixel(x, y)[3] >= 128) {
            out.push(img.get_pixel(x, y).0);
            let yb = (0..h).rev().find(|&y| img.get_pixel(x, y)[3] >= 128).unwrap_or(y);
            out.push(img.get_pixel(x, yb).0);
        }
    }
    out
}

/// 去除背景，返回带透明通道的图片和统计信息
pub fn remove_background(img: &DynamicImage, opts: &BackgroundRemoval) -> (DynamicImage, RemovalStats) {
    let mut rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let bg = opts.color.unwrap_or_else(|| detect_background(&rgba));
    let tol = opts.tolerance.min(100) as f32;
    let soft = opts.softness.min(100) as f32;
    let limit = tol + soft;

    // 每个像素的"背景程度"：1 = 完全是背景，0 = 不是背景
    // 已经透明的像素视为背景，洪水填充可以从透明边缘穿过去（如四周已有透明留白的图标）
    let bg_weight = |p: [u8; 4]| -> f32 {
        if p[3] < 32 {
            return 1.0;
        }
        let d = distance([p[0], p[1], p[2]], bg);
        if d <= tol {
            1.0
        } else if d < limit && soft > 0.0 {
            1.0 - (d - tol) / soft
        } else {
            0.0
        }
    };

    let border = border_pixels(&rgba);
    let border_match = if border.is_empty() {
        0.0
    } else {
        border.iter().filter(|p| p[3] < 32 || distance([p[0], p[1], p[2]], bg) <= limit).count() as f64 / border.len() as f64
    };

    let n = (w as usize) * (h as usize);
    let mut weight = vec![0f32; n];
    if opts.contiguous {
        // 从边缘出发的 4 邻域洪水填充。只沿"背景程度不升高"的方向扩散：
        // 能穿过整段柔和过渡带，但过渡带之后不会再爬升进入主体内部的同色区域
        let mut visited = vec![false; n];
        let mut queue = VecDeque::new();
        let px = |x: u32, y: u32| rgba.get_pixel(x, y).0;
        let try_seed = |x: u32, y: u32, visited: &mut Vec<bool>, weight: &mut Vec<f32>, queue: &mut VecDeque<(u32, u32)>| {
            let i = (y * w + x) as usize;
            let wgt = bg_weight(px(x, y));
            if !visited[i] && wgt > 0.0 {
                visited[i] = true;
                weight[i] = wgt;
                queue.push_back((x, y));
            }
        };
        for x in 0..w {
            try_seed(x, 0, &mut visited, &mut weight, &mut queue);
            try_seed(x, h - 1, &mut visited, &mut weight, &mut queue);
        }
        for y in 0..h {
            try_seed(0, y, &mut visited, &mut weight, &mut queue);
            try_seed(w - 1, y, &mut visited, &mut weight, &mut queue);
        }
        while let Some((x, y)) = queue.pop_front() {
            let cur = weight[(y * w + x) as usize];
            let neighbors = [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)];
            for (nx, ny) in neighbors {
                if nx >= w || ny >= h {
                    continue;
                }
                let j = (ny * w + nx) as usize;
                if visited[j] {
                    continue;
                }
                let wgt = bg_weight(px(nx, ny));
                if wgt > 0.0 && wgt <= cur + 1e-4 {
                    visited[j] = true;
                    weight[j] = wgt;
                    queue.push_back((nx, ny));
                }
            }
        }
    } else {
        for (i, p) in rgba.pixels().enumerate() {
            weight[i] = bg_weight(p.0);
        }
    }

    let mut removed = 0usize;
    for (i, p) in rgba.pixels_mut().enumerate() {
        let wgt = weight[i];
        if wgt <= 0.0 {
            continue;
        }
        removed += 1;
        let keep = 1.0 - wgt;
        let alpha = (p[3] as f32 * keep).round();
        if opts.decontaminate && keep > 0.0 && keep < 1.0 {
            // 观测色 = 前景 * a + 背景 * (1-a)，反解前景色
            for c in 0..3 {
                let v = (p[c] as f32 - bg[c] as f32 * (1.0 - keep)) / keep;
                p[c] = v.clamp(0.0, 255.0).round() as u8;
            }
        }
        p[3] = alpha as u8;
    }

    let mut out = DynamicImage::ImageRgba8(rgba);
    if opts.trim {
        out = trim_transparent(out, opts.trim_padding);
    }
    let stats = RemovalStats { color: bg, border_match, removed_ratio: removed as f64 / n.max(1) as f64 };
    (out, stats)
}

/// 裁到非透明内容的外接矩形（加边距）；全透明时原样返回
pub fn trim_transparent(img: DynamicImage, padding: u32) -> DynamicImage {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for (x, y, p) in rgba.enumerate_pixels() {
        if p[3] > 8 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x0 > x1 || y0 > y1 {
        return img;
    }
    let x0 = x0.saturating_sub(padding);
    let y0 = y0.saturating_sub(padding);
    let x1 = (x1 + padding).min(w - 1);
    let y1 = (y1 + padding).min(h - 1);
    DynamicImage::ImageRgba8(rgba).crop_imm(x0, y0, x1 - x0 + 1, y1 - y0 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// 白底 + 红色圆环，圆环中间是白色"洞"
    fn ring() -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_fn(100, 100, |x, y| {
            let d = (((x as i32 - 50).pow(2) + (y as i32 - 50).pow(2)) as f32).sqrt();
            if (20.0..35.0).contains(&d) {
                Rgba([220, 30, 30, 255])
            } else {
                Rgba([252, 252, 250, 255])
            }
        }))
    }

    fn alpha(img: &DynamicImage, x: u32, y: u32) -> u8 {
        img.to_rgba8().get_pixel(x, y)[3]
    }

    #[test]
    fn detects_border_color() {
        assert_eq!(detect_background(&ring().to_rgba8()), [252, 252, 250]);
    }

    #[test]
    fn contiguous_keeps_inner_hole() {
        let (out, stats) = remove_background(&ring(), &BackgroundRemoval::default());
        assert_eq!(alpha(&out, 0, 0), 0);
        assert_eq!(alpha(&out, 50, 22), 255, "红色主体保留");
        assert_eq!(alpha(&out, 50, 50), 255, "与边缘不相连的内部白色保留");
        assert!(stats.border_match > 0.99);
    }

    #[test]
    fn global_mode_removes_inner_hole() {
        let opts = BackgroundRemoval { contiguous: false, ..Default::default() };
        let (out, _) = remove_background(&ring(), &opts);
        assert_eq!(alpha(&out, 50, 50), 0);
        assert_eq!(alpha(&out, 50, 22), 255);
    }

    #[test]
    fn soft_edge_is_semi_transparent_and_decontaminated() {
        // 白底到红色的一条横向渐变
        let img = DynamicImage::ImageRgba8(RgbaImage::from_fn(60, 10, |x, _| {
            let t = (x as f32 / 59.0).min(1.0);
            let c = |a: f32, b: f32| (a + (b - a) * t) as u8;
            Rgba([c(255.0, 220.0), c(255.0, 30.0), c(255.0, 30.0), 255])
        }));
        let opts = BackgroundRemoval { color: Some([255, 255, 255]), tolerance: 5, softness: 30, ..Default::default() };
        let (out, _) = remove_background(&img, &opts);
        let rgba = out.to_rgba8();
        let alphas: Vec<u8> = (0..60).map(|x| rgba.get_pixel(x, 5)[3]).collect();
        assert_eq!(alphas[0], 0);
        assert_eq!(alphas[59], 255);
        assert!(alphas.iter().filter(|&&a| a > 0 && a < 255).count() >= 3, "过渡带有多个半透明像素: {alphas:?}");
        assert!(alphas.windows(2).all(|p| p[0] <= p[1]), "透明度单调过渡: {alphas:?}");
        // 去色边后半透明像素的颜色应偏向红色而不是发白
        let semi = (0..60).find(|&x| (40..200).contains(&rgba.get_pixel(x, 5)[3])).unwrap();
        assert!(rgba.get_pixel(semi, 5)[1] < 200);
    }

    #[test]
    fn trim_crops_to_content() {
        let opts = BackgroundRemoval { trim: true, trim_padding: 2, ..Default::default() };
        let (out, _) = remove_background(&ring(), &opts);
        // 圆环外径 35 → 内容约 70px，再加 2px 边距
        assert!((70..=76).contains(&out.width()), "{}", out.width());
        assert_eq!(out.width(), out.height());
    }

    #[test]
    fn removes_white_box_inside_transparent_border() {
        // 模拟从白底设计稿里切出的图标：四周 20px 透明留白，中间白底 + 灰色方块
        let img = DynamicImage::ImageRgba8(RgbaImage::from_fn(100, 100, |x, y| {
            let inside = (20..80).contains(&x) && (20..80).contains(&y);
            let icon = (40..60).contains(&x) && (40..60).contains(&y);
            match (inside, icon) {
                (_, true) => Rgba([40, 40, 40, 255]),
                (true, false) => Rgba([254, 254, 254, 255]),
                _ => Rgba([0, 0, 0, 0]),
            }
        }));
        assert_eq!(detect_background(&img.to_rgba8()), [254, 254, 254]);
        let (out, stats) = remove_background(&img, &BackgroundRemoval::default());
        assert_eq!(alpha(&out, 25, 25), 0, "白色区域应被去除");
        assert_eq!(alpha(&out, 50, 50), 255, "图标保留");
        assert!(stats.border_match > 0.99, "{stats:?}");
    }

    #[test]
    fn noisy_background_reports_low_border_match() {
        let mut seed = 1u32;
        let img = DynamicImage::ImageRgba8(RgbaImage::from_fn(50, 50, |_, _| {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            Rgba([(seed >> 16) as u8, (seed >> 8) as u8, seed as u8, 255])
        }));
        let (_, stats) = remove_background(&img, &BackgroundRemoval::default());
        assert!(stats.border_match < 0.5);
    }
}
