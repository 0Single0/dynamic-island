//! 本地播放列表（playingList）读取与曲目匹配。
//!
//! 文件由网易云持有写入，读取失败时按 50/100/250ms 短重试三次，不长期占用。

use std::path::PathBuf;
use std::sync::RwLock;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
struct Track {
    id: String,
    #[serde(default)]
    duration: u64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    artists: Vec<Artist>,
    #[serde(default)]
    album: Album,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Artist {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Album {
    #[serde(default)]
    name: String,
    #[serde(default)]
    pic_url: Option<String>,
    // 客户端 JSON 里 picUrl 与 cover 同时存在且值相同；
    // 同一字段用 alias 会让 serde 报 duplicate field，必须拆成两个字段
    #[serde(default)]
    cover: Option<String>,
}

impl Album {
    fn pic(&self) -> String {
        self.pic_url
            .clone()
            .or_else(|| self.cover.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ListEntry {
    #[serde(default)]
    track: Option<Track>,
}

#[derive(Debug, Deserialize)]
struct PlayingList {
    #[serde(default)]
    list: Vec<ListEntry>,
}

/// 匹配成功后给同步循环使用的曲目元数据。
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrackMeta {
    pub song_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover_url: String,
    pub duration_ms: u64,
}

fn playing_list_path() -> PathBuf {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    PathBuf::from(local)
        .join("Netease")
        .join("CloudMusic")
        .join("webdata")
        .join("file")
        .join("playingList")
}

fn read_once() -> anyhow::Result<Vec<Track>> {
    let text = std::fs::read_to_string(playing_list_path())?;
    let parsed: PlayingList = serde_json::from_str(&text)?;
    Ok(parsed.list.into_iter().filter_map(|e| e.track).collect())
}

/// 短重试读取（50/100/250ms）。
fn read_with_retry() -> Vec<Track> {
    for delay in [50u64, 100, 250] {
        match read_once() {
            Ok(tracks) if !tracks.is_empty() => return tracks,
            _ => std::thread::sleep(std::time::Duration::from_millis(delay)),
        }
    }
    read_once().unwrap_or_default()
}

/// 进程内缓存：文件 2.7MB，不必每次轮询都读盘。
fn cached_tracks() -> Vec<Track> {
    static CACHE: RwLock<Option<Vec<Track>>> = RwLock::new(None);
    let mut guard = CACHE.write().unwrap();
    match &*guard {
        Some(t) => t.clone(),
        None => {
            let tracks = read_with_retry();
            *guard = Some(tracks.clone());
            tracks
        }
    }
}

/// 文本归一化：全半角空格统一、全角括号转半角、去首尾空白、小写比较。
pub fn normalize(s: &str) -> String {
    s.trim()
        .replace('\u{3000}', " ")
        .replace('（', "(")
        .replace('）', ")")
        .to_lowercase()
}

/// 供指纹使用的归一化别名。
pub fn normalize_public(s: &str) -> String {
    normalize(s)
}

/// 方案中的打分匹配；返回 (分数, 元数据)。
fn score(track: &Track, title: &str, artist: &str, duration_ms: u64) -> i32 {
    let mut score = 0;
    let t_norm = normalize(title);
    let c_norm = normalize(&track.name);
    if !t_norm.is_empty() {
        if t_norm == c_norm {
            score += 60;
        } else if c_norm.contains(&t_norm) || t_norm.contains(&c_norm) {
            score += 25;
        }
    }

    // 快照歌手常是「A / B」「A,B」连接串，拆成单名逐一比对，取最好的结果
    let snapshot_artists: Vec<String> = artist
        .split(['/', ',', '&', '、'])
        .map(normalize)
        .filter(|s| !s.is_empty())
        .collect();
    let track_artists: Vec<String> = track.artists.iter().map(|a| normalize(&a.name)).collect();
    if !snapshot_artists.is_empty() {
        let exact = snapshot_artists
            .iter()
            .any(|s| track_artists.iter().any(|a| a == s));
        if exact {
            score += 30;
        } else {
            let partial = snapshot_artists.iter().any(|s| {
                track_artists
                    .iter()
                    .any(|a| (a.contains(s) || s.contains(a)) && !a.is_empty())
            });
            if partial {
                score += 20;
            }
        }
    }

    // 快照可能拿不到时长（迷你播放条没有时间文本）而为 0，
    // 此时跳过时长评分，否则必然吃满 -40 惩罚导致永远无法匹配。
    if duration_ms > 0 {
        let delta = track.duration.abs_diff(duration_ms) as i64;
        if delta <= 1000 {
            score += 25;
        } else if delta <= 3000 {
            score += 15;
        } else if delta > 8000 {
            score -= 40;
        }
    }
    score
}

/// 标题精确匹配的曲数；用于区分「同名同曲」与「同名不同曲」。
fn count_exact_titles(tracks: &[Track], title: &str) -> usize {
    let t_norm = normalize(title);
    if t_norm.is_empty() {
        return 0;
    }
    tracks
        .iter()
        .filter(|t| normalize(&t.name) == t_norm)
        .count()
}

/// 按“标题+歌手+时长”在本地播放列表中精确匹配；达不到门槛返回 None。
pub fn resolve(title: &str, artist: &str, duration_ms: u64) -> Option<TrackMeta> {
    let tracks = cached_tracks();
    let mut scored: Vec<(i32, &Track)> = tracks
        .iter()
        .map(|t| (score(t, title, artist, duration_ms), t))
        .collect();
    scored.sort_by_key(|(s, _)| -*s);

    let Some((best_score, best)) = scored.first() else {
        return None;
    };
    // 门槛 75 分；但迷你条拿不到时长（duration=0 时跳过时长评分），
    // 标题精确同名时保底 60 分（标题60+歌手0 也能过）。同曲不同版本
    // （Live/Remix）名称必带后缀，不会与正曲撞车。
    let unique_exact_title =
        count_exact_titles(&tracks, title) == 1 && normalize(&best.name) == normalize(title);
    let threshold = if unique_exact_title { 60 } else { 75 };
    if *best_score < threshold {
        eprintln!(
            "[netease] resolve miss: score {best_score} < {threshold} for {:?}/{:?}/{} -> id {}",
            title, artist, duration_ms, best.id
        );
        return None;
    }
    // 领先第二名 15 分才算唯一；同分同名（不同版本）交给标题唯一性兜底
    if let Some((second, _)) = scored.get(1) {
        if best_score - second >= 15 {
            return Some(TrackMeta {
                song_id: best.id.clone(),
                title: best.name.clone(),
                artist: best
                    .artists
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join("/"),
                album: best.album.name.clone(),
                cover_url: best.album.pic(),
                duration_ms: best.duration,
            });
        }
        // 同分：仅当标题精确唯一时接受，否则视为歧义
        if !(unique_exact_title && *second == *best_score) {
            eprintln!(
                "[netease] resolve ambiguous: score {best_score} vs second {second} for {:?}/{:?}",
                title, artist
            );
            return None;
        }
    }

    Some(TrackMeta {
        song_id: best.id.clone(),
        title: best.name.clone(),
        artist: best
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join("/"),
        album: best.album.name.clone(),
        cover_url: best.album.pic(),
        duration_ms: best.duration,
    })
}
