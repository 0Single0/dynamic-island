//! 面向岛内面板的 Tauri 命令。
//!
//! 与 HTTP 面共用同一份 `SharedGateway`：面板免鉴权、免 CORS，也不依赖 HTTP 服务是否在跑。
//! 命令只做参数校验与转发，业务都在 `core` / `state` 里。

use serde_json::{json, Value};
use tauri::State;

use crate::gateway::{
    channels::LoginPoll,
    config::{AccountConfig, RouteOverride},
    core::refresh,
    errors::GatewayError,
    state::SharedGateway,
    storage::AccountRecord,
    timeutil::now_ms,
};

fn message(error: GatewayError) -> String {
    error.message
}

#[tauri::command]
pub async fn gateway_status(state: State<'_, SharedGateway>) -> Result<Value, String> {
    let config = state.config();
    let status = state.server_status();
    let (routes, live, builtin) = match state.matrix_snapshot() {
        Some(matrix) => {
            let (live, builtin) = matrix.sources();
            (matrix.route_count(), live, builtin)
        }
        None => (0, 0, 0),
    };
    Ok(json!({
        "running": status.running,
        "addr": status.addr,
        "error": status.error,
        "enabled": config.enabled,
        "port": config.port,
        "auth": if config.api_keys.is_empty() { "open" } else { "api_key" },
        // 只回短哈希：面板能识别"配了几把、是不是同一把"，但拿不到明文。
        "api_keys": config.api_keys
            .iter()
            .map(|key| crate::gateway::security::short_hash(key))
            .collect::<Vec<_>>(),
        "affinity": state.affinity.stats(),
        "matrix": {
            "routes": routes,
            "live": live,
            "builtin": builtin,
            "ttl_seconds": config.catalog_ttl_seconds,
        },
        "concurrency": {
            "global": config.concurrency.global,
            "per_account": config.concurrency.per_account,
            "available": state.global_available(),
        },
        "stats": {
            "requests": state.stats.requests.load(std::sync::atomic::Ordering::Relaxed),
            "failures": state.stats.failures.load(std::sync::atomic::Ordering::Relaxed),
            "retries": state.stats.retries.load(std::sync::atomic::Ordering::Relaxed),
        },
        "uptime_ms": state.uptime_ms(),
        "warnings": state.warnings(),
    }))
}

/// 开关 HTTP 服务（重启岛后生效）与写入 API Key。
#[tauri::command]
pub async fn gateway_set_runtime(
    state: State<'_, SharedGateway>,
    enabled: Option<bool>,
    api_keys: Option<Vec<String>>,
) -> Result<Value, String> {
    let config = state
        .edit_config(|config| {
            if let Some(enabled) = enabled {
                config.enabled = enabled;
            }
            if let Some(api_keys) = api_keys {
                config.api_keys = api_keys;
            }
        })
        .map_err(message)?;
    Ok(json!({ "enabled": config.enabled, "api_keys": config.api_keys.len() }))
}

/// 立即重建模型矩阵（面板的"刷新目录"）。
#[tauri::command]
pub async fn gateway_refresh_matrix(state: State<'_, SharedGateway>) -> Result<Value, String> {
    let matrix = state.ensure_matrix(true).await;
    let (live, builtin) = matrix.sources();
    Ok(json!({ "routes": matrix.route_count(), "live": live, "builtin": builtin }))
}

#[tauri::command]
pub async fn gateway_list_channels(state: State<'_, SharedGateway>) -> Result<Value, String> {
    let config = state.config();
    let now = now_ms();
    let channels: Vec<Value> = state
        .channels()
        .iter()
        .map(|channel| {
            let accounts = state.pool.by_channel(channel.id());
            let entry = config.channel(channel.id());
            json!({
                "id": channel.id(),
                "adapter": channel.adapter_kind(),
                "display_name": entry.map(|entry| entry.display_name.clone()).unwrap_or_default(),
                "enabled": entry.map(|entry| entry.enabled).unwrap_or(true),
                "priority": entry.map(|entry| entry.priority).unwrap_or(100),
                "weight": entry.map(|entry| entry.weight).unwrap_or(1),
                "options": entry.map(|entry| entry.options.clone()).unwrap_or(Value::Null),
                "accounts": accounts.len(),
                "ready": accounts.iter().filter(|account| account.is_ready("", now)).count(),
            })
        })
        .collect();
    Ok(Value::Array(channels))
}

/// 改渠道的启停 / 优先级 / 权重（出站身份这类 options 走配置文件）。
#[tauri::command]
pub async fn gateway_update_channel(
    state: State<'_, SharedGateway>,
    channel_id: String,
    enabled: Option<bool>,
    priority: Option<u32>,
    weight: Option<u32>,
) -> Result<Value, String> {
    let config = state
        .edit_config(|config| {
            if let Some(channel) = config
                .channels
                .iter_mut()
                .find(|channel| channel.id == channel_id)
            {
                if let Some(enabled) = enabled {
                    channel.enabled = enabled;
                }
                if let Some(priority) = priority {
                    channel.priority = priority;
                }
                if let Some(weight) = weight {
                    channel.weight = weight;
                }
            }
        })
        .map_err(message)?;
    let _ = config;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn gateway_list_accounts(state: State<'_, SharedGateway>) -> Result<Value, String> {
    let now = now_ms();
    let views: Vec<Value> = state
        .pool
        .views()
        .into_iter()
        .map(|view| serde_json::to_value(view).unwrap_or(Value::Null))
        .collect();
    let _ = now;
    Ok(Value::Array(views))
}

/// 开始一次登录：返回 state（面板轮询用）与要打开的授权链接。
#[tauri::command]
pub async fn gateway_login_start(
    state: State<'_, SharedGateway>,
    channel_id: String,
) -> Result<Value, String> {
    let adapter = state.channel(&channel_id).ok_or_else(|| format!("渠道 {channel_id} 不存在"))?;
    let ticket = adapter
        .login_start(&state.client, "CLI")
        .await
        .map_err(message)?;
    Ok(json!({ "state": ticket.state, "auth_url": ticket.auth_url }))
}

/// 轮询登录结果；成功后落库并进池。
#[tauri::command]
pub async fn gateway_login_poll(
    state: State<'_, SharedGateway>,
    channel_id: String,
    login_state: String,
) -> Result<Value, String> {
    let adapter = state.channel(&channel_id).ok_or_else(|| format!("渠道 {channel_id} 不存在"))?;
    match adapter
        .login_poll(&state.client, &login_state)
        .await
        .map_err(message)?
    {
        LoginPoll::Pending => Ok(json!({ "status": "pending" })),
        LoginPoll::Failed(reason) => Ok(json!({ "status": "failed", "message": reason })),
        LoginPoll::Ready(credential) => {
            let uid = credential
                .uid
                .clone()
                .filter(|uid| !uid.is_empty())
                .ok_or_else(|| "上游没有返回账号标识（JWT sub）".to_string())?;
            let nickname = credential
                .nickname
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| uid.chars().take(8).collect());
            state
                .storage
                .upsert_account(&AccountRecord {
                    channel_id: channel_id.clone(),
                    uid: uid.clone(),
                    nickname: nickname.clone(),
                    enabled: true,
                    priority_override: None,
                    weight_override: None,
                    max_inflight: None,
                    proxy_slot: None,
                    added_at: now_ms(),
                    source: "oauth".into(),
                    credential: Some((*credential).clone()),
                })
                .map_err(message)?;
            let count = state.reload_accounts().map_err(message)?;
            // 入库后立刻签到一次（国内版积分）并抓一次额度，失败都不影响可用。
            if let Some(account) = state.account(&channel_id, &uid) {
                let snapshot = account.snapshot();
                let _ = adapter.sign_in(&state.client, &snapshot).await;
                if let Ok(quota) = adapter.fetch_quota(&state.client, &snapshot).await {
                    account.set_quota(quota);
                }
            }
            Ok(json!({
                "status": "ok",
                "account": { "channel_id": channel_id, "uid": uid, "nickname": nickname },
                "accounts": count,
            }))
        }
    }
}

#[tauri::command]
pub async fn gateway_set_account_enabled(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
    enabled: bool,
) -> Result<Value, String> {
    state
        .storage
        .set_account_enabled(&channel_id, &uid, enabled)
        .map_err(message)?;
    if !enabled {
        // 停用账号时同步清掉它的亲和绑定，避免请求一直亲和到一个废账号。
        state.affinity.clear_account(&channel_id, &uid);
    }
    state.reload_accounts().map_err(message)?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn gateway_delete_account(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
) -> Result<Value, String> {
    let removed = state
        .storage
        .delete_account(&channel_id, &uid)
        .map_err(message)?;
    state.affinity.clear_account(&channel_id, &uid);
    state.reload_accounts().map_err(message)?;
    Ok(json!({ "removed": removed }))
}

/// 手动刷新凭据（串行锁在 core 里）。
#[tauri::command]
pub async fn gateway_refresh_account(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
) -> Result<Value, String> {
    let account = state
        .account(&channel_id, &uid)
        .ok_or_else(|| format!("账号 {channel_id}/{uid} 不存在"))?;
    let credential = refresh::refresh(&state, &account).await.map_err(message)?;
    Ok(json!({
        "expires_at_ms": credential.expires_at_ms,
        "has_refresh_token": credential.refresh_token.is_some(),
    }))
}

/// 刷新账号额度（渠道无关：workbuddy 取积分，将来别的渠道取各自口径）。
#[tauri::command]
pub async fn gateway_fetch_quota(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
) -> Result<Value, String> {
    let account = state
        .account(&channel_id, &uid)
        .ok_or_else(|| format!("账号 {channel_id}/{uid} 不存在"))?;
    let adapter = state.channel(&channel_id).ok_or_else(|| "渠道不存在".to_string())?;
    let snapshot = adapter
        .fetch_quota(&state.client, &account.snapshot())
        .await
        .map_err(message)?;
    account.set_quota(snapshot.clone());
    serde_json::to_value(snapshot).map_err(|error| error.to_string())
}

/// 重置账号的运行态：清冷却、熔断、失败计数、模型级限流。
///
/// 用途：手动排障（比如确认上游已经恢复，不想等冷却到期）。
/// 不动凭据、不动额度、不动启停状态 —— 它只影响"惩罚"。
#[tauri::command]
pub async fn gateway_reset_account_state(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
) -> Result<Value, String> {
    let account = state
        .account(&channel_id, &uid)
        .ok_or_else(|| format!("账号 {channel_id}/{uid} 不存在"))?;
    account.reset_runtime();
    Ok(json!({ "ok": true, "ready": account.is_ready("", now_ms()) }))
}

/// 手动签到。
#[tauri::command]
pub async fn gateway_checkin_account(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
) -> Result<Value, String> {
    let account = state
        .account(&channel_id, &uid)
        .ok_or_else(|| format!("账号 {channel_id}/{uid} 不存在"))?;
    let adapter = state.channel(&channel_id).ok_or_else(|| "渠道不存在".to_string())?;
    let ok = adapter
        .sign_in(&state.client, &account.snapshot())
        .await
        .map_err(message)?;
    Ok(json!({ "ok": ok }))
}

#[tauri::command]
pub async fn gateway_add_account(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
    nickname: Option<String>,
) -> Result<Value, String> {
    if channel_id.trim().is_empty() || uid.trim().is_empty() {
        return Err("渠道与账号标识都不能为空".into());
    }
    state
        .storage
        .upsert_account(&AccountRecord {
            channel_id: channel_id.clone(),
            uid: uid.clone(),
            nickname: nickname.unwrap_or_else(|| uid.clone()),
            enabled: true,
            priority_override: None,
            weight_override: None,
            max_inflight: None,
            proxy_slot: None,
            added_at: now_ms(),
            source: "manual".into(),
            credential: None,
        })
        .map_err(message)?;
    state.reload_accounts().map_err(message)?;
    Ok(json!({ "ok": true }))
}

/// 模型矩阵：对外模型 → 候选渠道。
#[tauri::command]
pub async fn gateway_list_routes(state: State<'_, SharedGateway>) -> Result<Value, String> {
    let matrix = state.ensure_matrix(false).await;
    let routes: Vec<Value> = matrix
        .public_models()
        .into_iter()
        .map(|model| {
            let entries: Vec<Value> = matrix
                .routes(&model)
                .into_iter()
                .map(|route| {
                    json!({
                        "channel_id": route.channel_id,
                        "upstream_model": route.upstream_model,
                        "priority": route.priority,
                        "weight": route.weight,
                        "free": route.is_free(),
                        "credits": route.credits,
                        "caps_known": route.caps_known,
                        "source": route.source,
                        "context": route.limits.context,
                        "max_output": route.limits.max_output,
                        "context_windows": route.context_windows,
                        "capabilities": {
                            "vision": route.capabilities.vision,
                            "tool_calls": route.capabilities.tool_calls,
                            "reasoning": route.capabilities.reasoning,
                        },
                        "reasoning_efforts": route.reasoning_efforts,
                        "reasoning_fixed_effort": route.reasoning_fixed_effort,
                        "reasoning_default_effort": route.reasoning_default_effort,
                        "reasoning_can_disable": route.reasoning_can_disable,
                        "always_reasoning": route.always_reasoning,
                        "temperature": route.temperature,
                        "top_p": route.top_p,
                    })
                })
                .collect();
            json!({ "model": model, "routes": entries })
        })
        .collect();
    let (live, builtin) = matrix.sources();
    Ok(json!({ "models": routes, "live": live, "builtin": builtin }))
}

/// 写入人工覆盖（别名 + 路由覆盖）。
#[tauri::command]
pub async fn gateway_set_overrides(
    state: State<'_, SharedGateway>,
    aliases: Option<std::collections::HashMap<String, String>>,
    routes: Option<Vec<RouteOverride>>,
) -> Result<Value, String> {
    state
        .edit_config(|config| {
            if let Some(aliases) = aliases {
                config.aliases = aliases;
            }
            if let Some(routes) = routes {
                config.routes = routes;
            }
        })
        .map_err(message)?;
    let matrix = state.ensure_matrix(true).await;
    Ok(json!({ "routes": matrix.route_count() }))
}

#[tauri::command]
pub async fn gateway_clear_affinity(state: State<'_, SharedGateway>) -> Result<Value, String> {
    let cleared = state.affinity.clear_all();
    let _ = state.storage.prune_affinity();
    Ok(json!({ "cleared": cleared }))
}

#[tauri::command]
pub async fn gateway_recent_decisions(state: State<'_, SharedGateway>) -> Result<Value, String> {
    Ok(Value::Array(state.decisions()))
}

/// 请求明细（日志表格）：一条请求一行，含用量、耗时、首字、速度与错误原文。
#[tauri::command]
pub async fn gateway_recent_usage(
    state: State<'_, SharedGateway>,
    limit: Option<u32>,
) -> Result<Value, String> {
    state
        .storage
        .recent_usage(limit.unwrap_or(200))
        .map(Value::Array)
        .map_err(message)
}

/// 时间序列（折线图）：按小时分桶，可按模型筛选。
#[tauri::command]
pub async fn gateway_usage_series(
    state: State<'_, SharedGateway>,
    hours: Option<u64>,
    model: Option<String>,
    bucket_hours: Option<u64>,
) -> Result<Value, String> {
    let hours = hours.unwrap_or(24);
    let bucket = bucket_hours.unwrap_or(if hours > 72 { 24 } else { 1 });
    state
        .storage
        .usage_series(hours, model.as_deref(), bucket)
        .map(Value::Array)
        .map_err(message)
}

#[tauri::command]
pub async fn gateway_usage_summary(
    state: State<'_, SharedGateway>,
    hours: Option<u64>,
) -> Result<Value, String> {
    state
        .storage
        .usage_summary(hours.unwrap_or(24))
        .map_err(message)
}

/// 增加账号的便捷入口（等价于往配置里塞一条再重建）。
#[tauri::command]
pub async fn gateway_upsert_account_config(
    state: State<'_, SharedGateway>,
    account: AccountConfig,
) -> Result<Value, String> {
    state
        .edit_config(|config| {
            config
                .accounts
                .retain(|existing| existing.uid != account.uid || existing.channel_id != account.channel_id);
            config.accounts.push(account.clone());
        })
        .map_err(message)?;
    Ok(json!({ "ok": true }))
}

/// 数据目录与库位置（面板"关于"与排障用）。
#[tauri::command]
pub async fn gateway_paths(state: State<'_, SharedGateway>) -> Result<Value, String> {
    Ok(json!({
        "data_dir": state.dir.to_string_lossy(),
        "database": state.storage.path().to_string_lossy(),
    }))
}

/// 账号在飞与冷却明细（面板的"调度"视图）。
#[tauri::command]
pub async fn gateway_scheduler_snapshot(
    state: State<'_, SharedGateway>,
) -> Result<Value, String> {
    let now = now_ms();
    let accounts: Vec<Value> = state
        .pool
        .accounts()
        .into_iter()
        .map(|account| {
            json!({
                "key": account.key.to_string(),
                "enabled": account.enabled,
                "ready": account.is_ready("", now),
                "inflight": account.inflight(),
                "cooldown_ms": account.cooldown_until_ms().saturating_sub(now),
                "priority": account.priority,
                "weight": account.weight,
            })
        })
        .collect();
    let bindings: Vec<Value> = state
        .affinity
        .snapshot()
        .into_iter()
        .take(20)
        .map(|(key, binding)| {
            json!({
                "key": key,
                "channel_id": binding.channel_id,
                "uid": binding.uid,
                "upstream_model": binding.upstream_model,
                "expires_at_ms": binding.expires_at_ms,
                "hits": binding.hits,
            })
        })
        .collect();
    Ok(json!({
        "accounts": accounts,
        "affinity": state.affinity.stats(),
        "bindings": bindings,
    }))
}

/// 打开（或聚焦）工具窗口。
///
/// 工具窗口是**普通窗口**：可缩放、有标题栏，与岛体互不影响 ——
/// 面板塞在 780×480 的岛体里只适合看，不适合用。
#[tauri::command]
pub async fn open_tool_window(
    app: tauri::AppHandle,
    tool_id: String,
    title: String,
    width: f64,
    height: f64,
) -> Result<(), String> {
    use tauri::Manager as _;
    let label = format!("tool-{tool_id}");
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App(format!("index.html#/tool/{tool_id}").into()),
    )
    .title(title)
    .inner_size(width, height)
    .min_inner_size(720.0, 520.0)
    // 无系统标题栏：顶部由应用自己做（拖拽手柄 + 窗口按钮），与岛体风格统一。
    .decorations(false)
    .center()
    .build()
    .map_err(|error| error.to_string())?;
    Ok(())
}
