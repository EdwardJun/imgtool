//! 编号标注图：在原图上按类型着色画出每个节点的框，并在左上角写上编号（与节点 id 的数字一致），
//! 方便 Agent 看图后按编号补充语义。

use image::{DynamicImage, Rgba, RgbaImage};

use crate::model::{Bounds, Node, NodeType, Spec};

/// 5×7 点阵数字
const DIGITS: [[u8; 7]; 10] = [
    [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
    [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
    [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
    [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
    [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
    [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
    [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
    [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
    [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
    [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
];

pub fn type_color(t: NodeType, group: bool) -> [u8; 3] {
    if group {
        return [140, 140, 140];
    }
    match t {
        NodeType::Frame | NodeType::Rect => [22, 119, 255],
        NodeType::Text => [56, 158, 13],
        NodeType::Icon => [250, 140, 22],
        NodeType::Image => [114, 46, 209],
        NodeType::Line => [19, 194, 194],
    }
}

fn blend(img: &mut RgbaImage, x: i64, y: i64, c: [u8; 3], a: f32) {
    if x < 0 || y < 0 || x >= img.width() as i64 || y >= img.height() as i64 {
        return;
    }
    let p = img.get_pixel_mut(x as u32, y as u32);
    for i in 0..3 {
        p[i] = (p[i] as f32 * (1.0 - a) + c[i] as f32 * a).round() as u8;
    }
    p[3] = 255;
}

#[allow(clippy::too_many_arguments)]
fn rect_outline(img: &mut RgbaImage, x0: i64, y0: i64, x1: i64, y1: i64, w: i64, c: [u8; 3], dashed: bool) {
    for t in 0..w {
        for x in x0..=x1 {
            if dashed && (x / 4) % 2 == 1 {
                continue;
            }
            blend(img, x, y0 + t, c, 1.0);
            blend(img, x, y1 - t, c, 1.0);
        }
        for y in y0..=y1 {
            if dashed && (y / 4) % 2 == 1 {
                continue;
            }
            blend(img, x0 + t, y, c, 1.0);
            blend(img, x1 - t, y, c, 1.0);
        }
    }
}

fn label(img: &mut RgbaImage, x: i64, y: i64, text: &str, c: [u8; 3], s: i64) {
    let w = text.len() as i64 * 6 * s + s * 2;
    let h = 7 * s + s * 2;
    for yy in 0..h {
        for xx in 0..w {
            blend(img, x + xx, y + yy, c, 0.9);
        }
    }
    for (k, ch) in text.chars().enumerate() {
        let Some(d) = ch.to_digit(10) else { continue };
        for (row, bits) in DIGITS[d as usize].iter().enumerate() {
            for col in 0..5 {
                if bits & (0x10 >> col) != 0 {
                    for dy in 0..s {
                        for dx in 0..s {
                            blend(img, x + s + k as i64 * 6 * s + col * s + dx, y + s + row as i64 * s + dy, [255, 255, 255], 1.0);
                        }
                    }
                }
            }
        }
    }
}

/// 缩小到长边 ≤ max_edge，透明底铺成浅灰棋盘；返回画布与缩放比例
fn base_canvas(img: &DynamicImage, max_edge: u32) -> (RgbaImage, f32) {
    let long = img.width().max(img.height()).max(1);
    let k = (max_edge as f32 / long as f32).min(1.0);
    let base = if k < 1.0 {
        img.resize(((img.width() as f32 * k).round() as u32).max(1), ((img.height() as f32 * k).round() as u32).max(1), image::imageops::FilterType::Triangle)
    } else {
        img.clone()
    };
    let mut out = base.to_rgba8();
    for (x, y, p) in out.enumerate_pixels_mut() {
        if p[3] < 255 {
            let g = if ((x / 8) + (y / 8)) % 2 == 0 { 236.0 } else { 250.0 };
            let a = p[3] as f32 / 255.0;
            let v = [0, 1, 2].map(|i| (p[i] as f32 * a + g * (1.0 - a)) as u8);
            *p = Rgba([v[0], v[1], v[2], 255]);
        }
    }
    (out, k)
}

/// 在原图上只标出指定的几个框（加粗边框 + 编号），用作 AI 重绘图标时的上下文参考图
pub fn render_marks(img: &DynamicImage, items: &[(Bounds, String)], max_edge: u32, c: [u8; 3]) -> RgbaImage {
    let (mut out, k) = base_canvas(img, max_edge);
    let s = ((out.width().max(out.height()) as f32 / 500.0).round() as i64).clamp(2, 4);
    for (b, _) in items {
        let b = b.scaled(k);
        let pad = s * 2;
        rect_outline(&mut out, b.x as i64 - pad, b.y as i64 - pad, b.right() as i64 - 1 + pad, b.bottom() as i64 - 1 + pad, s, c, false);
    }
    for (b, text) in items {
        let b = b.scaled(k);
        let lh = 9 * s;
        let pad = s * 2;
        let y = if b.y as i64 - pad >= lh { b.y as i64 - pad - lh } else { b.bottom() as i64 + pad };
        label(&mut out, b.x as i64 - pad, y, text, c, s);
    }
    out
}

/// 生成标注图：长边不超过 max_edge
pub fn render(img: &DynamicImage, spec: &Spec, max_edge: u32) -> RgbaImage {
    // 透明底先铺成浅灰棋盘，避免框和编号看不清
    let (mut out, k) = base_canvas(img, max_edge);
    let s = ((out.width().max(out.height()) as f32 / 700.0).round() as i64).clamp(1, 3);
    let mut items: Vec<(&Node, usize)> = Vec::new();
    spec.root.walk(&mut |n, d| items.push((n, d)));
    // 先画框，再画编号，避免编号被后面的框盖住
    for (n, _) in &items {
        let b = n.bounds.scaled(k);
        let group = n.is_plain_group();
        let c = type_color(n.node_type, group);
        let w = if group { 1 } else { s.max(1) };
        rect_outline(&mut out, b.x as i64, b.y as i64, b.right() as i64 - 1, b.bottom() as i64 - 1, w, c, group);
    }
    for (n, _) in &items {
        let b = n.bounds.scaled(k);
        let num = n.id.trim_start_matches('n');
        let lh = 9 * s;
        let lw = num.len() as i64 * 6 * s + 2 * s;
        let c = type_color(n.node_type, n.is_plain_group());
        let fits = b.height as i64 >= lh + 2 && b.width as i64 >= lw + 2;
        let (x, y) = if fits {
            (b.x as i64 + 1, b.y as i64 + 1)
        } else if b.y as i64 >= lh {
            (b.x as i64, b.y as i64 - lh)
        } else {
            (b.x as i64, b.bottom() as i64)
        };
        label(&mut out, x, y, num, c, s);
    }
    out
}
