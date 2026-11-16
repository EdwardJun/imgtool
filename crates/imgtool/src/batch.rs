//! 批量处理：展开目录 → 并行处理 → 汇总

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;

use imgcore::project::{run_file_job, walk_images, FileJob};
use imgcore::OutputFormat;

use crate::options::Resolved;

/// 可解码的扩展名（目录展开时只处理这些）
const PROCESSABLE: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff", "ico"];

pub const MAX_BATCH: usize = 500;

#[derive(Debug, Serialize)]
pub struct BatchItem {
    pub input: String,
    pub output: Option<String>,
    pub written: bool,
    pub format: OutputFormat,
    pub width: u32,
    pub height: u32,
    pub original_bytes: u64,
    pub output_bytes: u64,
    /// 节省比例（%），正数表示变小
    pub saved_percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<u8>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub kept_original: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_met: Option<bool>,
    /// 去除背景时识别出的背景色、边缘匹配率、去除比例
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<imgcore::RemovalStats>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BatchError {
    pub input: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct BatchSummary {
    pub dry_run: bool,
    pub processed: usize,
    pub written: usize,
    pub failed: usize,
    pub original_bytes: u64,
    pub output_bytes: u64,
    pub saved_percent: f64,
    /// 超过单次上限被截断的文件数
    #[serde(skip_serializing_if = "is_zero")]
    pub skipped_over_limit: usize,
    pub items: Vec<BatchItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<BatchError>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

fn saved(before: u64, after: u64) -> f64 {
    if before == 0 {
        return 0.0;
    }
    ((1.0 - after as f64 / before as f64) * 1000.0).round() / 10.0
}

pub fn is_processable(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| PROCESSABLE.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// 是否是本工具生成的压缩产物（文件名主体以输出后缀结尾，如 photo.min.jpg）
fn is_generated(path: &Path, suffix: &str) -> bool {
    !suffix.is_empty()
        && path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.ends_with(suffix))
            .unwrap_or(false)
}

/// 展开后的单个输入文件；`base` 是它的"相对基准"：
/// 来自目录输入时为该目录（输出到目录时保留其下的子目录结构），单个文件时为文件所在目录（平铺）
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InputFile {
    pub path: PathBuf,
    pub base: PathBuf,
}

/// 展开输入：文件原样保留；目录递归查找可处理的图片，并跳过已生成的压缩产物
pub fn expand_inputs(paths: &[PathBuf], exclude: &[String], output_suffix: &str) -> Result<Vec<InputFile>, String> {
    let mut out: Vec<InputFile> = Vec::new();
    for p in paths {
        if p.is_dir() {
            let files = walk_images(p, exclude).map_err(|e| e.to_string())?;
            out.extend(
                files
                    .into_iter()
                    .filter(|f| is_processable(f) && !is_generated(f, output_suffix))
                    .map(|f| InputFile { path: f, base: p.clone() }),
            );
        } else {
            let base = p.parent().map(Path::to_path_buf).unwrap_or_default();
            out.push(InputFile { path: p.clone(), base });
        }
    }
    out.sort();
    out.dedup_by(|a, b| a.path == b.path);
    Ok(out)
}

pub fn run_batch(
    mut files: Vec<InputFile>,
    resolved: &Resolved,
    project_root: Option<&Path>,
    dry_run: bool,
    explicit_output: Option<PathBuf>,
    display: &(dyn Fn(&Path) -> String + Sync),
) -> BatchSummary {
    let skipped_over_limit = files.len().saturating_sub(MAX_BATCH);
    files.truncate(MAX_BATCH);

    let results: Vec<_> = files
        .par_iter()
        .map(|f| {
            let job = FileJob {
                input: f.path.clone(),
                options: resolved.options.clone(),
                output: explicit_output.clone(),
                policy: resolved.policy.clone(),
                on_conflict: resolved.on_conflict,
                project_root: project_root.map(Path::to_path_buf),
                relative_base: Some(f.base.clone()),
                dry_run,
                write_when_unchanged: false,
            };
            (&f.path, run_file_job(&job))
        })
        .collect();

    let mut items = Vec::new();
    let mut errors = Vec::new();
    for (input, r) in results {
        match r {
            Ok(res) => {
                let rep = res.report;
                items.push(BatchItem {
                    input: display(input),
                    output: res.output.as_deref().map(|o| display(Path::new(o))),
                    written: res.written,
                    format: rep.format,
                    width: rep.width,
                    height: rep.height,
                    original_bytes: rep.original_bytes,
                    output_bytes: rep.output_bytes,
                    saved_percent: saved(rep.original_bytes, rep.output_bytes),
                    quality: rep.quality,
                    kept_original: rep.kept_original,
                    target_met: rep.target_met,
                    background: rep.background,
                    warnings: rep.warnings,
                });
            }
            Err(e) => errors.push(BatchError { input: display(input), error: e.to_string() }),
        }
    }
    let original_bytes = items.iter().map(|i| i.original_bytes).sum();
    let output_bytes = items.iter().map(|i| i.output_bytes).sum();
    BatchSummary {
        dry_run,
        processed: items.len(),
        written: items.iter().filter(|i| i.written).count(),
        failed: errors.len(),
        original_bytes,
        output_bytes,
        saved_percent: saved(original_bytes, output_bytes),
        skipped_over_limit,
        items,
        errors,
    }
}

impl BatchSummary {
    pub fn text_summary(&self) -> String {
        let verb = if self.dry_run { "预计" } else { "已" };
        let mut s = format!(
            "{verb}处理 {} 张，写入 {} 张，失败 {} 张；体积 {} → {}（节省 {}%）",
            self.processed,
            self.written,
            self.failed,
            human_bytes(self.original_bytes),
            human_bytes(self.output_bytes),
            self.saved_percent
        );
        if self.skipped_over_limit > 0 {
            s.push_str(&format!("；超过单次上限 {MAX_BATCH}，另有 {} 张未处理", self.skipped_over_limit));
        }
        s
    }
}

pub fn human_bytes(b: u64) -> String {
    if b >= 1024 * 1024 {
        format!("{:.2} MB", b as f64 / 1048576.0)
    } else {
        format!("{:.1} KB", b as f64 / 1024.0)
    }
}
