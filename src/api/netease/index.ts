import { invoke } from "@tauri-apps/api/core";
import type { NeteaseControlAction, NeteaseState } from "./types";

/**
 * 网易云同步命令 API：与 src-tauri/src/netease/mod.rs 命令一一对应。
 * 浏览器环境（无 Tauri）下静默降级，便于纯前端调试 UI。
 */

export function startOrConnect() {
  return invoke<void>("netease_start_or_connect").catch(() => {});
}

export function restartWithSync() {
  return invoke<void>("netease_restart_with_sync").catch(() => {});
}

export function control(action: NeteaseControlAction) {
  return invoke<boolean>("netease_control", { action }).catch(() => false);
}

/** 将网易云原始播放器跳转到指定毫秒位置。 */
export function seek(positionMs: number) {
  return invoke<boolean>("netease_seek", { positionMs }).catch(() => false);
}

export function setLyricOffset(offsetMs: number) {
  return invoke<void>("netease_set_lyric_offset", { offsetMs }).catch(() => {});
}

export function disconnect() {
  return invoke<void>("netease_disconnect").catch(() => {});
}

/** orpheus:// 封面地址 → base64 data URL（仅允许网易图片 CDN 域名）。 */
export function coverDataUrl(rawUrl: string) {
  return invoke<string>("netease_cover_data_url", { rawUrl });
}

export async function currentState(): Promise<NeteaseState | null> {
  try {
    return await invoke<NeteaseState>("netease_current_state");
  } catch {
    return null;
  }
}
