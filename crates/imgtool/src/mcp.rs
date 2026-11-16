//! MCP Server（stdio）：让 Agent 在项目上下文中扫描、压缩、转换、裁剪图片

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::schemars::{self, JsonSchema};
use rmcp::service::{MaybeSendFuture, NotificationContext};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, Peer, RoleServer, ServerHandler, ServiceExt};
use serde::Deserialize;
use tokio::sync::RwLock;

use imgcore::project::{scan, ConflictPolicy, OutputMode, ProjectPreset, ScanOptions, PRESET_FILE};
use imgcore::{OutputFormat, ResizeFit};

use crate::batch::{expand_inputs, human_bytes, run_batch};
use crate::gen_tasks::{is_remote, GenJob, Status, TaskStore};
use crate::options::{parse_aspect, resolve, Overrides};
use crate::workspace::{file_uri_to_path, validate_relative_dir, Workspace};

const INSTRUCTIONS: &str = "imgtool：在当前项目内处理和生成图片（压缩、调整分辨率、裁剪、格式转换、去背景、拆分图标、AI 生成）。\n\
- 路径可用相对项目根目录的路径；所有读写都限制在项目根目录内。\n\
- 用户在对话里发来的图片：先用 import_image 存进项目（支持图片地址、base64、本机文件路径），再用返回的路径调用其他工具。\n\
- 生成或重新生成设计稿：generate_image 提交任务（立即返回 task_id），再用 get_generation 查询，它会等待最多 wait_seconds 秒，完成后返回文件路径和预览图。参考图可传项目内路径。提示词请你根据用户的规则自己写清楚。\n\
- 想看某张图的样子：view_image 返回缩略图。\n\
- 先用 get_project_context 了解根目录和项目预设（.imgtool.toml），用 scan_images 找出过大的图片。\n\
- 默认不覆盖原图：压缩写到同目录 name.min.ext，转格式写到 name.<新扩展名>；目标已存在时报错，可传 on_conflict。\n\
- 改动前可传 dry_run=true 预览体积和输出路径。\n\
- 转换格式后，项目代码里对旧文件名的引用需要你自行更新。\n\
- 支持读取 jpeg/png/webp/gif(首帧)/bmp/tiff/ico，输出 jpeg/png/webp/avif；不支持 svg/heic。";

#[derive(Clone)]
pub struct ImgToolServer {
    /// 命令行 --root 指定的根目录；为空时使用客户端 Roots，再退回 cwd
    explicit_roots: Arc<Vec<PathBuf>>,
    cache: Arc<RwLock<Option<Workspace>>>,
    /// 后台图片生成任务
    tasks: TaskStore,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

fn ok_json(summary: impl Into<String>, value: impl serde::Serialize) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![ContentBlock::text(summary), ContentBlock::json(value)?]))
}

fn tool_error(msg: impl Into<String>) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::error(vec![ContentBlock::text(msg)]))
}

/// 在阻塞线程池中执行 CPU 密集任务
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, McpError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| McpError::internal_error(format!("任务执行失败: {e}"), None))
}

impl ImgToolServer {
    pub fn new(explicit_roots: Vec<PathBuf>) -> Self {
        Self {
            explicit_roots: Arc::new(explicit_roots),
            cache: Arc::new(RwLock::new(None)),
            tasks: TaskStore::default(),
            tool_router: Self::tool_router(),
        }
    }

    /// 确定项目根目录：--root > 客户端 Roots > IMGTOOL_ROOT > 当前工作目录
    async fn workspace(&self, peer: &Peer<RoleServer>) -> Result<Workspace, String> {
        if let Some(ws) = self.cache.read().await.clone() {
            return Ok(ws);
        }
        let mut roots: Vec<PathBuf> = self.explicit_roots.to_vec();
        if roots.is_empty() {
            roots = client_roots(peer).await;
        }
        if roots.is_empty() {
            if let Ok(env_root) = std::env::var("IMGTOOL_ROOT") {
                roots.push(PathBuf::from(env_root));
            }
        }
        if roots.is_empty() {
            roots.push(std::env::current_dir().map_err(|e| e.to_string())?);
        }
        let ws = Workspace::new(roots)?;
        tracing::info!(roots = ?ws.roots(), "workspace resolved");
        *self.cache.write().await = Some(ws.clone());
        Ok(ws)
    }
}

#[allow(deprecated)]
async fn client_roots(peer: &Peer<RoleServer>) -> Vec<PathBuf> {
    let supports = peer.peer_info().map(|info| info.capabilities.roots.is_some()).unwrap_or(false);
    if !supports {
        return Vec::new();
    }
    match tokio::time::timeout(Duration::from_secs(5), peer.list_roots()).await {
        Ok(Ok(result)) => result.roots.iter().filter_map(|r| file_uri_to_path(&r.uri)).collect(),
        Ok(Err(e)) => {
            tracing::warn!("roots/list 失败: {e}");
            Vec::new()
        }
        Err(_) => {
            tracing::warn!("roots/list 超时");
            Vec::new()
        }
    }
}

/// 查找离 path 最近的项目预设（不越过所在根目录）
fn load_preset(ws: &Workspace, path: &Path) -> Result<(Option<PathBuf>, ProjectPreset), String> {
    let root = ws.root_of(path).unwrap_or(ws.primary());
    match ProjectPreset::find(path, Some(root)).map_err(|e| e.to_string())? {
        Some((p, preset)) => Ok((Some(p), preset)),
        None => Ok((None, ProjectPreset::default())),
    }
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScanParams {
    /// 扫描目录，相对项目根目录；默认整个项目
    #[serde(default)]
    pub path: Option<String>,
    /// 只返回不小于该体积（KB）的图片
    #[serde(default)]
    pub min_kb: Option<u64>,
    /// 只返回有问题（过大）的图片
    #[serde(default)]
    pub only_flagged: Option<bool>,
    /// 最多返回条数，默认 100
    #[serde(default)]
    pub max_results: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InfoParams {
    /// 图片路径列表
    pub paths: Vec<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct CompressParams {
    /// 图片文件或目录（目录会递归处理，遵守 .gitignore），相对项目根目录
    pub paths: Vec<String>,
    /// 质量 1-100（PNG 低于 95 时为有损调色板压缩）
    #[serde(default)]
    pub quality: Option<u8>,
    /// 每张图片体积上限（KB）。优先于 quality；质量降到下限仍超标会自动缩小分辨率
    #[serde(default)]
    pub max_kb: Option<u64>,
    /// 无损压缩（PNG/WebP）
    #[serde(default)]
    pub lossless: Option<bool>,
    /// 最大宽度（像素），等比缩放、不放大
    #[serde(default)]
    pub max_width: Option<u32>,
    /// 最大高度（像素）
    #[serde(default)]
    pub max_height: Option<u32>,
    /// 同时给出宽高时的适配方式，默认 contain
    #[serde(default)]
    pub fit: Option<ResizeFit>,
    /// 输出格式，默认保持原格式
    #[serde(default)]
    pub format: Option<OutputFormat>,
    /// 输出方式：suffix（默认，同目录新文件）/ overwrite（覆盖原图）/ dir（写到 out_dir）
    #[serde(default)]
    pub output_mode: Option<OutputMode>,
    /// dir 模式的输出目录（项目内相对路径），默认 .imgtool/out
    #[serde(default)]
    pub out_dir: Option<String>,
    /// 目标文件已存在时：error（默认）/ overwrite / rename
    #[serde(default)]
    pub on_conflict: Option<ConflictPolicy>,
    /// 只预览结果，不写文件
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ConvertParams {
    /// 图片文件或目录，相对项目根目录
    pub paths: Vec<String>,
    /// 目标格式
    pub format: OutputFormat,
    /// 质量 1-100，默认使用项目预设或 80
    #[serde(default)]
    pub quality: Option<u8>,
    /// 无损（PNG/WebP）
    #[serde(default)]
    pub lossless: Option<bool>,
    #[serde(default)]
    pub output_mode: Option<OutputMode>,
    #[serde(default)]
    pub out_dir: Option<String>,
    #[serde(default)]
    pub on_conflict: Option<ConflictPolicy>,
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CropParams {
    /// 图片路径
    pub path: String,
    /// 按宽高比居中裁剪，如 "16:9"、"1:1"；与 x/y/width/height 二选一
    #[serde(default)]
    pub aspect: Option<String>,
    /// 裁剪矩形左上角 x（像素，以旋正后的原图为坐标系）
    #[serde(default)]
    pub x: Option<u32>,
    #[serde(default)]
    pub y: Option<u32>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// 裁剪后再缩放到的宽度（像素）
    #[serde(default)]
    pub resize_width: Option<u32>,
    /// 裁剪后再缩放到的高度（像素）
    #[serde(default)]
    pub resize_height: Option<u32>,
    #[serde(default)]
    pub format: Option<OutputFormat>,
    #[serde(default)]
    pub quality: Option<u8>,
    /// 显式输出路径（项目内）；默认同目录 name.min.ext
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub on_conflict: Option<ConflictPolicy>,
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveBgParams {
    /// 图片文件或目录，相对项目根目录
    pub paths: Vec<String>,
    /// 背景色，如 "#ffffff"；不传则自动识别图片边缘颜色
    #[serde(default)]
    pub color: Option<String>,
    /// 容差 0-100，默认 12；背景有轻微渐变/噪点时调大
    #[serde(default)]
    pub tolerance: Option<u8>,
    /// 边缘柔和度 0-100，默认 8
    #[serde(default)]
    pub softness: Option<u8>,
    /// 只去除与边缘相连的背景（默认 true）；false 时全图匹配背景色
    #[serde(default)]
    pub contiguous: Option<bool>,
    /// 裁掉四周透明空白
    #[serde(default)]
    pub trim: Option<bool>,
    /// 裁剪时保留的边距（像素）
    #[serde(default)]
    pub trim_padding: Option<u32>,
    /// 输出格式 png（默认）/ webp / avif；jpeg 不支持透明
    #[serde(default)]
    pub format: Option<OutputFormat>,
    #[serde(default)]
    pub output_mode: Option<OutputMode>,
    #[serde(default)]
    pub out_dir: Option<String>,
    #[serde(default)]
    pub on_conflict: Option<ConflictPolicy>,
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DetectIconsParams {
    /// 包含多个图标的图片（如导航栏、图标合集），相对项目根目录
    pub path: String,
    /// 是否把图标下方的文字标签一起框进去，默认 false（只保留图标）
    #[serde(default)]
    pub include_labels: Option<bool>,
    /// 合并间距（像素）：距离小于它的笔画视为同一个图标；不传则自动。图标被拆碎时调大，相邻图标被合并时调小
    #[serde(default)]
    pub gap: Option<u32>,
    /// 最小尺寸（像素），更小的视为噪点；不传则自动
    #[serde(default)]
    pub min_size: Option<u32>,
    /// 背景色，如 "#ffffff"；不传则自动判断（透明底或边缘颜色）
    #[serde(default)]
    pub background: Option<String>,
    /// 与背景色的容差 0-100，默认 12
    #[serde(default)]
    pub tolerance: Option<u8>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SplitIconsParams {
    #[serde(flatten)]
    pub detect: DetectIconsParams,
    /// 直接使用这些框（像素坐标，可来自 detect_icons 的结果并按需修正）；不传则自动识别
    #[serde(default)]
    pub regions: Option<Vec<imgcore::detect::Region>>,
    /// 按顺序给每个图标命名（不含扩展名），如 ["home","discover"]；建议先看图再起语义化名字
    #[serde(default)]
    pub names: Option<Vec<String>>,
    /// 输出边长（像素），默认 256
    #[serde(default)]
    pub size: Option<u32>,
    /// 四周留白百分比 0-40，默认 10
    #[serde(default)]
    pub padding: Option<u8>,
    /// 输出格式，默认 png
    #[serde(default)]
    pub format: Option<OutputFormat>,
    /// 输出目录（项目内），默认与原图同目录的 <原名>_icons/
    #[serde(default)]
    pub out_dir: Option<String>,
    /// 目标文件已存在时：error（默认）/ overwrite / rename
    #[serde(default)]
    pub on_conflict: Option<ConflictPolicy>,
    #[serde(default)]
    pub dry_run: Option<bool>,
}

fn detect_options(p: &DetectIconsParams) -> Result<imgcore::detect::DetectOptions, String> {
    Ok(imgcore::detect::DetectOptions {
        background: p.background.as_deref().map(crate::options::parse_color).transpose()?,
        tolerance: p.tolerance.unwrap_or(12).min(100),
        gap: p.gap,
        include_labels: p.include_labels.unwrap_or(false),
        min_size: p.min_size,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImportImageParams {
    /// 图片地址（http/https）
    #[serde(default)]
    pub url: Option<String>,
    /// 图片的 base64 数据，可带 data:image/png;base64, 前缀
    #[serde(default)]
    pub base64: Option<String>,
    /// 本机图片文件的绝对路径（例如对话附件保存在项目外的临时目录）
    #[serde(default)]
    pub source_path: Option<String>,
    /// 保存时使用的文件名（扩展名按实际格式自动修正），如 "首页设计稿"
    #[serde(default)]
    pub filename: Option<String>,
    /// 保存目录（项目内相对路径），默认 .imgtool/inbox
    #[serde(default)]
    pub dir: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ViewImageParams {
    /// 图片路径，相对项目根目录
    pub path: String,
    /// 缩略图最长边（像素），默认 512，范围 64-1024
    #[serde(default)]
    pub max_size: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GenerateImageParams {
    /// 提示词：根据用户的需求和规则写清楚主体、构图、风格、配色、文字内容等；最多 5000 字
    pub prompt: String,
    /// 图像模型，默认 gpt-image-2（只支持 1K）；支持 2K/4K 的有 gpt-image-2.5-flare、gpt-image-2.6-flare、nano-banana-2、nano-banana-pro 等
    #[serde(default)]
    pub model: Option<String>,
    /// 画面比例：1:1、5:4、4:3、3:2、16:9、21:9、3:1、4:5、3:4、2:3、9:16；默认按 final_size 自动选择，否则 1:1
    #[serde(default)]
    pub ratio: Option<String>,
    /// 质量档位 1K / 2K / 4K，默认 1K（需模型支持）
    #[serde(default)]
    pub quality: Option<String>,
    /// 直接指定生成尺寸，如 "1024x1024"（优先于 ratio/quality，需是接口支持的尺寸）
    #[serde(default)]
    pub size: Option<String>,
    /// 参考图：项目内图片路径（会自动上传）或 http(s) 地址，最多 16 张
    #[serde(default)]
    pub reference_images: Vec<String>,
    /// 生成张数 1-4，默认 1
    #[serde(default)]
    pub count: Option<u32>,
    /// 生成后在本地精确调整到的尺寸，如 "750x1334"（接口只支持固定几档尺寸）
    #[serde(default)]
    pub final_size: Option<String>,
    /// final_size 的调整方式：cover（默认，裁剪填满）/ contain（等比缩放不超过）/ fill（拉伸）
    #[serde(default)]
    pub fit: Option<ResizeFit>,
    /// 生成后在本地去除纯色背景，输出透明图（提示词里写明"纯白背景"效果最好）
    #[serde(default)]
    pub remove_background: Option<bool>,
    /// 保存格式，默认 png
    #[serde(default)]
    pub format: Option<OutputFormat>,
    /// 输出目录（项目内相对路径），默认 .imgtool/generated
    #[serde(default)]
    pub out_dir: Option<String>,
    /// 文件名前缀，默认 gen
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetGenerationParams {
    /// generate_image 返回的任务编号
    pub task_id: String,
    /// 最多等待秒数（任务结束会提前返回），默认 30，范围 0-55
    #[serde(default)]
    pub wait_seconds: Option<u64>,
    /// 完成时附带结果缩略图，默认 true
    #[serde(default)]
    pub preview: Option<bool>,
}

/// 生成缩略图内容块
fn preview_block(path: &Path, max_edge: u32) -> Result<ContentBlock, String> {
    use base64::Engine;
    let data = std::fs::read(path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    let (thumb, mime) = imgcore::postprocess::thumbnail(&data, max_edge)?;
    Ok(ContentBlock::image(base64::engine::general_purpose::STANDARD.encode(thumb), mime))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InitPresetParams {
    /// 已存在时是否覆盖，默认 false
    #[serde(default)]
    pub overwrite: Option<bool>,
}
impl ImgToolServer {
    async fn process_paths(
        &self,
        peer: &Peer<RoleServer>,
        paths: Vec<String>,
        overrides: Overrides,
        dry_run: bool,
        explicit_output: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        if paths.is_empty() {
            return tool_error("paths 不能为空");
        }
        let mut inputs = Vec::with_capacity(paths.len());
        for p in &paths {
            match ws.resolve_existing(p) {
                Ok(abs) => inputs.push(abs),
                Err(e) => return tool_error(e),
            }
        }
        let (_, preset) = match load_preset(&ws, &inputs[0]) {
            Ok(v) => v,
            Err(e) => return tool_error(e),
        };
        let resolved = match resolve(&preset, &overrides) {
            Ok(r) => r,
            Err(e) => return tool_error(e),
        };
        if resolved.policy.mode == OutputMode::Dir {
            if let Err(e) = validate_relative_dir(&resolved.policy.dir) {
                return tool_error(e);
            }
        }
        let explicit = match explicit_output.map(|o| ws.resolve_output(&o)).transpose() {
            Ok(v) => v,
            Err(e) => return tool_error(e),
        };

        let exclude = preset.scan.exclude.clone();
        let suffix = resolved.policy.suffix.clone();
        let result = blocking(move || {
            let files = expand_inputs(&inputs, &exclude, &suffix)?;
            if files.is_empty() {
                return Err("没有找到可处理的图片（支持 jpeg/png/webp/gif/bmp/tiff/ico）".to_string());
            }
            let root = ws.root_of(&files[0].path).unwrap_or(ws.primary()).to_path_buf();
            Ok(run_batch(files, &resolved, Some(&root), dry_run, explicit, &|p| ws.relative(p)))
        })
        .await?;
        match result {
            Ok(summary) => ok_json(summary.text_summary(), summary),
            Err(e) => tool_error(e),
        }
    }
}

#[tool_router]
impl ImgToolServer {
    #[tool(
        description = "获取项目上下文：项目根目录、项目预设 .imgtool.toml（默认质量/格式/尺寸、输出策略、扫描阈值）以及支持的格式。处理图片前先调用。",
        annotations(read_only_hint = true)
    )]
    async fn get_project_context(&self, peer: Peer<RoleServer>) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(&peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        let (preset_path, preset) = match load_preset(&ws, ws.primary()) {
            Ok(v) => v,
            Err(e) => return tool_error(e),
        };
        let value = serde_json::json!({
            "roots": ws.roots().iter().map(|r| r.display().to_string()).collect::<Vec<_>>(),
            "preset_file": preset_path.map(|p| ws.relative(&p)),
            "preset": preset,
            "input_formats": ["jpeg", "png", "webp", "gif(首帧)", "bmp", "tiff", "ico"],
            "output_formats": OutputFormat::ALL,
            // 不返回 Key 本身
            "generation": match crate::genconfig::load() {
                Ok(c) => serde_json::json!({ "configured": true, "base_url": c.base_url, "source": c.source, "upload_local_references": c.upload_refs }),
                Err(e) => serde_json::json!({ "configured": false, "hint": e }),
            },
            "output_naming": {
                "suffix": format!("压缩: name{}.ext；转格式: name.<新扩展名>", preset.output.suffix),
                "overwrite": "覆盖原文件（格式变化时写新扩展名，原文件保留）",
                "dir": format!("写到 {}/ 下并保持相对路径", preset.output.dir),
            },
        });
        let summary = format!(
            "项目根目录: {}；预设: {}",
            ws.primary().display(),
            if value["preset_file"].is_null() { format!("未找到 {PRESET_FILE}，使用内置默认值") } else { value["preset_file"].as_str().unwrap_or_default().to_string() }
        );
        ok_json(summary, value)
    }

    #[tool(
        description = "扫描项目中的图片（遵守 .gitignore），按体积从大到小列出路径、格式、尺寸，并标记体积/尺寸超过预设阈值的图片。",
        annotations(read_only_hint = true)
    )]
    async fn scan_images(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<ScanParams>) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(&peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        let dir = match ws.resolve_existing(p.path.as_deref().unwrap_or(".")) {
            Ok(d) if d.is_dir() => d,
            Ok(_) => return tool_error("path 必须是目录"),
            Err(e) => return tool_error(e),
        };
        let (_, preset) = match load_preset(&ws, &dir) {
            Ok(v) => v,
            Err(e) => return tool_error(e),
        };
        let opts = ScanOptions {
            policy: preset.scan.clone(),
            min_bytes: p.min_kb.unwrap_or(0) * 1024,
            max_results: p.max_results.unwrap_or(100).clamp(1, 1000),
            only_flagged: p.only_flagged.unwrap_or(false),
        };
        let ws2 = ws.clone();
        let report = blocking(move || {
            scan(&dir, &opts).map(|mut r| {
                // 统一成相对项目根目录的路径，便于 Agent 直接传给其他工具
                for e in &mut r.images {
                    e.path = ws2.relative(&dir.join(&e.path));
                }
                r.root = ws2.relative(&dir);
                r
            })
        })
        .await?;
        match report {
            Ok(r) => ok_json(
                format!(
                    "共 {} 张图片（{}），{} 张需要处理（{}）",
                    r.total_images,
                    human_bytes(r.total_bytes),
                    r.flagged_images,
                    human_bytes(r.flagged_bytes)
                ),
                r,
            ),
            Err(e) => tool_error(e.to_string()),
        }
    }

    #[tool(description = "读取图片基础信息：格式、宽高、体积、能否处理。", annotations(read_only_hint = true))]
    async fn image_info(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<InfoParams>) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(&peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        let mut out = Vec::new();
        for path in &p.paths {
            let entry = match ws.resolve_existing(path) {
                Ok(abs) => match imgcore::probe_file(&abs) {
                    Ok(info) => serde_json::json!({ "path": ws.relative(&abs), "info": info }),
                    Err(e) => serde_json::json!({ "path": path, "error": e.to_string() }),
                },
                Err(e) => serde_json::json!({ "path": path, "error": e }),
            };
            out.push(entry);
        }
        ok_json(format!("{} 个文件", out.len()), out)
    }

    #[tool(
        description = "压缩图片（可同时限制分辨率、转换格式）。支持按质量、按目标体积 max_kb、无损三种方式；未指定的参数取项目预设。默认写到同目录 name.min.ext，不覆盖原图。可传目录批量处理（单次最多 500 张）。",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    async fn compress_images(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<CompressParams>) -> Result<CallToolResult, McpError> {
        let overrides = Overrides {
            quality: p.quality,
            max_kb: p.max_kb,
            lossless: p.lossless.unwrap_or(false),
            max_width: p.max_width,
            max_height: p.max_height,
            fit: p.fit,
            format: p.format,
            crop: None,
            remove_background: None,
            output_mode: p.output_mode,
            out_dir: p.out_dir,
            on_conflict: p.on_conflict,
        };
        self.process_paths(&peer, p.paths, overrides, p.dry_run.unwrap_or(false), None).await
    }

    #[tool(
        description = "转换图片格式（jpeg/png/webp/avif）。默认写到同目录 name.<新扩展名>，原图保留。转换后请自行更新代码中对旧文件名的引用。",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    async fn convert_images(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<ConvertParams>) -> Result<CallToolResult, McpError> {
        let overrides = Overrides {
            quality: p.quality,
            lossless: p.lossless.unwrap_or(false),
            format: Some(p.format),
            output_mode: p.output_mode,
            out_dir: p.out_dir,
            on_conflict: p.on_conflict,
            ..Default::default()
        };
        self.process_paths(&peer, p.paths, overrides, p.dry_run.unwrap_or(false), None).await
    }

    #[tool(
        description = "裁剪单张图片：按宽高比居中裁剪（aspect，如 16:9）或按像素矩形（x/y/width/height），可再缩放到 resize_width/resize_height。默认写到同目录 name.min.ext。",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    async fn crop_image(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<CropParams>) -> Result<CallToolResult, McpError> {
        let crop = match (&p.aspect, p.x, p.y, p.width, p.height) {
            (Some(a), None, None, None, None) => match parse_aspect(a) {
                Ok(c) => c,
                Err(e) => return tool_error(e),
            },
            (None, x, y, Some(w), Some(h)) if w > 0 && h > 0 => {
                imgcore::CropSpec::Rect { x: x.unwrap_or(0), y: y.unwrap_or(0), width: w, height: h }
            }
            _ => return tool_error("请提供 aspect，或提供 width/height（x/y 可选，默认 0），两者不能同时使用"),
        };
        let overrides = Overrides {
            quality: p.quality,
            max_width: p.resize_width,
            max_height: p.resize_height,
            // 同时给出宽高时精确输出该尺寸
            fit: (p.resize_width.is_some() && p.resize_height.is_some()).then_some(ResizeFit::Cover),
            format: p.format,
            crop: Some(crop),
            on_conflict: p.on_conflict,
            ..Default::default()
        };
        self.process_paths(&peer, vec![p.path], overrides, p.dry_run.unwrap_or(false), p.output).await
    }

    #[tool(
        description = "去除纯色/近纯色背景，输出透明 PNG（适合白底产品图、图标、AI 生成的纯色背景图）。自动识别边缘背景色，或用 color 指定。复杂背景效果有限，结果里会给出警告。默认写到同目录 name.min.png。",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    async fn remove_background(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<RemoveBgParams>) -> Result<CallToolResult, McpError> {
        if p.format == Some(OutputFormat::Jpeg) {
            return tool_error("JPEG 不支持透明，请使用 png / webp / avif");
        }
        let color = match p.color.as_deref().map(crate::options::parse_color).transpose() {
            Ok(c) => c,
            Err(e) => return tool_error(e),
        };
        let mut removal = imgcore::BackgroundRemoval {
            color,
            contiguous: p.contiguous.unwrap_or(true),
            trim: p.trim.unwrap_or(false),
            trim_padding: p.trim_padding.unwrap_or(0),
            ..Default::default()
        };
        if let Some(t) = p.tolerance {
            removal.tolerance = t.min(100);
        }
        if let Some(s) = p.softness {
            removal.softness = s.min(100);
        }
        let overrides = Overrides {
            lossless: true,
            format: Some(p.format.unwrap_or(OutputFormat::Png)),
            remove_background: Some(removal),
            output_mode: p.output_mode,
            out_dir: p.out_dir,
            on_conflict: p.on_conflict,
            ..Default::default()
        };
        self.process_paths(&peer, p.paths, overrides, p.dry_run.unwrap_or(false), None).await
    }

    #[tool(
        description = "智能识别一张图里的多个图标（底部导航栏、图标合集等），返回每个图标的像素框（x,y,width,height）。默认只框图标、去掉下方的文字标签（include_labels=true 时一起框进去）。适合透明底或纯色底、图标之间有空隙的图。只读，不写文件。",
        annotations(read_only_hint = true)
    )]
    async fn detect_icons(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<DetectIconsParams>) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(&peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        let path = match ws.resolve_existing(&p.path) {
            Ok(v) => v,
            Err(e) => return tool_error(e),
        };
        let opts = match detect_options(&p) {
            Ok(o) => o,
            Err(e) => return tool_error(e),
        };
        let res = blocking(move || -> Result<_, String> {
            let data = std::fs::read(&path).map_err(|e| e.to_string())?;
            let img = imgcore::decode::decode(&data).map_err(|e| e.to_string())?.image;
            Ok((img.width(), img.height(), imgcore::detect::detect_regions(&img, &opts)))
        })
        .await?;
        match res {
            Ok((w, h, r)) => ok_json(
                format!("在 {w}x{h} 的图中识别到 {} 个图标（合并间距 {}px，方式 {}）", r.regions.len(), r.gap, r.mode),
                serde_json::json!({ "image_width": w, "image_height": h, "result": r }),
            ),
            Err(e) => tool_error(e),
        }
    }

    #[tool(
        description = "智能裁剪：把图中的多个图标逐个导出为正方形图片（图标居中、等比缩放）。默认自动识别；也可传 regions 使用自己修正过的框，传 names 按顺序命名（如 home、discover）。默认输出到原图同目录的 <原名>_icons/。",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    async fn split_icons(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<SplitIconsParams>) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(&peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        let input = match ws.resolve_existing(&p.detect.path) {
            Ok(v) => v,
            Err(e) => return tool_error(e),
        };
        let out_dir = match p.out_dir.as_deref().map(|d| ws.resolve_output(d)).transpose() {
            Ok(v) => v,
            Err(e) => return tool_error(e),
        };
        let detect = match detect_options(&p.detect) {
            Ok(o) => o,
            Err(e) => return tool_error(e),
        };
        let job = imgcore::project::SplitJob {
            input,
            regions: p.regions,
            detect,
            square: imgcore::detect::SquareOptions { size: p.size.unwrap_or(256).clamp(8, 4096), padding: p.padding.unwrap_or(10).min(40), background: None },
            names: p.names.unwrap_or_default(),
            out_dir,
            format: p.format.unwrap_or(OutputFormat::Png),
            on_conflict: p.on_conflict.unwrap_or(ConflictPolicy::Error),
            dry_run: p.dry_run.unwrap_or(false),
        };
        let ws2 = ws.clone();
        let res = blocking(move || imgcore::project::split_file(&job).map_err(|e| e.to_string())).await?;
        match res {
            Ok(mut r) => {
                for f in &mut r.files {
                    f.path = ws2.relative(Path::new(&f.path));
                }
                r.out_dir = ws2.relative(Path::new(&r.out_dir));
                r.input = ws2.relative(Path::new(&r.input));
                let verb = if r.files.iter().any(|f| f.written) { "已导出" } else { "预计导出" };
                ok_json(format!("{verb} {} 个 {}×{} 图标到 {}", r.files.len(), r.size, r.size, r.out_dir), r)
            }
            Err(e) => tool_error(e),
        }
    }

    #[tool(
        description = "把用户在对话里发来的图片存进项目，返回项目内路径供其他工具使用。三选一：url（图片地址）、base64（图片数据，可带 data URL 前缀）、source_path（本机文件绝对路径，可在项目外）。默认存到 .imgtool/inbox/，按实际格式命名，重名自动加序号。",
        annotations(destructive_hint = false)
    )]
    async fn import_image(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<ImportImageParams>) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(&peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        let sources = [p.url.is_some(), p.base64.is_some(), p.source_path.is_some()].iter().filter(|b| **b).count();
        if sources != 1 {
            return tool_error("请只提供 url、base64、source_path 中的一个");
        }
        let dir = match ws.resolve_output(p.dir.as_deref().unwrap_or(crate::intake::INBOX_DIR)) {
            Ok(d) => d,
            Err(e) => return tool_error(e),
        };
        let (data, hint) = if let Some(url) = &p.url {
            match imggen::fetch::download_image(url).await {
                Ok((d, _)) => {
                    let from_url = url.split('?').next().and_then(|u| u.rsplit('/').next()).filter(|s| !s.is_empty()).map(str::to_string);
                    (d, p.filename.clone().or(from_url))
                }
                Err(e) => return tool_error(e.to_string()),
            }
        } else if let Some(b64) = &p.base64 {
            match crate::intake::decode_base64(b64) {
                Ok(d) => (d, p.filename.clone()),
                Err(e) => return tool_error(e),
            }
        } else {
            let src = PathBuf::from(p.source_path.as_deref().unwrap_or_default());
            if !src.is_absolute() {
                return tool_error("source_path 需要是绝对路径；项目内的图片可以直接用相对路径调用其他工具，无需导入");
            }
            match std::fs::metadata(&src) {
                Ok(m) if m.len() as usize > crate::intake::MAX_IMPORT_BYTES => return tool_error("图片超过 30MB"),
                Ok(m) if m.is_file() => {}
                _ => return tool_error(format!("文件不存在: {}", src.display())),
            }
            match std::fs::read(&src) {
                Ok(d) => (d, p.filename.clone().or_else(|| src.file_name().map(|n| n.to_string_lossy().to_string()))),
                Err(e) => return tool_error(format!("读取失败: {e}")),
            }
        };
        let res = blocking(move || crate::intake::save_image(&dir, hint.as_deref(), &data)).await?;
        match res {
            Ok((path, info)) => {
                let rel = ws.relative(&path);
                ok_json(
                    format!("已保存到 {rel}（{} {}×{}，{}）", info.format, info.width.unwrap_or(0), info.height.unwrap_or(0), human_bytes(info.bytes)),
                    serde_json::json!({ "path": rel, "info": info }),
                )
            }
            Err(e) => tool_error(e),
        }
    }

    #[tool(
        description = "查看项目内的一张图片：返回缩略图（让你直接看到图片内容）和基础信息。用于检查生成、裁剪、去背景的结果。",
        annotations(read_only_hint = true)
    )]
    async fn view_image(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<ViewImageParams>) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(&peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        let path = match ws.resolve_existing(&p.path) {
            Ok(v) => v,
            Err(e) => return tool_error(e),
        };
        let max = p.max_size.unwrap_or(512).clamp(64, 1024);
        let rel = ws.relative(&path);
        let res = blocking(move || -> Result<_, String> {
            let info = imgcore::probe_file(&path).map_err(|e| e.to_string())?;
            Ok((info, preview_block(&path, max)?))
        })
        .await?;
        match res {
            Ok((info, img)) => Ok(CallToolResult::success(vec![
                ContentBlock::text(format!("{rel}：{} {}×{}，{}", info.format, info.width.unwrap_or(0), info.height.unwrap_or(0), human_bytes(info.bytes))),
                img,
            ])),
            Err(e) => tool_error(e),
        }
    }

    #[tool(
        description = "提交 AI 图片生成任务（重新生成设计稿、按规则出图、图生图），立即返回 task_id；生成一张约 30 秒到 3 分钟，请接着调用 get_generation 查询结果。可带参考图（项目内路径或地址）、生成后精确调整到 final_size、生成后去除背景。接口和 API Key 读取桌面端「设置」或环境变量 IMGTOOL_API_KEY / IMGTOOL_API_BASE_URL。每次生成会消耗接口额度。",
        annotations(destructive_hint = false, open_world_hint = true)
    )]
    async fn generate_image(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<GenerateImageParams>) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(&peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        let cfg = match crate::genconfig::load() {
            Ok(c) => c,
            Err(e) => return tool_error(e),
        };
        let prompt = p.prompt.trim().to_string();
        if prompt.is_empty() {
            return tool_error("prompt 不能为空");
        }
        if prompt.chars().count() > imggen::prompt::PROMPT_MAX_CHARS {
            return tool_error("prompt 最多 5000 字");
        }
        let final_size = match p.final_size.as_deref().map(imggen::sizes::parse_size).transpose() {
            Ok(v) => v.map(|(w, h)| imgcore::postprocess::FinalSize { width: w, height: h, fit: p.fit.unwrap_or(ResizeFit::Cover) }),
            Err(e) => return tool_error(e),
        };
        let quality = p.quality.clone().unwrap_or_else(|| "1K".into());
        let size = match (&p.size, &p.ratio, &final_size) {
            (Some(s), _, _) => imggen::sizes::parse_size(s),
            (None, Some(r), _) => imggen::sizes::size_for(r, &quality),
            (None, None, Some(f)) => Ok(imggen::sizes::match_ratio(f.width, f.height, &quality).1),
            (None, None, None) => imggen::sizes::size_for("1:1", &quality),
        };
        let (width, height) = match size {
            Ok(v) => v,
            Err(e) => return tool_error(e),
        };
        if p.reference_images.len() > 16 {
            return tool_error("参考图最多 16 张");
        }
        let mut references = Vec::new();
        for r in &p.reference_images {
            if is_remote(r) {
                references.push(r.clone());
            } else {
                match ws.resolve_existing(r) {
                    Ok(abs) if abs.is_file() => references.push(abs.display().to_string()),
                    Ok(_) => return tool_error(format!("参考图不是文件: {r}")),
                    Err(e) => return tool_error(format!("{e}（对话里的图片请先用 import_image 存进项目）")),
                }
            }
        }
        let out_dir = match ws.resolve_output(p.out_dir.as_deref().unwrap_or(".imgtool/generated")) {
            Ok(d) => d,
            Err(e) => return tool_error(e),
        };
        let format = p.format.unwrap_or(OutputFormat::Png);
        let remove_background = p.remove_background.unwrap_or(false).then(imgcore::BackgroundRemoval::default);
        let model = p.model.clone().filter(|m| !m.trim().is_empty()).unwrap_or_else(|| "gpt-image-2".into());
        let count = p.count.unwrap_or(1).clamp(1, 4);
        let job = GenJob {
            prompt,
            model: model.clone(),
            width,
            height,
            count,
            references,
            format,
            remove_background,
            final_size,
            out_dir: out_dir.clone(),
            name: imgcore::project::sanitize_name(p.name.as_deref().unwrap_or("gen")),
        };
        let task_id = self.tasks.submit(cfg, job);
        ok_json(
            format!(
                "已提交生成任务 {task_id}（{model}，{width}x{height}{}，{count} 张）。请调用 get_generation 查询，task_id={task_id}",
                final_size.map(|f| format!(" → 调整为 {}x{}", f.width, f.height)).unwrap_or_default()
            ),
            serde_json::json!({ "task_id": task_id, "model": model, "size": format!("{width}x{height}"), "count": count, "out_dir": ws.relative(&out_dir) }),
        )
    }

    #[tool(
        description = "查询 generate_image 提交的生成任务。会等待最多 wait_seconds 秒（默认 30），任务结束时立即返回；status 为 running 时请再次调用。完成后返回生成文件的项目内路径、尺寸，并附缩略图。",
        annotations(read_only_hint = true)
    )]
    async fn get_generation(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<GetGenerationParams>) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(&peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        let wait = Duration::from_secs(p.wait_seconds.unwrap_or(30).min(55));
        let Some(mut state) = self.tasks.wait(p.task_id.trim(), wait).await else {
            return tool_error(format!("找不到任务 {}（MCP 重启后任务记录会丢失，已生成的文件在 .imgtool/generated/）", p.task_id));
        };
        for f in &mut state.files {
            f.path = ws.relative(Path::new(&f.path));
        }
        let summary = match state.status {
            Status::Running => format!("任务 {} 仍在进行：{}，已用 {} 秒。请再次调用 get_generation", state.task_id, state.stage, state.elapsed_secs),
            Status::Completed => format!("任务完成，用时 {} 秒，生成 {} 张：{}", state.elapsed_secs, state.files.len(), state.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>().join("、")),
            Status::Partial => format!("部分完成：成功 {} 张，失败 {} 张：{}", state.files.len(), state.errors.len(), state.errors.join("；")),
            Status::Failed => format!("任务失败：{}", state.errors.join("；")),
        };
        let mut blocks = vec![ContentBlock::text(summary), ContentBlock::json(&state)?];
        if p.preview.unwrap_or(true) && state.status != Status::Running {
            let paths: Vec<PathBuf> = state.files.iter().take(4).filter_map(|f| ws.resolve_existing(&f.path).ok()).collect();
            let previews = blocking(move || paths.iter().filter_map(|p| preview_block(p, 384).ok()).collect::<Vec<_>>()).await?;
            blocks.extend(previews);
        }
        let failed = state.status == Status::Failed;
        Ok(if failed { CallToolResult::error(blocks) } else { CallToolResult::success(blocks) })
    }

    #[tool(
        description = "在项目根目录创建带注释的预设文件 .imgtool.toml（默认质量、格式、尺寸上限、输出策略、扫描阈值），桌面端与 Agent 共用。",
        annotations(destructive_hint = false)
    )]
    async fn init_project_preset(&self, peer: Peer<RoleServer>, Parameters(p): Parameters<InitPresetParams>) -> Result<CallToolResult, McpError> {
        let ws = match self.workspace(&peer).await {
            Ok(ws) => ws,
            Err(e) => return tool_error(e),
        };
        let target = ws.primary().join(PRESET_FILE);
        if target.exists() && !p.overwrite.unwrap_or(false) {
            return tool_error(format!("{PRESET_FILE} 已存在；如需覆盖请传 overwrite=true"));
        }
        if let Err(e) = std::fs::write(&target, ProjectPreset::template()) {
            return tool_error(format!("写入失败: {e}"));
        }
        ok_json(format!("已创建 {}", target.display()), serde_json::json!({ "path": PRESET_FILE, "content": ProjectPreset::template() }))
    }
}

#[tool_handler]
impl ServerHandler for ImgToolServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("imgtool", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    fn on_roots_list_changed(&self, _context: NotificationContext<RoleServer>) -> impl std::future::Future<Output = ()> + MaybeSendFuture + '_ {
        let cache = self.cache.clone();
        async move {
            tracing::info!("roots changed, clearing workspace cache");
            *cache.write().await = None;
        }
    }
}

/// 以 stdio 方式运行 MCP Server，直到客户端断开
pub async fn serve_stdio(roots: Vec<PathBuf>) -> anyhow::Result<()> {
    let server = ImgToolServer::new(roots);
    let running = server.serve(rmcp::transport::stdio()).await?;
    running.waiting().await?;
    Ok(())
}
