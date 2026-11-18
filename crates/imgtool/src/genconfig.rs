//! MCP 侧的图片生成接口配置：优先读环境变量，其次读桌面端「设置」保存的配置文件，
//! 这样用户在桌面端填过一次 Key，Agent 就能直接用。

use std::path::PathBuf;

use serde::Deserialize;

/// 与桌面端 tauri.conf.json 的 identifier 一致
const APP_IDENTIFIER: &str = "com.imgtool.desktop";
const SETTINGS_FILE: &str = "generate-api.json";
pub const DEFAULT_BASE_URL: &str = "https://www.hfsyapi.cn";

#[derive(Debug, Clone)]
pub struct GenConfig {
    pub base_url: String,
    pub api_key: String,
    /// 本地参考图是否需要先上传到图床（hfsyapi 只接受图片地址）
    pub upload_refs: bool,
    /// 对话模型（SVG 图标生成用），空表示默认模型
    pub prompt_model: String,
    /// 配置来源，便于排查
    pub source: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct DesktopSettings {
    base_url: String,
    api_key: String,
    prompt_model: String,
}

/// 桌面端（Tauri app_config_dir）配置文件位置
pub fn desktop_settings_path() -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join(APP_IDENTIFIER).join(SETTINGS_FILE))
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn load() -> Result<GenConfig, String> {
    let file = desktop_settings_path()
        .and_then(|p| std::fs::read_to_string(&p).ok().map(|t| (p, t)))
        .and_then(|(p, t)| serde_json::from_str::<DesktopSettings>(&t).ok().map(|s| (p, s)));
    let (file_path, file_settings) = match file {
        Some((p, s)) => (Some(p), s),
        None => (None, DesktopSettings::default()),
    };
    let env_key = env("IMGTOOL_API_KEY");
    let api_key = env_key.clone().or_else(|| Some(file_settings.api_key.trim().to_string()).filter(|k| !k.is_empty()));
    let Some(api_key) = api_key else {
        return Err("还没有配置图片生成接口的 API Key：请在桌面端「设置」里填写并勾选「记住到本机」，或给 MCP 进程设置环境变量 IMGTOOL_API_KEY".into());
    };
    let base_url = env("IMGTOOL_API_BASE_URL")
        .or_else(|| Some(file_settings.base_url.trim().to_string()).filter(|u| !u.is_empty()))
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
    let upload_refs = imggen::supports_upload(&base_url) || env("IMGTOOL_REF_UPLOAD").as_deref() == Some("1");
    let source = if env_key.is_some() {
        "环境变量 IMGTOOL_API_KEY".to_string()
    } else {
        format!("桌面端设置 {}", file_path.map(|p| p.display().to_string()).unwrap_or_default())
    };
    let prompt_model = env("IMGTOOL_PROMPT_MODEL").unwrap_or_else(|| file_settings.prompt_model.trim().to_string());
    Ok(GenConfig { base_url: base_url.trim_end_matches('/').to_string(), api_key, upload_refs, prompt_model, source })
}
