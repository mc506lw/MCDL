//! `mcdl` 命令行入口。GUI 里的每条命令在这里都能找到对应的子命令，
//! 方便在不启动窗口的情况下验证引擎。

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use mcdl_core::types::{LogLevel, Progress, ProgressSink, Source};
use mcdl_core::{InstallRequest, Installer, LoaderSpec};

#[derive(Parser)]
#[command(name = "mcdl", version, about = "Minecraft 下载与安装工具")]
struct Cli {
    /// 游戏目录，默认是当前目录下的 .minecraft
    #[arg(long, global = true, default_value = ".minecraft")]
    dir: PathBuf,

    /// 下载源：official（默认）或 bmclapi
    #[arg(long, global = true, default_value = "official")]
    source: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 列出所有可安装的版本
    Versions {
        /// 只看正式版
        #[arg(long)]
        release_only: bool,
    },
    /// 安装一个版本
    Install {
        /// 版本号，例如 1.21.1
        version: String,
        /// 加载器，例如 fabric、fabric@0.16.0、forge、neoforge
        #[arg(long)]
        loader: Option<String>,
        /// 并发下载数
        #[arg(long, default_value_t = 16)]
        concurrency: usize,
        /// 跳过资源文件（只装能启动的最小集合）
        #[arg(long)]
        no_assets: bool,
        /// 指定 java 可执行文件
        #[arg(long)]
        java: Option<PathBuf>,
    },
}

/// 把引擎的进度事件打到终端。
///
/// 进度只在整数百分比变化时重画，否则 3000 个资源文件会把终端刷爆。
struct StdoutSink {
    last_pct: std::sync::atomic::AtomicUsize,
}

impl StdoutSink {
    fn new() -> Self {
        Self {
            last_pct: std::sync::atomic::AtomicUsize::new(usize::MAX),
        }
    }
}

impl ProgressSink for StdoutSink {
    fn emit(&self, progress: Progress) {
        use std::sync::atomic::Ordering;

        match progress {
            Progress::Stage {
                index, total, label, ..
            } => {
                self.last_pct.store(usize::MAX, Ordering::Relaxed);
                println!("\n[{index}/{total}] {label}");
            }
            Progress::Files {
                done,
                total,
                bytes_done,
                bytes_total,
                ..
            } => {
                let pct = if bytes_total > 0 {
                    bytes_done as f64 / bytes_total as f64 * 100.0
                } else if total > 0 {
                    done as f64 / total as f64 * 100.0
                } else {
                    100.0
                };
                let whole = pct.floor() as usize;
                let previous = self.last_pct.swap(whole, Ordering::Relaxed);
                if previous == whole && done != total {
                    return;
                }
                print!(
                    "\r  {:>5.1}%  {done}/{total} 个文件  {:.1}/{:.1} MB   ",
                    pct,
                    bytes_done as f64 / 1_048_576.0,
                    bytes_total as f64 / 1_048_576.0
                );
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
            Progress::Log { level, message } => {
                let tag = match level {
                    LogLevel::Info => "info",
                    LogLevel::Warn => "warn",
                    LogLevel::Error => "err ",
                };
                println!("\n  {tag}  {message}");
            }
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mcdl=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let source = match cli.source.to_ascii_lowercase().as_str() {
        "bmclapi" | "mirror" => Source::Bmclapi,
        "official" | "mojang" => Source::Official,
        other => {
            eprintln!("警告：不认识的下载源 `{other}`，已回退到官方源（可选：official / bmclapi）");
            Source::Official
        }
    };

    // 只打一次错误：anyhow 默认会再打一串 "Caused by"，和引擎自己的错误文案重复。
    if let Err(e) = run(cli, source).await {
        eprintln!("\n错误：{e}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli, source: Source) -> anyhow::Result<()> {
    match cli.command {
        Command::Versions { release_only } => {
            let client = mcdl_core::net::HttpClient::new()?;
            let manifest = mcdl_core::meta::fetch_manifest(&client, source).await?;
            println!("最新正式版：{}", manifest.latest.release);
            println!("最新快照：  {}", manifest.latest.snapshot);
            println!();
            for v in manifest
                .versions
                .iter()
                .filter(|v| !release_only || v.kind == "release")
            {
                println!("{:<16} {:<10} {}", v.id, v.kind, v.release_time);
            }
        }
        Command::Install {
            version,
            loader,
            concurrency,
            no_assets,
            java,
        } => {
            let mut req = InstallRequest::new(&cli.dir, &version);
            req.source = source;
            req.concurrency = concurrency;
            req.skip_assets = no_assets;
            req.java = java;
            if let Some(raw) = loader {
                req.loader = Some(LoaderSpec::parse(&raw, &version)?);
            }

            let installer = Installer::new()?;
            let sink = StdoutSink::new();
            let report = installer.run(&req, &sink).await?;
            println!("\n\n安装完成");
            println!("  版本目录：{}", report.installed_version);
            println!("  游戏目录：{}", report.game_dir.display());
            println!(
                "  新增文件：{} 个，共 {:.1} MB，用时 {:.1}s",
                report.files,
                report.bytes as f64 / 1_048_576.0,
                report.duration_ms as f64 / 1000.0
            );
            if let Some(j) = report.java {
                println!("  Java：    {}", j.display());
            }
        }
    }

    Ok(())
}
