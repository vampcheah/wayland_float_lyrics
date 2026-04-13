use anyhow::{Context, Result};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use zbus::fdo;
use zbus::names::BusName;
use zbus::zvariant::OwnedValue;
use zbus::Connection;

use crate::config::Config;
use crate::events::{Command, UiEvent};
use crate::fetcher::Fetcher;
use crate::lrc_parser::{self, LyricLine};
use std::sync::Arc;

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";

#[zbus::proxy(
    interface = "org.mpris.MediaPlayer2.Player",
    default_path = "/org/mpris/MediaPlayer2"
)]
pub trait MediaPlayer2Player {
    #[zbus(property)]
    fn playback_status(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn metadata(&self) -> zbus::Result<HashMap<String, OwnedValue>>;
    #[zbus(property)]
    fn position(&self) -> zbus::Result<i64>;
    #[zbus(signal)]
    fn seeked(&self, position: i64) -> zbus::Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub length_ms: u64,
    pub url: Option<String>,
}

pub fn parse_metadata(meta: &HashMap<String, OwnedValue>) -> TrackInfo {
    use zbus::zvariant::{Array, Str};

    let title = meta
        .get("xesam:title")
        .and_then(|v| v.downcast_ref::<Str>().ok())
        .map(|s| s.as_str().to_string())
        .unwrap_or_default();

    let artist = meta
        .get("xesam:artist")
        .and_then(|v| v.downcast_ref::<Array>().ok())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| <&str>::try_from(v).ok())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    let length_us = meta
        .get("mpris:length")
        .and_then(|v| v.downcast_ref::<i64>().ok())
        .unwrap_or(0);

    let url = meta
        .get("xesam:url")
        .and_then(|v| v.downcast_ref::<Str>().ok())
        .map(|s| s.as_str().to_string())
        .filter(|s| !s.is_empty());

    TrackInfo {
        title,
        artist,
        length_ms: (length_us / 1000).max(0) as u64,
        url,
    }
}

async fn build_proxy(
    conn: &Connection,
    bus: String,
) -> Result<MediaPlayer2PlayerProxy<'static>> {
    let name = BusName::try_from(bus).context("invalid bus name")?;
    let proxy = MediaPlayer2PlayerProxy::builder(conn)
        .destination(name)?
        .build()
        .await?;
    Ok(proxy)
}

/// 在所有已知播放器中挑一个"活跃"的：Playing 优先，其次 Paused，最后 Stopped。
async fn pick_active<'a>(
    players: &'a HashMap<String, MediaPlayer2PlayerProxy<'static>>,
) -> Option<(&'a String, &'a MediaPlayer2PlayerProxy<'static>)> {
    let mut best: Option<(&String, &MediaPlayer2PlayerProxy<'static>, u8)> = None;
    for (bus, proxy) in players.iter() {
        let rank = match proxy.playback_status().await {
            Ok(s) if s == "Playing" => 3,
            Ok(s) if s == "Paused" => 2,
            Ok(_) => 1,
            Err(_) => 0,
        };
        if best.map(|(_, _, r)| rank > r).unwrap_or(true) {
            best = Some((bus, proxy, rank));
        }
    }
    best.map(|(b, p, _)| (b, p))
}

/// Phase 1 CLI 验证：打印 MPRIS 播放器实时状态。
pub async fn run_cli() -> Result<()> {
    let conn = Connection::session()
        .await
        .context("failed to connect to session D-Bus")?;
    let dbus = fdo::DBusProxy::new(&conn).await?;

    let mut players: HashMap<String, MediaPlayer2PlayerProxy<'static>> = HashMap::new();
    for name in dbus.list_names().await? {
        let name_str = name.as_str().to_string();
        if name_str.starts_with(MPRIS_PREFIX) {
            match build_proxy(&conn, name_str.clone()).await {
                Ok(p) => {
                    players.insert(name_str, p);
                }
                Err(e) => tracing::warn!("skip {}: {e}", name_str),
            }
        }
    }
    tracing::info!(
        "initial players: {:?}",
        players.keys().collect::<Vec<_>>()
    );

    let mut owner_changes = dbus.receive_name_owner_changed().await?;
    let mut poll = tokio::time::interval(Duration::from_millis(500));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_print_key = String::new();

    loop {
        tokio::select! {
            Some(sig) = owner_changes.next() => {
                let args = sig.args()?;
                let name: &str = args.name();
                if !name.starts_with(MPRIS_PREFIX) { continue; }
                let joined = !args.new_owner().as_ref().map(|s| s.is_empty()).unwrap_or(true);
                if joined {
                    match build_proxy(&conn, name.to_string()).await {
                        Ok(p) => {
                            players.insert(name.to_string(), p);
                            tracing::info!("player joined: {}", name);
                        }
                        Err(e) => tracing::warn!("join build failed {}: {e}", name),
                    }
                } else {
                    players.remove(name);
                    tracing::info!("player left: {}", name);
                }
            }
            _ = poll.tick() => {
                if let Some((bus, proxy)) = pick_active(&players).await {
                    let status = proxy.playback_status().await.unwrap_or_else(|_| "?".into());
                    let meta = proxy.metadata().await.unwrap_or_default();
                    let info = parse_metadata(&meta);
                    let pos_ms = (proxy.position().await.unwrap_or(0) / 1000).max(0) as u64;
                    let key = format!("{bus}|{}|{}|{status}", info.title, info.artist);
                    if key != last_print_key {
                        println!(
                            "[{bus}] status={status} artist={:?} title={:?} length={}ms",
                            info.artist, info.title, info.length_ms
                        );
                        last_print_key = key;
                    }
                    println!(
                        "  pos={pos_ms}ms / {}ms",
                        info.length_ms
                    );
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("ctrl-c, exiting");
                return Ok(());
            }
        }
    }
}

/// 客户端时钟外推：Chromium 等播放器 Position 不实时更新，
/// 我们记一个"锚点"（MPRIS 报告的位置 + 本地墙钟），后续按墙钟外推。
#[derive(Debug, Clone, Copy)]
struct PositionAnchor {
    base_ms: i64,
    base_instant: Instant,
    playing: bool,
}

impl PositionAnchor {
    fn current(&self, offset_ms: i64) -> i64 {
        let drift = if self.playing {
            self.base_instant.elapsed().as_millis() as i64
        } else {
            0
        };
        self.base_ms + drift + offset_ms
    }
}

fn spawn_seeked_listener(
    proxy: MediaPlayer2PlayerProxy<'static>,
    bus: String,
    tx: tokio::sync::mpsc::UnboundedSender<(String, i64)>,
) {
    tokio::spawn(async move {
        let mut stream = match proxy.receive_seeked().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("seeked 订阅失败 {bus}: {e}");
                return;
            }
        };
        while let Some(sig) = stream.next().await {
            let Ok(args) = sig.args() else { continue };
            if tx.send((bus.clone(), args.position)).is_err() {
                break;
            }
        }
    });
}

/// Phase 4 后台主循环：MPRIS → 歌词获取 → 位置外推 → UiEvent 发给 GTK 侧。
pub async fn run_backend(
    ui_tx: async_channel::Sender<UiEvent>,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<Command>,
    cfg: Arc<Config>,
) -> Result<()> {
    let fetcher = Fetcher::with_sources(cfg.sources.enabled.clone())?;
    let poll_ms = cfg.behavior.poll_interval_ms.max(50);
    let debounce_ms = cfg.behavior.debounce_ms;
    let mut offset_ms: i64 = cfg.behavior.lyrics_offset_ms;
    let conn = Connection::session().await.context("session D-Bus 连接失败")?;
    let dbus = fdo::DBusProxy::new(&conn).await?;

    let filter = cfg.filter.clone();
    let mut players: HashMap<String, MediaPlayer2PlayerProxy<'static>> = HashMap::new();
    for name in dbus.list_names().await? {
        let name_str = name.as_str().to_string();
        if name_str.starts_with(MPRIS_PREFIX) {
            if !filter.player_allowed(&name_str) {
                tracing::info!("player 过滤（启动）：{name_str}");
                continue;
            }
            if let Ok(p) = build_proxy(&conn, name_str.clone()).await {
                players.insert(name_str, p);
            }
        }
    }
    tracing::info!(
        "backend 启动，初始播放器 {} 个（filter: players={:?}, exclude={:?}, urls={:?}）",
        players.len(),
        filter.players,
        filter.exclude_players,
        filter.urls
    );

    let mut owner_changes = dbus.receive_name_owner_changed().await?;
    let mut tick = tokio::time::interval(Duration::from_millis(poll_ms));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let (seek_tx, mut seek_rx) = tokio::sync::mpsc::unbounded_channel::<(String, i64)>();
    for (bus, proxy) in players.iter() {
        spawn_seeked_listener(proxy.clone(), bus.clone(), seek_tx.clone());
    }

    let mut current_song_key: Option<String> = None;
    let mut lyrics: Vec<LyricLine> = Vec::new();
    let mut anchor: Option<PositionAnchor> = None;
    let mut last_line_idx: Option<usize> = None;
    let mut last_is_playing: Option<bool> = None;
    let mut pending_song: Option<(String, String, Instant)> = None;
    let mut resync_tick = 0u32;
    let mut last_reported_ms: Option<i64> = None;
    let mut seek_cooldown_until: Option<Instant> = None;

    loop {
        tokio::select! {
            Some(sig) = owner_changes.next() => {
                let args = sig.args()?;
                let name: &str = args.name();
                if !name.starts_with(MPRIS_PREFIX) { continue; }
                let joined = !args.new_owner().as_ref().map(|s| s.is_empty()).unwrap_or(true);
                if joined {
                    if !filter.player_allowed(name) {
                        tracing::debug!("player 过滤（join）：{name}");
                        continue;
                    }
                    if let Ok(p) = build_proxy(&conn, name.to_string()).await {
                        spawn_seeked_listener(p.clone(), name.to_string(), seek_tx.clone());
                        players.insert(name.to_string(), p);
                        tracing::info!("player joined: {name}");
                    }
                } else if players.remove(name).is_some() {
                    tracing::info!("player left: {name}");
                }
            }
            Some((bus, pos_us)) = seek_rx.recv() => {
                let pos_ms = (pos_us / 1000).max(0);
                tracing::info!("Seeked 信号：player={bus} pos={pos_ms}ms");
                anchor = Some(PositionAnchor {
                    base_ms: pos_ms,
                    base_instant: Instant::now(),
                    playing: last_is_playing.unwrap_or(true),
                });
                last_reported_ms = Some(pos_ms);
                last_line_idx = None;
                // 防止随后 1.5s 内 Chromium/YouTube 陈旧 Position 把锚点拉回
                seek_cooldown_until = Some(Instant::now() + Duration::from_millis(1500));
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(Command::AdjustOffset(d)) => {
                        offset_ms = offset_ms.saturating_add(d);
                        tracing::info!("歌词偏移：{offset_ms}ms");
                    }
                    Some(Command::TogglePause) => { /* TODO: PlayPause 方法调用 */ }
                    Some(Command::Quit) | None => {
                        tracing::info!("backend 收到 Quit，退出");
                        return Ok(());
                    }
                }
            }
            _ = tick.tick() => {
                resync_tick = (resync_tick + 1) % 10;  // 每 ~1s 重同步位置

                // 歌曲切换 debounce：>= 300ms 未再变 → 真正 fetch
                let pending_ready = pending_song.as_ref()
                    .map(|(_, _, t)| t.elapsed() >= Duration::from_millis(debounce_ms))
                    .unwrap_or(false);
                if pending_ready {
                    if let Some((raw_artist, raw_title, _)) = pending_song.take() {
                        tracing::info!("歌曲切换：{raw_artist} - {raw_title}");
                        // 解析脏标题：YouTube 等场景要先清洗 / 拆 artist
                        let candidates = crate::title_parser::parse(&raw_title, &raw_artist);
                        tracing::debug!("候选查询: {:?}", candidates);
                        let _ = ui_tx.send(UiEvent::SongChanged {
                            artist: raw_artist.clone(),
                            title: raw_title.clone(),
                        }).await;

                        let mut found: Option<(String, String, String)> = None; // (artist, title, lrc)
                        for (a, t) in candidates.iter().take(4) {
                            if t.is_empty() { continue; }
                            tracing::info!("尝试歌词源：artist={a:?} title={t:?}");
                            match fetcher.fetch(a, t).await {
                                Ok(Some(lrc)) => {
                                    found = Some((a.clone(), t.clone(), lrc));
                                    break;
                                }
                                Ok(None) => {}
                                Err(e) => tracing::warn!("查询失败：{e}"),
                            }
                        }
                        match found {
                            Some((a, t, lrc)) => {
                                let parsed = lrc_parser::parse(&lrc);
                                tracing::info!("歌词命中：{a} - {t}，{} 行", parsed.len());
                                let _ = ui_tx.send(UiEvent::LyricsLoaded(parsed.clone())).await;
                                lyrics = parsed;
                                last_line_idx = None;
                                // 不重建锚点：debounce 期间已按 wall-clock 累积的锚点最可靠；
                                // 陈旧 Position 的播放器（Chromium）如果此时用 reported 重建反而会把锚点拉回 0。
                            }
                            None => {
                                tracing::info!("所有候选都未找到歌词");
                                lyrics.clear();
                                last_line_idx = None;
                                let _ = ui_tx.send(UiEvent::LyricsNotFound).await;
                            }
                        }
                    }
                }

                // 选当前活跃播放器
                let active_bus: Option<String> = {
                    let mut best: Option<(String, u8)> = None;
                    for (bus, proxy) in players.iter() {
                        let rank = match proxy.playback_status().await {
                            Ok(s) if s == "Playing" => 3,
                            Ok(s) if s == "Paused" => 2,
                            Ok(_) => 1,
                            Err(_) => 0,
                        };
                        if best.as_ref().map(|(_, r)| rank > *r).unwrap_or(true) {
                            best = Some((bus.clone(), rank));
                        }
                    }
                    best.map(|(b, _)| b)
                };

                let Some(bus) = active_bus else {
                    if resync_tick == 0 {
                        tracing::debug!("无活跃播放器 (共 {} 个注册)", players.len());
                    }
                    continue;
                };
                let Some(proxy) = players.get(&bus) else { continue; };

                let status = proxy.playback_status().await.unwrap_or_default();
                let is_playing = status == "Playing";
                let meta = proxy.metadata().await.unwrap_or_default();
                let info = parse_metadata(&meta);

                if !filter.url_allowed(info.url.as_deref()) {
                    if resync_tick == 0 {
                        tracing::debug!(
                            "URL 过滤：player={bus} url={:?} 不在 urls 白名单内",
                            info.url
                        );
                    }
                    if last_is_playing != Some(false) {
                        let _ = ui_tx.send(UiEvent::PlaybackState(false)).await;
                        last_is_playing = Some(false);
                    }
                    continue;
                }

                if last_is_playing != Some(is_playing) {
                    tracing::info!("播放状态变更：{status} (player={bus})");
                    let _ = ui_tx.send(UiEvent::PlaybackState(is_playing)).await;
                    last_is_playing = Some(is_playing);
                }

                if info.title.is_empty() {
                    if resync_tick == 0 {
                        tracing::debug!("活跃 player={bus} status={status} 但无 title");
                    }
                    continue;
                }

                let song_key = format!("{}__{}", info.artist, info.title);
                if Some(&song_key) != current_song_key.as_ref() {
                    current_song_key = Some(song_key);
                    pending_song = Some((info.artist.clone(), info.title.clone(), Instant::now()));
                    lyrics.clear();
                    last_line_idx = None;
                    anchor = None;
                    continue;  // 等 debounce
                }

                // 位置锚点维护：
                // Chromium 的 Position 永远返回陈旧值，跨 tick 保持不变 → 按"值是否变化"判断是否可信。
                // Spotify 之类正常更新 Position 的播放器每次读都是新值 → 持续 resync。
                let reported_ms = proxy.position().await.ok().map(|us| (us / 1000).max(0));
                let player_moved = match (reported_ms, last_reported_ms) {
                    (Some(r), Some(l)) => r != l,
                    (Some(_), None) => true,
                    _ => false,
                };
                last_reported_ms = reported_ms;

                match (anchor.as_mut(), reported_ms) {
                    (None, Some(ms)) => {
                        anchor = Some(PositionAnchor {
                            base_ms: ms,
                            base_instant: Instant::now(),
                            playing: is_playing,
                        });
                    }
                    (Some(a), Some(ms)) => {
                        if a.playing != is_playing {
                            a.base_ms = a.current(0);
                            a.base_instant = Instant::now();
                            a.playing = is_playing;
                        } else if player_moved {
                            // 播放器真的动了：可能是 seek，或正常实时上报。
                            // 小漂移（<250ms）忽略，避免每次 poll 都重置锚点造成 karaoke 抖动。
                            // seek 冷却期内完全跳过：Seeked 信号已给出权威位置，防止随后陈旧 Position 污染。
                            let in_cooldown = seek_cooldown_until
                                .map(|t| Instant::now() < t)
                                .unwrap_or(false);
                            let extrap = a.current(0);
                            let diff = (ms - extrap).abs();
                            if !in_cooldown && diff > 250 {
                                if diff > 1000 {
                                    tracing::info!(
                                        "位置校正：extrap={extrap}ms → reported={ms}ms"
                                    );
                                }
                                a.base_ms = ms;
                                a.base_instant = Instant::now();
                            }
                        }
                        // 否则（Chromium 卡住）保持纯外推，不动 anchor
                    }
                    _ => {}
                }

                // 当前歌词行
                if !lyrics.is_empty() {
                    let pos = anchor.as_ref().map(|a| a.current(offset_ms)).unwrap_or(0).max(0) as u64;
                    let idx = lrc_parser::current_index(&lyrics, pos);
                    if idx != last_line_idx {
                        let current = idx.map(|i| lyrics[i].text.clone()).unwrap_or_default();
                        let next = idx.and_then(|i| lyrics.get(i + 1).map(|l| l.text.clone()));
                        let (start_ms, end_ms) = match idx {
                            Some(i) => {
                                let s = lyrics[i].time_ms;
                                let e = lyrics.get(i + 1).map(|l| l.time_ms).unwrap_or(s + 5000);
                                (s, e.max(s + 1))
                            }
                            None => (0, 0),
                        };
                        let _ = ui_tx
                            .send(UiEvent::LineChanged {
                                current,
                                next,
                                start_ms,
                                end_ms,
                                pos_ms: pos,
                            })
                            .await;
                        last_line_idx = idx;
                    }
                }
            }
        }
    }
}
