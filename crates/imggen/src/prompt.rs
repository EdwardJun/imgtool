//! 提示词优化：把用户的简短描述交给对话大模型（OpenAI 兼容 `/v1/chat/completions`），
//! 结合本次生成的参数改写成专业的图像生成提示词。

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::chat::ChatClient;
use crate::{GenError, Result};

/// 默认使用的对话模型。2026-11-16 在 hfsyapi 与 gpt-6.1-sol 对比 3 个场景：质量相当，
/// 耗时约一半（5-7s），token 约 1/7（无长推理），单价也更低
pub const DEFAULT_PROMPT_MODEL: &str = "deepseek-v4.1-flash";
/// 生成接口的提示词上限
pub const PROMPT_MAX_CHARS: usize = 5000;

/// 本次生成的上下文，帮助大模型写出贴合参数的提示词
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PromptContext {
    /// 目标图像模型，如 gpt-image-2
    pub image_model: Option<String>,
    /// 宽高比，如 16:9
    pub ratio: Option<String>,
    /// 生成尺寸，如 1280x720
    pub size: Option<String>,
    /// 生成后会在本地去除背景
    pub remove_background: bool,
    /// 参考图数量
    pub reference_count: usize,
    /// 最终用途尺寸，如 750x1334
    pub final_size: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhanceResult {
    pub prompt: String,
    pub model: String,
    /// 消耗的 token（服务端返回时才有）
    pub total_tokens: Option<u64>,
}

const SYSTEM_PROMPT: &str = "你是资深的 AI 绘画提示词工程师。把用户的简短需求改写成一段可以直接交给图像生成模型的专业提示词。\n\
要求：\n\
1. 保留用户的全部意图，不要改变主体，也不要加入与需求冲突的元素；用户已写明的风格、颜色、文字内容必须保留。\n\
2. 补全画面要素：主体与动作、构图与视角、场景与背景、艺术风格或媒介、光线、色彩、材质与细节、画面氛围、画质描述。\n\
3. 画面中需要出现的文字，用引号原样写出；用户没有要求的文字一律不要加（包括标语、副标题、装饰字），并写明“画面中不出现其他文字”。\n\
4. 使用与用户相同的语言；写成一段连贯的描述，不分点，不用 Markdown，不加标题、解释或前后缀。\n\
5. 长度控制在 120 到 400 字之间（英文 80 到 250 词）。\n\
6. 生成参数只用来调整构图和画面要求，不要把像素尺寸、模型名等数字参数写进提示词。\n\
只输出提示词本身。";

/// 根据生成参数拼出额外约束
pub fn build_user_message(prompt: &str, ctx: &PromptContext) -> String {
    let mut notes = Vec::new();
    if let Some(m) = &ctx.image_model {
        notes.push(format!("目标图像模型：{m}"));
    }
    match (&ctx.ratio, &ctx.size) {
        (Some(r), Some(s)) => notes.push(format!("画面比例 {r}（{s}），构图要适配这个比例")),
        (Some(r), None) => notes.push(format!("画面比例 {r}，构图要适配这个比例")),
        (None, Some(s)) => notes.push(format!("画面尺寸 {s}")),
        _ => {}
    }
    if let Some(f) = &ctx.final_size {
        notes.push(format!("最终会裁剪/缩放到 {f}，主体放在画面中央、四周留出余量"));
    }
    if ctx.remove_background {
        notes.push("生成后会自动抠图：必须是纯白色背景（#FFFFFF），主体完整、边缘清晰，不要投影、倒影、地面或背景杂物".into());
    }
    if ctx.reference_count > 0 {
        notes.push(format!("用户提供了 {} 张参考图，提示词里要说明参考图中需要保留的内容（如主体外观、配色或风格）", ctx.reference_count));
    }
    if notes.is_empty() {
        format!("用户需求：{prompt}")
    } else {
        format!("用户需求：{prompt}\n\n生成参数（据此调整提示词，但不要把参数原样写进去）：\n- {}", notes.join("\n- "))
    }
}

/// 清理模型输出：去掉代码块、首尾引号、"提示词："之类的前缀，并限制长度
pub fn clean_output(text: &str) -> String {
    let mut s = text.trim().to_string();
    if s.starts_with("```") {
        s = s.trim_start_matches("```").trim_start_matches(|c: char| c.is_ascii_alphabetic()).to_string();
        if let Some(end) = s.rfind("```") {
            s.truncate(end);
        }
        s = s.trim().to_string();
    }
    for prefix in ["优化后的提示词：", "优化后的提示词:", "提示词：", "提示词:", "Prompt:", "prompt:"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim().to_string();
        }
    }
    let pairs = [('"', '"'), ('“', '”'), ('「', '」'), ('\'', '\'')];
    for (l, r) in pairs {
        if s.starts_with(l) && s.ends_with(r) && s.chars().count() > 2 {
            s = s.chars().skip(1).take(s.chars().count() - 2).collect::<String>().trim().to_string();
        }
    }
    if s.chars().count() > PROMPT_MAX_CHARS {
        s = s.chars().take(PROMPT_MAX_CHARS).collect();
    }
    s
}

pub struct PromptEnhancer {
    chat: ChatClient,
}

impl PromptEnhancer {
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Result<Self> {
        Ok(Self { chat: ChatClient::new(base_url, api_key, model, DEFAULT_PROMPT_MODEL, Duration::from_secs(90))? })
    }

    pub async fn enhance(&self, prompt: &str, ctx: &PromptContext) -> Result<EnhanceResult> {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Err(GenError::InvalidRequest("请先输入你想要的画面，再点优化".into()));
        }
        let reply = self.chat.complete("优化提示词", SYSTEM_PROMPT, &build_user_message(prompt, ctx), 0.7, 1500).await?;
        let cleaned = clean_output(&reply.content);
        if cleaned.is_empty() {
            return Err(GenError::Provider("优化提示词失败: 模型没有返回内容，可换一个对话模型重试".into()));
        }
        Ok(EnhanceResult { prompt: cleaned, model: reply.model, total_tokens: reply.total_tokens })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::serve_once;

    #[test]
    fn user_message_includes_context() {
        let ctx = PromptContext {
            image_model: Some("gpt-image-2".into()),
            ratio: Some("16:9".into()),
            size: Some("1280x720".into()),
            remove_background: true,
            reference_count: 2,
            final_size: Some("750x400".into()),
        };
        let m = build_user_message("一只猫", &ctx);
        assert!(m.starts_with("用户需求：一只猫"));
        for part in ["gpt-image-2", "16:9", "1280x720", "750x400", "纯白色背景", "2 张参考图"] {
            assert!(m.contains(part), "缺少 {part}: {m}");
        }
        assert_eq!(build_user_message("一只猫", &PromptContext::default()), "用户需求：一只猫");
    }

    #[test]
    fn cleans_model_output() {
        assert_eq!(clean_output("```text\n一只橘猫\n```"), "一只橘猫");
        assert_eq!(clean_output("提示词：“一只橘猫，水彩风格”"), "一只橘猫，水彩风格");
        assert_eq!(clean_output("  \"a cat\"  "), "a cat");
        assert_eq!(clean_output(&"字".repeat(6000)).chars().count(), PROMPT_MAX_CHARS);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn calls_chat_completions() {
        let (base, rx) = serve_once(
            "200 OK",
            r#"{"choices":[{"message":{"role":"assistant","content":"提示词：一只橘猫坐在窗台上，柔和晨光，扁平插画风格"}}],"usage":{"total_tokens":321}}"#,
        )
        .await;
        let e = PromptEnhancer::new(&base, "sk-test", "").unwrap();
        let r = e.enhance(" 窗台上的猫 ", &PromptContext { remove_background: true, ..Default::default() }).await.unwrap();
        assert_eq!(r.prompt, "一只橘猫坐在窗台上，柔和晨光，扁平插画风格");
        assert_eq!((r.model.as_str(), r.total_tokens), (DEFAULT_PROMPT_MODEL, Some(321)));
        let req = rx.await.unwrap();
        assert!(req.starts_with("POST /v1/chat/completions"));
        assert!(req.to_ascii_lowercase().contains("authorization: bearer sk-test"));
        assert!(req.contains("窗台上的猫") && req.contains("纯白色背景") && req.contains(&format!(r#""model":"{DEFAULT_PROMPT_MODEL}""#)));
    }

    /// 真实接口：HFSYAPI_API_KEY=sk-... cargo test -p imggen live_enhance -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_enhance() {
        let key = std::env::var("HFSYAPI_API_KEY").expect("需要 HFSYAPI_API_KEY");
        let base = std::env::var("HFSYAPI_BASE_URL").unwrap_or_else(|_| "https://www.hfsyapi.cn".into());
        let ctx = PromptContext {
            image_model: Some("gpt-image-2".into()),
            ratio: Some("1:1".into()),
            size: Some("1024x1024".into()),
            remove_background: true,
            ..Default::default()
        };
        let t = std::time::Instant::now();
        let r = PromptEnhancer::new(&base, &key, "").unwrap().enhance("一个可爱的咖啡杯图标", &ctx).await.unwrap();
        println!("{:.1}s tokens={:?} chars={}\n{}", t.elapsed().as_secs_f32(), r.total_tokens, r.prompt.chars().count(), r.prompt);
        assert!(r.prompt.contains("咖啡"));
    }

    /// 多模型对比：IMGGEN_PROMPT_MODELS=a,b HFSYAPI_API_KEY=... cargo test -p imggen live_compare -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_compare() {
        let key = std::env::var("HFSYAPI_API_KEY").expect("需要 HFSYAPI_API_KEY");
        let base = std::env::var("HFSYAPI_BASE_URL").unwrap_or_else(|_| "https://www.hfsyapi.cn".into());
        let models = std::env::var("IMGGEN_PROMPT_MODELS").unwrap_or_else(|_| DEFAULT_PROMPT_MODEL.into());
        let cases = [
            ("图标+去背景", "一个可爱的咖啡杯图标", PromptContext { image_model: Some("gpt-image-2".into()), ratio: Some("1:1".into()), size: Some("1024x1024".into()), remove_background: true, ..Default::default() }),
            ("横版海报+文字", "双十一促销海报，写上“全场五折”", PromptContext { image_model: Some("gpt-image-2.5-flare".into()), ratio: Some("16:9".into()), size: Some("2048x1152".into()), final_size: Some("1920x1080".into()), ..Default::default() }),
            ("参考图+竖版", "把参考图里的猫画成水彩风格的手机壁纸", PromptContext { image_model: Some("nano-banana-2".into()), ratio: Some("9:16".into()), size: Some("1152x2048".into()), reference_count: 1, ..Default::default() }),
        ];
        for model in models.split(',') {
            let e = PromptEnhancer::new(&base, &key, model.trim()).unwrap();
            for (name, prompt, ctx) in &cases {
                let t = std::time::Instant::now();
                match e.enhance(prompt, ctx).await {
                    Ok(r) => println!("\n### {model} | {name} | {:.1}s | tokens={:?} | {}字\n{}", t.elapsed().as_secs_f32(), r.total_tokens, r.prompt.chars().count(), r.prompt),
                    Err(err) => println!("\n### {model} | {name} | {:.1}s | 失败: {err}", t.elapsed().as_secs_f32()),
                }
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reports_errors() {
        let (base, _) = serve_once("404 Not Found", r#"{"error":{"message":"model not found"}}"#).await;
        let err = PromptEnhancer::new(&base, "k", "nope").unwrap().enhance("猫", &PromptContext::default()).await.unwrap_err().to_string();
        assert!(err.contains("model not found") && err.contains("模型名"), "{err}");

        let (base, _) = serve_once("200 OK", r#"{"choices":[{"message":{"content":"  "}}]}"#).await;
        let err = PromptEnhancer::new(&base, "k", "").unwrap().enhance("猫", &PromptContext::default()).await.unwrap_err().to_string();
        assert!(err.contains("没有返回内容"), "{err}");

        assert!(PromptEnhancer::new("https://x", "", "").is_err());
        let e = PromptEnhancer::new("https://x", "k", "").unwrap();
        assert!(e.enhance("  ", &PromptContext::default()).await.is_err());
    }
}
