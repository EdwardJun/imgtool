//! hfsyapi 免费图床：`POST {base}/v1/files/image-upload`，JSON 传 base64，返回带签名的临时链接（约 24 小时有效）。
//! 生成接口只接受 http(s) 图片地址作为参考图，本地图片需要先上传到这里。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{GenError, Result};

/// 返回的链接没有 Expires 参数时，按 24 小时计算
pub const DEFAULT_TTL_SECS: u64 = 24 * 3600;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UploadedImage {
    pub url: String,
    /// 链接过期时间（Unix 秒）
    pub expires_at: u64,
    pub size: u64,
}

pub struct ImageUploader {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
}

/// 是否是支持图床上传的 hfsyapi 地址
pub fn supports_upload(base_url: &str) -> bool {
    base_url.to_ascii_lowercase().contains("hfsyapi.cn")
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 从签名链接里读出 Expires=（OSS）或 X-Amz-Date + X-Amz-Expires（S3），读不到时用默认有效期
pub fn parse_expires(url: &str, uploaded_at: u64) -> u64 {
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    let param = |name: &str| {
        query.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            (k == name).then(|| v.to_string())
        })
    };
    if let Some(ts) = param("Expires").and_then(|v| v.parse::<u64>().ok()) {
        return ts;
    }
    if let Some(secs) = param("X-Amz-Expires").and_then(|v| v.parse::<u64>().ok()) {
        return uploaded_at + secs;
    }
    uploaded_at + DEFAULT_TTL_SECS
}

impl ImageUploader {
    pub fn new(base_url: &str, api_key: &str) -> Result<Self> {
        let base_url = base_url.trim().trim_end_matches('/').to_string();
        let api_key = api_key.trim().to_string();
        if api_key.is_empty() {
            return Err(GenError::InvalidRequest("请填写 API Key".into()));
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(120))
            .user_agent(concat!("imgtool/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| GenError::Provider(format!("HTTP 客户端初始化失败: {e}")))?;
        Ok(Self { base_url, api_key, client })
    }

    pub async fn upload(&self, data: &[u8], mime: &str, filename: &str) -> Result<UploadedImage> {
        if data.is_empty() {
            return Err(GenError::InvalidRequest("图片为空".into()));
        }
        let body = json!({
            "base64": base64::engine::general_purpose::STANDARD.encode(data),
            "mime_type": mime,
            "filename": filename,
        });
        let started = now_secs();
        let resp = self
            .client
            .post(format!("{}/v1/files/image-upload", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| GenError::Provider(format!("上传参考图失败: {}", if e.is_timeout() { "请求超时".to_string() } else { e.to_string() })))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| GenError::Provider(format!("上传参考图失败: {e}")))?;
        let v: Value = serde_json::from_str(&text).map_err(|_| {
            GenError::Provider(format!("上传参考图失败: HTTP {status} {}", text.chars().take(200).collect::<String>()))
        })?;
        let ok = status.is_success() && v.get("success").and_then(Value::as_bool).unwrap_or(true);
        let url = v.pointer("/data/url").and_then(Value::as_str).filter(|u| !u.is_empty());
        match (ok, url) {
            (true, Some(url)) => Ok(UploadedImage {
                url: url.to_string(),
                expires_at: parse_expires(url, started),
                size: v.pointer("/data/size").and_then(Value::as_u64).unwrap_or(data.len() as u64),
            }),
            _ => {
                let msg = v
                    .get("message")
                    .and_then(Value::as_str)
                    .filter(|m| !m.is_empty())
                    .or_else(|| v.pointer("/error/message").and_then(Value::as_str))
                    .unwrap_or("服务端没有返回图片地址");
                let hint = if status.as_u16() == 401 { "（请检查 API Key）" } else { "" };
                Err(GenError::Provider(format!("上传参考图失败: HTTP {status} {msg}{hint}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::serve_once;

    #[test]
    fn expires_parsing() {
        assert_eq!(parse_expires("https://x/a.png?Expires=1794900902&Signature=abc", 0), 1794900902);
        assert_eq!(parse_expires("https://x/a.png?X-Amz-Date=1&X-Amz-Expires=86400", 100), 86500);
        assert_eq!(parse_expires("https://x/a.png", 100), 100 + DEFAULT_TTL_SECS);
        assert!(supports_upload("https://www.HFSYAPI.cn/"));
        assert!(!supports_upload("https://api.openai.com"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn uploads_and_parses_url() {
        let (base, rx) = serve_once(
            "200 OK",
            r#"{"success":true,"message":"","data":{"url":"https://oss/x.png?Expires=1900000000&Signature=s","name":"a.png","filename":"img-1.png","size":4}}"#,
        )
        .await;
        let up = ImageUploader::new(&base, "sk-test").unwrap();
        let r = up.upload(b"\x89PNG", "image/png", "a.png").await.unwrap();
        assert_eq!(r.url, "https://oss/x.png?Expires=1900000000&Signature=s");
        assert_eq!((r.expires_at, r.size), (1_900_000_000, 4));
        let req = rx.await.unwrap();
        assert!(req.starts_with("POST /v1/files/image-upload"));
        assert!(req.to_ascii_lowercase().contains("authorization: bearer sk-test"));
        assert!(req.contains(r#""base64":"iVBORw==""#) && req.contains(r#""mime_type":"image/png""#));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reports_failures() {
        let (base, _) = serve_once("200 OK", r#"{"success":false,"message":"图片过大","data":null}"#).await;
        let err = ImageUploader::new(&base, "k").unwrap().upload(b"x", "image/png", "a.png").await.unwrap_err();
        assert!(err.to_string().contains("图片过大"), "{err}");

        let (base, _) = serve_once("401 Unauthorized", r#"{"message":"Invalid token","success":false}"#).await;
        let err = ImageUploader::new(&base, "k").unwrap().upload(b"x", "image/png", "a.png").await.unwrap_err();
        assert!(err.to_string().contains("Invalid token") && err.to_string().contains("API Key"), "{err}");
    }
}
