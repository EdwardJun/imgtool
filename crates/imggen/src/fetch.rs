//! 下载网络图片（Agent 传来的图片地址、生成结果），带体积上限

use std::time::Duration;

use crate::{GenError, Result};

/// 单张图片下载上限
pub const MAX_DOWNLOAD_BYTES: usize = 30 * 1024 * 1024;

pub async fn download_image(url: &str) -> Result<(Vec<u8>, Option<String>)> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(GenError::InvalidRequest("图片地址需以 http:// 或 https:// 开头".into()));
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(120))
        .user_agent(concat!("imgtool/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| GenError::Provider(format!("HTTP 客户端初始化失败: {e}")))?;
    let mut resp = client.get(url).send().await.map_err(|e| GenError::Provider(format!("下载图片失败: {e}")))?;
    if !resp.status().is_success() {
        return Err(GenError::Provider(format!("下载图片失败: HTTP {}（链接可能已过期）", resp.status())));
    }
    if resp.content_length().is_some_and(|n| n as usize > MAX_DOWNLOAD_BYTES) {
        return Err(GenError::InvalidRequest(format!("图片超过 {}MB", MAX_DOWNLOAD_BYTES / 1024 / 1024)));
    }
    let mime = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_string);
    let mut data = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| GenError::Provider(format!("下载图片失败: {e}")))? {
        data.extend_from_slice(&chunk);
        if data.len() > MAX_DOWNLOAD_BYTES {
            return Err(GenError::InvalidRequest(format!("图片超过 {}MB", MAX_DOWNLOAD_BYTES / 1024 / 1024)));
        }
    }
    Ok((data, mime))
}
