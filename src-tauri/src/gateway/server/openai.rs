//! OpenAI 兼容入口：Chat Completions（含旧版 completions）与模型清单。
//!
//! 入口只做四件事：鉴权、协议 ↔ CIR、把事件编码回 SSE/JSON、记一条用量。
//! 选号、冷却、重试、凭据刷新都在 `core` 里，入口不感知渠道。

use std::{sync::Arc, time::Instant};

use axum::{
    body::Bytes,
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::{mpsc, OwnedSemaphorePermit};
use uuid::Uuid;

use crate::gateway::{
    core::{
        dispatch::{self, UpstreamSession},
        usage as estimate,
    },
    errors::{secret_eq, GatewayError},
    protocol::{
        cir::{CanonicalEvent, Usage},
        decode, encode,
    },
    server::sse,
    state::SharedGateway,
    storage::UsageRecord,
    timeutil::now_ms,
};

pub async fn chat_completions(
    State(state): State<SharedGateway>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let recorder = Arc::clone(&state);
    match run_chat(state, headers, body).await {
        Ok(response) => response,
        Err(error) => {
            crate::gateway::server::record_rejection(&recorder, "/v1/chat/completions", &error);
            error.into_response()
        }
    }
}

async fn run_chat(
    state: SharedGateway,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, GatewayError> {
    authorize(&state, &headers)?;

    // 全局并发闸门：满载直接 503（排队逻辑在 P1 补）。permit 活到流结束。
    let permit = Arc::clone(&state.slots)
        .try_acquire_owned()
        .map_err(|_| GatewayError::overloaded())?;

    let body: Value = serde_json::from_slice(&body)
        .map_err(|error| GatewayError::invalid_request(format!("请求体不是合法 JSON：{error}")))?;
    let mut request = decode::openai::decode_chat(&body)?;
    request.metadata.header_session = header_session(&headers);
    let scope = api_key_scope(&state, &headers);

    let streaming = request.stream;
    let model = request.model.clone();
    let effort = request
        .reasoning
        .as_ref()
        .and_then(|reasoning| reasoning.effort.clone());
    let prompt_tokens = estimate::estimate_request_tokens(&request);
    let started = Instant::now();

    let session = match dispatch::open_chat_session(&state, &request, &scope).await {
        Ok(session) => session,
        Err(error) => {
            state
                .stats
                .failures
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Err(error);
        }
    };
    state
        .stats
        .requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let finish = CompletionMeta {
        model,
        effort,
        prompt_tokens,
        stream: streaming,
        started,
    };
    if streaming {
        Ok(stream_response(
            Arc::clone(&state),
            session,
            finish,
            permit,
        ))
    } else {
        let result = collect_completion(Arc::clone(&state), session, &finish).await;
        drop(permit);
        result.map(|completion| Json(completion).into_response())
    }
}

pub async fn list_models(
    State(state): State<SharedGateway>,
    headers: HeaderMap,
) -> Result<Response, GatewayError> {
    authorize(&state, &headers)?;
    // 清单来自矩阵：它已经把各渠道目录合并、清洗、按对外顺序排好。
    let matrix = state.ensure_matrix(false).await;
    let created = now_ms() / 1000;
    let data: Vec<Value> = matrix
        .public_models()
        .into_iter()
        .map(|id| {
            let routes = matrix.routes(&id);
            json!({
                "id": id,
                "object": "model",
                "created": created,
                "owned_by": routes
                    .first()
                    .map(|route| route.channel_id.clone())
                    .unwrap_or_default(),
                // 额外字段：这条模型能走哪些渠道（客户端会忽略，面板/脚本有用）。
                "channels": routes
                    .iter()
                    .map(|route| route.channel_id.clone())
                    .collect::<Vec<_>>(),
                "free": routes.iter().any(|route| route.is_free()),
            })
        })
        .collect();
    Ok(Json(json!({ "object": "list", "data": data })).into_response())
}

/// 一次请求里与用量记账有关的固定信息。
pub(super) struct CompletionMeta {
    pub(super) model: String,
    pub(super) effort: Option<String>,
    /// 请求的输入 token 估算值（上游不给 usage 时用）。
    pub(super) prompt_tokens: u64,
    pub(super) stream: bool,
    pub(super) started: Instant,
}

/// 流式：把上游帧流按渠道解析成 CIR 事件，再编码成 OpenAI SSE，结束时记一条用量。
fn stream_response(
    state: SharedGateway,
    session: UpstreamSession,
    meta: CompletionMeta,
    permit: OwnedSemaphorePermit,
) -> Response {
    let (sender, receiver) = mpsc::channel::<Bytes>(64);
    let id = format!("chatcmpl-{}", Uuid::new_v4().simple());
    let created = now_ms() / 1000;

    tokio::spawn(async move {
        // permit 与 session 活到任务结束：客户端断连导致 send 失败时一起回收。
        let _permit = permit;
        let mut session = session;
        let mut usage = Usage::default();
        let mut reported_usage = false;
        let mut ttft_ms: Option<u64> = None;
        let mut produced = String::new();
        let mut status = "completed";
        let mut error_detail: Option<String> = None;

        'stream: loop {
            let next = session.frames.next().await;
            let Some(frame) = next else { break };
            let events = match frame {
                Ok(frame) => session.adapter.parse_event(&frame),
                Err(error) => {
                    status = "upstream_aborted";
                    error_detail.get_or_insert_with(|| error.message.clone());
                    vec![CanonicalEvent::Error {
                        message: error.message,
                    }]
                }
            };
            let mut stop = false;
            for event in &events {
                match event {
                    CanonicalEvent::Error { message } => {
                        stop = true;
                        error_detail.get_or_insert_with(|| message.clone());
                    }
                    CanonicalEvent::Text { text } | CanonicalEvent::Reasoning { text } => {
                        if ttft_ms.is_none() {
                            ttft_ms = Some(meta.started.elapsed().as_millis() as u64);
                        }
                        produced.push_str(text);
                    }
                    CanonicalEvent::Usage(value) => {
                        usage = value.clone();
                        reported_usage = true;
                    }
                    _ => {}
                }
                if let Some(chunk) = encode::openai::chunk(event, &id, &meta.model, created) {
                    if sender.send(encode::openai::sse_data(&chunk)).await.is_err() {
                        status = "client_aborted";
                        break 'stream;
                    }
                }
            }
            if stop {
                break;
            }
        }
        let _ = sender.send(encode::openai::sse_done()).await;

        record_usage(
            &state,
            &session,
            &meta,
            status,
            reported_usage,
            usage,
            &produced,
            ttft_ms,
            error_detail,
        );
    });

    sse::sse_response(receiver)
}

/// 非流式：把整条事件流折叠成一个 `chat.completion`。
async fn collect_completion(
    state: SharedGateway,
    mut session: UpstreamSession,
    meta: &CompletionMeta,
) -> Result<Value, GatewayError> {
    let id = format!("chatcmpl-{}", Uuid::new_v4().simple());
    let created = now_ms() / 1000;
    let mut events = Vec::new();
    let mut ttft_ms: Option<u64> = None;
    loop {
        let next = session.frames.next().await;
        let Some(frame) = next else { break };
        let batch = match frame {
            Ok(frame) => session.adapter.parse_event(&frame),
            Err(error) => vec![CanonicalEvent::Error {
                message: error.message,
            }],
        };
        for event in &batch {
            if ttft_ms.is_none()
                && matches!(
                    event,
                    CanonicalEvent::Text { .. } | CanonicalEvent::Reasoning { .. }
                )
            {
                ttft_ms = Some(meta.started.elapsed().as_millis() as u64);
            }
        }
        events.extend(batch);
    }

    let completion = encode::openai::fold(&events, &id, &meta.model, created);
    let error_detail = completion.as_ref().err().map(|error| error.message.clone());
    let (status, usage, reported, produced) = match &completion {
        Ok(value) => {
            let text = value["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let (usage, reported) = usage_from_events(&events);
            ("completed", usage, reported, text)
        }
        Err(_) => {
            let (usage, reported) = usage_from_events(&events);
            ("failed", usage, reported, String::new())
        }
    };
    record_usage(
        &state, &session, meta, status, reported, usage, &produced, ttft_ms, error_detail,
    );
    completion
}

fn usage_from_events(events: &[CanonicalEvent]) -> (Usage, bool) {
    for event in events.iter().rev() {
        if let CanonicalEvent::Usage(usage) = event {
            return (usage.clone(), true);
        }
    }
    (Usage::default(), false)
}

/// 记一条用量：拿到上游 usage 就用它，拿不到就本地估算并标记。
pub(super) fn record_usage(
    state: &SharedGateway,
    session: &UpstreamSession,
    meta: &CompletionMeta,
    status: &str,
    reported_usage: bool,
    usage: Usage,
    produced: &str,
    ttft_ms: Option<u64>,
    error_detail: Option<String>,
) {
    let usage = if reported_usage {
        usage
    } else {
        estimate::estimate_completion(meta.prompt_tokens, produced)
    };
    state.storage.record_usage(UsageRecord {
        ts: now_ms(),
        model: meta.model.clone(),
        channel_id: session.account().key.channel_id.clone(),
        account_uid: session.account().key.uid.clone(),
        status: status.to_string(),
        stream: meta.stream,
        attempts: session.attempts,
        usage,
        estimated: !reported_usage,
        ttft_ms,
        total_ms: meta.started.elapsed().as_millis() as u64,
        effort: meta.effort.clone(),
        upstream_model: Some(session.selection.route.upstream_model.clone()),
        error_detail,
    });
    eprintln!(
        "[gateway] trace={} done account={} upstream={} via_affinity={} affinity_key={:?} attempts={} status={status} ttft={:?}ms estimated={}",
        session.trace_id,
        session.account().key,
        session.selection.route.upstream_model,
        session.selection.via_affinity,
        session.affinity_key,
        session.attempts,
        ttft_ms,
        !reported_usage
    );
}

/// 下游鉴权：`Authorization: Bearer` 或 `x-api-key`。
///
/// 未配置任何 Key 时放行（只绑回环，等价于"本机免鉴权"）；配置了就必须带对。
pub(super) fn authorize(state: &SharedGateway, headers: &HeaderMap) -> Result<(), GatewayError> {
    let keys = &state.config().api_keys;
    if keys.is_empty() {
        return Ok(());
    }
    match provided_key(headers) {
        Some(key) if keys.iter().any(|candidate| secret_eq(candidate, &key)) => Ok(()),
        _ => Err(GatewayError::auth("无效的 API Key")),
    }
}

/// 亲和作用域：不同调用方的绑定互不可见。
///
/// 用 Key 的短哈希而不是 Key 本身，避免把密钥写进内存键与决策日志。
pub(super) fn api_key_scope(state: &SharedGateway, headers: &HeaderMap) -> String {
    if state.config().api_keys.is_empty() {
        return "open".to_string();
    }
    match provided_key(headers) {
        Some(key) => crate::gateway::security::short_hash(&key),
        None => "anon".to_string(),
    }
}

fn provided_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
        })
        .map(str::to_string)
        .or_else(|| {
            headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        })
}

/// 会话线索：不同客户端习惯不同，三个常见头都收（亲和键的第一级来源）。
pub(super) fn header_session(headers: &HeaderMap) -> Option<String> {
    ["x-conversation-id", "session-id", "x-session-id"]
        .iter()
        .find_map(|name| headers.get(*name))
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
