//! MCP 的 SVG 工具：render_svg（渲染 Agent 自己写的或项目里的 SVG）、generate_svg_icons（调用对话模型生成整套图标）

use std::path::Path;

use base64::Engine;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{tool, tool_router, ErrorData as McpError, Peer, RoleServer};
use serde::Deserialize;

use imgcore::project::svg_export::{export_icon_set, export_svg, read_svg_file, SvgExportJob};
use imgcore::project::ConflictPolicy;
use imgcore::OutputFormat;

use super::{blocking, tool_error, ImgToolServer};

pub const ICONS_DIR: &str = ".imgtool/icons";
/// 生成图标默认颜色（深灰，适合浅色背景）
pub const DEFAULT_ICON_COLOR: &str = "#1F2937";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenderSvgParams {
    /// SVG 源码（例如你自己写的图标）。与 path 二选一
    #[serde(default)]
    pub svg: Option<String>,
    /// 项目内的 .svg 文件路径。与 svg 二选一
    #[serde(default)]
    pub path: Option<String>,
    /// 输出文件名（不含扩展名），默认取 path 的文件名或 "icon"
    #[serde(default)]
    pub name: Option<String>,
    /// 输出的正方形边长列表（像素，8-4096，最多 12 个），默认 [64, 128, 256]
    #[serde(default)]
    pub sizes: Option<Vec<u32>>,
    /// 位图格式 png（默认）/ webp / avif / jpeg（jpeg 无透明，填白底）
    #[serde(default)]
    pub format: Option<OutputFormat>,
    /// 位图质量 1-100；png 不传时无损
    #[serde(default)]
    pub quality: Option<u8>,
    /// 线条 / 填充颜色，如 "#4F46E5"；SVG 里用 currentColor 的部分会变成这个颜色
    #[serde(default)]
    pub color: Option<String>,
    /// 是否同时保存清洗后的 .svg；传 svg 源码时默认 true，传 path 时默认 false
    #[serde(default)]
    pub save_svg: Option<bool>,
    /// 输出目录（项目内），传 svg 源码时默认 .imgtool/icons，传 path 时默认与该文件同目录
    #[serde(default)]
    pub out_dir: Option<String>,
    /// 目标文件已存在时：error（默认）/ overwrite / rename
    #[serde(default)]
    pub on_conflict: Option<ConflictPolicy>,
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    /// 线性描边（默认）
    #[default]
    Outline,
    /// 面性实心
    Filled,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    /// 圆角端点（默认）
    #[default]
    Round,
    /// 直角端点
    Square,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GenerateSvgIconsParams {
    /// 图标清单，每项一个图标名称，可用"名称：说明"补充画法，如 ["首页", "发现：指南针", "购物车"]；最多 12 个
    pub icons: Vec<String>,
    /// outline（线性，默认）/ filled（面性）
    #[serde(default)]
    pub variant: Option<Variant>,
    /// 24×24 画布上的线宽 0.5-4，默认 2
    #[serde(default)]
    pub stroke_width: Option<f32>,
    /// round（默认）/ square
    #[serde(default)]
    pub corner: Option<Corner>,
    /// 额外的风格要求，如"圆润可爱"、"科技感、细节少"
    #[serde(default)]
    pub style_notes: Option<String>,
    /// 图标颜色，默认 #1F2937
    #[serde(default)]
    pub color: Option<String>,
    /// 每个图标导出的位图尺寸，默认 [64, 128, 256]
    #[serde(default)]
    pub sizes: Option<Vec<u32>>,
    /// 位图格式，默认 png
    #[serde(default)]
    pub format: Option<OutputFormat>,
    /// 输出目录（项目内），默认 .imgtool/icons
    #[serde(default)]
    pub out_dir: Option<String>,
    /// 对话模型，默认用桌面端「设置」里的提示词模型（deepseek-v4.1-flash）
    #[serde(default)]
    pub model: Option<String>,
    /// 目标文件已存在时：rename（默认，自动加序号）/ overwrite / error
    #[serde(default)]
    pub on_conflict: Option<ConflictPolicy>,
}

/// "名称：说明" → (名称, 说明)
pub fn parse_icon_item(s: &str) -> imggen::IconSpec {
    match s.split_once(['：', ':']) {
        Some((l, h)) if !l.trim().is_empty() => imggen::IconSpec { label: l.trim().into(), hint: Some(h.trim().into()).filter(|h: &String| !h.is_empty()) },
        _ => imggen::IconSpec { label: s.trim().into(), hint: None },
    }
}

fn png_block(img: &image::RgbaImage) -> Option<ContentBlock> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    Some(ContentBlock::image(base64::engine::general_purpose::STANDARD.encode(buf.into_inner()), "image/png"))
}

#[tool_router(router = svg_tool_router, vis = "pub(super)")]
impl ImgToolServer {
    #[tool(
        description = "把 SVG 渲染成多个尺寸的 PNG/WebP（适合线性图标、简单 logo），可统一换色，返回文件路径和预览图。简单图标推荐你直接写 SVG（viewBox 0 0 24 24、stroke/fill 用 currentColor）交给本工具，比 AI 绘图快、体积小、不花额度。保存前会清洗 SVG（去掉脚本、事件、外部引用）。输出 <name>.svg 和 <name>-<尺寸>.<格式>。",
        annotations(destructive_hint = false)
    )]
    async fn render_svg(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<RenderSvgParams>) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(&peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        let (data, source, default_name, default_dir) = match (&p.svg, &p.path) {
            (Some(s), None) => (s.clone().into_bytes(), None, "icon".to_string(), ws.primary().join(ICONS_DIR)),
            (None, Some(path)) => {
                let abs = match ws.resolve_existing(path) {
                    Ok(a) => a,
                    Err(e) => return tool_error(e),
                };
                let data = match read_svg_file(&abs) {
                    Ok(d) => d,
                    Err(e) => return tool_error(e.to_string()),
                };
                let stem = abs.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "icon".into());
                let dir = abs.parent().map(Path::to_path_buf).unwrap_or_else(|| ws.primary().to_path_buf());
                (data, Some(abs), stem, dir)
            }
            _ => return tool_error("请只提供 svg（源码）或 path（项目内 .svg 文件）中的一个"),
        };
        let out_dir = match &p.out_dir {
            Some(d) => match ws.resolve_output(d) {
                Ok(d) => d,
                Err(e) => return tool_error(e),
            },
            None => default_dir,
        };
        let job = SvgExportJob {
            svg: data,
            name: p.name.clone().unwrap_or(default_name),
            out_dir,
            sizes: p.sizes.clone().unwrap_or_default(),
            format: p.format.unwrap_or(OutputFormat::Png),
            quality: p.quality,
            color: p.color.clone(),
            save_svg: p.save_svg.unwrap_or(source.is_none()),
            source,
            on_conflict: p.on_conflict.unwrap_or(ConflictPolicy::Error),
            dry_run: p.dry_run.unwrap_or(false),
        };
        let res = blocking(move || {
            let r = export_svg(&job)?;
            let preview = imgcore::svg::render_square_sizes(r.svg.as_bytes(), &[256]).ok().and_then(|v| v.into_iter().next()).map(|v| v.image);
            Ok::<_, imgcore::Error>((r, preview))
        })
        .await?;
        match res {
            Ok((mut r, preview)) => {
                for f in &mut r.files {
                    f.path = ws.relative(Path::new(&f.path));
                }
                let verb = if r.files.iter().any(|f| f.written) { "已导出" } else { "预计导出" };
                let mut summary = format!("{verb} {} 个文件：{}", r.files.len(), r.files.iter().map(|f| format!("{}（{}）", f.path, imgcore_bytes(f.bytes))).collect::<Vec<_>>().join("、"));
                if !r.warnings.is_empty() {
                    summary.push_str(&format!("。注意：{}", r.warnings.join("；")));
                }
                let mut blocks = vec![ContentBlock::text(summary), ContentBlock::json(&r)?];
                // 预览铺白底，透明线条在任何客户端里都看得清
                if let Some(img) = preview.map(|img| on_white(&img)).as_ref().and_then(png_block) {
                    blocks.push(img);
                }
                Ok(CallToolResult::success(blocks))
            }
            Err(e) => tool_error(e.to_string()),
        }
    }

    #[tool(
        description = "调用对话大模型一次生成一套风格统一的 SVG 图标（线性或面性），清洗后保存 .svg 并渲染成多尺寸 PNG，返回全部文件和一张总览预览图。一般 4-8 个图标约 6-30 秒，会消耗少量对话 token。也可以不调用本工具，自己写 SVG 后用 render_svg 渲染。",
        annotations(destructive_hint = false, open_world_hint = true)
    )]
    async fn generate_svg_icons(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<GenerateSvgIconsParams>) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(&peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        let cfg = match crate::genconfig::load() {
            Ok(c) => c,
            Err(e) => return tool_error(e),
        };
        let specs: Vec<imggen::IconSpec> = p.icons.iter().map(|s| parse_icon_item(s)).collect();
        let style = imggen::IconStyle {
            variant: match p.variant.unwrap_or_default() {
                Variant::Outline => imggen::IconVariant::Outline,
                Variant::Filled => imggen::IconVariant::Filled,
            },
            stroke_width: p.stroke_width.unwrap_or(2.0),
            corner: match p.corner.unwrap_or_default() {
                Corner::Round => imggen::IconCorner::Round,
                Corner::Square => imggen::IconCorner::Square,
            },
            notes: p.style_notes.clone(),
        };
        if let Err(e) = imggen::svg_icons::validate(&specs, &style) {
            return tool_error(e.to_string());
        }
        let color = p.color.clone().unwrap_or_else(|| DEFAULT_ICON_COLOR.into());
        if let Err(e) = imgcore::project::svg_export::validate_color(&color) {
            return tool_error(e.to_string());
        }
        let out_dir = match ws.resolve_output(p.out_dir.as_deref().unwrap_or(ICONS_DIR)) {
            Ok(d) => d,
            Err(e) => return tool_error(e),
        };
        let out_rel = ws.relative(&out_dir);
        let model = p.model.clone().filter(|m| !m.trim().is_empty()).unwrap_or(cfg.prompt_model.clone());
        let generator = match imggen::SvgIconGenerator::new(&cfg.base_url, &cfg.api_key, &model) {
            Ok(g) => g,
            Err(e) => return tool_error(e.to_string()),
        };
        let started = std::time::Instant::now();
        let set = match generator.generate(&specs, &style).await {
            Ok(s) => s,
            Err(e) => return tool_error(e.to_string()),
        };
        let template = SvgExportJob {
            svg: Vec::new(),
            name: String::new(),
            out_dir,
            sizes: p.sizes.clone().unwrap_or_default(),
            format: p.format.unwrap_or(OutputFormat::Png),
            quality: None,
            color: Some(color),
            save_svg: true,
            source: None,
            on_conflict: p.on_conflict.unwrap_or(ConflictPolicy::Rename),
            dry_run: false,
        };
        let items: Vec<(String, String, String)> = set.icons.iter().map(|i| (i.label.clone(), imggen::svg_icons::file_stem(i), i.svg.clone())).collect();
        let (mut exported, sheet) = blocking(move || {
            let exported = export_icon_set(&items, &template);
            let svgs: Vec<&[u8]> = exported.iter().filter_map(|e| e.result.as_ref().map(|r| r.svg.as_bytes())).collect();
            let sheet = imgcore::svg::contact_sheet(&svgs, 96, 6);
            (exported, sheet)
        })
        .await?;
        for item in &mut exported {
            if let Some(r) = &mut item.result {
                for f in &mut r.files {
                    f.path = ws.relative(Path::new(&f.path));
                }
            }
        }
        let ok: Vec<&str> = exported.iter().filter(|e| e.result.is_some()).map(|e| e.label.as_str()).collect();
        let failed: Vec<String> = exported.iter().filter_map(|e| e.error.as_ref().map(|err| format!("{}：{err}", e.label))).chain(set.missing.iter().map(|m| format!("{m}：模型没有返回"))).collect();
        let mut summary = format!(
            "用 {}（{:.1} 秒）生成 {} 个图标：{}，已保存到 {}",
            set.model,
            started.elapsed().as_secs_f32(),
            ok.len(),
            ok.join("、"),
            out_rel
        );
        if !failed.is_empty() {
            summary.push_str(&format!("。失败 {} 个：{}（可只针对这些重试，或自己写 SVG 用 render_svg）", failed.len(), failed.join("；")));
        }
        let json = serde_json::json!({ "model": set.model, "total_tokens": set.total_tokens, "icons": exported, "failed": failed });
        let mut blocks = vec![ContentBlock::text(summary), ContentBlock::json(json)?];
        if !ok.is_empty() {
            if let Some(b) = png_block(&sheet) {
                blocks.push(b);
            }
        }
        Ok(if ok.is_empty() { CallToolResult::error(blocks) } else { CallToolResult::success(blocks) })
    }
}

fn on_white(img: &image::RgbaImage) -> image::RgbaImage {
    let mut bg = image::RgbaImage::from_pixel(img.width(), img.height(), image::Rgba([255, 255, 255, 255]));
    image::imageops::overlay(&mut bg, img, 0, 0);
    bg
}

fn imgcore_bytes(b: u64) -> String {
    crate::batch::human_bytes(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_icon_items() {
        let a = parse_icon_item("发现：指南针");
        assert_eq!((a.label.as_str(), a.hint.as_deref()), ("发现", Some("指南针")));
        let b = parse_icon_item(" home ");
        assert_eq!((b.label.as_str(), b.hint), ("home", None));
        let c = parse_icon_item("设置:");
        assert_eq!((c.label.as_str(), c.hint), ("设置", None));
    }
}
