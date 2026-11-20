//! 布局推断（方向、间距、内边距、对齐）、相对坐标、节点编号，以及编辑后的层级整理

use crate::group::{self, placeholder};
use crate::model::{Align, Bounds, CrossAlign, Layout, LayoutMode, Node, NodeSource, NodeType, Point, Spec, TextAlign};

fn median(v: &mut [u32]) -> Option<u32> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[v.len() / 2])
}

fn infer_mode(children: &[Node]) -> LayoutMode {
    if children.len() == 1 {
        return LayoutMode::Row;
    }
    let disjoint = |vertical: bool| {
        let mut s: Vec<(u32, u32)> = children.iter().map(|c| if vertical { (c.bounds.y, c.bounds.bottom()) } else { (c.bounds.x, c.bounds.right()) }).collect();
        s.sort_unstable();
        s.windows(2).all(|w| w[1].0 >= w[0].1)
    };
    if disjoint(true) {
        LayoutMode::Column
    } else if disjoint(false) {
        LayoutMode::Row
    } else {
        LayoutMode::None
    }
}

fn compute(node: &mut Node) {
    for c in &mut node.children {
        compute(c);
    }
    if node.children.is_empty() {
        node.layout = None;
        if node.node_type == NodeType::Frame && !node.style.is_empty() {
            node.node_type = NodeType::Rect;
        }
        return;
    }
    let mode = match node.layout.as_ref().map(|l| l.mode) {
        Some(m) if m != LayoutMode::None || node.children.len() > 1 => m,
        _ => infer_mode(&node.children),
    };
    let vertical = mode == LayoutMode::Column;
    match mode {
        LayoutMode::Column => node.children.sort_by_key(|c| (c.bounds.y, c.bounds.x)),
        LayoutMode::Row => node.children.sort_by_key(|c| (c.bounds.x, c.bounds.y)),
        LayoutMode::None => node.children.sort_by_key(|c| (c.bounds.y, c.bounds.x)),
    }
    let group = node.is_plain_group();
    let content = if group { node.bounds } else { node.content_bounds() };
    let u = group::union_all(&node.children).unwrap();
    let padding = [u.y.saturating_sub(content.y), content.right().saturating_sub(u.right()), content.bottom().saturating_sub(u.bottom()), u.x.saturating_sub(content.x)];
    let mut gaps: Vec<u32> = if mode == LayoutMode::None {
        Vec::new()
    } else {
        node.children
            .windows(2)
            .map(|w| if vertical { w[1].bounds.y.saturating_sub(w[0].bounds.bottom()) } else { w[1].bounds.x.saturating_sub(w[0].bounds.right()) })
            .collect()
    };
    let gap = median(&mut gaps);
    let gap_uniform = match gap {
        Some(g) => gaps.iter().all(|&x| (x as f32 - g as f32).abs() <= (0.1 * g as f32).max(2.0)),
        None => true,
    };
    let (ps, pe) = if vertical { (padding[0], padding[2]) } else { (padding[3], padding[1]) };
    let main_len = if vertical { content.height } else { content.width };
    let align = if mode == LayoutMode::None || group {
        Align::Start
    } else if (ps as f32 - pe as f32).abs() <= (0.04 * main_len as f32).max(2.0) {
        if ps <= 1 {
            Align::Start
        } else if node.children.len() == 2 && gap.map(|g| g > 2 * ps.max(pe) + 8).unwrap_or(false) {
            Align::SpaceBetween
        } else {
            Align::Center
        }
    } else if ps < pe {
        Align::Start
    } else {
        Align::End
    };
    let cross = |b: &Bounds| if vertical { (b.x, b.right()) } else { (b.y, b.bottom()) };
    let (cs, ce) = cross(&content);
    let leads: Vec<i64> = node.children.iter().map(|c| cross(&c.bounds).0 as i64 - cs as i64).collect();
    let trails: Vec<i64> = node.children.iter().map(|c| ce as i64 - cross(&c.bounds).1 as i64).collect();
    let centers: Vec<i64> = leads.iter().zip(&trails).map(|(l, t)| l - t).collect();
    let same = |v: &[i64]| v.iter().max().unwrap() - v.iter().min().unwrap() <= 2;
    let cross_align = if node.children.len() == 1 {
        let (l, t) = (leads[0], trails[0]);
        if l <= 1 && t <= 1 {
            CrossAlign::Stretch
        } else if (l - t).abs() <= 2 {
            CrossAlign::Center
        } else if l < t {
            CrossAlign::Start
        } else {
            CrossAlign::End
        }
    } else if same(&leads) && same(&trails) {
        CrossAlign::Stretch
    } else if same(&centers) {
        CrossAlign::Center
    } else if same(&leads) {
        CrossAlign::Start
    } else if same(&trails) {
        CrossAlign::End
    } else {
        CrossAlign::Mixed
    };
    // 单行文字的对齐方式跟随容器
    let center_text = (mode == LayoutMode::Row && node.children.len() == 1 && align == Align::Center) || (vertical && cross_align == CrossAlign::Center);
    if center_text {
        for c in &mut node.children {
            if let Some(ts) = c.text_style.as_mut() {
                if ts.lines == 1 {
                    ts.align = TextAlign::Center;
                }
            }
        }
    }
    node.layout = Some(Layout { mode, gap, gap_uniform, padding, align, cross_align });
}

fn set_relative(node: &mut Node, parent: Option<Bounds>) {
    node.relative = match parent {
        Some(p) => Point { x: node.bounds.x as i64 - p.x as i64, y: node.bounds.y as i64 - p.y as i64 },
        None => Point::default(),
    };
    let b = node.bounds;
    for c in &mut node.children {
        set_relative(c, Some(b));
    }
}

/// 计算布局与相对坐标
pub fn finalize(root: &mut Node) {
    compute(root);
    set_relative(root, None);
}

/// 深度优先（前序）编号：n1, n2, …
pub fn assign_ids(root: &mut Node) {
    let mut k = 0;
    root.walk_mut(&mut |n| {
        k += 1;
        n.id = format!("n{k}");
    });
}

fn id_num(id: &str) -> u32 {
    id.strip_prefix('n').and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// 可以作为父容器的节点：有底色/描边的色块，或带语义的分组
fn is_container(n: &Node) -> bool {
    matches!(n.node_type, NodeType::Frame | NodeType::Rect) && (!n.style.is_empty() || n.has_semantics() || n.source != NodeSource::Auto)
}

/// 编辑（移动、缩放、增删）之后重新整理层级：按包含关系重新嵌套，重新分组并推断布局。
/// 已有节点的编号不变，新增的分组和没有编号的节点接着最大编号往后排。
pub fn normalize(spec: &mut Spec) {
    let mut flat: Vec<Node> = Vec::new();
    let mut root = std::mem::replace(&mut spec.root, Node::new(NodeType::Frame, Bounds::default()));
    fn collect(n: Node, out: &mut Vec<Node>) {
        let mut n = n;
        let kids = std::mem::take(&mut n.children);
        if !n.is_plain_group() {
            n.layout = None;
            out.push(n);
        }
        for k in kids {
            collect(k, out);
        }
    }
    for k in std::mem::take(&mut root.children) {
        collect(k, &mut flat);
    }
    let mut max_id = id_num(&root.id);
    for n in &flat {
        max_id = max_id.max(id_num(&n.id));
    }
    // 大的先放，小的找最小的包含它的容器
    flat.sort_by_key(|n| std::cmp::Reverse(n.bounds.area()));
    let mut parent: Vec<Option<usize>> = vec![None; flat.len()];
    for i in 0..flat.len() {
        let mut best: Option<usize> = None;
        for j in 0..i {
            if is_container(&flat[j])
                && flat[j].bounds.area() > flat[i].bounds.area()
                && flat[j].bounds.contains_ratio(&flat[i].bounds) >= 0.9
                && best.map(|b| flat[j].bounds.area() < flat[b].bounds.area()).unwrap_or(true)
            {
                best = Some(j);
            }
        }
        parent[i] = best;
    }
    // 自底向上组装
    let mut slots: Vec<Option<Node>> = flat.into_iter().map(Some).collect();
    let mut kids: Vec<Vec<Node>> = vec![Vec::new(); slots.len()];
    let mut top: Vec<Node> = Vec::new();
    for i in (0..slots.len()).rev() {
        let mut n = slots[i].take().unwrap();
        let children = std::mem::take(&mut kids[i]);
        if !children.is_empty() {
            let (c, mode) = group::xy_cut(children);
            n.children = c;
            n.layout = Some(placeholder(mode));
            if n.node_type == NodeType::Rect {
                n.node_type = NodeType::Frame;
            }
        }
        match parent[i] {
            Some(p) => kids[p].push(n),
            None => top.push(n),
        }
    }
    let (c, mode) = group::xy_cut(top);
    root.children = c;
    root.layout = Some(placeholder(mode));
    root.walk_mut(&mut |n| {
        if n.id.is_empty() {
            max_id += 1;
            n.id = format!("n{max_id}");
        }
    });
    finalize(&mut root);
    spec.root = root;
}
