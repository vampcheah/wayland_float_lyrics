use crate::lrc_parser::LyricLine;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum UiEvent {
    SongChanged { title: String, artist: String },
    LyricsLoaded(Vec<LyricLine>),
    LyricsNotFound,
    LineChanged {
        current: String,
        next: Option<String>,
        /// 当前行 LRC 时间戳（ms）
        start_ms: u64,
        /// 下一行起始 / 当前行结束（ms）；无下一行时 = start_ms + 5000
        end_ms: u64,
        /// 事件发送时的歌曲位置（ms），UI 用 Instant::now 做锚点外推
        pos_ms: u64,
    },
    PlaybackState(bool),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Command {
    TogglePause,
    AdjustOffset(i64),
    Quit,
}
