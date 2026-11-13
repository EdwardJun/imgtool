//! 文件级任务：读取 → 处理 → 规划输出路径 → 写入。CLI、MCP、桌面端共用。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::project::output::{plan_output_path, resolve_conflict, write_atomic};
use crate::project::preset::{ConflictPolicy, OutputPolicy};
use crate::types::{ProcessOptions, ProcessReport};

#[derive(Debug, Clone)]
pub struct FileJob {
    pub input: PathBuf,
    pub options: ProcessOptions,
    /// 显式指定的输出路径，优先于输出策略
    pub output: Option<PathBuf>,
    pub policy: OutputPolicy,
    pub on_conflict: ConflictPolicy,
    pub project_root: Option<PathBuf>,
    /// 只计算结果，不写文件
    pub dry_run: bool,
    /// 结果保留了原图时是否仍写出副本（导出到目录时需要，同目录加后缀时不需要）
    pub write_when_unchanged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResult {
    pub input: String,
    /// 规划/写入的输出路径；未写入时也会给出
    pub output: Option<String>,
    pub written: bool,
    pub report: ProcessReport,
}

pub fn run_file_job(job: &FileJob) -> Result<JobResult> {
    let data = std::fs::read(&job.input)?;
    let out = crate::process(&data, &job.options)?;
    let planned = match &job.output {
        Some(p) => p.clone(),
        None => plan_output_path(&job.input, out.report.format, &job.policy, job.project_root.as_deref())?,
    };
    let skip_write = job.dry_run || (out.report.kept_original && !job.write_when_unchanged && !same_path(&planned, &job.input));
    if skip_write {
        return Ok(JobResult {
            input: display(&job.input),
            output: Some(display(&planned)),
            written: false,
            report: out.report,
        });
    }
    if out.report.kept_original && same_path(&planned, &job.input) {
        // 原地覆盖且结果就是原图：无需写入
        return Ok(JobResult { input: display(&job.input), output: Some(display(&planned)), written: false, report: out.report });
    }
    let target = resolve_conflict(planned, &job.input, &job.policy, job.on_conflict)?;
    write_atomic(&target, &out.data)?;
    Ok(JobResult { input: display(&job.input), output: Some(display(&target)), written: true, report: out.report })
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

fn display(p: &Path) -> String {
    p.display().to_string()
}
