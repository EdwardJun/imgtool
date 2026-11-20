//! 本地模型文件管理：AI 人像抠图模型（MODNet）的存放位置与下载。
//!
//! 桌面端和 MCP 共用同一个位置（与桌面端 Tauri 的 app_config_dir 一致），下载一次两边都能用。
//! 依次尝试多个下载源，下载完校验 SHA-256，先写临时文件再改名，中途失败不会留下坏文件。

use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::{GenError, Result};

/// 与桌面端 tauri.conf.json 的 identifier 一致
const APP_IDENTIFIER: &str = "com.imgtool.desktop";

/// 模型信息
pub struct ModelSpec {
    pub file_name: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    /// 按顺序尝试的下载地址（同一个文件）
    pub urls: &'static [&'static str],
    pub license: &'static str,
}

/// MODNet 官方人像抠图权重（photographic portrait matting，Apache-2.0）
pub const MODNET: ModelSpec = ModelSpec {
    file_name: "modnet_photographic_portrait_matting.onnx",
    bytes: 25_888_640,
    sha256: "07c308cf0fc7e6e8b2065a12ed7fc07e1de8febb7dc7839d7b7f15dd66584df9",
    urls: &[
        // 自有服务器（国内访问快）；HTTP 也安全：下载后校验大小和 SHA-256
        "http://106.75.18.68/imgtool/models/modnet_photographic_portrait_matting.onnx",
        "https://huggingface.co/Xenova/modnet/resolve/main/onnx/model.onnx",
        "https://github.com/Zeyi-Lin/HivisionIDPhotos/releases/download/pretrained-model/modnet_photographic_portrait_matting.onnx",
        "https://hf-mirror.com/Xenova/modnet/resolve/main/onnx/model.onnx",
    ],
    license: "Apache-2.0（MODNet, ZHKKKe/MODNet）",
};

/// 模型目录：macOS ~/Library/Application Support/com.imgtool.desktop/models，
/// Windows %APPDATA%\com.imgtool.desktop\models，Linux ~/.config/com.imgtool.desktop/models
pub fn models_dir() -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join(APP_IDENTIFIER).join("models"))
}

/// 模型的默认存放路径
pub fn model_path(spec: &ModelSpec) -> Option<PathBuf> {
    models_dir().map(|d| d.join(spec.file_name))
}

/// 已下载且大小正确（完整校验只在下载时做一次）
pub fn is_downloaded(spec: &ModelSpec) -> bool {
    model_path(spec).and_then(|p| std::fs::metadata(p).ok()).is_some_and(|m| m.len() == spec.bytes)
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// 下载模型到默认位置；已存在时直接返回。progress(已下载字节, 总字节)
pub async fn download(spec: &ModelSpec, progress: impl Fn(u64, u64) + Send) -> Result<PathBuf> {
    let path = model_path(spec).ok_or_else(|| GenError::Provider("找不到用户目录，无法保存模型".into()))?;
    if is_downloaded(spec) {
        return Ok(path);
    }
    download_to(spec, &path, progress).await?;
    Ok(path)
}

async fn download_to(spec: &ModelSpec, path: &Path, progress: impl Fn(u64, u64) + Send) -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .user_agent(concat!("imgtool/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| GenError::Provider(format!("HTTP 客户端初始化失败: {e}")))?;
    let mut errors = Vec::new();
    for url in spec.urls {
        match fetch(&client, url, spec, &progress).await {
            Ok(data) => {
                let dir = path.parent().ok_or_else(|| GenError::Provider("模型目录无效".into()))?;
                std::fs::create_dir_all(dir).map_err(|e| GenError::Provider(format!("创建模型目录失败: {e}")))?;
                let tmp = path.with_extension("onnx.part");
                std::fs::write(&tmp, &data).map_err(|e| GenError::Provider(format!("保存模型失败: {e}")))?;
                std::fs::rename(&tmp, path).map_err(|e| GenError::Provider(format!("保存模型失败: {e}")))?;
                return Ok(());
            }
            Err(e) => errors.push(format!("{}: {e}", host(url))),
        }
    }
    Err(GenError::Provider(format!("模型下载失败（{}）。可以手动下载后设置环境变量 IMGTOOL_MATTING_MODEL 指向文件", errors.join("；"))))
}

fn host(url: &str) -> &str {
    url.split('/').nth(2).unwrap_or(url)
}

async fn fetch(client: &reqwest::Client, url: &str, spec: &ModelSpec, progress: &impl Fn(u64, u64)) -> std::result::Result<Vec<u8>, String> {
    let mut resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let mut data = Vec::with_capacity(spec.bytes as usize);
    progress(0, spec.bytes);
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        data.extend_from_slice(&chunk);
        if data.len() as u64 > spec.bytes {
            return Err("文件大小不符".into());
        }
        progress(data.len() as u64, spec.bytes);
    }
    if data.len() as u64 != spec.bytes {
        return Err(format!("下载不完整（{} / {} 字节）", data.len(), spec.bytes));
    }
    if sha256_hex(&data) != spec.sha256 {
        return Err("校验失败，文件可能被篡改".into());
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_server_is_tried_first() {
        assert!(MODNET.urls[0].starts_with("http://106.75.18.68/"), "{:?}", MODNET.urls);
        assert!(MODNET.urls.len() > 1, "保留备用下载源");
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[tokio::test]
    async fn rejects_wrong_size_and_hash() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf).await;
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc").await;
            }
        });
        let url = format!("http://{addr}/m.onnx");
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::new();
        let mut spec = ModelSpec { file_name: "m.onnx", bytes: 3, sha256: "00", urls: &[], license: "" };
        let err = fetch(&client, &url, &spec, &|_, _| {}).await.unwrap_err();
        assert!(err.contains("校验失败"), "{err}");
        spec.sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(fetch(&client, &url, &spec, &|_, _| {}).await.unwrap(), b"abc");
        spec.bytes = 10;
        assert!(fetch(&client, &url, &spec, &|_, _| {}).await.unwrap_err().contains("不完整"));
    }
}
