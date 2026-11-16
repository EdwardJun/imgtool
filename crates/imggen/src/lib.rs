//! imggen：图片生成适配层。
//!
//! 生成服务以 [`ImageProvider`] 接入。拿到正式接口后，新增一个实现了该 trait 的
//! Provider（如 `HttpProvider`）并在 [`create_provider`] 里注册即可，
//! 桌面端与 MCP 不需要改动调用方式。生成结果是原始图片字节，可直接交给
//! `imgcore::process` 按项目预设压缩/转格式后落盘。

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

pub mod fetch;
pub mod prompt;
pub mod sizes;
pub mod task_api;
#[cfg(test)]
mod testutil;
pub mod upload;
pub use prompt::{EnhanceResult, PromptContext, PromptEnhancer, DEFAULT_PROMPT_MODEL};
pub use task_api::{PollConfig, TaskApiConfig, TaskApiProvider};
pub use upload::{supports_upload, ImageUploader, UploadedImage};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, thiserror::Error)]
pub enum GenError {
    #[error("未知的生成服务: {0}")]
    UnknownProvider(String),
    #[error("参数无效: {0}")]
    InvalidRequest(String),
    #[error("生成服务调用失败: {0}")]
    Provider(String),
}

pub type Result<T> = std::result::Result<T, GenError>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GenerateRequest {
    pub prompt: String,
    #[serde(default)]
    pub negative_prompt: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// 生成张数，默认 1
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// 参考图（本地路径），供图生图类接口使用
    #[serde(default)]
    pub reference_images: Vec<String>,
    /// 各服务特有参数，原样透传
    #[serde(default)]
    pub extra: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct GeneratedImage {
    pub data: Vec<u8>,
    pub mime: String,
    pub seed: Option<u64>,
    /// 服务端返回的原始图片地址
    pub source_url: Option<String>,
    /// 服务端改写后的提示词
    pub revised_prompt: Option<String>,
}

pub trait ImageProvider: Send + Sync {
    fn name(&self) -> &str;
    fn generate<'a>(&'a self, req: &'a GenerateRequest) -> BoxFuture<'a, Result<Vec<GeneratedImage>>>;
}

/// 生成服务配置（后续写入应用设置或项目预设）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum ProviderConfig {
    /// 本地占位实现，用于联调 UI 和流程
    Mock,
    /// 异步任务式接口（hfsyapi.cn 兼容）：create → query 轮询 → 下载
    TaskApi(TaskApiConfig),
}

pub fn create_provider(config: &ProviderConfig) -> Result<Box<dyn ImageProvider>> {
    match config {
        ProviderConfig::Mock => Ok(Box::new(MockProvider)),
        ProviderConfig::TaskApi(c) => Ok(Box::new(TaskApiProvider::new(c.clone())?)),
    }
}

/// 生成渐变占位图，尺寸与张数遵循请求
pub struct MockProvider;

impl ImageProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn generate<'a>(&'a self, req: &'a GenerateRequest) -> BoxFuture<'a, Result<Vec<GeneratedImage>>> {
        Box::pin(async move {
            if req.prompt.trim().is_empty() {
                return Err(GenError::InvalidRequest("prompt 不能为空".into()));
            }
            let (w, h) = (req.width.unwrap_or(512), req.height.unwrap_or(512));
            if w == 0 || h == 0 || w > 4096 || h > 4096 {
                return Err(GenError::InvalidRequest("宽高需在 1-4096".into()));
            }
            let count = req.count.unwrap_or(1).clamp(1, 4);
            // 用 prompt 的哈希决定色调，同一 prompt 结果稳定
            let hue = req.prompt.bytes().fold(req.seed.unwrap_or(0), |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
            (0..count)
                .map(|i| {
                    let base = hue.wrapping_add(i as u64 * 97);
                    let img = image::RgbImage::from_fn(w, h, |x, y| {
                        image::Rgb([
                            ((x * 255 / w) as u64 ^ base) as u8,
                            ((y * 255 / h) as u64 ^ (base >> 8)) as u8,
                            (base >> 16) as u8,
                        ])
                    });
                    let mut buf = std::io::Cursor::new(Vec::new());
                    img.write_to(&mut buf, image::ImageFormat::Png)
                        .map_err(|e| GenError::Provider(e.to_string()))?;
                    Ok(GeneratedImage {
                        data: buf.into_inner(),
                        mime: "image/png".into(),
                        seed: Some(base),
                        source_url: None,
                        revised_prompt: None,
                    })
                })
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_generates_requested_size() {
        let p = create_provider(&ProviderConfig::Mock).unwrap();
        let req = GenerateRequest { prompt: "logo".into(), width: Some(64), height: Some(32), count: Some(2), ..Default::default() };
        let images = p.generate(&req).await.unwrap();
        assert_eq!(images.len(), 2);
        let img = image::load_from_memory(&images[0].data).unwrap();
        assert_eq!((img.width(), img.height()), (64, 32));
    }

    #[tokio::test]
    async fn rejects_empty_prompt() {
        assert!(MockProvider.generate(&GenerateRequest::default()).await.is_err());
    }
}
