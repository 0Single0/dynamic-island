//! Web 面板：管理 API（`/api/gateway/*`）+ 登录（`/api/auth/*`）+ 静态资源托管。
//!
//! 与下游反代面（`/v1/*`）**分端口**：下游面给客户端程序用、按 `api_keys` 鉴权；
//! 面板面给人用、按会话 Cookie 鉴权。两者共享同一份 [`SharedGateway`] 状态。
//!
//! 约定：请求体字段名与前端 TypeScript 类型一致（camelCase），
//! 响应统一 `{ code, data, msg }`（`code === 0` 成功），与 `src/types/api.ts` 对齐。

pub mod auth;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

use crate::gateway::{commands, config::AccountConfig, state::SharedGateway};
use auth::AuthConfig;

/// 面板共享上下文：网关状态 + 鉴权配置。
#[derive(Clone)]
pub struct PanelCtx {
    pub gateway: SharedGateway,
    pub auth: AuthConfig,
}

/// 面板管理 API 路由（不含静态资源）。
pub fn router(ctx: PanelCtx) -> Router {
    Router::new()
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/session", get(auth::session))
        .route("/api/gateway/status", post(status))
        .route("/api/gateway/set-runtime", post(set_runtime))
        .route("/api/gateway/refresh-matrix", post(refresh_matrix))
        .route("/api/gateway/list-channels", post(list_channels))
        .route("/api/gateway/update-channel", post(update_channel))
        .route("/api/gateway/list-accounts", post(list_accounts))
        .route("/api/gateway/login-start", post(login_start))
        .route("/api/gateway/login-poll", post(login_poll))
        .route("/api/gateway/set-account-enabled", post(set_account_enabled))
        .route(
            "/api/gateway/set-account-max-inflight",
            post(set_account_max_inflight),
        )
        .route("/api/gateway/delete-account", post(delete_account))
        .route("/api/gateway/refresh-account", post(refresh_account))
        .route("/api/gateway/fetch-quota", post(fetch_quota))
        .route("/api/gateway/probe-account", post(probe_account))
        .route("/api/gateway/probe-models", post(probe_models))
        .route("/api/gateway/list-tasks", post(list_tasks))
        .route("/api/gateway/run-tasks", post(run_tasks))
        .route("/api/gateway/task-progress", post(task_progress))
        .route("/api/gateway/list-daily", post(list_daily))
        .route("/api/gateway/run-daily", post(run_daily))
        .route("/api/gateway/claim-task", post(claim_task))
        .route("/api/gateway/reset-account-state", post(reset_account_state))
        .route("/api/gateway/checkin-account", post(checkin_account))
        .route("/api/gateway/add-account", post(add_account))
        .route("/api/gateway/list-routes", post(list_routes))
        .route("/api/gateway/set-overrides", post(set_overrides))
        .route("/api/gateway/clear-affinity", post(clear_affinity))
        .route("/api/gateway/recent-decisions", post(recent_decisions))
        .route("/api/gateway/recent-usage", post(recent_usage))
        .route("/api/gateway/usage-series", post(usage_series))
        .route("/api/gateway/usage-series-range", post(usage_series_range))
        .route("/api/gateway/usage-summary-range", post(usage_summary_range))
        .route("/api/gateway/usage-summary", post(usage_summary))
        .route("/api/gateway/upsert-account-config", post(upsert_account_config))
        .route("/api/gateway/paths", post(paths))
        .route("/api/gateway/scheduler-snapshot", post(scheduler_snapshot))
        .with_state(ctx)
}

// ===== 响应封装 =====

/// 成功：`code = 0`，`data` 为命令返回值。
fn ok(data: Value) -> Response {
    (
        StatusCode::OK,
        Json(json!({ "code": 0, "data": data, "msg": "" })),
    )
        .into_response()
}

/// 业务失败：HTTP 仍 200，`code != 0` —— 前端拦截器据此 reject 并带上 `msg`。
///
/// 为什么不用 4xx/5xx：前端要的是**上游原文**（`errorText()` 直接展示），
/// 而 HTTP 错误码会把语义压平成"请求失败"，丢掉"上游到底报了什么"。
fn fail(message: impl Into<String>) -> Response {
    (
        StatusCode::OK,
        Json(json!({ "code": 1, "data": Value::Null, "msg": message.into() })),
    )
        .into_response()
}

/// 统一收敛命令结果；顺带处理鉴权。
fn finish(result: Result<Value, String>) -> Response {
    match result {
        Ok(data) => ok(data),
        Err(message) => fail(message),
    }
}

// ===== 参数取值（前端传 camelCase）=====

fn req_str(body: &Value, key: &str) -> Result<String, String> {
    body.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("缺少参数 {key}"))
}

fn opt_str(body: &Value, key: &str) -> Option<String> {
    body.get(key).and_then(Value::as_str).map(str::to_string)
}

fn opt_bool(body: &Value, key: &str) -> Option<bool> {
    body.get(key).and_then(Value::as_bool)
}

fn opt_u32(body: &Value, key: &str) -> Option<u32> {
    body.get(key).and_then(Value::as_u64).map(|value| value as u32)
}

fn opt_u64(body: &Value, key: &str) -> Option<u64> {
    body.get(key).and_then(Value::as_u64)
}

fn req_i64(body: &Value, key: &str) -> Result<i64, String> {
    body.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("缺少参数 {key}"))
}

/// 鉴权守卫：未登录直接返回 401。
macro_rules! guard {
    ($ctx:expr, $headers:expr) => {
        if let Some(response) = auth::require_session(&$ctx.auth, &$headers) {
            return response;
        }
    };
}

// ===== 各端点 =====

async fn status(State(ctx): State<PanelCtx>, headers: HeaderMap) -> Response {
    guard!(ctx, headers);
    match commands::gateway_status(&ctx.gateway).await {
        Ok(mut data) => {
            // 容器里 `addr` 是 `http://0.0.0.0:8790`，对用户没有意义：
            // 有 `PUBLIC_BASE_URL` 就换成它，否则按访问面板用的 Host 推一个。
            if let Some(public) = public_base_url().or_else(|| inferred_base_url(&headers)) {
                if let Some(object) = data.as_object_mut() {
                    object.insert("addr".into(), Value::String(public.clone()));
                    object.insert("public_base_url".into(), Value::String(public));
                }
            }
            ok(data)
        }
        Err(message) => fail(message),
    }
}

/// 客户端该把 base URL 指到哪里（部署时用 `PUBLIC_BASE_URL` 显式声明）。
fn public_base_url() -> Option<String> {
    std::env::var("PUBLIC_BASE_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
}

/// 没配 `PUBLIC_BASE_URL` 时，按面板的访问 Host 推一个（端口换成下游端口）。
///
/// 比 `http://0.0.0.0:8790` 有用得多 —— 用户正是用这个主机名打开面板的。
/// 只是提示，不参与任何鉴权，所以 Host 头被伪造也无所谓。
fn inferred_base_url(headers: &HeaderMap) -> Option<String> {
    let host = headers
        .get(axum::http::header::HOST)?
        .to_str()
        .ok()?
        .trim()
        .to_string();
    if host.is_empty() {
        return None;
    }
    let gateway_port = std::env::var("GATEWAY_PORT").unwrap_or_else(|_| "8790".into());
    let hostname = host.rsplit_once(':').map(|(name, _)| name).unwrap_or(&host);
    Some(format!("http://{hostname}:{gateway_port}"))
}

async fn set_runtime(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let api_keys = body
        .get("apiKeys")
        .and_then(Value::as_array)
        .map(|keys| {
            keys.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<String>>()
        });
    finish(
        commands::gateway_set_runtime(
            &ctx.gateway,
            opt_bool(&body, "enabled"),
            api_keys,
            opt_bool(&body, "allowLan"),
        )
        .await,
    )
}

async fn refresh_matrix(State(ctx): State<PanelCtx>, headers: HeaderMap) -> Response {
    guard!(ctx, headers);
    finish(commands::gateway_refresh_matrix(&ctx.gateway).await)
}

async fn list_channels(State(ctx): State<PanelCtx>, headers: HeaderMap) -> Response {
    guard!(ctx, headers);
    finish(commands::gateway_list_channels(&ctx.gateway).await)
}

async fn update_channel(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let channel_id = match req_str(&body, "channelId") {
        Ok(value) => value,
        Err(message) => return fail(message),
    };
    finish(
        commands::gateway_update_channel(
            &ctx.gateway,
            channel_id,
            opt_bool(&body, "enabled"),
            opt_u32(&body, "priority"),
            opt_u32(&body, "weight"),
        )
        .await,
    )
}

async fn list_accounts(State(ctx): State<PanelCtx>, headers: HeaderMap) -> Response {
    guard!(ctx, headers);
    finish(commands::gateway_list_accounts(&ctx.gateway).await)
}

async fn login_start(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    match req_str(&body, "channelId") {
        Ok(channel_id) => finish(commands::gateway_login_start(&ctx.gateway, channel_id).await),
        Err(message) => fail(message),
    }
}

async fn login_poll(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (channel_id, login_state) = match (req_str(&body, "channelId"), req_str(&body, "loginState"))
    {
        (Ok(channel_id), Ok(login_state)) => (channel_id, login_state),
        (Err(message), _) | (_, Err(message)) => return fail(message),
    };
    finish(commands::gateway_login_poll(&ctx.gateway, channel_id, login_state).await)
}

async fn set_account_enabled(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (channel_id, uid) = match (req_str(&body, "channelId"), req_str(&body, "uid")) {
        (Ok(channel_id), Ok(uid)) => (channel_id, uid),
        (Err(message), _) | (_, Err(message)) => return fail(message),
    };
    let enabled = opt_bool(&body, "enabled").unwrap_or(true);
    finish(commands::gateway_set_account_enabled(&ctx.gateway, channel_id, uid, enabled).await)
}

async fn set_account_max_inflight(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (channel_id, uid) = match (req_str(&body, "channelId"), req_str(&body, "uid")) {
        (Ok(channel_id), Ok(uid)) => (channel_id, uid),
        (Err(message), _) | (_, Err(message)) => return fail(message),
    };
    finish(
        commands::gateway_set_account_max_inflight(
            &ctx.gateway,
            channel_id,
            uid,
            opt_u32(&body, "maxInflight"),
        )
        .await,
    )
}

async fn delete_account(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (channel_id, uid) = match (req_str(&body, "channelId"), req_str(&body, "uid")) {
        (Ok(channel_id), Ok(uid)) => (channel_id, uid),
        (Err(message), _) | (_, Err(message)) => return fail(message),
    };
    finish(commands::gateway_delete_account(&ctx.gateway, channel_id, uid).await)
}

async fn refresh_account(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (channel_id, uid) = match (req_str(&body, "channelId"), req_str(&body, "uid")) {
        (Ok(channel_id), Ok(uid)) => (channel_id, uid),
        (Err(message), _) | (_, Err(message)) => return fail(message),
    };
    finish(commands::gateway_refresh_account(&ctx.gateway, channel_id, uid).await)
}

async fn fetch_quota(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (channel_id, uid) = match (req_str(&body, "channelId"), req_str(&body, "uid")) {
        (Ok(channel_id), Ok(uid)) => (channel_id, uid),
        (Err(message), _) | (_, Err(message)) => return fail(message),
    };
    finish(commands::gateway_fetch_quota(&ctx.gateway, channel_id, uid).await)
}

async fn probe_account(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (channel_id, uid) = match (req_str(&body, "channelId"), req_str(&body, "uid")) {
        (Ok(channel_id), Ok(uid)) => (channel_id, uid),
        (Err(message), _) | (_, Err(message)) => return fail(message),
    };
    finish(
        commands::gateway_probe_account(&ctx.gateway, channel_id, uid, opt_str(&body, "model")).await,
    )
}

async fn probe_models(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    match req_str(&body, "channelId") {
        Ok(channel_id) => finish(commands::gateway_probe_models(&ctx.gateway, channel_id).await),
        Err(message) => fail(message),
    }
}

async fn list_tasks(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    match req_str(&body, "channelId") {
        Ok(channel_id) => finish(commands::gateway_list_tasks(&ctx.gateway, channel_id).await),
        Err(message) => fail(message),
    }
}

async fn run_tasks(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (channel_id, uid) = match (req_str(&body, "channelId"), req_str(&body, "uid")) {
        (Ok(channel_id), Ok(uid)) => (channel_id, uid),
        (Err(message), _) | (_, Err(message)) => return fail(message),
    };
    finish(commands::gateway_run_tasks(&ctx.gateway, channel_id, uid).await)
}

async fn task_progress(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (channel_id, uid) = match (req_str(&body, "channelId"), req_str(&body, "uid")) {
        (Ok(channel_id), Ok(uid)) => (channel_id, uid),
        (Err(message), _) | (_, Err(message)) => return fail(message),
    };
    finish(commands::gateway_task_progress(&ctx.gateway, channel_id, uid).await)
}

async fn list_daily(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (channel_id, uid) = match (req_str(&body, "channelId"), req_str(&body, "uid")) {
        (Ok(channel_id), Ok(uid)) => (channel_id, uid),
        (Err(message), _) | (_, Err(message)) => return fail(message),
    };
    finish(commands::gateway_list_daily(&ctx.gateway, channel_id, uid).await)
}

async fn run_daily(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (channel_id, uid, code) = match (
        req_str(&body, "channelId"),
        req_str(&body, "uid"),
        req_str(&body, "code"),
    ) {
        (Ok(channel_id), Ok(uid), Ok(code)) => (channel_id, uid, code),
        (Err(message), _, _) | (_, Err(message), _) | (_, _, Err(message)) => return fail(message),
    };
    finish(commands::gateway_run_daily(&ctx.gateway, channel_id, uid, code).await)
}

async fn claim_task(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (channel_id, uid, code) = match (
        req_str(&body, "channelId"),
        req_str(&body, "uid"),
        req_str(&body, "code"),
    ) {
        (Ok(channel_id), Ok(uid), Ok(code)) => (channel_id, uid, code),
        (Err(message), _, _) | (_, Err(message), _) | (_, _, Err(message)) => return fail(message),
    };
    match commands::gateway_claim_task(&ctx.gateway, channel_id, uid, code).await {
        Ok(result) => ok(serde_json::to_value(result).unwrap_or(Value::Null)),
        Err(message) => fail(message),
    }
}

async fn reset_account_state(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (channel_id, uid) = match (req_str(&body, "channelId"), req_str(&body, "uid")) {
        (Ok(channel_id), Ok(uid)) => (channel_id, uid),
        (Err(message), _) | (_, Err(message)) => return fail(message),
    };
    finish(commands::gateway_reset_account_state(&ctx.gateway, channel_id, uid).await)
}

async fn checkin_account(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (channel_id, uid) = match (req_str(&body, "channelId"), req_str(&body, "uid")) {
        (Ok(channel_id), Ok(uid)) => (channel_id, uid),
        (Err(message), _) | (_, Err(message)) => return fail(message),
    };
    finish(commands::gateway_checkin_account(&ctx.gateway, channel_id, uid).await)
}

async fn add_account(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (channel_id, uid) = match (req_str(&body, "channelId"), req_str(&body, "uid")) {
        (Ok(channel_id), Ok(uid)) => (channel_id, uid),
        (Err(message), _) | (_, Err(message)) => return fail(message),
    };
    finish(
        commands::gateway_add_account(&ctx.gateway, channel_id, uid, opt_str(&body, "nickname"))
            .await,
    )
}

async fn list_routes(State(ctx): State<PanelCtx>, headers: HeaderMap) -> Response {
    guard!(ctx, headers);
    finish(commands::gateway_list_routes(&ctx.gateway).await)
}

async fn set_overrides(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let aliases = body
        .get("aliases")
        .cloned()
        .map(serde_json::from_value::<std::collections::HashMap<String, String>>)
        .transpose();
    let aliases = match aliases {
        Ok(value) => value,
        Err(error) => return fail(format!("aliases 格式不对：{error}")),
    };
    let routes = body
        .get("routes")
        .cloned()
        .map(serde_json::from_value::<Vec<crate::gateway::config::RouteOverride>>)
        .transpose();
    let routes = match routes {
        Ok(value) => value,
        Err(error) => return fail(format!("routes 格式不对：{error}")),
    };
    finish(commands::gateway_set_overrides(&ctx.gateway, aliases, routes).await)
}

async fn clear_affinity(State(ctx): State<PanelCtx>, headers: HeaderMap) -> Response {
    guard!(ctx, headers);
    finish(commands::gateway_clear_affinity(&ctx.gateway).await)
}

async fn recent_decisions(State(ctx): State<PanelCtx>, headers: HeaderMap) -> Response {
    guard!(ctx, headers);
    finish(commands::gateway_recent_decisions(&ctx.gateway).await)
}

async fn recent_usage(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    finish(commands::gateway_recent_usage(&ctx.gateway, opt_u32(&body, "limit")).await)
}

async fn usage_series(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    finish(
        commands::gateway_usage_series(
            &ctx.gateway,
            opt_u64(&body, "hours"),
            opt_str(&body, "model"),
            opt_u64(&body, "bucketHours"),
        )
        .await,
    )
}

async fn usage_series_range(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (from_ms, to_ms) = match (req_i64(&body, "fromMs"), req_i64(&body, "toMs")) {
        (Ok(from_ms), Ok(to_ms)) => (from_ms, to_ms),
        (Err(message), _) | (_, Err(message)) => return fail(message),
    };
    finish(
        commands::gateway_usage_series_range(&ctx.gateway, from_ms, to_ms, opt_str(&body, "model"))
            .await,
    )
}

async fn usage_summary_range(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let (from_ms, to_ms) = match (req_i64(&body, "fromMs"), req_i64(&body, "toMs")) {
        (Ok(from_ms), Ok(to_ms)) => (from_ms, to_ms),
        (Err(message), _) | (_, Err(message)) => return fail(message),
    };
    finish(commands::gateway_usage_summary_range(&ctx.gateway, from_ms, to_ms).await)
}

async fn usage_summary(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    finish(commands::gateway_usage_summary(&ctx.gateway, opt_u64(&body, "hours")).await)
}

async fn upsert_account_config(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    guard!(ctx, headers);
    let account: AccountConfig = match serde_json::from_value(body) {
        Ok(account) => account,
        Err(error) => return fail(format!("账号配置格式不对：{error}")),
    };
    finish(commands::gateway_upsert_account_config(&ctx.gateway, account).await)
}

async fn paths(State(ctx): State<PanelCtx>, headers: HeaderMap) -> Response {
    guard!(ctx, headers);
    finish(commands::gateway_paths(&ctx.gateway).await)
}

async fn scheduler_snapshot(State(ctx): State<PanelCtx>, headers: HeaderMap) -> Response {
    guard!(ctx, headers);
    finish(commands::gateway_scheduler_snapshot(&ctx.gateway).await)
}

/// 面板路由 + 静态资源（同一端口）。
///
/// 静态资源走 **fallback**（而不是 merge）：这样 `/api/*` 永远优先。
///
/// 关键点：`/assets/*` 单独挂一个 `ServeDir`，**miss 就是 404，不回落 `index.html`**。
/// 因为 `index.html` 里的资源名带内容哈希，重新部署后旧名字就没了。此时浏览器
/// （或反代）拿着缓存的旧 `index.html` 去要旧 chunk，若服务端回 **200 + HTML**，
/// 浏览器会按 `text/html` 解析 JS，报出 `Unexpected token '<'` 这种和真实原因
/// 毫无关系的错，极难排查。404 能让浏览器干脆地重新拉 `index.html`。
///
/// 其余路径仍然回落到 SPA 入口，前端刷新子路径不会 404。
pub fn full_router(ctx: PanelCtx, web_dir: Option<std::path::PathBuf>) -> Router {
    let api = router(ctx);
    let Some(dir) = web_dir else {
        return api;
    };
    use tower::ServiceBuilder;
    use tower_http::services::{ServeDir, ServeFile};

    let index = dir.join("index.html");

    // 构建产物：内容哈希命名 ⇒ 可以长缓存。
    let assets = ServiceBuilder::new()
        .map_response(|response| cache_when_ok(ASSET_CACHE, response))
        .service(ServeDir::new(dir.join("assets")));

    // 其余文件（index.html、favicon…）：每次回源校验，避免拿到过期的入口。
    let spa = ServiceBuilder::new()
        .map_response(|response| cache_when_ok(ENTRY_CACHE, response))
        .service(ServeDir::new(&dir).fallback(ServeFile::new(index)));

    api.nest_service("/assets", assets).fallback_service(spa)
}

/// 带哈希的构建产物：内容变则文件名变，可以放心长缓存。
const ASSET_CACHE: &str = "public, max-age=31536000, immutable";
/// 入口文件：必须回源校验，否则用户永远刷不出新版本。
const ENTRY_CACHE: &str = "no-cache";

/// 只给 **200** 加 `Cache-Control`。
///
/// 404 绝不能带缓存头：部署瞬间 `assets/` 可能还没铺好，一个真实存在的资源
/// 会先 404 一次；若这条 404 被 `immutable` 记上一整年，就成了持续一年的白屏。
fn cache_when_ok<B>(value: &'static str, mut response: axum::http::Response<B>) -> axum::http::Response<B> {
    use axum::http::header::{HeaderValue, CACHE_CONTROL};
    if response.status() == StatusCode::OK {
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static(value));
    }
    response
}

#[cfg(test)]
mod static_tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    /// 造一个最小的 `dist/`：一个入口 + 一个带哈希的资源。
    fn web_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("临时目录");
        std::fs::create_dir_all(dir.path().join("assets")).expect("建 assets");
        std::fs::write(dir.path().join("index.html"), "<!doctype html>SPA").expect("写入口");
        std::fs::write(dir.path().join("assets/app-abc123.js"), "console.log(1)").expect("写资源");
        dir
    }

    fn app(dir: &std::path::Path) -> Router {
        let ctx = PanelCtx {
            gateway: gateway(),
            auth: AuthConfig::for_tests(),
        };
        full_router(ctx, Some(dir.to_path_buf()))
    }

    /// 一个空号池的网关状态：静态资源测试不碰调度，只要它能构造出来。
    fn gateway() -> SharedGateway {
        let dir = tempfile::tempdir().expect("临时目录");
        let mut config = crate::gateway::config::Config::default();
        config.channels.clear();
        crate::gateway::state::GatewayState::initialize_with_secrets(
            config,
            dir.path().to_path_buf(),
            crate::gateway::security::tests_support::memory_store().as_ref(),
        )
        .expect("状态初始化")
    }

    async fn get(app: &Router, uri: &str) -> (StatusCode, Option<String>, Option<String>) {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("请求构造"),
            )
            .await
            .expect("路由命中");
        let status = response.status();
        let header = |name: axum::http::HeaderName| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        };
        (
            status,
            header(axum::http::header::CONTENT_TYPE),
            header(axum::http::header::CACHE_CONTROL),
        )
    }

    #[tokio::test]
    async fn existing_asset_is_served_and_cached_forever() {
        let dir = web_dir();
        let app = app(dir.path());
        let (status, content_type, cache) = get(&app, "/assets/app-abc123.js").await;
        assert_eq!(status, StatusCode::OK);
        assert!(content_type.unwrap_or_default().contains("javascript"));
        assert!(cache.unwrap_or_default().contains("immutable"));
    }

    /// 回归：重新部署后旧哈希资源必须 404，**不能**回落成 200 + HTML。
    ///
    /// 回落 HTML 会让浏览器拿 `text/html` 去解析 JS，报
    /// `Unexpected token '<'`，把真实原因（资源已被替换）藏得干干净净。
    #[tokio::test]
    async fn missing_asset_is_404_and_never_falls_back_to_html() {
        let dir = web_dir();
        let app = app(dir.path());
        let (status, content_type, cache) = get(&app, "/assets/app-STALE999.js").await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "旧哈希资源必须 404，否则前端会拿到 HTML 当 JS 用"
        );
        assert!(
            !content_type.unwrap_or_default().contains("html"),
            "404 响应不该带 HTML 内容类型"
        );
        // 404 若带上 `immutable`，一个"暂时还没有"的资源会被记成"一年都没有"。
        assert!(
            cache.is_none(),
            "404 不能带 Cache-Control（否则缺失会被缓存一整年），实际为 {cache:?}"
        );
    }

    #[tokio::test]
    async fn spa_deep_link_still_falls_back_to_index() {
        let dir = web_dir();
        let app = app(dir.path());
        let (status, content_type, cache) = get(&app, "/accounts/detail").await;
        assert_eq!(status, StatusCode::OK);
        assert!(content_type.unwrap_or_default().contains("html"));
        // 入口文件不能长缓存，否则永远刷不出新版本。
        assert_eq!(cache.as_deref(), Some("no-cache"));
    }

    #[tokio::test]
    async fn root_serves_index() {
        let dir = web_dir();
        let app = app(dir.path());
        let (status, content_type, _) = get(&app, "/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(content_type.unwrap_or_default().contains("html"));
    }
}
