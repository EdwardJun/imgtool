//! SVG 图标：调用对话模型生成一套图标，清洗后保存 .svg 并渲染多尺寸位图（与 MCP 共用 imgcore / imggen）

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use imgcore::project::svg_export::{export_icon_set, validate_color, IconSetItem, SvgExportJob};
use imgcore::project::ConflictPolicy;
use imgcore::OutputFormat;

static BATCH_SEQ: AtomicU64 = AtomicU64::new(0);

/// 位图导出参数（生成和换色共用）
#[derive(Debug, Clone, Deserialize)]
pub struct RenderOptions {
    pub color: String,
    #[serde(default)]
    pub sizes: Vec<u32>,
    #[serde(default)]
    pub format: Option<OutputFormat>,
}

#[derive(Debug, Deserialize)]
pub struct GenerateSvgParams {
    pub base_url: String,
    pub api_key: String,
    #[serde(default)]
    pub model: String,
    /// 每项一个图标名称，可写"名称：说明"
    pub icons: Vec<String>,
    #[serde(default)]
    pub style: imggen::IconStyle,
    #[serde(flatten)]
    pub render: RenderOptions,
}

#[derive(Debug, Serialize)]
pub struct SvgIconSet {
    pub model: Option<String>,
    pub total_tokens: Option<u64>,
    pub elapsed_ms: u64,
    pub out_dir: String,
    pub icons: Vec<IconSetItem>,
    /// 模型没有返回的图标名称
    pub missing: Vec<String>,
}

/// 一个已生成的图标（换色 / 重新导出时由前端传回）
#[derive(Debug, Deserialize)]
pub struct IconSource {
    pub label: String,
    pub name: String,
    pub svg: String,
}

#[derive(Debug, Deserialize)]
pub struct RenderSetParams {
    pub icons: Vec<IconSource>,
    #[serde(flatten)]
    pub render: RenderOptions,
}

/// "名称：说明" → IconSpec
pub fn parse_item(s: &str) -> imggen::IconSpec {
    match s.split_once(['：', ':']) {
        Some((l, h)) if !l.trim().is_empty() => imggen::IconSpec { label: l.trim().into(), hint: Some(h.trim().to_string()).filter(|h| !h.is_empty()) },
        _ => imggen::IconSpec { label: s.trim().into(), hint: None },
    }
}

fn batch_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("svg-icons")
        .join(format!("set-{ts}-{}", BATCH_SEQ.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn template(out_dir: PathBuf, r: &RenderOptions) -> Result<SvgExportJob, String> {
    validate_color(&r.color).map_err(|e| e.to_string())?;
    Ok(SvgExportJob {
        svg: Vec::new(),
        name: String::new(),
        out_dir,
        sizes: r.sizes.clone(),
        format: r.format.unwrap_or(OutputFormat::Png),
        quality: None,
        color: Some(r.color.clone()),
        save_svg: true,
        source: None,
        on_conflict: ConflictPolicy::Rename,
        dry_run: false,
    })
}

/// 生成一整套 SVG 图标并导出到缓存目录（之后可「保存」到用户目录）
#[tauri::command]
pub async fn generate_svg_icons(app: AppHandle, params: GenerateSvgParams) -> Result<SvgIconSet, String> {
    let specs: Vec<imggen::IconSpec> = params.icons.iter().map(|s| parse_item(s)).filter(|s| !s.label.is_empty()).collect();
    imggen::svg_icons::validate(&specs, &params.style).map_err(|e| e.to_string())?;
    let dir = batch_dir(&app)?;
    let tpl = template(dir.clone(), &params.render)?;
    let started = Instant::now();
    let generator = imggen::SvgIconGenerator::new(&params.base_url, &params.api_key, &params.model).map_err(|e| e.to_string())?;
    let set = generator.generate(&specs, &params.style).await.map_err(|e| e.to_string())?;
    let items: Vec<(String, String, String)> = set.icons.iter().map(|i| (i.label.clone(), imggen::svg_icons::file_stem(i), i.svg.clone())).collect();
    let icons = tauri::async_runtime::spawn_blocking(move || export_icon_set(&items, &tpl)).await.map_err(|e| e.to_string())?;
    Ok(SvgIconSet {
        model: Some(set.model),
        total_tokens: set.total_tokens,
        elapsed_ms: started.elapsed().as_millis() as u64,
        out_dir: dir.display().to_string(),
        icons,
        missing: set.missing,
    })
}

/// 用已有的 SVG 重新导出（换颜色、换尺寸），不调用模型
#[tauri::command]
pub async fn render_svg_set(app: AppHandle, params: RenderSetParams) -> Result<SvgIconSet, String> {
    if params.icons.is_empty() {
        return Err("没有可导出的图标".into());
    }
    let dir = batch_dir(&app)?;
    let tpl = template(dir.clone(), &params.render)?;
    let started = Instant::now();
    let items: Vec<(String, String, String)> = params.icons.into_iter().map(|i| (i.label, i.name, i.svg)).collect();
    let icons = tauri::async_runtime::spawn_blocking(move || export_icon_set(&items, &tpl)).await.map_err(|e| e.to_string())?;
    Ok(SvgIconSet { model: None, total_tokens: None, elapsed_ms: started.elapsed().as_millis() as u64, out_dir: dir.display().to_string(), icons, missing: vec![] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_items_and_payload() {
        let a = parse_item("发现：指南针");
        assert_eq!((a.label.as_str(), a.hint.as_deref()), ("发现", Some("指南针")));
        let p: GenerateSvgParams = serde_json::from_str(
            r##"{"base_url":"https://x","api_key":"k","icons":["首页"],"style":{"variant":"filled","stroke_width":1.5,"corner":"square"},"color":"#4F46E5","sizes":[32,64],"format":"webp"}"##,
        )
        .unwrap();
        assert_eq!(p.style.variant, imggen::IconVariant::Filled);
        assert_eq!((p.render.color.as_str(), p.render.sizes.as_slice(), p.render.format), ("#4F46E5", &[32, 64][..], Some(OutputFormat::Webp)));
    }

    #[test]
    fn template_rejects_bad_color() {
        let bad = RenderOptions { color: "red\" x=\"".into(), sizes: vec![], format: None };
        assert!(template(PathBuf::from("/tmp"), &bad).is_err());
    }
}
