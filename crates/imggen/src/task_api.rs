//! 异步任务式图片接口（hfsyapi.cn 兼容）：
//! `POST {base}/v1/image/create` 创建任务 → `GET {base}/v1/image/query/{id}` 轮询 → 下载结果图片。
//! 不同模型只换 `model` 名称，参数相同。

use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{BoxFuture, GenError, GenerateRequest, GeneratedImage, ImageProvider, Result};

/// 任务轮询参数
#[derive(Debug, Clone)]
pub struct PollConfig {
    pub interval: Duration,
    pub timeout: Duration,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self { interval: Duration::from_secs(3), timeout: Duration::from_secs(600) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskApiConfig {
    /// 接口根地址，如 https://www.hfsyapi.cn
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

pub struct TaskApiProvider {
    config: TaskApiConfig,
    poll: PollConfig,
    client: reqwest::Client,
}

/// 安装 rustls 使用的 ring 加密实现（只需一次，重复调用无副作用）
fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

impl TaskApiProvider {
    pub fn new(config: TaskApiConfig) -> Result<Self> {
        Self::with_poll(config, PollConfig::default())
    }

    pub fn with_poll(mut config: TaskApiConfig, poll: PollConfig) -> Result<Self> {
        config.base_url = config.base_url.trim().trim_end_matches('/').to_string();
        config.api_key = config.api_key.trim().to_string();
        config.model = config.model.trim().to_string();
        if !(config.base_url.starts_with("https://") || config.base_url.starts_with("http://")) {
            return Err(GenError::InvalidRequest("接口地址需以 http:// 或 https:// 开头".into()));
        }
        if config.api_key.is_empty() {
            return Err(GenError::InvalidRequest("请填写 API Key".into()));
        }
        if config.model.is_empty() {
            return Err(GenError::InvalidRequest("请选择模型".into()));
        }
        ensure_crypto_provider();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(180))
            .user_agent(concat!("imgtool/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| GenError::Provider(format!("HTTP 客户端初始化失败: {e}")))?;
        Ok(Self { config, poll, client })
    }

    /// 创建请求体。参考图：http(s) URL 原样传入，本地文件转为 data URL
    pub fn build_body(&self, req: &GenerateRequest) -> Result<Value> {
        let prompt = req.prompt.trim();
        if prompt.is_empty() {
            return Err(GenError::InvalidRequest("提示词不能为空".into()));
        }
        if prompt.chars().count() > 5000 {
            return Err(GenError::InvalidRequest("提示词最多 5000 字".into()));
        }
        let mut body = json!({ "model": self.config.model, "prompt": prompt });
        let size = match (req.width, req.height) {
            (Some(w), Some(h)) => format!("{w}x{h}"),
            _ => "1024x1024".to_string(),
        };
        body["size"] = Value::String(size);
        if !req.reference_images.is_empty() {
            let refs = req
                .reference_images
                .iter()
                .map(|r| reference_to_value(r))
                .collect::<Result<Vec<_>>>()?;
            body["reference_images"] = Value::Array(refs);
        }
        // extra 中的字段（如 background: "transparent"）原样合并
        if let Value::Object(extra) = &req.extra {
            for (k, v) in extra {
                if !v.is_null() {
                    body[k] = v.clone();
                }
            }
        }
        Ok(body)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.config.base_url)
    }

    async fn create_task(&self, body: &Value) -> Result<String> {
        let resp = self
            .client
            .post(self.url("/v1/image/create"))
            .bearer_auth(&self.config.api_key)
            .json(body)
            .send()
            .await
            .map_err(|e| GenError::Provider(format!("创建任务请求失败: {}", describe_reqwest(&e))))?;
        let task = parse_json(resp, "创建任务").await?;
        if let Some(err) = task_error(&task) {
            return Err(GenError::Provider(err));
        }
        task.get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| GenError::Provider(format!("创建任务的响应缺少 id: {}", truncate(&task.to_string(), 300))))
    }

    async fn query_task(&self, id: &str) -> Result<Value> {
        let resp = self
            .client
            .get(self.url(&format!("/v1/image/query/{id}")))
            .bearer_auth(&self.config.api_key)
            .send()
            .await
            .map_err(|e| GenError::Provider(format!("查询任务请求失败: {}", describe_reqwest(&e))))?;
        parse_json(resp, "查询任务").await
    }

    /// 轮询直到完成，返回结果图片 URL
    async fn wait_task(&self, id: &str) -> Result<(Vec<String>, Option<String>)> {
        let start = Instant::now();
        let mut transient_errors = 0;
        loop {
            match self.query_task(id).await {
                Ok(task) => {
                    transient_errors = 0;
                    let status = task.get("status").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
                    if let Some(err) = task_error(&task) {
                        return Err(GenError::Provider(format!("任务 {id} 失败: {err}")));
                    }
                    match status.as_str() {
                        "completed" | "succeeded" | "success" | "done" | "finished" => {
                            let images = task.get("images").and_then(Value::as_array).cloned().unwrap_or_default();
                            let urls: Vec<String> = images
                                .iter()
                                .filter_map(|i| i.get("url").and_then(Value::as_str).or_else(|| i.as_str()))
                                .map(str::to_string)
                                .collect();
                            if urls.is_empty() {
                                return Err(GenError::Provider(format!("任务 {id} 已完成但没有返回图片")));
                            }
                            let revised = images.first().and_then(|i| i.get("revised_prompt")).and_then(Value::as_str).map(str::to_string);
                            return Ok((urls, revised));
                        }
                        "failed" | "error" | "cancelled" | "canceled" | "expired" => {
                            return Err(GenError::Provider(format!("任务 {id} 状态为 {status}")));
                        }
                        _ => {}
                    }
                }
                // 网络抖动时允许重试几次；鉴权等明确错误直接返回
                Err(GenError::Provider(msg)) if msg.contains("请求失败") && transient_errors < 5 => {
                    transient_errors += 1;
                }
                Err(e) => return Err(e),
            }
            if start.elapsed() > self.poll.timeout {
                return Err(GenError::Provider(format!("任务 {id} 超过 {} 秒未完成", self.poll.timeout.as_secs())));
            }
            tokio::time::sleep(self.poll.interval).await;
        }
    }

    async fn download(&self, url: &str) -> Result<GeneratedImage> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| GenError::Provider(format!("下载图片失败: {}", describe_reqwest(&e))))?;
        if !resp.status().is_success() {
            return Err(GenError::Provider(format!("下载图片失败: HTTP {}", resp.status())));
        }
        let mime = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/png")
            .to_string();
        let data = resp.bytes().await.map_err(|e| GenError::Provider(format!("下载图片失败: {e}")))?.to_vec();
        Ok(GeneratedImage { data, mime, seed: None, source_url: Some(url.to_string()), revised_prompt: None })
    }

    /// 完整流程：创建 → 轮询 → 下载。返回任务 id 便于排查
    pub async fn run_once(&self, req: &GenerateRequest) -> Result<(String, Vec<GeneratedImage>)> {
        let body = self.build_body(req)?;
        let id = self.create_task(&body).await?;
        let (urls, revised) = self.wait_task(&id).await?;
        let mut out = Vec::with_capacity(urls.len());
        for url in urls {
            let mut img = self.download(&url).await?;
            img.revised_prompt = revised.clone();
            out.push(img);
        }
        Ok((id, out))
    }
}

impl ImageProvider for TaskApiProvider {
    fn name(&self) -> &str {
        "task_api"
    }

    /// 接口每个任务出 1 张，count > 1 时并发创建多个任务
    fn generate<'a>(&'a self, req: &'a GenerateRequest) -> BoxFuture<'a, Result<Vec<GeneratedImage>>> {
        Box::pin(async move {
            let count = req.count.unwrap_or(1).clamp(1, 4) as usize;
            let tasks = (0..count).map(|_| self.run_once(req));
            let results = futures_join_all(tasks).await;
            let mut images = Vec::new();
            let mut errors = Vec::new();
            for r in results {
                match r {
                    Ok((_, imgs)) => images.extend(imgs),
                    Err(e) => errors.push(e.to_string()),
                }
            }
            if images.is_empty() {
                return Err(GenError::Provider(errors.join("；")));
            }
            Ok(images)
        })
    }
}

/// 简单的 join_all，避免为此引入 futures crate
async fn futures_join_all<F: std::future::Future>(futs: impl Iterator<Item = F>) -> Vec<F::Output> {
    let mut handles: Vec<std::pin::Pin<Box<F>>> = futs.map(Box::pin).collect();
    let mut out: Vec<Option<F::Output>> = (0..handles.len()).map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (i, h) in handles.iter_mut().enumerate() {
            if out[i].is_none() {
                match h.as_mut().poll(cx) {
                    std::task::Poll::Ready(v) => out[i] = Some(v),
                    std::task::Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(())
        }
    })
    .await;
    out.into_iter().map(|o| o.expect("all futures completed")).collect()
}

fn reference_to_value(r: &str) -> Result<Value> {
    let r = r.trim();
    if r.starts_with("http://") || r.starts_with("https://") || r.starts_with("data:") {
        return Ok(Value::String(r.to_string()));
    }
    let data = std::fs::read(r).map_err(|e| GenError::InvalidRequest(format!("读取参考图 {r} 失败: {e}")))?;
    let mime = match r.rsplit('.').next().map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "image/png",
    };
    Ok(Value::String(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(data))))
}

/// 解析响应 JSON；非 2xx 时提取服务端错误信息
async fn parse_json(resp: reqwest::Response, action: &str) -> Result<Value> {
    let status = resp.status();
    let text = resp.text().await.map_err(|e| GenError::Provider(format!("{action}响应读取失败: {e}")))?;
    let value: Option<Value> = serde_json::from_str(&text).ok();
    if !status.is_success() {
        let msg = value
            .as_ref()
            .and_then(|v| task_error(v).or_else(|| v.get("message").and_then(Value::as_str).map(str::to_string)))
            .unwrap_or_else(|| truncate(&text, 300));
        let hint = if status.as_u16() == 401 { "（请检查 API Key）" } else { "" };
        return Err(GenError::Provider(format!("{action}失败: HTTP {status} {msg}{hint}")));
    }
    value.ok_or_else(|| GenError::Provider(format!("{action}响应不是 JSON: {}", truncate(&text, 300))))
}

/// 兼容 `{"error": "..."}`、`{"error": {"message": ...}}`、`{"success": false, "message": ...}`
fn task_error(v: &Value) -> Option<String> {
    match v.get("error") {
        Some(Value::String(s)) if !s.is_empty() => return Some(s.clone()),
        Some(Value::Object(o)) => {
            return Some(o.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| Value::Object(o.clone()).to_string()))
        }
        _ => {}
    }
    if v.get("success") == Some(&Value::Bool(false)) {
        return Some(v.get("message").and_then(Value::as_str).unwrap_or("请求失败").to_string());
    }
    None
}

fn describe_reqwest(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "请求超时".into()
    } else if e.is_connect() {
        format!("无法连接服务器（{e}）")
    } else {
        e.to_string()
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn provider(base: &str) -> TaskApiProvider {
        TaskApiProvider::with_poll(
            TaskApiConfig { base_url: base.into(), api_key: "sk-test".into(), model: "gpt-image-2".into() },
            PollConfig { interval: Duration::from_millis(20), timeout: Duration::from_secs(5) },
        )
        .unwrap()
    }

    #[test]
    fn body_includes_size_refs_and_extra() {
        let p = provider("https://example.com/");
        let req = GenerateRequest {
            prompt: " 机械小狗 ".into(),
            width: Some(1280),
            height: Some(720),
            reference_images: vec!["https://x/a.png".into()],
            extra: json!({ "background": "transparent", "ignored": null }),
            ..Default::default()
        };
        let b = p.build_body(&req).unwrap();
        assert_eq!(b["model"], "gpt-image-2");
        assert_eq!(b["prompt"], "机械小狗");
        assert_eq!(b["size"], "1280x720");
        assert_eq!(b["reference_images"][0], "https://x/a.png");
        assert_eq!(b["background"], "transparent");
        assert!(b.get("ignored").is_none());
        assert_eq!(p.url("/v1/image/create"), "https://example.com/v1/image/create");
    }

    #[test]
    fn local_reference_becomes_data_url() {
        let dir = std::env::temp_dir().join("imggen-ref-test.png");
        std::fs::write(&dir, b"\x89PNG").unwrap();
        let v = reference_to_value(dir.to_str().unwrap()).unwrap();
        assert!(v.as_str().unwrap().starts_with("data:image/png;base64,"));
    }

    #[test]
    fn validates_config_and_prompt() {
        let bad = TaskApiConfig { base_url: "www.x.com".into(), api_key: "k".into(), model: "m".into() };
        assert!(TaskApiProvider::new(bad).is_err());
        let p = provider("https://x");
        assert!(p.build_body(&GenerateRequest::default()).is_err());
        let long = GenerateRequest { prompt: "字".repeat(5001), ..Default::default() };
        assert!(p.build_body(&long).is_err());
    }

    #[test]
    fn error_shapes() {
        assert_eq!(task_error(&json!({"message":"Invalid token","success":false})).unwrap(), "Invalid token");
        assert_eq!(task_error(&json!({"error":{"message":"bad"}})).unwrap(), "bad");
        assert_eq!(task_error(&json!({"error":"boom"})).unwrap(), "boom");
        assert!(task_error(&json!({"error":null,"status":"queued"})).is_none());
    }

    /// 本地假服务器：create 返回任务 → 前两次 query 返回 processing → 之后返回 completed → 提供图片下载
    async fn fake_server(fail_create: bool) -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let queries = Arc::new(AtomicUsize::new(0));
        let q = queries.clone();
        let base2 = base.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                let q = q.clone();
                let base = base2.clone();
                tokio::spawn(async move {
                    // 读完整个请求（头 + Content-Length 指定的 body），避免提前关闭导致连接被重置
                    let mut data = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        data.extend_from_slice(&chunk[..n]);
                        let text = String::from_utf8_lossy(&data);
                        if let Some(end) = text.find("\r\n\r\n") {
                            let len = text[..end]
                                .lines()
                                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                                .unwrap_or(0);
                            if data.len() >= end + 4 + len {
                                break;
                            }
                        }
                    }
                    let req = String::from_utf8_lossy(&data).to_string();
                    let line = req.lines().next().unwrap_or("").to_string();
                    let authed = req.to_ascii_lowercase().contains("authorization: bearer sk-test");
                    let (status, ctype, body): (&str, &str, Vec<u8>) = if !authed && !line.contains("/files/") {
                        ("401 Unauthorized", "application/json", br#"{"message":"Invalid token","success":false}"#.to_vec())
                    } else if line.starts_with("POST /v1/image/create") {
                        if fail_create {
                            ("400 Bad Request", "application/json", br#"{"error":{"message":"size invalid"}}"#.to_vec())
                        } else {
                            ("200 OK", "application/json", br#"{"id":"task_1","status":"queued","images":[],"error":null}"#.to_vec())
                        }
                    } else if line.starts_with("GET /v1/image/query/task_1") {
                        let n = q.fetch_add(1, Ordering::SeqCst);
                        if n < 2 {
                            ("200 OK", "application/json", br#"{"id":"task_1","status":"processing","images":[],"error":null}"#.to_vec())
                        } else {
                            let b = format!(r#"{{"id":"task_1","status":"completed","images":[{{"url":"{base}/files/a.png","revised_prompt":"rp"}}],"error":null}}"#);
                            ("200 OK", "application/json", b.into_bytes())
                        }
                    } else if line.starts_with("GET /files/a.png") {
                        ("200 OK", "image/png", b"\x89PNGfake".to_vec())
                    } else {
                        ("404 Not Found", "text/plain", b"nf".to_vec())
                    };
                    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(&body).await;
                });
            }
        });
        (base, queries)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn full_flow_polls_until_completed() {
        let (base, queries) = fake_server(false).await;
        let p = provider(&base);
        let req = GenerateRequest { prompt: "dog".into(), width: Some(1024), height: Some(1024), ..Default::default() };
        let (id, imgs) = p.run_once(&req).await.unwrap();
        assert_eq!(id, "task_1");
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0].mime, "image/png");
        assert_eq!(imgs[0].revised_prompt.as_deref(), Some("rp"));
        assert!(queries.load(Ordering::SeqCst) >= 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reports_server_errors() {
        let (base, _) = fake_server(true).await;
        let err = provider(&base).run_once(&GenerateRequest { prompt: "x".into(), ..Default::default() }).await.unwrap_err();
        assert!(err.to_string().contains("size invalid"), "{err}");

        let (base, _) = fake_server(false).await;
        let p = TaskApiProvider::new(TaskApiConfig { base_url: base, api_key: "sk-wrong".into(), model: "m".into() }).unwrap();
        let err = p.run_once(&GenerateRequest { prompt: "x".into(), ..Default::default() }).await.unwrap_err();
        assert!(err.to_string().contains("401") && err.to_string().contains("API Key"), "{err}");
    }
}
