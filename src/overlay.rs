use anyhow::Result;
use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, Box as GtkBox, CssProvider, Label, Orientation};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use crate::config::Config;

#[derive(Debug, Clone)]
struct Karaoke {
    text: String,
    start_ms: u64,
    end_ms: u64,
    anchor_pos_ms: u64,
    anchor_instant: Instant,
    playing: bool,
}

impl Karaoke {
    fn progress(&self) -> f64 {
        let now_ms = if self.playing {
            self.anchor_pos_ms as i64 + self.anchor_instant.elapsed().as_millis() as i64
        } else {
            self.anchor_pos_ms as i64
        };
        let span = (self.end_ms.saturating_sub(self.start_ms)).max(1) as f64;
        ((now_ms - self.start_ms as i64) as f64 / span).clamp(0.0, 1.0)
    }
}

const APP_ID: &str = "com.github.wayland_float_lyrics";

fn env_override_i32(key: &str, fallback: i32) -> i32 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(fallback)
}

fn env_override_u32(key: &str, fallback: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(fallback)
}

pub struct Overlay {
    pub window: ApplicationWindow,
    pub current: Label,
    pub next: Label,
    cfg: Arc<Config>,
    karaoke: Rc<RefCell<Option<Karaoke>>>,
}

impl Overlay {
    pub fn set_lines(&self, current: &str, next: Option<&str>) {
        // 非卡拉 OK 文本（header / 状态提示）：清除 karaoke 状态，按普通淡入渲染。
        self.karaoke.borrow_mut().take();
        self.apply_fade(&self.current, current, false);
        let nxt_text = next.unwrap_or("");
        self.apply_fade(&self.next, nxt_text, false);
        self.next.set_visible(next.is_some());
    }

    fn apply_fade(&self, label: &Label, new_text: &str, _is_current: bool) {
        if label.label().as_str() == new_text {
            return;
        }
        label.add_css_class("fade-out");
        let label_c = label.clone();
        let text = new_text.to_string();
        gtk4::glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
            set_label_text(&label_c, &text, None);
            label_c.remove_css_class("fade-out");
        });
    }

    fn set_karaoke_line(
        &self,
        current: String,
        next: Option<String>,
        start_ms: u64,
        end_ms: u64,
        pos_ms: u64,
    ) {
        // next 行走普通路径（无 karaoke 填充）
        let nxt_text = next.as_deref().unwrap_or("");
        self.apply_fade(&self.next, nxt_text, false);
        self.next.set_visible(next.is_some());

        let cur_text = if current.is_empty() { "…".to_string() } else { current };
        let playing = self
            .karaoke
            .borrow()
            .as_ref()
            .map(|k| k.playing)
            .unwrap_or(true);
        let state = Karaoke {
            text: cur_text.clone(),
            start_ms,
            end_ms,
            anchor_pos_ms: pos_ms,
            anchor_instant: Instant::now(),
            playing,
        };

        // 若文本一致（如 dedup 相同行时间戳变化），只刷新 timing 不做淡入
        let same_text = self.current.label().as_str() == cur_text;
        if same_text {
            *self.karaoke.borrow_mut() = Some(state.clone());
            render_karaoke(&self.current, &state, &self.cfg);
            return;
        }

        // 先清空 karaoke 槽，避免 50ms ticker 在淡出期间抢先覆盖旧文本；
        // 淡出结束后再装入新状态并首次渲染。
        self.karaoke.borrow_mut().take();
        self.current.add_css_class("fade-out");
        let label = self.current.clone();
        let cfg = self.cfg.clone();
        let karaoke_slot = self.karaoke.clone();
        gtk4::glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
            *karaoke_slot.borrow_mut() = Some(state.clone());
            render_karaoke(&label, &state, &cfg);
            label.remove_css_class("fade-out");
        });
    }

    fn set_playing(&self, playing: bool) {
        let mut slot = self.karaoke.borrow_mut();
        if let Some(k) = slot.as_mut() {
            if k.playing == playing {
                return;
            }
            // 快照当前位置再切状态，暂停后进度不再推进
            let now_ms = if k.playing {
                k.anchor_pos_ms as i64 + k.anchor_instant.elapsed().as_millis() as i64
            } else {
                k.anchor_pos_ms as i64
            };
            k.anchor_pos_ms = now_ms.max(0) as u64;
            k.anchor_instant = Instant::now();
            k.playing = playing;
        }
    }

    pub fn handle_event(&self, event: crate::events::UiEvent) {
        use crate::events::UiEvent;
        match event {
            UiEvent::SongChanged { artist, title } => {
                let header = if artist.is_empty() {
                    format!("♪ {title}")
                } else {
                    format!("♪ {artist} - {title}")
                };
                self.set_lines(&header, Some("正在获取歌词…"));
            }
            UiEvent::LyricsLoaded(_) => {}
            UiEvent::LyricsNotFound => {
                self.set_lines("♪ 未找到歌词", None);
            }
            UiEvent::LineChanged {
                current,
                next,
                start_ms,
                end_ms,
                pos_ms,
            } => {
                self.set_karaoke_line(current, next, start_ms, end_ms, pos_ms);
            }
            UiEvent::PlaybackState(p) => self.set_playing(p),
        }
    }
}

fn parse_hex_color(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

fn escape_pango(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&apos;")
        .replace('"', "&quot;")
}

fn karaoke_markup(text: &str, progress: f64, from: &str, to: &str) -> Option<String> {
    let (r1, g1, b1) = parse_hex_color(from)?;
    let (r2, g2, b2) = parse_hex_color(to)?;
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Some(String::new());
    }
    let n = chars.len() as f64;
    let fill = progress.clamp(0.0, 1.0) * n;
    let full = fill.floor() as usize;
    let frac = fill - full as f64;
    let mut out = String::with_capacity(text.len() * 32);
    for (i, ch) in chars.iter().enumerate() {
        let t = if i < full {
            1.0
        } else if i == full {
            frac
        } else {
            0.0
        };
        let r = (r1 as f64 + (r2 as f64 - r1 as f64) * t).round() as u8;
        let g = (g1 as f64 + (g2 as f64 - g1 as f64) * t).round() as u8;
        let b = (b1 as f64 + (b2 as f64 - b1 as f64) * t).round() as u8;
        let esc = escape_pango(&ch.to_string());
        out.push_str(&format!(
            "<span foreground=\"#{r:02X}{g:02X}{b:02X}\">{esc}</span>"
        ));
    }
    Some(out)
}

fn set_label_text(label: &Label, text: &str, _unused: Option<&(String, String)>) {
    label.set_use_markup(false);
    label.set_label(text);
}

fn render_karaoke(label: &Label, k: &Karaoke, cfg: &Config) {
    if cfg.gradient_enabled() {
        if let Some(markup) = karaoke_markup(
            &k.text,
            k.progress(),
            &cfg.display.gradient_from,
            &cfg.display.gradient_to,
        ) {
            label.set_use_markup(true);
            label.set_markup(&markup);
            return;
        }
    }
    label.set_use_markup(false);
    label.set_label(&k.text);
}

fn load_css(cfg: &Config) {
    let provider = CssProvider::new();
    provider.load_from_data(&cfg.css());
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn margin_bottom(cfg: &Config) -> i32 {
    env_override_i32("WFL_MARGIN_BOTTOM", cfg.display.margin_bottom)
}

fn monitor_idx(cfg: &Config) -> u32 {
    env_override_u32("WFL_MONITOR", cfg.display.monitor)
}

fn enable_layer_shell(window: &ApplicationWindow, cfg: &Config) -> bool {
    use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
    if !gtk4_layer_shell::is_supported() {
        tracing::warn!("gtk4-layer-shell 不支持当前会话，尝试降级路径");
        return false;
    }
    window.init_layer_shell();
    window.set_layer(Layer::Top);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_exclusive_zone(-1);
    window.set_anchor(Edge::Bottom, true);
    window.set_anchor(Edge::Left, false);
    window.set_anchor(Edge::Right, false);
    window.set_margin(Edge::Bottom, margin_bottom(cfg));
    tracing::info!("layer-shell 启用：底部居中 margin={}", margin_bottom(cfg));
    true
}

#[cfg(feature = "x11-dock")]
fn pin_to_bottom_x11(window: &ApplicationWindow, cfg: &Config) -> anyhow::Result<()> {
    use anyhow::Context;
    use gdk4_x11::X11Surface;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt as _, *};
    use x11rb::wrapper::ConnectionExt as _;

    let surface = window.surface().context("window 尚未 realize，无 surface")?;
    let x11_surface: X11Surface = surface
        .clone()
        .downcast()
        .map_err(|_| anyhow::anyhow!("surface 不是 X11Surface — GDK_BACKEND 未设为 x11？"))?;
    let xid = x11_surface.xid() as u32;

    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;

    let display = gtk4::prelude::WidgetExt::display(window);
    let monitors = display.monitors();

    let picked: Option<gtk4::gdk::Monitor> = (|| {
        let idx = monitor_idx(cfg);
        if idx >= 1 {
            if let Some(m) = monitors
                .item(idx - 1)
                .and_then(|o| o.downcast::<gtk4::gdk::Monitor>().ok())
            {
                tracing::info!("config monitor={idx} 指定屏幕");
                return Some(m);
            }
        }
        let pointer = conn.query_pointer(root).ok().and_then(|c| c.reply().ok());
        if let Some(reply) = pointer {
            let px = reply.root_x as i32;
            let py = reply.root_y as i32;
            for i in 0..monitors.n_items() {
                if let Some(m) = monitors
                    .item(i)
                    .and_then(|o| o.downcast::<gtk4::gdk::Monitor>().ok())
                {
                    let g = m.geometry();
                    if px >= g.x()
                        && px < g.x() + g.width()
                        && py >= g.y()
                        && py < g.y() + g.height()
                    {
                        tracing::info!(
                            "鼠标位于屏 #{} ({}x{}@{},{})",
                            i,
                            g.width(),
                            g.height(),
                            g.x(),
                            g.y()
                        );
                        return Some(m);
                    }
                }
            }
        }
        display.monitor_at_surface(&surface)
    })();

    let geom = picked
        .as_ref()
        .map(|m| m.geometry())
        .or_else(|| {
            monitors
                .item(0)
                .and_then(|o| o.downcast::<gtk4::gdk::Monitor>().ok())
                .map(|m| m.geometry())
        })
        .unwrap_or_else(|| gtk4::gdk::Rectangle::new(0, 0, 1920, 1080));

    let win_w = window.width().max(window.default_width()).max(720);
    let win_h = window.height().max(window.default_height()).max(60);
    let x_px = (geom.x() + (geom.width() - win_w) / 2).max(0);
    let y_px = (geom.y() + geom.height() - win_h - margin_bottom(cfg)).max(0);

    let atom = |name: &[u8]| -> anyhow::Result<u32> {
        Ok(conn.intern_atom(false, name)?.reply()?.atom)
    };
    let a_type = atom(b"_NET_WM_WINDOW_TYPE")?;
    let a_dock = atom(b"_NET_WM_WINDOW_TYPE_DOCK")?;
    let a_state = atom(b"_NET_WM_STATE")?;
    let a_above = atom(b"_NET_WM_STATE_ABOVE")?;
    let a_skip_tb = atom(b"_NET_WM_STATE_SKIP_TASKBAR")?;
    let a_skip_pg = atom(b"_NET_WM_STATE_SKIP_PAGER")?;
    let a_sticky = atom(b"_NET_WM_STATE_STICKY")?;

    conn.change_property32(PropMode::REPLACE, xid, a_type, AtomEnum::ATOM, &[a_dock])?;
    conn.change_property32(
        PropMode::REPLACE,
        xid,
        a_state,
        AtomEnum::ATOM,
        &[a_above, a_skip_tb, a_skip_pg, a_sticky],
    )?;
    conn.configure_window(xid, &ConfigureWindowAux::new().x(x_px).y(y_px))?;
    conn.flush()?;

    tracing::info!(
        "X11 dock 已绑定：xid={xid:#x} pos=({x_px},{y_px}) monitor={}x{}@({},{}) win={win_w}x{win_h}",
        geom.width(),
        geom.height(),
        geom.x(),
        geom.y()
    );
    Ok(())
}

pub fn build_window(app: &Application, cfg: Arc<Config>) -> Overlay {
    let current = Label::builder()
        .label("♪ wayland-float-lyrics")
        .css_classes(vec!["current-line".to_string()])
        .wrap(true)
        .max_width_chars(80)
        .build();
    let next = Label::builder()
        .label("")
        .css_classes(vec!["next-line".to_string()])
        .visible(false)
        .wrap(true)
        .max_width_chars(80)
        .build();

    let container = GtkBox::new(Orientation::Vertical, 0);
    container.append(&current);
    container.append(&next);

    let window = ApplicationWindow::builder()
        .application(app)
        .child(&container)
        .decorated(false)
        .resizable(false)
        .default_width(720)
        .default_height(100)
        .css_classes(vec!["lyrics-overlay".to_string()])
        .build();

    let layer_ok = enable_layer_shell(&window, &cfg);

    #[cfg(feature = "x11-dock")]
    if !layer_ok {
        let cfg_realize = cfg.clone();
        window.connect_realize(move |w| {
            if let Err(e) = pin_to_bottom_x11(w, &cfg_realize) {
                tracing::warn!("X11 dock 绑定失败：{e}");
            }
        });
        let cfg_map = cfg.clone();
        window.connect_map(move |w| {
            let cfg_inner = cfg_map.clone();
            gtk4::glib::idle_add_local_once({
                let w = w.clone();
                move || {
                    if let Err(e) = pin_to_bottom_x11(&w, &cfg_inner) {
                        tracing::debug!("X11 dock 二次校正失败：{e}");
                    }
                }
            });
        });
    }

    if !layer_ok {
        window.set_title(Some("wayland-float-lyrics"));
    }

    window.present();

    let karaoke: Rc<RefCell<Option<Karaoke>>> = Rc::new(RefCell::new(None));

    // 50ms ticker：按墙钟外推进度，重绘当前行 karaoke markup
    {
        let karaoke = karaoke.clone();
        let label = current.clone();
        let cfg_tick = cfg.clone();
        gtk4::glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if let Some(k) = karaoke.borrow().as_ref() {
                render_karaoke(&label, k, &cfg_tick);
            }
            gtk4::glib::ControlFlow::Continue
        });
    }

    Overlay {
        window,
        current,
        next,
        cfg: cfg.clone(),
        karaoke,
    }
}

/// Phase 4 完整运行：GTK 主线程 + tokio 后台线程，MPRIS 驱动歌词。
pub fn run_full(cfg: Config) -> Result<()> {
    use crate::events::{Command, UiEvent};
    use std::cell::RefCell;
    use std::rc::Rc;

    let cfg = Arc::new(cfg);
    let (ui_tx, ui_rx) = async_channel::unbounded::<UiEvent>();
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<Command>();

    let cfg_backend = cfg.clone();
    let backend_handle = std::thread::Builder::new()
        .name("wfl-backend".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                if let Err(e) = crate::mpris::run_backend(ui_tx, cmd_rx, cfg_backend).await {
                    tracing::error!("backend 异常退出：{e}");
                }
            });
        })?;

    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let cfg_css = cfg.clone();
    app.connect_startup(move |_| load_css(&cfg_css));

    #[cfg(unix)]
    {
        let install = |sig: i32, app: &Application, cmd_tx: tokio::sync::mpsc::UnboundedSender<Command>| {
            let app_weak = app.downgrade();
            gtk4::glib::unix_signal_add_local(sig, move || {
                tracing::info!("收到信号 {sig}，退出");
                let _ = cmd_tx.send(Command::Quit);
                if let Some(app) = app_weak.upgrade() {
                    app.quit();
                }
                gtk4::glib::ControlFlow::Break
            });
        };
        install(2, &app, cmd_tx.clone());  // SIGINT
        install(15, &app, cmd_tx.clone()); // SIGTERM
    }

    let overlay_slot: Rc<RefCell<Option<Overlay>>> = Rc::new(RefCell::new(None));
    let show_no_lyrics = cfg.display.show_without_lyrics;
    app.connect_activate({
        let slot = overlay_slot.clone();
        let ui_rx = ui_rx.clone();
        let cfg = cfg.clone();
        move |app| {
            let ov = build_window(app, cfg.clone());
            ov.set_lines("♪ wayland-float-lyrics", Some("等待播放器…"));
            *slot.borrow_mut() = Some(ov);

            let slot = slot.clone();
            let ui_rx = ui_rx.clone();
            gtk4::glib::spawn_future_local(async move {
                while let Ok(event) = ui_rx.recv().await {
                    if let Some(ov) = slot.borrow().as_ref() {
                        // 无歌词时可按配置隐藏窗口
                        match &event {
                            crate::events::UiEvent::LyricsNotFound if !show_no_lyrics => {
                                ov.window.hide();
                            }
                            crate::events::UiEvent::LyricsLoaded(_)
                            | crate::events::UiEvent::SongChanged { .. } => {
                                ov.window.show();
                            }
                            _ => {}
                        }
                        ov.handle_event(event);
                    }
                }
            });
        }
    });

    let argv: [&str; 1] = ["wayland-float-lyrics"];
    let code = app.run_with_args(&argv);

    let _ = cmd_tx.send(Command::Quit);
    drop(cmd_tx);
    if let Err(e) = backend_handle.join() {
        tracing::warn!("backend 线程 join 失败：{e:?}");
    }

    if code.value() != 0 {
        anyhow::bail!("GTK Application exited with code {}", code.value());
    }
    Ok(())
}

/// Phase 3 独立 demo
pub fn run_demo(cfg: Config) -> Result<()> {
    let cfg = Arc::new(cfg);
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let cfg_css = cfg.clone();
    app.connect_startup(move |_| load_css(&cfg_css));
    app.connect_activate({
        let cfg = cfg.clone();
        move |app| {
            let overlay = build_window(app, cfg.clone());
            overlay.set_lines("这是当前行（high-contrast）", Some("下一行（半透明）"));
            let current = overlay.current.clone();
            let next = overlay.next.clone();
            let cfg_demo = cfg.clone();
            let samples = [
                ("好不容易又能再多愛一天", Some("但故事的最後你好像還是說了拜拜")),
                ("Hello from the other side", Some("I must've called a thousand times")),
                ("♪ wayland-float-lyrics demo", None),
            ];
            let mut i = 0usize;
            gtk4::glib::timeout_add_seconds_local(2, move || {
                let (cur, nxt) = samples[i % samples.len()];
                let grad = if cfg_demo.gradient_enabled() {
                    Some((
                        cfg_demo.display.gradient_from.clone(),
                        cfg_demo.display.gradient_to.clone(),
                    ))
                } else {
                    None
                };
                set_label_text(&current, cur, grad.as_ref());
                set_label_text(&next, nxt.unwrap_or(""), None);
                next.set_visible(nxt.is_some());
                i += 1;
                gtk4::glib::ControlFlow::Continue
            });
        }
    });
    let argv: [&str; 1] = ["wayland-float-lyrics"];
    let code = app.run_with_args(&argv);
    if code.value() != 0 {
        anyhow::bail!("GTK Application exited with code {}", code.value());
    }
    Ok(())
}
