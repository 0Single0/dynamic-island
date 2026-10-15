import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** 当前窗口标签（`main` / `tool-<id>` / `settings`）。浏览器预览下返回空串。 */
export function currentWindowLabel(): string {
  try {
    return getCurrentWindow().label;
  } catch {
    return "";
  }
}

/**
 * 窗口控制 API：与 src-tauri/src/commands/window.rs 一一对应。
 * 窗口尺寸固定不参与动画，只负责初始定位与置顶。
 */

/** 窗口置顶并按给定尺寸定位到屏幕顶部中央（仅启动时调用一次） */
export function positionWindow(marginTop: number, width: number, height: number) {
  return invoke<void>("pin_to_top_center", { marginTop, width, height }).catch(
    () => {},
  );
}

/** 窗口置顶 */
export function setAlwaysOnTop() {
  return invoke<void>("set_always_on_top").catch(() => {});
}

/**
 * 上报岛体在视口内的逻辑坐标。
 * Rust 光标轮询线程结合窗口实时位置据此决定窗口哪些区域对鼠标穿透。
 */
export function setIslandRect(x: number, y: number, width: number, height: number) {
  return invoke<void>("set_island_rect", { x, y, width, height }).catch(() => {});
}

/** 退出应用 */
export function quitApp() {
  return invoke<void>("quit_app").catch(() => {});
}

/**
 * 打开（或聚焦）工具窗口。传工具 id，窗口标签固定为 `tool-<id>`，
 * 所以重复点击只会聚焦已开的窗口，不会开一堆。
 */
export function openToolWindow(
  toolId: string,
  title: string,
  width: number,
  height: number,
) {
  return invoke<void>("open_tool_window", { toolId, title, width, height });
}

/** 设置窗口的标签（与 src-tauri/src/commands/window.rs 的常量一致）。 */
export const SETTINGS_WINDOW_LABEL = "settings";

/** 设置窗口已开着时，让它切分类的事件名。 */
export const SETTINGS_SECTION_EVENT = "agent://settings-section";

/**
 * 打开（或聚焦）设置窗口。窗口是单例：重复调用只聚焦，不会开一堆。
 * 已经开着时若传了 `section`，通知它切到该分类而非重建窗口。
 */
export function openSettingsWindow(section?: string) {
  return invoke<void>("open_settings_window", { section: section ?? null });
}

/** 隐藏窗口到托盘 */
export function hideWindow() {
  return invoke<void>("hide_window").catch(() => {});
}
