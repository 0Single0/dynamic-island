//! 歌词：请求、LRC 解析、磁盘缓存。
//!
//! 展示策略（与方案一致）：原文逐行为主；无原文但有翻译时主行用翻译；
//! 都没有则报告无可同步歌词。lrc 为空不算没歌词（tlyric 可能可用）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::Value;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LyricLine {
    pub start_ms: u64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Lyrics {
    pub kind: LyricsKind,
    pub lines: Vec<LyricLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LyricsKind {
    #[default]
    Original,
    Translation,
    None,
}

fn cache_dir() -> PathBuf {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    PathBuf::from(local)
        .join("dynamic-island")
        .join("cache")
        .join("netease")
        .join("lyrics")
}

/// 内存 + 磁盘双层缓存，避免重复请求歌词服务。
static MEM_LOCK: Mutex<Option<HashMap<String, Lyrics>>> = Mutex::new(None);

fn cache_get(song_id: &str) -> Option<Lyrics> {
    let mut mem = MEM_LOCK.lock().unwrap();
    if let Some(map) = &*mem {
        if let Some(hit) = map.get(song_id) {
            return Some(hit.clone());
        }
    }
    let file = cache_dir().join(format!("{song_id}.json"));
    let Ok(text) = std::fs::read_to_string(file) else {
        return None;
    };
    let lyrics: Lyrics = serde_json::from_str(&text).ok()?;
    mem.get_or_insert_with(HashMap::new)
        .insert(song_id.to_string(), lyrics.clone());
    Some(lyrics)
}

fn cache_put(song_id: &str, lyrics: &Lyrics) {
    let dir = cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(text) = serde_json::to_string(lyrics) {
        let _ = std::fs::write(dir.join(format!("{song_id}.json")), text);
    }
    let mut mem = MEM_LOCK.lock().unwrap();
    mem.get_or_insert_with(HashMap::new)
        .insert(song_id.to_string(), lyrics.clone());
}

pub async fn fetch(song_id: &str) -> Option<Lyrics> {
    if let Some(hit) = cache_get(song_id) {
        return Some(hit);
    }

    let url = format!("https://music.163.com/api/song/lyric?id={song_id}&lv=1&kv=1&tv=-1");
    let client = reqwest::Client::new();
    let value: Value = client
        .get(&url)
        .header("Referer", "https://music.163.com/")
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    let lrc = parse_lrc(value["lrc"]["lyric"].as_str().unwrap_or(""));
    let tl = parse_lrc(value["tlyric"]["lyric"].as_str().unwrap_or(""));

    let (kind, mut lines) = if !lrc.is_empty() {
        (LyricsKind::Original, lrc)
    } else if !tl.is_empty() {
        (LyricsKind::Translation, tl.clone())
    } else {
        return Some(Lyrics {
            kind: LyricsKind::None,
            lines: Vec::new(),
        });
    };

    if kind == LyricsKind::Original && !tl.is_empty() {
        // 翻译按时间戳挂到主行
        let tl_map: HashMap<u64, String> = tl.into_iter().map(|l| (l.start_ms, l.text)).collect();
        for line in &mut lines {
            if let Some(t) = tl_map.get(&line.start_ms) {
                line.translation = Some(t.clone());
            }
        }
    }

    let lyrics = Lyrics { kind, lines };
    cache_put(song_id, &lyrics);
    Some(lyrics)
}

/// 解析 LRC 文本：多时间标签拆分、兼容 mm:ss[.S{1,3}]、忽略元信息行。
pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut lines: Vec<LyricLine> = Vec::new();
    let re_time = |tag: &str| -> Option<u64> {
        let inner = tag.trim_start_matches('[').trim_end_matches(']');
        // 忽略 [by:] [ar:] [al:] 等元信息
        let (mm, rest) = inner.split_once(':')?;
        if mm.len() > 2 || !mm.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let mm: u64 = mm.parse().ok()?;
        let (ss, ms_str) = match rest.split_once('.') {
            Some((s, ms)) => (s.parse::<u64>().ok()?, ms.to_string()),
            None => (rest.parse::<u64>().ok()?, String::new()),
        };
        let mut ms = ms_str.as_str();
        let mut scale = 100u64;
        let mut value = 0u64;
        while let Some(d) = ms.chars().next() {
            if !d.is_ascii_digit() {
                break;
            }
            value += d.to_digit(10).unwrap() as u64 * scale;
            scale /= 10;
            ms = &ms[1..];
        }
        Some(mm * 60_000 + ss * 1000 + value)
    };

    for raw in text.lines() {
        let mut times: Vec<u64> = Vec::new();
        let mut content = raw;
        loop {
            let trimmed = content.trim_start();
            if !trimmed.starts_with('[') {
                break;
            }
            let Some(end) = trimmed.find(']') else { break };
            let tag = &trimmed[..=end];
            if let Some(ms) = re_time(tag) {
                times.push(ms);
            }
            content = &trimmed[end + 1..];
        }
        let content = content.trim();
        if times.is_empty() {
            continue;
        }
        for start_ms in times {
            lines.push(LyricLine {
                start_ms,
                text: content.to_string(),
                translation: None,
            });
        }
    }

    lines.sort_by_key(|l| l.start_ms);
    lines
}

/// 二分查找当前行索引：最大的 start_ms <= position。
pub fn current_index(lines: &[LyricLine], position_ms: u64) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    let mut low = 0usize;
    let mut high = lines.len();
    while low < high {
        let mid = (low + high) / 2;
        if lines[mid].start_ms <= position_ms {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    if low == 0 {
        None
    } else {
        Some(low - 1)
    }
}
