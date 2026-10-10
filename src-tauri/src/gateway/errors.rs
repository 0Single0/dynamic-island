//! 统一错误语义：渠道适配器把上游错误翻译成 [`ErrorKind`] + [`ErrorScope`]，
//! 调度器只认语义，不认具体渠道的状态码或错误码。
//!
//! 下游信封按 OpenAI 形状输出（`{"error":{"message","type","code"}}`）；
//! Anthropic / Responses 两条入口线在 P3 各自包一层信封。

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// 400/422：请求本身不合法，换号也没用。
    InvalidRequest,
    /// 401：凭据失效或过期。
    AuthExpired,
    /// 429：限流（带 reset 时刻 = 只冷该模型；不带 = 账号级软限流）。
    RateLimited,
    /// 402：余额/额度耗尽。
    QuotaExhausted,
    /// 403：内容审核。
    ContentPolicy,
    /// 上游 5xx。
    Upstream5xx,
    /// 连接/超时/断流。
    Network,
    /// 模型不存在或不在该渠道目录里。
    ModelNotFound,
    /// 本地判定：没有可用账号。
    NoAccount,
    /// 本地能力缺失（例如该渠道没有登录流程）。
    Unsupported,
    /// 网关自身缺陷。
    Internal,
}

impl ErrorKind {
    /// OpenAI 信封里的 `error.type`。
    pub fn openai_type(self) -> &'static str {
        match self {
            Self::InvalidRequest | Self::ModelNotFound | Self::Unsupported => "invalid_request_error",
            Self::AuthExpired => "authentication_error",
            Self::RateLimited | Self::NoAccount => "rate_limit_error",
            Self::QuotaExhausted => "insufficient_quota",
            Self::ContentPolicy => "content_policy_violation",
            Self::Upstream5xx | Self::Network | Self::Internal => "api_error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorScope {
    /// 只影响这一次请求。
    Request,
    /// 只冷这个模型（同账号其他模型照用）。
    AccountModel(String),
    /// 冷整个账号。
    Account,
}

#[derive(Debug, Clone)]
pub struct GatewayError {
    pub kind: ErrorKind,
    pub scope: ErrorScope,
    pub status: u16,
    pub message: String,
    /// 是否值得换号/重试；false 表示直接回给客户端。
    pub retryable: bool,
    /// 上游给出的恢复时刻（纪元毫秒），只有 429 一类会带。
    pub reset_at_ms: Option<u64>,
}

impl GatewayError {
    pub fn new(
        kind: ErrorKind,
        scope: ErrorScope,
        status: u16,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            kind,
            scope,
            status,
            message: message.into(),
            retryable,
            reset_at_ms: None,
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidRequest, ErrorScope::Request, 400, message, false)
    }

    /// 本地没有可用账号：可重试（等冷却结束或补账号），但当前请求没法继续。
    pub fn no_account(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NoAccount, ErrorScope::Request, 503, message, true)
    }

    pub fn overloaded() -> Self {
        Self::new(
            ErrorKind::RateLimited,
            ErrorScope::Request,
            503,
            "网关并发已满，请稍后重试",
            true,
        )
    }

    pub fn auth(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::AuthExpired, ErrorScope::Request, 401, message, false)
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Network, ErrorScope::Account, 502, message, true)
    }

    pub fn upstream(status: u16, message: impl Into<String>) -> Self {
        Self::new(
            ErrorKind::Upstream5xx,
            ErrorScope::Account,
            status,
            message,
            true,
        )
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, ErrorScope::Request, 500, message, false)
    }

    /// 本地不支持这个能力：明确告诉调用方，不要伪装成上游错误。
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(
            ErrorKind::Unsupported,
            ErrorScope::Request,
            501,
            message,
            false,
        )
    }

    pub fn from_reqwest(error: reqwest::Error) -> Self {
        let what = if error.is_timeout() {
            "上游请求超时"
        } else if error.is_connect() {
            "连接上游失败"
        } else {
            "上游网络异常"
        };
        Self::network(format!("{what}：{error}"))
    }

    /// Anthropic 的错误信封（`{"type":"error","error":{"type","message"}}`）。
    pub fn into_anthropic_response(self) -> Response {
        let kind = match self.status {
            400 | 422 => "invalid_request_error",
            401 => "authentication_error",
            403 => "permission_error",
            404 => "not_found_error",
            429 => "rate_limit_error",
            503 | 529 => "overloaded_error",
            _ => "api_error",
        };
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::BAD_GATEWAY);
        (
            status,
            Json(serde_json::json!({
                "type": "error",
                "error": { "type": kind, "message": self.message },
            })),
        )
            .into_response()
    }

    pub fn with_reset_at(mut self, reset_at_ms: Option<u64>) -> Self {
        self.reset_at_ms = reset_at_ms;
        self
    }
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for GatewayError {}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::BAD_GATEWAY);
        let body = Json(serde_json::json!({
            "error": {
                "message": self.message,
                "type": self.kind.openai_type(),
                "code": status.as_u16(),
            }
        }));
        (status, body).into_response()
    }
}

/// 定长比较，避免 Key 校验被时序侧信道区分出前缀长度。
pub fn secret_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_comparison_is_length_then_content() {
        assert!(secret_eq("sk-local", "sk-local"));
        assert!(!secret_eq("sk-local", "sk-locax"));
        assert!(!secret_eq("sk-local", "sk-local-longer"));
        assert!(!secret_eq("", "x"));
    }
}
