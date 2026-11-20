//! 合成测试图：圆角矩形、描边、渐变、阴影、仿文字、图标、照片块。单元测试与开发示例共用。

use image::{Rgba, RgbaImage};

use crate::model::Bounds;

pub fn canvas(w: u32, h: u32, bg: [u8; 3]) -> RgbaImage {
    RgbaImage::from_pixel(w, h, Rgba([bg[0], bg[1], bg[2], 255]))
}

fn mix(img: &mut RgbaImage, x: u32, y: u32, c: [u8; 3], a: f32) {
    if x >= img.width() || y >= img.height() || a <= 0.0 {
        return;
    }
    let p = img.get_pixel_mut(x, y);
    for i in 0..3 {
        p[i] = (p[i] as f32 * (1.0 - a) + c[i] as f32 * a).round() as u8;
    }
}

/// 点 (px,py) 是否在圆角矩形内
fn inside(b: &Bounds, r: f32, px: f32, py: f32) -> bool {
    let (x0, y0, x1, y1) = (b.x as f32, b.y as f32, b.right() as f32, b.bottom() as f32);
    if px < x0 || py < y0 || px > x1 || py > y1 {
        return false;
    }
    let cx = px.clamp(x0 + r, x1 - r);
    let cy = py.clamp(y0 + r, y1 - r);
    (px - cx).powi(2) + (py - cy).powi(2) <= r * r
}

/// 4×4 超采样的覆盖率
fn coverage(b: &Bounds, r: f32, x: u32, y: u32) -> f32 {
    let mut n = 0;
    for j in 0..4 {
        for i in 0..4 {
            if inside(b, r, x as f32 + (i as f32 + 0.5) / 4.0, y as f32 + (j as f32 + 0.5) / 4.0) {
                n += 1;
            }
        }
    }
    n as f32 / 16.0
}

pub fn rounded_rect(img: &mut RgbaImage, b: Bounds, r: f32, c: [u8; 3]) {
    paint(img, b, r, |_, _| c);
}

/// 按位置取色的圆角矩形（渐变用）
pub fn paint(img: &mut RgbaImage, b: Bounds, r: f32, f: impl Fn(u32, u32) -> [u8; 3]) {
    for y in b.y..b.bottom().min(img.height()) {
        for x in b.x..b.right().min(img.width()) {
            let a = coverage(&b, r, x, y);
            mix(img, x, y, f(x, y), a);
        }
    }
}

/// 描边 + 填充
pub fn stroked_rect(img: &mut RgbaImage, b: Bounds, r: f32, w: u32, stroke: [u8; 3], fill: [u8; 3]) {
    rounded_rect(img, b, r, stroke);
    let inner = b.inset(w).unwrap();
    rounded_rect(img, inner, (r - w as f32).max(0.0), fill);
}

/// 线性渐变（horizontal=true 时从左到右）
pub fn gradient_rect(img: &mut RgbaImage, b: Bounds, r: f32, c0: [u8; 3], c1: [u8; 3], horizontal: bool) {
    paint(img, b, r, |x, y| {
        let t = if horizontal { (x - b.x) as f32 / (b.width - 1).max(1) as f32 } else { (y - b.y) as f32 / (b.height - 1).max(1) as f32 };
        [0, 1, 2].map(|i| (c0[i] as f32 + (c1[i] as f32 - c0[i] as f32) * t).round() as u8)
    });
}

/// 黑色外阴影（与 CSS box-shadow: ox oy blur rgba(0,0,0,alpha) 近似：高斯 σ = blur / 2）
pub fn shadow(img: &mut RgbaImage, b: Bounds, r: f32, ox: i32, oy: i32, blur: f32, alpha: f32) {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut m = vec![0f32; w * h];
    let sb = Bounds::new((b.x as i32 + ox) as u32, (b.y as i32 + oy) as u32, b.width, b.height);
    for y in sb.y..sb.bottom().min(h as u32) {
        for x in sb.x..sb.right().min(w as u32) {
            m[y as usize * w + x as usize] = coverage(&sb, r, x, y);
        }
    }
    let sigma = (blur / 2.0).max(0.5);
    let rad = (sigma * 3.0).ceil() as i64;
    let k: Vec<f32> = (-rad..=rad).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let ks: f32 = k.iter().sum();
    let blur1 = |src: &[f32], horizontal: bool| {
        let mut out = vec![0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut s = 0f32;
                for (t, kv) in k.iter().enumerate() {
                    let d = t as i64 - rad;
                    let (sx, sy) = if horizontal { (x as i64 + d, y as i64) } else { (x as i64, y as i64 + d) };
                    if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
                        s += src[sy as usize * w + sx as usize] * kv;
                    }
                }
                out[y * w + x] = s / ks;
            }
        }
        out
    };
    let m = blur1(&blur1(&m, true), false);
    for y in 0..h {
        for x in 0..w {
            mix(img, x as u32, y as u32, [0, 0, 0], m[y * w + x] * alpha);
        }
    }
}

/// 仿文字：每个"字"是一个 H 形（两竖一横），墨迹高度约 0.87 × 字号
pub fn text_line(img: &mut RgbaImage, x: u32, y: u32, font_size: f32, chars: usize, c: [u8; 3], bold: bool) -> Bounds {
    let ink = (font_size / 1.15).round() as u32;
    let gw = (font_size * 0.6).round() as u32;
    let sw = ((font_size * if bold { 0.17 } else { 0.09 }).round() as u32).max(1);
    let gap = (font_size * 0.12).round().max(1.0) as u32;
    let mut cx = x;
    let mut end = x;
    for i in 0..chars {
        for yy in y..y + ink {
            for xx in cx..cx + gw {
                let left = xx < cx + sw;
                let right = xx >= cx + gw - sw;
                let bar = yy >= y + ink / 2 - sw / 2 && yy < y + ink / 2 - sw / 2 + sw;
                if left || right || bar {
                    mix(img, xx, yy, c, 1.0);
                }
            }
        }
        end = cx + gw;
        cx += gw + gap;
        // 每 4 个字一个词间距
        if i % 4 == 3 {
            cx += (font_size * 0.2) as u32;
        }
    }
    Bounds::new(x, y, end - x, ink)
}

/// 仿文字的宽度（与 text_line 一致），用于居中摆放
pub fn text_width(font_size: f32, chars: usize) -> u32 {
    let gw = (font_size * 0.6).round() as u32;
    let gap = (font_size * 0.12).round().max(1.0) as u32;
    let words = (chars.saturating_sub(1)) / 4;
    chars as u32 * gw + (chars as u32).saturating_sub(1) * gap + words as u32 * (font_size * 0.2) as u32
}

/// 仿文字的墨迹高度
pub fn text_ink(font_size: f32) -> u32 {
    (font_size / 1.15).round() as u32
}

/// 图标：圆环 + 中心圆点（两部分，测试笔画合并）
pub fn icon(img: &mut RgbaImage, b: Bounds, c: [u8; 3]) {
    let (cx, cy) = (b.x as f32 + b.width as f32 / 2.0, b.y as f32 + b.height as f32 / 2.0);
    let ro = b.width.min(b.height) as f32 / 2.0;
    let ri = ro * 0.72;
    let rd = ro * 0.28;
    for y in b.y..b.bottom() {
        for x in b.x..b.right() {
            let mut n = 0;
            for j in 0..4 {
                for i in 0..4 {
                    let (px, py) = (x as f32 + (i as f32 + 0.5) / 4.0, y as f32 + (j as f32 + 0.5) / 4.0);
                    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
                    if (d <= ro && d >= ri) || d <= rd {
                        n += 1;
                    }
                }
            }
            mix(img, x, y, c, n as f32 / 16.0);
        }
    }
}

/// 照片块：很多颜色
pub fn photo(img: &mut RgbaImage, b: Bounds) {
    let mut seed = 12345u32;
    for y in b.y..b.bottom() {
        for x in b.x..b.right() {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let n = (seed >> 16) as u8 % 40;
            let r = ((x - b.x) * 255 / b.width.max(1)) as u8;
            let g = ((y - b.y) * 255 / b.height.max(1)) as u8;
            let p = img.get_pixel_mut(x, y);
            *p = Rgba([r.saturating_add(n), g.saturating_add(n / 2), 120u8.saturating_add(n), 255]);
        }
    }
}

pub fn hline(img: &mut RgbaImage, x: u32, y: u32, w: u32, c: [u8; 3]) {
    for xx in x..x + w {
        mix(img, xx, y, c, 1.0);
    }
}
