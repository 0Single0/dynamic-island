// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
mod agent;
mod commands;
mod markdown;
mod netease;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
    Manager,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 注意：invoke_handler 只能调用一次，后设会覆盖先设，
    // 所有命令必须合并在同一个 generate_handler! 里。
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| error.to_string())?;
            let agent_state = agent::AgentState::initialize(&app_data_dir).map_err(|error| {
                std::io::Error::other(format!("agent storage init failed: {error}"))
            })?;
            app.manage(agent_state);
            // 注：尝试过 Windows Acrylic（window-vibrancy），但 Win11 的
            // Acrylic 材质自带蓝白系统渐变膜且会混入背后网页颜色，无法去除，
            // 故弃用。质感完全由 CSS 深色半透明底 + 高光描边实现。
            // 系统托盘：显示/隐藏岛体 + 退出
            let show_hide = MenuItem::with_id(app, "toggle", "显示 / 隐藏", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_hide, &quit])?;

            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().expect("no default icon").clone())
                .tooltip("Dynamic Island")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => {
                        if let Some(window) = app.get_webview_window("main") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::DoubleClick { .. } = event {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // 光标轮询线程：岛体之外的区域对鼠标穿透
            commands::window::spawn_click_through_thread(app.handle().clone());

            // 网易云同步共享状态
            netease::setup(app.handle())?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::window::set_always_on_top,
            commands::window::pin_to_top_center,
            commands::window::set_island_rect,
            commands::window::set_window_interactive,
            commands::window::quit_app,
            commands::window::hide_window,
            markdown::read_markdown_file,
            markdown::read_markdown_image_data,
            markdown::markdown_to_pdf,
            agent::agent_list_sessions,
            agent::agent_create_session,
            agent::agent_list_messages,
            agent::agent_send_message,
            agent::agent_cancel_run,
            agent::agent_preview_approval,
            agent::agent_execute_builtin_tool,
            agent::config::agent_list_provider_configs,
            agent::config::agent_list_model_configs,
            agent::config::agent_save_provider_config,
            agent::config::agent_save_model_config,
            agent::plugins::list_builtin_plugins,
            agent::tools::list_builtin_tools,
            netease::netease_start_or_connect,
            netease::netease_restart_with_sync,
            netease::netease_control,
            netease::netease_seek,
            netease::netease_set_lyric_offset,
            netease::netease_disconnect,
            netease::netease_current_state,
            netease::netease_cover_data_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
