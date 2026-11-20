//! 设计稿解析（imgspec）的 MCP 工具：analyze_design / annotate_design / generate_design_icons

use std::path::PathBuf;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{tool, tool_router, ErrorData as McpError, Peer, RoleServer};
use serde::Deserialize;

use imgcore::OutputFormat;
use imgspec::{Annotation, NodeSource, SpecKind};

use super::{blocking, load_preset, ok_json, preview_block, tool_error, ImgToolServer};
use crate::gen_tasks::{GenJob, PostProcess};
use crate::workspace::Workspace;

/// 返回给 Agent 的精简树最多列出的节点数
const OUTLINE_MAX: usize = 300;

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Detail {
    /// 精简文本树（默认）
    Outline,
    /// 额外附上完整 spec JSON
    Full,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnalyzeDesignParams {
    /// 设计稿或样式图片，相对项目根目录（对话里的图片先用 import_image 存进项目）
    pub path: String,
    /// page（整页设计稿）/ component（单个样式：卡片、按钮、标签等）；不传自动判断
    #[serde(default)]
    pub mode: Option<SpecKind>,
    /// 与底色的容差 0-100，不传自动（约 2）；细节被漏掉时调小，噪点太多时调大
    #[serde(default)]
    pub tolerance: Option<u8>,
    /// 最大嵌套层数，默认 6
    #[serde(default)]
    pub max_depth: Option<usize>,
    /// 已有 spec 时保留之前的语义标注（按位置匹配），默认 true
    #[serde(default)]
    pub keep_annotations: Option<bool>,
    /// 把识别出的图标裁成透明底素材，默认 true
    #[serde(default)]
    pub export_icons: Option<bool>,
    /// 返回内容：outline（默认，精简文本树）/ full（附完整 spec JSON）
    #[serde(default)]
    pub detail: Option<Detail>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnnotateDesignParams {
    /// spec 文件（.spec.json）或原图路径，相对项目根目录
    pub spec: String,
    /// 按编号补充语义；编号就是标注图上的数字（12 或 n12 都可以）
    pub annotations: Vec<Annotation>,
    /// 对整张设计稿的说明（页面用途、整体风格等）
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GenerateDesignIconsParams {
    /// spec 文件（.spec.json）或原图路径，相对项目根目录
    pub spec: String,
    /// 要重绘的节点编号，默认全部 icon 节点（一次最多 16 个）
    #[serde(default)]
    pub ids: Option<Vec<String>>,
    /// 风格描述，如 "线性图标，2px 线宽，圆角端点"；不传则与原图保持一致
    #[serde(default)]
    pub style: Option<String>,
    /// 图像模型，默认 gpt-image-2
    #[serde(default)]
    pub model: Option<String>,
    /// 质量档位 1K / 2K / 4K，默认 1K（需模型支持）
    #[serde(default)]
    pub quality: Option<String>,
    /// 图标素材边长（像素），默认读项目预设 [spec] icon_size（256）
    #[serde(default)]
    pub size: Option<u32>,
}
/// spec 参数：.spec.json 直接用；图片路径换算成它的 spec 文件。返回 (项目根目录, spec 文件)
fn resolve_spec(ws: &Workspace, input: &str) -> Result<(PathBuf, PathBuf), String> {
    let abs = ws.resolve_existing(input)?;
    let root = ws.root_of(&abs).unwrap_or(ws.primary()).to_path_buf();
    if imgspec::store::is_spec_file(&abs) {
        return Ok((root, abs));
    }
    let spec = imgspec::spec_paths(&root, &abs, None).spec;
    if !spec.is_file() {
        return Err(format!("{input} 还没有解析过，请先调用 analyze_design"));
    }
    Ok((root, spec))
}

fn icons_line(icons: &[imgspec::IconFile]) -> String {
    if icons.is_empty() {
        return String::new();
    }
    let list: Vec<String> = icons
        .iter()
        .take(20)
        .map(|i| if i.low_res { format!("{}→{}（原图仅 {}px，较模糊）", i.id, i.path, i.native) } else { format!("{}→{}", i.id, i.path) })
        .collect();
    format!("\n图标素材（{} 个）：{}{}", icons.len(), list.join("、"), if icons.len() > 20 { " ……" } else { "" })
}

#[tool_router(router = design_tool_router, vis = "pub(super)")]
impl ImgToolServer {
    #[tool(
        description = "设计稿解析：把 UI 设计稿（整页）或单个样式图（卡片、按钮、标签、输入框）转换成结构化描述——嵌套节点树、每个元素的像素坐标（绝对 + 相对父节点）、类型（frame/rect/text/icon/image/line）、填充/渐变/描边/圆角/阴影、文字颜色与估算字号字重、推断的自动布局（row/column、gap、padding、对齐）。纯本地算法，不花额度，不做 OCR。返回精简树、spec 文件路径和编号标注图：请看标注图，再用 annotate_design 按编号补上语义（role、name、文字内容）。识别出的图标会自动裁成透明底 PNG 素材；需要高清版时调用 generate_design_icons。一张图的全部产物（<原名>.spec.json、<原名>.spec.png 标注图、icons/ 图标素材）放在同一个文件夹 .imgtool/specs/<相对目录>/<原名>/，再次解析会保留已有标注。",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    async fn analyze_design(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<AnalyzeDesignParams>) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(&peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        let image = match ws.resolve_existing(&p.path) {
            Ok(v) if v.is_file() => v,
            Ok(_) => return tool_error("path 需要是图片文件"),
            Err(e) => return tool_error(e),
        };
        let root = ws.root_of(&image).unwrap_or(ws.primary()).to_path_buf();
        let preset = match load_preset(&ws, &image) {
            Ok((_, p)) => p.spec,
            Err(e) => return tool_error(e),
        };
        let paths = imgspec::spec_paths(&root, &image, preset.icons_dir.as_deref());
        let opts = imgspec::RunOptions {
            analyze: imgspec::AnalyzeOptions { mode: p.mode, tolerance: p.tolerance, max_depth: p.max_depth.unwrap_or(6).clamp(1, 12), ..Default::default() },
            keep_annotations: p.keep_annotations.unwrap_or(true),
            export_icons: p.export_icons.unwrap_or(true),
            icons: imgspec::IconOptions { size: preset.icon_size, ..Default::default() },
        };
        let overlay = paths.overlay.clone();
        let dir = ws.relative(&paths.dir);
        let res = blocking(move || -> Result<_, String> {
            let r = imgspec::analyze_file(&paths, &opts).map_err(|e| e.to_string())?;
            let img = preview_block(&overlay, 1024)?;
            Ok((r, img))
        })
        .await?;
        let (r, img) = match res {
            Ok(v) => v,
            Err(e) => return tool_error(e),
        };
        let count = r.spec.root.count();
        let kind = if r.spec.kind == SpecKind::Component { "单个样式" } else { "整页设计稿" };
        let mut summary = format!(
            "已解析为{kind}：{}×{}，{count} 个节点，产物都在文件夹 {dir}/：spec {}，编号标注图 {}（见附图）。{}",
            r.spec.source.width,
            r.spec.source.height,
            r.spec_path,
            r.overlay_path,
            if r.kept_annotations > 0 { format!("保留了 {} 个旧标注。", r.kept_annotations) } else { String::new() }
        );
        summary.push_str("下一步：看标注图，用 annotate_design 按编号写入 role/name/text（图标的 name 会作为素材文件名，建议英文）。坐标是原图像素；字号、阴影为估算值。");
        summary.push_str(&icons_line(&r.icons));
        let tree = imgspec::outline::outline(&r.spec, OUTLINE_MAX);
        let mut value = serde_json::json!({
            "dir": dir,
            "spec_path": r.spec_path,
            "overlay_path": r.overlay_path,
            "kind": r.spec.kind,
            "nodes": count,
            "palette": r.spec.palette,
            "icons": r.icons,
            "kept_annotations": r.kept_annotations,
        });
        if p.detail == Some(Detail::Full) {
            value["spec"] = serde_json::to_value(&r.spec).unwrap_or_default();
        }
        Ok(CallToolResult::success(vec![ContentBlock::text(summary), ContentBlock::text(tree), ContentBlock::json(value)?, img]))
    }
    #[tool(
        description = "把语义写回 analyze_design 产出的 spec：按编号（标注图上的数字）填写 role（如 button / card / nav_bar / tab_item / avatar / title / price / input）、name、text（画面上的文字原文）、description，也可以修正 type。图标填了 name 后素材文件会自动改名为 <name>.png。有任何未知编号时整体不修改并报错。传空字符串可清空字段。",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    async fn annotate_design(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<AnnotateDesignParams>) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(&peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        let (root, spec_file) = match resolve_spec(&ws, &p.spec) {
            Ok(v) => v,
            Err(e) => return tool_error(e),
        };
        if p.annotations.is_empty() && p.notes.is_none() {
            return tool_error("annotations 不能为空");
        }
        let icons_dir = load_preset(&ws, &spec_file).map(|(_, p)| p.spec.icons_dir).unwrap_or_default();
        let res = blocking(move || -> Result<_, String> {
            let mut spec = imgspec::store::load(&spec_file).map_err(|e| e.to_string())?;
            let ids = imgspec::store::apply_annotations(&mut spec, &p.annotations, NodeSource::Agent).map_err(|e| e.to_string())?;
            if let Some(n) = &p.notes {
                spec.notes = Some(n.trim().to_string()).filter(|s| !s.is_empty());
                spec.annotated = true;
            }
            let paths = imgspec::store::paths_for_spec(&root, &spec_file, &spec, icons_dir.as_deref());
            let renamed = imgspec::icons::rename_assets(&mut spec, &paths).map_err(|e| e.to_string())?;
            imgspec::store::save(&spec, &spec_file).map_err(|e| e.to_string())?;
            // 类型可能改了：重画标注图（原图不在了就跳过）
            if let Ok(img) = imgspec::decode_file(&paths.image) {
                let _ = imgspec::write_overlay(&img, &spec, &paths);
            }
            let lines: Vec<String> = ids.iter().filter_map(|id| spec.root.find(id)).map(imgspec::outline::node_line).collect();
            Ok((imgspec::store::rel(&root, &spec_file), ids, renamed, lines))
        })
        .await?;
        match res {
            Ok((spec_path, ids, renamed, lines)) => {
                let mut summary = format!("已更新 {} 个节点，写入 {spec_path}", ids.len());
                if !renamed.is_empty() {
                    summary.push_str(&format!("；图标素材改名：{}", renamed.iter().map(|i| i.path.as_str()).collect::<Vec<_>>().join("、")));
                }
                summary.push('\n');
                summary.push_str(&lines.join("\n"));
                ok_json(summary, serde_json::json!({ "spec_path": spec_path, "updated": ids, "renamed_icons": renamed }))
            }
            Err(e) => tool_error(e),
        }
    }

    #[tool(
        description = "把设计稿里的图标交给 AI 重绘成高清版：选中的图标按网格画在一张图上一次生成（风格统一、只消耗一次生成额度），再自动拆开、去掉白底，覆盖原来的裁剪素材，spec 里的素材来源改为 generated。默认选全部 icon 节点，但跳过原图长边小于 24px 的（放大后看不出造型，容易画错含义），确实需要时用 ids 指定；一次最多 16 个。会同时传两张参考图：裁出的原始图标网格，以及在完整设计稿上框出图标位置的上下文图。重绘前请先看标注图，用 annotate_design 按图标旁边的文字和用途给图标起好 name（如 calendar、store）和 description，含义才不会画错；没有命名的图标会在返回里列出。立即返回 task_id，约 30 秒到 3 分钟，请接着调用 get_generation 查询结果。",
        annotations(destructive_hint = false, open_world_hint = true)
    )]
    async fn generate_design_icons(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<GenerateDesignIconsParams>) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(&peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        let cfg = match crate::genconfig::load() {
            Ok(c) => c,
            Err(e) => return tool_error(e),
        };
        let (root, spec_file) = match resolve_spec(&ws, &p.spec) {
            Ok(v) => v,
            Err(e) => return tool_error(e),
        };
        let preset = load_preset(&ws, &spec_file).map(|(_, p)| p.spec).unwrap_or_default();
        let quality = p.quality.clone().unwrap_or_else(|| "1K".into());
        let (sf, rt, ids, style, icons_dir) = (spec_file.clone(), root.clone(), p.ids.clone(), p.style.clone(), preset.icons_dir.clone());
        let planned = blocking(move || -> Result<_, String> {
            let spec = imgspec::store::load(&sf).map_err(|e| e.to_string())?;
            let paths = imgspec::store::paths_for_spec(&rt, &sf, &spec, icons_dir.as_deref());
            let plan = imgspec::icons::plan_icon_sheet(&spec, ids.as_deref(), style.as_deref()).map_err(|e| e.to_string())?;
            let img = imgspec::decode_file(&paths.image).map_err(|e| format!("读取原图失败：{e}"))?;
            let sheet = imgspec::icons::reference_sheet(&img, &spec, &plan).map_err(|e| e.to_string())?;
            imgcore::project::write_atomic(&paths.reference, &sheet).map_err(|e| e.to_string())?;
            let context = imgspec::icons::context_sheet(&img, &spec, &plan).map_err(|e| e.to_string())?;
            imgcore::project::write_atomic(&paths.context, &context).map_err(|e| e.to_string())?;
            let stem = paths.image.file_stem().and_then(|s| s.to_str()).unwrap_or("design").to_string();
            Ok((plan, paths.reference, paths.context, paths.dir, stem))
        })
        .await?;
        // 生成的图标网格图也放在这张图的解析文件夹里
        let (plan, ref_path, context_path, out_dir, stem) = match planned {
            Ok(v) => v,
            Err(e) => return tool_error(e),
        };
        let (_, (width, height)) = imggen::sizes::match_ratio(plan.cols, plan.rows, &quality);
        let model = p.model.clone().filter(|m| !m.trim().is_empty()).unwrap_or_else(|| "gpt-image-2".into());
        let n = plan.ids.len();
        let job = GenJob {
            prompt: plan.prompt.clone(),
            model: model.clone(),
            width,
            height,
            count: 1,
            references: vec![ref_path.display().to_string(), context_path.display().to_string()],
            format: OutputFormat::Png,
            remove_background: None,
            final_size: None,
            out_dir,
            name: imgcore::project::sanitize_name(&format!("{stem}-icons-sheet")),
            post: Some(PostProcess::DesignIcons {
                root,
                spec_file: spec_file.clone(),
                plan: plan.clone(),
                icons_dir: preset.icons_dir,
                icon_size: p.size.unwrap_or(preset.icon_size).clamp(16, 2048),
            }),
        };
        let task_id = self.tasks.submit(cfg, job);
        let mut summary = format!("已提交图标重绘任务 {task_id}（{n} 个图标，{} 列 × {} 行，{model} {width}x{height}）。请调用 get_generation 查询，task_id={task_id}", plan.cols, plan.rows);
        if !plan.skipped.is_empty() {
            summary.push_str(&format!("。跳过 {} 个太小的图标：{}", plan.skipped.len(), plan.skipped.iter().map(|s| s.id.as_str()).collect::<Vec<_>>().join("、")));
        }
        if !plan.unnamed.is_empty() {
            summary.push_str(&format!("。注意：{} 还没有 name / description，模型只能看图猜含义，建议先用 annotate_design 标注后再重绘", plan.unnamed.join("、")));
        }
        if !plan.low_res.is_empty() {
            summary.push_str(&format!("。{} 原图太小，重绘结果可能与原意不符", plan.low_res.join("、")));
        }
        ok_json(
            summary,
            serde_json::json!({
                "task_id": task_id,
                "ids": plan.ids,
                "grid": [plan.cols, plan.rows],
                "reference": ws.relative(&ref_path),
                "context_reference": ws.relative(&context_path),
                "skipped": plan.skipped,
                "unnamed": plan.unnamed,
                "low_res": plan.low_res,
                "prompt": plan.prompt,
            }),
        )
    }
}

