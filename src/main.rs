mod config;
mod events;
mod fetcher;
mod lrc_parser;
mod mpris;
mod overlay;
mod title_parser;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("run") | None => {
            let cfg = config::Config::load_or_init()?;
            maybe_force_xwayland();
            tracing::info!("wayland-float-lyrics 完整模式启动");
            overlay::run_full(cfg)
        }
        Some("overlay") => {
            let cfg = config::Config::load_or_init()?;
            maybe_force_xwayland();
            overlay::run_demo(cfg)
        }
        Some("config") => {
            let cfg = config::Config::load_or_init()?;
            println!("config file: {}", config::Config::path()?.display());
            println!("{}", toml::to_string_pretty(&cfg)?);
            Ok(())
        }
        Some("fetch") => {
            let artist = args.get(2).map(String::as_str).unwrap_or("").to_string();
            let title = args.get(3).map(String::as_str).unwrap_or("").to_string();
            if title.is_empty() {
                eprintln!("用法: wayland-float-lyrics fetch <artist> <title>");
                std::process::exit(2);
            }
            runtime()?.block_on(cli_fetch(&artist, &title))
        }
        Some("mpris") => {
            tracing::info!("MPRIS CLI 监听模式（诊断用）");
            runtime()?.block_on(mpris::run_cli())
        }
        Some("--help") | Some("-h") | Some("help") => {
            print_help();
            Ok(())
        }
        Some(other) => {
            eprintln!("未知子命令: {other}");
            print_help();
            std::process::exit(2);
        }
    }
}

fn print_help() {
    println!("wayland-float-lyrics — Wayland/GNOME 桌面浮动歌词\n");
    println!("用法:");
    println!("  wayland-float-lyrics [run]               默认：完整模式（MPRIS + 浮层）");
    println!("  wayland-float-lyrics overlay             仅 GUI demo，循环切换示例文案");
    println!("  wayland-float-lyrics mpris               纯 MPRIS 诊断输出（stdout）");
    println!("  wayland-float-lyrics fetch ARTIST TITLE  手动测试歌词获取");
    println!("  wayland-float-lyrics config              打印当前配置与路径");
    println!("\n环境变量（覆盖 config）:");
    println!("  WFL_MONITOR=<1-based>     指定显示器（0=跟随鼠标）");
    println!("  WFL_MARGIN_BOTTOM=<px>    距屏幕底部像素");
    println!("  RUST_LOG=wayland_float_lyrics=debug  打开调试日志");
}

/// GNOME Wayland 不实现 wlr-layer-shell，浮层无法锚定底部。
/// 启用 x11-dock feature 时自动切回 XWayland，走 EWMH dock hint 路径。
fn maybe_force_xwayland() {
    #[cfg(feature = "x11-dock")]
    {
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        if session == "wayland" && desktop.to_uppercase().contains("GNOME") {
            std::env::set_var("GDK_BACKEND", "x11");
            // XWayland dock 在多屏拖拽 / 合成器 grab / 焦点切换后，mutter 会停掉
            // 本 surface 的 frame callback，GTK4 默认等 callback 驱动重绘 → 字幕卡死。
            // no-vsync 让 GDK 按定时器直接提交 buffer，绕过 frame callback 依赖。
            if std::env::var_os("GDK_DEBUG").is_none() {
                std::env::set_var("GDK_DEBUG", "no-vsync");
            }
            tracing::info!("检测到 GNOME Wayland，强制 GDK_BACKEND=x11 + GDK_DEBUG=no-vsync");
        }
    }
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?)
}

async fn cli_fetch(artist: &str, title: &str) -> Result<()> {
    let fetcher = fetcher::Fetcher::new()?;
    tracing::info!("cache dir: {}", fetcher.cache_dir().display());
    match fetcher.fetch(artist, title).await? {
        Some(lrc) => {
            println!("{lrc}");
            let parsed = lrc_parser::parse(&lrc);
            eprintln!("\n--- 解析出 {} 行 ---", parsed.len());
            for (i, line) in parsed.iter().take(5).enumerate() {
                eprintln!(
                    "  [{i}] {:>3}:{:02}.{:03}  {}",
                    line.time_ms / 60_000,
                    (line.time_ms / 1000) % 60,
                    line.time_ms % 1000,
                    line.text
                );
            }
            if parsed.len() > 5 {
                eprintln!("  ... 省略 {} 行", parsed.len() - 5);
            }
            Ok(())
        }
        None => {
            eprintln!("未找到歌词");
            std::process::exit(1);
        }
    }
}
