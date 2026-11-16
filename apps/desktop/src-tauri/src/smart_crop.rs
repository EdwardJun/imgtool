//! 智能裁剪：识别多个图标、批量导出为正方形

use std::path::{Path, PathBuf};

use serde::Deserialize;

use imgcore::detect::{DetectOptions, DetectResult, Region, SquareOptions};
use imgcore::project::{default_out_dir, split_file, ConflictPolicy, SplitJob, SplitResult};
use imgcore::OutputFormat;

#[tauri::command]
pub async fn detect_regions(path: String, options: DetectOptions) -> Result<DetectResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let data = std::fs::read(&path).map_err(|e| format!("读取失败: {e}"))?;
        let img = imgcore::decode::decode(&data).map_err(|e| e.to_string())?.image;
        Ok(imgcore::detect::detect_regions(&img, &options))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Deserialize)]
pub struct SplitItem {
    pub path: String,
    /// 用户在界面上确认/调整过的框；为空时按 detect 参数自动识别
    #[serde(default)]
    pub regions: Option<Vec<Region>>,
    #[serde(default)]
    pub names: Vec<String>,
}

/// 输出位置：dir 为空时写到原图旁的 `<原名>_icons/`，否则写到 `<dir>/<原名>_icons/`
pub fn out_dir_for(input: &Path, dir: Option<&str>) -> PathBuf {
    match dir {
        Some(d) if !d.is_empty() => {
            let name = default_out_dir(input).file_name().map(|n| n.to_owned()).unwrap_or_default();
            PathBuf::from(d).join(name)
        }
        _ => default_out_dir(input),
    }
}

#[tauri::command]
pub async fn export_regions(
    items: Vec<SplitItem>,
    detect: DetectOptions,
    square: SquareOptions,
    format: OutputFormat,
    dir: Option<String>,
) -> Result<Vec<Result<SplitResult, String>>, String> {
    if let Some(d) = dir.as_deref().filter(|d| !d.is_empty()) {
        if !Path::new(d).is_dir() {
            return Err("输出目录不存在".into());
        }
    }
    tauri::async_runtime::spawn_blocking(move || {
        Ok(items
            .into_iter()
            .map(|it| {
                let input = PathBuf::from(&it.path);
                let job = SplitJob {
                    out_dir: Some(out_dir_for(&input, dir.as_deref())),
                    input,
                    regions: it.regions.filter(|r| !r.is_empty()),
                    detect: detect.clone(),
                    square: square.clone(),
                    names: it.names,
                    format,
                    on_conflict: ConflictPolicy::Rename,
                    dry_run: false,
                };
                split_file(&job).map_err(|e| format!("{}: {e}", it.path))
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_dir_layout() {
        let p = Path::new("/a/b/nav.png");
        assert_eq!(out_dir_for(p, None), PathBuf::from("/a/b/nav_icons"));
        assert_eq!(out_dir_for(p, Some("")), PathBuf::from("/a/b/nav_icons"));
        assert_eq!(out_dir_for(p, Some("/out")), PathBuf::from("/out/nav_icons"));
    }

    #[test]
    fn deserializes_frontend_payload() {
        let item: SplitItem = serde_json::from_str(r#"{"path":"/a.png","regions":[{"x":1,"y":2,"width":3,"height":4,"kind":"icon_with_label"}],"names":["home"]}"#).unwrap();
        assert_eq!(item.regions.unwrap()[0].width, 3);
        let opts: DetectOptions = serde_json::from_str(r#"{"include_labels":false,"gap":null}"#).unwrap();
        assert!(!opts.include_labels && opts.tolerance == 12);
        let sq: SquareOptions = serde_json::from_str(r#"{"size":512}"#).unwrap();
        assert_eq!((sq.size, sq.padding), (512, 10));
    }
}
