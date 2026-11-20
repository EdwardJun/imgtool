//! AI 人像抠图（MODNet）：本地 ONNX 推理，输出与原图同尺寸的透明度遮罩。
//!
//! - 推理用纯 Rust 的 tract，不依赖系统库，跨平台构建与其他代码一致。
//! - 模型文件由调用方下载后通过 [`set_model_path`] 注册；也可用环境变量
//!   `IMGTOOL_MATTING_MODEL` 指向本地文件（离线环境、镜像）。
//! - 按 MODNet 官方推理方式：长边缩到 512 左右、宽高取 32 的倍数，像素归一化到 [-1, 1]，
//!   输出 [1,1,H,W] 的 0-1 遮罩，再放大回原图尺寸。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use image::{DynamicImage, GrayImage};
use tract_onnx::prelude::*;

use crate::error::{Error, Result};

/// 环境变量：指定模型文件路径，优先于 [`set_model_path`]
pub const MODEL_ENV: &str = "IMGTOOL_MATTING_MODEL";
/// 推理分辨率（长边）
const REF_SIZE: u32 = 512;
/// 遮罩两端的噪声截断：低于下限视为背景，高于上限视为主体
const LOW: f32 = 0.03;
const HIGH: f32 = 0.97;
/// 不同输入尺寸各自优化一份模型，最多缓存几份
const MAX_CACHED: usize = 4;

/// 优化好的推理计划（tract 返回 Arc，可直接克隆共享）
type Plan = std::sync::Arc<TypedSimplePlan>;

#[derive(Default)]
struct State {
    path: Option<PathBuf>,
    plans: HashMap<(u32, u32), Plan>,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}

/// 注册模型文件路径（下载完成后调用）；路径变化时清空已加载的模型
pub fn set_model_path(path: impl Into<PathBuf>) {
    let path = path.into();
    let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
    if s.path.as_ref() != Some(&path) {
        s.path = Some(path);
        s.plans.clear();
    }
}

/// 当前生效的模型路径（环境变量优先），文件不存在时返回 None
pub fn model_path() -> Option<PathBuf> {
    let from_env = std::env::var_os(MODEL_ENV).map(PathBuf::from);
    let path = from_env.or_else(|| state().lock().unwrap_or_else(|e| e.into_inner()).path.clone())?;
    path.is_file().then_some(path)
}

/// 模型是否可用
pub fn is_available() -> bool {
    model_path().is_some()
}

fn plan_for(path: &Path, w: u32, h: u32) -> Result<Plan> {
    {
        let s = state().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = s.plans.get(&(w, h)) {
            return Ok(p.clone());
        }
    }
    let err = |e: TractError| Error::Unsupported(format!("人像抠图模型加载失败: {e}"));
    let plan = tract_onnx::onnx()
        .model_for_path(path)
        .map_err(err)?
        .with_input_fact(0, f32::fact([1, 3, h as usize, w as usize]).into())
        .map_err(err)?
        .into_optimized()
        .map_err(err)?
        .into_runnable()
        .map_err(err)?;
    let mut s = state().lock().unwrap_or_else(|e| e.into_inner());
    if s.plans.len() >= MAX_CACHED {
        s.plans.clear();
    }
    s.plans.insert((w, h), plan.clone());
    Ok(plan)
}

/// 推理尺寸：长边缩到 REF_SIZE，宽高取 32 的倍数
fn infer_size(w: u32, h: u32) -> (u32, u32) {
    let scale = REF_SIZE as f32 / w.max(h).max(1) as f32;
    let round = |v: u32| (((v as f32 * scale).round() as u32) / 32 * 32).max(32);
    (round(w), round(h))
}

/// 对整张图做人像抠图，返回与原图同尺寸的遮罩（255 = 人像，0 = 背景）
pub fn matte(img: &DynamicImage) -> Result<GrayImage> {
    let path = model_path().ok_or_else(|| Error::Unsupported("AI 人像抠图模型还没有下载，请先下载模型，或改用按颜色抠图".into()))?;
    let (w, h) = (img.width(), img.height());
    let (iw, ih) = infer_size(w, h);
    let plan = plan_for(&path, iw, ih)?;

    let small = image::imageops::resize(&img.to_rgb8(), iw, ih, image::imageops::FilterType::Triangle);
    let input: Tensor = tract_ndarray::Array4::from_shape_fn((1, 3, ih as usize, iw as usize), |(_, c, y, x)| {
        small.get_pixel(x as u32, y as u32)[c] as f32 / 127.5 - 1.0
    })
    .into();
    let out = plan.run(tvec!(input.into())).map_err(|e| Error::Unsupported(format!("人像抠图推理失败: {e}")))?;
    let view = out[0].to_plain_array_view::<f32>().map_err(|e| Error::Unsupported(format!("人像抠图输出无效: {e}")))?;
    if view.shape() != [1, 1, ih as usize, iw as usize] {
        return Err(Error::Unsupported(format!("人像抠图输出尺寸不符: {:?}", view.shape())));
    }
    let mask = GrayImage::from_fn(iw, ih, |x, y| {
        let a = ((view[[0, 0, y as usize, x as usize]] - LOW) / (HIGH - LOW)).clamp(0.0, 1.0);
        image::Luma([(a * 255.0).round() as u8])
    });
    // 遮罩是平滑的，双线性放大即可，避免 Lanczos 在边缘产生振铃
    Ok(image::imageops::resize(&mask, w, h, image::imageops::FilterType::Triangle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_size_keeps_aspect_and_multiple_of_32() {
        assert_eq!(infer_size(2048, 2800), (352, 512));
        assert_eq!(infer_size(512, 512), (512, 512));
        assert_eq!(infer_size(10, 4000), (32, 512));
    }

    /// 需要真实模型：IMGTOOL_MATTING_MODEL=/path/modnet.onnx cargo test -p imgcore matting -- --ignored
    #[test]
    #[ignore]
    fn runs_modnet_and_returns_full_size_mask() {
        assert!(is_available(), "请设置 {MODEL_ENV}");
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_fn(300, 400, |x, y| {
            let head = ((x as f32 - 150.0).powi(2) + (y as f32 - 150.0).powi(2)).sqrt() < 70.0;
            let body = y > 230 && (40..260).contains(&x);
            if head || body { image::Rgb([200, 150, 120]) } else { image::Rgb([250, 250, 250]) }
        }));
        let t = std::time::Instant::now();
        let mask = matte(&img).unwrap();
        assert_eq!(mask.dimensions(), (300, 400));
        assert!(mask.get_pixel(5, 5)[0] < 128, "角落是背景");
        // 再跑一次走缓存的推理计划
        matte(&img).unwrap();
        eprintln!("两次推理共 {:?}", t.elapsed());
    }

    #[test]
    fn missing_model_reports_clear_error() {
        if std::env::var_os(MODEL_ENV).is_some() {
            return;
        }
        set_model_path("/nonexistent/modnet.onnx");
        let err = matte(&DynamicImage::new_rgb8(8, 8)).unwrap_err().to_string();
        assert!(err.contains("还没有下载"), "{err}");
    }
}
