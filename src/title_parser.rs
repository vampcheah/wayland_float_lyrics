use regex::Regex;
use std::sync::OnceLock;

/// 清洗 YouTube / 浏览器 MPRIS 里的脏标题，尝试拆出 (artist, title)。
/// 返回可能的多个候选（按置信度从高到低），调用方可用于依次尝试歌词搜索。
pub fn parse(raw_title: &str, raw_artist: &str) -> Vec<(String, String)> {
    let cleaned = clean_noise(raw_title);
    let mut cands: Vec<(String, String)> = Vec::new();

    // 按优先级拆分：常见分隔符
    for sep in [" - ", " – ", " — ", " : ", "｜", " | ", "—", "–"] {
        if let Some(idx) = cleaned.find(sep) {
            let left = cleaned[..idx].trim().to_string();
            let right = cleaned[idx + sep.len()..].trim().to_string();
            if left.is_empty() || right.is_empty() {
                continue;
            }
            // 若其中一边包含 channel 名（raw_artist），另一边当 title
            let ra = raw_artist.to_lowercase();
            if !ra.is_empty() {
                if left.to_lowercase().contains(&ra) {
                    cands.push((raw_artist.to_string(), right.clone()));
                    continue;
                }
                if right.to_lowercase().contains(&ra) {
                    cands.push((raw_artist.to_string(), left.clone()));
                    continue;
                }
            }
            cands.push((left, right));
        }
    }

    // 中日韩专用括号：「」『』【】《》〈〉
    for (open, close) in [("「", "」"), ("『", "』"), ("【", "】"), ("《", "》"), ("〈", "〉")] {
        if let (Some(a), Some(b)) = (cleaned.find(open), cleaned.find(close)) {
            if b > a + open.len() {
                let title = cleaned[a + open.len()..b].trim().to_string();
                let mut artist = cleaned[..a].trim().trim_end_matches(|c: char| !c.is_alphanumeric()).to_string();
                if artist.is_empty() {
                    artist = raw_artist.to_string();
                }
                if !title.is_empty() {
                    cands.push((artist, title));
                }
            }
        }
    }

    // 兜底：清洗后的全标题 + 原 artist
    cands.push((raw_artist.to_string(), cleaned.clone()));
    // 再兜底：完全不带 artist 的 query
    cands.push((String::new(), cleaned));

    // 去重（保持顺序）
    let mut seen = std::collections::HashSet::new();
    cands.retain(|(a, t)| seen.insert(format!("{a}||{t}")));
    cands
}

fn noise_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            // 括号/方括号/中文括号里含噪音词 → 整块删
            Regex::new(r"(?i)\s*[\(\[【（]\s*[^\)\]】）]*?(official|mv|m/v|audio|lyric[s]?\s*video|lyric[s]?|hd|4k|8k|1080p|720p|remaster(ed)?|live|concert|performance|cover|visualizer|explicit|clean|radio\s*edit|ver\.?|version)[^\)\]】）]*\s*[\)\]】）]").unwrap(),
            // 中文噪音
            Regex::new(r"\s*[\(\[【（]?\s*(官方(高畫質|高清|MV|Music Video|MV)?|高畫質|高清|官方影音|官方音訊)\s*[\)\]】）]?").unwrap(),
            // 尾部悬挂的噪音词（前面是 - | — 等分隔或直接空格）
            Regex::new(r"(?i)\s*[-|｜–—:]\s*(official\s*(music\s*video|video|mv|audio|lyric\s*video)?|music\s*video|m/v|mv|hd|4k|lyrics?)\s*$").unwrap(),
            Regex::new(r"(?i)\s+(official|mv|m/v|hd|4k|1080p|720p|audio|live|remaster(ed)?)\s*$").unwrap(),
            // 多余空白
            Regex::new(r"\s{2,}").unwrap(),
        ]
    })
}

pub fn clean_noise(raw: &str) -> String {
    // 反复清洗直到稳定：尾部多个噪音词（Official HD MV）需多轮才能全部剥掉
    let mut s = raw.to_string();
    for _ in 0..6 {
        let before = s.clone();
        for re in noise_patterns() {
            s = re.replace_all(&s, " ").to_string();
        }
        if s == before {
            break;
        }
    }
    s.trim_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '|' | '｜' | '—' | '–' | ':'))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_official_mv() {
        assert_eq!(
            clean_noise("Adele - Hello (Official Music Video)"),
            "Adele - Hello"
        );
        assert_eq!(
            clean_noise("周杰倫 Jay Chou - 晴天 Sunny Day -Official Music Video"),
            "周杰倫 Jay Chou - 晴天 Sunny Day"
        );
    }

    #[test]
    fn cleans_chinese_noise() {
        assert_eq!(
            clean_noise("毛不易Mao Buyi - 像我這樣的人 官方高畫質 Official HD MV"),
            "毛不易Mao Buyi - 像我這樣的人"
        );
    }

    #[test]
    fn splits_dash() {
        let c = parse("Adele - Hello (Official Music Video)", "AdeleVEVO");
        assert!(c.iter().any(|(a, t)| a == "Adele" && t == "Hello"));
    }

    #[test]
    fn strips_channel_side() {
        let c = parse(
            "Dreamer Music MV 毛不易Mao Buyi - 像我這樣的人 官方高畫質 Official HD MV",
            "Dreamer Music MV",
        );
        assert!(c.iter().any(|(a, t)| a == "Dreamer Music MV" && t.contains("像我這樣的人")));
    }

    #[test]
    fn chinese_brackets() {
        let c = parse("周杰倫《晴天》", "");
        assert!(c.iter().any(|(_, t)| t == "晴天"));
    }
}
