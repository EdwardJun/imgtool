//! AI 人像抠图模型：MCP / 命令行在使用前确保模型已下载并注册给 imgcore

use imggen::models::{self, MODNET};

/// 模型已就绪时直接注册；否则下载（约 25MB）。设置了 IMGTOOL_MATTING_MODEL 时用指定文件
pub async fn ensure_model() -> Result<(), String> {
    if imgcore::matting::is_available() {
        return Ok(());
    }
    if let Some(p) = std::env::var_os(imgcore::matting::MODEL_ENV) {
        return Err(format!("{} 指向的模型文件不存在: {}", imgcore::matting::MODEL_ENV, std::path::Path::new(&p).display()));
    }
    if !models::is_downloaded(&MODNET) {
        tracing::info!("下载 AI 人像抠图模型 {}（{} MB）", MODNET.file_name, MODNET.bytes / 1024 / 1024);
    }
    let path = models::download(&MODNET, |_, _| {}).await.map_err(|e| e.to_string())?;
    imgcore::matting::set_model_path(path);
    Ok(())
}
