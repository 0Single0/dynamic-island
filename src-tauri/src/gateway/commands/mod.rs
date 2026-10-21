//! 面向岛内面板的 Tauri 命令。
//!
//! 与 HTTP 面共用同一份 `SharedGateway`：面板免鉴权、免 CORS，也不依赖 HTTP 服务是否在跑。
//! 命令只做参数校验与转发，业务都在 `core` / `state` 里。
//!
//! 自定义供应商（渠道增删 + 「获取模型」）单独放 `providers`：那一块是**写配置**的完整流程，
//! 与这里"读状态"的命令不是一类。

mod providers;

// 必须 glob 重导出：Tauri 的 `#[tauri::command]` 还会生成 `__cmd__*` 辅助项，
// 逐项 `pub use` 不会把它们带出来，`generate_handler!` 就找不到命令。
pub use providers::*;

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
                // 面板据此决定「添加账号」是走轮询还是走**粘贴回调**。
                // Trae 的授权回调被上游强制成 127.0.0.1:8765，网关没有那个
                // 监听端口，只能让用户把地址栏整条 URL 交回来。缺了这个字段，
                // 前端会退化成轮询，于是永远只显示"正在等待授权…"。
                "login_needs_callback": channel.login_needs_callback(),
                // 有没有真正的授权页：false = login_url 只是一行填写说明（CodeArts），
                // 面板不能拿它去 openUrl。
                "login_has_auth_page": channel.login_has_auth_page(),
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

/// 完成一次「需要粘贴回调地址」的登录（Trae 这类渠道）。
///
/// 返回值与 `gateway_login_poll` 的成功分支同形，前端可以复用同一段入库提示。
#[tauri::command]
pub async fn gateway_login_complete(
    state: State<'_, SharedGateway>,
    channel_id: String,
    callback: String,
) -> Result<Value, String> {
    if callback.trim().is_empty() {
        return Err("请粘贴授权完成后浏览器地址栏里的完整 URL".into());
    }
    let adapter = state
        .channel(&channel_id)
        .ok_or_else(|| format!("渠道 {channel_id} 不存在"))?;
    let credential = adapter
        .login_complete(&state.client, callback.trim())
        .await
        .map_err(message)?;
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
            credential: Some(credential),
            model_allow: Vec::new(),
            model_rename: std::collections::BTreeMap::new(),
        })
        .map_err(message)?;
    let count = state.reload_accounts().map_err(message)?;
    // 新号顺手签一次到、抓一次额度；失败都不影响可用性。
    if let Some(account) = state.account(&channel_id, &uid) {
        let snapshot = account.snapshot();
        let _ = adapter.sign_in(&state.client, &snapshot).await;
        if let Ok(quota) = adapter.fetch_quota(&state.client, &snapshot).await {
            account.set_quota(quota);
        }
    }
    let _ = state.ensure_matrix(true).await;
    Ok(json!({
        "status": "ok",
        "account": { "channel_id": channel_id, "uid": uid, "nickname": nickname },
        "accounts": count,
    }))
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
                    model_allow: Vec::new(),
                    model_rename: std::collections::BTreeMap::new(),
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

/// 设置单个账号的并发上限（面板"请求中"列点开改）。`None` = 回落默认。
///
/// 立即生效：写库后重建账号池（`replace` 会带上运行态，冷却/额度不丢）。
#[tauri::command]
pub async fn gateway_set_account_max_inflight(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
    max_inflight: Option<u32>,
) -> Result<Value, String> {
    let Some(account) = state.account(&channel_id, &uid) else {
        return Err(format!("账号 {channel_id}/{uid} 不存在"));
    };
    // 先在池子上生效（不用等重建），再落库持久化。
    account.set_max_inflight(max_inflight.unwrap_or(state.config().concurrency.per_account.max(1)));
    state
        .storage
        .set_account_max_inflight(&channel_id, &uid, max_inflight)
        .map_err(message)?;
    state.reload_accounts().map_err(message)?;
    Ok(json!({ "ok": true, "max_inflight": account.max_inflight() }))
}

/// 改账号的**可变画像**：昵称 / 优先级 / 权重（面板「编辑账号」）。
///
/// 四个字段都是**可选覆盖**，与后端的 `None` 语义一致：
/// · `nickname` 省略 = 不动；空串 = 清空（面板回落到 uid 前 8 位）；
/// · `priority / weight / max_inflight` 省略或 null = 清掉覆盖、回落默认。
///
/// 刻意不碰凭据、额度与冷却 —— 「编辑账号」不该有"顺手把登录态弄丢"的副作用。
#[tauri::command]
pub async fn gateway_set_account_profile(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
    nickname: Option<String>,
    priority: Option<u32>,
    weight: Option<u32>,
    max_inflight: Option<u32>,
) -> Result<Value, String> {
    if state.account(&channel_id, &uid).is_none() {
        return Err(format!("账号 {channel_id}/{uid} 不存在"));
    }
    // 昵称清洗：去首尾空白；超长会顶坏面板的固定列宽，截到 40 个字符。
    // **空串是有意义的**（= 清空，视图回落到 uid 前 8 位），所以不能把它折成 None：
    // None 在这里表示"别动昵称"。
    let nickname = nickname.map(|value| value.trim().chars().take(40).collect::<String>());
    let changed = state
        .storage
        .set_account_profile(
            &channel_id,
            &uid,
            nickname.as_deref(),
            priority,
            weight,
            max_inflight,
        )
        .map_err(message)?;
    state.reload_accounts().map_err(message)?;
    // 回带**生效值**，面板据此刷新那一行（而不是让前端自己再猜一次回落规则）。
    let account = state.account(&channel_id, &uid);
    Ok(json!({
        "ok": changed,
        "nickname": account.as_ref().map(|account| account.nickname.clone()).unwrap_or_default(),
        "priority": account.as_ref().map(|account| account.priority).unwrap_or_default(),
        "weight": account.as_ref().map(|account| account.weight).unwrap_or_default(),
        "max_inflight": account.as_ref().map(|account| account.max_inflight()).unwrap_or_default(),
    }))
}

/// 改账号的模型清单：白名单（空 = 不限）+ 账号级模型改名。
///
/// 白名单语义是"这个号**只**服务这些对外模型名"，空列表 = 该渠道有什么就能跑什么。
/// 改名解决的是同名不同姓：Trae 把 `deepseek-v4.1-flash` 叫 `DeepSeek-V4-Flash-Official`，
/// 配上 `{ "deepseek-v4.1-flash": "DeepSeek-V4-Flash-Official" }` 之后，
/// 对外仍然是统一的 `deepseek-v4.1-flash`，出站自动换成该账号的上游名。
///
/// 两样都只影响调度，不改凭据、额度和冷却。
#[tauri::command]
pub async fn gateway_set_account_models(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
    allow: Vec<String>,
    rename: std::collections::BTreeMap<String, String>,
) -> Result<Value, String> {
    if state.account(&channel_id, &uid).is_none() {
        return Err(format!("账号 {channel_id}/{uid} 不存在"));
    }
    // 清掉空白项：面板里删到只剩空格时，别让它变成一条匹配不上的白名单。
    let allow: Vec<String> = allow
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    // 改名两边都去空白；映射到空名的条目等于让请求不带模型名，直接丢掉。
    let rename: std::collections::BTreeMap<String, String> = rename
        .into_iter()
        .map(|(from, to)| (from.trim().to_string(), to.trim().to_string()))
        .filter(|(from, to)| !from.is_empty() && !to.is_empty())
        .collect();
    let changed = state
        .storage
        .set_account_models(&channel_id, &uid, &allow, &rename)
        .map_err(message)?;

    // 自动补路由：账号把某渠道的模型改名成"本渠道目录里没有的对外名"时，
    // 矩阵里根本没有这条路由 —— 不改配置的话，客户端请求那个对外名会 400
    //（`没有渠道提供模型 X`）。所以替用户把这条路由覆盖写上。
    //
    // 只补"缺失"的，且只在 `upstream_model` 指向本渠道真实目录里的模型时才补：
    // 编一条指向不存在模型的路由，比不补更糟（请求一定打空）。
    // 直接问渠道要目录，**不看矩阵**：矩阵里已经有我们刚写进去的改名路由，
    // 拿它当"目录"会自我验证 —— 一个拼错的上游名也能通过校验。
    // `None` = 这次没拿到目录（网络/凭据/上游改版），**不等于**目录为空。
    // 这个区别很关键：拿不到目录时如果当成"没有模型"，所有改名都会被判成
    // "不在该渠道目录里"而静默丢弃 —— 用户看到的就是"保存了但没生效"。
    let catalog_names: Option<std::collections::HashSet<String>> = match state.channel(&channel_id) {
        Some(adapter) => {
            let account = state
                .first_account_for(&channel_id)
                .unwrap_or_else(|| crate::gateway::channels::AccountSnapshot {
                    channel_id: channel_id.clone(),
                    ..Default::default()
                });
            match adapter.list_models(&state.client, &account).await {
                Ok(Some(models)) => Some(models.into_iter().map(|model| model.id).collect()),
                _ => None,
            }
        }
        None => None,
    };

    let mut added: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();

    // 先清掉**这个渠道下、当前改名表里已不存在的**路由覆盖。
    //
    // 为什么必须清：改名的副作用是往 `config.routes` 里补一条覆盖。
    // 如果用户后来删掉改名（或改成了别的名字），那条覆盖会留在矩阵里，
    // 把该渠道的上游模型名**永久改写**成旧值 —— 表现为"目录里明明有
    // `DeepSeek-V4-Flash-Official`，下拉里却没有它"。
    //
    // 只清"名字形如对外模型名"且本渠道当前目录里没有的条目，避免误伤
    // 用户手工在 config.json 里写的路由覆盖。
    state
        .edit_config(|config| {
            config.routes.retain(|entry| {
                if entry.channel_id != channel_id {
                    return true;
                }
                // 仍在改名表里：保留（下面会更新它的 upstream_model）。
                if rename.contains_key(&entry.model) {
                    return true;
                }
                // 目录里本来就有这个名字：说明是正常的同名路由，不动。
                if catalog_names
                    .as_ref()
                    .is_some_and(|names| names.contains(&entry.model))
                {
                    return true;
                }
                removed.push(entry.model.clone());
                false
            });
        })
        .map_err(message)?;
    if !rename.is_empty() {
        state
            .edit_config(|config| {
                for (public_name, upstream) in &rename {
                    // 注意这里校验的是 **upstream**（右边，本渠道真实目录名），
                    // 而不是 `public_name` —— 对外名本来就是"本渠道没有的名字"，
                    // 它不可能出现在目录里。早先校验错了方向，导致所有跨渠道改名
                    // 都被判成"不在该渠道目录里"、路由永远补不上。
                    // 只有**确实拿到了目录**时才校验上游名。
                    // 没拿到目录时信任用户的输入：他已经从下拉里选了本渠道的
                    // 真实模型名（下拉候选就来自目录），再拦一次只会让功能看起来坏掉。
                    if let Some(names) = &catalog_names {
                        if !names.contains(upstream) {
                            skipped.push(format!(
                                "{public_name}：{upstream} 不在该渠道目录里（改名未生效）"
                            ));
                            continue;
                        }
                    }
                    let existing = config
                        .routes
                        .iter_mut()
                        .find(|entry| entry.model == *public_name && entry.channel_id == channel_id);
                    match existing {
                        Some(entry) => {
                            // 已有覆盖：把上游名对齐到账号这边的映射。
                            entry.upstream_model = Some(upstream.clone());
                        }
                        None => {
                            config.routes.push(RouteOverride {
                                model: public_name.clone(),
                                channel_id: channel_id.clone(),
                                upstream_model: Some(upstream.clone()),
                                priority: None,
                                weight: None,
                                enabled: Some(true),
                            });
                            added.push(public_name.clone());
                        }
                    }
                }
            })
            .map_err(message)?;
        // 配置变了 → 矩阵按需重建，新的对外名立刻可请求。
        let _ = state.ensure_matrix(true).await;
    }

    state.reload_accounts().map_err(message)?;
    Ok(json!({
        "ok": changed,
        "allow": allow,
        "rename": rename,
        // 新增的路由与因目录缺模型而跳过的条目：面板据此提示，别让用户以为全都生效了。
        "routes_added": added,
        "skipped": skipped,
    }))
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
    // 面板上的"刷新凭据"是**明确动作**：必须真的换一个，不能被"未到期"短路。
    let credential = refresh::refresh(&state, &account, true).await.map_err(message)?;
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
    let snapshot = match adapter.fetch_quota(&state.client, &account.snapshot()).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            // 额度接口明确说"凭据不被接受"时，账号其实已经不可用了 ——
            // 只把错误弹给用户、却让状态灯继续显示"可用"，会出现
            // 「状态可用 + 额度报 1001」这种自相矛盾的画面，也没法解释
            // 为什么点刷新没用。这里把惩罚状态落到账号上（与调度路径一致），
            // 面板的状态与原因就会如实变红。
            if error.kind == crate::gateway::errors::ErrorKind::AuthExpired {
                let policy = state.pool.policy();
                account.apply_error(&error, &policy, now_ms());
            }
            return Err(message(error));
        }
    };
    account.set_quota(snapshot.clone());
    // `null` 的语义必须让面板能读懂：这个渠道**没有额度概念**（CodeArts 按 token
    // 计量、只在控制台看），不是"查询失败"。早先两者都显示成 `—`，
    // 用户完全分不清是没实现还是坏了。
    Ok(json!({
        "supported": snapshot.is_some(),
        "quota": snapshot,
    }))
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

/// 诊断：用某个账号的真实凭据打一次上游，原样返回响应。
///
/// `target` 由渠道自己解释（Trae：`models` / `detail` / `credits`）。
/// 存在的理由：面板把缺失字段显示成 `—`/`?` 时，只有原始 JSON 能说清
/// 到底是"上游没给"、"字段名不同"还是"接口失败了"。
#[tauri::command]
pub async fn gateway_diagnose_channel(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
    target: String,
) -> Result<Value, String> {
    let account = state
        .account(&channel_id, &uid)
        .ok_or_else(|| format!("账号 {channel_id}/{uid} 不存在"))?;
    let adapter = state
        .channel(&channel_id)
        .ok_or_else(|| format!("渠道 {channel_id} 不存在"))?;
    adapter
        .diagnose(&state.client, &account.snapshot(), &target)
        .await
        .map_err(message)
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
            model_allow: Vec::new(),
            model_rename: std::collections::BTreeMap::new(),
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
