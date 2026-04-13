use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub display: DisplayConfig,
    pub behavior: BehaviorConfig,
    pub sources: SourcesConfig,
    pub filter: FilterConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    /// 显示器编号（1 起，从 GDK 枚举顺序）；0 = 跟随鼠标指针所在屏。
    pub monitor: u32,
    /// 距屏幕底部像素
    pub margin_bottom: i32,
    pub font_size_current: u32,
    pub font_size_next: u32,
    /// 字重（CSS font-weight，100~900），当前行。
    pub font_weight_current: u32,
    /// 字重（CSS font-weight，100~900），下一行。
    pub font_weight_next: u32,
    /// 0.0 - 1.0
    pub background_opacity: f64,
    pub text_color: String,
    pub secondary_text_color: String,
    pub border_radius: u32,
    pub show_without_lyrics: bool,
    /// 歌词文本最大宽度（px），超出换行
    pub max_width: i32,
    /// 字体描边颜色（CSS color），与 text_border_width 搭配。
    pub text_border_color: String,
    /// 字体描边宽度（px）。0 = 不描边。
    pub text_border_width: u32,
    /// 当前行渐变起始颜色（#RRGGBB）。为空时不启用渐变（退回 text_color）。
    pub gradient_from: String,
    /// 当前行渐变结束颜色（#RRGGBB）。
    pub gradient_to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BehaviorConfig {
    /// 轮询 MPRIS / 更新歌词行的间隔（ms）
    pub poll_interval_ms: u64,
    /// 全局歌词偏移（ms，正值提前）
    pub lyrics_offset_ms: i64,
    /// 歌曲切换去抖（ms）
    pub debounce_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SourcesConfig {
    /// 启用的歌词源，按顺序尝试；可选 "lrclib" | "netease" | "qq"
    pub enabled: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterConfig {
    /// 只监听 bus 名包含这些子串的播放器（大小写不敏感）。空 = 全部允许。
    /// 常见值： "chromium" | "chrome" | "firefox" | "spotify" | "mpv" | "vlc"
    pub players: Vec<String>,
    /// 明确排除的 bus 名子串（黑名单，先于 players 生效）。
    pub exclude_players: Vec<String>,
    /// 只监听 xesam:url 包含这些子串的曲目（大小写不敏感）。空 = 不限制。
    /// 例： ["youtube.com"] → 仅当前曲目 URL 含 youtube.com 才监听；其他一律跳过。
    /// 注意：非空时，没有 xesam:url 的播放器（如 Spotify 桌面端）也会被忽略。
    /// 如需混合原生 App 与指定网站，请改用 players 白名单并留空 urls。
    pub urls: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            display: DisplayConfig::default(),
            behavior: BehaviorConfig::default(),
            sources: SourcesConfig::default(),
            filter: FilterConfig::default(),
        }
    }
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            monitor: 0,
            margin_bottom: 120,
            font_size_current: 26,
            font_size_next: 18,
            font_weight_current: 500,
            font_weight_next: 400,
            background_opacity: 0.6,
            text_color: "#FFFFFF".into(),
            secondary_text_color: "rgba(255,255,255,0.55)".into(),
            border_radius: 14,
            show_without_lyrics: false,
            max_width: 1200,
            text_border_color: "#000000".into(),
            text_border_width: 2,
            gradient_from: "#FFFFFF".into(),
            gradient_to: "#3FA9FF".into(),
        }
    }
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 100,
            lyrics_offset_ms: 0,
            debounce_ms: 300,
        }
    }
}

impl Default for SourcesConfig {
    fn default() -> Self {
        Self {
            enabled: vec!["lrclib".into(), "netease".into(), "qq".into()],
        }
    }
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            players: Vec::new(),
            exclude_players: Vec::new(),
            urls: Vec::new(),
        }
    }
}

impl FilterConfig {
    /// bus 名是否允许（适用于 `org.mpris.MediaPlayer2.<suffix>`）。
    pub fn player_allowed(&self, bus: &str) -> bool {
        let lower = bus.to_ascii_lowercase();
        if self
            .exclude_players
            .iter()
            .any(|s| !s.is_empty() && lower.contains(&s.to_ascii_lowercase()))
        {
            return false;
        }
        if self.players.is_empty() {
            return true;
        }
        self.players
            .iter()
            .any(|s| !s.is_empty() && lower.contains(&s.to_ascii_lowercase()))
    }

    /// URL 是否允许。`urls` 为空 → 不做限制；非空且 url 为 None → 不通过。
    pub fn url_allowed(&self, url: Option<&str>) -> bool {
        if self.urls.is_empty() {
            return true;
        }
        let Some(u) = url else { return false };
        let lower = u.to_ascii_lowercase();
        self.urls
            .iter()
            .any(|s| !s.is_empty() && lower.contains(&s.to_ascii_lowercase()))
    }
}

impl Config {
    /// 加载配置；不存在则写入默认。
    pub fn load_or_init() -> Result<Self> {
        let path = Self::path()?;
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("读取配置失败: {}", path.display()))?;
            let cfg: Config = toml::from_str(&text)
                .with_context(|| format!("解析配置失败: {}", path.display()))?;
            tracing::info!("config 加载：{}", path.display());
            Ok(cfg)
        } else {
            let cfg = Config::default();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let text = toml::to_string_pretty(&cfg)?;
            std::fs::write(&path, text).ok();
            tracing::info!("config 默认已生成：{}", path.display());
            Ok(cfg)
        }
    }

    pub fn path() -> Result<PathBuf> {
        let proj = directories::ProjectDirs::from("com", "github", "wayland-float-lyrics")
            .ok_or_else(|| anyhow::anyhow!("无法定位 XDG config dir"))?;
        Ok(proj.config_dir().join("config.toml"))
    }

    /// 生成 GTK4 CSS
    pub fn css(&self) -> String {
        let d = &self.display;
        let bg_alpha = d.background_opacity.clamp(0.0, 1.0);
        let text_shadow = build_border_shadow(d.text_border_width, &d.text_border_color);
        format!(
            r#"
window.lyrics-overlay {{
    background: rgba(0, 0, 0, {bg_alpha});
    border-radius: {radius}px;
    padding: 10px 22px;
}}
.current-line {{
    color: {text};
    font-size: {fc}px;
    font-weight: {wc};
    margin-bottom: 4px;
    transition: opacity 180ms ease;
    {shadow}
}}
.next-line {{
    color: {sec};
    font-size: {fn_}px;
    font-weight: {wn};
    transition: opacity 180ms ease;
    {shadow}
}}
.fade-out {{ opacity: 0; }}
.fade-in  {{ opacity: 1; }}
"#,
            bg_alpha = bg_alpha,
            radius = d.border_radius,
            text = d.text_color,
            sec = d.secondary_text_color,
            fc = d.font_size_current,
            fn_ = d.font_size_next,
            wc = d.font_weight_current.clamp(100, 900),
            wn = d.font_weight_next.clamp(100, 900),
            shadow = text_shadow,
        )
    }

    pub fn gradient_enabled(&self) -> bool {
        !self.display.gradient_from.trim().is_empty()
            && !self.display.gradient_to.trim().is_empty()
    }
}

fn build_border_shadow(width: u32, color: &str) -> String {
    if width == 0 {
        return String::new();
    }
    let w = width as i32;
    let mut parts: Vec<String> = Vec::with_capacity(8);
    for (dx, dy) in [
        (w, 0), (-w, 0), (0, w), (0, -w),
        (w, w), (-w, -w), (w, -w), (-w, w),
    ] {
        parts.push(format!("{dx}px {dy}px 0 {color}"));
    }
    format!("text-shadow: {};", parts.join(", "))
}
