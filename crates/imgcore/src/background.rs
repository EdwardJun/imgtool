//! 去除背景（纯色/近纯色背景抠图）：
//! 1. 背景色：手动指定，或从图片四条边自动识别出现最多的颜色
//! 2. 从四条边开始洪水填充（或全图匹配），颜色距离 ≤ 容差的像素变为全透明，
//!    容差 ~ 容差+柔和度 之间按距离线性过渡为半透明，保证边缘平滑
//! 3. 去色边：半透明像素反混合掉背景色，避免白底抠图后留下白边
//! 4. 可选裁掉四周的透明空白
//! 5. 可选更换底色：把抠出的主体合成到指定纯色上，输出不透明图片（如证件照蓝底/红底）

use std::collections::{HashMap, VecDeque};

use image::{DynamicImage, RgbImage, RgbaImage};
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
    /// 更换底色 [r, g, b]：去除背景后合成到这个纯色上，输出不透明图片（可存 JPEG）；为空时保留透明
    #[serde(default)]
    pub fill: Option<[u8; 3]>,
    /// 人像模式：背景必须能从左、右或上边缘沿直线"看到"（可越过发丝等细小遮挡），
    /// 被主体挡住的同色区域（如西装里的白衬衫、领口）保留；边缘匹配率只看上边和两侧上部
    #[serde(default)]
    pub portrait: bool,
    /// 抠图方式：color 按颜色（默认）/ ai 用 MODNet 人像抠图模型（需先下载模型，忽略容差、柔和、相连、人像模式）
    #[serde(default)]
    pub method: RemovalMethod,
}

/// 抠图方式
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RemovalMethod {
    /// 按颜色：适合纯色背景的产品图、图标、人像
    #[default]
    Color,
    /// AI 人像抠图（MODNet）：背景不必是纯色，发丝边缘更自然，只适合人像
    Ai,
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
            fill: None,
            portrait: false,
            method: RemovalMethod::Color,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemovalStats {
    /// 实际使用的背景色
    pub color: [u8; 3],
    /// 边缘像素中与背景色匹配的比例（0-1），偏低说明背景不是纯色；
    /// 忽略匹配最差的一条边（主体常贴着一条边，如人像肩膀贴底边）
    pub border_match: f64,
    /// 变为透明或半透明的像素比例（0-1）
    pub removed_ratio: f64,
    /// 去除背景后主体（保留下来的像素）的范围，坐标以输出图为准；没有主体时为空
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<SubjectBounds>,
}

/// 主体外接矩形
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SubjectBounds {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// 主体顶部像素的水平中心（识别到头部时取头部，否则取上方 1/4 高度）；不受肩膀不对称影响
    pub top_center_x: u32,
    /// 人像估计：头部宽度（从顶部向下、肩膀明显变宽之前的最大行宽）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_width: Option<u32>,
    /// 人像估计：肩膀开始的行（y 坐标）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shoulder_y: Option<u32>,
}

/// 视为主体的最小不透明度（排除去背景后残留的淡色光晕）
const SUBJECT_ALPHA: u8 = 64;

/// 统计不透明主体的范围；全透明时返回 None
pub fn subject_bounds(img: &RgbaImage) -> Option<SubjectBounds> {
    let (w, h) = img.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] >= SUBJECT_ALPHA {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x0 > x1 || y0 > y1 {
        return None;
    }
    let height = y1 - y0 + 1;

    // 人像：自上而下看每行主体宽度，头部之后行宽突然超过头宽 1.5 倍处视为肩膀。
    // 只统计连续不透明达到 min_run 的片段，忽略图片边缘的细线、JPEG 噪点等
    let min_run = (w.min(h) / 256).max(1) + 2;
    let solid = |x: u32, y: u32| img.get_pixel(x, y)[3] >= SUBJECT_ALPHA;
    let row_width = |y: u32| -> u32 {
        let first_run = |xs: &mut dyn Iterator<Item = u32>| -> Option<u32> {
            let (mut start, mut run) = (0u32, 0u32);
            for x in xs {
                if solid(x, y) {
                    if run == 0 {
                        start = x;
                    }
                    run += 1;
                    if run >= min_run {
                        return Some(start);
                    }
                } else {
                    run = 0;
                }
            }
            None
        };
        let l = first_run(&mut (x0..=x1));
        let r = first_run(&mut (x0..=x1).rev());
        match (l, r) {
            (Some(l), Some(r)) if r >= l => r - l + 1,
            _ => 0,
        }
    };
    // 头宽取到脖子为止：行宽从头部最宽处回落到 85% 以下后不再增长，
    // 这样肩膀逐渐变宽（西装、斜肩）时也能认出来，而不是把肩膀算进头宽
    let (mut head_w, mut shoulder_y, mut past_head) = (0u32, None, false);
    for y in y0..=y1 {
        let rw = row_width(y);
        if head_w > 0 && (y - y0) as f64 >= head_w as f64 * 0.5 && rw as f64 > head_w as f64 * 1.5 {
            shoulder_y = Some(y);
            break;
        }
        if head_w > 0 && (y - y0) as f64 >= head_w as f64 * 0.5 && (rw as f64) < head_w as f64 * 0.85 {
            past_head = true;
        }
        if !past_head {
            head_w = head_w.max(rw);
        }
    }
    let head_width = shoulder_y.map(|_| head_w);

    let top_end = shoulder_y.unwrap_or(y0 + (height / 4).max(1));
    let (mut sum, mut n) = (0u64, 0u64);
    for y in y0..top_end.min(h) {
        for x in x0..=x1 {
            if img.get_pixel(x, y)[3] >= SUBJECT_ALPHA {
                sum += x as u64;
                n += 1;
            }
        }
    }
    let top_center_x = if n > 0 { (sum / n) as u32 } else { (x0 + x1) / 2 };
    Some(SubjectBounds { x: x0, y: y0, width: x1 - x0 + 1, height, top_center_x, head_width, shoulder_y })
}

/// 按透明度把图片合成到纯色底上，输出不透明 RGB 图片
pub fn fill_background(img: &RgbaImage, color: [u8; 3]) -> RgbImage {
    let (w, h) = img.dimensions();
    RgbImage::from_fn(w, h, |x, y| {
        let p = img.get_pixel(x, y);
        let a = p[3] as u32;
        let mix = |c: usize| ((p[c] as u32 * a + color[c] as u32 * (255 - a) + 127) / 255) as u8;
        image::Rgb([mix(0), mix(1), mix(2)])
    })
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

/// 边缘与背景色匹配的比例。主体常会贴着一条边（如人像的肩膀贴底边、产品放在底边上），
/// 所以忽略匹配率最低的那条边，只统计其余三条边
fn border_match(img: &RgbaImage, matches: impl Fn(&[u8; 4]) -> bool) -> f64 {
    let (w, h) = img.dimensions();
    if w < 2 || h < 2 {
        let border = border_pixels(img);
        return if border.is_empty() { 0.0 } else { border.iter().filter(|p| matches(p)).count() as f64 / border.len() as f64 };
    }
    let count = |pts: &mut dyn Iterator<Item = (u32, u32)>| {
        let (mut hit, mut n) = (0usize, 0usize);
        for (x, y) in pts {
            n += 1;
            if matches(&img.get_pixel(x, y).0) {
                hit += 1;
            }
        }
        (hit, n)
    };
    let mut edges = [
        count(&mut (0..w).map(|x| (x, 0))),
        count(&mut (0..w).map(|x| (x, h - 1))),
        count(&mut (0..h).map(|y| (0, y))),
        count(&mut (0..h).map(|y| (w - 1, y))),
    ];
    edges.sort_by(|a, b| (a.0 as f64 / a.1 as f64).total_cmp(&(b.0 as f64 / b.1 as f64)));
    let (hit, n) = edges[1..].iter().fold((0, 0), |acc, e| (acc.0 + e.0, acc.1 + e.1));
    hit as f64 / n.max(1) as f64
}

/// 人像的边缘匹配率：只看上边和左右两边的上半段（肩膀和衣服通常占满下半段和底边），
/// 并向内缩几个像素，避开扫描件 / 截图边缘的细线
fn portrait_border_match(img: &RgbaImage, matches: impl Fn(&[u8; 4]) -> bool) -> f64 {
    let (w, h) = img.dimensions();
    let inset = (w.min(h) / 256).clamp(2, 8);
    if w <= inset * 2 + 1 || h <= inset * 2 + 1 {
        return border_match(img, matches);
    }
    let (l, r, t) = (inset, w - 1 - inset, inset);
    let pts = (l..=r).map(|x| (x, t)).chain((t..h / 2).flat_map(|y| [(l, y), (r, y)]));
    let (mut hit, mut n) = (0usize, 0usize);
    for (x, y) in pts {
        n += 1;
        if matches(&img.get_pixel(x, y).0) {
            hit += 1;
        }
    }
    hit as f64 / n.max(1) as f64
}

/// 人像模式：只保留能从左、右、上边缘沿直线到达的背景。
/// 扫描时可以越过不超过 max_gap 的非背景像素（发丝、边缘细线），
/// 遇到更宽的主体（脸、西装）就停下，所以被主体包住的同色区域（白衬衫、领口）不会被去除
fn keep_only_visible(weight: &mut [f32], w: u32, h: u32) {
    let (wu, hu) = (w as usize, h as usize);
    let max_gap = ((w.min(h) as f32 * 0.008).round() as usize).max(3);
    let mut visible = vec![false; weight.len()];
    let mut scan = |idx: &mut dyn Iterator<Item = usize>, weight: &[f32]| {
        let mut gap = 0usize;
        for i in idx {
            if weight[i] > 0.0 {
                visible[i] = true;
                gap = 0;
            } else {
                gap += 1;
                if gap > max_gap {
                    break;
                }
            }
        }
    };
    for y in 0..hu {
        scan(&mut (0..wu).map(|x| y * wu + x), weight);
        scan(&mut (0..wu).rev().map(|x| y * wu + x), weight);
    }
    for x in 0..wu {
        scan(&mut (0..hu).map(|y| y * wu + x), weight);
    }
    for (wgt, vis) in weight.iter_mut().zip(&visible) {
        if !vis {
            *wgt = 0.0;
        }
    }
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

/// 去除背景，返回带透明通道（或已更换底色）的图片和统计信息。
/// 只有 AI 抠图会失败（模型未下载、推理出错）
pub fn remove_background(img: &DynamicImage, opts: &BackgroundRemoval) -> crate::Result<(DynamicImage, RemovalStats)> {
    let rgba = img.to_rgba8();
    let bg = opts.color.unwrap_or_else(|| detect_background(&rgba));
    let (weight, border_match) = match opts.method {
        RemovalMethod::Color => color_weights(&rgba, bg, opts),
        RemovalMethod::Ai => {
            let mask = crate::matting::matte(img)?;
            let tol = opts.tolerance.min(100) as f32;
            let soft = opts.softness.min(100) as f32;
            let limit = tol + soft;
            let border_match = border_match(&rgba, |p| p[3] < 32 || distance([p[0], p[1], p[2]], bg) <= limit);
            let weight = rgba
                .pixels()
                .zip(mask.pixels())
                .map(|(p, m)| {
                    // 已经透明的像素保持透明
                    if p[3] < 32 {
                        return 1.0;
                    }
                    let ai = 1.0 - m[0] as f32 / 255.0;
                    // 模型在低分辨率上推理，放大后边缘有一圈过渡带，会把原背景色带进新底色形成光晕。
                    // 模型不确定的地方（人像透明度 < 0.5）再按颜色判断一次，像原背景的就算背景；
                    // 模型确定是人像的地方（白衬衫、脸）不看颜色
                    if ai <= 0.5 {
                        return ai;
                    }
                    let d = distance([p[0], p[1], p[2]], bg);
                    let by_color = if d <= tol { 1.0 } else if d < limit && soft > 0.0 { 1.0 - (d - tol) / soft } else { 0.0 };
                    ai.max(by_color)
                })
                .collect();
            (weight, border_match)
        }
    };
    Ok(apply_weights(rgba, &weight, bg, border_match, opts))
}

/// 按颜色计算每个像素的"背景程度"和边缘匹配率
fn color_weights(rgba: &RgbaImage, bg: [u8; 3], opts: &BackgroundRemoval) -> (Vec<f32>, f64) {
    let (w, h) = rgba.dimensions();
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

    let is_match = |p: &[u8; 4]| p[3] < 32 || distance([p[0], p[1], p[2]], bg) <= limit;
    let border_match = if opts.portrait { portrait_border_match(rgba, is_match) } else { border_match(rgba, is_match) };

    let n = (w as usize) * (h as usize);
    let mut weight = vec![0f32; n];
    if opts.portrait {
        // 人像模式：全图按颜色匹配，再由可见性约束决定哪些算背景。
        // 发丝之间的小块背景洪水填充进不去，但沿直线能"看到"，所以也能去掉
        for (i, p) in rgba.pixels().enumerate() {
            weight[i] = bg_weight(p.0);
        }
        keep_only_visible(&mut weight, w, h);
    } else if opts.contiguous {
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
    (weight, border_match)
}

/// 按背景程度设置透明度、去色边，再按需裁空白、更换底色
fn apply_weights(mut rgba: RgbaImage, weight: &[f32], bg: [u8; 3], border_match: f64, opts: &BackgroundRemoval) -> (DynamicImage, RemovalStats) {
    let n = weight.len();
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
    let out_rgba = out.to_rgba8();
    let subject = subject_bounds(&out_rgba);
    if let Some(fill) = opts.fill {
        out = DynamicImage::ImageRgb8(fill_background(&out_rgba, fill));
    }
    let stats = RemovalStats { color: bg, border_match, removed_ratio: removed as f64 / n.max(1) as f64, subject };
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
        let (out, stats) = remove_background(&ring(), &BackgroundRemoval::default()).unwrap();
        assert_eq!(alpha(&out, 0, 0), 0);
        assert_eq!(alpha(&out, 50, 22), 255, "红色主体保留");
        assert_eq!(alpha(&out, 50, 50), 255, "与边缘不相连的内部白色保留");
        assert!(stats.border_match > 0.99);
    }

    #[test]
    fn global_mode_removes_inner_hole() {
        let opts = BackgroundRemoval { contiguous: false, ..Default::default() };
        let (out, _) = remove_background(&ring(), &opts).unwrap();
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
        let (out, _) = remove_background(&img, &opts).unwrap();
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
        let (out, _) = remove_background(&ring(), &opts).unwrap();
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
        let (out, stats) = remove_background(&img, &BackgroundRemoval::default()).unwrap();
        assert_eq!(alpha(&out, 25, 25), 0, "白色区域应被去除");
        assert_eq!(alpha(&out, 50, 50), 255, "图标保留");
        assert!(stats.border_match > 0.99, "{stats:?}");
    }

    #[test]
    fn fill_replaces_background_and_reports_subject() {
        let opts = BackgroundRemoval { fill: Some([67, 142, 219]), ..Default::default() };
        let (out, stats) = remove_background(&ring(), &opts).unwrap();
        assert!(!out.color().has_alpha(), "更换底色后不带透明通道");
        let rgb = out.to_rgb8();
        assert_eq!(rgb.get_pixel(0, 0).0, [67, 142, 219]);
        assert_eq!(rgb.get_pixel(50, 22).0, [220, 30, 30], "主体颜色不变");
        assert_eq!(rgb.get_pixel(50, 50).0, [252, 252, 250], "与边缘不相连的内部白色保留");
        let s = stats.subject.unwrap();
        // 圆环外径 35：主体约 x,y ∈ [15, 85]
        assert!((14..=17).contains(&s.x) && (14..=17).contains(&s.y), "{s:?}");
        assert!((68..=72).contains(&s.width), "{s:?}");
        assert!((49..=51).contains(&s.top_center_x), "{s:?}");
        assert_eq!(s.head_width, None, "圆环没有肩膀，不按人像估计");
    }

    /// 白墙前穿西装白衬衫的"人像"：头部和西装之间有一条与背景同色的缝（领口处），
    /// 普通洪水填充会顺着这条缝把整块白衬衫当成背景
    fn suit_on_white() -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_fn(200, 200, |x, y| {
            let head = (70..130).contains(&x) && (30..100).contains(&y);
            let shirt = (85..115).contains(&x) && y >= 100;
            let jacket = y >= 110 && !shirt;
            // 右边缘 2px 的灰线，模拟扫描 / 截图留下的边框
            let line = x >= 198 && y >= 60;
            match () {
                _ if head => Rgba([210, 160, 130, 255]),
                _ if jacket || line => Rgba([25, 25, 35, 255]),
                _ => Rgba([253, 253, 253, 255]),
            }
        }))
    }

    #[test]
    fn portrait_mode_keeps_enclosed_white_shirt() {
        let white = Some([253, 253, 253]);
        let (plain, _) = remove_background(&suit_on_white(), &BackgroundRemoval { color: white, ..Default::default() }).unwrap();
        assert_eq!(alpha(&plain, 100, 160), 0, "普通模式会顺着领口的缝把衬衫去掉");

        let opts = BackgroundRemoval { color: white, portrait: true, ..Default::default() };
        let (out, stats) = remove_background(&suit_on_white(), &opts).unwrap();
        assert_eq!(alpha(&out, 100, 160), 255, "人像模式保留被西装包住的白衬衫");
        assert_eq!(alpha(&out, 10, 10), 0, "背景照常去除");
        assert_eq!(alpha(&out, 190, 80), 0, "越过右边缘细线的背景也去除");
        assert!(stats.border_match > 0.99, "只看上边和两侧上部，不受西装和细线影响: {stats:?}");
    }

    #[test]
    fn subject_bounds_finds_head_and_shoulders() {
        // 透明底：圆形头部（半径 30，中心 x=100）+ 下方宽 180 的肩膀
        let img = RgbaImage::from_fn(200, 260, |x, y| {
            let head = ((x as f32 - 100.0).powi(2) + (y as f32 - 80.0).powi(2)).sqrt() < 30.0;
            let body = y >= 120 && (10..190).contains(&x);
            Rgba([90, 60, 40, if head || body { 255 } else { 0 }])
        });
        let s = subject_bounds(&img).unwrap();
        assert_eq!(s.shoulder_y, Some(120), "{s:?}");
        assert!((57..=60).contains(&s.head_width.unwrap()), "{s:?}");
        assert!((99..=101).contains(&s.top_center_x), "{s:?}");
    }

    #[test]
    fn noisy_background_reports_low_border_match() {
        let mut seed = 1u32;
        let img = DynamicImage::ImageRgba8(RgbaImage::from_fn(50, 50, |_, _| {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            Rgba([(seed >> 16) as u8, (seed >> 8) as u8, seed as u8, 255])
        }));
        let (_, stats) = remove_background(&img, &BackgroundRemoval::default()).unwrap();
        assert!(stats.border_match < 0.5);
    }
}
