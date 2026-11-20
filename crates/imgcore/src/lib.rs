//! imgcore：图片处理核心库，桌面端与 MCP Server 共用，保证两边结果一致。

pub mod background;
pub mod decode;
pub mod detect;
pub mod encode;
pub mod error;
pub mod matting;
pub mod pipeline;
pub mod portrait;
pub mod postprocess;
pub mod project;
pub mod resize;
pub mod svg;
pub mod types;

pub use background::{BackgroundRemoval, RemovalMethod, RemovalStats};
pub use decode::probe;
pub use error::{Error, Result};
pub use pipeline::process;
pub use types::*;

use std::path::Path;

/// 读取文件并处理
pub fn process_file(path: &Path, opts: &ProcessOptions) -> Result<ProcessOutput> {
    let data = std::fs::read(path)?;
    process(&data, opts)
}

/// 读取文件基础信息（只读头部）
pub fn probe_file(path: &Path) -> Result<ImageInfo> {
    use std::io::Read;
    let bytes = std::fs::metadata(path)?.len();
    let mut head = Vec::new();
    std::fs::File::open(path)?.take(512 * 1024).read_to_end(&mut head)?;
    let mut info = probe(&head)?;
    info.bytes = bytes;
    Ok(info)
}
