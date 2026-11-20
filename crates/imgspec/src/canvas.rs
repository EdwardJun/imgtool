//! 分析用的像素画布与背景模型（纯色 / 线性渐变）

use image::DynamicImage;

use crate::color::{self, Rgb};
use crate::model::{Bounds, Fill, GradientStop};

/// RGB 浮点画布；透明像素按白底合成
pub struct Canvas {
    pub w: u32,
    pub h: u32,
    px: Vec<Rgb>,
    /// 原图边缘基本是透明的
    pub transparent_border: bool,
}

impl Canvas {
    /// 长边超过 max_edge 时先缩小；返回画布和缩放比例（画布 / 原图，≤ 1）
    pub fn from_image(img: &DynamicImage, max_edge: u32) -> (Canvas, f32) {
        let (ow, oh) = (img.width().max(1), img.height().max(1));
        let long = ow.max(oh);
        let (img, scale) = if long > max_edge {
            let k = max_edge as f32 / long as f32;
            let (w, h) = (((ow as f32 * k).round() as u32).max(1), ((oh as f32 * k).round() as u32).max(1));
            (img.resize_exact(w, h, image::imageops::FilterType::Triangle), w as f32 / ow as f32)
        } else {
            (img.clone(), 1.0)
        };
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let mut border = 0usize;
        let mut clear = 0usize;
        for (x, y, p) in rgba.enumerate_pixels() {
            if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
                border += 1;
                clear += (p[3] < 128) as usize;
            }
        }
        let px = rgba
            .pixels()
            .map(|p| {
                let a = p[3] as f32 / 255.0;
                [0, 1, 2].map(|i| p[i] as f32 * a + 255.0 * (1.0 - a))
            })
            .collect();
        (Canvas { w, h, px, transparent_border: clear * 2 >= border.max(1) }, scale)
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> Rgb {
        self.px[(y * self.w + x) as usize]
    }

    pub fn full(&self) -> Bounds {
        Bounds::new(0, 0, self.w, self.h)
    }

    /// 沿矩形内侧 inset 处的一圈采样（跳过 skip 比例的四角）
    pub fn ring(&self, r: &Bounds, inset: u32, skip: f32) -> Vec<(f32, f32, Rgb)> {
        let mut out = Vec::new();
        if r.width <= 2 * inset || r.height <= 2 * inset {
            return out;
        }
        let (x0, y0) = (r.x + inset, r.y + inset);
        let (x1, y1) = (r.right() - 1 - inset, r.bottom() - 1 - inset);
        let sx = ((x1 - x0) as f32 * skip) as u32;
        let sy = ((y1 - y0) as f32 * skip) as u32;
        let step_x = ((x1 - x0) / 400).max(1);
        let step_y = ((y1 - y0) / 400).max(1);
        let mut x = x0 + sx;
        while x <= x1.saturating_sub(sx) {
            out.push((x as f32, y0 as f32, self.get(x, y0)));
            out.push((x as f32, y1 as f32, self.get(x, y1)));
            x += step_x;
        }
        let mut y = y0 + sy;
        while y <= y1.saturating_sub(sy) {
            out.push((x0 as f32, y as f32, self.get(x0, y)));
            out.push((x1 as f32, y as f32, self.get(x1, y)));
            y += step_y;
        }
        out
    }
}

/// 背景（或填充）模型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Bg {
    Solid(Rgb),
    /// c(x,y) = c0 + gx·(x-ox) + gy·(y-oy)
    Linear { c0: Rgb, gx: Rgb, gy: Rgb, ox: f32, oy: f32 },
}

impl Bg {
    #[inline]
    pub fn at(&self, x: u32, y: u32) -> Rgb {
        match *self {
            Bg::Solid(c) => c,
            Bg::Linear { c0, gx, gy, ox, oy } => {
                let (dx, dy) = (x as f32 - ox, y as f32 - oy);
                [0, 1, 2].map(|i| c0[i] + gx[i] * dx + gy[i] * dy)
            }
        }
    }

    /// 代表色
    pub fn mean(&self) -> Rgb {
        match *self {
            Bg::Solid(c) => c,
            Bg::Linear { c0, .. } => c0,
        }
    }

    pub fn is_solid(&self) -> bool {
        matches!(self, Bg::Solid(_))
    }

    /// 转成 spec 里的填充描述；渐变按 CSS linear-gradient 的角度与色标表示
    pub fn to_fill(&self, r: &Bounds) -> Fill {
        match *self {
            Bg::Solid(c) => Fill::Solid { color: color::hex_f(c) },
            Bg::Linear { c0, gx, gy, ox, oy } => {
                // 主方向：各通道梯度向量的主成分
                let (mut sxx, mut sxy, mut syy) = (0f32, 0f32, 0f32);
                for i in 0..3 {
                    sxx += gx[i] * gx[i];
                    sxy += gx[i] * gy[i];
                    syy += gy[i] * gy[i];
                }
                let theta = 0.5 * (2.0 * sxy).atan2(sxx - syy);
                let (mut dx, mut dy) = (theta.cos(), theta.sin());
                // CSS 角度：0 = 向上，90 = 向右，180 = 向下；统一取 [0, 180)
                let mut angle = dx.atan2(-dy).to_degrees();
                if angle < 0.0 {
                    angle += 360.0;
                }
                if angle >= 180.0 {
                    angle -= 180.0;
                    dx = -dx;
                    dy = -dy;
                }
                let snapped = (angle / 45.0).round() * 45.0;
                let angle = if (angle - snapped).abs() <= 4.0 { snapped % 180.0 } else { angle.round() };
                // 渐变线两端：四个角在方向上的投影范围
                let corners = [(r.x as f32, r.y as f32), (r.right() as f32, r.y as f32), (r.x as f32, r.bottom() as f32), (r.right() as f32, r.bottom() as f32)];
                let proj: Vec<f32> = corners.iter().map(|(x, y)| (x - ox) * dx + (y - oy) * dy).collect();
                let (tmin, tmax) = proj.iter().fold((f32::MAX, f32::MIN), |(a, b), &t| (a.min(t), b.max(t)));
                let at = |t: f32| [0, 1, 2].map(|i| c0[i] + (gx[i] * dx + gy[i] * dy) * t);
                Fill::Linear {
                    angle,
                    stops: vec![GradientStop { pos: 0.0, color: color::hex_f(at(tmin)) }, GradientStop { pos: 1.0, color: color::hex_f(at(tmax)) }],
                }
            }
        }
    }
}

/// 拟合结果：模型与残差（RGB 距离的均方根）
pub struct Fit {
    pub bg: Bg,
    pub rms: f32,
    /// 参与拟合（非离群）的样本比例
    pub inliers: f32,
}

fn solve3(m: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let det = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(m);
    if d.abs() < 1e-9 {
        return None;
    }
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let mut mk = m;
        for r in 0..3 {
            mk[r][k] = b[r];
        }
        *o = det(mk) / d;
    }
    Some(out)
}

fn linear_fit(samples: &[(f32, f32, Rgb)], keep: &[bool], ox: f32, oy: f32) -> Option<Bg> {
    let mut m = [[0f64; 3]; 3];
    let mut b = [[0f64; 3]; 3];
    for (s, &k) in samples.iter().zip(keep) {
        if !k {
            continue;
        }
        let v = [1.0, (s.0 - ox) as f64, (s.1 - oy) as f64];
        for r in 0..3 {
            for c in 0..3 {
                m[r][c] += v[r] * v[c];
            }
            for ch in 0..3 {
                b[ch][r] += v[r] * s.2[ch] as f64;
            }
        }
    }
    let mut c0 = [0f32; 3];
    let mut gx = [0f32; 3];
    let mut gy = [0f32; 3];
    for ch in 0..3 {
        let sol = solve3(m, b[ch])?;
        c0[ch] = sol[0] as f32;
        gx[ch] = sol[1] as f32;
        gy[ch] = sol[2] as f32;
    }
    Some(Bg::Linear { c0, gx, gy, ox, oy })
}

fn rms_of(samples: &[(f32, f32, Rgb)], keep: &[bool], bg: &Bg) -> f32 {
    let (mut s, mut n) = (0f64, 0usize);
    for (p, &k) in samples.iter().zip(keep) {
        if k {
            let d = color::dist(p.2, bg.at(p.0 as u32, p.1 as u32)) as f64;
            s += d * d;
            n += 1;
        }
    }
    if n == 0 {
        f32::MAX
    } else {
        (s / n as f64).sqrt() as f32
    }
}

/// 拟合纯色或线性渐变：先取众数色，离群点（内容）剔除后再比较线性模型是否明显更好
pub fn fit(samples: &[(f32, f32, Rgb)], area: &Bounds) -> Option<Fit> {
    if samples.len() < 4 {
        return None;
    }
    let (mode, _) = color::mode_color(samples.iter().map(|s| s.2), 4)?;
    let solid = Bg::Solid(mode);
    let n = samples.len() as f32;
    let all = vec![true; samples.len()];
    let solid_keep: Vec<bool> = samples.iter().map(|s| color::dist(s.2, mode) <= 10.0).collect();
    let solid_rms = rms_of(samples, &solid_keep, &solid);
    let solid_in = solid_keep.iter().filter(|k| **k).count() as f32 / n;
    // 众数色已经能解释大部分样本：纯色（贴边的色块、内容不会被误当成渐变）
    if solid_in >= 0.6 {
        return Some(Fit { bg: solid, rms: solid_rms, inliers: solid_in });
    }

    let (ox, oy) = area.center();
    let mut keep = all;
    let mut best: Option<Bg> = None;
    for _ in 0..3 {
        let Some(m) = linear_fit(samples, &keep, ox, oy) else { break };
        let rms = rms_of(samples, &keep, &m).max(1.0);
        keep = samples.iter().map(|s| color::dist(s.2, m.at(s.0 as u32, s.1 as u32)) <= (2.5 * rms).max(6.0)).collect();
        best = Some(m);
    }
    if let Some(lin @ Bg::Linear { gx, gy, .. }) = best {
        let lin_in = keep.iter().filter(|k| **k).count() as f32 / n;
        let lin_rms = rms_of(samples, &keep, &lin);
        let span = [0, 1, 2].iter().map(|&i| (gx[i] * area.width as f32).abs() + (gy[i] * area.height as f32).abs()).fold(0f32, f32::max);
        // 渐变要真的有变化，并且比纯色解释了明显更多的样本
        if span >= 14.0 && lin_in >= solid_in + 0.15 && lin_in >= 0.6 {
            return Some(Fit { bg: lin, rms: lin_rms, inliers: lin_in });
        }
    }
    Some(Fit { bg: solid, rms: solid_rms, inliers: solid_in })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_solid_and_linear() {
        let area = Bounds::new(0, 0, 200, 100);
        let solid: Vec<_> = (0..200).map(|x| (x as f32, 0.0, [240.0, 240.0, 240.0])).collect();
        assert!(matches!(fit(&solid, &area).unwrap().bg, Bg::Solid(_)));
        // 从左到右 红 → 蓝
        let lin: Vec<_> = (0..200)
            .flat_map(|x| [0.0f32, 99.0].map(|y| (x as f32, y, [255.0 - x as f32 * 255.0 / 199.0, 0.0, x as f32 * 255.0 / 199.0])))
            .collect();
        let f = fit(&lin, &area).unwrap();
        let Fill::Linear { angle, stops } = f.bg.to_fill(&area) else { panic!("应为渐变") };
        assert_eq!(angle, 90.0);
        let a = color::parse_hex(&stops[0].color).unwrap();
        let b = color::parse_hex(&stops[1].color).unwrap();
        assert!(a[0] > 240 && a[2] < 15, "{a:?}");
        assert!(b[2] > 240 && b[0] < 15, "{b:?}");
    }
}
