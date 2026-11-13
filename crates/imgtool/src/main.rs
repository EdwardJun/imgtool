//! imgtool：命令行 + MCP Server。桌面端把它作为 sidecar 一起分发。

mod batch;
mod mcp;
mod options;
mod workspace;

use std::path::PathBuf;

use anyhow::{bail, Context};
use clap::{Args, Parser, Subcommand, ValueEnum};

use imgcore::project::{scan, ConflictPolicy, OutputMode, ProjectPreset, ScanOptions, PRESET_FILE};
use imgcore::{OutputFormat, ResizeFit};

use crate::batch::{expand_inputs, human_bytes, run_batch};
use crate::options::{parse_aspect, parse_rect, resolve, Overrides};

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
    let overrides = Overrides {
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
