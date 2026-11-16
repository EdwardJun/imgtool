//! 智能裁剪：识别一张图里的多个小图标（如底部导航栏、图标合集），返回各自的外接框。
//!
//! 纯本地算法，不调用大模型：
//! 1. 前景掩码：有透明背景时按 alpha，否则按与背景色（自动识别或指定）的距离
//! 2. 按"合并间距"膨胀掩码后做连通区域标记，把同一图标的零散笔画、同一行文字的多个字连成一块；
//!    再用原始掩码收紧外接框
//! 3. 互相包含/重叠的框合并（如气泡里的小点、圆圈里的指针）
//! 4. 按行分组：明显比上一行矮、且位于上一行图标正下方的行判定为文字标签，
//!    默认把标签并入上方图标，也可以丢弃只保留图标
//! 5. 过滤噪点，按从上到下、从左到右排序

use serde::{Deserialize, Serialize};

use image::{DynamicImage, RgbaImage};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DetectOptions {
    /// 背景色 [r,g,b]；为空时自动判断：图片有透明背景就按透明度，否则识别边缘颜色
    #[serde(default)]
    pub background: Option<[u8; 3]>,
    /// 与背景色的容差 0-100，默认 12（透明背景时不使用）
    #[serde(default = "default_tolerance")]
    pub tolerance: u8,
    /// 合并间距（像素）：距离小于它的笔画视为同一个图标；为空时按图片尺寸自动估算
    #[serde(default)]
    pub gap: Option<u32>,
    /// 把图标下方的文字标签一起框进去，默认 false（只保留图标，识别出的文字会被去掉）
    #[serde(default)]
    pub include_labels: bool,
    /// 最小尺寸（像素），长边小于它的区域视为噪点；为空时自动估算
    #[serde(default)]
    pub min_size: Option<u32>,
}

fn default_tolerance() -> u8 {
    12
}

impl Default for DetectOptions {
    fn default() -> Self {
        Self { background: None, tolerance: default_tolerance(), gap: None, include_labels: false, min_size: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RegionKind {
    /// 单独的图标
    Icon,
    /// 图标 + 下方文字标签
    IconWithLabel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    #[serde(default = "default_kind")]
    pub kind: RegionKind,
}

fn default_kind() -> RegionKind {
    RegionKind::Icon
}

impl Region {
    fn right(&self) -> u32 {
        self.x + self.width
    }
    fn bottom(&self) -> u32 {
        self.y + self.height
    }
    fn union(&self, o: &Region) -> Region {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Region { x, y, width: self.right().max(o.right()) - x, height: self.bottom().max(o.bottom()) - y, kind: self.kind }
    }
    fn intersects(&self, o: &Region) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
    fn h_overlap(&self, o: &Region) -> u32 {
        self.right().min(o.right()).saturating_sub(self.x.max(o.x))
    }
    fn center_x(&self) -> f32 {
        self.x as f32 + self.width as f32 / 2.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectResult {
    pub regions: Vec<Region>,
    /// 实际使用的合并间距
    pub gap: u32,
    /// 实际使用的最小尺寸
    pub min_size: u32,
    /// "alpha"（按透明度）或 "color"（按背景色）
    pub mode: String,
    /// 按背景色识别时使用的背景色
    pub background: Option<[u8; 3]>,
    /// 识别出的文字标签数量（无论是否并入图标）
    pub labels_found: usize,
}
/// 区域超过这个数量基本说明不是图标拼图（比如照片），截断返回
const MAX_REGIONS: usize = 300;

/// 前景掩码：true = 前景
fn foreground_mask(img: &RgbaImage, opts: &DetectOptions) -> (Vec<bool>, String, Option<[u8; 3]>) {
    let (w, h) = img.dimensions();
    let border_transparent = {
        let mut total = 0usize;
        let mut clear = 0usize;
        for x in 0..w {
            for y in [0, h - 1] {
                total += 1;
                clear += (img.get_pixel(x, y)[3] < 128) as usize;
            }
        }
        for y in 0..h {
            for x in [0, w - 1] {
                total += 1;
                clear += (img.get_pixel(x, y)[3] < 128) as usize;
            }
        }
        clear * 2 >= total
    };
    if opts.background.is_none() && border_transparent {
        let mask = img.pixels().map(|p| p[3] >= 32).collect();
        return (mask, "alpha".into(), None);
    }
    let bg = opts.background.unwrap_or_else(|| crate::background::detect_background(img));
    let tol = opts.tolerance.min(100) as f32;
    let mask = img
        .pixels()
        .map(|p| {
            if p[3] < 32 {
                return false;
            }
            let d: f32 = (0..3).map(|i| (p[i] as f32 - bg[i] as f32).powi(2)).sum::<f32>().sqrt() / 441.672_96 * 100.0;
            d > tol
        })
        .collect();
    (mask, "color".into(), Some(bg))
}

/// 方形膨胀（半径 r）：用前缀和在 O(n) 内完成
fn dilate(mask: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    if r == 0 {
        return mask.to_vec();
    }
    // 水平方向
    let mut horiz = vec![false; w * h];
    let mut prefix = vec![0u32; w + 1];
    for y in 0..h {
        let row = &mask[y * w..(y + 1) * w];
        for x in 0..w {
            prefix[x + 1] = prefix[x] + row[x] as u32;
        }
        for x in 0..w {
            let a = x.saturating_sub(r);
            let b = (x + r + 1).min(w);
            horiz[y * w + x] = prefix[b] > prefix[a];
        }
    }
    // 垂直方向
    let mut out = vec![false; w * h];
    let mut col = vec![0u32; h + 1];
    for x in 0..w {
        for y in 0..h {
            col[y + 1] = col[y] + horiz[y * w + x] as u32;
        }
        for y in 0..h {
            let a = y.saturating_sub(r);
            let b = (y + r + 1).min(h);
            out[y * w + x] = col[b] > col[a];
        }
    }
    out
}

/// 在膨胀后的掩码上做 8 邻域连通标记，用原始掩码计算每块的紧外接框
fn components(mask: &[bool], dilated: &[bool], w: usize, h: usize) -> Vec<Region> {
    let mut label = vec![u32::MAX; w * h];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !dilated[start] || label[start] != u32::MAX {
            continue;
        }
        let id = out.len() as u32;
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        let mut has_fg = false;
        label[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            if mask[i] {
                has_fg = true;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if dilated[j] && label[j] == u32::MAX {
                        label[j] = id;
                        stack.push(j);
                    }
                }
            }
        }
        // 占位，保证 id 与 out 下标一致
        out.push(if has_fg {
            Some(Region { x: x0 as u32, y: y0 as u32, width: (x1 - x0 + 1) as u32, height: (y1 - y0 + 1) as u32, kind: RegionKind::Icon })
        } else {
            None
        });
    }
    out.into_iter().flatten().collect()
}

/// 反复合并互相重叠的框，直到稳定
fn merge_overlapping(mut boxes: Vec<Region>) -> Vec<Region> {
    loop {
        let mut merged = false;
        let mut i = 0;
        while i < boxes.len() {
            let mut j = i + 1;
            while j < boxes.len() {
                if boxes[i].intersects(&boxes[j]) {
                    let b = boxes.swap_remove(j);
                    boxes[i] = boxes[i].union(&b);
                    merged = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        if !merged {
            return boxes;
        }
    }
}

/// 按行分组：纵向重叠超过较矮者一半的框属于同一行；行按顶部排序，行内按左边排序
fn group_rows(mut boxes: Vec<Region>) -> Vec<Vec<Region>> {
    boxes.sort_by_key(|b| (b.y, b.x));
    let mut rows: Vec<(u32, u32, Vec<Region>)> = Vec::new();
    for b in boxes {
        let found = rows.iter_mut().find(|(top, bottom, _)| {
            let overlap = (*bottom).min(b.bottom()).saturating_sub((*top).max(b.y));
            let min_h = (*bottom - *top).min(b.height).max(1);
            overlap * 2 >= min_h
        });
        match found {
            Some((top, bottom, row)) => {
                *top = (*top).min(b.y);
                *bottom = (*bottom).max(b.bottom());
                row.push(b);
            }
            None => rows.push((b.y, b.bottom(), vec![b])),
        }
    }
    rows.sort_by_key(|(top, _, _)| *top);
    rows.into_iter()
        .map(|(_, _, mut r)| {
            r.sort_by_key(|b| b.x);
            r
        })
        .collect()
}

fn median_height(row: &[Region]) -> f32 {
    let mut hs: Vec<u32> = row.iter().map(|b| b.height).collect();
    hs.sort_unstable();
    hs[hs.len() / 2] as f32
}

/// 识别图中的多个图标
pub fn detect_regions(img: &DynamicImage, opts: &DetectOptions) -> DetectResult {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let long = w.max(h);
    let gap = opts.gap.unwrap_or_else(|| ((long as f32 * 0.015).round() as u32).max(4));
    let min_size = opts.min_size.unwrap_or_else(|| ((long as f32 * 0.01).round() as u32).max(4));
    let (mask, mode, background) = foreground_mask(&rgba, opts);
    let (wu, hu) = (w as usize, h as usize);
    let dilated = dilate(&mask, wu, hu, (gap / 2) as usize);

    let boxes = merge_overlapping(components(&mask, &dilated, wu, hu));
    let boxes: Vec<Region> = boxes.into_iter().filter(|b| b.width.max(b.height) >= min_size).collect();

    // 识别文字标签行并归属到上一行的图标
    let rows = group_rows(boxes);
    let mut result: Vec<Region> = Vec::new();
    let mut labels_found = 0;
    let mut prev_icons: Vec<usize> = Vec::new(); // 上一行图标在 result 中的下标
    let mut prev_median = 0f32;
    let mut prev_bottom = 0u32;
    for row in rows {
        let med = median_height(&row);
        let top = row.iter().map(|b| b.y).min().unwrap_or(0);
        let is_label_row = !prev_icons.is_empty()
            && med < prev_median * 0.6
            && (top.saturating_sub(prev_bottom) as f32) < prev_median
            && row.iter().all(|l| prev_icons.iter().any(|&i| result[i].h_overlap(l) > 0 || (result[i].center_x() - l.center_x()).abs() < result[i].width as f32));
        if is_label_row {
            labels_found += row.len();
            for label in row {
                // 归给水平重叠最多（其次中心最近）的上方图标
                let owner = prev_icons
                    .iter()
                    .copied()
                    .max_by(|&a, &b| {
                        let key = |i: usize| (result[i].h_overlap(&label), -(result[i].center_x() - label.center_x()).abs() as i64);
                        key(a).cmp(&key(b))
                    })
                    .expect("prev_icons 非空");
                if opts.include_labels {
                    let merged = result[owner].union(&label);
                    result[owner] = Region { kind: RegionKind::IconWithLabel, ..merged };
                }
            }
            // 标签行之后不再作为下一行的"上方图标"
            prev_icons.clear();
            continue;
        }
        let start = result.len();
        result.extend(row);
        prev_icons = (start..result.len()).collect();
        prev_median = med;
        prev_bottom = result[start..].iter().map(|b| b.bottom()).max().unwrap_or(0);
    }

    // 并入标签后可能产生新的重叠；最终按行列排序
    let rows = group_rows(merge_overlapping(result));
    let mut regions: Vec<Region> = rows.into_iter().flatten().collect();
    regions.truncate(MAX_REGIONS);
    DetectResult { regions, gap, min_size, mode, background, labels_found }
}

/// 把区域等比缩放后居中放入 size×size 的正方形画布
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SquareOptions {
    /// 输出边长（像素），默认 256
    #[serde(default = "default_size")]
    pub size: u32,
    /// 四周留白占边长的百分比 0-40，默认 10
    #[serde(default = "default_padding")]
    pub padding: u8,
    /// 画布底色 [r,g,b]；为空时透明
    #[serde(default)]
    pub background: Option<[u8; 3]>,
}

fn default_size() -> u32 {
    256
}
fn default_padding() -> u8 {
    10
}

impl Default for SquareOptions {
    fn default() -> Self {
        Self { size: default_size(), padding: default_padding(), background: None }
    }
}

pub fn render_square(img: &DynamicImage, region: &Region, opts: &SquareOptions) -> DynamicImage {
    use image::imageops::{overlay, FilterType};
    let (iw, ih) = (img.width(), img.height());
    let x = region.x.min(iw.saturating_sub(1));
    let y = region.y.min(ih.saturating_sub(1));
    let cw = region.width.clamp(1, iw - x);
    let ch = region.height.clamp(1, ih - y);
    let crop = img.crop_imm(x, y, cw, ch);

    let size = opts.size.clamp(8, 8192);
    let inner = ((size as f32) * (1.0 - 2.0 * opts.padding.min(40) as f32 / 100.0)).round().max(1.0);
    let scale = inner / cw.max(ch) as f32;
    let tw = ((cw as f32 * scale).round() as u32).clamp(1, size);
    let th = ((ch as f32 * scale).round() as u32).clamp(1, size);
    let resized = crop.resize_exact(tw, th, FilterType::Lanczos3).to_rgba8();

    let fill = match opts.background {
        Some([r, g, b]) => image::Rgba([r, g, b, 255]),
        None => image::Rgba([0, 0, 0, 0]),
    };
    let mut canvas = RgbaImage::from_pixel(size, size, fill);
    overlay(&mut canvas, &resized, ((size - tw) / 2) as i64, ((size - th) / 2) as i64);
    DynamicImage::ImageRgba8(canvas)
}
#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    const INK: Rgba<u8> = Rgba([30, 30, 30, 255]);

    struct Canvas(RgbaImage);
    impl Canvas {
        fn new(w: u32, h: u32, bg: Rgba<u8>) -> Self {
            Canvas(RgbaImage::from_pixel(w, h, bg))
        }
        fn rect(&mut self, x: u32, y: u32, w: u32, h: u32) {
            for yy in y..y + h {
                for xx in x..x + w {
                    self.0.put_pixel(xx, yy, INK);
                }
            }
        }
        /// 空心矩形，线宽 t
        fn frame(&mut self, x: u32, y: u32, w: u32, h: u32, t: u32) {
            self.rect(x, y, w, t);
            self.rect(x, y + h - t, w, t);
            self.rect(x, y, t, h);
            self.rect(x + w - t, y, t, h);
        }
        fn ring(&mut self, cx: f32, cy: f32, r: f32, t: f32) {
            let (w, h) = self.0.dimensions();
            for y in 0..h {
                for x in 0..w {
                    let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                    if d <= r && d >= r - t {
                        self.0.put_pixel(x, y, INK);
                    }
                }
            }
        }
        fn img(self) -> DynamicImage {
            DynamicImage::ImageRgba8(self.0)
        }
    }

    /// 模拟底部导航栏：5 个图标（y 80..160），下方 2 字标签（y 185..205）
    fn nav_bar(bg: Rgba<u8>) -> DynamicImage {
        let mut c = Canvas::new(1000, 300, bg);
        let xs = [100u32, 280, 460, 640, 820];
        // 首页：空心方框
        c.frame(xs[0], 80, 80, 80, 6);
        // 发现：圆环 + 中间不相连的指针
        c.ring(xs[1] as f32 + 40.0, 120.0, 40.0, 6.0);
        c.rect(xs[1] + 34, 100, 12, 40);
        // 发布：方框 + 中间加号（与外框不相连）
        c.frame(xs[2], 80, 80, 80, 6);
        c.rect(xs[2] + 37, 95, 6, 50);
        c.rect(xs[2] + 15, 117, 50, 6);
        // 消息：气泡外框 + 3 个小点
        c.frame(xs[3], 85, 80, 70, 6);
        for i in 0..3 {
            c.rect(xs[3] + 18 + i * 18, 116, 8, 8);
        }
        // 我的：头（圆环）+ 与头分开 6px 的身体
        c.ring(xs[4] as f32 + 40.0, 100.0, 20.0, 6.0);
        c.rect(xs[4] + 10, 126, 60, 34);
        // 标签：每个 2 个"字"，字间距 6px
        for &x in &xs {
            c.rect(x + 19, 185, 18, 20);
            c.rect(x + 43, 185, 18, 20);
        }
        c.img()
    }

    const CLEAR: Rgba<u8> = Rgba([0, 0, 0, 0]);
    const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);

    #[test]
    fn include_labels_frames_icon_and_text() {
        let opts = DetectOptions { include_labels: true, ..Default::default() };
        let r = detect_regions(&nav_bar(CLEAR), &opts);
        assert_eq!(r.mode, "alpha");
        assert_eq!(r.regions.len(), 5, "{:?}", r.regions);
        assert_eq!(r.labels_found, 5);
        for (i, reg) in r.regions.iter().enumerate() {
            assert_eq!(reg.kind, RegionKind::IconWithLabel, "第 {i} 个");
            assert!(reg.y <= 85 && reg.bottom() >= 205, "第 {i} 个应包含标签: {reg:?}");
        }
        // 从左到右
        assert!(r.regions.windows(2).all(|p| p[0].x < p[1].x));
    }

    #[test]
    fn default_keeps_icons_only() {
        // 默认去掉文字标签，只保留图标
        let r = detect_regions(&nav_bar(CLEAR), &DetectOptions::default());
        assert_eq!(r.labels_found, 5, "文字仍会被识别出来再去掉");
        assert_eq!(r.regions.len(), 5, "{:?}", r.regions);
        for reg in &r.regions {
            assert_eq!(reg.kind, RegionKind::Icon);
            assert!(reg.bottom() < 185, "不应包含标签（标签从 y=185 开始）: {reg:?}");
            // "我的"宽 60，其余约 80；高度都约 70-81（头和身体分开 6px 也应合成一个）
            assert!((60..=81).contains(&reg.width) && (70..=81).contains(&reg.height), "图标尺寸 {reg:?}");
        }
    }

    #[test]
    fn white_background_matches_transparent() {
        let a = detect_regions(&nav_bar(CLEAR), &DetectOptions::default());
        let b = detect_regions(&nav_bar(WHITE), &DetectOptions::default());
        assert_eq!(b.mode, "color");
        assert_eq!(b.background, Some([255, 255, 255]));
        assert_eq!(a.regions, b.regions);
    }

    #[test]
    fn filters_noise() {
        let mut img = nav_bar(CLEAR).to_rgba8();
        img.put_pixel(5, 5, INK);
        img.put_pixel(6, 5, INK);
        let r = detect_regions(&DynamicImage::ImageRgba8(img), &DetectOptions::default());
        assert_eq!(r.regions.len(), 5);
    }

    #[test]
    fn touching_icons_merge_into_one() {
        // 已知限制：互相粘连的图标会被合成一个
        let mut c = Canvas::new(400, 200, CLEAR);
        c.frame(50, 50, 80, 80, 6);
        c.frame(126, 50, 80, 80, 6);
        let r = detect_regions(&c.img(), &DetectOptions::default());
        assert_eq!(r.regions.len(), 1);
    }

    #[test]
    fn grid_of_two_icon_rows_is_not_mistaken_for_labels() {
        let mut c = Canvas::new(600, 400, CLEAR);
        for row in 0..2u32 {
            for col in 0..3u32 {
                c.frame(60 + col * 180, 60 + row * 160, 80, 80, 6);
            }
        }
        let r = detect_regions(&c.img(), &DetectOptions::default());
        assert_eq!(r.regions.len(), 6);
        assert_eq!(r.labels_found, 0);
        // 先第一行从左到右，再第二行
        assert!(r.regions[..3].iter().all(|b| b.y < 100) && r.regions[3..].iter().all(|b| b.y > 200));
    }

    #[test]
    fn explicit_gap_controls_merging() {
        // 两个方框相距 30px：默认间距（15）分开，间距 40 时合并
        let mut c = Canvas::new(400, 200, CLEAR);
        c.frame(50, 50, 80, 80, 6);
        c.frame(160, 50, 80, 80, 6);
        let img = c.img();
        assert_eq!(detect_regions(&img, &DetectOptions::default()).regions.len(), 2);
        assert_eq!(detect_regions(&img, &DetectOptions { gap: Some(40), ..Default::default() }).regions.len(), 1);
    }

    #[test]
    fn render_square_centers_and_keeps_aspect() {
        let mut c = Canvas::new(300, 200, CLEAR);
        c.rect(100, 50, 100, 50); // 2:1 的实心块
        let img = c.img();
        let region = Region { x: 100, y: 50, width: 100, height: 50, kind: RegionKind::Icon };
        let out = render_square(&img, &region, &SquareOptions { size: 200, padding: 10, background: None }).to_rgba8();
        assert_eq!(out.dimensions(), (200, 200));
        // 内容宽 160、高 80，水平居中、垂直居中
        assert_eq!(out.get_pixel(100, 100)[3], 255);
        assert_eq!(out.get_pixel(25, 100)[3], 255);
        assert_eq!(out.get_pixel(15, 100)[3], 0, "左侧留白");
        assert_eq!(out.get_pixel(100, 50)[3], 0, "上方留白（内容只有 80 高）");
        assert_eq!(out.get_pixel(0, 0)[3], 0);

        let filled = render_square(&img, &region, &SquareOptions { size: 64, padding: 0, background: Some([255, 255, 255]) }).to_rgba8();
        assert_eq!(*filled.get_pixel(0, 0), Rgba([255, 255, 255, 255]));
    }
}
