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
        // 实际生效的端口：端口冲突时会回退，配置里的 port ≠ 实际监听端口。
        "actual_port": status.addr.as_deref().and_then(|addr| {
            addr.rsplit(':').next().and_then(|port| port.parse::<u16>().ok())
        }),
        "allow_lan": config.allow_lan,
        // 内网访问地址（allow_lan 开着才给）：枚举本机网卡 IP，拼上实际端口。
        "lan_addr": if config.allow_lan {
            Some(format!("http://{}:{}", lan_ip().unwrap_or_else(|| "<内网IP>".into()), actual_port(&status)))
        } else {
            None
        },
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

/// 实际监听端口（回退后可能与配置不同）。
fn actual_port(status: &crate::gateway::state::ServerStatus) -> u16 {
    status
        .addr
        .as_deref()
        .and_then(|addr| addr.rsplit(':').next()?.parse::<u16>().ok())
        .unwrap_or(0)
}

/// 本机 primary 内网 IP（发 UDP 包到外网地址、看本地 socket 绑到哪个 IP ——
/// 不真正发包）。失败就给 None，面板显示占位符。
fn lan_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("223.5.5.5:80").ok()?;
    let local = socket.local_addr().ok()?;
    Some(local.ip().to_string())
}

/// 开关 HTTP 服务（重启岛后生效）与写入 API Key / 内网访问开关。
#[tauri::command]
pub async fn gateway_set_runtime(
    state: State<'_, SharedGateway>,
    enabled: Option<bool>,
    api_keys: Option<Vec<String>>,
    allow_lan: Option<bool>,
) -> Result<Value, String> {
    let config = state
        .edit_config(|config| {
            if let Some(enabled) = enabled {
                config.enabled = enabled;
            }
            if let Some(api_keys) = api_keys {
                config.api_keys = api_keys;
            }
            if let Some(allow_lan) = allow_lan {
                config.allow_lan = allow_lan;
            }
        })
        .map_err(message)?;
    Ok(json!({ "enabled": config.enabled, "allow_lan": config.allow_lan, "api_keys": config.api_keys.len() }))
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

/// 单账号真实探针：发一条最小请求验证这个账号到底能不能用。
///
/// `model` 可指定（不指定就用该渠道第一个可用模型），
/// 与参考实现的 `/accounts/test` 同口径：真实打上游、成功清错误、失败记冷却。
#[tauri::command]
pub async fn gateway_probe_account(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
    model: Option<String>,
) -> Result<Value, String> {
    let account = state
        .account(&channel_id, &uid)
        .ok_or_else(|| format!("账号 {channel_id}/{uid} 不存在"))?;
    let outcome =
        crate::gateway::core::dispatch::probe_account(
            &state,
            &account,
            model.as_deref(),
            std::time::Duration::from_secs(45),
        )
        .await
        // 探针失败要把**上游原文**带回去：只报"失败"等于让人猜。
        .map_err(|error| error.describe())?;
    Ok(serde_json::to_value(outcome).unwrap_or(Value::Null))
}

/// 该渠道可用于测试的模型列表（探针的下拉选项）。
#[tauri::command]
pub async fn gateway_probe_models(
    state: State<'_, SharedGateway>,
    channel_id: String,
) -> Result<Value, String> {
    let matrix = state.ensure_matrix(false).await;
    let models: Vec<String> = matrix
        .public_models()
        .into_iter()
        .filter(|model| {
            matrix
                .routes(model)
                .iter()
                .any(|route| route.channel_id == channel_id)
        })
        .collect();
    Ok(json!(models))
}

/// 渠道任务清单（面板「任务」表格）。渠道没有任务体系时返回 null。
#[tauri::command]
pub async fn gateway_list_tasks(
    state: State<'_, SharedGateway>,
    channel_id: String,
) -> Result<Value, String> {
    let adapter = state.channel(&channel_id).ok_or_else(|| format!("渠道 {channel_id} 不存在"))?;
    let account = state
        .first_account_for(&channel_id)
        .ok_or_else(|| format!("渠道 {channel_id} 还没有账号，先登录一个"))?;
    let tasks = adapter
        .list_tasks(&state.client, &account)
        .await
        .map_err(message)?;
    Ok(match tasks {
        Some(tasks) => Value::Array(
            tasks
                .into_iter()
                .map(|task| serde_json::to_value(task).unwrap_or(Value::Null))
                .collect(),
        ),
        None => Value::Null,
    })
}

/// 一键执行该账号的任务：**后台跑**，结果逐条进进度表，前端轮询即可实时看到。
///
/// 同一账号已在执行时直接返回 false（跳过），避免重复上报把任务搞乱。
#[tauri::command]
pub async fn gateway_run_tasks(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
) -> Result<Value, String> {
    let adapter = state
        .channel(&channel_id)
        .ok_or_else(|| format!("渠道 {channel_id} 不存在"))?;
    let account = state
        .account(&channel_id, &uid)
        .ok_or_else(|| format!("账号 {channel_id}/{uid} 不存在"))?;
    if !state.begin_task_run(&channel_id, &uid) {
        return Ok(json!({ "started": false, "reason": "该账号的任务正在执行中" }));
    }

    let shared = std::sync::Arc::clone(&state);
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    // 进度消费者：把每条结果写进状态表，前端轮询读到就能实时展示。
    let consumer_state = std::sync::Arc::clone(&shared);
    let consumer_channel = channel_id.clone();
    let consumer_uid = uid.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(result) = receiver.recv().await {
            consumer_state.push_task_result(&consumer_channel, &consumer_uid, result);
        }
    });

    let runner_state = std::sync::Arc::clone(&shared);
    let runner_channel = channel_id.clone();
    let runner_uid = uid.clone();
    let client = state.client.clone();
    tauri::async_runtime::spawn(async move {
        let snapshot = account.snapshot();
        let outcome = adapter.run_tasks(&client, &snapshot, sender).await;
        let (done, earned) = match &outcome {
            Ok(results) => (
                results.iter().filter(|r| r.status == "done").count() as u32,
                results.iter().map(|r| r.earned).sum(),
            ),
            Err(error) => {
                runner_state.fail_task_run(&runner_channel, &runner_uid, &error.message);
                return;
            }
        };
        runner_state.finish_task_run(&runner_channel, &runner_uid, done, earned);
    });

    Ok(json!({ "started": true }))
}

/// 任务执行进度（前端轮询）：running / 逐条结果 / 汇总。
#[tauri::command]
pub async fn gateway_task_progress(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
) -> Result<Value, String> {
    Ok(state.task_progress(&channel_id, &uid))
}

/// 该账号今天有哪些每日动作（签到 / 猫猫旅行 / 每日活跃对话）。
///
/// 动作清单由**渠道**按 realm 给出：国内版有签到与猫猫旅行、国际版有每日活跃对话，
/// 两边不重叠 —— 面板不认识任何具体渠道，只按清单渲染。
#[tauri::command]
pub async fn gateway_list_daily(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
) -> Result<Value, String> {
    let adapter = state
        .channel(&channel_id)
        .ok_or_else(|| format!("渠道 {channel_id} 不存在"))?;
    let account = state
        .account(&channel_id, &uid)
        .ok_or_else(|| format!("账号 {channel_id}/{uid} 不存在"))?;
    let actions = adapter
        .daily_actions(&state.client, &account.snapshot())
        .await
        .map_err(|error| error.describe())?;
    Ok(json!(actions))
}

/// 执行一个每日动作（签到 / 派出旅行 / 领奖 / 每日活跃对话）。
///
/// 同步返回：这几个动作都是单次请求，不像任务中心那样要几十秒 ——
/// 不需要后台跑 + 轮询那一套。
#[tauri::command]
pub async fn gateway_run_daily(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
    code: String,
) -> Result<Value, String> {
    let adapter = state
        .channel(&channel_id)
        .ok_or_else(|| format!("渠道 {channel_id} 不存在"))?;
    let account = state
        .account(&channel_id, &uid)
        .ok_or_else(|| format!("账号 {channel_id}/{uid} 不存在"))?;
    let outcome = adapter
        .run_daily_action(&state.client, &account.snapshot(), &code)
        .await
        .map_err(|error| error.describe())?;
    Ok(serde_json::to_value(outcome).unwrap_or(Value::Null))
}

/// 领取**单项**已达标任务的奖励（任务行上的「领取」）。
///
/// 与 `gateway_run_tasks` 的区别：这里不上报任何事件，只调领奖接口 ——
/// 用于"进度早就达标了、只是没领过"的情况（例如你在官方客户端手动做完的任务）。
#[tauri::command]
pub async fn gateway_claim_task(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
    code: String,
) -> Result<crate::gateway::channels::TaskRunResult, String> {
    let adapter = state
        .channel(&channel_id)
        .ok_or_else(|| format!("渠道 {channel_id} 不存在"))?;
    let account = state
        .account(&channel_id, &uid)
        .ok_or_else(|| format!("账号 {channel_id}/{uid} 不存在"))?;
    adapter
        .claim_task(&state.client, &account.snapshot(), &code)
        .await
        .map_err(|error| error.message)
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

/// 指定毫秒时间戳区间（日期选择器用）。桶宽按跨度自适应。
#[tauri::command]
pub async fn gateway_usage_series_range(
    state: State<'_, SharedGateway>,
    from_ms: i64,
    to_ms: i64,
    model: Option<String>,
) -> Result<Value, String> {
    let span_hours = ((to_ms - from_ms).max(1) / 3_600_000).max(1) as u64;
    let bucket = if span_hours > 72 { 24 } else { 1 };
    state
        .storage
        .usage_series_range(from_ms, to_ms, model.as_deref(), bucket)
        .map(Value::Array)
        .map_err(message)
}

/// 指定毫秒时间戳区间的汇总。
#[tauri::command]
pub async fn gateway_usage_summary_range(
    state: State<'_, SharedGateway>,
    from_ms: i64,
    to_ms: i64,
) -> Result<Value, String> {
    state
        .storage
        .usage_summary_range(from_ms, to_ms)
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
