/** 网易云同步状态类型：与 src-tauri/src/netease/sync.rs 的 NeteaseState 对应。 */

export type NeteaseConnection =
  | "idle"
  | "needs-restart"
  | "starting"
  | "waiting-for-cdp"
  | "synchronizing"
  | "degraded"
  | "error";

export interface NeteaseTrack {
  songId?: string;
  title?: string;
  artist?: string;
  album?: string;
  coverUrl?: string;
  durationMs?: number;
}

export type LyricsKind = "original" | "translation" | "none";

export interface NeteaseLyrics {
  kind: LyricsKind;
  currentText?: string;
  currentTranslation?: string;
  nextText?: string;
  visibleLines: NeteaseLyricDisplayLine[];
  offsetMs: number;
}

/** 展开歌词区的一行：当前行高亮，其余行按距离淡出。 */
export interface NeteaseLyricDisplayLine {
  /** Lyric start time; keeps the same rendered line stable across updates. */
  startMs: number;
  text: string;
  translation?: string;
  active: boolean;
}

export interface NeteaseState {
  connection: NeteaseConnection;
  message?: string;
  track?: NeteaseTrack;
  lyrics?: NeteaseLyrics;
  isPlaying: boolean;
  positionMs: number;
  sampledAtEpochMs: number;
}

export type NeteaseControlAction = "previous" | "toggle" | "next";
