//! 样式拟合：圆角、外阴影、文字度量

use crate::canvas::{Bg, Canvas};
use crate::color::{self, Rgb};
use crate::model::{Bounds, CornerRadius, FontWeight, Shadow};

/// 1 - π/4：半径为 r 的圆角缺掉的面积 = 该系数 × r²
const CORNER_AREA: f32 = 1.0 - std::f32::consts::FRAC_PI_4;

/// 像素对"外圈颜色"的覆盖率：投影到 bg → outer 的连线上
fn coverage(p: Rgb, bg: Rgb, outer: Rgb) -> Option<f32> {
    let d = [outer[0] - bg[0], outer[1] - bg[1], outer[2] - bg[2]];
    let len2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    if len2 < 144.0 {
        return None;
    }
    let v = (p[0] - bg[0]) * d[0] + (p[1] - bg[1]) * d[1] + (p[2] - bg[2]) * d[2];
    Some((v / len2).clamp(0.0, 1.0))
}

/// 用"角上缺失面积"估算四个圆角半径（抗锯齿下也能到亚像素精度）。
/// outer：最外圈颜色（有描边时是描边色，否则是填充）；bg：父容器底色。
/// 返回 [左上, 右上, 右下, 左下]；外圈与底色太接近时无法测量返回 None
pub fn corner_radii(cv: &Canvas, r: &Bounds, bg: &Bg, outer: &Bg) -> Option<[f32; 4]> {
    let s = (r.width.min(r.height) / 2) as i64;
    if s < 2 {
        return None;
    }
    let mut out = [0f32; 4];
    // (起点 x, 起点 y, x 方向, y 方向)
    let corners = [
        (r.x as i64, r.y as i64, 1i64, 1i64),
        (r.right() as i64 - 1, r.y as i64, -1, 1),
        (r.right() as i64 - 1, r.bottom() as i64 - 1, -1, -1),
        (r.x as i64, r.bottom() as i64 - 1, 1, -1),
    ];
    for (k, &(cx, cy, sx, sy)) in corners.iter().enumerate() {
        let (px, py) = (cx as u32, cy as u32);
        let (b, o) = (bg.at(px, py), outer.at(px, py));
        coverage(o, b, o)?;
        let mut rows = Vec::with_capacity(s as usize);
        for j in 0..s {
            let y = (cy + j * sy) as u32;
            let mut row = 0f32;
            let mut peak = 0f32;
            for i in 0..s {
                let x = (cx + i * sx) as u32;
                let c = coverage(cv.get(x, y), b, o).unwrap_or(1.0);
                // 到达实心部分，或越过了细描边的峰值
                if c >= 0.97 || (peak >= 0.4 && c < peak - 0.1) {
                    break;
                }
                peak = peak.max(c);
                row += 1.0 - c;
            }
            rows.push(row);
        }
        // 直边部分每行的缺失（缩放后的半像素边缘）作为基线扣掉
        let tail = &rows[rows.len() * 3 / 4..];
        let mut t: Vec<f32> = tail.to_vec();
        t.sort_by(f32::total_cmp);
        let base = t.get(t.len() / 2).copied().unwrap_or(0.0);
        let mut missing = 0f32;
        for &row in &rows {
            let m = row - base;
            if m < 0.03 {
                break;
            }
            missing += m;
        }
        out[k] = (missing / CORNER_AREA).sqrt().min(s as f32);
    }
    Some(out)
}

/// 半径数组 → spec 表示；四个角接近时合并成一个数
pub fn radius_value(r: [f32; 4], scale: f32) -> Option<CornerRadius> {
    let v = r.map(|x| (x / scale).round() as u32);
    if v.iter().all(|&x| x <= 1) {
        return None;
    }
    let mean = r.iter().sum::<f32>() / 4.0;
    if r.iter().all(|&x| (x - mean).abs() <= 1.5_f32.max(mean * 0.12)) {
        Some(CornerRadius::Uniform((mean / scale).round() as u32))
    } else {
        Some(CornerRadius::Corners(v))
    }
}

/// 估算外阴影：沿四边向外取亮度下降的剖面，用剖面延伸距离推算偏移与模糊
pub fn estimate_shadow(cv: &Canvas, r: &Bounds, area: &Bounds, bg: &Bg) -> Option<Shadow> {
    let m = (r.width.min(r.height) / 2).clamp(8, 80) as i64;
    let mut profiles: [Vec<f32>; 4] = Default::default(); // 上 下 左 右
    for (side, prof) in profiles.iter_mut().enumerate() {
        let horizontal = side < 2;
        let (len, start) = if horizontal { (r.width, r.x) } else { (r.height, r.y) };
        let a = start + len / 4;
        let b = start + len * 3 / 4;
        if b <= a {
            return None;
        }
        for k in 0..m {
            let pos = match side {
                0 => r.y as i64 - 1 - k,
                1 => r.bottom() as i64 + k,
                2 => r.x as i64 - 1 - k,
                _ => r.right() as i64 + k,
            };
            let inside = if horizontal { pos >= area.y as i64 && pos < area.bottom() as i64 } else { pos >= area.x as i64 && pos < area.right() as i64 };
            if !inside {
                break;
            }
            let (mut sum, mut n, mut blocked) = (0f32, 0f32, false);
            let step = ((b - a) / 64).max(1);
            let mut t = a;
            while t < b {
                let (x, y) = if horizontal { (t, pos as u32) } else { (pos as u32, t) };
                let p = cv.get(x, y);
                let q = bg.at(x, y);
                // 碰到别的元素就停下
                if color::dist(p, q) > 70.0 {
                    blocked = true;
                    break;
                }
                sum += color::lum(q) - color::lum(p);
                n += 1.0;
                t += step;
            }
            if blocked || n == 0.0 {
                break;
            }
            prof.push(sum / n);
        }
    }
    let peak = profiles.iter().filter_map(|p| p.iter().take(3).copied().reduce(f32::max)).fold(0f32, f32::max);
    if peak < 2.5 || profiles.iter().filter(|p| p.len() >= 3).count() < 3 {
        return None;
    }
    let thr = (0.06 * peak).max(0.5);
    let ext: Vec<f32> = profiles
        .iter()
        .map(|p| match p.iter().position(|&d| d < thr) {
            Some(k) => k as f32,
            None => p.len() as f32,
        })
        .collect();
    let edge = profiles.iter().map(|p| p.first().copied().unwrap_or(0.0).max(0.0)).sum::<f32>() / 4.0;
    let (et, eb, el, er) = (ext[0], ext[1], ext[2], ext[3]);
    let blur = (et + eb + el + er) / 4.0 * 1.15;
    if blur < 1.5 {
        return None;
    }
    let oy = (eb - et) / 2.0;
    let ox = (er - el) / 2.0;
    let base = color::lum(bg.mean()).max(1.0);
    let alpha = (2.0 * edge / base).clamp(0.02, 1.0);
    // 剖面被截断（贴边或碰到别的元素）时降低置信度
    let truncated = profiles.iter().filter(|p| p.len() < m as usize && p.last().map(|&d| d >= thr).unwrap_or(true)).count();
    let confidence = (0.35 + 0.4 * (peak / 12.0).min(1.0) - 0.15 * truncated as f32).clamp(0.1, 0.85);
    Some(Shadow { x: ox.round() as i32, y: oy.round() as i32, blur: blur.round() as u32, color: color::hex_alpha([0, 0, 0], (alpha * 255.0).round() as u8), confidence })
}

/// 文字度量：墨迹高度 → 字号；笔画宽度 / 墨迹高度 → 字重
pub fn font_size_from_ink(ink: f32) -> f32 {
    ink * 1.15
}

pub fn weight_from_runs(mask_rows: impl Iterator<Item = Vec<bool>>, ink: f32) -> FontWeight {
    let mut runs = Vec::new();
    for row in mask_rows {
        let mut n = 0u32;
        for v in row.into_iter().chain(std::iter::once(false)) {
            if v {
                n += 1;
            } else if n > 0 {
                runs.push(n);
                n = 0;
            }
        }
    }
    if runs.is_empty() || ink <= 0.0 {
        return FontWeight::Regular;
    }
    runs.sort_unstable();
    let ratio = runs[runs.len() / 2] as f32 / ink;
    if ratio < 0.13 {
        FontWeight::Regular
    } else if ratio < 0.17 {
        FontWeight::Medium
    } else {
        FontWeight::Bold
    }
}
