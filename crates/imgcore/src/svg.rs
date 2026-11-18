//! SVG：解析、渲染成位图、按白名单清洗。
//!
//! 安全约束：渲染时不读取任何外部文件或网络资源（`<image href="/etc/...">`、相对路径都会被忽略），
//! 只允许内嵌的 `data:` 图片；不执行脚本（resvg 本身不支持脚本）。
//! 写出 SVG 文件前统一经过 [`sanitize`]，去掉脚本、事件属性、外链等。

use std::sync::{Arc, OnceLock};

use image::{DynamicImage, RgbaImage};
use resvg::{tiny_skia, usvg};

use crate::error::{Error, Result};

/// SVG 没有声明 width/height 时，按长边这么多像素渲染
pub const DEFAULT_RENDER_EDGE: u32 = 512;
/// 渲染尺寸上限（长边），防止声明超大尺寸的 SVG 耗尽内存
pub const MAX_RENDER_EDGE: u32 = 8192;
/// 输入 SVG 文本上限
pub const MAX_SVG_BYTES: usize = 5 * 1024 * 1024;

/// 解析后的 SVG
pub struct SvgDoc {
    tree: usvg::Tree,
    /// 根元素声明了 width 和 height
    has_size: bool,
}

impl SvgDoc {
    /// 文档的宽高（像素，可能是小数）
    pub fn size(&self) -> (f32, f32) {
        let s = self.tree.size();
        (s.width(), s.height())
    }

    /// 默认渲染尺寸：有声明尺寸时用声明尺寸，否则长边 512；均不超过 8192
    pub fn intrinsic_size(&self) -> (u32, u32) {
        let (w, h) = self.size();
        let scale = if self.has_size { 1.0 } else { DEFAULT_RENDER_EDGE as f32 / w.max(h) };
        fit_edge(w * scale, h * scale, MAX_RENDER_EDGE)
    }

    /// 等比渲染，长边至少为 `min_long_edge`（矢量放大不失真）
    pub fn render_default(&self, min_long_edge: Option<u32>) -> RgbaImage {
        let (mut w, mut h) = self.intrinsic_size();
        if let Some(min) = min_long_edge.filter(|m| *m > w.max(h)) {
            let scale = min.min(MAX_RENDER_EDGE) as f32 / w.max(h) as f32;
            (w, h) = fit_edge(w as f32 * scale, h as f32 * scale, MAX_RENDER_EDGE);
        }
        self.render_fit(w, h)
    }

    /// 等比缩放到 width×height 的框内居中渲染，其余透明
    pub fn render_fit(&self, width: u32, height: u32) -> RgbaImage {
        let (width, height) = (width.clamp(1, MAX_RENDER_EDGE), height.clamp(1, MAX_RENDER_EDGE));
        let (sw, sh) = self.size();
        let scale = (width as f32 / sw).min(height as f32 / sh);
        let dx = (width as f32 - sw * scale) / 2.0;
        let dy = (height as f32 - sh * scale) / 2.0;
        let mut pixmap = tiny_skia::Pixmap::new(width, height).expect("尺寸已限制在合法范围");
        resvg::render(&self.tree, tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy), &mut pixmap.as_mut());
        // tiny-skia 是预乘 alpha，转成普通 RGBA
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for p in pixmap.pixels() {
            let c = p.demultiply();
            out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        RgbaImage::from_raw(width, height, out).expect("缓冲区长度匹配")
    }
}

fn fit_edge(w: f32, h: f32, max_edge: u32) -> (u32, u32) {
    let long = w.max(h).max(1.0);
    let scale = if long > max_edge as f32 { max_edge as f32 / long } else { 1.0 };
    ((w * scale).round().max(1.0) as u32, (h * scale).round().max(1.0) as u32)
}

/// 系统字体只在 SVG 里有文字时才加载（首次约几十到几百毫秒）
fn system_fonts() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

/// 解析 SVG（支持 .svgz）
pub fn parse(data: &[u8]) -> Result<SvgDoc> {
    if data.len() > MAX_SVG_BYTES {
        return Err(Error::InvalidArgument(format!("SVG 超过 {} MB", MAX_SVG_BYTES / 1024 / 1024)));
    }
    let mut opt = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            // 不加载任何外部文件 / 地址
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let text = String::from_utf8_lossy(&data[..data.len().min(MAX_SVG_BYTES)]);
    if text.contains("<text") || text.contains(":text") {
        opt.fontdb = system_fonts();
    }
    let tree = usvg::Tree::from_data(data, &opt).map_err(|e| Error::Decode(format!("SVG 解析失败: {e}")))?;
    Ok(SvgDoc { tree, has_size: root_has_size(data) })
}

fn root_has_size(data: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(data) else { return true };
    match roxmltree::Document::parse(text) {
        Ok(doc) => {
            let root = doc.root_element();
            root.attribute("width").is_some() && root.attribute("height").is_some()
        }
        // gzip 等无法直接读取时按 usvg 的结果处理
        Err(_) => true,
    }
}

/// 解码成位图，供压缩 / 转换 / 裁剪流水线使用
pub fn rasterize(data: &[u8], min_long_edge: Option<u32>) -> Result<DynamicImage> {
    Ok(DynamicImage::ImageRgba8(parse(data)?.render_default(min_long_edge)))
}

/// 读取 SVG 尺寸（按默认渲染尺寸）
pub fn dimensions(data: &[u8]) -> Option<(u32, u32)> {
    parse(data).ok().map(|d| d.intrinsic_size())
}

// ---------------------------------------------------------------- 清洗

const ALLOWED_ELEMENTS: &[&str] = &[
    "svg", "g", "path", "circle", "ellipse", "line", "polyline", "polygon", "rect", "defs", "use", "symbol",
    "linearGradient", "radialGradient", "stop", "clipPath", "mask", "text", "tspan",
];

const ALLOWED_ATTRS: &[&str] = &[
    // 结构与几何
    "id", "viewBox", "width", "height", "x", "y", "x1", "y1", "x2", "y2", "cx", "cy", "r", "rx", "ry", "d", "points",
    "transform", "preserveAspectRatio", "href",
    // 渐变 / 裁剪 / 遮罩
    "offset", "gradientUnits", "gradientTransform", "spreadMethod", "fx", "fy", "fr", "clipPathUnits", "maskUnits",
    "maskContentUnits", "clip-path", "mask", "clip-rule",
    // 外观
    "fill", "fill-opacity", "fill-rule", "stroke", "stroke-width", "stroke-linecap", "stroke-linejoin",
    "stroke-miterlimit", "stroke-dasharray", "stroke-dashoffset", "stroke-opacity", "opacity", "color", "stop-color",
    "stop-opacity", "visibility", "display", "vector-effect", "shape-rendering", "paint-order", "style",
    // 文字
    "font-family", "font-size", "font-weight", "font-style", "text-anchor", "dominant-baseline", "letter-spacing",
    "dx", "dy",
];

/// 清洗选项
#[derive(Debug, Clone, Default)]
pub struct SanitizeOptions {
    /// 设置根元素的 color（图标用 currentColor 时即为线条 / 填充颜色），如 "#4F46E5"
    pub color: Option<String>,
}

/// 清洗结果
#[derive(Debug, Clone)]
pub struct Sanitized {
    pub svg: String,
    /// 被移除的元素 / 属性名（去重），便于提示
    pub removed: Vec<String>,
}

/// 按白名单重建 SVG：只保留图形元素和外观属性；去掉脚本、事件、foreignObject、外部引用、
/// 注释、元数据和 `<style>`。`href` 只允许指向文档内（`#id`），`url(...)` 只允许 `url(#id)`。
pub fn sanitize(text: &str, opts: &SanitizeOptions) -> Result<Sanitized> {
    if text.len() > MAX_SVG_BYTES {
        return Err(Error::InvalidArgument(format!("SVG 超过 {} MB", MAX_SVG_BYTES / 1024 / 1024)));
    }
    let doc = roxmltree::Document::parse_with_options(text, roxmltree::ParsingOptions { allow_dtd: false, ..Default::default() })
        .map_err(|e| Error::Decode(format!("SVG 不是合法的 XML: {e}")))?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err(Error::Decode("根元素不是 <svg>".into()));
    }
    let mut out = String::with_capacity(text.len());
    let mut removed = Vec::new();
    write_node(root, &mut out, &mut removed, true, opts);
    removed.sort();
    removed.dedup();
    // 清洗后的结果必须能被渲染器解析
    parse(out.as_bytes())?;
    Ok(Sanitized { svg: out, removed })
}

fn write_node(node: roxmltree::Node, out: &mut String, removed: &mut Vec<String>, is_root: bool, opts: &SanitizeOptions) {
    let name = node.tag_name().name();
    let svg_ns = matches!(node.tag_name().namespace(), None | Some("http://www.w3.org/2000/svg"));
    if !svg_ns || !ALLOWED_ELEMENTS.contains(&name) {
        removed.push(format!("<{name}>"));
        return;
    }
    out.push('<');
    out.push_str(name);
    if is_root {
        out.push_str(r#" xmlns="http://www.w3.org/2000/svg""#);
    }
    for attr in node.attributes() {
        let an = attr.name();
        let value = attr.value().trim();
        // xlink:href 统一写成 href
        let ns_ok = match attr.namespace() {
            None => true,
            Some("http://www.w3.org/1999/xlink") => an == "href",
            Some(_) => false,
        };
        // data-* 无害但无用（如生成时的 data-index），直接丢弃且不计入提示
        if (is_root && an == "color" && opts.color.is_some()) || an.starts_with("data-") {
            continue;
        }
        let allowed = ns_ok && ALLOWED_ATTRS.contains(&an) && attr_value_safe(an, value);
        if !allowed {
            removed.push(format!("@{an}"));
            continue;
        }
        out.push(' ');
        out.push_str(an);
        out.push_str("=\"");
        escape_into(value, out);
        out.push('"');
    }
    if is_root {
        if let Some(c) = &opts.color {
            out.push_str(" color=\"");
            escape_into(c, out);
            out.push('"');
        }
    }
    let mut body = String::new();
    for child in node.children() {
        if child.is_element() {
            write_node(child, &mut body, removed, false, opts);
        } else if child.is_text() && matches!(name, "text" | "tspan") {
            escape_into(child.text().unwrap_or(""), &mut body);
        }
    }
    if body.is_empty() {
        out.push_str("/>");
    } else {
        out.push('>');
        out.push_str(&body);
        out.push_str("</");
        out.push_str(name);
        out.push('>');
    }
}

fn attr_value_safe(name: &str, value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if lower.contains("javascript:") || lower.contains("expression(") || lower.contains("@import") {
        return false;
    }
    if name == "href" {
        return value.starts_with('#');
    }
    // url(...) 只允许引用文档内的渐变 / 裁剪等
    let mut rest = lower.as_str();
    while let Some(i) = rest.find("url(") {
        let after = rest[i + 4..].trim_start().trim_start_matches(['"', '\'']);
        if !after.starts_with('#') {
            return false;
        }
        rest = &rest[i + 4..];
    }
    true
}

fn escape_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// 把多份 SVG 排成一张白底预览图（每行最多 `per_row` 个，每格 cell×cell），无法解析的格子留空
pub fn contact_sheet(svgs: &[&[u8]], cell: u32, per_row: usize) -> RgbaImage {
    let cell = cell.clamp(16, 512);
    let per_row = per_row.max(1);
    let gap = cell / 4;
    let cols = svgs.len().clamp(1, per_row) as u32;
    let rows = svgs.len().div_ceil(per_row).max(1) as u32;
    let (w, h) = (cols * cell + (cols + 1) * gap, rows * cell + (rows + 1) * gap);
    let mut sheet = RgbaImage::from_pixel(w, h, image::Rgba([255, 255, 255, 255]));
    for (i, data) in svgs.iter().enumerate() {
        let Ok(doc) = parse(data) else { continue };
        let tile = doc.render_fit(cell, cell);
        let (c, r) = ((i % per_row) as u32, (i / per_row) as u32);
        image::imageops::overlay(&mut sheet, &tile, (gap + c * (cell + gap)) as i64, (gap + r * (cell + gap)) as i64);
    }
    sheet
}

/// 一份 SVG 渲染成多个正方形尺寸后的结果
#[derive(Debug, Clone)]
pub struct RenderedSize {
    pub size: u32,
    pub image: RgbaImage,
}

/// 把 SVG 渲染成多个 size×size 的正方形位图（等比居中，四周透明）
pub fn render_square_sizes(data: &[u8], sizes: &[u32]) -> Result<Vec<RenderedSize>> {
    if sizes.is_empty() {
        return Err(Error::InvalidArgument("至少需要一个输出尺寸".into()));
    }
    let doc = parse(data)?;
    sizes
        .iter()
        .map(|&s| {
            if !(8..=4096).contains(&s) {
                return Err(Error::InvalidArgument(format!("输出尺寸 {s} 超出范围 8-4096")));
            }
            Ok(RenderedSize { size: s, image: doc.render_fit(s, s) })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M3 10l9-7 9 7v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z"/></svg>"#;

    #[test]
    fn default_size_when_undeclared() {
        let doc = parse(ICON.as_bytes()).unwrap();
        assert_eq!(doc.intrinsic_size(), (512, 512));
        let sized = parse(br#"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="24"><rect width="48" height="24"/></svg>"#).unwrap();
        assert_eq!(sized.intrinsic_size(), (48, 24));
        assert_eq!(sized.render_default(Some(96)).dimensions(), (96, 48), "矢量按需放大");
        let huge = parse(br#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="50000"/>"#).unwrap();
        assert_eq!(huge.intrinsic_size(), (MAX_RENDER_EDGE, MAX_RENDER_EDGE / 2));
    }

    #[test]
    fn renders_with_color_and_transparency() {
        let s = sanitize(ICON, &SanitizeOptions { color: Some("#ff0000".into()) }).unwrap();
        let out = render_square_sizes(s.svg.as_bytes(), &[24, 128]).unwrap();
        assert_eq!(out[1].image.dimensions(), (128, 128));
        let img = &out[1].image;
        assert_eq!(img.get_pixel(0, 0).0[3], 0, "背景透明");
        let red = img.pixels().filter(|p| p.0[3] > 200 && p.0[0] > 200 && p.0[1] < 40).count();
        assert!(red > 500, "线条按 color 着色: {red}");
    }

    #[test]
    fn sanitize_strips_dangerous_content() {
        let evil = r##"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 10 10" onload="alert(1)">
  <!-- 注释 --><metadata>m</metadata><style>rect{fill:red}</style>
  <script>alert(1)</script>
  <foreignObject><div/></foreignObject>
  <image href="file:///etc/passwd" width="10" height="10"/>
  <a href="javascript:alert(1)"><rect width="5" height="5"/></a>
  <defs><linearGradient id="g"><stop offset="0" stop-color="#fff"/></linearGradient></defs>
  <rect width="10" height="10" fill="url(#g)" style="stroke:url(http://x/y)" onclick="x()"/>
  <use xlink:href="#g"/><use href="http://evil/a.svg#x"/>
</svg>"##;
        let s = sanitize(evil, &SanitizeOptions::default()).unwrap();
        for bad in ["script", "onload", "onclick", "foreignObject", "image", "javascript", "http://evil", "http://x", "/etc/passwd", "style=", "<a", "metadata", "<!--"] {
            assert!(!s.svg.contains(bad), "{bad} 应被移除: {}", s.svg);
        }
        assert!(s.svg.contains(r##"fill="url(#g)""##) && s.svg.contains(r##"<use href="#g"/>"##), "{}", s.svg);
        assert!(s.removed.iter().any(|r| r == "<script>"));
    }

    #[test]
    fn external_images_are_not_loaded() {
        // 指向本地文件的 <image> 渲染时被忽略，不会读文件
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="/etc/hosts" width="10" height="10"/></svg>"#;
        let img = rasterize(svg, None).unwrap().to_rgba8();
        assert!(img.pixels().all(|p| p.0[3] == 0));
    }

    #[test]
    fn rejects_non_svg() {
        assert!(sanitize("<html/>", &SanitizeOptions::default()).is_err());
        assert!(sanitize("not xml", &SanitizeOptions::default()).is_err());
        assert!(sanitize(r#"<!DOCTYPE svg [<!ENTITY x "y">]><svg xmlns="http://www.w3.org/2000/svg"/>"#, &SanitizeOptions::default()).is_err());
    }
}

#[cfg(test)]
mod sheet_tests {
    use super::*;

    #[test]
    fn contact_sheet_layout() {
        let a = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10"/></svg>"#;
        let sheet = contact_sheet(&[a, a, a, b"bad"], 64, 3);
        // 3 列 2 行，间距 16
        assert_eq!(sheet.dimensions(), (3 * 64 + 4 * 16, 2 * 64 + 3 * 16));
        assert_eq!(sheet.get_pixel(16 + 32, 16 + 32).0, [0, 0, 0, 255]);
        assert_eq!(sheet.get_pixel(5, 5).0, [255, 255, 255, 255]);
    }
}
