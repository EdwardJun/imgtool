//! 真实 AI 生成稿里暴露的问题，用合成图复现

use image::{DynamicImage, Rgba};
use imgspec::fixtures::*;
use imgspec::*;

fn best(spec: &Spec, b: Bounds) -> (Node, f32) {
    let mut out: Option<(Node, f32)> = None;
    spec.root.walk(&mut |n, _| {
        let iou = n.bounds.iou(&b);
        if out.as_ref().map(|o| iou > o.1).unwrap_or(true) {
            out = Some((n.clone(), iou));
        }
    });
    out.unwrap()
}

/// 胶囊形渐变按钮：两端是弧线，边缘检查不能只看贴边一像素
#[test]
fn pill_gradient_button_is_a_box_with_centered_text() {
    let mut img = canvas(900, 400, [255, 255, 255]);
    let btn = Bounds::new(40, 130, 820, 140);
    gradient_rect(&mut img, btn, 70.0, [169, 133, 255], [93, 64, 237], true);
    let tw = text_width(56.0, 4);
    text_line(&mut img, btn.x + (btn.width - tw) / 2, btn.y + (btn.height - text_ink(56.0)) / 2, 56.0, 4, [255, 255, 255], false);
    let spec = analyze(&DynamicImage::ImageRgba8(img), "btn.png", &AnalyzeOptions { mode: Some(SpecKind::Page), ..Default::default() });
    let (n, iou) = best(&spec, btn);
    assert!(iou > 0.95, "{}", outline::outline(&spec, 30));
    assert!(matches!(n.style.fill, Some(Fill::Linear { .. })), "{:?}", n.style.fill);
    assert!((n.style.corner_radius.as_ref().unwrap().max() as i32 - 70).abs() <= 3);
    assert_eq!(n.children.len(), 1);
    assert_eq!(n.children[0].node_type, NodeType::Text);
    assert_eq!(n.layout.as_ref().unwrap().align, Align::Center);
}

/// 卡片里放着一张底色与页面接近的商品图：连通块中间是空的，但它仍是一张卡片
#[test]
fn card_whose_photo_matches_page_background() {
    let page = [238, 239, 241];
    let mut img = canvas(600, 600, page);
    let card = Bounds::new(60, 60, 420, 420);
    rounded_rect(&mut img, card, 22.0, [253, 253, 252]);
    let photo = Bounds::new(72, 72, 396, 260);
    rounded_rect(&mut img, photo, 16.0, [241, 241, 241]);
    rounded_rect(&mut img, Bounds::new(200, 120, 140, 160), 30.0, [190, 190, 192]);
    text_line(&mut img, 84, 360, 34.0, 8, [24, 24, 24], false);
    text_line(&mut img, 84, 420, 34.0, 3, [252, 64, 58], true);
    let spec = analyze(&DynamicImage::ImageRgba8(img), "card.png", &AnalyzeOptions { mode: Some(SpecKind::Page), ..Default::default() });
    let (n, iou) = best(&spec, card);
    assert!(iou > 0.95 && n.node_type == NodeType::Frame, "{}", outline::outline(&spec, 30));
    let mut texts = 0;
    n.walk(&mut |d, _| texts += (d.node_type == NodeType::Text) as usize);
    assert_eq!(texts, 2, "标题与价格都在卡片里且没有被合成一块：{}", outline::outline(&spec, 30));
    // 卡片外不应再有重复的元素
    assert_eq!(spec.root.children.len(), 1, "{}", outline::outline(&spec, 30));
}

/// 两行字之间的零星噪点不能把它们连成一块
#[test]
fn noise_specks_do_not_join_text_lines() {
    let mut img = canvas(600, 300, [255, 255, 255]);
    let a = text_line(&mut img, 40, 60, 36.0, 8, [20, 20, 20], false);
    let b = text_line(&mut img, 40, 120, 36.0, 3, [250, 60, 60], true);
    let mut seed = 3u32;
    for y in (a.bottom() - 2)..(b.y + 2) {
        for x in 40..400 {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            if (seed >> 16).is_multiple_of(23) {
                img.put_pixel(x, y, Rgba([200, 200, 200, 255]));
            }
        }
    }
    let spec = analyze(&DynamicImage::ImageRgba8(img), "t.png", &AnalyzeOptions { mode: Some(SpecKind::Page), ..Default::default() });
    let (ta, ia) = best(&spec, a);
    let (tb, ib) = best(&spec, b);
    assert!(ia > 0.8 && ib > 0.8, "{}", outline::outline(&spec, 30));
    assert_eq!(ta.node_type, NodeType::Text);
    assert_eq!(tb.node_type, NodeType::Text);
}

fn icons_of(spec: &Spec) -> Vec<Node> {
    let mut v = Vec::new();
    spec.root.walk(&mut |n, _| {
        if n.node_type == NodeType::Icon {
            v.push(n.clone());
        }
    });
    v
}

/// 粗体数字（"2026"）每个字都可能被当成实心色块：要一个带一个地并回文字行，不能拆出"图标"
#[test]
fn dense_bold_digits_stay_in_text_line() {
    let mut img = canvas(800, 200, [255, 255, 255]);
    let green = [5, 122, 64];
    // 4 个很"实"的字：填满的圆角块中间挖一道缝（实心度约 0.8）
    let mut x = 40;
    for _ in 0..4 {
        rounded_rect(&mut img, Bounds::new(x, 60, 13, 20), 3.0, green);
        rounded_rect(&mut img, Bounds::new(x + 4, 64, 5, 12), 0.0, [255, 255, 255]);
        x += 16;
    }
    let rest = text_line(&mut img, x + 2, 58, 27.0, 8, green, true);
    let spec = analyze(&DynamicImage::ImageRgba8(img), "d.png", &AnalyzeOptions { mode: Some(SpecKind::Page), ..Default::default() });
    assert!(icons_of(&spec).is_empty(), "{}", outline::outline(&spec, 40));
    let (t, iou) = best(&spec, Bounds::new(40, 58, rest.right() - 40, 24));
    assert!(iou > 0.8 && t.node_type == NodeType::Text, "{}", outline::outline(&spec, 40));
}

/// 带底色的序号徽标「01」：里面是文字，不是图标
#[test]
fn number_badge_is_frame_with_text() {
    let mut img = canvas(1086, 300, [255, 255, 255]);
    let badge = Bounds::new(40, 60, 47, 34);
    rounded_rect(&mut img, badge, 6.0, [68, 137, 91]);
    text_line(&mut img, 51, 67, 23.0, 2, [255, 255, 255], true);
    text_line(&mut img, 110, 63, 30.0, 5, [2, 27, 22], true);
    let spec = analyze(&DynamicImage::ImageRgba8(img), "b.png", &AnalyzeOptions { mode: Some(SpecKind::Page), ..Default::default() });
    assert!(icons_of(&spec).is_empty(), "{}", outline::outline(&spec, 40));
    let (n, _) = best(&spec, badge);
    assert_eq!(n.node_type, NodeType::Frame, "{}", outline::outline(&spec, 40));
    assert_eq!(n.children[0].node_type, NodeType::Text);
}

/// 列表前的小圆点放大后只是一团色块，不能当图标送去重绘
#[test]
fn list_bullets_are_not_icons() {
    let mut img = canvas(1086, 400, [255, 255, 255]);
    for k in 0..4u32 {
        let y = 60 + k * 50;
        rounded_rect(&mut img, Bounds::new(60, y + 6, 8, 8), 4.0, [4, 128, 63]);
        text_line(&mut img, 80, y, 21.0, 8, [70, 70, 71], false);
    }
    let spec = analyze(&DynamicImage::ImageRgba8(img), "l.png", &AnalyzeOptions { mode: Some(SpecKind::Page), ..Default::default() });
    assert!(icons_of(&spec).is_empty(), "{}", outline::outline(&spec, 40));
    let (dot, _) = best(&spec, Bounds::new(60, 66, 8, 8));
    assert_eq!(dot.node_type, NodeType::Rect);
}
