//! OpenAI 兼容的对话接口（`/v1/chat/completions`），提示词优化和 SVG 图标生成共用。

use std::time::Duration;

use serde_json::{json, Value};

use crate::{GenError, Result};

pub struct ChatClient {
    base_url: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
    /// 合并进请求体的额外参数（如关闭思考）
    extra: Value,
}

pub struct ChatReply {
    pub content: String,
    pub model: String,
    pub total_tokens: Option<u64>,
    /// stop / length 等
    pub finish_reason: Option<String>,
    /// 思考模型的推理 token 数（服务端返回时才有）
    pub reasoning_tokens: Option<u64>,
}

impl ChatClient {
    /// `model` 为空时使用 `default_model`
    pub fn new(base_url: &str, api_key: &str, model: &str, default_model: &str, timeout: Duration) -> Result<Self> {
        let base_url = base_url.trim().trim_end_matches('/').to_string();
        if !(base_url.starts_with("https://") || base_url.starts_with("http://")) {
            return Err(GenError::InvalidRequest("接口地址需以 http:// 或 https:// 开头".into()));
        }
        let api_key = api_key.trim().to_string();
        if api_key.is_empty() {
            return Err(GenError::InvalidRequest("请先在「设置」里填写 API Key".into()));
        }
        let model = match model.trim() {
            "" => default_model.to_string(),
            m => m.to_string(),
        };
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(timeout)
            .user_agent(concat!("imgtool/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| GenError::Provider(format!("HTTP 客户端初始化失败: {e}")))?;
        Ok(Self { base_url, api_key, model, client, extra: Value::Null })
    }

    /// 额外请求参数，键会覆盖默认请求体中的同名字段
    pub fn with_extra(mut self, extra: Value) -> Self {
        self.extra = extra;
        self
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// 发送一轮对话。`label` 用于错误信息，如"优化提示词"
    pub async fn complete(&self, label: &str, system: &str, user: &str, temperature: f32, max_tokens: u32) -> Result<ChatReply> {
        let mut body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
            "temperature": temperature,
            "max_tokens": max_tokens,
        });
        if let (Some(b), Some(e)) = (body.as_object_mut(), self.extra.as_object()) {
            for (k, v) in e {
                b.insert(k.clone(), v.clone());
            }
        }
        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| GenError::Provider(format!("{label}失败: {}", if e.is_timeout() { "请求超时".to_string() } else { e.to_string() })))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| GenError::Provider(format!("{label}失败: {e}")))?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|_| GenError::Provider(format!("{label}失败: HTTP {status} {}", text.chars().take(200).collect::<String>())))?;
        if !status.is_success() {
            let msg = v
                .pointer("/error/message")
                .and_then(Value::as_str)
                .or_else(|| v.get("message").and_then(Value::as_str))
                .unwrap_or("服务端返回错误");
            let hint = match status.as_u16() {
                401 => "（请检查 API Key）",
                404 => "（该接口可能不支持对话模型，或模型名不存在）",
                _ => "",
            };
            return Err(GenError::Provider(format!("{label}失败: HTTP {status} {msg}{hint}")));
        }
        let content = v.pointer("/choices/0/message/content").and_then(Value::as_str).unwrap_or("").to_string();
        Ok(ChatReply {
            content,
            model: self.model.clone(),
            total_tokens: v.pointer("/usage/total_tokens").and_then(Value::as_u64),
            finish_reason: v.pointer("/choices/0/finish_reason").and_then(Value::as_str).map(str::to_string),
            reasoning_tokens: v.pointer("/usage/completion_tokens_details/reasoning_tokens").and_then(Value::as_u64),
        })
    }
}
