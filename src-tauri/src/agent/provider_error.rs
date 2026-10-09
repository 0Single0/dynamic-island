use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderError {
    pub category: String,
    pub message: String,
    pub status: Option<u16>,
    pub protocol: String,
    pub model: String,
    pub endpoint: String,
    pub code: Option<String>,
    pub param: Option<String>,
    pub request_id: Option<String>,
    pub body: Option<String>,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(status) = self.status {
            write!(f, "HTTP {status}: ")?;
        }
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProviderError {}

pub fn redact(text: &str, secret: &str) -> String {
    let text = if secret.is_empty() {
        text.to_owned()
    } else {
        text.replace(secret, "[REDACTED]")
    };
    text.lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if lower.contains("authorization")
                || lower.contains("x-api-key")
                || lower.contains("bearer ")
            {
                "[REDACTED AUTHENTICATION]".to_owned()
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn safe_endpoint(endpoint: &str) -> String {
    match reqwest::Url::parse(endpoint) {
        Ok(mut url) => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        }
        Err(_) => "[invalid endpoint]".into(),
    }
}

impl ProviderError {
    pub fn local(
        category: &str,
        message: impl Into<String>,
        protocol: &str,
        model: &str,
        endpoint: &str,
    ) -> Self {
        Self {
            category: category.into(),
            message: message.into(),
            status: None,
            protocol: protocol.into(),
            model: model.into(),
            endpoint: safe_endpoint(endpoint),
            code: None,
            param: None,
            request_id: None,
            body: None,
        }
    }

    pub fn from_body(
        status: u16,
        body: &str,
        request_id: Option<String>,
        protocol: &str,
        model: &str,
        endpoint: &str,
        key: &str,
    ) -> Self {
        let safe = redact(body, key);
        let value: Value = serde_json::from_str(&safe).unwrap_or(Value::Null);
        let error = value.get("error").unwrap_or(&value);
        let string = |v: &Value| {
            v.as_str()
                .map(str::to_owned)
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        };
        let message = error["message"]
            .as_str()
            .or(error.as_str())
            .or(value["message"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if safe.trim().is_empty() {
                    "供应商未返回错误说明".into()
                } else {
                    safe.chars().take(1000).collect()
                }
            });
        Self {
            category: match status {
                400 | 422 => "INVALID_REQUEST",
                401 | 403 => "AUTH",
                429 => "RATE_LIMIT",
                500..=599 => "SERVER",
                _ => "HTTP",
            }
            .into(),
            message,
            status: Some(status),
            protocol: protocol.into(),
            model: model.into(),
            endpoint: safe_endpoint(endpoint),
            code: string(&error["code"]),
            param: string(&error["param"]),
            request_id: request_id
                .map(|s| redact(&s, key))
                .or_else(|| string(&value["request_id"])),
            body: Some(safe),
        }
    }

    pub async fn from_response(
        response: reqwest::Response,
        protocol: &str,
        model: &str,
        endpoint: &str,
        key: &str,
    ) -> Self {
        let status = response.status().as_u16();
        let request_id = ["x-request-id", "x-tt-logid", "request-id"]
            .iter()
            .find_map(|h| {
                response
                    .headers()
                    .get(*h)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned)
            });
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while bytes.len() < 16 * 1024 {
            match stream.next().await {
                Some(Ok(chunk)) => {
                    bytes.extend_from_slice(&chunk[..chunk.len().min(16 * 1024 - bytes.len())])
                }
                _ => break,
            }
        }
        Self::from_body(
            status,
            &String::from_utf8_lossy(&bytes),
            request_id,
            protocol,
            model,
            endpoint,
            key,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn errors_keep_actionable_fields_without_credentials() {
        for (status, category) in [
            (400, "INVALID_REQUEST"),
            (401, "AUTH"),
            (429, "RATE_LIMIT"),
            (500, "SERVER"),
        ] {
            let e = ProviderError::from_body(
                status,
                r#"{"error":{"message":"bad test-secret","code":"bad_name","param":"tools[0].name"}}"#,
                Some("req-1".into()),
                "chat-completions",
                "demo",
                "https://user:pass@example.com/v1?key=secret",
                "test-secret",
            );
            assert_eq!(e.category, category);
            assert_eq!(e.param.as_deref(), Some("tools[0].name"));
            assert!(!serde_json::to_string(&e).unwrap().contains("test-secret"));
            assert_eq!(e.endpoint, "https://example.com/v1");
        }
        assert_eq!(
            redact("Authorization: Bearer abc", ""),
            "[REDACTED AUTHENTICATION]"
        );
    }
}
