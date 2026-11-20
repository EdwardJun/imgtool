//! 用合成图校验解析精度：层级、边界、圆角、描边、渐变、阴影、文字、图标、布局

use image::DynamicImage;
use imgspec::fixtures::*;
use imgspec::*;

fn best(spec: &Spec, b: Bounds) -> (&Node, f32) {
    let mut out: Option<(&Node, f32)> = None;
    spec.root.walk(&mut |n, _| {
        let iou = n.bounds.iou(&b);
        if out.map(|o| iou > o.1).unwrap_or(true) {
            out = Some((n, iou));
        }
    });
    out.unwrap()
}

fn near(a: u32, b: u32, tol: u32) -> bool {
    (a as i64 - b as i64).unsigned_abs() as u32 <= tol
}

fn assert_bounds(n: &Node, b: Bounds, tol: u32) {
    let nb = n.bounds;
    assert!(near(nb.x, b.x, tol) && near(nb.y, b.y, tol) && near(nb.right(), b.right(), tol) && near(nb.bottom(), b.bottom(), tol), "{} 的框 {:?} 与期望 {:?} 相差超过 {tol}px", n.id, nb, b);
}

fn color_of(f: &Option<Fill>) -> [u8; 3] {
    f.as_ref().and_then(|f| f.main_color()).expect("应有填充色")
}

fn de(a: [u8; 3], b: [u8; 3]) -> f32 {
    color::delta_e(a, b)
}

fn radius(n: &Node) -> u32 {
    n.style.corner_radius.as_ref().map(|r| r.max()).unwrap_or(0)
}

const BG: [u8; 3] = [245, 246, 248];
const BLUE: [u8; 3] = [22, 119, 255];
const WHITE: [u8; 3] = [255, 255, 255];
const DARK: [u8; 3] = [17, 24, 39];

struct Page {
    img: DynamicImage,
    header: Bounds,
    card1: Bounds,
    title: Bounds,
    para: Bounds,
    button: Bounds,
    card2: Bounds,
    icon: Bounds,
    label: Bounds,
    photo: Bounds,
    line: Bounds,
}

fn page() -> Page {
    let mut img = canvas(750, 1100, BG);
    let header = Bounds::new(0, 0, 750, 96);
    rounded_rect(&mut img, header, 0.0, BLUE);
    text_line(&mut img, 300, 34, 32.0, 4, WHITE, true);

    let card1 = Bounds::new(24, 128, 702, 330);
    shadow(&mut img, card1, 16.0, 0, 4, 16.0, 0.12);
    rounded_rect(&mut img, card1, 16.0, WHITE);
    let title = text_line(&mut img, 56, 160, 36.0, 8, DARK, true);
    let l1 = text_line(&mut img, 56, 224, 26.0, 16, [75, 85, 99], false);
    let l2 = text_line(&mut img, 56, 266, 26.0, 10, [75, 85, 99], false);
    let para = l1.union(&l2);
    let button = Bounds::new(56, 354, 240, 72);
    rounded_rect(&mut img, button, 36.0, BLUE);
    let bx = button.x + (button.width - text_width(30.0, 4)) / 2;
    let by = button.y + (button.height - text_ink(30.0)) / 2;
    text_line(&mut img, bx, by, 30.0, 4, WHITE, false);

    let card2 = Bounds::new(24, 490, 702, 140);
    stroked_rect(&mut img, card2, 12.0, 2, [229, 231, 235], WHITE);
    let icon_b = Bounds::new(56, 530, 60, 60);
    icon(&mut img, icon_b, [250, 140, 22]);
    let label = text_line(&mut img, 140, 545, 30.0, 8, DARK, false);

    let photo_b = Bounds::new(24, 670, 340, 260);
    photo(&mut img, photo_b);
    let line = Bounds::new(24, 970, 702, 2);
    hline(&mut img, 24, 970, 702, [209, 213, 219]);
    hline(&mut img, 24, 971, 702, [209, 213, 219]);
    Page { img: DynamicImage::ImageRgba8(img), header, card1, title, para, button, card2, icon: icon_b, label, photo: photo_b, line }
}

#[test]
fn page_structure_and_styles() {
    let p = page();
    let spec = analyze(&p.img, "page.png", &AnalyzeOptions::default());
    println!("{}", imgspec::outline::outline(&spec, 200));
    assert_eq!(spec.kind, SpecKind::Page);
    assert!(de(color_of(&spec.root.style.fill), BG) < 3.0);

    let (h, iou) = best(&spec, p.header);
    assert!(iou > 0.9);
    assert_bounds(h, p.header, 2);
    assert!(de(color_of(&h.style.fill), BLUE) < 3.0);

    let (c1, _) = best(&spec, p.card1);
    assert_bounds(c1, p.card1, 2);
    assert_eq!(c1.node_type, NodeType::Frame);
    assert!(de(color_of(&c1.style.fill), WHITE) < 3.0);
    assert!(near(radius(c1), 16, 2), "card1 圆角 {:?}", c1.style.corner_radius);
    let sh = c1.style.shadow.as_ref().expect("card1 应有阴影");
    assert!(sh.x.abs() <= 2 && (sh.y - 4).abs() <= 2, "阴影偏移 {sh:?}");
    assert!((sh.blur as f32 - 16.0).abs() <= 16.0 * 0.3, "阴影模糊 {sh:?}");

    let (t, _) = best(&spec, p.title);
    assert_eq!(t.node_type, NodeType::Text);
    assert_bounds(t, p.title, 2);
    let ts = t.text_style.as_ref().unwrap();
    assert!((ts.font_size as f32 - 36.0).abs() <= 36.0 * 0.15, "标题字号 {}", ts.font_size);
    assert_eq!(ts.weight, FontWeight::Bold);

    let (para, _) = best(&spec, p.para);
    assert_eq!(para.node_type, NodeType::Text);
    let ps = para.text_style.as_ref().unwrap();
    assert_eq!(ps.lines, 2, "段落应合并为 2 行");
    assert!(near(ps.line_height.unwrap(), 42, 2), "行高 {:?}", ps.line_height);
    assert_eq!(ps.weight, FontWeight::Regular);
    assert_eq!(ps.align, TextAlign::Left);

    let (btn, _) = best(&spec, p.button);
    assert_bounds(btn, p.button, 2);
    assert!(near(radius(btn), 36, 2), "按钮圆角 {:?}", btn.style.corner_radius);
    let bl = btn.layout.as_ref().expect("按钮有内容");
    assert_eq!(bl.align, Align::Center);
    assert_eq!(bl.cross_align, CrossAlign::Center);
    assert_eq!(btn.children[0].node_type, NodeType::Text);

    let (c2, _) = best(&spec, p.card2);
    assert_bounds(c2, p.card2, 2);
    let st = c2.style.stroke.as_ref().expect("card2 应有描边");
    assert!(near(st.width, 2, 1), "描边宽度 {}", st.width);
    assert!(de(color::parse_hex(&st.color).unwrap(), [229, 231, 235]) < 5.0);
    assert!(near(radius(c2), 12, 2), "card2 圆角 {:?}", c2.style.corner_radius);
    let l2 = c2.layout.as_ref().unwrap();
    assert_eq!(l2.mode, LayoutMode::Row);

    let (ic, _) = best(&spec, p.icon);
    assert_eq!(ic.node_type, NodeType::Icon);
    assert_bounds(ic, p.icon, 2);
    let (lb, _) = best(&spec, p.label);
    assert_eq!(lb.node_type, NodeType::Text);

    let (ph, _) = best(&spec, p.photo);
    assert_eq!(ph.node_type, NodeType::Image);
    assert_bounds(ph, p.photo, 2);
    let (ln, _) = best(&spec, p.line);
    assert_eq!(ln.node_type, NodeType::Line);

    // 相对坐标
    assert_eq!(btn.relative.x, btn.bounds.x as i64 - c1.bounds.x as i64);
    // 根节点竖排
    assert_eq!(spec.root.layout.as_ref().unwrap().mode, LayoutMode::Column);
    // 编号是深度优先、从 n1 开始
    assert_eq!(spec.root.id, "n1");
}

#[test]
fn component_card_with_gradient_button() {
    let mut img = canvas(520, 360, WHITE);
    let card = Bounds::new(60, 50, 400, 260);
    shadow(&mut img, card, 20.0, 0, 8, 24.0, 0.15);
    rounded_rect(&mut img, card, 20.0, [250, 250, 252]);
    let btn = Bounds::new(100, 200, 320, 72);
    gradient_rect(&mut img, btn, 16.0, [255, 94, 58], [255, 42, 104], true);
    text_line(&mut img, 200, 222, 28.0, 4, WHITE, true);
    text_line(&mut img, 100, 90, 32.0, 6, DARK, true);
    let spec = analyze(&DynamicImage::ImageRgba8(img), "card.png", &AnalyzeOptions::default());
    println!("{}", imgspec::outline::outline(&spec, 100));
    assert_eq!(spec.kind, SpecKind::Component);
    assert_bounds(&spec.root, card, 2);
    assert!(near(radius(&spec.root), 20, 2));
    let sh = spec.root.style.shadow.as_ref().expect("应有阴影");
    assert!((sh.y - 8).abs() <= 2 && sh.x.abs() <= 2, "{sh:?}");
    assert!((sh.blur as f32 - 24.0).abs() <= 24.0 * 0.3, "{sh:?}");

    let (b, _) = best(&spec, btn);
    assert_bounds(b, btn, 2);
    let Some(Fill::Linear { angle, stops }) = &b.style.fill else { panic!("按钮应为线性渐变: {:?}", b.style.fill) };
    assert_eq!(*angle, 90.0);
    assert!(de(color::parse_hex(&stops[0].color).unwrap(), [255, 94, 58]) < 5.0, "{stops:?}");
    assert!(de(color::parse_hex(&stops[1].color).unwrap(), [255, 42, 104]) < 5.0, "{stops:?}");
    assert!(near(radius(b), 16, 2));
}

#[test]
fn white_card_on_white_with_shadow_and_input_outline() {
    let mut img = canvas(600, 400, WHITE);
    let card = Bounds::new(40, 40, 520, 200);
    shadow(&mut img, card, 12.0, 0, 6, 20.0, 0.18);
    rounded_rect(&mut img, card, 12.0, WHITE);
    text_line(&mut img, 80, 80, 28.0, 6, DARK, true);
    let input = Bounds::new(40, 290, 520, 64);
    stroked_rect(&mut img, input, 8.0, 1, [200, 204, 210], WHITE);
    text_line(&mut img, 64, 312, 22.0, 8, [156, 163, 175], false);
    let spec = analyze(&DynamicImage::ImageRgba8(img), "form.png", &AnalyzeOptions::default());
    println!("{}", imgspec::outline::outline(&spec, 100));
    let (c, iou) = best(&spec, card);
    assert!(iou > 0.85, "白卡片 iou {iou}");
    assert_bounds(c, card, 3);
    assert!(c.style.shadow.is_some());
    assert!(!c.children.is_empty(), "卡片里的文字应在卡片内");
    let (i, _) = best(&spec, input);
    assert_bounds(i, input, 2);
    let st = i.style.stroke.as_ref().expect("输入框描边");
    assert_eq!(st.width, 1);
    assert!(near(radius(i), 8, 2), "{:?}", i.style.corner_radius);
    assert_eq!(i.children.len(), 1);
}

#[test]
fn list_layout_gap_and_alignment() {
    let mut img = canvas(400, 504, WHITE);
    let mut rows = Vec::new();
    for k in 0..5u32 {
        let b = Bounds::new(20, 20 + k * 96, 360, 80);
        rounded_rect(&mut img, b, 10.0, [240, 242, 245]);
        rows.push(b);
    }
    let spec = analyze(&DynamicImage::ImageRgba8(img), "list.png", &AnalyzeOptions { mode: Some(SpecKind::Page), ..Default::default() });
    let l = spec.root.layout.as_ref().unwrap();
    assert_eq!(l.mode, LayoutMode::Column);
    assert_eq!(l.gap, Some(16));
    assert!(l.gap_uniform);
    assert_eq!(l.padding, [20, 20, 20, 20]);
    assert_eq!(l.cross_align, CrossAlign::Stretch);
    assert_eq!(spec.root.children.len(), 5);
}

#[test]
fn downscales_large_images_and_maps_back() {
    let mut img = canvas(3000, 1600, WHITE);
    let b = Bounds::new(300, 200, 1200, 600);
    rounded_rect(&mut img, b, 40.0, BLUE);
    let img = DynamicImage::ImageRgba8(img);
    let spec = analyze(&img, "big.png", &AnalyzeOptions { mode: Some(SpecKind::Page), ..Default::default() });
    println!("{}", imgspec::outline::outline(&spec, 50));
    let small = analyze(&img, "big.png", &AnalyzeOptions { mode: Some(SpecKind::Page), max_edge: 4000, ..Default::default() });
    println!("{}", imgspec::outline::outline(&small, 50));
    let (n, _) = best(&spec, b);
    assert_bounds(n, b, 4);
    assert!(near(radius(n), 40, 4), "{:?}", n.style.corner_radius);
    assert_eq!(spec.source.width, 3000);
}

#[test]
fn spec_roundtrip_json() {
    let p = page();
    let spec = analyze(&p.img, "page.png", &AnalyzeOptions::default());
    let text = serde_json::to_string(&spec).unwrap();
    let back: Spec = serde_json::from_str(&text).unwrap();
    assert_eq!(back, spec);
}
