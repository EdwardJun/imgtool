//! 项目图片扫描：遵守 .gitignore，列出图片体积/尺寸并标记问题

use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::decode::probe;
use crate::error::{Error, Result};
use crate::project::preset::ScanPolicy;

pub const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff", "ico", "avif", "heic", "heif", "svg",
];

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub policy: ScanPolicy,
    /// 只返回体积不小于该值的图片
    pub min_bytes: u64,
    pub max_results: usize,
    /// 只返回有问题的图片
    pub only_flagged: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self { policy: ScanPolicy::default(), min_bytes: 0, max_results: 200, only_flagged: false }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanEntry {
    /// 相对扫描根目录的路径（统一用 /）
    pub path: String,
    pub bytes: u64,
    pub format: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    pub root: String,
    pub total_images: usize,
    pub total_bytes: u64,
    pub flagged_images: usize,
    pub flagged_bytes: u64,
    /// 是否因 max_results 截断
    pub truncated: bool,
    /// 按体积从大到小排序
    pub images: Vec<ScanEntry>,
}

fn build_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        b.add(Glob::new(p).map_err(|e| Error::InvalidArgument(format!("无效 glob {p}: {e}")))?);
    }
    b.build().map_err(|e| Error::InvalidArgument(e.to_string()))
}

pub fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// 遍历 dir 下的图片文件（遵守 .gitignore 与 exclude），返回绝对路径
pub fn walk_images(dir: &Path, exclude: &[String]) -> Result<Vec<PathBuf>> {
    let excludes = build_globset(exclude)?;
    let mut out = Vec::new();
    let walker = ignore::WalkBuilder::new(dir)
        .hidden(true)
        .git_ignore(true)
        .git_global(false)
        .require_git(false)
        .build();
    for entry in walker.flatten() {
        let path = entry.path();
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) || !is_image_path(path) {
            continue;
        }
        let rel = path.strip_prefix(dir).unwrap_or(path);
        if excludes.is_match(rel) {
            continue;
        }
        out.push(path.to_path_buf());
    }
    Ok(out)
}

pub fn scan(dir: &Path, opts: &ScanOptions) -> Result<ScanReport> {
    let files = walk_images(dir, &opts.policy.exclude)?;
    let warn_bytes = opts.policy.warn_kb * 1024;
    let mut entries = Vec::new();
    let (mut total_bytes, mut flagged_images, mut flagged_bytes) = (0u64, 0usize, 0u64);

    for path in &files {
        let Ok(meta) = std::fs::metadata(path) else { continue };
        let bytes = meta.len();
        total_bytes += bytes;
        // 只读取文件头部即可得到格式与尺寸
        let head = read_head(path, 512 * 1024).unwrap_or_default();
        let (format, width, height, decodable) = match probe(&head) {
            Ok(info) => (info.format, info.width, info.height, info.decodable),
            Err(_) => ("unknown".to_string(), None, None, false),
        };

        let mut issues = Vec::new();
        if bytes > warn_bytes {
            issues.push(format!("体积 {} KB 超过 {} KB", bytes / 1024, opts.policy.warn_kb));
        }
        if let (Some(w), Some(h)) = (width, height) {
            if w.max(h) > opts.policy.warn_dimension {
                issues.push(format!("尺寸 {w}x{h} 超过 {}px", opts.policy.warn_dimension));
            }
        }
        if decodable && matches!(format.as_str(), "bmp" | "tiff") {
            issues.push(format!("{format} 未压缩，建议转为 webp/png"));
        }
        if !issues.is_empty() {
            flagged_images += 1;
            flagged_bytes += bytes;
        }
        if bytes < opts.min_bytes || (opts.only_flagged && issues.is_empty()) {
            continue;
        }
        let rel = path.strip_prefix(dir).unwrap_or(path);
        entries.push(ScanEntry {
            path: rel.to_string_lossy().replace('\\', "/"),
            bytes,
            format,
            width,
            height,
            issues,
        });
    }

    entries.sort_by_key(|e| std::cmp::Reverse(e.bytes));
    let truncated = entries.len() > opts.max_results;
    entries.truncate(opts.max_results);
    Ok(ScanReport {
        root: dir.display().to_string(),
        total_images: files.len(),
        total_bytes,
        flagged_images,
        flagged_bytes,
        truncated,
        images: entries,
    })
}

fn read_head(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::new();
    std::fs::File::open(path)?.take(limit).read_to_end(&mut buf)?;
    Ok(buf)
}
