// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
mod agent;
mod commands;
mod computer;
mod gateway;
mod markdown;
mod netease;

// tauri-plugin-dialog 依赖的 rfd 启用了 common-controls-v6，静态导入 comctl32 的
// TaskDialogIndirect；没有 Common-Controls v6 激活上下文时，加载器会以
// STATUS_ENTRYPOINT_NOT_FOUND (0xC0000139) 拒绝启动进程。
// 主程序与 bins 的清单由 tauri-build 的 resource.lib 提供，集成测试由 build.rs 的
// rustc-link-arg-tests 提供；只有 `cargo test --lib` 的单元测试可执行文件两者都拿不到
// （cargo 的 rustc-link-arg-tests 不覆盖 lib 单元测试）。这里用 .drectve 段补一条
// 清单依赖：link.exe 会据此生成同名 .exe.manifest 外部清单，cfg(test) 保证只影响单元测试。
#[cfg(all(test, windows))]
#[used]
#[allow(dead_code)]
#[link_section = ".drectve"]
static TEST_COMMON_CONTROLS_MANIFEST: [u8; 167] = *b"/MANIFESTDEPENDENCY:\"type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' publicKeyToken='6595b64144ccf1df' language='*' processorArchitecture='*'\"\0";

use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
    Manager,
};

/// 注册命令所需状态，返回共享的插件运行时句柄。
///
/// 命令签名是 `State<'_, AgentState>`，因此必须按值注册 `AgentState`；
/// 插件运行时另外按 `Arc<PluginRuntime>` 注册。类型不一致只会在运行时暴露
/// （“state not managed”），所以抽成函数让 `state_wiring` 测试用同一段代码锁定契约。
fn register_agent_state<R: tauri::Runtime>(
    app: &tauri::App<R>,
    agent_state: agent::AgentState,
) -> std::sync::Arc<agent::plugins::PluginRuntime> {
    let plugins = std::sync::Arc::clone(&agent_state.plugins);
    app.manage(agent_state);
    plugins
}

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
            let plugins = register_agent_state(app, agent_state);
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

            // 反代号池网关：进程内嵌 HTTP 服务（外部客户端走 /v1/*，面板走 Tauri command）。
            // 端口被占用时自行回退，不阻塞岛启动。
            gateway::setup(app.handle())?;

            // 内置插件自动注册：业务模块只注册能力，插件清单只描述暴露关系。
            // 这里没有用户启停开关；`@` 只影响单次请求的工具范围。
            let netease = app
                .state::<std::sync::Arc<netease::NeteaseShared>>()
                .inner()
                .clone();
            // 挂载失败是程序缺陷：直接中止启动，不留下半注册状态。
            tauri::async_runtime::block_on(agent::plugins::mount_builtin_plugins(
                &plugins,
                Some(netease),
            ))
            .map_err(|error| {
                std::io::Error::other(format!(
                    "内置插件挂载失败 [{}]: {}",
                    error.plugin_id.unwrap_or_else(|| "unknown".into()),
                    error.message
                ))
            })?;
            // 插件运行时按 Arc 共享给只读目录命令（挂载完成后再注册）。
            app.manage(plugins);

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
            agent::agent_delete_session,
            agent::agent_list_messages,
            agent::agent_list_timeline,
            agent::agent_list_events,
            agent::agent_send_message,
            agent::agent_cancel_run,
            agent::agent_preview_approval,
            agent::agent_execute_builtin_tool,
            agent::config::agent_list_provider_configs,
            agent::config::agent_list_model_configs,
            agent::config::agent_save_provider_config,
            agent::config::agent_save_model_config,
            agent::config::agent_delete_model_config,
            agent::plugins::list_builtin_plugins,
            agent::plugins::list_plugin_tools,
            agent::tools::list_builtin_tools,
            agent::plugins::web_search::agent_get_search_config,
            agent::plugins::web_search::agent_save_search_config,
            netease::netease_start_or_connect,
            netease::netease_restart_with_sync,
            netease::netease_control,
            netease::netease_seek,
            netease::netease_set_lyric_offset,
            netease::netease_disconnect,
            netease::netease_current_state,
            netease::netease_cover_data_url,
            // 反代号池：面板命令（HTTP 面在 gateway::server，不经这里）
            gateway::commands::gateway_status,
            gateway::commands::gateway_set_runtime,
            gateway::commands::gateway_refresh_matrix,
            gateway::commands::gateway_list_channels,
            gateway::commands::gateway_update_channel,
            gateway::commands::gateway_list_accounts,
            gateway::commands::gateway_login_start,
            gateway::commands::gateway_login_poll,
            gateway::commands::gateway_set_account_enabled,
            gateway::commands::gateway_delete_account,
            gateway::commands::gateway_refresh_account,
            gateway::commands::gateway_checkin_account,
            gateway::commands::gateway_fetch_quota,
            gateway::commands::gateway_reset_account_state,
            gateway::commands::gateway_list_tasks,
            gateway::commands::gateway_probe_account,
            gateway::commands::gateway_probe_models,
            gateway::commands::gateway_run_tasks,
            gateway::commands::gateway_task_progress,
            gateway::commands::gateway_claim_task,
            gateway::commands::gateway_add_account,
            gateway::commands::gateway_upsert_account_config,
            gateway::commands::gateway_list_routes,
            gateway::commands::gateway_set_overrides,
            gateway::commands::gateway_clear_affinity,
            gateway::commands::gateway_recent_decisions,
            gateway::commands::gateway_recent_usage,
            gateway::commands::gateway_usage_series,
            gateway::commands::gateway_usage_summary,
            gateway::commands::gateway_paths,
            gateway::commands::gateway_scheduler_snapshot,
            gateway::commands::open_tool_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod state_wiring {
    //! 命令的 `State<T>` 类型与 `manage` 注册类型不匹配时，编译和普通单元测试都不会报错，
    //! 只有运行时第一条 IPC 才会返回 "state not managed"。这里用 mock runtime 把真实命令
    //! 跑一遍，锁住 `register_agent_state` 的注册契约。

    use super::*;
    use tauri::{
        ipc::{CallbackFn, InvokeBody},
        test::{get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY},
        webview::InvokeRequest,
        WebviewWindow, WebviewWindowBuilder,
    };

    fn invoke(webview: &WebviewWindow<MockRuntime>, cmd: &str) -> Result<serde_json::Value, serde_json::Value> {
        get_ipc_response(
            webview,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: "http://tauri.localhost".parse().expect("valid url"),
                body: InvokeBody::default(),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|body| body.deserialize::<serde_json::Value>().expect("json body"))
    }

    #[test]
    fn commands_find_the_managed_agent_state_and_plugin_runtime() {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = mock_builder()
            .invoke_handler(tauri::generate_handler![
                agent::agent_create_session,
                agent::agent_list_sessions,
                agent::plugins::list_builtin_plugins,
            ])
            .build(mock_context(noop_assets()))
            .expect("mock app builds");
        // setup 只在 App::run 时执行，这里直接调用 run() 用的同一段注册逻辑。
        let state = agent::AgentState::initialize(dir.path()).expect("agent state");
        let plugins = register_agent_state(&app, state);
        tauri::async_runtime::block_on(agent::plugins::mount_builtin_plugins(&plugins, None))
            .expect("builtin plugins mount");
        app.manage(plugins);
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("mock webview");

        let created = invoke(&webview, "agent_create_session").expect("创建会话无需重新注册状态");
        assert!(created["id"].as_str().is_some(), "{created}");
        let sessions = invoke(&webview, "agent_list_sessions").expect("会话列表可读");
        assert!(sessions.is_array(), "{sessions}");
        let plugins = invoke(&webview, "list_builtin_plugins").expect("插件目录可读");
        assert_eq!(plugins.as_array().map(Vec::len), Some(6), "{plugins}");
    }
}
