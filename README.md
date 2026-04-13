# wayland-float-lyrics

Wayland / GNOME 桌面悬浮歌词。监听系统 MPRIS 广播（Spotify、VLC、Chrome/Firefox YouTube 等），自动从 LRCLIB / 网易云拉取同步歌词，以无装饰浮动窗口显示在屏幕底部。

## 特性

- **自动识别当前播放**：通过 D-Bus MPRIS，无需绑定特定播放器
- **多源歌词**：LRCLIB（优先）→ 网易云兜底，磁盘缓存
- **YouTube 标题解析**：清洗 "Official MV / HD / 官方高畫質" 等噪音，拆出 artist/title
- **客户端时钟外推**：补偿 Chromium MPRIS `Position` 不实时更新的 bug
- **三条渲染路径自动切换**：
  - wlr-layer-shell（sway / Hyprland / KDE）— 原生浮层
  - XWayland + EWMH dock（GNOME Wayland）— Conky 同款方案，纯用户态
  - 普通无装饰窗口（兜底）
- **配置文件**：`~/.config/wayland-float-lyrics/config.toml`，首次运行自动生成

## 系统要求

- Ubuntu 22.04+ / 任意使用 GTK4 的 Linux
- Rust 1.75+

安装系统依赖：

```bash
bash install.sh
```

## 构建运行

```bash
cargo build --release
./target/release/wayland-float-lyrics
```

子命令：

| 命令 | 作用 |
|---|---|
| `wayland-float-lyrics` 或 `run` | 完整模式（默认） |
| `wayland-float-lyrics overlay` | 只跑 GUI demo，循环切换示例文案 |
| `wayland-float-lyrics mpris` | 纯 CLI 打印 MPRIS 状态（诊断用） |
| `wayland-float-lyrics fetch ARTIST TITLE` | 手动测试歌词获取 |
| `wayland-float-lyrics config` | 打印当前配置与路径 |
| `wayland-float-lyrics --help` | 帮助 |

## 配置

完整示例见 `config.example.toml`。核心字段：

```toml
[display]
monitor = 0              # 0 = 跟随鼠标；1/2 = 指定显示器
margin_bottom = 120
font_size_current = 26
font_size_next = 18
background_opacity = 0.6

[behavior]
poll_interval_ms = 100
lyrics_offset_ms = 0     # 正值提前，负值延后
debounce_ms = 300

[sources]
enabled = ["lrclib", "netease"]
```

环境变量（临时覆盖 config，免重启改 toml）：

- `WFL_MONITOR=2` 指定显示器
- `WFL_MARGIN_BOTTOM=100` 底部间距
- `RUST_LOG=wayland_float_lyrics=debug` 打开调试日志

## 开机自启（systemd user）

```bash
cargo install --path .
mkdir -p ~/.config/systemd/user
cp wayland-float-lyrics.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now wayland-float-lyrics
```

查看日志：

```bash
journalctl --user -u wayland-float-lyrics -f
```

## 已知限制

- **GNOME Wayland**：compositor 不支持 wlr-layer-shell 协议，我们走 XWayland + `_NET_WM_WINDOW_TYPE_DOCK` 路径。性能开销可忽略，但客户端无法精确跟踪鼠标位置，多屏环境建议 `monitor = 2` 显式指定。
- **Chromium / Chrome / Brave**：MPRIS `Position` 属性不实时上报，靠客户端时钟外推。极少数情况下（缓冲卡顿）可能漂移 ±1s。
- **Firefox**：MPRIS 默认关闭，需 `about:config` 打开 `media.hardwaremediakeys.enabled`。
- **Brave**：某些版本对媒体会话的广播比较吝啬。建议 `brave://flags/#hardware-media-key-handling` 置 Enabled。
- **YouTube 标题解析**：启发式，非 `Artist - Title` 格式的视频（频道合辑、MV 拼接）可能匹配到错的歌。可手动在 config 里加 `lyrics_offset_ms` 微调时序。

## 架构

```
┌─────────────── 主线程（GTK）───────────────┐
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
┌──── 后台 tokio runtime（独立线程）─────┼────┐
│                                         │    │
│  mpris::run_backend                     │    │
│    ├─ zbus Connection + SignalStream    │    │
│    ├─ PositionAnchor 外推               │    │
│    ├─ title_parser::parse → candidates  │    │
│    └─ fetcher::fetch (LRCLIB → NetEase) │    │
│                                         │    │
│  async_channel::Sender<UiEvent> ────────┘    │
└──────────────────────────────────────────────┘
```

## 许可

MIT
