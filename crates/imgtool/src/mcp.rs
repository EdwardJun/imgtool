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
use crate::options::{parse_aspect, resolve, Overrides};
use crate::workspace::{file_uri_to_path, validate_relative_dir, Workspace};

const INSTRUCTIONS: &str = "imgtool：在当前项目内处理图片（压缩、调整分辨率、裁剪、格式转换）。\n\
- 路径可用相对项目根目录的路径；所有读写都限制在项目根目录内。\n\
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
            let root = ws.root_of(&files[0]).unwrap_or(ws.primary()).to_path_buf();
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
