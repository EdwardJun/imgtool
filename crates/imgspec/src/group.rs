//! 递归 XY-cut 分组：按投影空隙把兄弟元素切成行 / 列，得到类似 auto-layout 的嵌套结构；
//! 竖排里相邻的文字行合并成段落。

use crate::color;
use crate::model::{Align, Bounds, CrossAlign, Layout, LayoutMode, Node, NodeType, TextAlign};

pub(crate) fn placeholder(mode: LayoutMode) -> Layout {
    Layout { mode, gap: None, gap_uniform: true, padding: [0; 4], align: Align::Start, cross_align: CrossAlign::Start }
}

fn span(b: &Bounds, vertical: bool) -> (u32, u32) {
    if vertical {
        (b.y, b.bottom())
    } else {
        (b.x, b.right())
    }
}

/// 沿一个方向切成互不重叠的带；返回每条带的下标和带之间最大的空隙
fn bands(nodes: &[Node], vertical: bool) -> (Vec<Vec<usize>>, u32) {
    let mut idx: Vec<usize> = (0..nodes.len()).collect();
    idx.sort_by_key(|&i| span(&nodes[i].bounds, vertical));
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut end = 0u32;
    let mut max_gap = 0u32;
    for i in idx {
        let (s, e) = span(&nodes[i].bounds, vertical);
        match out.last_mut() {
            Some(band) if s < end => {
                band.push(i);
                end = end.max(e);
            }
            _ => {
                if !out.is_empty() {
                    max_gap = max_gap.max(s - end);
                }
                out.push(vec![i]);
                end = e;
            }
        }
    }
    (out, max_gap)
}

pub(crate) fn union_all(nodes: &[Node]) -> Option<Bounds> {
    nodes.iter().map(|n| n.bounds).reduce(|a, b| a.union(&b))
}

/// 按 XY-cut 分组；返回子节点和它们在父容器里的排列方向
pub(crate) fn xy_cut(nodes: Vec<Node>) -> (Vec<Node>, LayoutMode) {
    if nodes.len() <= 1 {
        let mode = if nodes.is_empty() { LayoutMode::None } else { LayoutMode::Row };
        return (nodes, mode);
    }
    let (by, gy) = bands(&nodes, true);
    let (bx, gx) = bands(&nodes, false);
    let vertical = match (by.len() >= 2, bx.len() >= 2) {
        (true, true) => gy >= gx,
        (true, false) => true,
        (false, true) => false,
        (false, false) => return (sort_reading(nodes), LayoutMode::None),
    };
    if !vertical {
        return build(nodes, bx, false);
    }
    let merged = merge_aligned_rows(&nodes, by.clone());
    if merged.len() >= 2 {
        return build(nodes, merged, true);
    }
    // 整体是一组"图标在上、文字在下"的条目：改为横向切
    if bx.len() >= 2 {
        return build(nodes, bx, false);
    }
    build(nodes, by, true)
}

/// 竖排里相邻的两行，元素个数相同且一一对齐（如导航栏的图标行和文字行）：合成一条，交给横向切出每个条目
fn merge_aligned_rows(nodes: &[Node], bands_v: Vec<Vec<usize>>) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    for band in bands_v {
        if let Some(prev) = out.last_mut() {
            if aligned(nodes, prev, &band) {
                prev.extend(band);
                continue;
            }
        }
        out.push(band);
    }
    out
}

fn aligned(nodes: &[Node], a: &[usize], b: &[usize]) -> bool {
    if a.len() < 2 || a.len() != b.len() {
        return false;
    }
    // 两行本身都要能横向切开成同样多的列
    let row = |idx: &[usize]| {
        let mut v: Vec<&Node> = idx.iter().map(|&i| &nodes[i]).collect();
        v.sort_by_key(|n| n.bounds.x);
        v
    };
    let (ra, rb) = (row(a), row(b));
    let ua = ra.iter().map(|n| n.bounds).reduce(|x, y| x.union(&y)).unwrap();
    let ub = rb.iter().map(|n| n.bounds).reduce(|x, y| x.union(&y)).unwrap();
    let gap = ub.y.saturating_sub(ua.bottom());
    if gap > ua.height.max(ub.height) {
        return false;
    }
    ra.iter().zip(&rb).all(|(p, q)| {
        let (pc, qc) = (p.bounds.center().0, q.bounds.center().0);
        (pc - qc).abs() <= (0.3 * p.bounds.width.min(q.bounds.width) as f32).max(6.0)
    }) && ra.windows(2).all(|w| w[1].bounds.x >= w[0].bounds.right())
}

fn build(nodes: Vec<Node>, chosen: Vec<Vec<usize>>, vertical: bool) -> (Vec<Node>, LayoutMode) {
    let mut slots: Vec<Option<Node>> = nodes.into_iter().map(Some).collect();
    let mut children = Vec::with_capacity(chosen.len());
    for band in chosen {
        let mut members: Vec<Node> = band.iter().map(|&i| slots[i].take().unwrap()).collect();
        if members.len() == 1 {
            children.push(members.pop().unwrap());
            continue;
        }
        let (kids, mode) = xy_cut(members);
        if kids.len() == 1 {
            children.extend(kids);
            continue;
        }
        let mut g = Node::new(NodeType::Frame, union_all(&kids).unwrap());
        g.layout = Some(placeholder(mode));
        g.children = kids;
        children.push(g);
    }
    if vertical {
        children = merge_paragraphs(children);
        if children.len() == 1 {
            let only = children.pop().unwrap();
            return (vec![only], LayoutMode::Row);
        }
    }
    (children, if vertical { LayoutMode::Column } else { LayoutMode::Row })
}

fn sort_reading(mut nodes: Vec<Node>) -> Vec<Node> {
    nodes.sort_by_key(|n| (n.bounds.y, n.bounds.x));
    nodes
}

/// 竖排里相邻、字号与颜色接近、对齐一致、行距正常的文字行合并为一个多行文字节点
fn merge_paragraphs(children: Vec<Node>) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::with_capacity(children.len());
    for n in children {
        if let Some(prev) = out.last_mut() {
            if let Some(align) = paragraph_align(prev, &n) {
                let (pt, nt) = (prev.text_style.clone().unwrap(), n.text_style.clone().unwrap());
                let lines = pt.lines + nt.lines;
                let pitch = (n.bounds.y - prev.bounds.y) as f32 / pt.lines as f32;
                let mut ts = pt.clone();
                ts.font_size = ((pt.font_size * pt.lines + nt.font_size * nt.lines) as f32 / lines as f32).round() as u32;
                ts.line_height = Some(pitch.round() as u32);
                ts.lines = lines;
                ts.align = align;
                prev.bounds = prev.bounds.union(&n.bounds);
                prev.text_style = Some(ts);
                continue;
            }
        }
        out.push(n);
    }
    out
}

fn paragraph_align(a: &Node, b: &Node) -> Option<TextAlign> {
    if a.node_type != NodeType::Text || b.node_type != NodeType::Text {
        return None;
    }
    let (ta, tb) = (a.text_style.as_ref()?, b.text_style.as_ref()?);
    let fa = ta.font_size as f32;
    let fb = tb.font_size as f32;
    if (fa - fb).abs() > 0.15 * fa.max(fb) {
        return None;
    }
    let (ca, cb) = (color::parse_hex(&ta.color)?, color::parse_hex(&tb.color)?);
    if color::delta_e(ca, cb) > 12.0 {
        return None;
    }
    let line = (a.bounds.height as f32 / ta.lines as f32).max(1.0);
    let gap = b.bounds.y.saturating_sub(a.bounds.bottom()) as f32;
    if gap > 0.9 * line {
        return None;
    }
    let tol = (0.6 * fa).max(2.0);
    let (al, bl) = (a.bounds.x as f32, b.bounds.x as f32);
    let (ar, br) = (a.bounds.right() as f32, b.bounds.right() as f32);
    let (ac, bc) = (a.bounds.center().0, b.bounds.center().0);
    let already = (ta.lines > 1).then_some(ta.align);
    let left = (al - bl).abs() <= tol;
    let center = (ac - bc).abs() <= tol;
    let right = (ar - br).abs() <= tol;
    match already {
        Some(TextAlign::Left) if left => Some(TextAlign::Left),
        Some(TextAlign::Center) if center => Some(TextAlign::Center),
        Some(TextAlign::Right) if right => Some(TextAlign::Right),
        Some(_) => None,
        // 两行：左对齐优先（最后一行通常较短）
        None if left => Some(TextAlign::Left),
        None if center => Some(TextAlign::Center),
        None if right => Some(TextAlign::Right),
        None => None,
    }
}
