//! 面板鉴权：环境变量账号密码 + HMAC 签名会话 Cookie。
//!
//! 为什么不用服务端会话表：单容器、单进程，签名 Cookie 免状态，
//! 重启后只要 `PANEL_SECRET` 不变，已登录的浏览器不用重新登录。
//!
//! 安全口径：
//! - 密码比较走**定长比较**（`errors::secret_eq`），不泄露前缀长度；
//! - Cookie 是 `HttpOnly` + `SameSite=Lax`，JS 读不到，跨站表单也带不上；
//! - 未配 `PANEL_PASSWORD` 时**拒绝启动面板**（除非显式 `PANEL_ALLOW_OPEN=1`），
//!   避免"部署上去发现管理面裸奔"。

use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::gateway::errors::secret_eq;

use super::PanelCtx;

pub const SESSION_COOKIE: &str = "gw_session";
/// 会话有效期：12 小时（够一个工作日，又不会长期有效）。
const SESSION_TTL_SECS: i64 = 12 * 3600;

/// 面板鉴权配置（启动时从环境变量读一次）。
#[derive(Clone)]
pub struct AuthConfig {
    pub username: String,
    pub password: String,
    secret: Arc<Vec<u8>>,
    /// 未设密码且显式允许开放（仅本地调试）。
    pub open: bool,
}

impl AuthConfig {
    /// 从环境变量装配。缺密码时返回 `Err`（除非 `PANEL_ALLOW_OPEN=1`）。
    pub fn from_env() -> Result<Self, String> {
        let username = std::env::var("PANEL_USERNAME")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "admin".to_string());
        let password = std::env::var("PANEL_PASSWORD")
            .ok()
            .unwrap_or_default();
        let allow_open = std::env::var("PANEL_ALLOW_OPEN")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        if password.is_empty() && !allow_open {
            return Err(
                "面板密码未配置：请设置 PANEL_PASSWORD 环境变量（本地调试可设 PANEL_ALLOW_OPEN=1 显式开放）"
                    .to_string(),
            );
        }

        let secret = match std::env::var("PANEL_SECRET") {
            Ok(value) if !value.trim().is_empty() => value.trim().as_bytes().to_vec(),
            _ => {
                // 没给盐就随机生成：重启后旧会话失效，但不会用弱默认值。
                let mut bytes = [0u8; 32];
                getrandom::fill(&mut bytes)
                    .map_err(|error| format!("生成会话盐失败：{error}"))?;
                eprintln!("[panel] 未设置 PANEL_SECRET，本次启动随机生成（重启后需重新登录）");
                bytes.to_vec()
            }
        };

        let open = password.is_empty() && allow_open;
        Ok(Self {
            username,
            password,
            secret: Arc::new(secret),
            open,
        })
    }

    /// 测试用：固定账号密码与会话盐，避免读环境变量。
    #[cfg(test)]
    pub fn for_tests() -> Self {
        Self {
            username: "admin".into(),
            password: "s3cret".into(),
            secret: Arc::new(b"unit-test-secret".to_vec()),
            open: false,
        }
    }

    /// 校验账号密码。
    pub fn verify(&self, username: &str, password: &str) -> bool {        if self.open {
            return true;
        }
        secret_eq(username, &self.username) && secret_eq(password, &self.password)
    }

    /// 签发会话 Cookie 值：`expiry.hmac`。
    fn issue(&self) -> String {
        let expiry = crate::gateway::timeutil::now_ms() / 1000 + SESSION_TTL_SECS as u64;
        let signature = self.sign(expiry);
        format!("{expiry}.{signature}")
    }

    fn sign(&self, expiry: u64) -> String {
        let mut hasher = Sha256::new();
        hasher.update(&*self.secret);
        hasher.update(b"|session|");
        hasher.update(expiry.to_string().as_bytes());
        let digest = hasher.finalize();
        let mut hex = String::with_capacity(64);
        for byte in digest {
            hex.push_str(&format!("{byte:02x}"));
        }
        hex
    }

    /// 校验 Cookie 值：签名对得上且没过期。
    pub fn valid_session(&self, cookie: &str) -> bool {
        if self.open {
            return true;
        }
        let Some((expiry_text, signature)) = cookie.split_once('.') else {
            return false;
        };
        let Ok(expiry) = expiry_text.parse::<u64>() else {
            return false;
        };
        if expiry <= crate::gateway::timeutil::now_ms() / 1000 {
            return false;
        }
        secret_eq(signature, &self.sign(expiry))
    }
}

/// 从 `Cookie` 头里取会话值。
pub fn session_from_headers(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix(&format!("{SESSION_COOKIE}=")) {
            return Some(value.to_string());
        }
    }
    None
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    username: String,
    password: String,
}

/// `POST /api/auth/login`
pub async fn login(
    State(ctx): State<PanelCtx>,
    Json(body): Json<LoginRequest>,
) -> Response {
    let auth = &ctx.auth;
    if !auth.verify(&body.username, &body.password) {
        // 失败信息不区分"用户不存在"与"密码错误"。
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "code": 1, "data": null, "msg": "账号或密码不正确" })),
        )
            .into_response();
    }
    let cookie = format!(
        "{SESSION_COOKIE}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_TTL_SECS}",
        auth.issue()
    );
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(json!({ "code": 0, "data": { "username": auth.username }, "msg": "" })),
    )
        .into_response()
}

/// `POST /api/auth/logout`
pub async fn logout() -> Response {
    let cookie = format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(json!({ "code": 0, "data": null, "msg": "" })),
    )
        .into_response()
}

/// `GET /api/auth/session`：面板启动时探一次登录态。
pub async fn session(
    State(ctx): State<PanelCtx>,
    headers: HeaderMap,
) -> Response {
    let auth = &ctx.auth;
    let ok = session_from_headers(&headers)
        .map(|cookie| auth.valid_session(&cookie))
        .unwrap_or(auth.open);
    if ok {
        (
            StatusCode::OK,
            Json(json!({ "code": 0, "data": { "username": auth.username }, "msg": "" })),
        )
            .into_response()
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "code": 1, "data": null, "msg": "未登录" })),
        )
            .into_response()
    }
}

/// 面板 API 的统一鉴权结果：未登录时返回 401 响应。
pub fn require_session(auth: &AuthConfig, headers: &HeaderMap) -> Option<Response> {
    let ok = session_from_headers(headers)
        .map(|cookie| auth.valid_session(&cookie))
        .unwrap_or(false);
    if ok {
        None
    } else {
        Some(
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "code": 1, "data": null, "msg": "未登录或会话已过期" })),
            )
                .into_response(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth() -> AuthConfig {
        AuthConfig {
            username: "admin".into(),
            password: "s3cret".into(),
            secret: Arc::new(b"unit-test-secret".to_vec()),
            open: false,
        }
    }

    #[test]
    fn wrong_password_is_rejected() {
        let auth = auth();
        assert!(auth.verify("admin", "s3cret"));
        assert!(!auth.verify("admin", "wrong"));
        assert!(!auth.verify("root", "s3cret"));
    }

    #[test]
    fn issued_session_verifies_and_tampering_fails() {
        let auth = auth();
        let cookie = auth.issue();
        assert!(auth.valid_session(&cookie));
        assert!(!auth.valid_session("not-a-session"));
        // 改签名
        let mut tampered = cookie.clone();
        tampered.push('a');
        assert!(!auth.valid_session(&tampered));
        // 改过期时间但保留原签名
        let (_, signature) = cookie.split_once('.').expect("格式");
        assert!(!auth.valid_session(&format!("9999999999.{signature}")));
    }

    #[test]
    fn session_is_scoped_to_the_secret() {
        let first = auth();
        let second = AuthConfig {
            secret: Arc::new(b"another-secret".to_vec()),
            ..auth()
        };
        assert!(!second.valid_session(&first.issue()));
    }

    #[test]
    fn cookie_header_is_parsed_among_other_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "other=1; gw_session=abc.def; tail=2".parse().expect("头"),
        );
        assert_eq!(session_from_headers(&headers).as_deref(), Some("abc.def"));
    }
}
