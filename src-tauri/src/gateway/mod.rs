//! 岛内「反代号池」网关：进程内嵌 HTTP 网关 + 多渠道账号池调度。
//!
//! 形态与 `netease` 模块一致：`setup(app)` 建共享状态、注册进 Tauri，后台任务跑在
//! Tauri 已有的 tokio 运行时里。两个边界面共用同一份状态：
//! · 外部客户端（任何按 OpenAI / Anthropic / Responses 协议说话的进程）走 HTTP `/v1/*`；
//! · 岛内面板走 Tauri command（同进程直连，免去面板鉴权与 CORS）。
//!
//! **契约是协议，不是客户端**：网关不识别调用方身份，不为某个客户端写分支。
//!
//! 依赖方向：`server → core → channels → transport`，`channels` 不得引用 `core`。

pub mod channels;
pub mod commands;
pub mod config;
pub mod core;
pub mod errors;
pub mod ops;
pub mod protocol;
pub mod security;
pub mod server;
pub mod state;
pub mod storage;
pub mod timeutil;
pub mod transport;

use std::sync::Arc;

use tauri::{AppHandle, Manager};

pub use state::{GatewayState, SharedGateway};

/// 建立网关状态；配置里 `enabled` 为真时在后台起 HTTP 服务。
///
/// 端口占用不阻塞岛启动：绑定失败只写进服务状态，由面板显示。
pub fn setup(app: &AppHandle) -> Result<SharedGateway, Box<dyn std::error::Error>> {
    let dir = app.path().app_data_dir()?.join("gateway");
    std::fs::create_dir_all(&dir)?;
    let config = config::Config::load_or_create(&dir)?;
    let state = GatewayState::initialize(config, dir)?;
    app.manage(Arc::clone(&state));
    if state.config().enabled {
        tauri::async_runtime::spawn(server::run(Arc::clone(&state)));
    }
    // 常驻定时任务（额度刷新等）不依赖服务是否开启，也不依赖面板是否打开。
    ops::spawn(Arc::clone(&state));
    Ok(state)
}
