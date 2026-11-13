//! 输出路径规划与安全写入

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::project::preset::{ConflictPolicy, OutputMode, OutputPolicy};
use crate::types::OutputFormat;

/// 根据输出策略计算目标路径（不处理冲突）
pub fn plan_output_path(
    input: &Path,
    format: OutputFormat,
    policy: &OutputPolicy,
    project_root: Option<&Path>,
) -> Result<PathBuf> {
    let parent = input.parent().unwrap_or_else(|| Path::new("."));
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::InvalidArgument(format!("无效文件名: {}", input.display())))?;
    let in_ext = input.extension().and_then(|e| e.to_str()).unwrap_or("");
    let format_changed = OutputFormat::from_extension(in_ext) != Some(format);
    // 格式不变时保留原扩展名写法（如 .jpeg / .JPG）
    let ext = if format_changed { format.extension() } else { in_ext };

    let path = match policy.mode {
        OutputMode::Suffix => {
            if format_changed {
                parent.join(format!("{stem}.{ext}"))
            } else {
                parent.join(format!("{stem}{}.{ext}", policy.suffix))
            }
        }
        OutputMode::Overwrite => parent.join(format!("{stem}.{ext}")),
        OutputMode::Dir => {
            let out_dir = match project_root {
                Some(root) => root.join(&policy.dir),
                None => parent.join(&policy.dir),
            };
            let rel_parent = project_root
                .and_then(|root| parent.strip_prefix(root).ok())
                .unwrap_or_else(|| Path::new(""));
            out_dir.join(rel_parent).join(format!("{stem}.{ext}"))
        }
    };
    Ok(path)
}

/// 处理目标文件冲突，返回最终可写入的路径
pub fn resolve_conflict(target: PathBuf, input: &Path, policy: &OutputPolicy, on_conflict: ConflictPolicy) -> Result<PathBuf> {
    let is_input = same_file(&target, input);
    if is_input {
        // 只有显式选择覆盖模式才允许写回原文件
        if policy.mode == OutputMode::Overwrite {
            return Ok(target);
        }
        return Err(Error::InvalidArgument(format!("输出路径与原图相同: {}", target.display())));
    }
    if !target.exists() {
        return Ok(target);
    }
    match on_conflict {
        ConflictPolicy::Overwrite => Ok(target),
        ConflictPolicy::Error => Err(Error::InvalidArgument(format!(
            "目标文件已存在: {}（可设置 on_conflict 为 overwrite 或 rename）",
            target.display()
        ))),
        ConflictPolicy::Rename => {
            let parent = target.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
            let stem = target.file_stem().and_then(|s| s.to_str()).unwrap_or("image").to_string();
            let ext = target.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
            for i in 1..10_000 {
                let candidate = parent.join(format!("{stem}-{i}.{ext}"));
                if !candidate.exists() {
                    return Ok(candidate);
                }
            }
            Err(Error::InvalidArgument("无法生成不冲突的文件名".into()))
        }
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// 先写临时文件再重命名，避免中途失败留下半个文件
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("out");
    let tmp = parent.join(format!(".{file_name}.imgtool-tmp"));
    std::fs::write(&tmp, data)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(mode: OutputMode) -> OutputPolicy {
        OutputPolicy { mode, ..Default::default() }
    }

    #[test]
    fn suffix_naming() {
        let p = Path::new("/p/assets/photo.JPG");
        assert_eq!(
            plan_output_path(p, OutputFormat::Jpeg, &policy(OutputMode::Suffix), None).unwrap(),
            PathBuf::from("/p/assets/photo.min.JPG")
        );
        assert_eq!(
            plan_output_path(p, OutputFormat::Webp, &policy(OutputMode::Suffix), None).unwrap(),
            PathBuf::from("/p/assets/photo.webp")
        );
    }

    #[test]
    fn dir_mode_keeps_structure() {
        let p = Path::new("/p/src/img/a.png");
        assert_eq!(
            plan_output_path(p, OutputFormat::Png, &policy(OutputMode::Dir), Some(Path::new("/p"))).unwrap(),
            PathBuf::from("/p/.imgtool/out/src/img/a.png")
        );
    }

    #[test]
    fn conflict_policies() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.png");
        let target = dir.path().join("a.min.png");
        std::fs::write(&input, b"x").unwrap();
        std::fs::write(&target, b"y").unwrap();
        let pol = policy(OutputMode::Suffix);
        assert!(resolve_conflict(target.clone(), &input, &pol, ConflictPolicy::Error).is_err());
        assert_eq!(resolve_conflict(target.clone(), &input, &pol, ConflictPolicy::Overwrite).unwrap(), target);
        assert_eq!(
            resolve_conflict(target.clone(), &input, &pol, ConflictPolicy::Rename).unwrap(),
            dir.path().join("a.min-1.png")
        );
        // 非覆盖模式下禁止写回原图
        assert!(resolve_conflict(input.clone(), &input, &pol, ConflictPolicy::Overwrite).is_err());
    }
}
