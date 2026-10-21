//! 华为云 `SDK-HMAC-SHA256` 请求签名（AK/SK + 可选 STS security token）。
//!
//! 为什么必须自己实现：CodeArts 的每个请求都要带签名头，签名覆盖**全部请求头**
//! （按小写排序后进入 canonical request）。少算一个头、或多算一个头，上游都只回
//! 一个笼统的 401/403，看不出是哪里错了 —— 所以这一层必须逐字节对齐官方 SDK。
//!
//! 对齐 `@huaweicloud/huaweicloud-sdk-core` 的 AKSKSigner：
//! ① signed headers = 请求里**全部**头部（小写排序），含 `x-sdk-content-sha256`；
//! ② CanonicalURI 每段 percent-encode，且**末尾补 `/`**；
//! ③ payload hash 直接取 `X-Sdk-Content-Sha256` 头的值；
//! ④ `Authorization: SDK-HMAC-SHA256 Access=…, SignedHeaders=…, Signature=…`。
//!
//! 一个容易踩的顺序问题：`maas_type: benefit` 这类业务头**必须在签名前**写进去，
//! 否则它会出现在请求里却不在 SignedHeaders 中，签名直接失效。

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

pub const SIGNING_ALGORITHM: &str = "SDK-HMAC-SHA256";

/// 一次请求的签名材料。`security_token` 是 STS 临时凭据的会话 token。
#[derive(Debug, Clone, Default)]
pub struct SignCredentials {
    pub access_key_id: String,
    pub secret_key: String,
    /// STS security token；长期 AK/SK 时为空。
    pub security_token: Option<String>,
}

impl SignCredentials {
    /// 三个字段齐全才算可用（缺 SK 时签名必然失败，不如提前判定）。
    pub fn is_usable(&self) -> bool {
        !self.access_key_id.is_empty() && !self.secret_key.is_empty()
    }
}

/// 签名结果：要追加到请求上的头。
#[derive(Debug, Clone, PartialEq)]
pub struct SignedHeaders {
    /// `X-Sdk-Date` 的值（`20261020T031500Z`）。
    pub x_sdk_date: String,
    /// `X-Sdk-Content-Sha256`（payload 的十六进制摘要）。
    pub content_sha256: String,
    /// `X-Security-Token`（有 STS token 时才有）。
    pub security_token: Option<String>,
    /// 完整的 `Authorization` 头。
    pub authorization: String,
}

/// 对一次请求签名。
///
/// `headers` 是**将要实际发送的全部头**（不含签名自身产生的四个头 ——
/// `X-Sdk-Date` / `X-Security-Token` / `X-Sdk-Content-Sha256` / `Authorization`
/// 由本函数生成并纳入签名范围）。
///
/// `x_sdk_date` 由调用方给（便于单测固定时间）；生产用 [`now_sdk_date`]。
pub fn sign(
    method: &str,
    url: &url_parts::UrlParts,
    headers: &[(String, String)],
    body: &[u8],
    credentials: &SignCredentials,
    x_sdk_date: &str,
) -> SignedHeaders {
    let payload_hash = sha256_hex(body);

    // 1) 组装「参与签名的全部头」：业务头 + 三个签名头（Authorization 不算）。
    let mut all: Vec<(String, String)> = headers
        .iter()
        .filter(|(name, _)| {
            // 签名自身产生的头后面统一加，避免重复/顺序问题。
            let lower = name.to_ascii_lowercase();
            lower != "authorization"
                && lower != "x-sdk-date"
                && lower != "x-sdk-content-sha256"
                && lower != "x-security-token"
        })
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
        .collect();

    // `host` 必须参与签名；调用方没给就用 URL 里的。
    if !all.iter().any(|(name, _)| name == "host") {
        all.push(("host".to_string(), url.host.clone()));
    }
    all.push(("x-sdk-date".to_string(), x_sdk_date.to_string()));
    all.push(("x-sdk-content-sha256".to_string(), payload_hash.clone()));
    if let Some(token) = credentials.security_token.as_deref().filter(|t| !t.is_empty()) {
        all.push(("x-security-token".to_string(), token.to_string()));
    }

    // 同名头合并成一个（官方 SDK 也是这个行为），再按 key 排序。
    all.sort_by(|a, b| a.0.cmp(&b.0));
    let mut merged: Vec<(String, String)> = Vec::with_capacity(all.len());
    for (name, value) in all {
        match merged.last_mut() {
            Some((last_name, last_value)) if *last_name == name => {
                last_value.push(',');
                last_value.push_str(&value);
            }
            _ => merged.push((name, value)),
        }
    }

    // 2) CanonicalRequest。
    let canonical_headers: String = merged
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect();
    let signed_headers: String = merged
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonical_request = [
        method.to_string(),
        canonical_uri(&url.path),
        canonical_query(&url.query),
        canonical_headers,
        signed_headers.clone(),
        payload_hash.clone(),
    ]
    .join("\n");

    // 3) StringToSign → HMAC。这里**只做一层 HMAC**（不是 AWS 的四步派生）。
    let string_to_sign = [
        SIGNING_ALGORITHM.to_string(),
        x_sdk_date.to_string(),
        sha256_hex(canonical_request.as_bytes()),
    ]
    .join("\n");
    let signature = hmac_sha256_hex(&credentials.secret_key, &string_to_sign);

    let authorization = format!(
        "{SIGNING_ALGORITHM} Access={}, SignedHeaders={}, Signature={}",
        credentials.access_key_id, signed_headers, signature
    );

    SignedHeaders {
        x_sdk_date: x_sdk_date.to_string(),
        content_sha256: payload_hash,
        security_token: credentials
            .security_token
            .clone()
            .filter(|token| !token.is_empty()),
        authorization,
    }
}

/// 当前 UTC 时间的 SDK 日期格式：`20261020T031500Z`。
pub fn now_sdk_date() -> String {
    let now_ms = crate::gateway::timeutil::now_ms();
    format_sdk_date(now_ms)
}

/// 纪元毫秒 → `YYYYMMDDTHHMMSSZ`。
pub fn format_sdk_date(epoch_ms: u64) -> String {
    let total_secs = epoch_ms / 1000;
    let days = (total_secs / 86_400) as i64;
    let secs_of_day = total_secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z")
}

/// Howard Hinnant 的 `civil_from_days`：1970-01-01 起的天数 → 公历。
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// CanonicalURI：逐段 percent-encode，末尾补 `/`。
///
/// `path` 是**未解码**的路径（`/api/v2/chat/completions`）。
pub fn canonical_uri(path: &str) -> String {
    let encoded: Vec<String> = path
        .split('/')
        .map(percent_encode_segment)
        .collect();
    let mut out = encoded.join("/");
    if !out.ends_with('/') {
        out.push('/');
    }
    out
}

/// CanonicalQuery：按 key 排序，同名多值各自排序后拼接。
pub fn canonical_query(query: &str) -> String {
    if query.is_empty() {
        return String::new();
    }
    let mut pairs: Vec<(String, String)> = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((key, value)) => (decode_uri(key), decode_uri(value)),
            None => (decode_uri(pair), String::new()),
        })
        .collect();
    pairs.sort();
    pairs
        .into_iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                percent_encode_segment(&key),
                percent_encode_segment(&value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// 单段 percent-encode：保留 RFC 3986 unreserved，其余按字节转义。
///
/// 不复用 `url::form_urlencoded`：那个把空格编码成 `+`，
/// 而华为云 SDK 用的是 `%20`，差一个字符签名就全错。
fn percent_encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn decode_uri(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes.as_ref());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hmac_sha256_hex(key: &str, message: &str) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key.as_bytes())
        .expect("HMAC 接受任意长度的密钥");
    mac.update(message.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 拆分 URL 成签名需要的三段（不引入 `url` crate）。
pub mod url_parts {
    #[derive(Debug, Clone, Default)]
    pub struct UrlParts {
        /// `snap-access.cn-north-4.myhuaweicloud.com[:port]`（参与签名的 host）。
        pub host: String,
        /// 未解码的 path。
        pub path: String,
        /// 未解码、未排序的 query（不含 `?`）。
        pub query: String,
    }

    impl UrlParts {
        pub fn parse(url: &str) -> Self {
            let without_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
            let (authority, rest) = match without_scheme.find('/') {
                Some(index) => (&without_scheme[..index], &without_scheme[index..]),
                None => (without_scheme, "/"),
            };
            let (path, query) = match rest.split_once('?') {
                Some((path, query)) => (path, query),
                None => (rest, ""),
            };
            Self {
                host: authority.to_string(),
                path: path.to_string(),
                query: query.to_string(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::url_parts::UrlParts;
    use super::*;

    fn credentials() -> SignCredentials {
        SignCredentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_key: "SECRETEXAMPLE".into(),
            security_token: Some("STS-TOKEN".into()),
        }
    }

    #[test]
    fn url_parsing_splits_host_path_and_query() {
        let parts = UrlParts::parse(
            "https://snap-access.cn-north-4.myhuaweicloud.com/api/v2/chat/completions?b=2&a=1",
        );
        assert_eq!(parts.host, "snap-access.cn-north-4.myhuaweicloud.com");
        assert_eq!(parts.path, "/api/v2/chat/completions");
        assert_eq!(parts.query, "b=2&a=1");
    }

    #[test]
    fn sdk_date_matches_huawei_format() {
        // 2026-10-20T04:15:00Z（epoch 秒 1792469700）
        assert_eq!(format_sdk_date(1_792_469_700_000), "20261020T041500Z");
        // 1970-01-01T00:00:00Z
        assert_eq!(format_sdk_date(0), "19700101T000000Z");
        // 跨日边界：2026-10-20T00:00:00Z 的前一秒仍是 19 号。
        assert_eq!(format_sdk_date(1_792_454_399_000), "20261019T235959Z");
        assert_eq!(format_sdk_date(1_792_454_400_000), "20261020T000000Z");
    }

    #[test]
    fn canonical_uri_appends_trailing_slash_and_encodes() {
        assert_eq!(canonical_uri("/api/v2/chat/completions"), "/api/v2/chat/completions/");
        // 空格编码成 %20（不是 +），这是与 form 编码的关键差异。
        assert_eq!(canonical_uri("/a b/c"), "/a%20b/c/");
        // 已有的斜杠不重复添加。
        assert_eq!(canonical_uri("/api/"), "/api/");
    }

    #[test]
    fn canonical_query_sorts_keys_and_values() {
        assert_eq!(canonical_query("b=2&a=1"), "a=1&b=2");
        assert_eq!(canonical_query(""), "");
        // 同名多值：排序后保留全部。
        assert_eq!(canonical_query("a=2&a=1"), "a=1&a=2");
    }

    #[test]
    fn signature_shape_matches_the_sdk_contract() {
        let url = UrlParts::parse(
            "https://snap-access.cn-north-4.myhuaweicloud.com/api/v2/chat/completions",
        );
        let headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("x-auth-token".to_string(), "tok".to_string()),
            ("app-id".to_string(), "CodeAgent3.0".to_string()),
        ];
        let body = br#"{"model":"x"}"#;
        let signed = sign("POST", &url, &headers, body, &credentials(), "20261020T031500Z");

        assert_eq!(signed.x_sdk_date, "20261020T031500Z");
        assert_eq!(signed.security_token.as_deref(), Some("STS-TOKEN"));
        assert_eq!(signed.content_sha256.len(), 64, "sha256 是 64 位十六进制");
        assert!(
            signed.authorization.starts_with("SDK-HMAC-SHA256 Access=AKIDEXAMPLE, SignedHeaders="),
            "{}",
            signed.authorization
        );
        // host 必须进签名（官方 SDK 强制要求）。
        assert!(signed.authorization.contains("host;"));
        // 业务头按小写参与排序。
        assert!(signed.authorization.contains("app-id"));
        assert!(signed.authorization.contains("x-auth-token"));
        // 三个签名头也必须在 SignedHeaders 里。
        assert!(signed.authorization.contains("x-sdk-content-sha256"));
        assert!(signed.authorization.contains("x-sdk-date"));
        assert!(signed.authorization.contains("x-security-token"));
        assert!(signed.authorization.contains("Signature="));
    }

    /// 签名对 payload 与头敏感：改一个字节就该换一个签名。
    #[test]
    fn signature_changes_with_payload_and_headers() {
        let url =
            UrlParts::parse("https://snap-access.cn-north-4.myhuaweicloud.com/api/v2/chat");
        let headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        let base = sign("POST", &url, &headers, b"{\"a\":1}", &credentials(), "20261020T031500Z");
        let other_body = sign("POST", &url, &headers, b"{\"a\":2}", &credentials(), "20261020T031500Z");
        assert_ne!(base.authorization, other_body.authorization);

        let mut extra = headers.clone();
        extra.push(("x-extra".to_string(), "1".to_string()));
        let with_header = sign("POST", &url, &extra, b"{\"a\":1}", &credentials(), "20261020T031500Z");
        assert_ne!(base.authorization, with_header.authorization);
    }

    /// 没有 STS token 时不该带 `x-security-token`。
    #[test]
    fn long_term_credentials_omit_security_token() {
        let url = UrlParts::parse("https://iam.myhuaweicloud.com/v5/caller-identity");
        let creds = SignCredentials {
            access_key_id: "AK".into(),
            secret_key: "SK".into(),
            security_token: None,
        };
        let signed = sign("GET", &url, &[], b"", &creds, "20261020T031500Z");
        assert!(signed.security_token.is_none());
        assert!(!signed.authorization.contains("x-security-token"));
    }

    #[test]
    fn credentials_need_both_key_parts() {
        assert!(credentials().is_usable());
        assert!(!SignCredentials::default().is_usable());
        assert!(!SignCredentials {
            access_key_id: "AK".into(),
            secret_key: String::new(),
            security_token: None,
        }
        .is_usable());
    }
}
