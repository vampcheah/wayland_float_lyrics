# wayland-float-lyrics

Floating lyrics overlay for Wayland / GNOME desktops. Listens to system MPRIS broadcasts (Spotify, VLC, Chrome/Firefox YouTube, etc.), automatically pulls synced lyrics from LRCLIB / NetEase, and displays them in an undecorated floating window at the bottom of the screen.

## Features

- **Auto-detects current playback** via D-Bus MPRIS — no player-specific integration required
- **Multi-source lyrics**: LRCLIB (primary) → NetEase (fallback), with on-disk caching
- **YouTube title parsing**: strips noise like "Official MV / HD / 官方高畫質" and splits artist/title
- **Client-side clock extrapolation** to work around Chromium's MPRIS `Position` not updating in real time
- **Three rendering paths with auto-fallback**:
  - wlr-layer-shell (sway / Hyprland / KDE) — native overlay
  - XWayland + EWMH dock (GNOME Wayland) — the Conky approach, pure userspace
  - Plain undecorated window (last resort)
- **Config file** at `~/.config/wayland-float-lyrics/config.toml`, auto-generated on first run

## Requirements

- Ubuntu 22.04+ / any Linux distro with GTK4
- Rust 1.75+

Install system dependencies:

```bash
bash install.sh
```

## Build & Run

```bash
cargo build --release
./target/release/wayland-float-lyrics
```

Subcommands:

| Command | Purpose |
|---|---|
| `wayland-float-lyrics` or `run` | Full mode (default) |
| `wayland-float-lyrics overlay` | GUI demo only, cycles through sample text |
| `wayland-float-lyrics mpris` | CLI-only, prints MPRIS state (diagnostics) |
| `wayland-float-lyrics fetch ARTIST TITLE` | Manually test lyrics fetching |
| `wayland-float-lyrics config` | Print current config and paths |
| `wayland-float-lyrics --help` | Help |

## Configuration

See `config.example.toml` for the full example. Key fields:

```toml
[display]
monitor = 0              # 0 = follow cursor; 1/2 = specific monitor
margin_bottom = 120
font_size_current = 26
font_size_next = 18
background_opacity = 0.6

[behavior]
poll_interval_ms = 100
lyrics_offset_ms = 0     # positive = earlier, negative = later
debounce_ms = 300

[sources]
enabled = ["lrclib", "netease"]
```

Environment variables (override config without editing the toml):

- `WFL_MONITOR=2` — target monitor
- `WFL_MARGIN_BOTTOM=100` — bottom margin
- `RUST_LOG=wayland_float_lyrics=debug` — enable debug logging

## Autostart (systemd user)

```bash
cargo install --path .
mkdir -p ~/.config/systemd/user
cp wayland-float-lyrics.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now wayland-float-lyrics
```

View logs:

```bash
journalctl --user -u wayland-float-lyrics -f
```

## Known Limitations

- **GNOME Wayland**: the compositor does not support wlr-layer-shell, so we use XWayland + `_NET_WM_WINDOW_TYPE_DOCK`. Performance overhead is negligible, but the client cannot track cursor position precisely — on multi-monitor setups, set `monitor = 2` (or similar) explicitly.
- **Chromium / Chrome / Brave**: MPRIS `Position` is not reported in real time, so we extrapolate client-side. In rare cases (buffering hiccups) this may drift ±1s.
- **Firefox**: MPRIS is off by default; enable `media.hardwaremediakeys.enabled` in `about:config`.
- **Brave**: some builds are stingy about media session broadcasts. Set `brave://flags/#hardware-media-key-handling` to Enabled.
- **YouTube title parsing**: heuristic. Videos not in `Artist - Title` form (channel compilations, MV mashups) may match the wrong song. Use `lyrics_offset_ms` in the config to fine-tune timing.

## Architecture

```
┌─────────────── Main thread (GTK) ───────────┐
│                                             │
│  gtk::Application                           │
│    └─ ApplicationWindow (layer-shell or     │
│         XWayland + EWMH dock)               │
│           └─ Overlay (current / next label) │
│                                             │
│  async_channel::Receiver<UiEvent>  ◄──┐    │
│    (glib::spawn_future_local)          │    │
└────────────────────────────────────────┼────┘
                                         │
┌──── Background tokio runtime (own thread) ──┐
│                                         │    │
│  mpris::run_backend                     │    │
│    ├─ zbus Connection + SignalStream    │    │
│    ├─ PositionAnchor extrapolation      │    │
│    ├─ title_parser::parse → candidates  │    │
│    └─ fetcher::fetch (LRCLIB → NetEase) │    │
│                                         │    │
│  async_channel::Sender<UiEvent> ────────┘    │
└──────────────────────────────────────────────┘
```

## License

MIT
