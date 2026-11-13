use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use imgcore::project::{
    is_image_path, run_file_job, walk_images, ConflictPolicy, FileJob, OutputMode, OutputPolicy, ProjectPreset,
};
use imgcore::{ImageInfo, ProcessOptions, ProcessReport};

use crate::mcp_config::{self, Client, ClientStatus};

static PREVIEW_SEQ: AtomicU64 = AtomicU64::new(0);

fn previews_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("previews");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// 启动时清理上次遗留的预览文件
pub fn clean_previews(app: &AppHandle) {
    if let Ok(dir) = previews_dir(app) {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct LoadedImage {
    pub path: String,
    pub name: String,
    pub info: ImageInfo,
    /// WebView 可直接显示的路径（原图或转换后的 PNG）
    pub display_path: String,
}

/// 展开拖入的文件/目录，返回图片文件列表
#[tauri::command]
pub async fn expand_paths(paths: Vec<String>) -> Result<Vec<String>, String> {
    blocking(move || {
        let mut out = Vec::new();
        for p in paths {
            let path = PathBuf::from(&p);
            if path.is_dir() {
                let files = walk_images(&path, &[]).map_err(|e| e.to_string())?;
                out.extend(files.into_iter().map(|f| f.display().to_string()));
            } else if is_image_path(&path) {
                out.push(p);
            }
        }
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn load_images(app: AppHandle, paths: Vec<String>) -> Result<Vec<Result<LoadedImage, String>>, String> {
    let dir = previews_dir(&app)?;
    blocking(move || {
        Ok(paths
            .par_iter()
            .map(|p| {
                let path = Path::new(p);
                let info = imgcore::probe_file(path).map_err(|e| format!("{p}: {e}"))?;
                if !info.decodable {
                    return Err(format!("{}: 暂不支持 {} 格式", file_name(path), info.format));
                }
                // TIFF 等 WebView 不一定能显示的格式，先转 PNG 用于预览
                let display_path = if matches!(info.format.as_str(), "tiff" | "ico") {
                    let out = imgcore::process_file(
                        path,
                        &ProcessOptions {
                            format: Some(imgcore::OutputFormat::Png),
                            mode: imgcore::CompressMode::Quality { quality: 100 },
                            ..Default::default()
                        },
                    )
                    .map_err(|e| e.to_string())?;
                    let target = dir.join(format!("orig-{}.png", PREVIEW_SEQ.fetch_add(1, Ordering::Relaxed)));
                    std::fs::write(&target, out.data).map_err(|e| e.to_string())?;
                    target.display().to_string()
                } else {
                    p.clone()
                };
                Ok(LoadedImage { path: p.clone(), name: file_name(path), info, display_path })
            })
            .collect())
    })
    .await
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

#[derive(Serialize)]
pub struct PreviewResult {
    pub output_path: String,
    pub report: ProcessReport,
}

/// 按当前参数处理一张图，结果写到缓存目录供对比预览
#[tauri::command]
pub async fn preview(app: AppHandle, path: String, options: ProcessOptions, previous: Option<String>) -> Result<PreviewResult, String> {
    let dir = previews_dir(&app)?;
    blocking(move || {
        let out = imgcore::process_file(Path::new(&path), &options).map_err(|e| e.to_string())?;
        let seq = PREVIEW_SEQ.fetch_add(1, Ordering::Relaxed);
        let target = dir.join(format!("preview-{seq}.{}", out.report.format.extension()));
        std::fs::write(&target, &out.data).map_err(|e| e.to_string())?;
        // 只删除缓存目录内的旧预览
        if let Some(prev) = previous {
            let prev = PathBuf::from(prev);
            if prev.starts_with(&dir) {
                let _ = std::fs::remove_file(prev);
            }
        }
        Ok(PreviewResult { output_path: target.display().to_string(), report: out.report })
    })
    .await
}

#[derive(Deserialize)]
pub struct ExportItem {
    pub path: String,
    pub options: ProcessOptions,
}

#[derive(Deserialize)]
pub struct ExportTarget {
    /// suffix：原图同目录；dir：指定目录
    pub mode: OutputMode,
    pub dir: Option<String>,
    pub suffix: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct ExportProgress {
    pub path: String,
    pub ok: bool,
    pub output: Option<String>,
    pub report: Option<ProcessReport>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct ExportSummary {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub original_bytes: u64,
    pub output_bytes: u64,
}

#[tauri::command]
pub async fn export_images(app: AppHandle, items: Vec<ExportItem>, target: ExportTarget) -> Result<ExportSummary, String> {
    let mut policy = OutputPolicy { mode: target.mode, on_conflict: ConflictPolicy::Rename, ..Default::default() };
    match target.mode {
        OutputMode::Dir => {
            let dir = target.dir.ok_or("请选择输出目录")?;
            if !Path::new(&dir).is_absolute() {
                return Err("输出目录必须是绝对路径".into());
            }
            policy.dir = dir;
        }
        OutputMode::Suffix => {
            if let Some(s) = target.suffix.filter(|s| !s.is_empty()) {
                policy.suffix = s;
            }
        }
        OutputMode::Overwrite => return Err("桌面端不支持覆盖原图".into()),
    }
    let write_when_unchanged = target.mode == OutputMode::Dir;
    let app2 = app.clone();
    blocking(move || {
        let results: Vec<ExportProgress> = items
            .par_iter()
            .map(|item| {
                let job = FileJob {
                    input: PathBuf::from(&item.path),
                    options: item.options.clone(),
                    output: None,
                    policy: policy.clone(),
                    on_conflict: ConflictPolicy::Rename,
                    project_root: None,
                    dry_run: false,
                    write_when_unchanged,
                };
                let progress = match run_file_job(&job) {
                    Ok(r) => ExportProgress {
                        path: item.path.clone(),
                        ok: true,
                        output: r.written.then_some(r.output).flatten(),
                        report: Some(r.report),
                        error: None,
                    },
                    Err(e) => ExportProgress { path: item.path.clone(), ok: false, output: None, report: None, error: Some(e.to_string()) },
                };
                let _ = app2.emit("export-progress", progress.clone());
                progress
            })
            .collect();
        let ok: Vec<_> = results.iter().filter_map(|r| r.report.as_ref()).collect();
        Ok(ExportSummary {
            total: results.len(),
            succeeded: ok.len(),
            failed: results.len() - ok.len(),
            original_bytes: ok.iter().map(|r| r.original_bytes).sum(),
            output_bytes: ok.iter().map(|r| r.output_bytes).sum(),
        })
    })
    .await
}

#[derive(Serialize)]
pub struct FoundPreset {
    pub path: String,
    pub preset: ProjectPreset,
    pub options: ProcessOptions,
}

/// 查找图片所在项目的 .imgtool.toml（与 MCP 共用同一份预设）
#[tauri::command]
pub async fn find_preset(path: String) -> Result<Option<FoundPreset>, String> {
    blocking(move || {
        Ok(ProjectPreset::find(Path::new(&path), None).map_err(|e| e.to_string())?.map(|(p, preset)| FoundPreset {
            path: p.display().to_string(),
            options: preset.defaults.to_options(),
            preset,
        }))
    })
    .await
}

/// 启动参数中的文件/目录（支持 `imgtool-desktop a.png dir/` 或系统"打开方式"）
#[tauri::command]
pub fn initial_paths() -> Vec<String> {
    std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-') && Path::new(a).exists())
        .collect()
}

fn sidecar_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = exe
        .parent()
        .ok_or("无法定位程序目录")?
        .join(format!("imgtool{}", std::env::consts::EXE_SUFFIX));
    if path.exists() {
        Ok(path)
    } else {
        Err(format!("未找到 imgtool：{}（开发环境请先运行 pnpm sidecar）", path.display()))
    }
}

#[derive(Serialize)]
pub struct McpStatus {
    pub command: Option<String>,
    pub error: Option<String>,
    pub clients: Vec<ClientStatus>,
    pub snippet: Option<String>,
}

#[tauri::command]
pub fn mcp_status(app: AppHandle) -> Result<McpStatus, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    match sidecar_path() {
        Ok(p) => {
            let cmd = p.display().to_string();
            Ok(McpStatus { clients: mcp_config::status(&home, &cmd), snippet: Some(mcp_config::snippet(&cmd)), command: Some(cmd), error: None })
        }
        Err(e) => Ok(McpStatus { command: None, error: Some(e), clients: mcp_config::status(&home, ""), snippet: None }),
    }
}

#[tauri::command]
pub fn mcp_install(app: AppHandle, client: Client) -> Result<String, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let cmd = sidecar_path()?;
    mcp_config::install(&home, client, &cmd.display().to_string()).map(|p| p.display().to_string())
}
