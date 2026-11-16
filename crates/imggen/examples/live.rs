//! 真实接口联调：
//! IMGGEN_BASE_URL=https://www.hfsyapi.cn IMGGEN_API_KEY=sk-... \
//!   cargo run -p imggen --example live -- gpt-image-2 1024x1024 "机械小狗" [参考图URL或本地路径...]
//! 结果保存到当前目录 live-<task>.png。API Key 只从环境变量读取，不要写进代码。

use imggen::{GenerateRequest, TaskApiConfig, TaskApiProvider};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let model = args.first().cloned().unwrap_or_else(|| "gpt-image-2".into());
    let size = args.get(1).cloned().unwrap_or_else(|| "1024x1024".into());
    let prompt = args.get(2).cloned().unwrap_or_else(|| "一只橘猫坐在窗台上，简洁插画风格".into());
    let refs: Vec<String> = args.iter().skip(3).cloned().collect();
    let (w, h) = size.split_once('x').expect("size 格式应为 1024x1024");

    let provider = TaskApiProvider::new(TaskApiConfig {
        base_url: std::env::var("IMGGEN_BASE_URL").unwrap_or_else(|_| "https://www.hfsyapi.cn".into()),
        api_key: std::env::var("IMGGEN_API_KEY").expect("请设置 IMGGEN_API_KEY"),
        model: model.clone(),
    })
    .expect("配置无效");
    // 额外参数，如 IMGGEN_EXTRA='{"background":"transparent"}'
    let extra = std::env::var("IMGGEN_EXTRA")
        .ok()
        .map(|s| serde_json::from_str(&s).expect("IMGGEN_EXTRA 不是合法 JSON"))
        .unwrap_or(serde_json::Value::Null);
    let req = GenerateRequest {
        prompt,
        width: w.parse().ok(),
        height: h.parse().ok(),
        reference_images: refs,
        extra,
        ..Default::default()
    };
    let start = std::time::Instant::now();
    match provider.run_once(&req).await {
        Ok((id, images)) => {
            for (i, img) in images.iter().enumerate() {
                let file = format!("live-{id}-{i}.png");
                std::fs::write(&file, &img.data).unwrap();
                println!(
                    "OK model={model} task={id} {:.1}s bytes={} mime={} url={} -> {file}",
                    start.elapsed().as_secs_f32(),
                    img.data.len(),
                    img.mime,
                    img.source_url.as_deref().unwrap_or("-")
                );
            }
        }
        Err(e) => {
            eprintln!("FAIL model={model} {:.1}s: {e}", start.elapsed().as_secs_f32());
            std::process::exit(1);
        }
    }
}
