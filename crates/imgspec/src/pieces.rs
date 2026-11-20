//! 叶子元素：把零散的前景连通块合并成图标、文字行，再分类为 text / icon / image / line

use crate::canvas::{Bg, Canvas};
use crate::color::{self, Rgb};
use crate::model::{Bounds, Fill, Node, NodeType, TextAlign, TextStyle};
use crate::segment::{Comp, Ctx};
use crate::style;

pub(crate) struct Piece {
    pub b: Bounds,
    pub pix: Vec<u32>,
    /// 由几个连通块合并而来
    pub parts: u32,
    /// 墨迹颜色（与底色差异最大的那部分像素的平均色）
    pub color: Rgb,
    /// 大而实心的块（照片等），不参与文字合并
    pub solid: bool,
}

fn ink_color(cv: &Canvas, pix: &[u32], bg: &Bg) -> Rgb {
    let step = (pix.len() / 5000).max(1);
    let mut v: Vec<(f32, Rgb)> = pix
        .iter()
        .step_by(step)
        .map(|&i| {
            let (x, y) = (i % cv.w, i / cv.w);
            let p = cv.get(x, y);
            (color::dist(p, bg.at(x, y)), p)
        })
        .collect();
    v.sort_by(|a, b| b.0.total_cmp(&a.0));
    let n = (v.len() * 3 / 10).max(1);
    let mut s = [0f32; 3];
    for (_, p) in &v[..n] {
        for i in 0..3 {
            s[i] += p[i] / n as f32;
        }
    }
    s
}

impl Piece {
    pub fn from_comp(ctx: &Ctx, c: Comp, bg: &Bg) -> Piece {
        let color = ink_color(ctx.cv, &c.pix, bg);
        let solid = c.pix.len() as f32 >= 0.85 * c.b.area() as f32 && c.b.width.min(c.b.height) >= 12;
        Piece { b: c.b, pix: c.pix, parts: 1, color, solid }
    }
}

fn find(p: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while p[r] != r {
        r = p[r];
    }
    let mut i = i;
    while p[i] != r {
        let n = p[i];
        p[i] = r;
        i = n;
    }
    r
}

/// 一轮合并：按 x 排序后只比较横向可能相邻的块
fn merge_pass(mut ps: Vec<Piece>, reach: impl Fn(&Piece) -> u32, pred: impl Fn(&Piece, &Piece) -> bool) -> (Vec<Piece>, bool) {
    let n = ps.len();
    ps.sort_by_key(|p| p.b.x);
    let mut parent: Vec<usize> = (0..n).collect();
    for i in 0..n {
        let lim = ps[i].b.right() + reach(&ps[i]);
        for j in i + 1..n {
            if ps[j].b.x > lim {
                break;
            }
            if pred(&ps[i], &ps[j]) {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[b] = a;
                }
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for i in 0..n {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    if groups.len() == n {
        return (ps, false);
    }
    let mut slots: Vec<Option<Piece>> = ps.into_iter().map(Some).collect();
    let mut out = Vec::with_capacity(groups.len());
    for (_, idx) in groups {
        let mut it = idx.into_iter();
        let mut acc = slots[it.next().unwrap()].take().unwrap();
        for k in it {
            let p = slots[k].take().unwrap();
            let (na, nb) = (acc.pix.len() as f32, p.pix.len() as f32);
            acc.color = [0, 1, 2].map(|i| (acc.color[i] * na + p.color[i] * nb) / (na + nb));
            acc.b = acc.b.union(&p.b);
            acc.parts += p.parts;
            acc.solid = false;
            acc.pix.extend(p.pix);
        }
        out.push(acc);
    }
    (out, true)
}

fn merge_until_stable(mut ps: Vec<Piece>, reach: impl Fn(&Piece) -> u32, pred: impl Fn(&Piece, &Piece) -> bool) -> Vec<Piece> {
    for _ in 0..8 {
        let (next, changed) = merge_pass(ps, &reach, &pred);
        ps = next;
        if !changed {
            break;
        }
    }
    ps
}

pub(crate) fn gap_x(a: &Bounds, b: &Bounds) -> u32 {
    b.x.saturating_sub(a.right()).max(a.x.saturating_sub(b.right()))
}
pub(crate) fn gap_y(a: &Bounds, b: &Bounds) -> u32 {
    b.y.saturating_sub(a.bottom()).max(a.y.saturating_sub(b.bottom()))
}
pub(crate) fn v_overlap(a: &Bounds, b: &Bounds) -> u32 {
    a.bottom().min(b.bottom()).saturating_sub(a.y.max(b.y))
}

pub(crate) fn text_max_h(ctx: &Ctx) -> f32 {
    // 大标题在 1K 竖屏稿上可到 100px 以上
    (0.12 * ctx.short).max(0.06 * ctx.long).max(24.0)
}

/// 合并：重叠 → 近邻（同一图标的笔画）→ 同一行的字
pub(crate) fn merge(ctx: &Ctx, ps: Vec<Piece>) -> Vec<Piece> {
    // AI 生成图的压缩噪点会把相邻的行"串"起来：先丢掉零星的小噪点
    let ps: Vec<Piece> = ps.into_iter().filter(|p| p.pix.len() >= 5 || p.b.width.max(p.b.height) > 3).collect();
    let ps = merge_until_stable(ps, |_| 0, |a, b| a.b.intersection(&b.b).is_some());
    let near = ((ctx.long * 0.003).round() as u32).max(2);
    // 近邻合并：零星小块只并入离它最近的大块，避免经由一串小块把上下两行字连起来；
    // 大块之间合并后的高度不超过较高者的 1.5 倍（同一图标的笔画本来就交叠）
    let speck = near * 2;
    let (specks, bigs): (Vec<Piece>, Vec<Piece>) = ps.into_iter().partition(|p| !p.solid && p.b.width.max(p.b.height) <= speck);
    let mut bigs = merge_until_stable(
        bigs,
        |_| near,
        |a, b| {
            !a.solid && !b.solid && gap_x(&a.b, &b.b) <= near && gap_y(&a.b, &b.b) <= near && {
                let u = a.b.union(&b.b);
                let hmax = a.b.height.max(b.b.height) as f32;
                (u.height as f32) <= 1.5 * hmax || v_overlap(&a.b, &b.b) > 0
            }
        },
    );
    let originals: Vec<Bounds> = bigs.iter().map(|p| p.b).collect();
    let mut rest = Vec::new();
    for s in specks {
        let best = originals
            .iter()
            .enumerate()
            .filter(|(i, b)| !bigs[*i].solid && gap_x(b, &s.b) <= near && gap_y(b, &s.b) <= near)
            .min_by_key(|(_, b)| gap_x(b, &s.b) + gap_y(b, &s.b))
            .map(|(i, _)| i);
        match best {
            Some(i) => {
                // 只并入像素，不扩大外接框（噪点会把文字框撑高，导致字号偏大）
                let t = &mut bigs[i];
                t.parts += s.parts;
                t.pix.extend(s.pix.into_iter().filter(|&q| {
                    let (x, y) = (q % ctx.cv.w, q / ctx.cv.w);
                    x >= t.b.x && y >= t.b.y && x < t.b.right() && y < t.b.bottom()
                }));
            }
            None => rest.push(s),
        }
    }
    bigs.extend(rest);
    let ps = bigs;
    let tmax = text_max_h(ctx);
    // 大图标旁边的同色小部件（轮子、角标里的点）
    let ps = merge_until_stable(
        ps,
        |p| p.b.width.max(p.b.height) / 4 + 1,
        |a, b| {
            if a.solid || b.solid {
                return false;
            }
            let (ma, mb) = (a.b.width.max(a.b.height) as f32, b.b.width.max(b.b.height) as f32);
            let (big, small) = (ma.max(mb), ma.min(mb));
            let bp = if ma >= mb { &a.b } else { &b.b };
            let aspect = bp.width as f32 / bp.height as f32;
            // 只针对接近方形的大图形；文字行之间不走这条规则
            if big <= tmax || small > 0.5 * big || !(0.5..=2.0).contains(&aspect) {
                return false;
            }
            let gap = gap_x(&a.b, &b.b).max(gap_y(&a.b, &b.b)) as f32;
            gap <= 0.25 * big && color::delta_e(color::to_u8(a.color), color::to_u8(b.color)) < 15.0
        },
    );
    merge_until_stable(
        ps,
        |p| p.b.height / 2 + 1,
        |a, b| {
            if a.solid || b.solid {
                return false;
            }
            let (ha, hb) = (a.b.height as f32, b.b.height as f32);
            let (hs, hl) = (ha.min(hb), ha.max(hb));
            if hl > tmax {
                return false;
            }
            let ov = v_overlap(&a.b, &b.b) as f32;
            let similar = hl <= 1.6 * hs && ov >= 0.5 * hs;
            // 标点、i 的点等小块：落在大块的高度范围内
            let small_inside = hs < 0.6 * hl && ov >= 0.8 * hs;
            if !(similar || small_inside) {
                return false;
            }
            if gap_x(&a.b, &b.b) as f32 > 0.5 * hl {
                return false;
            }
            color::delta_e(color::to_u8(a.color), color::to_u8(b.color)) <= 25.0
        },
    )
}

/// 颜色复杂度：占比不可忽略的量化颜色数。只统计与底色差异较大的像素，
/// 排除图标、文字边缘的抗锯齿过渡色
fn color_count(cv: &Canvas, pix: &[u32], bg: &Bg, ink: Rgb) -> usize {
    let step = (pix.len() / 20000).max(1);
    let mut map: std::collections::HashMap<u32, u32> = Default::default();
    let mut n = 0u32;
    let mut far = 0f32;
    for &i in pix.iter().step_by(step) {
        let (x, y) = (i % cv.w, i / cv.w);
        far = far.max(color::dist(cv.get(x, y), bg.at(x, y)));
    }
    let strong = 0.6 * far.max(color::dist(ink, bg.mean()));
    for &i in pix.iter().step_by(step) {
        let (x, y) = (i % cv.w, i / cv.w);
        let c = cv.get(x, y);
        if color::dist(c, bg.at(x, y)) < strong {
            continue;
        }
        *map.entry(color::bucket(c, 4)).or_default() += 1;
        n += 1;
    }
    let min = (n / 500).max(2);
    map.values().filter(|&&c| c >= min).count()
}

pub(crate) fn classify(ctx: &Ctx, p: &Piece, bg: &Bg) -> NodeType {
    let (w, h) = (p.b.width as f32, p.b.height as f32);
    let aspect = w / h;
    let lt = ctx.line_thick() as f32;
    if (h <= lt && aspect >= 6.0) || (w <= lt && aspect <= 1.0 / 6.0) {
        return NodeType::Line;
    }
    let colors = color_count(ctx.cv, &p.pix, bg, p.color);
    let area_frac = p.b.area() as f32 / (ctx.cv.w as f32 * ctx.cv.h as f32);
    if colors >= 48 || (p.solid && colors >= 20) || (area_frac >= 0.02 && colors >= 24) {
        return NodeType::Image;
    }
    let density = p.pix.len() as f32 / p.b.area() as f32;
    let tmax = text_max_h(ctx);
    if h <= tmax && ((p.parts >= 2 && aspect >= 1.6) || (aspect >= 2.5 && (0.12..0.7).contains(&density))) {
        return NodeType::Text;
    }
    // 几个字并排（如徽标里的「01」）：按列投影能切成 ≥2 段、每段都几乎占满整个高度
    if h <= tmax && aspect >= 0.8 && glyph_columns(ctx.cv, p, bg) >= 2 {
        return NodeType::Text;
    }
    // 太小的实心点 / 短块（列表圆点、装饰点）只是色块，不是图标：放大后没有可辨认的细节
    if w.max(h) < min_icon_px(ctx) {
        return NodeType::Rect;
    }
    let icon_max = (0.15 * ctx.short).max(24.0);
    if w.max(h) <= icon_max && (0.4..=2.5).contains(&aspect) {
        return NodeType::Icon;
    }
    // 颜色很少的大图形（大尺寸的线性 / 面性图标）
    if colors <= 20 && w.max(h) <= 0.45 * ctx.short && (0.4..=2.5).contains(&aspect) {
        return NodeType::Icon;
    }
    if h <= tmax && aspect >= 1.2 && p.parts >= 2 {
        return NodeType::Text;
    }
    if w.max(h) <= icon_max {
        return NodeType::Icon;
    }
    NodeType::Image
}

/// 图标的最小边长：再小就没有可辨认的造型（长边约为画面短边的 1.2%，至少 12px）
pub(crate) fn min_icon_px(ctx: &Ctx) -> f32 {
    (0.012 * ctx.short).max(12.0)
}

/// 按列投影切段：空列把墨迹分成几段，统计纵向几乎占满整块高度的段数（并排的字）
fn glyph_columns(cv: &Canvas, p: &Piece, bg: &Bg) -> usize {
    let b = p.b;
    // 只看"实"的墨迹像素，字与字之间的抗锯齿过渡不算
    let (cx, cy) = b.center();
    let strong = 0.6 * color::dist(p.color, bg.at(cx as u32, cy as u32));
    let (w, h) = (b.width as usize, b.height as usize);
    let mut col_top = vec![usize::MAX; w];
    let mut col_bot = vec![0usize; w];
    for &i in &p.pix {
        let (gx, gy) = (i % cv.w, i / cv.w);
        if color::dist(cv.get(gx, gy), bg.at(gx, gy)) < strong {
            continue;
        }
        let (x, y) = ((gx - b.x) as usize, (gy - b.y) as usize);
        col_top[x] = col_top[x].min(y);
        col_bot[x] = col_bot[x].max(y);
    }
    let mut runs = 0;
    let mut minor = 0usize;
    let mut x = 0;
    while x < w {
        if col_top[x] == usize::MAX {
            x += 1;
            continue;
        }
        let (mut top, mut bot, start) = (col_top[x], col_bot[x], x);
        while x < w && col_top[x] != usize::MAX {
            top = top.min(col_top[x]);
            bot = bot.max(col_bot[x]);
            x += 1;
        }
        let tall = (bot - top + 1) as f32 >= 0.75 * h as f32;
        let wide = (x - start) as f32 >= 0.08 * h as f32;
        if tall && wide {
            runs += 1;
        } else {
            // 偏旁、标点这类矮块允许存在，但不能占太多宽度
            minor += x - start;
        }
    }
    if (minor as f32) > 0.3 * w as f32 {
        return 0;
    }
    runs
}

fn rows_of(cv: &Canvas, p: &Piece) -> Vec<Vec<bool>> {
    let b = p.b;
    let mut rows = vec![vec![false; b.width as usize]; b.height as usize];
    for &i in &p.pix {
        let (x, y) = (i % cv.w, i / cv.w);
        rows[(y - b.y) as usize][(x - b.x) as usize] = true;
    }
    rows
}

pub(crate) fn into_nodes(ctx: &Ctx, ps: Vec<Piece>, bg: &Bg) -> Vec<Node> {
    let min = ((ctx.long * 0.004).round() as u32).max(3);
    merge(ctx, ps)
        .into_iter()
        // 噪点
        .filter(|p| p.b.width.max(p.b.height) >= min && p.pix.len() >= 6)
        .map(|p| {
            let t = classify(ctx, &p, bg);
            let mut n = Node::new(t, p.b);
            n.confidence = 0.6;
            let c = color::to_u8(p.color);
            match t {
                NodeType::Text => {
                    let ink = p.b.height as f32;
                    n.text_style = Some(TextStyle {
                        color: color::hex(c),
                        font_size: style::font_size_from_ink(ink).round() as u32,
                        line_height: None,
                        lines: 1,
                        weight: style::weight_from_runs(rows_of(ctx.cv, &p).into_iter(), ink),
                        align: TextAlign::Left,
                        confidence: 0.5,
                    });
                }
                NodeType::Icon | NodeType::Line | NodeType::Rect => n.style.fill = Some(Fill::solid(c)),
                _ => {}
            }
            n
        })
        .collect()
}
