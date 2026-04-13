use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    pub time_ms: u64,
    pub text: String,
}

fn tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?\]").unwrap())
}

/// 解析 LRC 文本为按时间排序的 `Vec<LyricLine>`。
/// - 支持 `[mm:ss.xx]` / `[mm:ss.xxx]` / `[mm:ss]` / `[mm:ss:xx]`
/// - 支持一行多时间标签（翻唱/循环复用）
/// - 忽略 `[ti:...]` `[ar:...]` `[al:...]` `[by:...]` `[offset:...]` 等元数据
pub fn parse(input: &str) -> Vec<LyricLine> {
    let re = tag_re();
    let mut out: Vec<LyricLine> = Vec::new();
    for raw in input.lines() {
        let mut cursor = 0usize;
        let mut times: Vec<u64> = Vec::new();
        for caps in re.captures_iter(raw) {
            let m = caps.get(0).unwrap();
            if m.start() != cursor {
                break;
            }
            cursor = m.end();
            let min: u64 = caps[1].parse().unwrap_or(0);
            let sec: u64 = caps[2].parse().unwrap_or(0);
            let frac = caps.get(3).map(|m| {
                let s = m.as_str();
                let v: u64 = s.parse().unwrap_or(0);
                match s.len() {
                    1 => v * 100,
                    2 => v * 10,
                    _ => v,
                }
            }).unwrap_or(0);
            times.push(min * 60_000 + sec * 1_000 + frac);
        }
        if times.is_empty() {
            continue;
        }
        let text = raw[cursor..].trim().to_string();
        for t in times {
            out.push(LyricLine {
                time_ms: t,
                text: text.clone(),
            });
        }
    }
    out.sort_by_key(|l| l.time_ms);
    out.dedup_by(|a, b| a.time_ms == b.time_ms && a.text == b.text);
    out
}

/// 返回最后一个 `time_ms <= position_ms` 的下标；全部晚于位置则返回 `None`。
pub fn current_index(lines: &[LyricLine], position_ms: u64) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    let idx = lines.partition_point(|l| l.time_ms <= position_ms);
    if idx == 0 {
        None
    } else {
        Some(idx - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        let s = "[00:01.00]hello\n[00:02.50]world\n";
        let lines = parse(s);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].time_ms, 1000);
        assert_eq!(lines[0].text, "hello");
        assert_eq!(lines[1].time_ms, 2500);
    }

    #[test]
    fn parse_multi_tag() {
        let s = "[00:01.00][01:01.00]chorus";
        let lines = parse(s);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].time_ms, 1000);
        assert_eq!(lines[1].time_ms, 61000);
        assert_eq!(lines[1].text, "chorus");
    }

    #[test]
    fn parse_skips_metadata() {
        let s = "[ti:title]\n[ar:artist]\n[00:05.00]real line";
        let lines = parse(s);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "real line");
    }

    #[test]
    fn parse_three_digit_ms() {
        let s = "[00:01.123]x";
        let lines = parse(s);
        assert_eq!(lines[0].time_ms, 1123);
    }

    #[test]
    fn current_index_works() {
        let lines = vec![
            LyricLine { time_ms: 1000, text: "a".into() },
            LyricLine { time_ms: 2000, text: "b".into() },
            LyricLine { time_ms: 3000, text: "c".into() },
        ];
        assert_eq!(current_index(&lines, 500), None);
        assert_eq!(current_index(&lines, 1000), Some(0));
        assert_eq!(current_index(&lines, 1999), Some(0));
        assert_eq!(current_index(&lines, 2500), Some(1));
        assert_eq!(current_index(&lines, 9999), Some(2));
    }
}
