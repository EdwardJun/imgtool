//! MCP 的异步图片生成任务：提交后立即返回任务编号，后台执行
//! （参考图上传 → 调用生成接口 → 下载 → 去背景/调整尺寸/转格式 → 写入项目目录），
//! Agent 用 get_generation 查询（可等待一段时间），避免单次工具调用超时。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use imgcore::postprocess::{finalize, prepare_reference, FinalSize};
use imgcore::project::{resolve_conflict, write_atomic, ConflictPolicy, OutputPolicy};
use imgcore::{BackgroundRemoval, OutputFormat};
use imggen::{GenerateRequest, ImageUploader, TaskApiConfig, TaskApiProvider};

use crate::genconfig::GenConfig;

static SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Completed,
    /// 部分成功
    Partial,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutputFile {
    /// 写入的绝对路径（返回给 Agent 前会转成相对项目根目录）
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskState {
    pub task_id: String,
    pub status: Status,
    pub model: String,
    pub size: String,
    pub count: u32,
    /// 当前进度说明
    pub stage: String,
    pub elapsed_secs: u64,
    pub files: Vec<OutputFile>,
    pub errors: Vec<String>,
    #[serde(skip)]
    started: Option<Instant>,
}

/// 已解析好的生成参数
#[derive(Debug, Clone)]
pub struct GenJob {
    pub prompt: String,
    pub model: String,
    pub width: u32,
    pub height: u32,
    pub count: u32,
    /// 本地绝对路径或 http(s) 地址
    pub references: Vec<String>,
    pub format: OutputFormat,
    pub remove_background: Option<BackgroundRemoval>,
    pub final_size: Option<FinalSize>,
    pub out_dir: PathBuf,
    /// 文件名前缀
    pub name: String,
}

#[derive(Clone, Default)]
pub struct TaskStore(Arc<Mutex<HashMap<String, TaskState>>>);

impl TaskStore {
    pub fn get(&self, id: &str) -> Option<TaskState> {
        let mut s = self.0.lock().ok()?.get(id).cloned()?;
        if let Some(t) = s.started {
            s.elapsed_secs = t.elapsed().as_secs();
        }
        Some(s)
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut TaskState)) {
        if let Ok(mut map) = self.0.lock() {
            if let Some(s) = map.get_mut(id) {
                f(s);
            }
        }
    }

    /// 等待任务结束，最多 wait
    pub async fn wait(&self, id: &str, wait: Duration) -> Option<TaskState> {
        let deadline = Instant::now() + wait;
        loop {
            let s = self.get(id)?;
            if s.status != Status::Running || Instant::now() >= deadline {
                return Some(s);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// 提交任务并在后台执行
    pub fn submit(&self, cfg: GenConfig, job: GenJob) -> String {
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let id = format!("gen_{:x}{:02x}", now_ms, SEQ.fetch_add(1, Ordering::Relaxed) % 256);
        let state = TaskState {
            task_id: id.clone(),
            status: Status::Running,
            model: job.model.clone(),
            size: format!("{}x{}", job.width, job.height),
            count: job.count,
            stage: "已提交".into(),
            elapsed_secs: 0,
            files: vec![],
            errors: vec![],
            started: Some(Instant::now()),
        };
        if let Ok(mut map) = self.0.lock() {
            // 只保留最近 50 个任务
            if map.len() >= 50 {
                if let Some(oldest) = map.values().filter(|s| s.status != Status::Running).min_by_key(|s| s.task_id.clone()).map(|s| s.task_id.clone()) {
                    map.remove(&oldest);
                }
            }
            map.insert(id.clone(), state);
        }
        let store = self.clone();
        let task_id = id.clone();
        tokio::spawn(async move { store.run(task_id, cfg, job).await });
        id
    }

    async fn run(&self, id: String, cfg: GenConfig, job: GenJob) {
        // 1. 参考图：hfsyapi 只接受图片地址，本地图片先压缩并上传到图床
        let mut refs = Vec::with_capacity(job.references.len());
        if job.references.iter().any(|r| !is_remote(r)) && cfg.upload_refs {
            self.update(&id, |s| s.stage = "上传参考图".into());
        }
        for (i, r) in job.references.iter().enumerate() {
            if is_remote(r) || !cfg.upload_refs {
                refs.push(r.clone());
                continue;
            }
            match upload_reference(&cfg, Path::new(r)).await {
                Ok(url) => refs.push(url),
                Err(e) => {
                    self.update(&id, |s| {
                        s.status = Status::Failed;
                        s.stage = "参考图上传失败".into();
                        s.errors.push(format!("第 {} 张参考图上传失败：{e}", i + 1));
                    });
                    return;
                }
            }
        }

        // 2. 生成：每张图一个任务，并发执行
        let provider = match TaskApiProvider::new(TaskApiConfig { base_url: cfg.base_url.clone(), api_key: cfg.api_key.clone(), model: job.model.clone() }) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                self.update(&id, |s| {
                    s.status = Status::Failed;
                    s.errors.push(e.to_string());
                });
                return;
            }
        };
        self.update(&id, |s| s.stage = format!("生成中（{} 张，每张约 30 秒到 3 分钟）", job.count));
        let req = GenerateRequest {
            prompt: job.prompt.clone(),
            width: Some(job.width),
            height: Some(job.height),
            count: Some(1),
            reference_images: refs,
            ..Default::default()
        };
        let handles: Vec<_> = (0..job.count)
            .map(|_| {
                let p = provider.clone();
                let r = req.clone();
                tokio::spawn(async move { p.run_once(&r).await })
            })
            .collect();

        // 3. 逐个处理结果并写入项目
        for (i, h) in handles.into_iter().enumerate() {
            let result = match h.await {
                Ok(r) => r.map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            match result {
                Ok((task, images)) => {
                    for img in images {
                        let job2 = job.clone();
                        let task2 = task.clone();
                        let saved = tokio::task::spawn_blocking(move || save_output(&job2, i, &task2, img.data, img.revised_prompt)).await;
                        match saved {
                            Ok(Ok(file)) => self.update(&id, |s| s.files.push(file)),
                            Ok(Err(e)) => self.update(&id, |s| s.errors.push(format!("第 {} 张保存失败：{e}", i + 1))),
                            Err(e) => self.update(&id, |s| s.errors.push(e.to_string())),
                        }
                    }
                }
                Err(e) => self.update(&id, |s| s.errors.push(format!("第 {} 张生成失败：{e}", i + 1))),
            }
            self.update(&id, |s| s.stage = format!("已完成 {}/{} 张", i + 1, job.count));
        }
        self.update(&id, |s| {
            s.status = match (s.files.is_empty(), s.errors.is_empty()) {
                (false, true) => Status::Completed,
                (false, false) => Status::Partial,
                _ => Status::Failed,
            };
            s.stage = match s.status {
                Status::Completed => "已完成".into(),
                Status::Partial => "部分完成".into(),
                _ => "失败".into(),
            };
            s.elapsed_secs = s.started.map(|t| t.elapsed().as_secs()).unwrap_or(0);
            s.started = None;
        });
    }
}

pub fn is_remote(r: &str) -> bool {
    let r = r.trim();
    r.starts_with("http://") || r.starts_with("https://") || r.starts_with("data:")
}

async fn upload_reference(cfg: &GenConfig, path: &Path) -> Result<String, String> {
    let owned = path.to_path_buf();
    let prepared = tokio::task::spawn_blocking(move || prepare_reference(&owned)).await.map_err(|e| e.to_string())??;
    let uploader = ImageUploader::new(&cfg.base_url, &cfg.api_key).map_err(|e| e.to_string())?;
    let up = uploader.upload(&prepared.data, prepared.mime, &prepared.filename).await.map_err(|e| e.to_string())?;
    Ok(up.url)
}

fn save_output(job: &GenJob, index: usize, task: &str, data: Vec<u8>, revised_prompt: Option<String>) -> Result<OutputFile, String> {
    let out = finalize(data, job.format, job.remove_background.clone(), job.final_size)?;
    let ext = if out.ext == "jpeg" { "jpg".to_string() } else { out.ext.clone() };
    let tag: String = task.chars().rev().filter(|c| c.is_ascii_alphanumeric()).take(6).collect::<Vec<_>>().into_iter().rev().collect();
    let file_name = if job.count > 1 { format!("{}-{}-{}.{ext}", job.name, tag, index + 1) } else { format!("{}-{}.{ext}", job.name, tag) };
    let target = job.out_dir.join(file_name);
    let target = resolve_conflict(target, Path::new(""), &OutputPolicy::default(), ConflictPolicy::Rename).map_err(|e| e.to_string())?;
    write_atomic(&target, &out.data).map_err(|e| e.to_string())?;
    Ok(OutputFile {
        path: target.display().to_string(),
        width: out.width,
        height: out.height,
        format: out.ext,
        bytes: out.data.len() as u64,
        revised_prompt,
        warnings: out.warnings,
    })
}
