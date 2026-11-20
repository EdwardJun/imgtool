//! 颜色工具：十六进制、色差、亮度

pub type Rgb = [f32; 3];

pub fn to_u8(c: Rgb) -> [u8; 3] {
    c.map(|v| v.round().clamp(0.0, 255.0) as u8)
}

pub fn to_f(c: [u8; 3]) -> Rgb {
    c.map(|v| v as f32)
}

pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

pub fn hex_f(c: Rgb) -> String {
    hex(to_u8(c))
}

pub fn hex_alpha(c: [u8; 3], a: u8) -> String {
    format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], a)
}

/// 解析 #rgb / #rrggbb / #rrggbbaa（忽略透明度）
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    let full: String = match s.len() {
        3 => s.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => s[..6].to_string(),
        _ => return None,
    };
    let v = u32::from_str_radix(&full, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn dist(a: Rgb, b: Rgb) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

pub fn lum(c: Rgb) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

fn lab(c: [u8; 3]) -> [f32; 3] {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f32| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIE76 色差
pub fn delta_e(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (p, q) = (lab(a), lab(b));
    ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
}

/// 颜色量化后的桶编号（每通道 bits 位）
pub fn bucket(c: Rgb, bits: u32) -> u32 {
    let shift = 8 - bits;
    let q = |v: f32| (v.clamp(0.0, 255.0) as u32) >> shift;
    (q(c[0]) << (2 * bits)) | (q(c[1]) << bits) | q(c[2])
}

/// 众数颜色：先按 bits 位量化找最多的桶，再取桶内的平均色
pub fn mode_color(colors: impl Iterator<Item = Rgb>, bits: u32) -> Option<(Rgb, f32)> {
    let mut map: std::collections::HashMap<u32, (u32, [f64; 3])> = std::collections::HashMap::new();
    let mut total = 0u32;
    for c in colors {
        total += 1;
        let e = map.entry(bucket(c, bits)).or_insert((0, [0.0; 3]));
        e.0 += 1;
        for i in 0..3 {
            e.1[i] += c[i] as f64;
        }
    }
    let (_, (n, sum)) = map.into_iter().max_by_key(|(k, (n, _))| (*n, std::cmp::Reverse(*k)))?;
    Some((sum.map(|s| (s / n as f64) as f32), n as f32 / total.max(1) as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        assert_eq!(hex([22, 119, 255]), "#1677ff");
        assert_eq!(parse_hex("#1677ff"), Some([22, 119, 255]));
        assert_eq!(parse_hex("#fff"), Some([255, 255, 255]));
        assert_eq!(parse_hex("#00000033"), Some([0, 0, 0]));
        assert_eq!(parse_hex("xyz"), None);
        assert_eq!(hex_alpha([0, 0, 0], 0x33), "#00000033");
    }

    #[test]
    fn delta_e_basics() {
        assert!(delta_e([255, 255, 255], [255, 255, 255]) < 0.01);
        assert!(delta_e([255, 0, 0], [0, 0, 255]) > 100.0);
    }

    #[test]
    fn mode_prefers_majority() {
        let cs = vec![[10.0, 10.0, 10.0]; 5].into_iter().chain(vec![[200.0, 0.0, 0.0]; 2]);
        let (c, r) = mode_color(cs, 4).unwrap();
        assert_eq!(c, [10.0, 10.0, 10.0]);
        assert!((r - 5.0 / 7.0).abs() < 1e-4);
    }
}
