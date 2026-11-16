//! 把 CLI / MCP 的参数叠加到项目预设上，得到最终处理参数

use imgcore::project::{ConflictPolicy, OutputMode, OutputPolicy, ProjectPreset};
use imgcore::{CompressMode, CropSpec, OutputFormat, ProcessOptions, ResizeFit, ResizeSpec};

#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub quality: Option<u8>,
    pub max_kb: Option<u64>,
    pub lossless: bool,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    pub fit: Option<ResizeFit>,
    pub format: Option<OutputFormat>,
    pub crop: Option<CropSpec>,
    pub remove_background: Option<imgcore::BackgroundRemoval>,
    pub output_mode: Option<OutputMode>,
    pub out_dir: Option<String>,
    pub on_conflict: Option<ConflictPolicy>,
}

pub struct Resolved {
    pub options: ProcessOptions,
    pub policy: OutputPolicy,
    pub on_conflict: ConflictPolicy,
}

pub fn resolve(preset: &ProjectPreset, o: &Overrides) -> Result<Resolved, String> {
    let mut options = preset.defaults.to_options();

    // 优先级：无损 > 目标体积 > 质量 > 预设
    if o.lossless {
        options.mode = CompressMode::Lossless;
    } else if let Some(kb) = o.max_kb {
        if kb == 0 {
            return Err("max_kb 必须大于 0".into());
        }
        options.mode = CompressMode::TargetSize { max_bytes: kb * 1024, min_quality: None };
    } else if let Some(q) = o.quality {
        if !(1..=100).contains(&q) {
            return Err("quality 必须在 1-100".into());
        }
        options.mode = CompressMode::Quality { quality: q };
    }

    if o.max_width.is_some() || o.max_height.is_some() {
        options.resize = Some(ResizeSpec {
            width: o.max_width,
            height: o.max_height,
            fit: o.fit.unwrap_or_default(),
            without_enlargement: true,
        });
    } else if let (Some(fit), Some(r)) = (o.fit, options.resize.as_mut()) {
        r.fit = fit;
    }
    if o.format.is_some() {
        options.format = o.format;
    }
    if o.crop.is_some() {
        options.crop = o.crop.clone();
    }
    if o.remove_background.is_some() {
        options.remove_background = o.remove_background.clone();
    }

    let mut policy = preset.output.clone();
    if let Some(m) = o.output_mode {
        policy.mode = m;
    }
    if let Some(d) = &o.out_dir {
        policy.dir = d.clone();
        if o.output_mode.is_none() {
            policy.mode = OutputMode::Dir;
        }
    }
    let on_conflict = o.on_conflict.unwrap_or(policy.on_conflict);
    Ok(Resolved { options, policy, on_conflict })
}

/// 解析 "16:9" / "16x9"
pub fn parse_aspect(s: &str) -> Result<CropSpec, String> {
    let (a, b) = s
        .split_once([':', 'x', '/'])
        .ok_or_else(|| format!("宽高比格式应为 16:9，收到 {s}"))?;
    let w: u32 = a.trim().parse().map_err(|_| format!("无效宽高比 {s}"))?;
    let h: u32 = b.trim().parse().map_err(|_| format!("无效宽高比 {s}"))?;
    if w == 0 || h == 0 {
        return Err("宽高比必须大于 0".into());
    }
    Ok(CropSpec::Aspect { width: w, height: h })
}

/// 解析 "#ffffff" / "ffffff" / "255,255,255"
pub fn parse_color(s: &str) -> Result<[u8; 3], String> {
    let t = s.trim().trim_start_matches('#');
    if t.len() == 6 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        let v = |i: usize| u8::from_str_radix(&t[i..i + 2], 16).unwrap_or(0);
        return Ok([v(0), v(2), v(4)]);
    }
    let parts: Vec<u8> = t.split(',').map(|p| p.trim().parse::<u8>()).collect::<Result<_, _>>().map_err(|_| format!("无效颜色 {s}，示例 #ffffff 或 255,255,255"))?;
    match parts[..] {
        [r, g, b] => Ok([r, g, b]),
        _ => Err(format!("无效颜色 {s}，示例 #ffffff 或 255,255,255")),
    }
}

/// 解析 "x,y,w,h"
pub fn parse_rect(s: &str) -> Result<CropSpec, String> {
    let parts: Vec<u32> = s
        .split(',')
        .map(|p| p.trim().parse::<u32>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("裁剪矩形格式应为 x,y,w,h，收到 {s}"))?;
    match parts[..] {
        [x, y, width, height] if width > 0 && height > 0 => Ok(CropSpec::Rect { x, y, width, height }),
        _ => Err(format!("裁剪矩形格式应为 x,y,w,h，收到 {s}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imgcore::project::PresetDefaults;

    #[test]
    fn explicit_quality_beats_preset_max_kb() {
        let preset = ProjectPreset { defaults: PresetDefaults { max_kb: Some(100), ..Default::default() }, ..Default::default() };
        let r = resolve(&preset, &Overrides { quality: Some(70), ..Default::default() }).unwrap();
        assert_eq!(r.options.mode, CompressMode::Quality { quality: 70 });
        let r = resolve(&preset, &Overrides::default()).unwrap();
        assert!(matches!(r.options.mode, CompressMode::TargetSize { max_bytes: 102400, .. }));
    }

    #[test]
    fn out_dir_implies_dir_mode() {
        let r = resolve(&ProjectPreset::default(), &Overrides { out_dir: Some("out".into()), ..Default::default() }).unwrap();
        assert_eq!(r.policy.mode, OutputMode::Dir);
        assert_eq!(r.policy.dir, "out");
    }

    #[test]
    fn parses_crop_args() {
        assert_eq!(parse_aspect("16:9").unwrap(), CropSpec::Aspect { width: 16, height: 9 });
        assert_eq!(parse_rect("1,2,3,4").unwrap(), CropSpec::Rect { x: 1, y: 2, width: 3, height: 4 });
        assert!(parse_rect("1,2,3").is_err());
        assert_eq!(parse_color("#FFfe00").unwrap(), [255, 254, 0]);
        assert_eq!(parse_color("1, 2,3").unwrap(), [1, 2, 3]);
        assert!(parse_color("#fff").is_err());
    }
}
