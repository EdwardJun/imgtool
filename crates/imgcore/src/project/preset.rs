//! 项目预设 `.imgtool.toml`：桌面端与 MCP 共用，保证团队处理结果一致。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::types::{CompressMode, OutputFormat, ProcessOptions, ResizeFit, ResizeSpec};

pub const PRESET_FILE: &str = ".imgtool.toml";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectPreset {
    pub defaults: PresetDefaults,
    pub output: OutputPolicy,
    pub scan: ScanPolicy,
    pub spec: SpecPolicy,
}

/// 设计稿解析（imgspec）的项目级设置
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpecPolicy {
    /// 图标素材目录（相对项目根目录），如 "src/assets/icons"；不填则放在 .imgtool/specs/<相对目录>/<原名>/icons/
    pub icons_dir: Option<String>,
    /// 图标素材边长（像素）
    pub icon_size: u32,
}

impl Default for SpecPolicy {
    fn default() -> Self {
        Self { icons_dir: None, icon_size: 256 }
    }
}

/// 默认处理参数
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PresetDefaults {
    /// 默认输出格式，缺省保持原格式
    pub format: Option<OutputFormat>,
    /// 默认质量 1-100
    pub quality: Option<u8>,
    /// 单张体积上限（KB），设置后默认走目标体积模式
    pub max_kb: Option<u64>,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
}

impl PresetDefaults {
    /// 转为处理参数；未配置的字段使用内置默认值
    pub fn to_options(&self) -> ProcessOptions {
        let mode = match (self.max_kb, self.quality) {
            (Some(kb), _) => CompressMode::TargetSize { max_bytes: kb * 1024, min_quality: None, keep_dimensions: false },
            (None, Some(q)) => CompressMode::Quality { quality: q },
            (None, None) => CompressMode::default(),
        };
        let resize = (self.max_width.is_some() || self.max_height.is_some()).then_some(ResizeSpec {
            width: self.max_width,
            height: self.max_height,
            fit: ResizeFit::Contain,
            without_enlargement: true,
        });
        ProcessOptions { format: self.format, mode, resize, ..Default::default() }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    /// 同目录新文件：压缩为 name.min.ext，转格式为 name.<新扩展名>（默认）
    #[default]
    Suffix,
    /// 原地覆盖（格式改变时写到新扩展名，原文件保留）
    Overwrite,
    /// 统一写到输出目录，保持相对目录结构
    Dir,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// 目标文件已存在时报错（默认，Agent 场景下最安全）
    #[default]
    Error,
    /// 覆盖已存在的目标文件
    Overwrite,
    /// 自动追加序号，如 name.min-1.jpg
    Rename,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputPolicy {
    pub mode: OutputMode,
    /// suffix 模式下压缩产物的后缀
    pub suffix: String,
    /// dir 模式下的输出目录（相对项目根目录）
    pub dir: String,
    pub on_conflict: ConflictPolicy,
}

impl Default for OutputPolicy {
    fn default() -> Self {
        Self {
            mode: OutputMode::Suffix,
            suffix: ".min".into(),
            dir: ".imgtool/out".into(),
            on_conflict: ConflictPolicy::Error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScanPolicy {
    /// 额外排除的 glob（.gitignore 已自动生效）
    pub exclude: Vec<String>,
    /// 超过该体积（KB）标记为过大
    pub warn_kb: u64,
    /// 宽或高超过该像素数标记为尺寸过大
    pub warn_dimension: u32,
}

impl Default for ScanPolicy {
    fn default() -> Self {
        Self { exclude: Vec::new(), warn_kb: 500, warn_dimension: 2560 }
    }
}

impl ProjectPreset {
    pub fn parse(text: &str, path: &Path) -> Result<Self> {
        let preset: Self = toml::from_str(text).map_err(|e| Error::Preset {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        if !(16..=2048).contains(&preset.spec.icon_size) {
            return Err(Error::Preset { path: path.display().to_string(), message: "spec.icon_size 必须在 16-2048".into() });
        }
        if let Some(d) = &preset.spec.icons_dir {
            let p = Path::new(d);
            if p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
                return Err(Error::Preset { path: path.display().to_string(), message: "spec.icons_dir 必须是项目内的相对路径".into() });
            }
        }
        if let Some(q) = preset.defaults.quality {
            if !(1..=100).contains(&q) {
                return Err(Error::Preset { path: path.display().to_string(), message: "quality 必须在 1-100".into() });
            }
        }
        Ok(preset)
    }

    pub fn load(path: &Path) -> Result<Self> {
        Self::parse(&std::fs::read_to_string(path)?, path)
    }

    /// 从 start 开始逐级向上查找 `.imgtool.toml`，最多到 stop_at（含）为止
    pub fn find(start: &Path, stop_at: Option<&Path>) -> Result<Option<(PathBuf, Self)>> {
        let mut dir = if start.is_dir() { Some(start) } else { start.parent() };
        while let Some(d) = dir {
            let candidate = d.join(PRESET_FILE);
            if candidate.is_file() {
                return Ok(Some((candidate.clone(), Self::load(&candidate)?)));
            }
            if stop_at.map(|s| d == s).unwrap_or(false) {
                break;
            }
            dir = d.parent();
        }
        Ok(None)
    }

    /// 生成带注释的示例预设
    pub fn template() -> &'static str {
        include_str!("preset_template.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses() {
        let p = ProjectPreset::parse(ProjectPreset::template(), Path::new("t")).unwrap();
        assert_eq!(p.output.mode, OutputMode::Suffix);
        assert_eq!(p.defaults.quality, Some(80));
    }

    #[test]
    fn max_kb_switches_to_target_mode() {
        let d = PresetDefaults { max_kb: Some(200), max_width: Some(1920), ..Default::default() };
        let o = d.to_options();
        assert_eq!(o.mode, CompressMode::TargetSize { max_bytes: 200 * 1024, min_quality: None, keep_dimensions: false });
        assert_eq!(o.resize.unwrap().width, Some(1920));
    }

    #[test]
    fn spec_section() {
        let p = ProjectPreset::parse("[spec]\nicons_dir = \"src/assets/icons\"\nicon_size = 128", Path::new("t")).unwrap();
        assert_eq!(p.spec.icons_dir.as_deref(), Some("src/assets/icons"));
        assert_eq!(p.spec.icon_size, 128);
        assert_eq!(ProjectPreset::default().spec.icon_size, 256);
        assert!(ProjectPreset::parse("[spec]\nicons_dir = \"../x\"", Path::new("t")).is_err());
        assert!(ProjectPreset::parse("[spec]\nicon_size = 4", Path::new("t")).is_err());
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(ProjectPreset::parse("[defaults]\nqualty = 3", Path::new("t")).is_err());
    }
}
