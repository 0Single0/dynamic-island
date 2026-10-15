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

/// 将岛体窗口吸附到当前显示器的指定边（top / left / right）。
/// 窗口是固定尺寸的透明矩形，岛体在其中按边对齐摆放：顶停靠时窗口
/// 顶边贴屏幕顶、岛水平居中；左右停靠时窗口贴对应屏幕边、岛垂直居中。
/// `offset` 为沿边方向相对居中位置的逻辑像素偏移，会被钳制在屏幕内。
/// width/height 传窗口应有的逻辑尺寸：吸附前先落尺寸，保证布局假设
/// （岛体在窗口内的偏移）与真实窗口一致，否则岛体会"飞"离屏幕边。
#[tauri::command]
pub fn dock_island(
    window: Window,
    edge: String,
    offset: f64,
    width: f64,
    height: f64,
) -> tauri::Result<()> {
    let (mx, my, mw, mh) = monitor_origin(&window);
    let scale = window.scale_factor().unwrap_or(1.0);
    window.set_size(tauri::LogicalSize::new(width, height))?;
    let w = (width * scale).round() as i32;
    let h = (height * scale).round() as i32;

    // 窗口矩形不许超出显示器（留 0 边距即可，岛体自身在窗口内还有 padding）。
    let (x, y) = match edge.as_str() {
        "left" => {
            let y = my + (mh as i32 - h) / 2 + (offset * scale).round() as i32;
            (mx, y.clamp(my, my + mh as i32 - h))
        }
        "right" => {
            let y = my + (mh as i32 - h) / 2 + (offset * scale).round() as i32;
            (mx + mw as i32 - w, y.clamp(my, my + mh as i32 - h))
        }
        _ => {
            let x = mx + (mw as i32 - w) / 2 + (offset * scale).round() as i32;
            (x.clamp(mx, mx + mw as i32 - w), my)
        }
    };

    window.set_position(PhysicalPosition::new(x, y))
}

/// 设置窗口标签。工具窗口用 `tool-<id>`，设置窗口固定单例。
pub const SETTINGS_WINDOW_LABEL: &str = "settings";

/// 设置窗口已开着、又要跳到某个分类时的事件名（窗口已存在就不重建，只能通知它自己切）。
pub const SETTINGS_SECTION_EVENT: &str = "agent://settings-section";

/// 打开（或聚焦）设置窗口。
///
/// 设置中心是**独立普通窗口**：岛体窗口被钉死成 812×512 的透明无边框矩形
/// （置顶、不可缩放、任务栏里没有），多栏表单挤进去只能靠压缩字号和行高，
/// 而且打开设置必然卸载整个岛体。独立窗口有正常尺寸、进任务栏、会被别的窗口
/// 盖住，岛体也不受影响。
///
/// 必须是 `async`：Windows 上同步命令跑在主线程，而 `WebviewWindowBuilder`
/// 要等主线程消息循环来初始化 WebView2 —— 同步命令会把初始导航堵死在
/// about:blank（白屏窗口）。工具窗口 `open_tool_window` 正因此是 async 的。
#[tauri::command]
pub async fn open_settings_window(app: AppHandle, section: Option<String>) -> Result<(), String> {
    let section = section.filter(|value| !value.trim().is_empty());

    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        if let Some(section) = section {
            use tauri::Emitter as _;
            let _ = app.emit_to(SETTINGS_WINDOW_LABEL, SETTINGS_SECTION_EVENT, section);
        }
        return Ok(());
    }

    let url = match section {
        Some(section) => format!("index.html#/settings?section={section}"),
        None => "index.html#/settings".to_owned(),
    };
    tauri::WebviewWindowBuilder::new(
        &app,
        SETTINGS_WINDOW_LABEL,
        tauri::WebviewUrl::App(url.into()),
    )
    .title("设置中心")
    .inner_size(1040.0, 720.0)
    .min_inner_size(820.0, 560.0)
    // 与工具窗口一致：无系统标题栏，顶栏由页面自绘（拖拽手柄 + 窗口按钮）。
    .decorations(false)
    .center()
    .build()
    .map_err(|error| error.to_string())?;
    Ok(())
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
//
// 前端只上报岛体在视口内的逻辑偏移；屏幕矩形由轮询线程每帧用实时的
// 窗口位置现算——窗口被拖走后位置立刻变化，矩形不会停留在旧屏幕
// 坐标上导致光标明明在岛上却被判定为穿透。

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use windows::Win32::Foundation::{COLORREF, HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetCursorPos, GetWindowLongW, SetLayeredWindowAttributes, SetWindowLongW, GA_ROOT,
    GWL_EXSTYLE, LWA_ALPHA, WS_EX_LAYERED, WS_EX_TRANSPARENT,
};

const POLL_INTERVAL_MS: u64 = 16;
/// 判定外扩边距（物理像素）：补偿轮询间隔内鼠标快速移入岛体的帧差，
/// 保证岛体边缘的点击不被穿透吞掉。
const EDGE_PAD_PX: i32 = 6;

/// 岛体在视口内的逻辑偏移与尺寸（x/y/w/h 各一个槽位）；w 为 0 表示尚未上报。
static ISLAND_VIEW_X: AtomicI32 = AtomicI32::new(0);
static ISLAND_VIEW_Y: AtomicI32 = AtomicI32::new(0);
static ISLAND_VIEW_W: AtomicI32 = AtomicI32::new(0);
static ISLAND_VIEW_H: AtomicI32 = AtomicI32::new(0);
/// 当前是否处于穿透态（避免重复写窗口样式）。
static CLICK_THROUGH: AtomicBool = AtomicBool::new(false);
/// 拖拽中：指针会随窗口移动短暂脱离岛体矩形，此时必须保持可点击，
/// 否则窗口瞬间切到穿透态，拖拽事件被吞掉（表现为"拖不动"）。
static DRAGGING: AtomicBool = AtomicBool::new(false);

/// 前端在拖拽开始/结束时上报，拖拽期间强制不穿透。
#[tauri::command]
pub fn set_island_dragging(active: bool) {
    DRAGGING.store(active, Ordering::Release);
}

/// 前端在动画帧上实时上报岛体的视口内逻辑坐标，屏幕矩形由轮询线程现算。
#[tauri::command]
pub fn set_island_rect(x: f64, y: f64, width: f64, height: f64) {
    ISLAND_VIEW_X.store(x.round() as i32, Ordering::Release);
    ISLAND_VIEW_Y.store(y.round() as i32, Ordering::Release);
    ISLAND_VIEW_W.store(width.round() as i32, Ordering::Release);
    ISLAND_VIEW_H.store(height.round() as i32, Ordering::Release);
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

            let inside = {
                let w = ISLAND_VIEW_W.load(Ordering::Acquire);
                let h = ISLAND_VIEW_H.load(Ordering::Acquire);
                if DRAGGING.load(Ordering::Acquire) {
                    true // 拖拽中：保持可点击，避免事件在窗口跟手移动时被吞
                } else if w == 0 || h == 0 {
                    true // 尚未上报前不穿透，保证岛体可点
                } else {
                    let scale = window.scale_factor().unwrap_or(1.0);
                    let Ok(pos) = window.outer_position() else {
                        continue;
                    };
                    let x = pos.x + (ISLAND_VIEW_X.load(Ordering::Acquire) as f64 * scale).round() as i32;
                    let y = pos.y + (ISLAND_VIEW_Y.load(Ordering::Acquire) as f64 * scale).round() as i32;
                    let pw = (w as f64 * scale).round() as i32;
                    let ph = (h as f64 * scale).round() as i32;
                    cursor.x >= x - EDGE_PAD_PX
                        && cursor.x < x + pw + EDGE_PAD_PX
                        && cursor.y >= y - EDGE_PAD_PX
                        && cursor.y < y + ph + EDGE_PAD_PX
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
