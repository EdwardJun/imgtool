//! 设计解析（imgspec）：解析、读取、保存（人工修正后）、局部重新取样式、整理层级、AI 重绘图标

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use imgcore::project::{ProjectPreset, SpecPolicy};
use imgspec::{Bounds, IconFile, Node, RunOptions, Spec, SpecPaths};

/// 项目根目录：最近的 .imgtool.toml 所在目录（与 MCP 一致）；没有预设时用图片所在目录
fn context(image: &Path) -> (PathBuf, SpecPolicy) {
    match ProjectPreset::find(image, None) {
        Ok(Some((p, preset))) => (p.parent().unwrap_or(Path::new("/")).to_path_buf(), preset.spec),
        _ => (image.parent().unwrap_or(Path::new("/")).to_path_buf(), SpecPolicy::default()),
    }
}

fn paths_of(image: &Path) -> (SpecPaths, SpecPolicy) {
    let (root, policy) = context(image);
    (imgspec::spec_paths(&root, image, policy.icons_dir.as_deref()), policy)
}

#[derive(Debug, Serialize)]
pub struct DesignResult {
    pub spec: Spec,
    /// spec 里的相对路径都相对这个目录
    pub root: String,
    /// 这张图的解析文件夹（spec、标注图、图标素材都在里面）
    pub dir: String,
    pub spec_path: String,
    pub overlay_path: String,
    pub icons: Vec<IconFile>,
    pub warnings: Vec<String>,
    pub kept_annotations: usize,
}

fn result(paths: &SpecPaths, spec: Spec, icons: Vec<IconFile>, warnings: Vec<String>, kept: usize) -> DesignResult {
    DesignResult {
        spec,
        root: paths.root.display().to_string(),
        dir: paths.dir.display().to_string(),
        spec_path: paths.spec.display().to_string(),
        overlay_path: paths.overlay.display().to_string(),
        icons,
        warnings,
        kept_annotations: kept,
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn analyze_design(path: String, options: RunOptions) -> Result<DesignResult, String> {
    blocking(move || {
        let image = PathBuf::from(&path);
        let (paths, policy) = paths_of(&image);
        let mut opts = options;
        opts.icons.size = policy.icon_size;
        let r = imgspec::analyze_file(&paths, &opts).map_err(|e| e.to_string())?;
        Ok(result(&paths, r.spec, r.icons, vec![], r.kept_annotations))
    })
    .await
}

/// 读取已有的解析结果；没有解析过时返回 None
#[tauri::command]
pub async fn load_design_spec(path: String) -> Result<Option<DesignResult>, String> {
    blocking(move || {
        let (paths, _) = paths_of(Path::new(&path));
        if !paths.spec.is_file() {
            return Ok(None);
        }
        let spec = imgspec::store::load(&paths.spec).map_err(|e| e.to_string())?;
        Ok(Some(result(&paths, spec, vec![], vec![], 0)))
    })
    .await
}

/// 保存人工修正后的 spec：按当前的框和名称重新裁出图标素材，重画标注图
#[tauri::command]
pub async fn save_design_spec(path: String, spec: Spec) -> Result<DesignResult, String> {
    blocking(move || {
        let image = PathBuf::from(&path);
        let (paths, policy) = paths_of(&image);
        let previous = paths.spec.is_file().then(|| imgspec::store::load(&paths.spec).ok()).flatten();
        let img = imgspec::decode_file(&image).map_err(|e| e.to_string())?;
        let mut spec = spec;
        spec.annotated = spec.annotated || {
            let mut any = false;
            spec.root.walk(&mut |n, _| any |= n.has_semantics());
            any
        };
        let opts = imgspec::IconOptions { size: policy.icon_size, ..Default::default() };
        let icons = imgspec::icons::export_icons(&img, &mut spec, &paths, &opts, previous.as_ref()).map_err(|e| e.to_string())?;
        imgspec::store::save(&spec, &paths.spec).map_err(|e| e.to_string())?;
        imgspec::write_overlay(&img, &spec, &paths).map_err(|e| e.to_string())?;
        Ok(result(&paths, spec, icons, vec![], 0))
    })
    .await
}

/// 框被移动或缩放后：重新取这块区域的样式
#[tauri::command]
pub async fn reanalyze_region(path: String, bounds: Bounds) -> Result<Node, String> {
    blocking(move || {
        let img = imgspec::decode_file(Path::new(&path)).map_err(|e| e.to_string())?;
        imgspec::analyze_region(&img, bounds).map_err(|e| e.to_string())
    })
    .await
}

/// 按包含关系重新整理层级、重新推断布局（编号不变）
#[tauri::command]
pub async fn normalize_spec(spec: Spec) -> Result<Spec, String> {
    blocking(move || {
        let mut spec = spec;
        imgspec::layout::normalize(&mut spec);
        Ok(spec)
    })
    .await
}

#[derive(Debug, Deserialize)]
pub struct DesignIconsParams {
    pub path: String,
    pub base_url: String,
    pub api_key: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub ids: Option<Vec<String>>,
    #[serde(default)]
    pub style: Option<String>,
}

/// 把图标画在一张网格图上交给 AI 一次重绘，再拆开替换素材
#[tauri::command]
pub async fn generate_design_icons(ref_cache: tauri::State<'_, crate::references::RefCache>, params: DesignIconsParams) -> Result<DesignResult, String> {
    let image = PathBuf::from(&params.path);
    let (paths, policy) = paths_of(&image);
    let (p2, ids, style) = (paths.clone(), params.ids.clone(), params.style.clone());
    let (plan, ref_path) = blocking(move || {
        let spec = imgspec::store::load(&p2.spec).map_err(|_| "请先解析这张图".to_string())?;
        let plan = imgspec::icons::plan_icon_sheet(&spec, ids.as_deref(), style.as_deref()).map_err(|e| e.to_string())?;
        let img = imgspec::decode_file(&p2.image).map_err(|e| e.to_string())?;
        let sheet = imgspec::icons::reference_sheet(&img, &spec, &plan).map_err(|e| e.to_string())?;
        imgcore::project::write_atomic(&p2.reference, &sheet).map_err(|e| e.to_string())?;
        // 第二张参考图：完整设计稿上框出图标，让模型结合位置和旁边的文字理解含义
        let context = imgspec::icons::context_sheet(&img, &spec, &plan).map_err(|e| e.to_string())?;
        imgcore::project::write_atomic(&p2.context, &context).map_err(|e| e.to_string())?;
        Ok((plan, vec![p2.reference.display().to_string(), p2.context.display().to_string()]))
    })
    .await?;
    let refs = crate::references::resolve_references(&ref_cache, &params.base_url, &params.api_key, &ref_path).await?;
    let quality = params.quality.clone().unwrap_or_else(|| "1K".into());
    let (_, (width, height)) = imggen::sizes::match_ratio(plan.cols, plan.rows, &quality);
    let model = if params.model.trim().is_empty() { "gpt-image-2".to_string() } else { params.model.trim().to_string() };
    let provider = imggen::TaskApiProvider::new(imggen::TaskApiConfig { base_url: params.base_url.clone(), api_key: params.api_key.clone(), model }).map_err(|e| e.to_string())?;
    let req = imggen::GenerateRequest { prompt: plan.prompt.clone(), width: Some(width), height: Some(height), count: Some(1), reference_images: refs, ..Default::default() };
    let (_, images) = provider.run_once(&req).await.map_err(|e| e.to_string())?;
    let data = images.into_iter().next().ok_or("没有返回图片")?.data;
    blocking(move || {
        let sheet = imgcore::decode::decode(&data).map_err(|e| e.to_string())?.image;
        // 生成的网格原图也放进解析文件夹，方便对照
        let stem = paths.image.file_stem().and_then(|s| s.to_str()).unwrap_or("design").to_string();
        imgcore::project::write_atomic(&paths.dir.join(format!("{stem}-icons-sheet.png")), &data).map_err(|e| e.to_string())?;
        let mut spec = imgspec::store::load(&paths.spec).map_err(|e| e.to_string())?;
        let opts = imgspec::IconOptions { size: policy.icon_size, ..Default::default() };
        let (icons, mut warnings) = imgspec::icons::apply_icon_sheet(&sheet, &mut spec, &plan, &paths, &opts).map_err(|e| e.to_string())?;
        imgspec::store::save(&spec, &paths.spec).map_err(|e| e.to_string())?;
        if !plan.skipped.is_empty() {
            warnings.insert(0, format!("跳过 {} 个太小的图标（原图长边小于 {}px）：{}", plan.skipped.len(), imgspec::icons::MIN_REDRAW_PX, plan.skipped.iter().map(|s| s.id.as_str()).collect::<Vec<_>>().join("、")));
        }
        Ok(result(&paths, spec, icons, warnings, 0))
    })
    .await
}

/// 在系统文件管理器里打开一个文件夹（Finder / 资源管理器 / 默认文件管理器）。
/// 只接受已存在的目录，传文件时打开它所在的目录
#[tauri::command]
pub async fn open_folder(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    let dir = if p.is_dir() { p } else { p.parent().map(Path::to_path_buf).filter(|d| d.is_dir()).ok_or_else(|| format!("文件夹不存在：{path}"))? };
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    // 直接传参数，不经过 shell
    std::process::Command::new(program).arg(&dir).spawn().map_err(|e| format!("打开文件夹失败：{e}"))?;
    Ok(())
}

/// 生成结果自动解析（图片生成页的"生成后解析设计稿"）
pub fn analyze_generated(path: &Path) -> Result<String, String> {
    let (paths, policy) = paths_of(path);
    let opts = RunOptions { icons: imgspec::IconOptions { size: policy.icon_size, ..Default::default() }, ..Default::default() };
    imgspec::analyze_file(&paths, &opts).map_err(|e| e.to_string())?;
    Ok(paths.spec.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_falls_back_to_image_dir_and_prefers_preset() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("a/b/ui.png");
        std::fs::create_dir_all(img.parent().unwrap()).unwrap();
        image::RgbaImage::from_pixel(120, 80, image::Rgba([255, 255, 255, 255])).save(&img).unwrap();
        let (p, _) = paths_of(&img);
        assert_eq!(p.dir, dir.path().join("a/b/.imgtool/specs/ui"));
        assert_eq!(p.spec, dir.path().join("a/b/.imgtool/specs/ui/ui.spec.json"));
        assert_eq!(p.icons_dir, dir.path().join("a/b/.imgtool/specs/ui/icons"));
        std::fs::write(dir.path().join(".imgtool.toml"), "[spec]\nicons_dir = \"assets/icons\"\n").unwrap();
        let (p, policy) = paths_of(&img);
        assert_eq!(p.spec, dir.path().join(".imgtool/specs/a/b/ui/ui.spec.json"));
        assert_eq!(policy.icons_dir.as_deref(), Some("assets/icons"));
        assert_eq!(p.icons_dir, dir.path().join("assets/icons"));
    }

    #[test]
    fn deserializes_frontend_payloads() {
        let o: RunOptions = serde_json::from_str(r#"{"analyze":{"mode":"component","tolerance":3},"export_icons":false}"#).unwrap();
        assert_eq!(o.analyze.mode, Some(imgspec::SpecKind::Component));
        assert!(!o.export_icons && o.keep_annotations);
        let b: Bounds = serde_json::from_str(r#"{"x":1,"y":2,"width":3,"height":4}"#).unwrap();
        assert_eq!(b.width, 3);
        let p: DesignIconsParams = serde_json::from_str(r#"{"path":"/a.png","base_url":"https://x","api_key":"k","ids":["n3"]}"#).unwrap();
        assert_eq!(p.ids.unwrap(), ["n3"]);
    }
}
