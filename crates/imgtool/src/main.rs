//! imgtool：命令行 + MCP Server。桌面端把它作为 sidecar 一起分发。

mod batch;
mod gen_tasks;
mod genconfig;
mod intake;
mod matting;
mod mcp;
mod options;
mod workspace;

use std::path::PathBuf;

use anyhow::{bail, Context};
use clap::{Args, Parser, Subcommand, ValueEnum};

use imgcore::project::{scan, ConflictPolicy, OutputMode, ProjectPreset, ScanOptions, PRESET_FILE};
use imgcore::{OutputFormat, ResizeFit};

use crate::batch::{expand_inputs, human_bytes, run_batch};
use crate::options::{id_photo_resize, parse_aspect, parse_color, parse_fill, parse_rect, resolve, Overrides};

#[derive(Parser)]
#[command(name = "imgtool", version, about = "图片压缩 / 裁剪 / 格式转换，支持作为 MCP Server 供 Agent 调用")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 压缩图片（可同时缩放、转格式）
    Compress(ProcessArgs),
    /// 转换格式（等同 compress --format）
    Convert(ProcessArgs),
    /// 读取图片信息
    Info {
        paths: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// 扫描目录中的图片并标记过大的文件
    Scan {
        #[arg(default_value = ".")]
        dir: PathBuf,
        #[arg(long)]
        only_flagged: bool,
        #[arg(long)]
        min_kb: Option<u64>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// 智能裁剪：识别图中的多个图标，逐个导出为正方形图片
    Split(SplitArgs),
    /// 在目录下创建 .imgtool.toml 预设模板
    Init {
        #[arg(default_value = ".")]
        dir: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// 以 stdio 方式运行 MCP Server
    Mcp {
        /// 项目根目录，可重复；不传时使用客户端提供的 Roots，再退回当前目录
        #[arg(long = "root")]
        roots: Vec<PathBuf>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum FormatArg {
    Jpeg,
    Jpg,
    Png,
    Webp,
    Avif,
}

impl From<FormatArg> for OutputFormat {
    fn from(f: FormatArg) -> Self {
        match f {
            FormatArg::Jpeg | FormatArg::Jpg => OutputFormat::Jpeg,
            FormatArg::Png => OutputFormat::Png,
            FormatArg::Webp => OutputFormat::Webp,
            FormatArg::Avif => OutputFormat::Avif,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum MethodArg {
    Color,
    Ai,
}

#[derive(Clone, Copy, ValueEnum)]
enum FitArg {
    Contain,
    Cover,
    Fill,
}

#[derive(Clone, Copy, ValueEnum)]
enum ModeArg {
    Suffix,
    Overwrite,
    Dir,
}

#[derive(Clone, Copy, ValueEnum)]
enum ConflictArg {
    Error,
    Overwrite,
    Rename,
}

#[derive(Args)]
struct SplitArgs {
    /// 包含多个图标的图片
    path: PathBuf,
    /// 把图标下方的文字标签一起裁进去（默认只保留图标）
    #[arg(long)]
    with_labels: bool,
    /// 输出边长（像素）
    #[arg(long, default_value_t = 256)]
    size: u32,
    /// 四周留白百分比 0-40
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u8).range(0..=40))]
    padding: u8,
    /// 合并间距（像素），不传则自动
    #[arg(long)]
    gap: Option<u32>,
    /// 最小尺寸（像素），不传则自动
    #[arg(long)]
    min_size: Option<u32>,
    /// 背景色，如 #ffffff；不传则自动判断（透明底或边缘颜色）
    #[arg(long)]
    bg_color: Option<String>,
    /// 与背景色的容差 0-100
    #[arg(long, default_value_t = 12, value_parser = clap::value_parser!(u8).range(0..=100))]
    tolerance: u8,
    /// 按顺序命名，逗号分隔，如 home,discover,publish
    #[arg(long, value_delimiter = ',')]
    names: Vec<String>,
    /// 输出格式
    #[arg(short, long, value_enum, default_value = "png")]
    format: FormatArg,
    /// 输出目录，默认 <原名>_icons/
    #[arg(short, long)]
    out_dir: Option<PathBuf>,
    #[arg(long, value_enum)]
    on_conflict: Option<ConflictArg>,
    /// 只识别，不写文件
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct ProcessArgs {
    /// 图片文件或目录
    #[arg(required = true)]
    paths: Vec<PathBuf>,
    /// 质量 1-100
    #[arg(short, long, value_parser = clap::value_parser!(u8).range(1..=100))]
    quality: Option<u8>,
    /// 每张图片体积上限（KB）
    #[arg(long, conflicts_with = "quality")]
    max_kb: Option<u64>,
    /// 无损压缩
    #[arg(long, conflicts_with_all = ["quality", "max_kb"])]
    lossless: bool,
    /// 最大宽度
    #[arg(long)]
    width: Option<u32>,
    /// 最大高度
    #[arg(long)]
    height: Option<u32>,
    #[arg(long, value_enum)]
    fit: Option<FitArg>,
    /// 输出格式
    #[arg(short, long, value_enum)]
    format: Option<FormatArg>,
    /// 按宽高比居中裁剪，如 16:9
    #[arg(long, conflicts_with = "rect")]
    aspect: Option<String>,
    /// 按矩形裁剪：x,y,w,h
    #[arg(long)]
    rect: Option<String>,
    /// 去除纯色背景（输出 jpg 时会改为 png 以保留透明；配合 --bg-fill 更换底色则可输出 jpg）
    #[arg(long)]
    remove_bg: bool,
    /// 更换底色：white / blue / red（证件照白、蓝、红底）或 #rrggbb，输出不透明图片
    #[arg(long, requires = "remove_bg")]
    bg_fill: Option<String>,
    /// 证件照尺寸：1inch / 2inch / small-1inch / small-2inch 或 宽x高；按主体自动构图后缩放到精确像素
    #[arg(long, requires = "remove_bg", conflicts_with_all = ["aspect", "rect", "width", "height", "trim"])]
    photo_size: Option<String>,
    /// 人像模式 true/false：保护被主体包住的同色衣物（如白衬衫）；默认在 --bg-fill / --photo-size 时开启
    #[arg(long, requires = "remove_bg")]
    bg_portrait: Option<bool>,
    /// 抠图方式：color 按颜色（默认）/ ai 用 MODNet 人像抠图模型（首次使用自动下载约 25MB）
    #[arg(long, requires = "remove_bg", value_enum)]
    bg_method: Option<MethodArg>,
    /// 背景色，如 #ffffff；不传则自动识别边缘颜色
    #[arg(long, requires = "remove_bg")]
    bg_color: Option<String>,
    /// 去背景容差 0-100（默认 12）
    #[arg(long, requires = "remove_bg", value_parser = clap::value_parser!(u8).range(0..=100))]
    bg_tolerance: Option<u8>,
    /// 边缘柔和度 0-100（默认 8）
    #[arg(long, requires = "remove_bg", value_parser = clap::value_parser!(u8).range(0..=100))]
    bg_softness: Option<u8>,
    /// 全图匹配背景色（默认只去除与边缘相连的背景）
    #[arg(long, requires = "remove_bg")]
    bg_global: bool,
    /// 去背景后裁掉四周透明空白，值为保留边距像素
    #[arg(long, requires = "remove_bg", num_args = 0..=1, default_missing_value = "0")]
    trim: Option<u32>,
    /// 输出方式
    #[arg(long, value_enum)]
    mode: Option<ModeArg>,
    /// 输出目录（隐含 --mode dir）
    #[arg(short, long)]
    out_dir: Option<String>,
    #[arg(long, value_enum)]
    on_conflict: Option<ConflictArg>,
    /// 只预览，不写文件
    #[arg(long)]
    dry_run: bool,
    /// 输出 JSON
    #[arg(long)]
    json: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Compress(args) => run_process(args, false),
        Command::Convert(args) => run_process(args, true),
        Command::Info { paths, json } => {
            let infos: Vec<_> = paths
                .iter()
                .map(|p| (p.display().to_string(), imgcore::probe_file(p).map_err(|e| e.to_string())))
                .collect();
            if json {
                let v: Vec<_> = infos
                    .iter()
                    .map(|(p, r)| match r {
                        Ok(i) => serde_json::json!({ "path": p, "info": i }),
                        Err(e) => serde_json::json!({ "path": p, "error": e }),
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                for (p, r) in infos {
                    match r {
                        Ok(i) => println!(
                            "{p}\t{}\t{}x{}\t{}",
                            i.format,
                            i.width.map(|w| w.to_string()).unwrap_or("?".into()),
                            i.height.map(|h| h.to_string()).unwrap_or("?".into()),
                            human_bytes(i.bytes)
                        ),
                        Err(e) => eprintln!("{p}\t错误: {e}"),
                    }
                }
            }
            Ok(())
        }
        Command::Scan { dir, only_flagged, min_kb, limit, json } => {
            let preset = ProjectPreset::find(&dir, None)?.map(|(_, p)| p).unwrap_or_default();
            let opts = ScanOptions { policy: preset.scan, min_bytes: min_kb.unwrap_or(0) * 1024, max_results: limit, only_flagged };
            let report = scan(&dir, &opts)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "共 {} 张（{}），需处理 {} 张（{}）",
                    report.total_images,
                    human_bytes(report.total_bytes),
                    report.flagged_images,
                    human_bytes(report.flagged_bytes)
                );
                for e in &report.images {
                    let dims = match (e.width, e.height) {
                        (Some(w), Some(h)) => format!("{w}x{h}"),
                        _ => "?".into(),
                    };
                    println!("{:>10}  {:<5} {:>11}  {}  {}", human_bytes(e.bytes), e.format, dims, e.path, e.issues.join("；"));
                }
            }
            Ok(())
        }
        Command::Split(a) => {
            let job = imgcore::project::SplitJob {
                input: a.path.clone(),
                regions: None,
                detect: imgcore::detect::DetectOptions {
                    background: a.bg_color.as_deref().map(parse_color).transpose().map_err(anyhow::Error::msg)?,
                    tolerance: a.tolerance,
                    gap: a.gap,
                    include_labels: a.with_labels,
                    min_size: a.min_size,
                },
                square: imgcore::detect::SquareOptions { size: a.size, padding: a.padding, background: None },
                names: a.names.clone(),
                out_dir: a.out_dir.clone(),
                format: a.format.into(),
                on_conflict: match a.on_conflict.unwrap_or(ConflictArg::Rename) {
                    ConflictArg::Error => ConflictPolicy::Error,
                    ConflictArg::Overwrite => ConflictPolicy::Overwrite,
                    ConflictArg::Rename => ConflictPolicy::Rename,
                },
                dry_run: a.dry_run,
            };
            let r = imgcore::project::split_file(&job)?;
            if a.json {
                println!("{}", serde_json::to_string_pretty(&r)?);
            } else {
                for f in &r.files {
                    let g = f.region;
                    println!(
                        "{} {}  ({},{} {}x{})  {}",
                        if f.written { "✓" } else { "·" },
                        f.path,
                        g.x,
                        g.y,
                        g.width,
                        g.height,
                        human_bytes(f.bytes)
                    );
                }
                println!(
                    "{}{} 个图标 → {}（{}×{}，识别方式 {}）",
                    if a.dry_run { "预计导出 " } else { "已导出 " },
                    r.files.len(),
                    r.out_dir,
                    r.size,
                    r.size,
                    r.mode
                );
            }
            Ok(())
        }
        Command::Init { dir, force } => {
            let target = dir.join(PRESET_FILE);
            if target.exists() && !force {
                bail!("{} 已存在，使用 --force 覆盖", target.display());
            }
            std::fs::write(&target, ProjectPreset::template()).with_context(|| format!("写入 {}", target.display()))?;
            println!("已创建 {}", target.display());
            Ok(())
        }
        Command::Mcp { roots } => {
            // stdout 是协议通道，日志只能写 stderr
            tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .with_ansi(false)
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_env("IMGTOOL_LOG").unwrap_or_else(|_| "info".into()),
                )
                .init();
            let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
            rt.block_on(mcp::serve_stdio(roots))
        }
    }
}

fn run_process(args: ProcessArgs, require_format: bool) -> anyhow::Result<()> {
    if require_format && args.format.is_none() {
        bail!("convert 需要 --format");
    }
    let crop = match (&args.aspect, &args.rect) {
        (Some(a), _) => Some(parse_aspect(a).map_err(anyhow::Error::msg)?),
        (_, Some(r)) => Some(parse_rect(r).map_err(anyhow::Error::msg)?),
        _ => None,
    };
    let photo_size = args.photo_size.as_deref().map(imgcore::portrait::parse_photo_size).transpose().map_err(anyhow::Error::msg)?;
    let crop = match photo_size {
        Some((width, height)) => Some(imgcore::CropSpec::Subject { width, height }),
        None => crop,
    };
    let remove_background = if args.remove_bg {
        let mut r = imgcore::BackgroundRemoval {
            color: args.bg_color.as_deref().map(parse_color).transpose().map_err(anyhow::Error::msg)?,
            contiguous: !args.bg_global,
            trim: args.trim.is_some(),
            trim_padding: args.trim.unwrap_or(0),
            fill: args.bg_fill.as_deref().map(parse_fill).transpose().map_err(anyhow::Error::msg)?,
            portrait: args.bg_portrait.unwrap_or(args.bg_fill.is_some() || photo_size.is_some()),
            method: match args.bg_method {
                Some(MethodArg::Ai) => imgcore::RemovalMethod::Ai,
                _ => imgcore::RemovalMethod::Color,
            },
            ..Default::default()
        };
        if r.method == imgcore::RemovalMethod::Ai {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
            if !imgcore::matting::is_available() && !imggen::models::is_downloaded(&imggen::models::MODNET) {
                eprintln!("首次使用 AI 抠图，正在下载模型（约 25MB）…");
            }
            rt.block_on(matting::ensure_model()).map_err(anyhow::Error::msg)?;
        }
        if let Some(t) = args.bg_tolerance {
            r.tolerance = t;
        }
        if let Some(s) = args.bg_softness {
            r.softness = s;
        }
        Some(r)
    } else {
        None
    };
    let overrides = Overrides {
        remove_background,
        quality: args.quality,
        max_kb: args.max_kb,
        lossless: args.lossless,
        max_width: args.width,
        max_height: args.height,
        fit: args.fit.map(|f| match f {
            FitArg::Contain => ResizeFit::Contain,
            FitArg::Cover => ResizeFit::Cover,
            FitArg::Fill => ResizeFit::Fill,
        }),
        format: args.format.map(Into::into),
        crop,
        // 证件照必须是精确像素：缩放到目标尺寸，限制体积时也不缩小分辨率
        resize: photo_size.map(|(w, h)| id_photo_resize(w, h)),
        keep_dimensions: photo_size.is_some(),
        output_mode: args.mode.map(|m| match m {
            ModeArg::Suffix => OutputMode::Suffix,
            ModeArg::Overwrite => OutputMode::Overwrite,
            ModeArg::Dir => OutputMode::Dir,
        }),
        out_dir: args.out_dir.clone(),
        on_conflict: args.on_conflict.map(|c| match c {
            ConflictArg::Error => ConflictPolicy::Error,
            ConflictArg::Overwrite => ConflictPolicy::Overwrite,
            ConflictArg::Rename => ConflictPolicy::Rename,
        }),
    };

    let first = args.paths[0].canonicalize().with_context(|| format!("路径不存在: {}", args.paths[0].display()))?;
    let found = ProjectPreset::find(&first, None)?;
    let project_root = match &found {
        Some((p, _)) => p.parent().map(|d| d.to_path_buf()),
        None => std::env::current_dir().ok(),
    };
    let preset = found.map(|(_, p)| p).unwrap_or_default();
    let resolved = resolve(&preset, &overrides).map_err(anyhow::Error::msg)?;

    let inputs: Vec<PathBuf> = args
        .paths
        .iter()
        .map(|p| p.canonicalize().with_context(|| format!("路径不存在: {}", p.display())))
        .collect::<Result<_, _>>()?;
    let files = expand_inputs(&inputs, &preset.scan.exclude, &resolved.policy.suffix).map_err(anyhow::Error::msg)?;
    if files.is_empty() {
        bail!("没有找到可处理的图片");
    }
    let cwd = std::env::current_dir()?;
    let display = |p: &std::path::Path| p.strip_prefix(&cwd).unwrap_or(p).display().to_string();
    let summary = run_batch(files, &resolved, project_root.as_deref(), args.dry_run, None, &display);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        for item in &summary.items {
            let flag = if item.written { "✓" } else { "·" };
            println!(
                "{flag} {} → {}  {} → {}（{}%）{}x{}{}",
                item.input,
                item.output.as_deref().unwrap_or("-"),
                human_bytes(item.original_bytes),
                human_bytes(item.output_bytes),
                item.saved_percent,
                item.width,
                item.height,
                if item.warnings.is_empty() { String::new() } else { format!("  [{}]", item.warnings.join("；")) }
            );
        }
        for e in &summary.errors {
            eprintln!("✗ {}: {}", e.input, e.error);
        }
        println!("{}", summary.text_summary());
    }
    if summary.failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
