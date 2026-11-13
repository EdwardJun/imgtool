#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("无法识别的图片格式")]
    UnknownFormat,
    #[error("暂不支持处理该格式: {0}")]
    Unsupported(String),
    #[error("解码失败: {0}")]
    Decode(String),
    #[error("编码失败: {0}")]
    Encode(String),
    #[error("参数无效: {0}")]
    InvalidArgument(String),
    #[error("预设文件 {path} 解析失败: {message}")]
    Preset { path: String, message: String },
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
