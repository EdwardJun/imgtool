//! 精简文本树：一行一个节点，给 Agent 快速浏览（完整信息见 spec 文件）

use crate::model::{Align, CornerRadius, CrossAlign, Fill, FontWeight, LayoutMode, Node, Spec};

fn fill_str(f: &Fill) -> String {
    match f {
        Fill::Solid { color } => color.clone(),
        Fill::Linear { angle, stops } => format!("linear {}° {}→{}", angle, stops.first().map(|s| s.color.as_str()).unwrap_or(""), stops.last().map(|s| s.color.as_str()).unwrap_or("")),
    }
}

fn align_str(a: Align) -> &'static str {
    match a {
        Align::Start => "start",
        Align::Center => "center",
        Align::End => "end",
        Align::SpaceBetween => "space-between",
    }
}

fn cross_str(a: CrossAlign) -> &'static str {
    match a {
        CrossAlign::Start => "start",
        CrossAlign::Center => "center",
        CrossAlign::End => "end",
        CrossAlign::Stretch => "stretch",
        CrossAlign::Mixed => "mixed",
    }
}

pub fn node_line(n: &Node) -> String {
    let b = n.bounds;
    let kind = if n.is_plain_group() { "group" } else { n.node_type.as_str() };
    let mut s = format!("{} {kind}", n.id);
    if let Some(r) = &n.role {
        s += &format!(" [{r}]");
    }
    if let Some(name) = &n.name {
        s += &format!(" “{name}”");
    }
    s += &format!(" {},{} {}×{}", b.x, b.y, b.width, b.height);
    if let Some(t) = &n.text {
        let t: String = t.chars().take(40).collect();
        s += &format!(" text=“{t}”");
    }
    if let Some(f) = &n.style.fill {
        s += &format!(" fill {}", fill_str(f));
    }
    if let Some(st) = &n.style.stroke {
        s += &format!(" stroke {} {}", st.width, st.color);
    }
    match &n.style.corner_radius {
        Some(CornerRadius::Uniform(r)) => s += &format!(" r{r}"),
        Some(CornerRadius::Corners(c)) => s += &format!(" r{}/{}/{}/{}", c[0], c[1], c[2], c[3]),
        None => {}
    }
    if let Some(sh) = &n.style.shadow {
        s += &format!(" shadow {},{} blur{} {}", sh.x, sh.y, sh.blur, sh.color);
    }
    if let Some(t) = &n.text_style {
        let w = match t.weight {
            FontWeight::Regular => "",
            FontWeight::Medium => " medium",
            FontWeight::Bold => " bold",
        };
        s += &format!(" {}px{w} {}", t.font_size, t.color);
        if t.lines > 1 {
            s += &format!(" {}行 lh{}", t.lines, t.line_height.unwrap_or(0));
        }
    }
    if let Some(l) = &n.layout {
        let m = match l.mode {
            LayoutMode::Row => "row",
            LayoutMode::Column => "column",
            LayoutMode::None => "free",
        };
        s += &format!(" | {m}");
        if let Some(g) = l.gap {
            s += &format!(" gap{g}{}", if l.gap_uniform { "" } else { "~" });
        }
        if l.padding.iter().any(|&p| p > 0) {
            let p = l.padding;
            s += &format!(" pad {},{},{},{}", p[0], p[1], p[2], p[3]);
        }
        s += &format!(" {}/{}", align_str(l.align), cross_str(l.cross_align));
    }
    if let Some(a) = &n.asset {
        s += &format!(" asset={}", a.path);
    }
    s
}

/// 缩进文本树；节点过多时截断
pub fn outline(spec: &Spec, max_nodes: usize) -> String {
    let mut lines = Vec::new();
    let total = spec.root.count();
    spec.root.walk(&mut |n, depth| {
        if lines.len() < max_nodes {
            lines.push(format!("{}{}", "  ".repeat(depth), node_line(n)));
        }
    });
    if total > max_nodes {
        lines.push(format!("…… 还有 {} 个节点未列出，完整内容见 spec 文件", total - max_nodes));
    }
    lines.join("\n")
}
