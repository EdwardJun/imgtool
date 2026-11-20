//! 证件照：常用尺寸、常用底色，以及按去背景后的主体自动构图。
//! 自动构图只依赖透明度遮罩（不做人脸检测）：主体顶部水平居中，头顶上方留出固定比例的空白，
//! 高度覆盖到主体底部，再按目标宽高比取框，超出原图时整体缩小框。

use crate::background::SubjectBounds;
use crate::types::PixelRect;

/// 证件照尺寸（像素，按 300 DPI 换算）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhotoSize {
    pub key: &'static str,
    pub label: &'static str,
    pub width: u32,
    pub height: u32,
}

pub const PHOTO_SIZES: [PhotoSize; 4] = [
    PhotoSize { key: "1inch", label: "一寸", width: 295, height: 413 },
    PhotoSize { key: "2inch", label: "二寸", width: 413, height: 579 },
    PhotoSize { key: "small-1inch", label: "小一寸", width: 260, height: 378 },
    PhotoSize { key: "small-2inch", label: "小二寸", width: 413, height: 531 },
];

/// 常用证件照底色
pub const FILL_COLORS: [(&str, &str, [u8; 3]); 3] = [
    ("white", "白", [255, 255, 255]),
    ("blue", "蓝", [67, 142, 219]),
    ("red", "红", [255, 0, 0]),
];

/// 头顶上方留白占框高的比例
pub const HEAD_ROOM: f64 = 0.1;
/// 识别到头部时，头宽占框宽的比例（真人头高约为头宽 1.3 倍，对应头部约占框高 55%，接近常见证件照规范）
pub const HEAD_WIDTH_RATIO: f64 = 0.6;

/// 解析证件照尺寸：预设名（1inch / 一寸 …）或 "宽x高"
pub fn parse_photo_size(s: &str) -> Result<(u32, u32), String> {
    let t = s.trim();
    if let Some(p) = PHOTO_SIZES.iter().find(|p| p.key.eq_ignore_ascii_case(t) || p.label == t) {
        return Ok((p.width, p.height));
    }
    let names = PHOTO_SIZES.iter().map(|p| format!("{}({}x{})", p.key, p.width, p.height)).collect::<Vec<_>>().join(" / ");
    let err = || format!("无效证件照尺寸 {s}，可选 {names}，或 \"宽x高\" 如 358x441");
    let (a, b) = t.split_once(['x', 'X', '*', '×']).ok_or_else(err)?;
    let w: u32 = a.trim().parse().map_err(|_| err())?;
    let h: u32 = b.trim().parse().map_err(|_| err())?;
    if w == 0 || h == 0 || w > 10000 || h > 10000 {
        return Err(err());
    }
    Ok((w, h))
}

/// 常用底色名（white/blue/red、白/蓝/红）对应的颜色
pub fn named_fill(s: &str) -> Option<[u8; 3]> {
    let t = s.trim().trim_end_matches('色').trim_end_matches('底');
    FILL_COLORS.iter().find(|(k, l, _)| k.eq_ignore_ascii_case(t) || *l == t).map(|(_, _, c)| *c)
}

/// 按主体自动构图，返回 aspect_w:aspect_h 比例的裁剪框（不超出 img_w × img_h）
pub fn frame(subject: &SubjectBounds, img_w: u32, img_h: u32, aspect_w: u32, aspect_h: u32) -> PixelRect {
    let r = aspect_w.max(1) as f64 / aspect_h.max(1) as f64;
    let (iw, ih) = (img_w.max(1) as f64, img_h.max(1) as f64);
    let top = subject.y as f64;
    let bottom = (subject.y + subject.height) as f64;
    let (mut w, mut h) = match subject.head_width {
        // 识别到头部和肩膀：按头宽定框大小
        Some(hw) if hw > 0 => {
            let w = hw as f64 / HEAD_WIDTH_RATIO;
            (w, w / r)
        }
        // 否则框住整个主体：从头顶留白到主体底部
        _ => {
            let h = ((bottom - top) / (1.0 - HEAD_ROOM)).max(1.0);
            (h * r, h)
        }
    };
    if w > iw {
        w = iw;
        h = w / r;
    }
    if h > ih {
        h = ih;
        w = h * r;
    }
    let (wi, hi) = ((w.round() as u32).clamp(1, img_w.max(1)), (h.round() as u32).clamp(1, img_h.max(1)));
    let x = (subject.top_center_x as f64 - wi as f64 / 2.0).round().clamp(0.0, (img_w - wi) as f64) as u32;
    let y = (top - HEAD_ROOM * hi as f64).round().clamp(0.0, (img_h - hi) as f64) as u32;
    PixelRect { x, y, width: wi, height: hi }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject(x: u32, y: u32, width: u32, height: u32, cx: u32) -> SubjectBounds {
        SubjectBounds { x, y, width, height, top_center_x: cx, head_width: None, shoulder_y: None }
    }

    #[test]
    fn frames_by_head_width_when_shoulders_found() {
        let s = SubjectBounds { head_width: Some(300), shoulder_y: Some(560), ..subject(100, 200, 800, 900, 500) };
        let r = frame(&s, 1000, 1100, 295, 413);
        assert_eq!(r.width, 500, "头宽占框宽 60%: {r:?}");
        assert!((r.height as i64 - 700).abs() <= 1, "{r:?}");
        assert_eq!(r.x, 250);
        assert_eq!(r.y, 130, "头顶留白 10%: {r:?}");
    }

    #[test]
    fn parses_sizes() {
        assert_eq!(parse_photo_size("1inch").unwrap(), (295, 413));
        assert_eq!(parse_photo_size("二寸").unwrap(), (413, 579));
        assert_eq!(parse_photo_size("358x441").unwrap(), (358, 441));
        assert!(parse_photo_size("3inch").is_err());
        assert!(parse_photo_size("0x10").is_err());
        assert_eq!(named_fill("Blue"), Some([67, 142, 219]));
        assert_eq!(named_fill("红底"), Some([255, 0, 0]));
        assert_eq!(named_fill("#fff"), None);
    }

    #[test]
    fn frames_portrait_with_head_room() {
        // 1000x1200 的半身照：头顶 y=300，主体延伸到底部，头部中心 x=520
        let r = frame(&subject(200, 300, 640, 900, 520), 1000, 1200, 295, 413);
        assert_eq!(r.y + r.height, 1200, "框底贴着主体底部: {r:?}");
        let head_room = (300 - r.y) as f64 / r.height as f64;
        assert!((head_room - HEAD_ROOM).abs() < 0.01, "{r:?}");
        let ratio = r.width as f64 / r.height as f64;
        assert!((ratio - 295.0 / 413.0).abs() < 0.01, "{r:?}");
        assert!((r.x as i64 + r.width as i64 / 2 - 520).abs() <= 1, "按头部水平居中: {r:?}");
    }

    #[test]
    fn frame_stays_inside_image() {
        // 主体占满一张横图：框要按宽高比缩小并留在图内
        let r = frame(&subject(0, 0, 800, 400, 790), 800, 400, 3, 4);
        assert!(r.x + r.width <= 800 && r.y + r.height <= 400, "{r:?}");
        assert_eq!(r.height, 400);
        assert_eq!(r.width, 300);
        assert_eq!(r.x, 500, "靠右的主体，框贴右边: {r:?}");
    }
}
