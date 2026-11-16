//! hfsyapi 图片接口的尺寸表（比例 × 1K/2K/4K），与桌面端 models.ts 保持一致

/// (比例, [1K, 2K, 4K])
pub const SIZE_TABLE: &[(&str, [(u32, u32); 3])] = &[
    ("1:1", [(1024, 1024), (2048, 2048), (2880, 2880)]),
    ("5:4", [(1040, 832), (2080, 1664), (3200, 2560)]),
    ("4:3", [(1024, 768), (2048, 1536), (3264, 2448)]),
    ("3:2", [(1008, 672), (2016, 1344), (3504, 2336)]),
    ("16:9", [(1280, 720), (2048, 1152), (3840, 2160)]),
    ("21:9", [(1344, 576), (2016, 864), (3696, 1584)]),
    ("3:1", [(1008, 336), (2016, 672), (3504, 1168)]),
    ("4:5", [(832, 1040), (1664, 2080), (2560, 3200)]),
    ("3:4", [(768, 1024), (1536, 2048), (2448, 3264)]),
    ("2:3", [(672, 1008), (1344, 2016), (2336, 3504)]),
    ("9:16", [(720, 1280), (1152, 2048), (2160, 3840)]),
];

fn tier_index(tier: &str) -> Option<usize> {
    match tier.trim().to_ascii_uppercase().as_str() {
        "1K" => Some(0),
        "2K" => Some(1),
        "4K" => Some(2),
        _ => None,
    }
}

/// 比例 + 档位 → 像素尺寸
pub fn size_for(ratio: &str, tier: &str) -> Result<(u32, u32), String> {
    let row = SIZE_TABLE
        .iter()
        .find(|(r, _)| *r == ratio.trim())
        .ok_or_else(|| format!("不支持的比例 {ratio}，可选：{}", SIZE_TABLE.iter().map(|(r, _)| *r).collect::<Vec<_>>().join("、")))?;
    let i = tier_index(tier).ok_or_else(|| format!("不支持的质量档位 {tier}，可选 1K / 2K / 4K"))?;
    Ok(row.1[i])
}

/// 解析 "1024x1024"
pub fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.trim().split_once(['x', 'X', '*']).ok_or_else(|| format!("尺寸格式应为 1024x1024，收到 {s}"))?;
    let w: u32 = w.trim().parse().map_err(|_| format!("无效尺寸 {s}"))?;
    let h: u32 = h.trim().parse().map_err(|_| format!("无效尺寸 {s}"))?;
    if w == 0 || h == 0 {
        return Err("尺寸必须大于 0".into());
    }
    Ok((w, h))
}

/// 为最终尺寸挑最接近的比例（按对数距离），档位只在 allowed 里选能覆盖长边的最低档
pub fn match_ratio(width: u32, height: u32, tier: &str) -> (&'static str, (u32, u32)) {
    let target = (width as f64 / height.max(1) as f64).ln();
    let (ratio, row) = SIZE_TABLE
        .iter()
        .min_by(|(a, _), (b, _)| {
            let d = |r: &str| {
                let (x, y) = r.split_once(':').unwrap();
                ((x.parse::<f64>().unwrap() / y.parse::<f64>().unwrap()).ln() - target).abs()
            };
            d(a).total_cmp(&d(b))
        })
        .expect("尺寸表非空");
    let i = tier_index(tier).unwrap_or(0);
    (ratio, row[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups() {
        assert_eq!(size_for("16:9", "2k").unwrap(), (2048, 1152));
        assert!(size_for("7:5", "1K").is_err());
        assert!(size_for("1:1", "8K").is_err());
        assert_eq!(parse_size("750x1334").unwrap(), (750, 1334));
        assert!(parse_size("750").is_err());
        assert_eq!(match_ratio(750, 1334, "1K"), ("9:16", (720, 1280)));
        assert_eq!(match_ratio(1920, 1080, "2K").0, "16:9");
        assert_eq!(match_ratio(512, 512, "1K").1, (1024, 1024));
    }
}
