import { invoke } from "@tauri-apps/api/core";

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
 * 上报岛体在视口内的逻辑坐标（物理像素由 Rust 侧换算）。
 * Rust 光标轮询线程据此决定窗口哪些区域对鼠标穿透。
 */
export function setIslandRect(x: number, y: number, width: number, height: number) {
  return invoke<void>("set_island_rect", { x, y, width, height }).catch(() => {});
}

/** 切换完整页面 / 灵动岛局部点击模式。 */
export function setWindowInteractive(interactive: boolean) {
  return invoke<void>("set_window_interactive", { interactive }).catch(() => {});
}

/** 退出应用 */
export function quitApp() {
  return invoke<void>("quit_app").catch(() => {});
}

/** 隐藏窗口到托盘 */
export function hideWindow() {
  return invoke<void>("hide_window").catch(() => {});
}
