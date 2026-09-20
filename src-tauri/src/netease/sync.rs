//! 同步主循环：250ms CDP 轮询 → 曲目匹配 → 歌词同步 → `netease-state` 事件。

use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use tauri::Emitter;

use super::cdp::{CdpSession, PlayerSnapshot};
use super::lyrics::{Lyrics, LyricsKind};
use super::playlist::TrackMeta;

pub const POLL_INTERVAL_MS: u64 = 250;
/// 指纹需连续两次一致才确认切歌，避免页面过渡渲染误判。
const FINGERPRINT_CONFIRM_ROUNDS: u32 = 2;
/// 歌词显示偏移（ms），补偿页面时间文本采样延迟；可由设置调整。
static LYRIC_OFFSET_MS: AtomicU64 = AtomicU64::new(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionState {
    Idle,
    NeedsRestart,
    Starting,
    WaitingForCdp,
    Synchronizing,
    Degraded,
    Error,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsState {
    pub kind: LyricsKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_translation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_text: Option<String>,
    /// 当前行前后各三行，供前端渲染网易云式的居中滚动歌词区。
    pub visible_lines: Vec<LyricDisplayLine>,
    pub offset_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricDisplayLine {
    /// Lyric start time, used by the frontend as a stable row key.
    pub start_ms: u64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseState {
    pub connection: ConnectionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<TrackState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lyrics: Option<LyricsState>,
    pub is_playing: bool,
    pub position_ms: u64,
    pub sampled_at_epoch_ms: u64,
}

pub struct SyncRuntime {
    app: tauri::AppHandle,
    inner: std::sync::Mutex<SyncInner>,
}

struct SyncInner {
    state: NeteaseState,
    lyrics: Option<Lyrics>,
    fingerprint: Option<String>,
    fingerprint_rounds: u32,
    confirmed_song_id: Option<String>,
}

impl SyncRuntime {
    pub fn new(app: tauri::AppHandle) -> Self {
        let state = NeteaseState {
            connection: ConnectionState::Idle,
            message: None,
            track: None,
            lyrics: None,
            is_playing: false,
            position_ms: 0,
            sampled_at_epoch_ms: 0,
        };
        Self {
            app,
            inner: std::sync::Mutex::new(SyncInner {
                state,
                lyrics: None,
                fingerprint: None,
                fingerprint_rounds: 0,
                confirmed_song_id: None,
            }),
        }
    }

    /// 推送状态到前端。播放进度本身就是可见状态：若跳过位置变化事件，
    /// 前端只能从首个快照插值，5 秒保护一触发就会停在旧时间。
    pub fn publish(&self, state: NeteaseState) {
        self.inner.lock().unwrap().state = state.clone();
        let _ = self.app.emit("netease-state", &state);
    }

    pub fn current(&self) -> NeteaseState {
        self.inner.lock().unwrap().state.clone()
    }

    pub fn set_lyric_offset(&self, offset_ms: u64) {
        LYRIC_OFFSET_MS.store(offset_ms, Ordering::Relaxed);
        // 立即用新偏移重算一次当前歌词
        let mut inner = self.inner.lock().unwrap();
        let position_ms = inner.state.position_ms;
        if let Some(lyrics) = &inner.lyrics {
            let lyrics_state = build_lyrics_state(lyrics, position_ms);
            inner.state.lyrics = Some(lyrics_state.clone());
            let _ = self.app.emit("netease-state", &inner.state);
        }
    }

    fn reset_track(&mut self) {
        let inner = self.inner.get_mut().unwrap();
        inner.lyrics = None;
        inner.fingerprint = None;
        inner.fingerprint_rounds = 0;
        inner.confirmed_song_id = None;
    }
}

/// 轮询一次：读快照 → 指纹确认切歌 → 本地匹配 → 歌词定位 → 推送。
pub async fn poll_once(runtime: &SyncRuntime, session: &mut CdpSession) -> anyhow::Result<()> {
    let PlayerSnapshot {
        available,
        title,
        artist,
        duration_ms: snapshot_duration_ms,
        cover_art,
        is_playing,
        position_ms: snapshot_position_ms,
        ..
    } = session.read_snapshot().await?;

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    if !available {
        let state = NeteaseState {
            connection: ConnectionState::Degraded,
            message: Some("播放器元素不可用，网易云客户端界面可能已更新".into()),
            track: None,
            lyrics: None,
            is_playing,
            position_ms: snapshot_position_ms,
            sampled_at_epoch_ms: now_ms,
        };
        runtime.publish(state);
        return Ok(());
    }

    if title.is_empty() {
        let state = NeteaseState {
            connection: ConnectionState::Synchronizing,
            message: None,
            track: None,
            lyrics: None,
            is_playing,
            position_ms: 0,
            sampled_at_epoch_ms: now_ms,
        };
        runtime.publish(state);
        return Ok(());
    }

    // —— 曲目指纹：连续两轮一致才确认切歌 ——
    let fingerprint =
        format!("{}|{}", super::playlist::normalize_public(&title), snapshot_duration_ms);
    let confirmed = {
        let mut inner = runtime.inner.lock().unwrap();
        if inner.fingerprint.as_deref() == Some(fingerprint.as_str()) {
            inner.fingerprint_rounds = inner.fingerprint_rounds.max(1) + 1;
        } else {
            inner.fingerprint = Some(fingerprint);
            inner.fingerprint_rounds = 1;
        }
        inner.fingerprint_rounds >= FINGERPRINT_CONFIRM_ROUNDS
    };

    // 本地匹配（仅在指纹确认后执行，匹配是 CPU 工作，放到阻塞线程池）
    let meta: Option<TrackMeta> = if confirmed {
        let title = title.clone();
        let artist = artist.clone();
        tokio::task::spawn_blocking(move || {
            super::playlist::resolve(&title, &artist, snapshot_duration_ms)
        })
        .await
        .unwrap_or(None)
    } else {
        None
    };

    // 旧版页面只能给进度比例（0..1000）而没有时长；一旦通过本地播放
    // 列表取得真实时长，就把比例还原为毫秒。当前版本则直接使用时间文本。
    let effective_duration_ms = meta
        .as_ref()
        .map(|m| m.duration_ms)
        .filter(|_| snapshot_duration_ms == 0)
        .unwrap_or(snapshot_duration_ms);
    let effective_position_ms = if snapshot_duration_ms == 0
        && snapshot_position_ms <= 1000
        && effective_duration_ms > 0
    {
        snapshot_position_ms.saturating_mul(effective_duration_ms) / 1000
    } else {
        snapshot_position_ms
    };

    // 歌词：只在 songId 变化时拉取
    {
        let mut inner = runtime.inner.lock().unwrap();
        let new_song_id = meta.as_ref().map(|m| m.song_id.clone());
        if new_song_id != inner.confirmed_song_id {
            inner.lyrics = None;
            inner.confirmed_song_id = new_song_id.clone();
        }
    }
    if let Some(id) = meta.as_ref().map(|m| m.song_id.clone()) {
        let has_lyrics = runtime.inner.lock().unwrap().lyrics.is_some();
        if !has_lyrics {
            let fetched = super::lyrics::fetch(&id).await;
            runtime.inner.lock().unwrap().lyrics = fetched;
        }
    }

    // —— 组装推送状态 ——
    let (connection, track_state, lyrics_state) = match &meta {
        Some(m) => {
            let inner_lyrics = runtime.inner.lock().unwrap().lyrics.clone();
            let lyrics_state =
                inner_lyrics
                    .as_ref()
                    .map(|l| build_lyrics_state(l, effective_position_ms));
            (
                if inner_lyrics.is_some() {
                    ConnectionState::Synchronizing
                } else {
                    ConnectionState::Degraded
                },
                TrackState {
                    song_id: Some(m.song_id.clone()),
                    title: Some(m.title.clone()),
                    artist: Some(m.artist.clone()),
                    album: Some(m.album.clone()),
                    cover_url: {
                        let local = if m.cover_url.is_empty() {
                            cover_art.clone()
                        } else {
                            m.cover_url.clone()
                        };
                        if local.is_empty() { None } else { Some(local) }
                    },
                    duration_ms: Some(if m.duration_ms > 0 {
                        m.duration_ms
                    } else {
                        effective_duration_ms
                    }),
                },
                lyrics_state,
            )
        }
        None => (
            // 未精确匹配：显示快照信息但绝不加载可能错误的歌词
            ConnectionState::Degraded,
            TrackState {
                song_id: None,
                title: Some(title.clone()),
                artist: Some(artist.clone()),
                album: None,
                cover_url: if cover_art.is_empty() {
                    None
                } else {
                    Some(cover_art.clone())
                },
                duration_ms: Some(effective_duration_ms),
            },
            None,
        ),
    };

    let state = NeteaseState {
        connection,
        message: None,
        track: Some(track_state),
        lyrics: lyrics_state,
        is_playing,
        position_ms: effective_position_ms,
        sampled_at_epoch_ms: now_ms,
    };
    runtime.publish(state);
    Ok(())
}

/// 用户控制命令：previous / toggle / next。
pub async fn control(runtime: &SyncRuntime, session: &mut CdpSession, action: &str) -> bool {
    let _ = runtime;
    session.control(action).await.unwrap_or(false)
}

/// 用户拖动我们自己的进度条时，将位置回写给网易云原始进度滑轨。
pub async fn seek(runtime: &SyncRuntime, session: &mut CdpSession, position_ms: u64) -> bool {
    let duration_ms = runtime
        .current()
        .track
        .and_then(|track| track.duration_ms)
        .unwrap_or(0);
    session.seek(position_ms, duration_ms).await.unwrap_or(false)
}

fn build_lyrics_state(lyrics: &Lyrics, position_ms: u64) -> LyricsState {
    let offset = LYRIC_OFFSET_MS.load(Ordering::Relaxed);
    let display_pos = position_ms.saturating_add(offset);

    let empty = LyricsState {
        kind: lyrics.kind,
        current_text: None,
        current_translation: None,
        next_text: None,
        visible_lines: Vec::new(),
        offset_ms: offset,
    };
    if lyrics.kind == LyricsKind::None {
        return empty;
    }
    let Some(idx) = super::lyrics::current_index(&lyrics.lines, display_pos) else {
        return empty;
    };

    LyricsState {
        kind: lyrics.kind,
        current_text: Some(lyrics.lines[idx].text.clone()),
        current_translation: lyrics.lines[idx].translation.clone(),
        next_text: lyrics.lines.get(idx + 1).map(|l| l.text.clone()),
        visible_lines: visible_lyric_lines(lyrics, idx),
        offset_ms: offset,
    }
}

fn visible_lyric_lines(lyrics: &Lyrics, current_idx: usize) -> Vec<LyricDisplayLine> {
    const BEFORE: usize = 3;
    const AFTER: usize = 3;
    let start = current_idx.saturating_sub(BEFORE);
    let end = (current_idx + AFTER + 1).min(lyrics.lines.len());
    lyrics.lines[start..end]
        .iter()
        .enumerate()
        .map(|(offset, line)| LyricDisplayLine {
            start_ms: line.start_ms,
            text: line.text.clone(),
            translation: line.translation.clone(),
            active: start + offset == current_idx,
        })
        .collect()
}
