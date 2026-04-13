use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub struct Fetcher {
    client: Client,
    cache_dir: PathBuf,
    sources: Vec<String>,
}

impl Fetcher {
    pub fn new() -> Result<Self> {
        Self::with_sources(vec!["lrclib".into(), "netease".into(), "qq".into()])
    }

    pub fn with_sources(sources: Vec<String>) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(UA)
            .build()?;
        let proj = directories::ProjectDirs::from("com", "github", "wayland-float-lyrics")
            .ok_or_else(|| anyhow!("无法定位 XDG cache 目录"))?;
        let cache_dir = proj.cache_dir().to_path_buf();
        std::fs::create_dir_all(&cache_dir)?;
        tracing::debug!("cache dir: {}  sources={:?}", cache_dir.display(), sources);
        Ok(Self {
            client,
            cache_dir,
            sources,
        })
    }

    pub fn cache_dir(&self) -> &PathBuf {
        &self.cache_dir
    }

    pub async fn fetch(&self, artist: &str, title: &str) -> Result<Option<String>> {
        let path = self.cache_path(artist, title);
        if let Ok(s) = tokio::fs::read_to_string(&path).await {
            tracing::info!("cache hit: {}", path.display());
            return Ok(Some(s));
        }

        for source in &self.sources {
            let result = match source.as_str() {
                "lrclib" => self.try_lrclib(artist, title).await,
                "netease" => self.try_netease(artist, title).await,
                "qq" => self.try_qq(artist, title).await,
                other => {
                    tracing::warn!("未知歌词源：{other}");
                    continue;
                }
            };
            match result {
                Ok(Some(lrc)) => {
                    tracing::info!("{source} hit ({} bytes)", lrc.len());
                    let _ = tokio::fs::write(&path, &lrc).await;
                    return Ok(Some(lrc));
                }
                Ok(None) => tracing::info!("{source} miss"),
                Err(e) => tracing::warn!("{source} error: {e}"),
            }
        }
        Ok(None)
    }

    fn cache_path(&self, artist: &str, title: &str) -> PathBuf {
        let key = sanitize(&format!("{artist}__{title}"));
        self.cache_dir.join(format!("{key}.lrc"))
    }

    async fn try_lrclib(&self, artist: &str, title: &str) -> Result<Option<String>> {
        #[derive(Deserialize)]
        struct Item {
            #[serde(rename = "syncedLyrics")]
            synced_lyrics: Option<String>,
            #[serde(rename = "plainLyrics")]
            plain_lyrics: Option<String>,
        }
        let resp = self
            .client
            .get("https://lrclib.net/api/search")
            .query(&[("artist_name", artist), ("track_name", title)])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let items: Vec<Item> = resp.json().await?;
        for it in &items {
            if let Some(s) = it.synced_lyrics.as_deref().filter(|s| !s.is_empty()) {
                return Ok(Some(s.to_string()));
            }
        }
        for it in items {
            if let Some(s) = it.plain_lyrics.filter(|s| !s.is_empty()) {
                return Ok(Some(s));
            }
        }
        Ok(None)
    }

    async fn try_netease(&self, artist: &str, title: &str) -> Result<Option<String>> {
        #[derive(Deserialize)]
        struct SearchResp {
            result: Option<SearchResult>,
        }
        #[derive(Deserialize)]
        struct SearchResult {
            songs: Option<Vec<Song>>,
        }
        #[derive(Deserialize)]
        struct Song {
            id: u64,
        }
        #[derive(Deserialize)]
        struct LyricResp {
            lrc: Option<Lrc>,
        }
        #[derive(Deserialize)]
        struct Lrc {
            lyric: Option<String>,
        }

        let q = if artist.is_empty() {
            title.to_string()
        } else {
            format!("{artist} {title}")
        };
        let resp = self
            .client
            .get("https://music.163.com/api/search/get")
            .header("Referer", "https://music.163.com")
            .query(&[("s", q.as_str()), ("type", "1"), ("limit", "5")])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let s: SearchResp = resp.json().await?;
        let id = s
            .result
            .and_then(|r| r.songs)
            .and_then(|v| v.into_iter().next())
            .map(|s| s.id);
        let Some(id) = id else {
            return Ok(None);
        };

        let id_str = id.to_string();
        let resp = self
            .client
            .get("https://music.163.com/api/song/lyric")
            .header("Referer", "https://music.163.com")
            .query(&[("id", id_str.as_str()), ("lv", "1"), ("kv", "1"), ("tv", "-1")])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let lr: LyricResp = resp.json().await?;
        Ok(lr.lrc.and_then(|l| l.lyric).filter(|s| !s.is_empty()))
    }

    async fn try_qq(&self, artist: &str, title: &str) -> Result<Option<String>> {
        #[derive(Deserialize)]
        struct SearchResp {
            data: Option<SearchData>,
        }
        #[derive(Deserialize)]
        struct SearchData {
            song: Option<SongList>,
        }
        #[derive(Deserialize)]
        struct SongList {
            list: Option<Vec<SongItem>>,
        }
        #[derive(Deserialize)]
        struct SongItem {
            songmid: Option<String>,
        }
        #[derive(Deserialize)]
        struct LyricResp {
            lyric: Option<String>,
        }

        let q = if artist.is_empty() {
            title.to_string()
        } else {
            format!("{artist} {title}")
        };
        let resp = self
            .client
            .get("https://c.y.qq.com/soso/fcgi-bin/client_search_cp")
            .header("Referer", "https://y.qq.com")
            .query(&[
                ("format", "json"),
                ("w", q.as_str()),
                ("n", "5"),
                ("p", "1"),
            ])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let body = resp.text().await?;
        let sr: SearchResp = serde_json::from_str(strip_jsonp(&body))?;
        let mid = sr
            .data
            .and_then(|d| d.song)
            .and_then(|s| s.list)
            .and_then(|v| v.into_iter().find_map(|it| it.songmid))
            .filter(|s| !s.is_empty());
        let Some(mid) = mid else {
            return Ok(None);
        };

        let resp = self
            .client
            .get("https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg")
            .header("Referer", "https://y.qq.com")
            .query(&[
                ("format", "json"),
                ("nobase64", "1"),
                ("songmid", mid.as_str()),
                ("g_tk", "5381"),
            ])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let body = resp.text().await?;
        let lr: LyricResp = serde_json::from_str(strip_jsonp(&body))?;
        Ok(lr.lyric.filter(|s| !s.is_empty()))
    }
}

/// 剥离 JSONP 包裹（如 `MusicJsonCallback(...)`），返回内部 JSON；没有包裹则原样返回。
fn strip_jsonp(body: &str) -> &str {
    let t = body.trim();
    if t.ends_with(')') {
        if let Some(open) = t.find('(') {
            if open > 0 {
                return t[open + 1..t.len() - 1].trim();
            }
        }
    }
    t
}

/// 把歌手/歌名里非文件名安全字符替换成 `_`，过长则截断并追加 hash 避免冲突。
fn sanitize(s: &str) -> String {
    let replaced: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut out = String::with_capacity(replaced.len());
    let mut prev_us = false;
    for c in replaced.chars() {
        if c == '_' && prev_us {
            continue;
        }
        prev_us = c == '_';
        out.push(c);
    }
    let out = out.trim_matches('_').to_string();
    if out.chars().count() > 80 {
        let hash = simple_hash(s);
        let truncated: String = out.chars().take(60).collect();
        format!("{truncated}_{hash:x}")
    } else if out.is_empty() {
        format!("unknown_{:x}", simple_hash(s))
    } else {
        out
    }
}

fn simple_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_utf8() {
        let s = sanitize("周杰伦/晴天");
        assert!(!s.contains('/'));
        assert!(s.contains("周杰伦") || s.contains("晴天"));
    }

    #[test]
    fn sanitize_collapses_underscores() {
        assert_eq!(sanitize("a//b\\\\c"), "a_b_c");
    }

    #[test]
    fn sanitize_truncates_long() {
        let long = "a".repeat(200);
        let s = sanitize(&long);
        assert!(s.chars().count() <= 80);
    }
}
