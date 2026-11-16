//! 图片生成：接口配置的持久化、调用生成接口、结果落盘

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use imgcore::OutputFormat;
#[cfg(test)]
use imgcore::ResizeFit;
use imggen::{GenerateRequest, TaskApiConfig, TaskApiProvider};

static GEN_SEQ: AtomicU64 = AtomicU64::new(0);

const SETTINGS_FILE: &str = "generate-api.json";

/// 生成接口配置（API Key 只保存在本机应用配置目录，不进仓库）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiSettings {
    pub base_url: String,
    pub api_key: String,
    /// 是否记住到本机
    pub remember: bool,
    /// 提示词优化用的对话模型，为空时用默认模型
    pub prompt_model: String,
}

#[derive(Debug, Deserialize)]
pub struct EnhanceParams {
    pub base_url: String,
    pub api_key: String,
    #[serde(default)]
    pub model: String,
    pub prompt: String,
    #[serde(default)]
    pub context: imggen::PromptContext,
}

/// 调用对话大模型，把用户的简短描述改写成专业提示词
#[tauri::command]
pub async fn enhance_prompt(params: EnhanceParams) -> Result<imggen::EnhanceResult, String> {
    let enhancer = imggen::PromptEnhancer::new(&params.base_url, &params.api_key, &params.model).map_err(|e| e.to_string())?;
    enhancer.enhance(&params.prompt, &params.context).await.map_err(|e| e.to_string())
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(SETTINGS_FILE))
}

pub fn load_settings_from(path: &Path) -> ApiSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_settings_to(path: &Path, s: &ApiSettings) -> Result<(), String> {
    if !s.remember {
        // 不记住：删除本机保存的配置
        let _ = std::fs::remove_file(path);
        return Ok(());
    }
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 只有当前用户可读写
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_api_settings(app: AppHandle) -> Result<ApiSettings, String> {
    Ok(load_settings_from(&settings_path(&app)?))
}

#[tauri::command]
pub fn save_api_settings(app: AppHandle, settings: ApiSettings) -> Result<(), String> {
    save_settings_to(&settings_path(&app)?, &settings)
}

#[derive(Debug, Deserialize)]
pub struct GenerateParams {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub prompt: String,
    pub width: u32,
    pub height: u32,
    /// URL 或本地文件路径
    #[serde(default)]
    pub reference_images: Vec<String>,
    /// 透明背景（仅部分模型支持）
    #[serde(default)]
    pub transparent: bool,
    /// 保存格式，默认 png
    #[serde(default)]
    pub format: Option<OutputFormat>,
    /// 生成后在本地去除背景
    #[serde(default)]
    pub remove_background: Option<imgcore::BackgroundRemoval>,
    /// 生成后在本地调整到的最终尺寸（接口只支持固定几档尺寸）
    #[serde(default)]
    pub final_size: Option<FinalSize>,
}

pub use imgcore::postprocess::FinalSize;

#[derive(Debug, Serialize)]
pub struct GeneratedFile {
    pub task_id: String,
    pub path: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub source_url: Option<String>,
    pub revised_prompt: Option<String>,
    pub warnings: Vec<String>,
}

fn generated_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("generated");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub use imgcore::postprocess::Finalized as Converted;

/// 下载的原图按需去背景、调整尺寸、转换为目标格式（与 MCP 共用 imgcore::postprocess::finalize）
pub fn convert_output(
    data: Vec<u8>,
    format: OutputFormat,
    remove_background: Option<imgcore::BackgroundRemoval>,
    final_size: Option<FinalSize>,
) -> Result<Converted, String> {
    imgcore::postprocess::finalize(data, format, remove_background, final_size)
}

/// 生成一张图（一个任务）。前端按"生成数量"并发调用
#[tauri::command]
pub async fn generate_image(
    app: AppHandle,
    ref_cache: tauri::State<'_, crate::references::RefCache>,
    params: GenerateParams,
) -> Result<GeneratedFile, String> {
    // hfsyapi 只接受图片地址：本地参考图先上传到图床（有缓存，不会重复上传）
    let references = crate::references::resolve_references(&ref_cache, &params.base_url, &params.api_key, &params.reference_images).await?;
    let provider = TaskApiProvider::new(TaskApiConfig {
        base_url: params.base_url.clone(),
        api_key: params.api_key.clone(),
        model: params.model.clone(),
    })
    .map_err(|e| e.to_string())?;
    let mut extra = serde_json::Map::new();
    if params.transparent {
        extra.insert("background".into(), serde_json::Value::String("transparent".into()));
    }
    let req = GenerateRequest {
        prompt: params.prompt.clone(),
        width: Some(params.width),
        height: Some(params.height),
        count: Some(1),
        reference_images: references,
        extra: serde_json::Value::Object(extra),
        ..Default::default()
    };
    let (task_id, images) = provider.run_once(&req).await.map_err(|e| e.to_string())?;
    let img = images.into_iter().next().ok_or("没有返回图片")?;
    let format = params.format.unwrap_or(OutputFormat::Png);
    let dir = generated_dir(&app)?;
    let source_url = img.source_url.clone();
    let revised_prompt = img.revised_prompt.clone();
    let removal = params.remove_background.clone();
    let final_size = params.final_size;
    tauri::async_runtime::spawn_blocking(move || {
        let c = convert_output(img.data, format, removal, final_size)?;
        let ext = if c.ext == "jpeg" { "jpg".to_string() } else { c.ext };
        let name = format!("gen-{}-{}.{ext}", sanitize(&task_id), GEN_SEQ.fetch_add(1, Ordering::Relaxed));
        let path = dir.join(name);
        std::fs::write(&path, &c.data).map_err(|e| e.to_string())?;
        Ok(GeneratedFile {
            task_id,
            path: path.display().to_string(),
            format: ext,
            width: c.width,
            height: c.height,
            bytes: c.data.len() as u64,
            source_url,
            revised_prompt,
            warnings: c.warnings,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

fn sanitize(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').take(40).collect()
}

/// 把生成结果复制到指定目录，重名自动加序号
#[tauri::command]
pub async fn save_files(paths: Vec<String>, dir: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = PathBuf::from(dir);
        if !dir.is_dir() {
            return Err("输出目录不存在".to_string());
        }
        let mut out = Vec::new();
        for p in paths {
            let src = PathBuf::from(&p);
            let name = src.file_name().ok_or("无效文件名")?.to_owned();
            let mut target = dir.join(&name);
            let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("image").to_string();
            let ext = src.extension().and_then(|s| s.to_str()).unwrap_or("png").to_string();
            let mut i = 1;
            while target.exists() {
                target = dir.join(format!("{stem}-{i}.{ext}"));
                i += 1;
            }
            std::fs::copy(&src, &target).map_err(|e| format!("复制 {p} 失败: {e}"))?;
            out.push(target.display().to_string());
        }
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip_and_forget() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        let s = ApiSettings { base_url: "https://x".into(), api_key: "sk-1".into(), remember: true, prompt_model: "gpt-x".into() };
        save_settings_to(&path, &s).unwrap();
        let loaded = load_settings_from(&path);
        assert_eq!((loaded.api_key.as_str(), loaded.prompt_model.as_str()), ("sk-1", "gpt-x"));
        // 旧版配置文件没有 prompt_model 字段也能读
        std::fs::write(&path, r#"{"base_url":"https://x","api_key":"sk-old","remember":true}"#).unwrap();
        assert_eq!(load_settings_from(&path).api_key, "sk-old");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        save_settings_to(&path, &ApiSettings { remember: false, ..s }).unwrap();
        assert!(!path.exists());
        assert_eq!(load_settings_from(&path).api_key, "");
    }

    #[test]
    fn converts_png_to_webp_and_keeps_same_format() {
        let img = image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([x as u8 * 4, y as u8 * 5, 90]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let png = buf.into_inner();
        let same = convert_output(png.clone(), OutputFormat::Png, None, None).unwrap();
        assert_eq!((same.data.len(), same.ext.as_str(), same.width, same.height), (png.len(), "png", 64, 48));
        // 已经是目标尺寸：不重新编码
        let same_size = convert_output(png.clone(), OutputFormat::Png, None, Some(FinalSize { width: 64, height: 48, fit: ResizeFit::Cover })).unwrap();
        assert_eq!(same_size.data.len(), png.len());
        let webp = convert_output(png, OutputFormat::Webp, None, None).unwrap();
        assert_eq!(webp.ext, "webp");
        assert_eq!(&webp.data[8..12], b"WEBP");
    }

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 90]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    #[test]
    fn final_size_fits() {
        // 模拟接口返回 1254x1254，要求最终 750x400
        let src = png_bytes(1254, 1254);
        let cover = convert_output(src.clone(), OutputFormat::Png, None, Some(FinalSize { width: 750, height: 400, fit: ResizeFit::Cover })).unwrap();
        assert_eq!((cover.width, cover.height), (750, 400), "裁剪填满得到精确尺寸");
        let decoded = image::load_from_memory(&cover.data).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (750, 400));
        assert!(cover.warnings.is_empty());

        let contain = convert_output(src.clone(), OutputFormat::Png, None, Some(FinalSize { width: 750, height: 400, fit: ResizeFit::Contain })).unwrap();
        assert_eq!((contain.width, contain.height), (400, 400), "完整缩放不超过目标框");

        let fill = convert_output(src, OutputFormat::Webp, None, Some(FinalSize { width: 750, height: 400, fit: ResizeFit::Fill })).unwrap();
        assert_eq!((fill.width, fill.height, fill.ext.as_str()), (750, 400, "webp"));
    }

    #[test]
    fn warns_when_upscaling() {
        let c = convert_output(png_bytes(512, 512), OutputFormat::Png, None, Some(FinalSize { width: 1024, height: 1024, fit: ResizeFit::Cover })).unwrap();
        assert_eq!((c.width, c.height), (1024, 1024));
        assert!(c.warnings.iter().any(|w| w.contains("已放大")), "{:?}", c.warnings);
    }

    #[test]
    fn deserializes_final_size_default_fit() {
        let f: FinalSize = serde_json::from_str(r#"{"width":10,"height":20}"#).unwrap();
        assert_eq!(f.fit, ResizeFit::Cover);
    }

    #[test]
    fn remove_background_forces_alpha_format() {
        // 白底中间一个红块，要求存 JPEG → 自动改为 PNG 且四角透明
        let img = image::RgbImage::from_fn(40, 40, |x, y| if (10..30).contains(&x) && (10..30).contains(&y) { image::Rgb([200, 20, 20]) } else { image::Rgb([255, 255, 255]) });
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let c = convert_output(buf.into_inner(), OutputFormat::Jpeg, Some(imgcore::BackgroundRemoval::default()), None).unwrap();
        assert_eq!(c.ext, "png");
        let out = image::load_from_memory(&c.data).unwrap().to_rgba8();
        assert_eq!(out.get_pixel(0, 0)[3], 0);
        assert_eq!(out.get_pixel(20, 20)[3], 255);
    }
}
