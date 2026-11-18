//! 用对话大模型生成一套风格统一的 SVG 线性 / 面性图标。
//!
//! 模型只负责写 SVG 文本；清洗、渲染成位图由调用方交给 `imgcore`。
//! 一次请求生成整套图标，让同一套里的线宽、圆角、视觉重量保持一致。

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::chat::ChatClient;
use crate::prompt::DEFAULT_PROMPT_MODEL;
use crate::{GenError, Result};

/// 单次最多生成的图标数（太多时模型输出会被截断，也容易超过客户端超时）
pub const MAX_ICONS: usize = 12;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IconVariant {
    /// 线性（描边）
    #[default]
    Outline,
    /// 面性（实心填充）
    Filled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IconCorner {
    /// 圆角端点与转角
    #[default]
    Round,
    /// 直角端点与转角
    Square,
}

/// 一个要生成的图标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IconSpec {
    /// 图标名称，如"首页"或 "home"
    pub label: String,
    /// 补充说明画什么，如"带烟囱的房子"
    #[serde(default)]
    pub hint: Option<String>,
}

/// 整套图标的风格
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IconStyle {
    pub variant: IconVariant,
    /// 24×24 画布上的线宽，0.5-4，默认 2
    pub stroke_width: f32,
    pub corner: IconCorner,
    /// 额外的风格要求，如"偏可爱、圆润"
    pub notes: Option<String>,
}

impl Default for IconStyle {
    fn default() -> Self {
        Self { variant: IconVariant::Outline, stroke_width: 2.0, corner: IconCorner::Round, notes: None }
    }
}

/// 模型返回的一个图标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedIcon {
    /// 与请求顺序对应，从 0 开始
    pub index: usize,
    pub label: String,
    /// 模型给的英文短名（kebab-case），可用作文件名
    pub slug: Option<String>,
    pub svg: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IconSetResult {
    pub icons: Vec<GeneratedIcon>,
    /// 没有生成出来的图标名称
    pub missing: Vec<String>,
    pub model: String,
    pub total_tokens: Option<u64>,
}

const SYSTEM_PROMPT: &str = "你是资深的 UI 图标设计师，擅长手写精简、规范的 SVG 图标。按用户给出的图标清单和风格，输出一套风格统一的 SVG 图标。\n\
硬性规则：\n\
1. 每个图标是一个独立的 <svg> 元素，按清单顺序依次输出，带 xmlns=\"http://www.w3.org/2000/svg\" 和 viewBox=\"0 0 24 24\"，不写 width/height。\n\
2. 每个 <svg> 带 data-index（清单序号，从 1 开始）和 data-name（英文 kebab-case 短名，如 shopping-cart）。\n\
3. 只用 path、circle、ellipse、rect、line、polyline、polygon、g 元素；不要 text、image、style、script、filter、渐变、clipPath、mask，也不要 class 和 style 属性。\n\
4. 颜色只能写 currentColor（不要任何具体色值），方便统一换色。\n\
5. 图形主体落在 2 到 22 的范围内，四周留边；同一套图标的视觉大小、线宽、圆角、细节密度保持一致。\n\
6. 使用业界通用的图标隐喻（如设置用齿轮、收藏用星形、消息用对话气泡），造型简洁、语义清晰，在 16px 下仍能辨认；不要多余装饰；坐标最多保留 2 位小数，路径尽量精简。\n\
7. 只输出 SVG 元素本身，不要 Markdown 代码块、注释或任何解释文字。";

/// 根据清单和风格拼出用户消息
pub fn build_user_message(icons: &[IconSpec], style: &IconStyle) -> String {
    let w = style.stroke_width;
    let cap = match style.corner {
        IconCorner::Round => "round",
        IconCorner::Square => "square",
    };
    let join = match style.corner {
        IconCorner::Round => "round",
        IconCorner::Square => "miter",
    };
    let mut msg = String::from("风格：");
    match style.variant {
        IconVariant::Outline => msg.push_str(&format!(
            "线性描边图标。根 <svg> 上写 fill=\"none\" stroke=\"currentColor\" stroke-width=\"{w}\" stroke-linecap=\"{cap}\" stroke-linejoin=\"{join}\"，子元素不要重复设置线宽，整套线宽统一为 {w}。"
        )),
        IconVariant::Filled => msg.push_str(&format!(
            "面性实心图标。根 <svg> 上写 fill=\"currentColor\" stroke=\"none\"，需要镂空的地方用 fill-rule=\"evenodd\" 或留白表现，{}转角。",
            if style.corner == IconCorner::Round { "圆角" } else { "直角" }
        )),
    }
    if let Some(n) = style.notes.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        msg.push_str(&format!("\n额外要求：{n}"));
    }
    msg.push_str("\n\n图标清单：");
    for (i, icon) in icons.iter().enumerate() {
        msg.push_str(&format!("\n{}. {}", i + 1, icon.label.trim()));
        if let Some(h) = icon.hint.as_deref().map(str::trim).filter(|h| !h.is_empty()) {
            msg.push_str(&format!("（{h}）"));
        }
    }
    msg
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!("{name}=");
    let i = tag.find(&key)? + key.len();
    let rest = &tag[i..];
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let end = rest[1..].find(quote)?;
    Some(rest[1..1 + end].trim().to_string())
}

/// 从模型输出里取出各个 <svg>…</svg>，按 data-index 对应到清单，缺失 index 时按出现顺序
pub fn parse_icons(output: &str, icons: &[IconSpec]) -> (Vec<GeneratedIcon>, Vec<String>) {
    let mut found: Vec<GeneratedIcon> = Vec::new();
    let mut rest = output;
    let mut order = 0usize;
    while let Some(start) = rest.find("<svg") {
        let Some(end) = rest[start..].find("</svg>") else { break };
        let block = &rest[start..start + end + "</svg>".len()];
        rest = &rest[start + end + "</svg>".len()..];
        let open_end = block.find('>').unwrap_or(block.len());
        let open = &block[..open_end];
        let index = attr(open, "data-index").and_then(|v| v.parse::<usize>().ok()).filter(|i| (1..=icons.len()).contains(i)).map(|i| i - 1).unwrap_or(order);
        order += 1;
        if index >= icons.len() || found.iter().any(|f| f.index == index) {
            continue;
        }
        let slug = attr(open, "data-name").map(|s| slugify(&s)).filter(|s| !s.is_empty());
        found.push(GeneratedIcon { index, label: icons[index].label.trim().to_string(), slug, svg: block.to_string() });
    }
    found.sort_by_key(|f| f.index);
    let missing = icons.iter().enumerate().filter(|(i, _)| !found.iter().any(|f| f.index == *i)).map(|(_, s)| s.label.trim().to_string()).collect();
    (found, missing)
}

fn slugify(s: &str) -> String {
    let s: String = s.trim().to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-").chars().take(40).collect()
}

/// 文件名：名称本身是英文 / 数字时直接用，否则用模型给的英文短名，再不行用 icon-序号
pub fn file_stem(icon: &GeneratedIcon) -> String {
    let label = icon.label.trim();
    if !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' ')) {
        return slugify(label);
    }
    icon.slug.clone().unwrap_or_else(|| format!("icon-{:02}", icon.index + 1))
}

pub fn validate(icons: &[IconSpec], style: &IconStyle) -> Result<()> {
    if icons.is_empty() {
        return Err(GenError::InvalidRequest("请至少填写一个图标名称".into()));
    }
    if icons.len() > MAX_ICONS {
        return Err(GenError::InvalidRequest(format!("一次最多生成 {MAX_ICONS} 个图标，请分批生成")));
    }
    if icons.iter().any(|i| i.label.trim().is_empty() || i.label.chars().count() > 40) {
        return Err(GenError::InvalidRequest("图标名称不能为空，且不超过 40 个字".into()));
    }
    if !(0.5..=4.0).contains(&style.stroke_width) {
        return Err(GenError::InvalidRequest("线宽需在 0.5-4 之间".into()));
    }
    Ok(())
}

pub struct SvgIconGenerator {
    chat: ChatClient,
}

impl SvgIconGenerator {
    /// `model` 为空时使用提示词优化的默认对话模型
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Result<Self> {
        let chat = ChatClient::new(base_url, api_key, model, DEFAULT_PROMPT_MODEL, Duration::from_secs(120))?;
        // DeepSeek 默认会长时间思考：2026-11-17 实测 8 个图标约一半请求把 6100 token 全用在推理上、
        // 没有输出；关闭思考后 6/6 成功、约 6 秒。gpt-6.1-sol 会忽略该参数
        let chat = if chat.model().to_ascii_lowercase().contains("deepseek") { chat.with_extra(serde_json::json!({ "thinking": { "type": "disabled" } })) } else { chat };
        Ok(Self { chat })
    }

    pub async fn generate(&self, icons: &[IconSpec], style: &IconStyle) -> Result<IconSetResult> {
        validate(icons, style)?;
        let max_tokens = (700 * icons.len() as u32 + 500).min(9000);
        let reply = self.chat.complete("生成 SVG 图标", SYSTEM_PROMPT, &build_user_message(icons, style), 0.3, max_tokens).await?;
        let (found, missing) = parse_icons(&reply.content, icons);
        if found.is_empty() {
            let why = if reply.finish_reason.as_deref() == Some("length") {
                format!("输出达到长度上限被截断（推理 {} token），可减少图标数量或换一个对话模型", reply.reasoning_tokens.unwrap_or(0))
            } else {
                format!("模型没有返回 SVG（{}），可换一个对话模型重试", reply.content.chars().take(80).collect::<String>())
            };
            return Err(GenError::Provider(format!("生成 SVG 图标失败: {why}")));
        }
        Ok(IconSetResult { icons: found, missing, model: reply.model, total_tokens: reply.total_tokens })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::serve_once;

    fn specs(names: &[&str]) -> Vec<IconSpec> {
        names.iter().map(|n| IconSpec { label: n.to_string(), hint: None }).collect()
    }

    #[test]
    fn builds_message_with_style() {
        let m = build_user_message(&[IconSpec { label: "首页".into(), hint: Some("带烟囱".into()) }], &IconStyle { stroke_width: 1.5, notes: Some("圆润可爱".into()), ..Default::default() });
        for part in ["stroke-width=\"1.5\"", "stroke-linecap=\"round\"", "1. 首页（带烟囱）", "圆润可爱"] {
            assert!(m.contains(part), "缺少 {part}: {m}");
        }
        let f = build_user_message(&specs(&["a"]), &IconStyle { variant: IconVariant::Filled, corner: IconCorner::Square, ..Default::default() });
        assert!(f.contains("fill=\"currentColor\"") && f.contains("直角"));
    }

    #[test]
    fn parses_by_index_and_order() {
        let out = "好的：\n```svg\n<svg data-index=\"2\" data-name=\"Shopping Cart\" viewBox=\"0 0 24 24\"><path d=\"M1 1\"/></svg>\n<svg data-index='1' viewBox=\"0 0 24 24\"><circle r=\"1\"/></svg><svg viewBox=\"0 0 24 24\"/></svg>\n```";
        let (icons, missing) = parse_icons(out, &specs(&["首页", "购物车", "我的"]));
        assert_eq!(icons.len(), 3);
        assert_eq!((icons[0].index, icons[0].label.as_str()), (0, "首页"));
        assert!(icons[0].svg.contains("circle"));
        assert_eq!(icons[1].slug.as_deref(), Some("shopping-cart"));
        assert!(missing.is_empty());
        assert_eq!(file_stem(&icons[1]), "shopping-cart");
        assert_eq!(file_stem(&icons[0]), "icon-01", "中文名且没有英文短名");
        assert_eq!(file_stem(&GeneratedIcon { index: 0, label: "Home Page".into(), slug: Some("x".into()), svg: String::new() }), "home-page");

        let (icons, missing) = parse_icons("<svg data-index=\"1\"></svg>", &specs(&["a", "b"]));
        assert_eq!((icons.len(), missing), (1, vec!["b".to_string()]));
    }

    #[test]
    fn validates_requests() {
        assert!(validate(&[], &IconStyle::default()).is_err());
        assert!(validate(&specs(&["a"; 13]), &IconStyle::default()).is_err());
        assert!(validate(&specs(&["a"]), &IconStyle { stroke_width: 9.0, ..Default::default() }).is_err());
        assert!(validate(&specs(&[" "]), &IconStyle::default()).is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn calls_chat_and_parses() {
        let (base, rx) = serve_once(
            "200 OK",
            r#"{"choices":[{"message":{"content":"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" data-index=\"1\" data-name=\"home\"><path d=\"M3 10l9-7 9 7\"/></svg>"}}],"usage":{"total_tokens":99}}"#,
        )
        .await;
        let g = SvgIconGenerator::new(&base, "sk-test", "").unwrap();
        let r = g.generate(&specs(&["首页"]), &IconStyle::default()).await.unwrap();
        assert_eq!((r.icons.len(), r.icons[0].slug.as_deref(), r.total_tokens), (1, Some("home"), Some(99)));
        let req = rx.await.unwrap();
        assert!(req.starts_with("POST /v1/chat/completions") && req.contains("首页") && req.contains(DEFAULT_PROMPT_MODEL));
    }

    /// 真实接口：IMGGEN_SVG_MODELS=a,b HFSYAPI_API_KEY=... cargo test -p imggen live_svg_icons -- --ignored --nocapture
    /// 生成的 SVG 写到 target/live-svg/<model>/，便于人工检查
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_svg_icons() {
        let key = std::env::var("HFSYAPI_API_KEY").expect("需要 HFSYAPI_API_KEY");
        let base = std::env::var("HFSYAPI_BASE_URL").unwrap_or_else(|_| "https://www.hfsyapi.cn".into());
        let models = std::env::var("IMGGEN_SVG_MODELS").unwrap_or_else(|_| DEFAULT_PROMPT_MODEL.into());
        let list = specs(&["首页", "发现", "购物车", "我的", "消息", "设置", "搜索", "收藏"]);
        for model in models.split(',') {
            let mut g = SvgIconGenerator::new(&base, &key, model.trim()).unwrap();
            if let Ok(extra) = std::env::var("IMGGEN_CHAT_EXTRA") {
                g.chat = std::mem::replace(&mut g.chat, ChatClient::new("http://x", "k", "m", "m", Duration::from_secs(1)).unwrap()).with_extra(serde_json::from_str(&extra).unwrap());
            }
            let t = std::time::Instant::now();
            match g.generate(&list, &IconStyle::default()).await {
                Ok(r) => {
                    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/live-svg").join(model.trim());
                    std::fs::create_dir_all(&dir).unwrap();
                    let bytes: usize = r.icons.iter().map(|i| i.svg.len()).sum();
                    for i in &r.icons {
                        std::fs::write(dir.join(format!("{:02}-{}.svg", i.index + 1, file_stem(i))), &i.svg).unwrap();
                    }
                    println!("### {model} | {:.1}s | tokens={:?} | {} 个 | 缺 {:?} | 共 {bytes} 字节 → {}", t.elapsed().as_secs_f32(), r.total_tokens, r.icons.len(), r.missing, dir.display());
                }
                Err(e) => println!("### {model} | {:.1}s | 失败: {e}", t.elapsed().as_secs_f32()),
            }
        }
    }
}
