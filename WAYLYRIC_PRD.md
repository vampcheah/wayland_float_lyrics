# Wayland Float Lyrics — Wayland 桌面歌词显示器

## 项目概述

一个运行在 Ubuntu Wayland 上的桌面悬浮歌词应用。自动监听系统中正在播放的音乐（包括 YouTube），从网上歌词库获取同步歌词（LRC 格式），并以透明悬浮窗的形式实时显示在桌面上。

## 技术栈

- **语言**: Rust (edition 2021, MSRV 1.75+)
- **GUI**: gtk4-rs + gtk4-layer-shell-rs（Wayland 原生悬浮层）
- **D-Bus**: zbus（纯 Rust 异步 D-Bus，监听 MPRIS）
- **异步运行时**: tokio（网络请求、D-Bus 信号监听）
- **网络**: reqwest（歌词 API 请求）
- **配置**: toml + serde（解析配置文件）
- **构建**: cargo，提供 install.sh 一键安装系统级依赖

## Cargo.toml 核心依赖

```toml
[package]
name = "wayland-float-lyrics"
version = "0.1.0"
edition = "2021"

[dependencies]
gtk4 = "0.9"
gtk4-layer-shell = "0.4"
zbus = { version = "4", default-features = false, features = ["tokio"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time"] }
reqwest = { version = "0.12", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
regex = "1"
directories = "5"          # XDG 路径（cache、config）
tracing = "0.1"            # 日志
tracing-subscriber = "0.3"
anyhow = "1"               # 错误处理
```

## 核心功能

### 1. 歌曲识别（MPRIS 监听）

- 通过 zbus 连接 Session Bus，监听所有 `org.mpris.MediaPlayer2` 开头的 bus name
- 支持的播放器：Spotify、VLC、网易云音乐、Rhythmbox、Chrome/Firefox（YouTube）等所有实现 MPRIS 的播放器
- 监听信号：
  - `PropertiesChanged` on `org.mpris.MediaPlayer2.Player` → 检测歌曲切换、播放/暂停状态
  - `Seeked` → 检测进度跳转
  - `NameOwnerChanged` → 检测播放器启动/退出
- 定时轮询 `Position` 属性（每 200ms），用于歌词同步
- zbus 实现要点：
  - 用 `zbus::proxy` 宏定义 `MediaPlayer2PlayerProxy`
  - 用 `zbus::fdo::DBusProxy` 的 `list_names()` 枚举现有播放器
  - 用 `MessageStream` 或 `SignalStream` 接收异步信号

### 2. YouTube 标题智能解析

YouTube 通过浏览器 MPRIS 传出的元数据只有视频标题，需要额外解析：

- 用 `regex` crate 匹配常见格式：
  - `Artist - Title`
  - `Artist「Title」`
  - `Artist【Title】`
  - `【Title】Artist`
  - `Artist《Title》`
- 清洗噪音词：`Official MV`, `MV`, `Official Video`, `Lyrics`, `4K`, `HD`, `Audio`, `Live`, `Concert`, `feat.`, `ft.` 等
- 解析失败时，用清洗后的完整标题作为搜索关键词进行模糊搜索
- 支持中英日韩标题

### 3. 歌词获取（多源 + 缓存）

按优先级依次尝试以下歌词源：

| 优先级 | 来源 | API | 说明 |
|--------|------|-----|------|
| 1 | LRCLIB | `https://lrclib.net/api/search?q={query}` | 免费，无需 API Key，优先获取 syncedLyrics 字段 |
| 2 | 网易云音乐 | `https://music.163.com/api/search/get` + `/api/song/lyric` | 先搜歌曲 ID，再取歌词，中文歌词最全 |

- 获取逻辑：
  1. 先用 `artist + title` 精确搜索
  2. 失败则用 `title` 模糊搜索
  3. 优先取同步歌词（LRC），无同步歌词则取纯文本歌词（逐行静态显示）
- 本地缓存：`~/.cache/wayland-float-lyrics/`，以 `{artist}_{title}.lrc` 命名，避免重复请求
- 用 `directories` crate 获取 XDG cache 路径
- reqwest 注意：网易云 API 需要设置 `Referer: https://music.163.com` 和浏览器 User-Agent

### 4. LRC 歌词解析

- 用 `regex` 解析标准 LRC 时间标签：`[mm:ss.xx]` 或 `[mm:ss.xxx]`
- 支持一行多时间标签（翻唱歌词复用）
- 按时间戳排序（`Vec<LyricLine>` + `sort_by_key`）
- 数据结构：
  ```rust
  struct LyricLine {
      time_ms: u64,
      text: String,
  }
  ```

### 5. 桌面悬浮显示（Wayland 原生）

#### 窗口行为
- 使用 `gtk4-layer-shell` 创建 Wayland overlay 层
  - Layer: `TOP`（在普通窗口之上，在全屏之下）
  - Anchor: 底部居中（默认），可配置
  - Margin: 距底部 80px
  - `auto_exclusive_zone_enable(false)` — 不占用屏幕空间
  - `set_keyboard_mode(None)` — 不抢键盘焦点
- 窗口属性：
  - CSS 透明背景（半透明黑色背景带圆角）
  - 不出现在任务栏

#### 歌词渲染
- 使用 GTK4 Label + CSS 样式控制
- 显示两行：当前行（高亮大字）+ 下一行（半透明小字）
- 当前行：白色，24-28px，字重 Bold
- 下一行：白色 50% 透明度，18-20px，字重 Regular
- 歌曲切换时淡入淡出过渡（GTK4 CSS transition 或手动 alpha 动画）
- 歌词行切换时平滑滚动动画
- 没有歌词时显示 `♪ {歌曲名} - {歌手}` 或根据配置隐藏窗口

#### 无 gtk4-layer-shell 的降级方案
- 编译时用 cargo feature 控制：`default = ["layer-shell"]`
- 运行时检测 `gtk4_layer_shell::is_supported()`
- 降级方案：普通 GTK4 Window，`set_decorated(false)` + CSS 透明 + 手动设置窗口位置

#### GTK + Tokio 集成
- GTK4 主循环运行在主线程
- Tokio runtime 在后台线程
- 跨线程通信用 `tokio::sync::mpsc` channel：
  - **后台 → UI**：`mpsc::UnboundedSender<UiEvent>`，在 GTK 端用 `glib::MainContext::channel()` 接收
  - **UI → 后台**：`mpsc::Sender<Command>`（暂停、偏移调节等）
- 事件类型：
  ```rust
  enum UiEvent {
      SongChanged { title: String, artist: String },
      LyricsLoaded(Vec<LyricLine>),
      LyricsNotFound,
      LineChanged { current: String, next: Option<String> },
      PlaybackState(bool), // playing / paused
  }
  ```

### 6. 配置系统

配置文件路径：`~/.config/wayland-float-lyrics/config.toml`，用 `directories` crate 定位。

```toml
[display]
position = "bottom"          # top / bottom / center
margin_bottom = 80           # 距边缘像素
font_size_current = 26       # 当前行字号
font_size_next = 18          # 下一行字号
background_opacity = 0.6     # 背景透明度 0-1
text_color = "#FFFFFF"       # 文字颜色
show_without_lyrics = false  # 没歌词时是否显示歌曲信息

[behavior]
auto_hide_delay = 5          # 暂停后自动隐藏（秒），0 = 不隐藏
poll_interval_ms = 200       # 播放位置轮询间隔
lyrics_offset_ms = 0         # 全局歌词偏移（正值提前，负值延后）

[sources]
enabled = ["lrclib", "netease"]  # 启用的歌词源及优先级
```

- 用 `serde` derive 反序列化到 `Config` struct
- 所有字段提供 `Default` impl
- 配置文件不存在时自动生成默认配置

### 7. 系统托盘 / 快捷操作（可选，低优先级）

- 右键托盘菜单：显示/隐藏、歌词偏移调节（±500ms）、退出
- 或暴露 D-Bus 接口 `com.github.wayland_float_lyrics`，供快捷键绑定调用

## 项目结构

```
wayland-float-lyrics/
├── Cargo.toml
├── src/
│   ├── main.rs              # 入口：初始化 tokio runtime、GTK Application，建立 channel，启动各模块
│   ├── mpris.rs             # MPRIS D-Bus 监听：播放器发现、元数据变化、位置轮询
│   ├── title_parser.rs      # YouTube 等非标准标题的歌手/歌名解析
│   ├── fetcher.rs           # 多源歌词获取（LRCLIB、网易云）+ 磁盘缓存
│   ├── lrc_parser.rs        # LRC 格式解析
│   ├── overlay.rs           # GTK4 悬浮窗口：创建、样式、歌词渲染、动画
│   ├── config.rs            # 配置文件加载与默认值
│   └── events.rs            # UiEvent / Command 枚举定义
├── config.example.toml
├── install.sh               # 一键安装系统级构建依赖
├── README.md
└── LICENSE
```

## 系统依赖安装（install.sh 应包含）

```bash
#!/bin/bash
set -e

sudo apt update
sudo apt install -y \
  build-essential \
  libgtk-4-dev \
  libdbus-1-dev \
  pkg-config \
  gtk4-layer-shell-dev

# 安装 Rust（如果没有）
if ! command -v cargo &> /dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  source "$HOME/.cargo/env"
fi

echo "依赖安装完成，运行 cargo build --release 构建项目"
```

## 关键实现注意事项

1. **GTK + Tokio 双运行时**：`main()` 中先 spawn tokio runtime 到后台线程，再在主线程跑 `gtk::Application::run()`。绝对不要在 tokio 线程里操作 GTK 对象。
2. **glib channel 桥接**：用 `glib::MainContext::channel()` 创建一个 GLib 端的 receiver，绑定到 GTK 主循环，后台通过对应的 sender 推送 `UiEvent`。
3. **MPRIS 播放器选择**：如果有多个播放器，优先选状态为 Playing 的；都在播放则选最近 metadata 变化的。
4. **Position 轮询**：zbus proxy 的 `position()` 方法返回微秒，转换为毫秒后与 `LyricLine.time_ms` 比较，用 `partition_point` 或二分查找定位当前行。
5. **歌曲切换去抖**：MPRIS metadata 可能短时间内触发多次，加 300ms debounce 避免重复请求歌词。
6. **缓存文件名清洗**：去掉文件名中的 `/`、`\0` 等非法字符，过长则截断 + hash。
7. **信号处理**：`tokio::signal::ctrl_c()` + `glib::MainContext::quit()` 优雅退出。
8. **编码**：所有歌词缓存文件用 UTF-8。

## 构建与运行

```bash
# 开发
cargo run

# 发布构建（单二进制，约 5-10MB）
cargo build --release
./target/release/wayland-float-lyrics

# 开机自启（用户级 systemd）
mkdir -p ~/.config/systemd/user/
cp wayland-float-lyrics.service ~/.config/systemd/user/
systemctl --user enable --now wayland-float-lyrics
```

## wayland-float-lyrics.service 参考

```ini
[Unit]
Description=Wayland Float Lyrics
After=graphical-session.target

[Service]
ExecStart=%h/.cargo/bin/wayland-float-lyrics
Restart=on-failure
RestartSec=3

[Install]
WantedBy=graphical-session.target
```
