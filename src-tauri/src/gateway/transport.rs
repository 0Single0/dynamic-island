//! 上游传输：一次 HTTP 请求 + SSE 帧重组。
//!
//! 这一层不认识任何渠道，也不认识调度器：渠道适配器只提供报文，调度器只消费帧流。

use std::{pin::Pin, time::Duration};

use futures_util::{stream, Stream};

use crate::gateway::errors::GatewayError;

/// 上游帧流：每一项是一帧 SSE 的 `data` 负载（已去掉 `data:` 前缀与心跳）。
pub type FrameStream = Pin<Box<dyn Stream<Item = Result<String, GatewayError>> + Send>>;

/// 上游请求报文，由渠道适配器构建（纯数据，便于单测）。
pub struct UpstreamRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: serde_json::Value,
}

/// 上游失败：`status == 0` 表示没拿到 HTTP 状态码（连接/超时/断流）。
pub struct UpstreamFailure {
    pub status: u16,
    pub body: String,
}

/// 错误正文只留前 8KB：翻译错误码用不到更多，避免把大段 HTML 带进日志。
const ERROR_BODY_LIMIT: usize = 8 * 1024;

/// 一次性 JSON 请求（授权 / 刷新 / 目录这类非流式接口）。
pub async fn post_json(
    client: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
    body: &serde_json::Value,
) -> Result<serde_json::Value, GatewayError> {
    let mut builder = client.post(url);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    let response = builder.json(body).send().await.map_err(GatewayError::from_reqwest)?;
    decode_json_response(response).await
}

pub async fn get_json(
    client: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
) -> Result<serde_json::Value, GatewayError> {
    let mut builder = client.get(url);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    let response = builder.send().await.map_err(GatewayError::from_reqwest)?;
    decode_json_response(response).await
}

async fn decode_json_response(response: reqwest::Response) -> Result<serde_json::Value, GatewayError> {
    let status = response.status();
    let mut text = response.text().await.unwrap_or_default();
    if text.len() > ERROR_BODY_LIMIT {
        text.truncate(ERROR_BODY_LIMIT);
    }
    if !status.is_success() {
        return Err(GatewayError::upstream(status.as_u16(), text));
    }
    serde_json::from_str(&text)
        .map_err(|error| GatewayError::network(format!("上游返回的不是合法 JSON：{error}")))
}

pub async fn send(
    upstream: &UpstreamRequest,
    client: &reqwest::Client,
) -> Result<reqwest::Response, UpstreamFailure> {
    let mut builder = client.post(&upstream.url);
    for (name, value) in &upstream.headers {
        builder = builder.header(name, value);
    }
    let response = builder
        .json(&upstream.body)
        .send()
        .await
        .map_err(|error| UpstreamFailure {
            status: 0,
            body: error.to_string(),
        })?;

    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let mut body = response.text().await.unwrap_or_default();
    if body.len() > ERROR_BODY_LIMIT {
        body.truncate(ERROR_BODY_LIMIT);
    }
    Err(UpstreamFailure {
        status: status.as_u16(),
        body,
    })
}

struct FrameState {
    response: reqwest::Response,
    buffer: Vec<u8>,
    done: bool,
}

/// 把上游响应体切成 SSE 帧流。
///
/// 两个细节：① 等分隔符到位再解 UTF-8，中文被切在 chunk 边界上也不会乱码
/// （复用 agent 侧的 `pop_sse_frame`）；② 每帧读取都有空闲超时，避免半死连接占着账号槽。
pub fn frame_stream(response: reqwest::Response, idle_timeout: Duration) -> FrameStream {
    let state = FrameState {
        response,
        buffer: Vec::new(),
        done: false,
    };
    Box::pin(stream::unfold(state, move |mut state| async move {
        loop {
            match crate::agent::providers::pop_sse_frame(&mut state.buffer) {
                Ok(Some(frame)) => {
                    if let Some(payload) = sse_payload(&frame) {
                        if payload != "[DONE]" {
                            return Some((Ok(payload), state));
                        }
                    }
                    continue;
                }
                Ok(None) => {}
                Err(message) => return Some((Err(GatewayError::network(message)), state)),
            }
            if state.done {
                // 收尾：缓冲区里剩的是不完整的一帧，丢弃（上游异常断流）。
                return None;
            }
            match tokio::time::timeout(idle_timeout, state.response.chunk()).await {
                Err(_) => {
                    return Some((
                        Err(GatewayError::network("上游流空闲超时（300 秒无数据）")),
                        state,
                    ))
                }
                Ok(Ok(Some(chunk))) => state.buffer.extend_from_slice(&chunk),
                Ok(Ok(None)) => state.done = true,
                Ok(Err(error)) => {
                    state.done = true;
                    return Some((Err(GatewayError::from_reqwest(error)), state));
                }
            }
        }
    }))
}

/// 从一帧 SSE 里取 `data` 负载（多行 data 用换行拼接）。心跳与注释行返回 None。
fn sse_payload(frame: &str) -> Option<String> {
    let mut lines = Vec::new();
    for line in frame.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("data:") {
            lines.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_joins_multiline_data_and_skips_heartbeat() {
        assert_eq!(sse_payload(": ping\n\n"), None);
        assert_eq!(
            sse_payload("data: one\ndata: two\r\n\r\n").as_deref(),
            Some("one\ntwo")
        );
        assert_eq!(sse_payload("data: [DONE]\n\n").as_deref(), Some("[DONE]"));
    }
}
