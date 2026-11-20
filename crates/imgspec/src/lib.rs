//! imgspec：设计稿解析。把 UI 设计稿（整页）或单个样式（卡片、按钮……）转换成 Agent 可读的结构化描述：
//! 嵌套节点树、像素坐标（绝对 + 相对父节点）、填充/渐变/描边/圆角/阴影、文字样式估算、推断的自动布局。
//!
//! 纯本地算法（色块分层 + 递归 XY-cut），不做 OCR、不调用大模型；语义（这是什么、写了什么字）
//! 由 Agent 看编号标注图后用 [`store::apply_annotations`] 写回，或在桌面端手动标注。
//! 图标节点会自动裁出透明底素材，也可以按 [`icons::plan_icon_sheet`] 交给 AI 一次重绘成高清版。

// 像素与颜色通道的计算用下标更直观
#![allow(clippy::needless_range_loop)]

pub mod canvas;
pub mod color;
mod group;
pub mod icons;
pub mod layout;
pub mod model;
pub mod outline;
pub mod overlay;
mod pieces;
mod segment;
pub mod store;
mod style;

#[doc(hidden)]
pub mod fixtures;

use std::path::Path;

use image::DynamicImage;
use serde::{Deserialize, Serialize};

pub use icons::{IconFile, IconOptions, IconSheetPlan};
pub use model::*;
pub use store::{spec_paths, Annotation, SpecPaths};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("spec 文件格式错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("图片处理失败: {0}")]
    Image(String),
    #[error("{0}")]
    Invalid(String),
    #[error("找不到这些节点编号: {}", .0.join("、"))]
    UnknownIds(Vec<String>),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalyzeOptions {
    /// page / component；为空时自动判断（画面主体是单个色块时按 component）
    pub mode: Option<SpecKind>,
    /// 与底色的容差 0-100；为空时按背景噪点自动估算（约 2）
    pub tolerance: Option<u8>,
    /// 最大嵌套层数，默认 6
    pub max_depth: usize,
    /// 长边超过它时先缩小再分析（坐标会换算回原图），默认 2560
    pub max_edge: u32,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self { mode: None, tolerance: None, max_depth: 6, max_edge: 2560 }
    }
}

fn rescale(n: &mut Node, k: f32) {
    n.bounds = n.bounds.scaled(k);
    if let Some(s) = n.style.stroke.as_mut() {
        s.width = ((s.width as f32 * k).round() as u32).max(1);
    }
    if let Some(r) = n.style.corner_radius.as_mut() {
        *r = match r {
            CornerRadius::Uniform(v) => CornerRadius::Uniform((*v as f32 * k).round() as u32),
            CornerRadius::Corners(c) => CornerRadius::Corners(c.map(|v| (v as f32 * k).round() as u32)),
        };
    }
    if let Some(s) = n.style.shadow.as_mut() {
        s.x = (s.x as f32 * k).round() as i32;
        s.y = (s.y as f32 * k).round() as i32;
        s.blur = (s.blur as f32 * k).round() as u32;
    }
    if let Some(t) = n.text_style.as_mut() {
        t.font_size = ((t.font_size as f32 * k).round() as u32).max(1);
        t.line_height = t.line_height.map(|v| (v as f32 * k).round() as u32);
    }
    for c in &mut n.children {
        rescale(c, k);
    }
}

fn palette(cv: &canvas::Canvas) -> Vec<PaletteEntry> {
    let total = cv.w as usize * cv.h as usize;
    let step = (total / 200_000).max(1);
    let mut map: std::collections::HashMap<u32, (u32, [f64; 3])> = Default::default();
    let mut n = 0u32;
    for i in (0..total).step_by(step) {
        let c = cv.get((i % cv.w as usize) as u32, (i / cv.w as usize) as u32);
        let e = map.entry(color::bucket(c, 4)).or_insert((0, [0.0; 3]));
        e.0 += 1;
        for k in 0..3 {
            e.1[k] += c[k] as f64;
        }
        n += 1;
    }
    let mut v: Vec<(u32, [u8; 3])> = map.into_values().map(|(c, s)| (c, s.map(|x| (x / c as f64).round() as u8))).collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut out: Vec<(u32, [u8; 3])> = Vec::new();
    for (c, col) in v {
        match out.iter_mut().find(|(_, o)| color::delta_e(*o, col) < 10.0) {
            Some(o) => o.0 += c,
            None => out.push((c, col)),
        }
    }
    out.sort_by_key(|e| std::cmp::Reverse(e.0));
    out.into_iter()
        .filter(|(c, _)| *c as f32 / n as f32 >= 0.01)
        .take(8)
        .map(|(c, col)| PaletteEntry { color: color::hex(col), ratio: ((c as f32 / n as f32) * 1000.0).round() / 1000.0 })
        .collect()
}

fn is_box(n: &Node) -> bool {
    matches!(n.node_type, NodeType::Frame | NodeType::Rect) && !n.style.is_empty()
}

/// 解析一张图（source_path 只记录在 spec 里）
pub fn analyze(img: &DynamicImage, source_path: &str, opts: &AnalyzeOptions) -> Spec {
    let (cv, scale) = canvas::Canvas::from_image(img, opts.max_edge.max(256));
    let full = cv.full();
    let mut ring = Vec::new();
    for d in 0..3 {
        ring.extend(cv.ring(&full, d, 0.0));
    }
    let fit = canvas::fit(&ring, &full);
    let (bg, noise) = match fit {
        Some(f) => (f.bg, f.rms),
        None => (canvas::Bg::Solid([255.0; 3]), 0.0),
    };
    let user = opts.tolerance.map(|t| t.min(100) as f32 / 100.0 * 441.67);
    let thr = user.unwrap_or_else(|| (3.0 * noise).clamp(8.0, 30.0));
    let ctx = segment::Ctx { cv: &cv, thr, max_depth: opts.max_depth.clamp(1, 12), long: cv.w.max(cv.h) as f32, short: cv.w.min(cv.h) as f32 };
    let raw = segment::analyze_raw(&ctx, full, &bg, None, 0);
    let component = match opts.mode {
        Some(SpecKind::Component) => true,
        Some(SpecKind::Page) => false,
        None => raw.len() == 1 && is_box(&raw[0]) && (raw[0].bounds.area() as f32) < 0.9 * full.area() as f32,
    };
    let mut root = if component && raw.len() == 1 && raw[0].node_type != NodeType::Text {
        raw.into_iter().next().unwrap()
    } else if component {
        let (kids, mode) = group::xy_cut(raw);
        let mut g = Node::new(NodeType::Frame, group::union_all(&kids).unwrap_or(full));
        g.layout = Some(group::placeholder(mode));
        g.children = kids;
        g
    } else {
        let (kids, mode) = group::xy_cut(raw);
        let mut r = Node::new(NodeType::Frame, full);
        if !cv.transparent_border {
            r.style.fill = Some(bg.to_fill(&full));
        }
        r.layout = Some(group::placeholder(mode));
        r.children = kids;
        r
    };
    if scale < 1.0 {
        rescale(&mut root, 1.0 / scale);
        let (w, h) = (img.width(), img.height());
        root.walk_mut(&mut |n| {
            n.bounds.width = n.bounds.width.min(w.saturating_sub(n.bounds.x)).max(1);
            n.bounds.height = n.bounds.height.min(h.saturating_sub(n.bounds.y)).max(1);
        });
    }
    layout::finalize(&mut root);
    layout::assign_ids(&mut root);
    let background = (!cv.transparent_border).then(|| bg.to_fill(&full));
    Spec {
        version: SPEC_VERSION,
        generator: concat!("imgspec ", env!("CARGO_PKG_VERSION")).to_string(),
        source: SourceInfo { path: source_path.to_string(), width: img.width(), height: img.height() },
        kind: if component { SpecKind::Component } else { SpecKind::Page },
        background,
        transparent: cv.transparent_border,
        palette: palette(&cv),
        annotated: false,
        notes: None,
        root,
    }
}

/// 重新分析图中的一块区域（桌面端修正框之后刷新样式）。返回该区域的节点（含子节点），坐标为原图坐标
pub fn analyze_region(img: &DynamicImage, b: Bounds) -> Result<Node> {
    if b.width == 0 || b.height == 0 || b.x >= img.width() || b.y >= img.height() {
        return Err(Error::Invalid("区域超出图片范围".into()));
    }
    let b = Bounds::new(b.x, b.y, b.width.min(img.width() - b.x), b.height.min(img.height() - b.y));
    let m = (b.width.min(b.height) / 4).clamp(8, 64);
    let x0 = b.x.saturating_sub(m);
    let y0 = b.y.saturating_sub(m);
    let x1 = (b.right() + m).min(img.width());
    let y1 = (b.bottom() + m).min(img.height());
    let crop = img.crop_imm(x0, y0, x1 - x0, y1 - y0);
    let spec = analyze(&crop, "", &AnalyzeOptions { mode: Some(SpecKind::Component), ..Default::default() });
    let mut node = spec.root;
    if node.is_plain_group() && node.children.len() == 1 {
        node = node.children.pop().unwrap();
    }
    node.walk_mut(&mut |n| {
        n.bounds.x += x0;
        n.bounds.y += y0;
    });
    if node.bounds.iou(&b) < 0.5 {
        node.bounds = b;
    }
    layout::finalize(&mut node);
    Ok(node)
}

/// 一次完整解析的参数
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunOptions {
    pub analyze: AnalyzeOptions,
    /// 已有 spec 时保留旧的语义标注，默认 true
    pub keep_annotations: bool,
    /// 自动裁出图标素材，默认 true
    pub export_icons: bool,
    pub icons: IconOptions,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self { analyze: AnalyzeOptions::default(), keep_annotations: true, export_icons: true, icons: IconOptions::default() }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    pub spec: Spec,
    /// 相对项目根目录
    pub spec_path: String,
    pub overlay_path: String,
    pub icons: Vec<IconFile>,
    /// 从旧 spec 保留下来的标注数
    pub kept_annotations: usize,
}

pub fn decode_file(path: &Path) -> Result<DynamicImage> {
    let data = std::fs::read(path)?;
    Ok(imgcore::decode::decode(&data).map_err(|e| Error::Image(e.to_string()))?.image)
}

/// 标注图的最长边
pub const OVERLAY_MAX_EDGE: u32 = 1600;

pub fn write_overlay(img: &DynamicImage, spec: &Spec, paths: &SpecPaths) -> Result<()> {
    let ov = overlay::render(img, spec, OVERLAY_MAX_EDGE);
    let mut buf = std::io::Cursor::new(Vec::new());
    ov.write_to(&mut buf, image::ImageFormat::Png).map_err(|e| Error::Image(e.to_string()))?;
    imgcore::project::write_atomic(&paths.overlay, &buf.into_inner()).map_err(|e| Error::Invalid(e.to_string()))
}

/// 解析 → 保留旧标注 → 裁出图标 → 写 spec 与标注图
pub fn analyze_file(paths: &SpecPaths, opts: &RunOptions) -> Result<RunResult> {
    let img = decode_file(&paths.image)?;
    let mut spec = analyze(&img, &store::rel(&paths.root, &paths.image), &opts.analyze);
    let previous = paths.spec.is_file().then(|| store::load(&paths.spec).ok()).flatten();
    let kept = match (&previous, opts.keep_annotations) {
        (Some(old), true) => store::merge_previous(&mut spec, old),
        _ => 0,
    };
    let icons = if opts.export_icons { icons::export_icons(&img, &mut spec, paths, &opts.icons, previous.as_ref())? } else { Vec::new() };
    store::save(&spec, &paths.spec)?;
    write_overlay(&img, &spec, paths)?;
    Ok(RunResult { spec, spec_path: store::rel(&paths.root, &paths.spec), overlay_path: store::rel(&paths.root, &paths.overlay), icons, kept_annotations: kept })
}
