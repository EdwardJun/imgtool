//! AI 人像抠图模型（MODNet）：查询状态、下载。模型与 MCP 共用同一个位置

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use imggen::models::{self, MODNET};

static DOWNLOADING: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
pub struct ModelStatus {
    pub available: bool,
    pub path: Option<String>,
    pub bytes: u64,
    pub license: &'static str,
}

#[derive(Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: u64,
}

/// 启动时注册已下载的模型
pub fn register_downloaded() {
    if models::is_downloaded(&MODNET) {
        if let Some(p) = models::model_path(&MODNET) {
            imgcore::matting::set_model_path(p);
        }
    }
}

fn status() -> ModelStatus {
    let path = imgcore::matting::model_path().or_else(|| models::is_downloaded(&MODNET).then(|| models::model_path(&MODNET)).flatten());
    ModelStatus { available: path.is_some(), path: path.map(|p| p.display().to_string()), bytes: MODNET.bytes, license: MODNET.license }
}

#[tauri::command]
pub fn matting_model_status() -> ModelStatus {
    register_downloaded();
    status()
}

/// 下载模型，进度通过 model-download-progress 事件推送
#[tauri::command]
pub async fn download_matting_model(app: AppHandle) -> Result<ModelStatus, String> {
    if DOWNLOADING.swap(true, Ordering::SeqCst) {
        return Err("模型正在下载中".into());
    }
    let last = std::sync::atomic::AtomicU64::new(0);
    let result = models::download(&MODNET, |downloaded, total| {
        // 每 256KB 推送一次，避免事件过多
        if downloaded == total || downloaded - last.load(Ordering::Relaxed) >= 256 * 1024 || downloaded == 0 {
            last.store(downloaded, Ordering::Relaxed);
            let _ = app.emit("model-download-progress", Progress { downloaded, total });
        }
    })
    .await;
    DOWNLOADING.store(false, Ordering::SeqCst);
    let path = result.map_err(|e| e.to_string())?;
    imgcore::matting::set_model_path(path);
    Ok(status())
}
