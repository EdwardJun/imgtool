//! spec 数据模型（version 1）：简化版的 Figma 节点树。
//!
//! - 坐标一律是原图像素；`bounds` 是相对整张图的绝对坐标，`relative` 是相对父节点左上角的偏移
//! - 几何与样式由本地算法得出（`source = auto`），语义字段（role/name/text/description）由 Agent 或人工补充
//! - 颜色统一用 `#rrggbb`，带透明度时用 `#rrggbbaa`

use serde::{Deserialize, Serialize};

pub const SPEC_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    pub version: u32,
    /// 生成该 spec 的程序，如 "imgspec 0.1.0"
    pub generator: String,
    pub source: SourceInfo,
    pub kind: SpecKind,
    /// 画布背景（component 模式下是组件周围的底色；透明底的图记为 transparent=true）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Fill>,
    #[serde(default)]
    pub transparent: bool,
    /// 主要颜色及面积占比，按占比从大到小
    #[serde(default)]
    pub palette: Vec<PaletteEntry>,
    /// 是否已经有 Agent 或人工补充过语义
    #[serde(default)]
    pub annotated: bool,
    /// Agent 或人工对整张设计稿的说明
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub root: Node,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceInfo {
    /// 原图路径（相对项目根目录，统一用 /）
    pub path: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SpecKind {
    /// 整页设计稿（App / 网页）
    #[default]
    Page,
    /// 单个样式（卡片、按钮、标签、输入框等）
    Component,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaletteEntry {
    pub color: String,
    /// 面积占比 0-1
    pub ratio: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    /// 容器：有子节点（带底色的卡片，或无底色的分组）
    Frame,
    /// 没有子节点的色块（分割块、色条、头像底等）
    Rect,
    Text,
    Icon,
    /// 照片、插画等复杂图像
    Image,
    /// 分割线
    Line,
}

impl NodeType {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeType::Frame => "frame",
            NodeType::Rect => "rect",
            NodeType::Text => "text",
            NodeType::Icon => "icon",
            NodeType::Image => "image",
            NodeType::Line => "line",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "frame" => NodeType::Frame,
            "rect" => NodeType::Rect,
            "text" => NodeType::Text,
            "icon" => NodeType::Icon,
            "image" => NodeType::Image,
            "line" => NodeType::Line,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeSource {
    /// 本地算法识别
    #[default]
    Auto,
    /// Agent 补充或修改过
    Agent,
    /// 用户在桌面端修改过
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Bounds {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Bounds {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }
    pub fn right(&self) -> u32 {
        self.x + self.width
    }
    pub fn bottom(&self) -> u32 {
        self.y + self.height
    }
    pub fn area(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
    pub fn union(&self, o: &Bounds) -> Bounds {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Bounds { x, y, width: self.right().max(o.right()) - x, height: self.bottom().max(o.bottom()) - y }
    }
    pub fn intersection(&self, o: &Bounds) -> Option<Bounds> {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        (x1 > x0 && y1 > y0).then(|| Bounds { x: x0, y: y0, width: x1 - x0, height: y1 - y0 })
    }
    pub fn iou(&self, o: &Bounds) -> f32 {
        let inter = self.intersection(o).map(|b| b.area()).unwrap_or(0) as f64;
        let uni = (self.area() + o.area()) as f64 - inter;
        if uni <= 0.0 {
            0.0
        } else {
            (inter / uni) as f32
        }
    }
    /// o 有多大比例落在 self 内
    pub fn contains_ratio(&self, o: &Bounds) -> f32 {
        if o.area() == 0 {
            return 0.0;
        }
        self.intersection(o).map(|b| b.area()).unwrap_or(0) as f32 / o.area() as f32
    }
    pub fn center(&self) -> (f32, f32) {
        (self.x as f32 + self.width as f32 / 2.0, self.y as f32 + self.height as f32 / 2.0)
    }
    /// 按比例缩放（分析时为了速度可能先缩小图片）
    pub fn scaled(&self, k: f32) -> Bounds {
        let x0 = (self.x as f32 * k).round() as u32;
        let y0 = (self.y as f32 * k).round() as u32;
        let x1 = (self.right() as f32 * k).round() as u32;
        let y1 = (self.bottom() as f32 * k).round() as u32;
        Bounds { x: x0, y: y0, width: (x1 - x0).max(1), height: (y1 - y0).max(1) }
    }
    pub fn inset(&self, d: u32) -> Option<Bounds> {
        (self.width > 2 * d && self.height > 2 * d).then(|| Bounds { x: self.x + d, y: self.y + d, width: self.width - 2 * d, height: self.height - 2 * d })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Point {
    pub x: i64,
    pub y: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// 节点编号，与标注图上的数字一致（n1 → 1）
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    /// 语义角色，如 button / card / nav_bar / tab_item / avatar / price / title（由 Agent 或人工填写）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// 名称，如 "登录按钮"；图标的名称会用作素材文件名
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 文字内容（本地不做 OCR，由 Agent 或人工填写）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub bounds: Bounds,
    #[serde(default)]
    pub relative: Point,
    #[serde(default, skip_serializing_if = "Style::is_empty")]
    pub style: Style,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_style: Option<TextStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    /// 图标素材文件
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<Asset>,
    /// 识别置信度 0-1
    #[serde(default = "one")]
    pub confidence: f32,
    #[serde(default)]
    pub source: NodeSource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

fn one() -> f32 {
    1.0
}

impl Node {
    pub fn new(node_type: NodeType, bounds: Bounds) -> Self {
        Self {
            id: String::new(),
            node_type,
            role: None,
            name: None,
            text: None,
            description: None,
            bounds,
            relative: Point::default(),
            style: Style::default(),
            text_style: None,
            layout: None,
            asset: None,
            confidence: 1.0,
            source: NodeSource::Auto,
            children: Vec::new(),
        }
    }

    /// 没有任何视觉样式、也没有语义标注的自动分组
    pub fn is_plain_group(&self) -> bool {
        self.node_type == NodeType::Frame && self.style.is_empty() && self.source == NodeSource::Auto && !self.has_semantics()
    }

    pub fn has_semantics(&self) -> bool {
        self.role.is_some() || self.name.is_some() || self.text.is_some() || self.description.is_some()
    }

    /// 深度优先遍历（前序）
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Node, usize)) {
        fn go<'a>(n: &'a Node, depth: usize, f: &mut impl FnMut(&'a Node, usize)) {
            f(n, depth);
            for c in &n.children {
                go(c, depth + 1, f);
            }
        }
        go(self, 0, f)
    }

    pub fn walk_mut(&mut self, f: &mut impl FnMut(&mut Node)) {
        f(self);
        for c in &mut self.children {
            c.walk_mut(f);
        }
    }

    pub fn find(&self, id: &str) -> Option<&Node> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(id))
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Node> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter_mut().find_map(|c| c.find_mut(id))
    }

    pub fn count(&self) -> usize {
        1 + self.children.iter().map(Node::count).sum::<usize>()
    }

    /// 内容区：去掉描边
    pub fn content_bounds(&self) -> Bounds {
        let s = self.style.stroke.as_ref().map(|s| s.width).unwrap_or(0);
        self.bounds.inset(s).unwrap_or(self.bounds)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Style {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Fill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<CornerRadius>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
}

impl Style {
    pub fn is_empty(&self) -> bool {
        self.fill.is_none() && self.stroke.is_none() && self.corner_radius.is_none() && self.shadow.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Fill {
    Solid {
        color: String,
    },
    /// 线性渐变；angle 与 CSS linear-gradient 一致：0 = 从下到上，90 = 从左到右，180 = 从上到下
    Linear {
        angle: f32,
        stops: Vec<GradientStop>,
    },
}

impl Fill {
    pub fn solid(c: [u8; 3]) -> Self {
        Fill::Solid { color: crate::color::hex(c) }
    }
    /// 代表色（渐变取中间色）
    pub fn main_color(&self) -> Option<[u8; 3]> {
        match self {
            Fill::Solid { color } => crate::color::parse_hex(color),
            Fill::Linear { stops, .. } => {
                let a = crate::color::parse_hex(&stops.first()?.color)?;
                let b = crate::color::parse_hex(&stops.last()?.color)?;
                Some([0, 1, 2].map(|i| ((a[i] as u16 + b[i] as u16) / 2) as u8))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    /// 0-1
    pub pos: f32,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub color: String,
    pub width: u32,
}

/// 统一圆角，或 [左上, 右上, 右下, 左下]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CornerRadius {
    Uniform(u32),
    Corners([u32; 4]),
}

impl CornerRadius {
    pub fn max(&self) -> u32 {
        match self {
            CornerRadius::Uniform(r) => *r,
            CornerRadius::Corners(c) => c.iter().copied().max().unwrap_or(0),
        }
    }
}

/// 外阴影（只在纯色底上估算）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    pub x: i32,
    pub y: i32,
    /// 模糊半径，与 CSS box-shadow 的 blur 含义一致
    pub blur: u32,
    /// #rrggbbaa
    pub color: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontWeight {
    Regular,
    Medium,
    Bold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    pub color: String,
    /// 估算字号（像素），由文字墨迹高度推算
    pub font_size: u32,
    /// 多行文字的行高（像素）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height: Option<u32>,
    pub lines: u32,
    pub weight: FontWeight,
    pub align: TextAlign,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutMode {
    /// 横排（flex-direction: row）
    Row,
    /// 竖排（flex-direction: column）
    Column,
    /// 自由摆放
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Start,
    Center,
    End,
    SpaceBetween,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossAlign {
    Start,
    Center,
    End,
    Stretch,
    /// 子节点对齐方式不一致
    Mixed,
}

/// 推断出的自动布局（类似 Figma auto-layout / CSS flex）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub mode: LayoutMode,
    /// 子节点间距（中位数）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<u32>,
    /// 间距是否基本一致（误差 ≤ 2px 或 10%）
    #[serde(default = "yes")]
    pub gap_uniform: bool,
    /// [上, 右, 下, 左]，相对内容区（去掉描边）
    pub padding: [u32; 4],
    /// 主轴对齐
    pub align: Align,
    /// 交叉轴对齐
    pub cross_align: CrossAlign,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetSource {
    /// 从原图裁出
    Crop,
    /// AI 重新生成的高清版
    Generated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    /// 素材文件路径（相对项目根目录，统一用 /）
    pub path: String,
    pub source: AssetSource,
    pub width: u32,
    pub height: u32,
}
