//! 网易云同步模块：CDP 连接、本地列表匹配、歌词、轮询推送。

pub mod cdp;
pub mod launcher;
pub mod lyrics;
pub mod playlist;
pub mod sync;

use std::sync::Arc;

use tauri::{AppHandle, Manager};
use tokio::sync::Mutex as AsyncMutex;

use launcher::StartOutcome;
use sync::{ConnectionState, NeteaseState, SyncRuntime};

/// 全局共享：运行时状态 + 当前 CDP 会话。Arc 包装便于丢进后台任务。
pub struct NeteaseShared {
    pub app: AppHandle,
    pub runtime: SyncRuntime,
    pub session: AsyncMutex<Option<cdp::CdpSession>>,
}

pub type SharedNetease = Arc<NeteaseShared>;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn base_state() -> NeteaseState {
    NeteaseState {
        connection: ConnectionState::Idle,
        message: None,
        track: None,
        lyrics: None,
        is_playing: false,
        position_ms: 0,
        sampled_at_epoch_ms: now_ms(),
    }
}

fn outcome_state(outcome: &StartOutcome) -> NeteaseState {
    let (connection, message) = match outcome {
        StartOutcome::Ready => (ConnectionState::WaitingForCdp, None),
        StartOutcome::NeedsRestart => (
            ConnectionState::NeedsRestart,
            Some("当前网易云不是由本工具启动，无法取得精确歌词进度。请保存内容后确认重启。".into()),
        ),
        StartOutcome::NotFound => (
            ConnectionState::Error,
            Some("未找到 cloudmusic.exe，请在设置中指定安装路径".into()),
        ),
        StartOutcome::Timeout => (
            ConnectionState::Error,
            Some("网易云启动后 15 秒内未开放调试页面".into()),
        ),
    };
    NeteaseState {
        connection,
        message,
        ..base_state()
    }
}

/// 连接 CDP 页面并进入轮询循环（指数退避重连，网易云退出则回到 Idle）。
async fn connect_and_poll(shared: SharedNetease) {
    let target = loop {
        match cdp::find_target(launcher::DEFAULT_PORT).await {
            Ok(Some(t)) => break t,
            Ok(None) | Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    };

    let session = match cdp::CdpSession::connect(&target.ws_url).await {
        Ok(s) => s,
        Err(e) => {
            shared.runtime.publish(NeteaseState {
                connection: ConnectionState::Error,
                message: Some(format!("连接播放器页面失败：{e}")),
                ..base_state()
            });
            return;
        }
    };

    *shared.session.lock().await = Some(session);
    shared.runtime.publish(NeteaseState {
        connection: ConnectionState::Synchronizing,
        ..base_state()
    });

    let mut backoff_ms = 500u64;
    loop {
        let result = {
            let mut guard = shared.session.lock().await;
            match guard.as_mut() {
                None => return, // 用户主动断开
                Some(session) => sync::poll_once(&shared.runtime, session).await,
            }
        };

        match result {
            Ok(()) => backoff_ms = 500,
            Err(_) => {
                shared.runtime.publish(NeteaseState {
                    connection: ConnectionState::Error,
                    message: Some("连接已断开，正在重连…".into()),
                    ..base_state()
                });
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                backoff_ms = (backoff_ms * 2).min(8000);

                match cdp::find_target(launcher::DEFAULT_PORT).await {
                    Ok(Some(t)) => {
                        if let Ok(s) = cdp::CdpSession::connect(&t.ws_url).await {
                            *shared.session.lock().await = Some(s);
                        }
                    }
                    Ok(None) | Err(_) => {
                        // 网易云已退出
                        *shared.session.lock().await = None;
                        shared.runtime.publish(NeteaseState {
                            connection: ConnectionState::Idle,
                            message: Some("网易云已退出".into()),
                            ..base_state()
                        });
                        return;
                    }
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(sync::POLL_INTERVAL_MS)).await;
    }
}

/// 建立连接的统一入口：Ready 则进入轮询循环（后台任务），否则推送错误状态。
async fn establish(shared: SharedNetease, outcome: StartOutcome) {
    match outcome {
        StartOutcome::Ready => {
            shared.runtime.publish(NeteaseState {
                connection: ConnectionState::WaitingForCdp,
                ..base_state()
            });
            connect_and_poll(shared).await;
        }
        other => shared.runtime.publish(outcome_state(&other)),
    }
}

// —— Tauri 命令 ——

/// Agent startup returns an outcome; polling must not hold the tool loop open.
pub async fn start_for_agent(shared: SharedNetease) -> StartOutcome {
    let outcome = launcher::start_or_connect().await;
    if matches!(outcome, StartOutcome::Ready) {
        let needs_connection = shared.session.lock().await.is_none();
        if needs_connection {
            tauri::async_runtime::spawn(establish(shared, StartOutcome::Ready));
        }
    } else {
        shared.runtime.publish(outcome_state(&outcome));
    }
    outcome
}

#[tauri::command]
pub async fn netease_start_or_connect(
    shared: tauri::State<'_, SharedNetease>,
) -> Result<(), String> {
    let shared = shared.inner().clone();
    let outcome = launcher::start_or_connect().await;
    eprintln!("[netease] start_or_connect outcome: {outcome:?}");
    establish(shared, outcome).await;
    Ok(())
}

#[tauri::command]
pub async fn netease_restart_with_sync(
    shared: tauri::State<'_, SharedNetease>,
) -> Result<(), String> {
    let shared = shared.inner().clone();
    let outcome = launcher::restart_with_sync().await;
    eprintln!("[netease] restart_with_sync outcome: {outcome:?}");
    establish(shared, outcome).await;
    Ok(())
}

#[tauri::command]
pub async fn netease_control(
    shared: tauri::State<'_, SharedNetease>,
    action: String,
) -> Result<bool, String> {
    let mut guard = shared.session.lock().await;
    match guard.as_mut() {
        Some(session) => Ok(sync::control(&shared.runtime, session, &action).await),
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn netease_seek(
    shared: tauri::State<'_, SharedNetease>,
    position_ms: u64,
) -> Result<bool, String> {
    let mut guard = shared.session.lock().await;
    match guard.as_mut() {
        Some(session) => Ok(sync::seek(&shared.runtime, session, position_ms).await),
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn netease_set_lyric_offset(
    shared: tauri::State<'_, SharedNetease>,
    offset_ms: u64,
) -> Result<(), String> {
    shared.runtime.set_lyric_offset(offset_ms);
    Ok(())
}

#[tauri::command]
pub async fn netease_disconnect(shared: tauri::State<'_, SharedNetease>) -> Result<(), String> {
    *shared.session.lock().await = None;
    shared.runtime.publish(NeteaseState {
        connection: ConnectionState::Idle,
        ..base_state()
    });
    Ok(())
}

#[tauri::command]
pub async fn netease_current_state(
    shared: tauri::State<'_, SharedNetease>,
) -> Result<NeteaseState, String> {
    Ok(shared.runtime.current())
}

/// 封面图代理：orpheus://cache/?<https-url> 在岛窗口内无法加载，
/// 这里把其中的 https 地址在本机下载后转成 base64 数据 URL 返回。
/// 只放行图片 CDN 域名 + 图片后缀，返回体直接进 <img src>，不落盘。
#[tauri::command]
pub async fn netease_cover_data_url(raw_url: String) -> Result<String, String> {
    const ALLOWED_HOST_SUFFIXES: [&str; 3] = ["music.126.net", "126.net", "netease.im"];
    // 解出 orpheus://cache/?<real-url> 的真实地址；纯 http(s) 也接受
    let real = raw_url
        .strip_prefix("orpheus://cache/?")
        .unwrap_or(&raw_url)
        .to_string();
    if !real.starts_with("https://") && !real.starts_with("http://") {
        return Err("unsupported cover url scheme".into());
    }
    let host_ok = reqwest::Url::parse(&real)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .map(|h| ALLOWED_HOST_SUFFIXES.iter().any(|s| h.ends_with(s)))
        .unwrap_or(false);
    if !host_ok {
        return Err("cover host not allowed".into());
    }
    let client = cdp::http_client();
    let bytes = client
        .get(&real)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("cover too large".into());
    }
    let mime = match bytes.as_ref().get(..3) {
        Some([0xFF, 0xD8, 0xFF]) => "image/jpeg",
        Some([0x89, b'P', b'N']) => "image/png",
        Some([b'G', b'I', b'F']) => "image/gif",
        Some([b'R', b'I', b'F']) => "image/webp",
        _ => "image/jpeg",
    };
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{mime};base64,{b64}"))
}

/// 播放控制与进度跳转的逐次确认；由插件能力处理器调用，CDP 细节不外泄。
pub async fn confirm_action(
    shared: &SharedNetease,
    description: &str,
    ctx: &crate::agent::plugins::registry::HandlerContext,
) -> Result<(), crate::agent::tools::ToolExecutionResult> {
    crate::agent::native_dialog::confirm(
        &shared.app,
        format!("{description}\n\n允许本次操作吗？"),
        ctx.cancelled.as_ref(),
    )
    .await
    .map_err(|error| {
        if error.error_code.as_deref() == Some("USER_DENIED") {
            crate::agent::tools::ToolExecutionResult::failure(
                "USER_DENIED",
                "用户未确认网易云操作，未执行操作。",
                false,
            )
        } else {
            error
        }
    })
}

/// 注册命令。
pub fn setup(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("[netease] setup registered");
    let shared: SharedNetease = Arc::new(NeteaseShared {
        app: app.clone(),
        runtime: SyncRuntime::new(app.clone()),
        session: AsyncMutex::new(None),
    });
    app.manage(shared);
    Ok(())
}
