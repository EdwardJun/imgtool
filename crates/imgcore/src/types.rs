use serde::{Deserialize, Serialize};

/// 可输出的编码格式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[serde(alias = "jpg")]
    Jpeg,
    Png,
    Webp,
    Avif,
}

impl OutputFormat {
    pub const ALL: [OutputFormat; 4] = [Self::Jpeg, Self::Png, Self::Webp, Self::Avif];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Avif => "avif",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Webp => "image/webp",
            Self::Avif => "image/avif",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" | "jpe" | "jfif" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "webp" => Some(Self::Webp),
            "avif" => Some(Self::Avif),
            _ => None,
        }
    }

    /// 是否支持透明通道
    pub fn supports_alpha(self) -> bool {
        !matches!(self, Self::Jpeg)
    }
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.extension())
    }
}

impl std::str::FromStr for OutputFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_extension(s).ok_or_else(|| format!("不支持的输出格式: {s}（可选 jpeg/png/webp/avif）"))
    }
}

/// 压缩方式
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompressMode {
    /// 指定质量 1-100。PNG 会映射为调色板颜色数，>=95 时走无损优化
    Quality { quality: u8 },
    /// 目标体积：先二分搜索质量，质量到下限仍超标则按比例缩小分辨率
    TargetSize {
        max_bytes: u64,
        /// 质量下限，默认 40
        #[serde(default)]
        min_quality: Option<u8>,
    },
    /// 无损：PNG 用 oxipng 重压缩、WebP 用无损模式；JPEG/AVIF 退化为质量 95
    Lossless,
}

impl Default for CompressMode {
    fn default() -> Self {
        Self::Quality { quality: 80 }
    }
}

/// 缩放时的适配方式
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ResizeFit {
    /// 等比缩放到框内（默认）
    #[default]
    Contain,
    /// 等比缩放铺满框，再居中裁掉多余部分
    Cover,
    /// 拉伸到精确尺寸
    Fill,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ResizeSpec {
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub fit: ResizeFit,
    /// 不放大小图，默认 true
    #[serde(default = "default_true")]
    pub without_enlargement: bool,
}

/// 裁剪
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CropSpec {
    /// 像素矩形（以旋正后的原图为坐标系），超出边界的部分会被截断
    Rect { x: u32, y: u32, width: u32, height: u32 },
    /// 按宽高比居中裁剪，例如 16:9
    Aspect { width: u32, height: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProcessOptions {
    /// 先裁剪
    #[serde(default)]
    pub crop: Option<CropSpec>,
    /// 再缩放
    #[serde(default)]
    pub resize: Option<ResizeSpec>,
    /// 输出格式，缺省时保持原格式（GIF/BMP/TIFF 等转为 PNG）
    #[serde(default)]
    pub format: Option<OutputFormat>,
    #[serde(default)]
    pub mode: CompressMode,
    /// 尺寸和格式都未改变且结果比原图大时，直接保留原图，默认 true
    #[serde(default = "default_true")]
    pub keep_original_if_larger: bool,
    /// 输出 JPEG 时透明区域的填充色，默认白色
    #[serde(default)]
    pub background: Option<[u8; 3]>,
}

impl Default for ProcessOptions {
    fn default() -> Self {
        Self {
            crop: None,
            resize: None,
            format: None,
            mode: CompressMode::default(),
            keep_original_if_larger: true,
            background: None,
        }
    }
}

fn default_true() -> bool {
    true
}

/// 图片基础信息（只读文件头，不完整解码）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInfo {
    /// 识别出的格式，如 jpeg/png/webp/gif/avif/heic
    pub format: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bytes: u64,
    /// 本工具能否解码处理
    pub decodable: bool,
}

/// 处理结果报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessReport {
    pub format: OutputFormat,
    pub width: u32,
    pub height: u32,
    pub original_format: String,
    pub original_width: u32,
    pub original_height: u32,
    pub original_bytes: u64,
    pub output_bytes: u64,
    /// 最终使用的质量参数（无损时为空）
    pub quality: Option<u8>,
    /// 是否直接保留了原图字节
    pub kept_original: bool,
    /// 目标体积模式下是否达标
    pub target_met: Option<bool>,
    pub warnings: Vec<String>,
}

impl ProcessReport {
    /// 体积变化比例，负数表示变小，例如 -0.42 = 减少 42%
    pub fn ratio(&self) -> f64 {
        if self.original_bytes == 0 {
            return 0.0;
        }
        self.output_bytes as f64 / self.original_bytes as f64 - 1.0
    }
}

pub struct ProcessOutput {
    pub data: Vec<u8>,
    pub report: ProcessReport,
}
