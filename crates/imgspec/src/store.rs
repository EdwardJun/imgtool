//! spec 的存放位置、读写、语义写回，以及重新解析时保留旧标注

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::model::{AssetSource, Node, NodeSource, NodeType, Spec, SPEC_VERSION};
use crate::{Error, Result};

/// 默认输出目录（相对项目根目录）
pub const SPECS_DIR: &str = ".imgtool/specs";

/// 默认图标素材子目录名
pub const ICONS_SUBDIR: &str = "icons";

#[derive(Debug, Clone, PartialEq)]
pub struct SpecPaths {
    /// 项目根目录：spec 里的路径都相对它
    pub root: PathBuf,
    pub image: PathBuf,
    /// 一张图的全部解析产物都在这个文件夹里：`<root>/.imgtool/specs/<相对目录>/<原名>/`
    pub dir: PathBuf,
    /// `<dir>/<原名>.spec.json`
    pub spec: PathBuf,
    /// `<dir>/<原名>.spec.png` 编号标注图
    pub overlay: PathBuf,
    /// `<dir>/<原名>.icons-ref.png` AI 重绘图标用的参考图（裁出的原始图标拼成的网格）
    pub reference: PathBuf,
    /// `<dir>/<原名>.icons-context.png` AI 重绘图标用的上下文参考图（完整设计稿上框出图标）
    pub context: PathBuf,
    /// 图标素材目录：默认 `<dir>/icons/`，项目预设 `[spec] icons_dir` 可改到项目素材目录
    pub icons_dir: PathBuf,
    /// 图标目录是否来自项目预设（素材直接进工程目录）
    pub custom_icons_dir: bool,
}

fn paths_in(root: &Path, image: &Path, dir: PathBuf, stem: &str, icons_dir: Option<&str>) -> SpecPaths {
    let custom = icons_dir.map(str::trim).filter(|d| !d.is_empty());
    SpecPaths {
        root: root.to_path_buf(),
        image: image.to_path_buf(),
        spec: dir.join(format!("{stem}.spec.json")),
        overlay: dir.join(format!("{stem}.spec.png")),
        reference: dir.join(format!("{stem}.icons-ref.png")),
        context: dir.join(format!("{stem}.icons-context.png")),
        icons_dir: match custom {
            Some(d) => root.join(d),
            None => dir.join(ICONS_SUBDIR),
        },
        custom_icons_dir: custom.is_some(),
        dir,
    }
}

/// 计算一张图对应的 spec 文件位置。图片不在 root 下时按图片所在目录处理
pub fn spec_paths(root: &Path, image: &Path, icons_dir: Option<&str>) -> SpecPaths {
    let stem = image.file_stem().and_then(|s| s.to_str()).unwrap_or("image").to_string();
    let rel_dir = image.parent().and_then(|p| p.strip_prefix(root).ok()).unwrap_or_else(|| Path::new("")).to_path_buf();
    let dir = root.join(SPECS_DIR).join(&rel_dir).join(&stem);
    paths_in(root, image, dir, &stem, icons_dir)
}

/// 由 spec 文件路径还原各个路径（原图路径从 spec 内容读取）
pub fn paths_for_spec(root: &Path, spec_file: &Path, spec: &Spec, icons_dir: Option<&str>) -> SpecPaths {
    let image = abs(root, &spec.source.path);
    let p = spec_paths(root, &image, icons_dir);
    if p.spec == spec_file {
        return p;
    }
    // spec 被挪过位置：以实际文件所在的文件夹为准
    let stem = spec_file.file_name().and_then(|n| n.to_str()).unwrap_or("image.spec.json").trim_end_matches(".spec.json").to_string();
    let dir = spec_file.parent().unwrap_or(root).to_path_buf();
    paths_in(root, &image, dir, &stem, icons_dir)
}

/// 相对 root 的路径（统一用 /）；不在 root 下时返回绝对路径
pub fn rel(root: &Path, p: &Path) -> String {
    match p.strip_prefix(root) {
        Ok(r) => r.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/"),
        Err(_) => p.display().to_string(),
    }
}

pub fn abs(root: &Path, s: &str) -> PathBuf {
    let p = Path::new(s);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    }
}

pub fn is_spec_file(p: &Path) -> bool {
    p.file_name().and_then(|n| n.to_str()).map(|n| n.ends_with(".spec.json")).unwrap_or(false)
}

pub fn save(spec: &Spec, path: &Path) -> Result<()> {
    let text = serde_json::to_string_pretty(spec)?;
    imgcore::project::write_atomic(path, text.as_bytes()).map_err(|e| Error::Invalid(e.to_string()))
}

pub fn load(path: &Path) -> Result<Spec> {
    let text = std::fs::read_to_string(path)?;
    let spec: Spec = serde_json::from_str(&text)?;
    if spec.version != SPEC_VERSION {
        return Err(Error::Invalid(format!("不支持的 spec 版本 {}（当前支持 {SPEC_VERSION}）", spec.version)));
    }
    Ok(spec)
}

/// 一条语义标注：按编号修改节点；传空字符串表示清空该字段
#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Annotation {
    /// 节点编号，如 "n12"（标注图上的数字 12 也可以）
    pub id: String,
    /// 语义角色，如 button / card / nav_bar / tab_item / avatar / title / price / input / badge
    #[serde(default)]
    pub role: Option<String>,
    /// 名称，如 "登录按钮"；图标的名称会作为素材文件名（建议用英文，如 home、search）
    #[serde(default)]
    pub name: Option<String>,
    /// 文字内容（看图写出画面上的原文）
    #[serde(default)]
    pub text: Option<String>,
    /// 补充说明，如交互、状态、用途
    #[serde(default)]
    pub description: Option<String>,
    /// 修正节点类型：frame / rect / text / icon / image / line
    #[serde(default, rename = "type")]
    pub node_type: Option<NodeType>,
}

fn normalize_id(id: &str) -> String {
    let id = id.trim();
    if id.chars().all(|c| c.is_ascii_digit()) && !id.is_empty() {
        format!("n{id}")
    } else {
        id.to_string()
    }
}

fn set(field: &mut Option<String>, v: &Option<String>) {
    if let Some(v) = v {
        let t = v.trim();
        *field = (!t.is_empty()).then(|| t.to_string());
    }
}

/// 把语义写回 spec；有任何未知编号时整体不修改并报错
pub fn apply_annotations(spec: &mut Spec, anns: &[Annotation], source: NodeSource) -> Result<Vec<String>> {
    let ids: Vec<String> = anns.iter().map(|a| normalize_id(&a.id)).collect();
    let unknown: Vec<String> = ids.iter().filter(|id| spec.root.find(id).is_none()).cloned().collect();
    if !unknown.is_empty() {
        return Err(Error::UnknownIds(unknown));
    }
    for (a, id) in anns.iter().zip(&ids) {
        let n = spec.root.find_mut(id).expect("已校验");
        set(&mut n.role, &a.role);
        set(&mut n.name, &a.name);
        set(&mut n.text, &a.text);
        set(&mut n.description, &a.description);
        if let Some(t) = a.node_type {
            n.node_type = t;
        }
        n.source = source;
    }
    if !anns.is_empty() {
        spec.annotated = true;
    }
    Ok(ids)
}

fn compatible(a: NodeType, b: NodeType) -> bool {
    a == b || (matches!(a, NodeType::Frame | NodeType::Rect) && matches!(b, NodeType::Frame | NodeType::Rect))
}

/// 重新解析同一张图时：按位置（IoU ≥ 0.8）匹配旧节点，保留语义标注、人工修正的类型和 AI 生成的图标素材
pub fn merge_previous(new: &mut Spec, old: &Spec) -> usize {
    let mut olds: Vec<&Node> = Vec::new();
    old.root.walk(&mut |n, _| {
        if n.has_semantics() || n.source != NodeSource::Auto || n.asset.as_ref().map(|a| a.source == AssetSource::Generated).unwrap_or(false) {
            olds.push(n);
        }
    });
    let mut used = vec![false; olds.len()];
    let mut kept = 0;
    new.root.walk_mut(&mut |n| {
        let best = olds
            .iter()
            .enumerate()
            .filter(|(i, o)| !used[*i] && (compatible(o.node_type, n.node_type) || o.source != NodeSource::Auto))
            .map(|(i, o)| (i, o.bounds.iou(&n.bounds)))
            .filter(|(_, iou)| *iou >= 0.8)
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, _)) = best {
            used[i] = true;
            let o = olds[i];
            n.role = o.role.clone();
            n.name = o.name.clone();
            n.text = o.text.clone();
            n.description = o.description.clone();
            if o.source != NodeSource::Auto {
                n.node_type = o.node_type;
                n.source = o.source;
            }
            if o.asset.as_ref().map(|a| a.source == AssetSource::Generated).unwrap_or(false) {
                n.asset = o.asset.clone();
            }
            kept += 1;
        }
    });
    if new.notes.is_none() {
        new.notes = old.notes.clone();
    }
    new.annotated = kept > 0 || old.annotated && new.notes.is_some();
    kept
}
