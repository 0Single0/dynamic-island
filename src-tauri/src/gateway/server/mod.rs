//! HTTP 服务：监听、路由、端口回退、健康探针。
//!
//! 只绑回环：外部客户端在同一台机器上按三种协议之一调用即可，不识别调用方身份。
//! 面板不经过这里（走 Tauri command），所以这里没有面板会话鉴权。

pub mod anthropic;
pub mod openai;
pub mod responses;
pub mod sse;

use std::sync::atomic::Ordering;

use axum::{
    extract::State,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::json;

use crate::gateway::{
    config::PORT_SCAN_RANGE,
    state::{ServerStatus, SharedGateway},
    timeutil::now_ms,
};

pub fn router(state: SharedGateway) -> Router {
    use axum::extract::DefaultBodyLimit;
    // 请求体大小**完全不限制**（axum 默认 2MB 会把长上下文的请求在网关就 413 掉）。
    // 上限交给上游：超了上游会自己报错，我们原样透传 —— 网关不做第二道闸。
    // 注意 layer 的顺序：axum 的 .layer() 包装的是**在此之前**定义的路由，
    // 所以 DefaultBodyLimit 必须放在 .route() 之后调用才能作用到它们身上
    // （放前面 = 作用在空 Router 上 = 完全不生效，这正是 413 一直阴魂不散的原因）。
    Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(openai::list_models))
        .route("/v1/chat/completions", post(openai::chat_completions))
        // 旧版 completions 与 chat 同管线（协议差异在解码层归一）。
        .route("/v1/completions", post(openai::chat_completions))
        // Anthropic Messages：Claude Code 等客户端；错误信封与事件序列都是 Anthropic 形状。
        .route("/v1/messages", post(anthropic::messages))
        .route("/v1/messages/count_tokens", post(anthropic::count_tokens))
        // OpenAI Responses：Codex 等客户端；无状态网关，previous_response_id 会被拒绝。
        .route("/v1/responses", post(responses::create))
        .layer(DefaultBodyLimit::disable())
        .with_state(state)
}

/// 起服务：绑定失败只写状态，不 panic —— 网关挂了岛还得能用。
pub async fn run(state: SharedGateway) {
    let config = state.config();
    let (preferred, allow_lan) = (config.port, config.allow_lan);
    let listener = match bind_with_fallback(preferred, allow_lan).await {
        Ok(listener) => listener,
        Err(error) => {
            let message = format!("监听 127.0.0.1:{preferred} 失败：{error}");
            eprintln!("[gateway] {message}");
            state.set_server_status(ServerStatus {
                running: false,
                addr: None,
                error: Some(message),
            });
            return;
        }
    };
    if let Ok(addr) = listener.local_addr() {
        if addr.port() != preferred {
            eprintln!("[gateway] 端口 {preferred} 被占用，已回退到 http://{addr}");
        }
    }
    serve(listener, state).await;
}

/// 在已绑定的监听器上提供服务：写服务状态 → 跑 axum。
///
/// 与 `run` 拆开是为了让测试能走同一条生产路径（绑定的端口可以由测试自己挑）。
pub async fn serve(listener: tokio::net::TcpListener, state: SharedGateway) {
    let addr = listener
        .local_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_else(|_| "127.0.0.1:?".to_string());
    eprintln!("[gateway] 已监听 http://{addr}");
    state.set_server_status(ServerStatus {
        running: true,
        addr: Some(format!("http://{addr}")),
        error: None,
    });

    if let Err(error) = axum::serve(listener, router(std::sync::Arc::clone(&state))).await {
        let message = format!("HTTP 服务异常退出：{error}");
        eprintln!("[gateway] {message}");
        state.set_server_status(ServerStatus {
            running: false,
            addr: None,
            error: Some(message),
        });
    }
}

async fn bind_with_fallback(port: u16, allow_lan: bool) -> Result<tokio::net::TcpListener, std::io::Error> {
    // 允许局域网访问时绑 0.0.0.0（IPv4 全网卡）；否则只绑回环。
    // 回环绑定的监听器拿不到 0.0.0.0 地址，所以分两条路径。
    let host = if allow_lan { "0.0.0.0" } else { "127.0.0.1" };
    let mut last_error = None;
    for candidate in port..port.saturating_add(PORT_SCAN_RANGE) {
        match tokio::net::TcpListener::bind((host, candidate)).await {
            Ok(listener) => return Ok(listener),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| std::io::Error::other("没有可用端口")))
}

/// 记录**没能产出响应**的请求（含"未进入调度"与"调度中途失败"两类）。
///
/// 这类失败以前不留任何痕迹，客户端只表现为反复重连，排查时无从下手。
/// 现在复用用量表记一行：面板的「请求日志」会带红色 `?`，点开就是原因原文。
///
/// **"未进入调度"这个名字只对一部分情况成立**：
/// · 鉴权失败、并发满、协议解码失败 —— 确实没进调度；
/// · 上游 5xx / 换号后仍失败 —— 已经打过上游，只是最终没拿到响应。
/// 早期两类都标成"未进入调度"，会让人误以为请求根本没发出去
///（可上游明明已经返回了 `995000 internal server error`）。
/// 现在按错误类型给前缀，一眼能看出是"本地拒"还是"上游挂了"。
pub(super) fn record_rejection(
    state: &SharedGateway,
    path: &str,
    error: &crate::gateway::errors::GatewayError,
) {
    state
        .storage
        .record_usage(crate::gateway::storage::UsageRecord {
            ts: now_ms(),
            // 前缀按错误来源区分：本地拒绝 vs 上游/调度失败。
            model: format!(
                "({} {path})",
                match error.kind {
                    crate::gateway::errors::ErrorKind::Upstream5xx
                    | crate::gateway::errors::ErrorKind::Network => "上游失败",
                    crate::gateway::errors::ErrorKind::NoAccount => "无可用账号",
                    crate::gateway::errors::ErrorKind::RateLimited => "限流或并发",
                    _ => "请求被拒",
                }
            ),
            channel_id: "-".into(),
            account_uid: "-".into(),
            status: "rejected".into(),
            stream: false,
            attempts: 0,
            usage: Default::default(),
            estimated: false,
            ttft_ms: None,
            total_ms: 0,
            effort: None,
            upstream_model: None,
            error_detail: Some(format!("[{:?}] {}", error.kind, error.describe())),
        });
}

async fn health(State(state): State<SharedGateway>) -> Response {
    let config = state.config();
    let now = now_ms();
    let channels: Vec<serde_json::Value> = state
        .channels()
        .iter()
        .map(|channel| {
            let accounts = state.pool.by_channel(channel.id());
            let ready = accounts
                .iter()
                .filter(|account| account.is_ready("", now))
                .count();
            json!({
                "id": channel.id(),
                "adapter": channel.adapter_kind(),
                "accounts": accounts.len(),
                "ready": ready,
            })
        })
        .collect();
    let status = state.server_status();
    let matrix = state.matrix_snapshot();
    let (live_routes, builtin_routes, route_count, matrix_age_ms) = match &matrix {
        Some(matrix) => {
            let (live, builtin) = matrix.sources();
            (
                live,
                builtin,
                matrix.route_count(),
                now_ms().saturating_sub(matrix.built_at_ms()),
            )
        }
        None => (0, 0, 0, 0),
    };

    Json(json!({
        "status": if status.running { "ok" } else { "stopped" },
        "addr": status.addr,
        "error": status.error,
        "uptime_ms": state.uptime_ms(),
        "auth": if config.api_keys.is_empty() { "open" } else { "api_key" },
        "concurrency": { "global": config.concurrency.global, "available": state.global_available() },
        "affinity": state.affinity.stats(),
        "matrix": {
            "routes": route_count,
            "live": live_routes,
            "builtin": builtin_routes,
            "age_ms": matrix_age_ms,
            "ttl_seconds": config.catalog_ttl_seconds,
        },
        "stats": {
            "requests": state.stats.requests.load(Ordering::Relaxed),
            "failures": state.stats.failures.load(Ordering::Relaxed),
            "retries": state.stats.retries.load(Ordering::Relaxed),
        },
        "channels": channels,
        "accounts": state.pool.views(),
        "warnings": state.warnings(),
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::{
        config::{AccountConfig, ChannelConfig, Config},
        state::GatewayState,
    };
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serde_json::{json, Value};
    use tower::ServiceExt;

    fn state_with(options: Value, accounts: usize) -> SharedGateway {
        let dir = tempfile::tempdir().expect("临时目录");
        let mut config = Config::default();
        // 测试只挂假渠道：默认配置里的 workbuddy 会带来真实快照目录。
        config.channels.clear();
        config.channels.push(ChannelConfig {
            id: "fake".into(),
            adapter: "fake".into(),
            options,
            ..ChannelConfig::default()
        });
        for index in 0..accounts {
            config.accounts.push(AccountConfig {
                channel_id: "fake".into(),
                uid: format!("u{index}"),
                max_inflight: Some(1),
                ..AccountConfig::default()
            });
        }
        GatewayState::initialize_with_secrets(
            config,
            dir.path().to_path_buf(),
            crate::gateway::security::tests_support::memory_store().as_ref(),
        )
        .expect("状态初始化")
    }

    async fn post(app: &Router, path: &str, body: Value) -> (StatusCode, Vec<u8>) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("请求构造"),
            )
            .await
            .expect("路由命中");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("读响应体");
        (status, bytes.to_vec())
    }

    /// **跨渠道切换**：同一个对外模型名，两个渠道各有一个账号。
    /// 首选渠道被上游限流后，下一跳必须落到另一个渠道 ——
    /// 这正是"wb 限额了自动切 trae"的核心诉求。
    #[tokio::test]
    async fn a_rate_limited_channel_falls_through_to_the_next_channel() {
        let dir = tempfile::tempdir().expect("临时目录");
        let mut config = Config::default();
        config.channels.clear();
        // 两个渠道提供**同一个**对外模型名（模拟 wb 与 trae 都能跑 ds）。
        for (id, fail_first, priority) in [("wb", 1u32, 100u32), ("trae", 0u32, 90u32)] {
            config.channels.push(ChannelConfig {
                id: id.into(),
                adapter: "fake".into(),
                options: json!({ "model": "ds", "fail_first": fail_first }),
                // wb 优先级更高：先选它，失败后才验证是否**跨档**降级到 trae。
                priority,
                ..ChannelConfig::default()
            });
            config.accounts.push(AccountConfig {
                channel_id: id.into(),
                uid: "u1".into(),
                max_inflight: Some(1),
                ..AccountConfig::default()
            });
        }
        let state = GatewayState::initialize_with_secrets(
            config,
            dir.path().to_path_buf(),
            crate::gateway::security::tests_support::memory_store().as_ref(),
        )
        .expect("状态初始化");
        // 关掉亲和：否则首次成功会把后续请求钉在同一账号上，掩盖换渠道行为。
        let app = router(std::sync::Arc::clone(&state));

        let (status, body) = post(
            &app,
            "/v1/chat/completions",
            json!({
                "model": "ds",
                "messages": [{"role": "user", "content": "hi"}]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "换渠道后应当成功");
        let text = String::from_utf8(body).expect("utf8");
        assert!(text.contains("ds"), "{text}");

        // 关键断言：wb 因为注入的限流而进入冷却，trae 接住了这次请求。
        let decisions = state.decisions();
        let last = decisions.last().expect("至少有一条决策");
        assert_eq!(last["attempts"], 2, "第一次失败后应当换号重试：{last}");
        assert_eq!(
            last["channel"], "trae",
            "首选渠道限流后必须降级到备用渠道：{last}"
        );
    }
    /// 模型级限额（workbuddy 的 6004 语义）：**只停这个模型**，同账号其他模型照跑。
    ///
    /// 这是"wb 限额了要能切走"的另一半：如果限额把整账号拉黑，
    /// 该账号的其他模型也会一起不可用，切走就没意义了。
    ///
    /// 注意：这里直接构造 `AccountModel` 作用域的错误，而不是靠假渠道注入 429 ——
    /// 通用 429 只有在正文能解析出恢复时刻时才收窄到模型级（保守策略），
    /// 而 6004 这类"渠道自己知道得更准"的情形在 classify_error 里直接给出作用域。
    #[test]
    fn a_model_level_limit_does_not_block_other_models_on_the_same_account() {
        use crate::gateway::errors::{ErrorKind, ErrorScope, GatewayError};

        let mut account = crate::gateway::core::pool::PoolAccount::new("wb-cn", "u1");
        // 测试不搭真实凭据：关掉凭据要求，只验证限额的作用域。
        account.requires_credential = false;
        let policy = crate::gateway::config::CooldownConfig::default();
        let now = crate::gateway::timeutil::now_ms();

        // 6004：模型级限流，带恢复时刻。
        let error = GatewayError::new(
            ErrorKind::RateLimited,
            ErrorScope::AccountModel("deepseek-v4.1-flash".into()),
            429,
            "该模型被上游限流",
            true,
        )
        .with_reset_at(Some(now + 600_000));
        account.apply_error(&error, &policy, now);

        assert!(
            !account.is_ready("deepseek-v4.1-flash", now),
            "被限流的模型应当暂时不可用"
        );
        assert!(
            account.is_ready("glm-5.3", now),
            "同账号其他模型必须仍然可用（否则一限额就得整号停摆）"
        );
        assert!(
            account.is_ready("kimi-k2.6", now),
            "多个无关模型都不该受影响"
        );
    }

    /// 账号级 429（上游没说限哪个模型）保守地停整个账号 —— 这是刻意的，
    /// 因为"不知道限什么"时继续用同账号打其他模型可能一直吃 429。
    #[test]
    fn an_account_level_limit_still_stops_the_whole_account() {
        use crate::gateway::errors::{ErrorKind, ErrorScope, GatewayError};

        let mut account = crate::gateway::core::pool::PoolAccount::new("wb-cn", "u1");
        account.requires_credential = false;
        let policy = crate::gateway::config::CooldownConfig::default();
        let now = crate::gateway::timeutil::now_ms();
        let error = GatewayError::new(
            ErrorKind::RateLimited,
            ErrorScope::Account,
            429,
            "账号级限流",
            true,
        )
        .with_reset_at(Some(now + 600_000));
        account.apply_error(&error, &policy, now);

        assert!(!account.is_ready("glm-5.3", now), "账号级限额要停整个账号");
    }

    #[tokio::test]
    async fn streaming_request_returns_openai_sse_frames() {
        let app = router(state_with(json!({}), 1));
        let (status, body) = post(
            &app,
            "/v1/chat/completions",
            json!({
                "model": "fake-model",
                "stream": true,
                "messages": [{"role": "user", "content": "你好"}]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let text = String::from_utf8(body).expect("UTF-8 流");
        assert!(text.contains("\"object\":\"chat.completion.chunk\""), "{text}");
        assert!(text.contains("\"content\":\"[fake-model] \""), "{text}");
        assert!(text.contains("你好"), "{text}");
        // 上游 usage 的缓存字段要按别名补齐给客户端。
        assert!(text.contains("prompt_cache_hit_tokens"), "{text}");
        assert!(text.trim_end().ends_with("data: [DONE]"), "{text}");
    }

    #[tokio::test]
    async fn non_streaming_request_folds_into_a_single_completion() {
        let app = router(state_with(json!({}), 1));
        let (status, body) = post(
            &app,
            "/v1/chat/completions",
            json!({
                "model": "fake-model",
                "messages": [{"role": "user", "content": "你好"}]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let value: Value = serde_json::from_slice(&body).expect("JSON 响应");
        assert_eq!(value["object"], "chat.completion");
        assert_eq!(value["choices"][0]["message"]["role"], "assistant");
        assert_eq!(value["choices"][0]["message"]["content"], "[fake-model] 你好");
        assert_eq!(value["choices"][0]["finish_reason"], "stop");
        assert_eq!(value["usage"]["cached_tokens"], 5);
    }

    #[tokio::test]
    async fn failed_account_is_swapped_for_the_next_one() {
        // 第一个请求注入 429：账号级软限流 → 换号重试 → 第二个账号成功。
        let app = router(state_with(json!({"fail_first": 1, "fail_status": 429}), 2));
        let (status, body) = post(
            &app,
            "/v1/chat/completions",
            json!({
                "model": "fake-model",
                "messages": [{"role": "user", "content": "hi"}]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let value: Value = serde_json::from_slice(&body).expect("JSON 响应");
        assert_eq!(value["choices"][0]["finish_reason"], "stop");
    }

    #[tokio::test]
    async fn missing_model_is_rejected_before_any_upstream_call() {
        let app = router(state_with(json!({}), 1));
        let (status, body) = post(
            &app,
            "/v1/chat/completions",
            json!({"messages": [{"role": "user", "content": "hi"}]}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let value: Value = serde_json::from_slice(&body).expect("JSON 错误信封");
        assert_eq!(value["error"]["type"], "invalid_request_error");
    }

    #[tokio::test]
    async fn empty_pool_reports_no_account_instead_of_hanging() {
        let app = router(state_with(json!({}), 0));
        let (status, _) = post(
            &app,
            "/v1/chat/completions",
            json!({"model": "fake-model", "messages": [{"role": "user", "content": "hi"}]}),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn models_endpoint_lists_the_channel_catalog() {
        let app = router(state_with(json!({}), 1));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/models")
                    .body(Body::empty())
                    .expect("请求构造"),
            )
            .await
            .expect("路由命中");
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("读响应体");
        let value: Value = serde_json::from_slice(&bytes).expect("JSON 响应");
        assert_eq!(value["object"], "list");
        assert_eq!(value["data"][0]["id"], "fake-model");
        assert_eq!(value["data"][0]["owned_by"], "fake");
    }

    /// 与 `state_with` 相同，但配置了下游 API Key。
    fn state_with_keys(api_keys: Vec<String>) -> SharedGateway {
        let dir = tempfile::tempdir().expect("临时目录");
        let mut config = Config::default();
        config.api_keys = api_keys;
        config.channels.push(ChannelConfig {
            id: "fake".into(),
            adapter: "fake".into(),
            ..ChannelConfig::default()
        });
        config.accounts.push(AccountConfig {
            channel_id: "fake".into(),
            uid: "u0".into(),
            ..AccountConfig::default()
        });
        GatewayState::initialize_with_secrets(
            config,
            dir.path().to_path_buf(),
            crate::gateway::security::tests_support::memory_store().as_ref(),
        )
        .expect("状态初始化")
    }

    async fn post_with_api_key(app: &Router, key: Option<&str>) -> StatusCode {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json");
        if let Some(key) = key {
            builder = builder.header("x-api-key", key);
        }
        let response = app
            .clone()
            .oneshot(
                builder
                    .body(Body::from(
                        json!({"model": "fake-model", "messages": [{"role": "user", "content": "hi"}]})
                            .to_string(),
                    ))
                    .expect("请求构造"),
            )
            .await
            .expect("路由命中");
        response.status()
    }

    /// 回归：真实默认配置（带 workbuddy 渠道）下 `/v1/models` 必须正常返回。
    ///
    /// 早前的单测全部清掉了 workbuddy（怕真实快照目录污染），唯独这条路径上
    /// `list_models` → `ensure_matrix` 会走 workbuddy 的目录构建 —— 真实环境里
    /// 一次对 `/v1/models` 的请求直接被断连（handler panic，hyper 关连接不回包），
    /// 而测试矩阵里只有 fake 渠道所以测不出来。
    #[tokio::test]
    async fn models_endpoint_survives_default_workbuddy_config() {
        let dir = tempfile::tempdir().expect("临时目录");
        let state = GatewayState::initialize_with_secrets(
            Config::default(),
            dir.path().to_path_buf(),
            crate::gateway::security::tests_support::memory_store().as_ref(),
        )
        .expect("状态初始化");
        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/models")
                    .body(Body::empty())
                    .expect("请求构造"),
            )
            .await
            .expect("路由命中（handler 若 panic 这里会收到 JoinError）");
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("读响应体");
        let value: Value = serde_json::from_slice(&bytes).expect("JSON 响应");
        assert_eq!(value["object"], "list");
        assert!(value["data"].is_array());
    }

    #[tokio::test]
    async fn api_key_is_required_and_checked_when_configured() {
        let app = router(state_with_keys(vec!["sk-test".into()]));
        assert_eq!(
            post_with_api_key(&app, None).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            post_with_api_key(&app, Some("sk-wrong")).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(post_with_api_key(&app, Some("sk-test")).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn empty_key_list_means_loopback_open() {
        let app = router(state_with_keys(Vec::new()));
        assert_eq!(post_with_api_key(&app, None).await, StatusCode::OK);
    }

    /// 真 socket 上的端到端：绑定 → 端口回退 → HTTP/SSE 过 TCP → `/health`。
    /// 这条覆盖 P0 的验收口径（等价于对 `127.0.0.1:<port>` 发 curl）。
    #[tokio::test]
    async fn serves_real_http_and_falls_back_when_the_port_is_taken() {
        let occupied = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("占住一个端口");
        let taken = occupied.local_addr().expect("本地地址").port();

        let listener = bind_with_fallback(taken, false).await.expect("回退绑定");
        let port = listener.local_addr().expect("本地地址").port();
        assert_ne!(port, taken, "首选端口被占用时必须顺序回退");

        let state = state_with(json!({}), 1);
        let server = tokio::spawn(serve(listener, state));

        let client = reqwest::Client::new();
        let response = client
            .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .json(&json!({
                "model": "fake-model",
                "stream": true,
                "messages": [{"role": "user", "content": "ping"}]
            }))
            .send()
            .await
            .expect("真实 HTTP 请求");
        assert_eq!(response.status(), 200);
        let body = response.text().await.expect("读 SSE 正文");
        assert!(body.contains("data: [DONE]"), "{body}");
        assert!(body.contains("ping"), "{body}");

        let health: Value = client
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
            .expect("健康探针")
            .json()
            .await
            .expect("JSON");
        assert_eq!(health["status"], "ok");
        assert_eq!(health["channels"][0]["id"], "fake");
        assert_eq!(health["channels"][0]["ready"], 1);
        assert_eq!(health["stats"]["requests"], 1);

        server.abort();
        drop(occupied);
    }
}

#[cfg(test)]
mod body_limit_tests {
    //! 端到端验证请求体限制确实被解除：发一个 >2MB（axum 旧默认）的请求体，
    //! 断言不会得到 413 / "length limit exceeded"。
    use super::*;
    use crate::gateway::{config::{AccountConfig, ChannelConfig, Config}, GatewayState};
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn large_request_body_is_not_rejected() {
        let dir = tempfile::tempdir().expect("临时目录");
        let mut config = Config::default();
        // 没有账号：请求会走到"无可用账号"503，但**不应该**在 body 层就被 413 拦掉。
        config.channels.push(ChannelConfig {
            id: "wb-cn".into(),
            adapter: "workbuddy".into(),
            ..ChannelConfig::default()
        });
        config.accounts.push(AccountConfig {
            channel_id: "wb-cn".into(),
            uid: "u1".into(),
            ..AccountConfig::default()
        });
        let state = GatewayState::initialize_with_secrets(
            config,
            dir.path().to_path_buf(),
            crate::gateway::security::tests_support::memory_store().as_ref(),
        )
        .expect("状态初始化");

        // 5MB 请求体：超过 axum 旧默认 2MB。
        let big = "x".repeat(5 * 1024 * 1024);
        let body = format!(
            r#"{{"model":"glm-5.3-flash","messages":[{{"role":"user","content":"{}"}}]}}"#,
            big
        );
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("请求构造"),
            )
            .await
            .expect("路由命中");
        let status = response.status().as_u16();
        // 413 = body 层限制还在（要修的是这个）；
        // 400/503 = body 已经完整进入了业务层（模型/账号层面被拒），这正是我们要的。
        assert_ne!(status, 413, "大请求体不该在网关层被 413");
        assert_ne!(status, 500, "也不该是内部错误");
    }
}
