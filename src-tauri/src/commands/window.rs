use tauri::{AppHandle, Manager, PhysicalPosition, Window};

/// 当前显示器的尺寸与位置。
fn monitor_origin(window: &Window) -> (i32, i32, u32, u32) {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());

    if let Some(monitor) = monitor {
        let size = monitor.size();
        let pos = monitor.position();
        (pos.x, pos.y, size.width, size.height)
    } else {
        (0, 0, 1920, 1080)
    }
}

/// 窗口置顶。
#[tauri::command]
pub fn set_always_on_top(window: Window) -> tauri::Result<()> {
    window.set_always_on_top(true)
}

/// 窗口按给定逻辑尺寸定位到屏幕顶部中央（仅启动/需要时调用一次）。
/// 窗口尺寸此后固定不变，所有伸缩动画由前端在透明窗口内渲染。
#[tauri::command]
pub fn pin_to_top_center(
    window: Window,
    margin_top: i32,
    width: f64,
    height: f64,
) -> tauri::Result<()> {
    let (mx, my, mw, _mh) = monitor_origin(&window);
    let scale = window.scale_factor().unwrap_or(1.0);
    let w = (width * scale).round() as i32;
    let x = mx + ((mw as i32 - w) / 2);
    let y = my + (margin_top as f64 * scale) as i32;

    window.set_always_on_top(true)?;
    let _ = window.set_focus();
    window.set_size(tauri::LogicalSize::new(width, height))?;
    window.set_position(PhysicalPosition::new(x, y))
}

/// 退出应用。
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// 隐藏窗口到托盘。
#[tauri::command]
pub fn hide_window(window: Window) -> tauri::Result<()> {
    window.hide()
}

// —— 点击穿透 ——

// 窗口是固定最大尺寸的透明矩形，岛体只占其中一小块；岛体之外必须
// 对鼠标完全穿透，否则屏幕顶部会有一大片“看不见但点不到”的空白。
//
// 做法：后台线程轮询全局光标位置，光标落在岛体（物理像素）范围内时
// 摘掉 WS_EX_TRANSPARENT 恢复接收点击，离开时加回该样式。状态只在
// 变化时才写窗口样式，轮询本身开销可忽略。

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use windows::Win32::Foundation::{COLORREF, HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetCursorPos, GetWindowLongW, SetLayeredWindowAttributes, SetWindowLongW, GA_ROOT,
    GWL_EXSTYLE, LWA_ALPHA, WS_EX_LAYERED, WS_EX_TRANSPARENT,
};

const POLL_INTERVAL_MS: u64 = 16;
/// 判定外扩边距（物理像素）：补偿轮询间隔内鼠标快速移入岛体的帧差，
/// 保证岛体边缘的点击不被穿透吞掉。
const EDGE_PAD_PX: i32 = 6;

/// 岛体屏幕物理坐标，打包进 i64（x/y/w/h 各 16 位）；0 表示尚未上报。
static ISLAND_RECT: AtomicI64 = AtomicI64::new(0);
/// 当前是否处于穿透态（避免重复写窗口样式）。
static CLICK_THROUGH: AtomicBool = AtomicBool::new(false);
/// 设置页等完整窗口页面需要整窗接收鼠标；灵动岛页面则只接收岛体矩形内的鼠标。
static FULL_WINDOW_INTERACTIVE: AtomicBool = AtomicBool::new(false);

fn pack_rect(x: i32, y: i32, w: i32, h: i32) -> i64 {
    ((x as i16 as i64) & 0xFFFF)
        | (((y as i16 as i64) & 0xFFFF) << 16)
        | (((w as i16 as i64) & 0xFFFF) << 32)
        | (((h as i16 as i64) & 0xFFFF) << 48)
}

fn unpack_rect(v: i64) -> (i32, i32, i32, i32) {
    (
        (v & 0xFFFF) as u16 as i16 as i32,
        ((v >> 16) & 0xFFFF) as u16 as i16 as i32,
        ((v >> 32) & 0xFFFF) as u16 as i16 as i32,
        ((v >> 48) & 0xFFFF) as u16 as i16 as i32,
    )
}

/// 前端在动画帧上实时上报岛体的视口逻辑坐标，Rust 侧换算为屏幕物理坐标。
#[tauri::command]
pub fn set_island_rect(window: Window, x: f64, y: f64, width: f64, height: f64) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let Ok(pos) = window.outer_position() else {
        return;
    };
    let px = pos.x + (x * scale).round() as i32;
    let py = pos.y + (y * scale).round() as i32;
    let pw = (width * scale).round() as i32;
    let ph = (height * scale).round() as i32;
    ISLAND_RECT.store(pack_rect(px, py, pw, ph), Ordering::Release);
}

/// 切换窗口的鼠标命中模式。
///
/// `true`：整个透明窗口都接收鼠标，供设置中心等完整页面使用。
/// `false`：只让前端上报的灵动岛矩形接收鼠标，矩形外点击穿透到桌面。
#[tauri::command]
pub fn set_window_interactive(interactive: bool) {
    FULL_WINDOW_INTERACTIVE.store(interactive, Ordering::Release);
}

/// 启动光标轮询线程（仅 setup 时调用一次）。
pub fn spawn_click_through_thread(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_through: Option<bool> = None;

        loop {
            std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));

            let Some(window) = app.get_webview_window("main") else {
                continue;
            };
            if !window.is_visible().unwrap_or(false) {
                continue;
            }

            // tauri 的 hwnd() 可能拿到 WebView2 子窗口，样式必须设在顶层窗口上
            let Ok(hwnd) = window.hwnd() else { continue };
            let root = unsafe { GetAncestor(HWND(hwnd.0), GA_ROOT) };
            let hwnd = if !root.0.is_null() {
                root
            } else {
                HWND(hwnd.0)
            };

            // 兜底：确保窗口是 layered 且 alpha=255，切换 WS_EX_TRANSPARENT 才生效
            let mut ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) };
            if ex_style & WS_EX_LAYERED.0 as i32 == 0 {
                ex_style |= WS_EX_LAYERED.0 as i32;
                unsafe { SetWindowLongW(hwnd, GWL_EXSTYLE, ex_style) };
            }
            let _ = unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA) };

            let mut cursor = POINT::default();
            if unsafe { GetCursorPos(&mut cursor) }.is_err() {
                continue;
            }

            let inside = if FULL_WINDOW_INTERACTIVE.load(Ordering::Acquire) {
                true
            } else {
                match ISLAND_RECT.load(Ordering::Acquire) {
                    0 => true, // 尚未上报前不穿透，保证岛体可点
                    packed => {
                        let (x, y, w, h) = unpack_rect(packed);
                        cursor.x >= x - EDGE_PAD_PX
                            && cursor.x < x + w + EDGE_PAD_PX
                            && cursor.y >= y - EDGE_PAD_PX
                            && cursor.y < y + h + EDGE_PAD_PX
                    }
                }
            };

            let want_through = !inside;
            if last_through != Some(want_through) {
                let style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) };
                let new_style = if want_through {
                    style | WS_EX_TRANSPARENT.0 as i32
                } else {
                    style & !WS_EX_TRANSPARENT.0 as i32
                };
                unsafe { SetWindowLongW(hwnd, GWL_EXSTYLE, new_style) };
                CLICK_THROUGH.store(want_through, Ordering::Release);
                last_through = Some(want_through);
            }
        }
    });
}
