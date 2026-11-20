//! 设计稿里的图标素材：
//! - 解析时从原图裁出每个 icon 节点，去掉底色后导出透明底正方形 PNG（立即可用，清晰度受原图限制）
//! - 按需把选中的图标画在一张网格图上交给 AI 一次重绘（风格统一、只花一次额度），再拆开替换素材

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use image::{DynamicImage, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

use imgcore::detect::{render_square, Region, RegionKind, SquareOptions};

use crate::color::{self, Rgb};
use crate::model::{Asset, AssetSource, Bounds, Node, NodeType, Spec};
use crate::store::{abs, rel, SpecPaths};
use crate::{Error, Result};

/// 一次重绘最多的图标数（4 × 4 网格）
pub const MAX_SHEET_ICONS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IconOptions {
    /// 输出边长（像素），默认 256
    pub size: u32,
    /// 四周留白百分比 0-40，默认 10
    pub padding: u8,
}

impl Default for IconOptions {
    fn default() -> Self {
        Self { size: 256, padding: 10 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IconFile {
    pub id: String,
    pub name: String,
    /// 相对项目根目录
    pub path: String,
    pub source: AssetSource,
    pub size: u32,
    /// 图标在原图里的长边像素
    #[serde(default)]
    pub native: u32,
    /// 原图太小（长边 < MIN_REDRAW_PX）：素材是放大出来的，会比较模糊，默认不参与 AI 重绘
    #[serde(default)]
    pub low_res: bool,
}

fn icon_file(node: &Node, name: String, path: String, source: AssetSource, size: u32) -> IconFile {
    let native = native_px(node);
    IconFile { id: node.id.clone(), name, path, source, size, native, low_res: native < MIN_REDRAW_PX }
}

/// 图标素材的文件名（不含扩展名）：优先用标注的名称
pub fn icon_stem(n: &Node) -> String {
    match n.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => imgcore::project::sanitize_name(name),
        None => format!("icon-{}", n.id),
    }
}

fn ring_bg(img: &RgbaImage, b: &Bounds) -> Option<Rgb> {
    let e = ((b.width.max(b.height) as f32 * 0.08).round() as u32).max(2);
    let (x0, y0) = (b.x.saturating_sub(e), b.y.saturating_sub(e));
    let (x1, y1) = ((b.right() + e).min(img.width()), (b.bottom() + e).min(img.height()));
    let mut cs = Vec::new();
    let mut clear = 0usize;
    for y in y0..y1 {
        for x in x0..x1 {
            if x >= b.x && x < b.right() && y >= b.y && y < b.bottom() {
                continue;
            }
            let p = img.get_pixel(x, y);
            if p[3] < 128 {
                clear += 1;
            } else {
                cs.push([p[0] as f32, p[1] as f32, p[2] as f32]);
            }
        }
    }
    if clear > cs.len() {
        return None;
    }
    color::mode_color(cs.into_iter(), 4).map(|c| c.0)
}

/// 去掉底色（按与底色的距离得到透明度，并还原边缘颜色），再居中放进正方形画布
pub fn cut_icon(img: &RgbaImage, b: &Bounds, opts: &IconOptions) -> Option<RgbaImage> {
    let pad = 1;
    let x0 = b.x.saturating_sub(pad);
    let y0 = b.y.saturating_sub(pad);
    let x1 = (b.right() + pad).min(img.width());
    let y1 = (b.bottom() + pad).min(img.height());
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let bg = ring_bg(img, b);
    let mut crop = RgbaImage::new(x1 - x0, y1 - y0);
    let dmax = match bg {
        Some(bgc) => (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .map(|(x, y)| {
                let p = img.get_pixel(x, y);
                color::dist([p[0] as f32, p[1] as f32, p[2] as f32], bgc)
            })
            .fold(0f32, f32::max),
        None => 0.0,
    };
    let (t0, t1) = (8.0f32, (0.5 * dmax).max(24.0));
    // 只有从四周连通过来的底色才去掉：圆形底上的白色图案（微信气泡、企鹅）要保留
    let (cw, ch) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut outside = vec![false; cw * ch];
    if let Some(bgc) = bg {
        let near = |x: usize, y: usize| {
            let p = img.get_pixel(x0 + x as u32, y0 + y as u32);
            color::dist([p[0] as f32, p[1] as f32, p[2] as f32], bgc) < t1
        };
        let mut stack: Vec<(usize, usize)> = Vec::new();
        // 线性图标（笔画占比低）内部的底色本来就该透明，只对实心的徽标保留内部图案
        let solid = (0..ch).flat_map(|y| (0..cw).map(move |x| (x, y))).filter(|&(x, y)| !near(x, y)).count();
        if (solid as f32) < 0.45 * (cw * ch) as f32 {
            outside.iter_mut().for_each(|o| *o = true);
        } else {
            for x in 0..cw {
                stack.push((x, 0));
                stack.push((x, ch - 1));
            }
            for y in 0..ch {
                stack.push((0, y));
                stack.push((cw - 1, y));
            }
        }
        while let Some((x, y)) = stack.pop() {
            let i = y * cw + x;
            if outside[i] || !near(x, y) {
                continue;
            }
            outside[i] = true;
            if x > 0 {
                stack.push((x - 1, y));
            }
            if x + 1 < cw {
                stack.push((x + 1, y));
            }
            if y > 0 {
                stack.push((x, y - 1));
            }
            if y + 1 < ch {
                stack.push((x, y + 1));
            }
        }
    }
    for y in y0..y1 {
        for x in x0..x1 {
            let p = *img.get_pixel(x, y);
            let enclosed = !outside[((y - y0) as usize) * cw + (x - x0) as usize];
            let out = match bg {
                None => p,
                Some(_) if enclosed => p,
                Some(bgc) => {
                    let c = [p[0] as f32, p[1] as f32, p[2] as f32];
                    let d = color::dist(c, bgc);
                    let a = ((d - t0) / (t1 - t0)).clamp(0.0, 1.0);
                    if a <= 0.0 {
                        Rgba([0, 0, 0, 0])
                    } else {
                        let v = [0, 1, 2].map(|i| (bgc[i] + (c[i] - bgc[i]) / a).round().clamp(0.0, 255.0) as u8);
                        Rgba([v[0], v[1], v[2], (a * p[3] as f32).round() as u8])
                    }
                }
            };
            crop.put_pixel(x - x0, y - y0, out);
        }
    }
    // 收紧到不透明内容
    let (mut ax0, mut ay0, mut ax1, mut ay1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in crop.enumerate_pixels() {
        if p[3] > 8 {
            ax0 = ax0.min(x);
            ay0 = ay0.min(y);
            ax1 = ax1.max(x);
            ay1 = ay1.max(y);
        }
    }
    if ax0 == u32::MAX {
        return None;
    }
    let region = Region { x: ax0, y: ay0, width: ax1 - ax0 + 1, height: ay1 - ay0 + 1, kind: RegionKind::Icon };
    let sq = SquareOptions { size: opts.size.clamp(16, 2048), padding: opts.padding.min(40), background: None };
    Some(render_square(&DynamicImage::ImageRgba8(crop), &region, &sq).to_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 白底上的绿色圆形，中间有白色圆点：外面透明，里面的白点保留
    #[test]
    fn keeps_enclosed_white_details() {
        let mut img = RgbaImage::from_pixel(120, 120, Rgba([255, 255, 255, 255]));
        for (x, y, p) in img.enumerate_pixels_mut() {
            let d = ((x as f32 - 60.0).powi(2) + (y as f32 - 60.0).powi(2)).sqrt();
            if d <= 40.0 && d > 12.0 {
                *p = Rgba([27, 188, 39, 255]);
            }
        }
        let tile = cut_icon(&img, &Bounds::new(20, 20, 81, 81), &IconOptions { size: 100, padding: 0 }).unwrap();
        assert_eq!(tile.get_pixel(1, 1)[3], 0, "圆外透明");
        let c = tile.get_pixel(50, 50);
        assert!(c[3] == 255 && c[0] > 240, "中心白点保留: {c:?}");
        assert_eq!(tile.get_pixel(50, 20)[1], 188);
    }
}

fn encode_png(img: &RgbaImage) -> Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).map_err(|e| Error::Image(e.to_string()))?;
    Ok(buf.into_inner())
}

/// 选文件名：重名时，属于本 spec 的旧素材直接覆盖，否则自动加序号
fn target_path(dir: &Path, stem: &str, owned: &HashSet<PathBuf>, used: &mut HashSet<PathBuf>, id: &str) -> Result<PathBuf> {
    let mut p = dir.join(format!("{stem}.png"));
    if used.contains(&p) {
        p = dir.join(format!("{stem}-{id}.png"));
    }
    if p.exists() && !owned.contains(&p) && !used.contains(&p) {
        p = imgcore::project::resolve_conflict(p, Path::new(""), &Default::default(), imgcore::project::ConflictPolicy::Rename).map_err(|e| Error::Invalid(e.to_string()))?;
    }
    used.insert(p.clone());
    Ok(p)
}

fn owned_assets(root: &Path, specs: &[&Spec]) -> HashSet<PathBuf> {
    let mut s = HashSet::new();
    for sp in specs {
        sp.root.walk(&mut |n, _| {
            if let Some(a) = &n.asset {
                s.insert(abs(root, &a.path));
            }
        });
    }
    s
}

fn icon_ids(spec: &Spec) -> Vec<String> {
    let mut v = Vec::new();
    spec.root.walk(&mut |n, _| {
        if n.node_type == NodeType::Icon {
            v.push(n.id.clone());
        }
    });
    v
}

/// 从原图裁出全部图标素材并写进 spec；已有 AI 高清版的图标保留不动。
/// previous：重新解析前的旧 spec，用来清理不再使用的旧裁剪素材
pub fn export_icons(img: &DynamicImage, spec: &mut Spec, paths: &SpecPaths, opts: &IconOptions, previous: Option<&Spec>) -> Result<Vec<IconFile>> {
    let rgba = img.to_rgba8();
    let mut prev_list: Vec<&Spec> = Vec::new();
    if let Some(p) = previous {
        prev_list.push(p);
    }
    let owned = {
        let mut o = owned_assets(&paths.root, &prev_list);
        o.extend(owned_assets(&paths.root, &[&*spec]));
        o
    };
    let mut used = HashSet::new();
    let mut files = Vec::new();
    for id in icon_ids(spec) {
        let node = spec.root.find_mut(&id).expect("编号存在");
        if let Some(a) = &node.asset {
            if a.source == AssetSource::Generated && abs(&paths.root, &a.path).is_file() {
                used.insert(abs(&paths.root, &a.path));
                files.push(icon_file(node, icon_stem(node), a.path.clone(), a.source, a.width));
                continue;
            }
        }
        let Some(tile) = cut_icon(&rgba, &node.bounds, opts) else { continue };
        let stem = icon_stem(node);
        let target = target_path(&paths.icons_dir, &stem, &owned, &mut used, &id)?;
        imgcore::project::write_atomic(&target, &encode_png(&tile)?).map_err(|e| Error::Invalid(e.to_string()))?;
        let path = rel(&paths.root, &target);
        node.asset = Some(Asset { path: path.clone(), source: AssetSource::Crop, width: tile.width(), height: tile.height() });
        files.push(icon_file(node, stem, path, AssetSource::Crop, tile.width()));
    }
    // 清理旧 spec 里不再使用的裁剪素材（只清理默认目录 .imgtool/specs 下的文件）
    if let Some(prev) = previous {
        let specs_dir = paths.root.join(crate::store::SPECS_DIR);
        prev.root.walk(&mut |n, _| {
            if let Some(a) = &n.asset {
                let p = abs(&paths.root, &a.path);
                if a.source == AssetSource::Crop && !used.contains(&p) && p.starts_with(&specs_dir) {
                    let _ = std::fs::remove_file(&p);
                }
            }
        });
    }
    Ok(files)
}

/// 标注了名称的图标：素材文件改成用名称命名
pub fn rename_assets(spec: &mut Spec, paths: &SpecPaths) -> Result<Vec<IconFile>> {
    let owned = owned_assets(&paths.root, &[&*spec]);
    let mut used: HashSet<PathBuf> = owned.clone();
    let mut changed = Vec::new();
    for id in icon_ids(spec) {
        let node = spec.root.find_mut(&id).expect("编号存在");
        let Some(asset) = node.asset.clone() else { continue };
        let current = abs(&paths.root, &asset.path);
        let stem = icon_stem(node);
        let mut want = current.with_file_name(format!("{stem}.png"));
        if want == current || !current.is_file() {
            continue;
        }
        used.remove(&current);
        if used.contains(&want) || want.exists() {
            want = current.with_file_name(format!("{stem}-{id}.png"));
            if want.exists() && want != current {
                want = imgcore::project::resolve_conflict(want, Path::new(""), &Default::default(), imgcore::project::ConflictPolicy::Rename).map_err(|e| Error::Invalid(e.to_string()))?;
            }
        }
        std::fs::rename(&current, &want)?;
        used.insert(want.clone());
        let path = rel(&paths.root, &want);
        node.asset = Some(Asset { path: path.clone(), ..asset });
        changed.push(icon_file(node, stem, path, asset.source, asset.width));
    }
    Ok(changed)
}
/// 图标原图长边低于它时默认不送去 AI 重绘：放大后没有可辨认的造型，模型只能乱猜
pub const MIN_REDRAW_PX: u32 = 24;

/// 没有被选进重绘的图标及原因
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkippedIcon {
    pub id: String,
    pub reason: String,
}

/// 图标重绘计划：网格排布与提示词
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IconSheetPlan {
    pub ids: Vec<String>,
    pub cols: u32,
    pub rows: u32,
    pub prompt: String,
    /// 默认选择时因为太小被跳过的图标（显式传 ids 时为空）
    #[serde(default)]
    pub skipped: Vec<SkippedIcon>,
    /// 选中的图标里还没有名称 / 说明的（模型只能看图猜含义，建议先标注）
    #[serde(default)]
    pub unnamed: Vec<String>,
    /// 选中的图标里原图太小、只是被强制加入的
    #[serde(default)]
    pub low_res: Vec<String>,
}

/// 图标在原图里的长边像素
pub fn native_px(n: &Node) -> u32 {
    n.bounds.width.max(n.bounds.height)
}

/// 离图标最近的已标注文字（同一行右侧或正下方），帮模型理解含义，如「申报时间」旁边的日历
fn nearby_text(spec: &Spec, icon: &Node) -> Option<String> {
    let b = icon.bounds;
    let reach = (b.width.max(b.height) * 3).max(48) as i64;
    let mut best: Option<(i64, String)> = None;
    spec.root.walk(&mut |n, _| {
        let Some(t) = n.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) else { return };
        if n.id == icon.id {
            return;
        }
        let nb = n.bounds;
        let dx = (nb.x as i64 - b.right() as i64).max(b.x as i64 - nb.right() as i64).max(0);
        let dy = (nb.y as i64 - b.bottom() as i64).max(b.y as i64 - nb.bottom() as i64).max(0);
        if dx > reach || dy > reach {
            return;
        }
        let d = dx + dy;
        if best.as_ref().map(|(bd, _)| d < *bd).unwrap_or(true) {
            best = Some((d, t.chars().take(20).collect()));
        }
    });
    best.map(|(_, t)| t)
}

fn describe(spec: &Spec, n: &Node, i: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    match (&n.name, &n.role) {
        (Some(name), Some(role)) => parts.push(format!("{name}（{role}）")),
        (Some(name), None) => parts.push(name.clone()),
        (None, Some(role)) => parts.push(role.clone()),
        (None, None) => parts.push("含义请结合第二张参考图中它所在的位置和旁边的文字判断".to_string()),
    }
    if let Some(d) = &n.description {
        parts.push(d.clone());
    }
    if let Some(t) = nearby_text(spec, n) {
        parts.push(format!("旁边的文字是「{t}」"));
    }
    if let Some(c) = n.style.fill.as_ref().and_then(|f| f.main_color()) {
        parts.push(format!("主色 {}", color::hex(c).to_uppercase()));
    }
    format!("{}. {}", i + 1, parts.join("，"))
}

fn normalize_id(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_digit()) {
        format!("n{s}")
    } else {
        s.trim().to_string()
    }
}

/// 选出要重绘的图标并生成网格提示词。
/// 默认选全部 icon 节点，但跳过原图长边 < MIN_REDRAW_PX 的；显式传 ids 时按传入的全部重绘（强制）
pub fn plan_icon_sheet(spec: &Spec, ids: Option<&[String]>, style: Option<&str>) -> Result<IconSheetPlan> {
    let mut skipped = Vec::new();
    let ids: Vec<String> = match ids.filter(|v| !v.is_empty()) {
        Some(v) => {
            let norm: Vec<String> = v.iter().map(|s| normalize_id(s)).collect();
            let unknown: Vec<String> = norm.iter().filter(|id| spec.root.find(id).is_none()).cloned().collect();
            if !unknown.is_empty() {
                return Err(Error::UnknownIds(unknown));
            }
            norm
        }
        None => {
            let mut keep = Vec::new();
            for id in icon_ids(spec) {
                let px = native_px(spec.root.find(&id).expect("编号存在"));
                if px < MIN_REDRAW_PX {
                    skipped.push(SkippedIcon { id, reason: format!("原图只有 {px}px（小于 {MIN_REDRAW_PX}px），放大后看不出造型，重绘容易画错；确实需要时用 ids 指定") });
                } else {
                    keep.push(id);
                }
            }
            if keep.is_empty() && !skipped.is_empty() {
                return Err(Error::Invalid(format!(
                    "识别出的 {} 个图标都太小（原图长边小于 {MIN_REDRAW_PX}px），不适合 AI 重绘；可以先确认它们是不是图标，确实需要时用 ids 指定",
                    skipped.len()
                )));
            }
            keep
        }
    };
    if ids.is_empty() {
        return Err(Error::Invalid("设计稿里没有识别到图标；可先用 annotate_design 把对应节点的 type 改成 icon，或传 ids 指定".into()));
    }
    if ids.len() > MAX_SHEET_ICONS {
        return Err(Error::Invalid(format!("一次最多重绘 {MAX_SHEET_ICONS} 个图标，当前 {} 个，请用 ids 分批", ids.len())));
    }
    let nodes: Vec<&Node> = ids.iter().map(|id| spec.root.find(id).expect("已校验")).collect();
    let unnamed = nodes.iter().filter(|n| n.name.is_none() && n.role.is_none() && n.description.is_none()).map(|n| n.id.clone()).collect();
    let low_res = nodes.iter().filter(|n| native_px(n) < MIN_REDRAW_PX).map(|n| n.id.clone()).collect();
    let n = ids.len() as u32;
    let cols = (n as f32).sqrt().ceil() as u32;
    let rows = n.div_ceil(cols);
    let mut s = format!(
        "一张 UI 图标集设计图：纯白色背景（#FFFFFF），{n} 个图标按 {cols} 列 × {rows} 行的网格整齐排列，顺序从左到右、从上到下。每个图标居中放在自己的格子里，大小一致，约占格子的 55%，图标之间留出充足的空白，互不重叠、互不相连。"
    );
    if n < cols * rows {
        s.push_str(&format!("最后一行只有 {} 个图标，靠左排列，其余格子留空。", n - cols * (rows - 1)));
    }
    s.push_str("第一张参考图是从设计稿中裁出的原始图标（排列顺序相同），可能比较模糊；第二张参考图是完整设计稿，用红框和编号 1 到 ");
    s.push_str(&format!("{n} 标出了每个图标在页面上的位置，请结合它周围的文字和用途理解图标的含义。"));
    s.push_str("保持每个图标的含义、整体造型和配色，重绘成边缘清晰、线条精致、细节完整的高清版本，整套图标的线宽、圆角和风格保持统一。");
    s.push_str(&format!("风格：{}。", style.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("与设计稿一致的扁平化 UI 图标")));
    s.push_str("图标依次为：");
    let descs: Vec<String> = nodes.iter().enumerate().map(|(i, node)| describe(spec, node, i)).collect();
    s.push_str(&descs.join("；"));
    s.push_str("。画面中不出现任何文字、数字、编号、网格线、边框或阴影。");
    Ok(IconSheetPlan { ids, cols, rows, prompt: s, skipped, unnamed, low_res })
}

/// 上下文参考图：完整设计稿上用红框和编号（与网格顺序一致）标出要重绘的图标
pub fn context_sheet(img: &DynamicImage, spec: &Spec, plan: &IconSheetPlan) -> Result<Vec<u8>> {
    let items: Vec<(Bounds, String)> = plan.ids.iter().enumerate().filter_map(|(i, id)| spec.root.find(id).map(|n| (n.bounds, (i + 1).to_string()))).collect();
    let out = crate::overlay::render_marks(img, &items, 1536, [255, 40, 40]);
    encode_png(&out)
}

/// 参考图：把原图里裁出的图标按同样的网格排在白底上
pub fn reference_sheet(img: &DynamicImage, spec: &Spec, plan: &IconSheetPlan) -> Result<Vec<u8>> {
    let cell = 256u32;
    let rgba = img.to_rgba8();
    let mut sheet = RgbaImage::from_pixel(cell * plan.cols, cell * plan.rows, Rgba([255, 255, 255, 255]));
    let opts = IconOptions { size: cell, padding: 22 };
    for (i, id) in plan.ids.iter().enumerate() {
        let Some(node) = spec.root.find(id) else { continue };
        let Some(tile) = cut_icon(&rgba, &node.bounds, &opts) else { continue };
        let (c, r) = (i as u32 % plan.cols, i as u32 / plan.cols);
        image::imageops::overlay(&mut sheet, &tile, (c * cell) as i64, (r * cell) as i64);
    }
    encode_png(&sheet)
}

/// 网格图里每个图标的位置：识别数量正好时用识别结果，否则按网格格子找
fn locate(sheet: &RgbaImage, plan: &IconSheetPlan) -> Vec<Option<Bounds>> {
    let n = plan.ids.len();
    let d = imgcore::detect::detect_regions(&DynamicImage::ImageRgba8(sheet.clone()), &imgcore::detect::DetectOptions { tolerance: 8, ..Default::default() });
    if d.regions.len() == n {
        return d.regions.iter().map(|r| Some(Bounds::new(r.x, r.y, r.width, r.height))).collect();
    }
    let (w, h) = sheet.dimensions();
    (0..n)
        .map(|i| {
            let (c, r) = (i as u32 % plan.cols, i as u32 / plan.cols);
            let (cx0, cx1) = (c * w / plan.cols, (c + 1) * w / plan.cols);
            let (cy0, cy1) = (r * h / plan.rows, (r + 1) * h / plan.rows);
            let ix = (cx1 - cx0) * 3 / 100;
            let iy = (cy1 - cy0) * 3 / 100;
            let cellb = Bounds::new(cx0 + ix, cy0 + iy, cx1 - cx0 - 2 * ix, cy1 - cy0 - 2 * iy);
            let border: Vec<Rgb> = (cellb.x..cellb.right())
                .flat_map(|x| [(x, cellb.y), (x, cellb.bottom() - 1)])
                .chain((cellb.y..cellb.bottom()).flat_map(|y| [(cellb.x, y), (cellb.right() - 1, y)]))
                .map(|(x, y)| {
                    let p = sheet.get_pixel(x, y);
                    [p[0] as f32, p[1] as f32, p[2] as f32]
                })
                .collect();
            let bg = color::mode_color(border.into_iter(), 4)?.0;
            let (mut x0, mut y0, mut x1, mut y1, mut cnt) = (u32::MAX, u32::MAX, 0, 0, 0u32);
            for y in cellb.y..cellb.bottom() {
                for x in cellb.x..cellb.right() {
                    let p = sheet.get_pixel(x, y);
                    if color::dist([p[0] as f32, p[1] as f32, p[2] as f32], bg) > 24.0 {
                        x0 = x0.min(x);
                        y0 = y0.min(y);
                        x1 = x1.max(x);
                        y1 = y1.max(y);
                        cnt += 1;
                    }
                }
            }
            (cnt as u64 * 1000 >= cellb.area()).then(|| Bounds::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1))
        })
        .collect()
}

/// 把 AI 生成的网格图拆成单个图标，覆盖原来的裁剪素材，spec 里标记为 generated。返回写出的文件和警告
pub fn apply_icon_sheet(sheet: &DynamicImage, spec: &mut Spec, plan: &IconSheetPlan, paths: &SpecPaths, opts: &IconOptions) -> Result<(Vec<IconFile>, Vec<String>)> {
    let rgba = sheet.to_rgba8();
    let spots = locate(&rgba, plan);
    let owned = owned_assets(&paths.root, &[&*spec]);
    let mut used = HashSet::new();
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    for (id, spot) in plan.ids.iter().zip(spots) {
        let Some(b) = spot else {
            warnings.push(format!("{id}：生成图里对应的格子是空的，保留原来的素材"));
            continue;
        };
        let Some(tile) = cut_icon(&rgba, &b, opts) else {
            warnings.push(format!("{id}：没能从生成图里取出图标，保留原来的素材"));
            continue;
        };
        let node = spec.root.find_mut(id).ok_or_else(|| Error::UnknownIds(vec![id.clone()]))?;
        let stem = icon_stem(node);
        let mut own = owned.clone();
        if let Some(a) = &node.asset {
            own.insert(abs(&paths.root, &a.path));
        }
        let target = match &node.asset {
            Some(a) if !used.contains(&abs(&paths.root, &a.path)) => {
                let p = abs(&paths.root, &a.path);
                used.insert(p.clone());
                p
            }
            _ => target_path(&paths.icons_dir, &stem, &own, &mut used, id)?,
        };
        imgcore::project::write_atomic(&target, &encode_png(&tile)?).map_err(|e| Error::Invalid(e.to_string()))?;
        let path = rel(&paths.root, &target);
        node.asset = Some(Asset { path: path.clone(), source: AssetSource::Generated, width: tile.width(), height: tile.height() });
        files.push(icon_file(node, stem, path, AssetSource::Generated, tile.width()));
    }
    Ok((files, warnings))
}
