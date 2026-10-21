//! CodeArts（华为云码道 / CodeArts Agent）渠道适配器。
//!
//! ```json
//! { "id": "codearts", "adapter": "codearts",
//!   "options": { "base_url": "https://snap-access.cn-north-4.myhuaweicloud.com" } }
//! ```
//!
//! **与 Trae 的关键差异：鉴权是「STS token + AK/SK 签名」双重的。**
//! 每个请求都要：
//! ① 带 `x-auth-token: <STS security token>`；
//! ② 用临时 AK/SK 做 `SDK-HMAC-SHA256` 签名（见 [`signer`]）；
//! ③ 带 `app-id: CodeAgent3.0` 这组客户端头。
//! 三者缺一，上游只回一个笼统的 401。
//!
//! 好消息是**聊天协议本身是原生 OpenAI 形状**（`/api/v2/chat/completions`），
//! 所以报文编解码直接复用 [`openai_compat`]，不需要像 Trae 那样写有状态解码器。

pub mod signer;

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde_json::Value;

use crate::gateway::{
    channels::{
        openai_compat, AccountSnapshot, ChannelAdapter, Credential, UpstreamModel,
    },
    errors::{ErrorKind, ErrorScope, GatewayError},
    protocol::cir::{CanonicalEvent, CanonicalRequest},
    transport::UpstreamRequest,
};

use signer::url_parts::UrlParts;
use signer::SignCredentials;

/// 聊天端点（商业版 snap-access 域）。
const CHAT_PATH: &str = "/api/v2/chat/completions";
/// 内置模型目录。
const BUILTIN_MODELS_PATH: &str = "/v1/model/builtin";
/// 默认 OAuth client（与 refresh_token 绑定，换值会被 STS 拒）。
pub const CLIENT_ID: &str = "codearts-agent";

pub struct CodeArtsChannel {
    id: String,
    /// 网关主机（如 `https://snap-access.cn-north-4.myhuaweicloud.com`）。
    base_url: String,
    /// 单调序号：给 `x-snap-traceid` 用，同一纳秒并发下也能唯一。
    sequence: AtomicU64,
}

impl CodeArtsChannel {
    pub fn from_options(id: &str, options: &Value) -> Self {
        let base_url = options
            .get("base_url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
            .map(|value| value.trim_end_matches('/').to_string())
            .unwrap_or_else(|| "https://snap-access.cn-north-4.myhuaweicloud.com".to_string());
        Self {
            id: id.to_string(),
            base_url,
            sequence: AtomicU64::new(0),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// 从账号凭据里取出签名材料。缺 AK/SK 时**直接报错**，
    /// 而不是发一个注定 401 的请求（那样只会让账号进冷却，更难排查）。
    fn credentials(account: &AccountSnapshot) -> Result<SignCredentials, GatewayError> {
        let access_key_id = account.access_key_id.clone().unwrap_or_default();
        let secret_key = account.secret_key.clone().unwrap_or_default();
        if access_key_id.is_empty() || secret_key.is_empty() {
            return Err(GatewayError::new(
                ErrorKind::AuthExpired,
                ErrorScope::Account,
                401,
                "CodeArts 账号缺少 AK/SK（每个请求都要签名）：请重新登录获取临时凭据",
                false,
            ));
        }
        Ok(SignCredentials {
            access_key_id,
            secret_key,
            security_token: account.access_token.clone().filter(|token| !token.is_empty()),
        })
    }

    /// 组装带签名的请求头。
    ///
    /// 顺序很重要：**业务头先全部写进去，再签名** —— 签名覆盖全部头，
    /// 签完再加头会让签名失效。
    fn signed_headers(
        &self,
        account: &AccountSnapshot,
        credentials: &SignCredentials,
        method: &str,
        url: &str,
        accept: &str,
        extra: &[(String, String)],
        body: &[u8],
    ) -> Vec<(String, String)> {
        let mut headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            // **`Accept` 决定上游把请求路由到哪条链路**：
            // 目录接口必须 `application/json`；发 `text/event-stream` 会被当成
            // SSE 请求，回 `TM.00001001 未找到合法可路由api`（真实实测）。
            ("Accept".to_string(), accept.to_string()),
            (
                "x-snap-traceid".to_string(),
                format!("{:x}{:04x}", crate::gateway::timeutil::now_ms(), self.sequence.fetch_add(1, Ordering::Relaxed) & 0xffff),
            ),
            ("X-Language".to_string(), "zh-cn".to_string()),
            ("app-id".to_string(), "CodeAgent3.0".to_string()),
            ("is_confidential".to_string(), "false".to_string()),
        ];
        // `x-auth-token` 只在用 **STS 临时凭据** 时发。
        // 长期 AK/SK 没有 security token，发空值会被上游判成非法调用。
        if let Some(token) = account
            .access_token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            headers.push(("x-auth-token".to_string(), token.to_string()));
        }
        headers.extend(extra.iter().cloned());

        let signed = signer::sign(
            method,
            &UrlParts::parse(url),
            &headers,
            body,
            credentials,
            &signer::now_sdk_date(),
        );
        headers.push(("X-Sdk-Date".to_string(), signed.x_sdk_date));
        headers.push((
            "X-Sdk-Content-Sha256".to_string(),
            signed.content_sha256,
        ));
        if let Some(token) = signed.security_token {
            headers.push(("X-Security-Token".to_string(), token));
        }
        headers.push(("Authorization".to_string(), signed.authorization));
        headers
    }
}


#[async_trait]
impl ChannelAdapter for CodeArtsChannel {
    fn id(&self) -> &str {
        &self.id
    }

    fn adapter_kind(&self) -> &str {
        "codearts"
    }

    fn build_chat_request(
        &self,
        account: &AccountSnapshot,
        request: &CanonicalRequest,
    ) -> Result<UpstreamRequest, GatewayError> {
        let credentials = Self::credentials(account)?;
        // 上游就是 OpenAI 形状：直接复用通用编码（把 CIR 还原成 OpenAI 报文）。
        let body = crate::gateway::channels::custom::build_openai_chat_body(request);
        let raw = serde_json::to_vec(&body)
            .map_err(|error| GatewayError::internal(format!("请求体序列化失败：{error}")))?;
        let url = self.url(CHAT_PATH);
        let mut extra = vec![
            ("tool_stream".to_string(), "true".to_string()),
        ];
        // 会话粘性：把会话键作为 chat_id，保证多轮落在同一上游会话
        // （上游按它做 prompt cache 亲和）。
        if let Some(session) = request.metadata.session_key() {
            extra.push(("Chat-Id".to_string(), session.clone()));
        }
        let headers = self.signed_headers(
            account,
            &credentials,
            "POST",
            &url,
            "text/event-stream",
            &extra,
            &raw,
        );
        Ok(UpstreamRequest {
            url,
            headers,
            body,
        })
    }

    /// 上游是标准 OpenAI SSE：直接复用通用解析。
    fn parse_event(&self, frame: &str) -> Vec<CanonicalEvent> {
        openai_compat::parse_openai_chunk(frame)
    }

    fn classify_error(&self, status: u16, body: &str, upstream_model: &str) -> GatewayError {
        refine(status, body, upstream_model)
    }

    async fn list_models(
        &self,
        client: &reqwest::Client,
        account: &AccountSnapshot,
    ) -> Result<Option<Vec<UpstreamModel>>, GatewayError> {
        let credentials = match Self::credentials(account) {
            Ok(credentials) => credentials,
            // 没 AK/SK 时不该假装"没有模型"：让上层知道是凭据问题。
            Err(error) => return Err(error),
        };
        let url = self.url(BUILTIN_MODELS_PATH);
        let request = reqwest_compat::SignedGet {
            url: url.clone(),
            headers: self.signed_headers(
                account,
                &credentials,
                "GET",
                &url,
                // 目录是普通 JSON 接口：**不能**发 text/event-stream。
                "application/json",
                &[("Agent-Type".to_string(), "PromptCenter".to_string())],
                &[],
            ),
        };
        let payload = match request.send(client).await {
            Ok(payload) => payload,
            // 凭据类错误如实上报；其它失败按"拉取失败"处理（返回 None）。
            Err(error) if error.kind == ErrorKind::AuthExpired => return Err(error),
            // 其它失败**不能静默吞掉**：面板会显示"暂无模型"，而用户完全看不出
            // 是签名错、地址错、还是上游改版 —— 这正是"为什么没有模型"的成因。
            // 落一行日志，再按"拉取失败"返回 None（不影响服务启动）。
            Err(error) => {
                eprintln!(
                    "[gateway] CodeArts 模型目录拉取失败（{}）：{}",
                    url,
                    error.describe()
                );
                return Ok(None);
            }
        };
        let models = parse_builtin_models(&payload);
        Ok((!models.is_empty()).then_some(models))
    }

    /// CodeArts 的凭据是**用户粘贴的静态材料**（AK/SK，可能带 STS token），
    /// 没有可用的自动刷新端点（OAuth 需要 PKCE + DPoP，尚未实现）。
    ///
    /// 必须标记为静态：否则保鲜链路会尝试刷新 → 必然报错 → 账号被判为不可用，
    /// 表现为"刚导入就受限、要先登录"。这正是导入 AK/SK 后立刻不可用的原因。
    fn static_credential(&self) -> bool {
        true
    }


    async fn refresh(
        &self,
        client: &reqwest::Client,
        account: &AccountSnapshot,
    ) -> Result<Option<Credential>, GatewayError> {
        // OAuth 刷新（PKCE + DPoP）尚未实现：见模块说明。
        // 这里明确返回"需要重新登录"，而不是假装刷新成功。
        let refresh_token = account.refresh_token.as_deref().unwrap_or_default();
        if refresh_token.is_empty() {
            return Ok(None);
        }
        Err(GatewayError::new(
            ErrorKind::AuthExpired,            ErrorScope::Account,
            401,
            "CodeArts 的自动刷新需要 PKCE + DPoP，当前版本请重新登录导入凭据",
            false,
        ))
    }

    /// CodeArts 的登录**不走浏览器轮询**：它需要 PKCE + DPoP，
    /// 当前版本改为让用户粘贴凭据（STS token + 临时 AK/SK）。
    ///
    /// `auth_url` 放的是**给用户看的填写说明**（面板会把它当提示显示，不会拿去打开）。
    ///
    /// 文案只描述「去哪拿、要哪三样」，不引用任何第三方项目名 ——
    /// 用户没有义务知道参考实现叫什么。
    async fn login_start(
        &self,
        _client: &reqwest::Client,
        _platform: &str,
    ) -> Result<crate::gateway::channels::LoginTicket, GatewayError> {
        Ok(crate::gateway::channels::LoginTicket {
            state: LOGIN_PASTE_STATE.to_string(),
            // 只说清"要什么、从哪来"，不引用任何第三方项目名。

            auth_url: "直接用华为云控制台导出的访问密钥文件（credentials.csv）。\\n\

                     把整份内容粘进来即可，或只填 Access Key 与 Secret Key 两项。\\n\

                     有 CodeArts 的 STS 令牌也可以一并给出（更安全，会随会话过期）。"                .to_string(),
        })
    }

    fn login_needs_callback(&self) -> bool {
        true
    }

    /// CodeArts **没有浏览器授权页**：登录靠粘贴 codearts2api 的凭据 JSON。
    /// `login_start` 返回的 `auth_url` 是填写说明，面板不能拿去打开。
    fn login_has_auth_page(&self) -> bool {
        false
    }

    /// 解析粘贴进来的凭据。支持三种输入：
    /// ① codearts2api 的 `codearts-*.json` 原文；
    /// ② 同结构的 JSON 片段（`cloud_dragon_token` / `access_key_id` / `secret_access_key`）；
    /// ③ 纯文本三行：`token`、`AK`、`SK`。
    async fn login_complete(
        &self,
        _client: &reqwest::Client,
        callback: &str,
    ) -> Result<Credential, GatewayError> {
        parse_pasted_credential(callback)
    }
}

/// `login_start` 返回的固定 state（粘贴模式没有真正的授权票据）。
pub const LOGIN_PASTE_STATE: &str = "codearts:paste-credential";

/// 把用户粘贴的文本解析成凭据。
pub fn parse_pasted_credential(text: &str) -> Result<Credential, GatewayError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(GatewayError::invalid_request("请粘贴凭据内容"));
    }
    // ②b：华为云控制台导出的 credentials.csv（表头 + 一行数据，只有 AK/SK）。
    //
    // **必须放在 JSON 分支之前**：CSV 文本常能"侥幸"被 serde_json 解析成字符串，
    // 那样会一路走进 JSON 分支、然后因为找不到字段而报"无法识别"。
    if let Some((access_key_id, secret_key, user_name)) = parse_credentials_csv(trimmed) {
        return build_credential(
            None,
            Some(access_key_id),
            Some(secret_key),
            None,
            user_name,
            None,
            None,
            None,
        );
    }

    // ① / ②：JSON 形态。
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        let pick = |keys: &[&str]| -> Option<String> {
            keys.iter()
                .find_map(|key| value.get(*key))
                .and_then(|entry| entry.as_str())
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_string)
        };
        let token = pick(&["cloud_dragon_token", "security_token", "access_token", "token"]);
        let access_key_id = pick(&["access_key_id", "accessKeyId", "ak", "AccessKeyId"]);
        let secret_key = pick(&[
            "secret_access_key",
            "secretAccessKey",
            "sk",
            "SecretAccessKey",
        ]);
        let uid = pick(&["user_id", "userId", "UserID"]);
        let nickname = pick(&["user_name", "userName", "UserName"]);
        return build_credential(token, access_key_id, secret_key, uid, nickname, pick(&[
            "refresh_token",
            "refreshToken",
        ]), pick(&["code_verifier", "codeVerifier"]), pick(&["expiration", "expires_at"]));
    }

    // ③：纯文本三行（令牌 / AK / SK，或 AK / SK 两行）。

    let lines: Vec<&str> = trimmed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.len() >= 3 {
        return build_credential(
            Some(lines[0].to_string()),
            Some(lines[1].to_string()),
            Some(lines[2].to_string()),
            lines.get(3).map(|value| value.to_string()),
            None,
            None,
            None,
            None,
        );
    }

    Err(GatewayError::invalid_request(
        "无法识别这份凭据。请粘贴华为云控制台导出的 credentials.csv，\n\
         或提供 Access Key 与 Secret Key（JSON 字段：access_key_id / secret_access_key）。",
    ))
}

/// 解析华为云控制台导出的 `credentials.csv`。
///
/// 真实形状（「我的凭证 → 访问密钥 → 新增访问密钥」下载的就是它）：
///
/// ```csv
/// User Name,Access Key Id,Secret Access Key
/// "hid_to9t7v-tyvrj4uz",HPUA2REATHC7Z05QOR2K,s8lJJBnmbauEW7RqsEmIM948poc0VHLQpGzPDcqv
/// ```
///
/// 只有 AK/SK、**没有 STS token** —— 这是合法的长期凭据形态（见 `build_credential`）。
/// 列名中英文都可能出现，所以按表头关键字定位列，而不是写死下标。
///
/// 返回 `(access_key_id, secret_key, user_name)`。
fn parse_credentials_csv(text: &str) -> Option<(String, String, Option<String>)> {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let header = lines.next()?;
    let header_lower = header.to_ascii_lowercase();
    // 只处理"看起来像表头"的输入：必须同时有 access key 与 secret 两列。
    if !(header_lower.contains("access key") && header_lower.contains("secret")) {
        return None;
    }
    let columns: Vec<String> = header
        .split(',')
        .map(|column| column.trim().trim_matches('"').to_ascii_lowercase())
        .collect();
    let find = |needles: &[&str], exclude: &[&str]| -> Option<usize> {
        columns.iter().position(|column| {
            needles.iter().any(|needle| column.contains(needle))
                && !exclude.iter().any(|word| column.contains(word))
        })
    };
    let key_column = find(&["access key"], &[])?;
    let secret_column = find(&["secret"], &[])?;
    // `User Name` 是账号名（可空）；排除掉含 access 的列，免得匹配到 `Access Key Id`。
    let name_column = find(&["user name", "username"], &["access"]);

    let row = lines.next()?;
    let cells = split_csv_row(row);
    let cell = |index: Option<usize>| -> Option<String> {
        index
            .and_then(|index| cells.get(index))
            .map(|value| value.trim().trim_matches('"').to_string())
            .filter(|value| !value.is_empty())
    };
    Some((
        cell(Some(key_column))?,
        cell(Some(secret_column))?,
        cell(name_column),
    ))
}

/// 按逗号切一行 CSV，支持双引号包裹与 `""` 转义。
fn split_csv_row(row: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = row.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                cells.push(current.clone());
                current.clear();
            }
            other => current.push(other),
        }
    }
    cells.push(current);
    cells
}

fn build_credential(
    token: Option<String>,
    access_key_id: Option<String>,
    secret_key: Option<String>,
    uid: Option<String>,
    nickname: Option<String>,
    refresh_token: Option<String>,
    code_verifier: Option<String>,
    expiration: Option<String>,
) -> Result<Credential, GatewayError> {
    // **只有 AK/SK 是必需的**：签名只需要这两样（`signer::SignCredentials::is_usable`
    // 也只校验它们）。STS token 是"临时凭据"形态下才有的可选项 ——
    // 控制台里导出的长期访问密钥（`credentials.csv`）就只有 User Name / AK / SK 三列。
    //
    // 早先这里要求三项齐全，导致**长期 AK/SK 根本导不进来**：
    // 用户手里明明是正确的凭据，却被告知"不完整"。
    let missing: Vec<&str> = [
        (access_key_id.is_none(), "Access Key"),
        (secret_key.is_none(), "Secret Key"),
    ]
    .into_iter()
    .filter_map(|(absent, name)| absent.then_some(name))
    .collect();
    if !missing.is_empty() {
        return Err(GatewayError::invalid_request(format!(
            "凭据不完整，缺少：{}。需要 Access Key 与 Secret Key（华为云控制台「我的凭证 → 访问密钥」导出）。",
            missing.join("、")
        )));
    }
    let token = token.unwrap_or_default();
    Ok(Credential {
        // 没有 user_id 时用**凭据本身**派生稳定 uid：同一份凭据永远落在同一个账号上。
        // 注意不能只拿 token 派生 —— 长期 AK/SK 没有 token，那样所有账号会撞成同一个 uid。
        uid: Some(uid.filter(|value| !value.is_empty()).unwrap_or_else(|| {
            let seed = if token.is_empty() {
                format!("{}|{}", access_key_id.as_deref().unwrap_or_default(), secret_key.as_deref().unwrap_or_default())
            } else {
                token.clone()
            };
            crate::gateway::security::derive_id("codearts", &seed, "uid")
        })),
        nickname,
        access_token: token,
        access_key_id,
        secret_key,
        refresh_token: refresh_token.filter(|value| !value.is_empty()),
        code_verifier: code_verifier.filter(|value| !value.is_empty()),
        expires_at_ms: expiration.as_deref().and_then(parse_rfc3339_ms),
        ..Default::default()
    })
}

/// `2026-10-21T03:15:00Z` / `2026-10-21T11:15:00+08:00` → 纪元毫秒。
fn parse_rfc3339_ms(text: &str) -> Option<u64> {
    let (date, rest) = text.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;

    let (time, offset_secs) = if let Some(stripped) = rest.strip_suffix('Z') {
        (stripped, 0i64)
    } else {
        // 找时区标记（`+08:00` / `-05:00`）；日期段已切掉，所以这里的
        // 最后一个 `+`/`-` 一定是时区分隔符。
        let split_at = rest.rfind(['+', '-']);
        match split_at {
            Some(index) => {
                let offset = &rest[index..];
                let sign = if offset.starts_with('+') { 1i64 } else { -1 };
                let mut parts = offset[1..].split(':');
                let hours: i64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
                let minutes: i64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
                (&rest[..index], sign * (hours * 3600 + minutes * 60))
            }
            None => (rest, 0),
        }
    };
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts
        .next()
        .unwrap_or("0")
        .split('.')
        .next()?
        .parse()
        .ok()?;
    Some(crate::gateway::timeutil::civil_to_epoch_ms(
        year, month, day, hour, minute, second, offset_secs,
    ))
}

/// 解析 `/v1/model/builtin` 的响应。
///
/// 形状：`{"builtinModels":[{"model_id","model_name","model_desc","context_window","max_tokens"}]}`
pub fn parse_builtin_models(payload: &Value) -> Vec<UpstreamModel> {
    let list = payload
        .get("builtinModels")
        .or_else(|| payload.get("builtin_models"))
        .or_else(|| {
            payload
                .get("data")
                .and_then(|data| data.get("builtinModels"))
        })
        .and_then(Value::as_array);
    let mut models = Vec::new();
    for raw in list.into_iter().flatten() {
        let id = raw
            .get("model_id")
            .or_else(|| raw.get("modelId"))
            .or_else(|| raw.get("model_name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty());
        let Some(id) = id else { continue };
        let number = |key: &str| {
            raw.get(key).and_then(|value| match value {
                Value::Number(number) => number.as_u64(),
                Value::String(text) => text.parse().ok(),
                _ => None,
            })
        };
        // `supports_images` 是这条上游的权威视觉能力位（目录里没有顶层 multimodal）。
        let vision = raw
            .get("supports_images")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        models.push(UpstreamModel {
            id: id.to_string(),
            credits: parse_builtin_credits(raw),
            context_length: number("context_window").filter(|value| *value > 0),
            max_output_tokens: number("max_tokens").filter(|value| *value > 0),
            vendor: Some("codearts".into()),
            capabilities: crate::gateway::channels::ModelCapabilities {
                vision,
                // 未知即乐观放行：目录不声明工具/思考能力时不该在本地拦掉。
                tool_calls: false,
                reasoning: false,
                // 但"没声明"要如实记下来（区别于"声明了不支持"）。
                declared_vision: raw.get("supports_images").and_then(Value::as_bool).is_some(),
                declared_tools: false,
            },
            ..UpstreamModel::default()
        });
    }
    models
}

/// 从 `credit[]` 里取**倍率**。
///
/// 上游给的是分段计价表：
/// ```json
/// "credit":[{"ratio":"0.05","ratio_display":"0.7x","input_from":0,"input_to":-1,
///            "input_price":"8","cache_price":"2","output_price":"28"}]
/// ```
/// `ratio_display` 是给用户看的倍率（`0.7x`），`ratio` 是结算系数。
/// 优先透出 `ratio_display`，没有就用 `ratio` 拼一个 —— 面板只认 `x…` 形式。
pub fn parse_builtin_credits(raw: &Value) -> Option<String> {
    let segments = raw.get("credit").and_then(Value::as_array)?;
    // `input_from: 0 / input_to: -1` 是"全区间"那一段，取它最准。
    let whole = segments.iter().find(|segment| {
        segment.get("input_from").and_then(Value::as_i64) == Some(0)
            && segment.get("input_to").and_then(Value::as_i64) == Some(-1)
    });
    let segment = whole.or_else(|| segments.first())?;
    if let Some(display) = segment
        .get("ratio_display")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(normalize_rate(display));
    }
    segment
        .get("ratio")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_rate)
}

/// 把 `0.7x` / `x0.5` / `0.05` 统一成 `x0.70` / `x0.50` / `x0.05`。
fn normalize_rate(value: &str) -> String {
    // `x` 可能在前也可能在后（上游同一个字段两种写法都出现过）。
    let trimmed = value
        .trim()
        .trim_start_matches(['x', 'X'])
        .trim_end_matches(['x', 'X'])
        .trim();
    match trimmed.parse::<f64>() {
        Ok(rate) => format!("x{rate:.2}"),
        // 解析不了就原样透出，别把上游给的东西吞掉。
        Err(_) => value.trim().to_string(),
    }
}

/// 上游错误分类：先认业务码，再按状态码兜底。
///
/// 这些码来自参考实现的实测（`internal/upstream/sse.go` 的 `embeddedSSEError`）：
/// · `429` / `tm.00001041` / `tpm` / `并发会话` —— 限流，可换号重试；
/// · `002002009` / `not registered` / `4004.200` / `benefit not found` ——
///   该账号用不了这个模型，是**模型级**问题，不该冷却整个账号；
/// · `InferHub.001001005` / `request param is invalid` —— 请求参数错，重试无用。
pub fn refine(status: u16, body: &str, upstream_model: &str) -> GatewayError {
    let lowered = body.to_ascii_lowercase();
    if lowered.contains("tm.00001041")
        || lowered.contains("并发会话")
        || lowered.contains("\"tpm\"")
        || status == 429
    {
        return GatewayError::new(
            ErrorKind::RateLimited,
            ErrorScope::Account,
            429,
            openai_compat::short(body, "CodeArts 上游限流（TPM/并发）"),
            true,
        )
        .with_upstream(openai_compat::raw(body));
    }
    if lowered.contains("002002009")
        || lowered.contains("not registered")
        || lowered.contains("4004.200")
        || lowered.contains("benefit not found")
    {
        // 模型级：同账号的其他模型照用。
        return GatewayError::new(
            ErrorKind::ModelNotFound,
            ErrorScope::AccountModel(upstream_model.to_string()),
            400,
            openai_compat::short(body, "该账号未开通这个模型（福利额度可能已领完）"),
            false,
        )
        .with_upstream(openai_compat::raw(body));
    }
    if lowered.contains("inferhub.001001005") || lowered.contains("request param is invalid") {
        return GatewayError::invalid_request(openai_compat::short(body, "上游拒绝了请求参数"))
            .with_upstream(openai_compat::raw(body));
    }
    openai_compat::classify_openai_error(status, body)
}

/// 带签名的 GET（`transport` 只有 POST/GET-without-sign 的通用封装，
/// 而 CodeArts 的 GET 也要签名，所以单独走一条）。
mod reqwest_compat {
    use crate::gateway::errors::GatewayError;

    pub struct SignedGet {
        pub url: String,
        pub headers: Vec<(String, String)>,
    }

    impl SignedGet {
        pub async fn send(
            &self,
            client: &reqwest::Client,
        ) -> Result<serde_json::Value, GatewayError> {
            let mut builder = client.get(&self.url);
            for (name, value) in &self.headers {
                builder = builder.header(name, value);
            }
            let response = builder.send().await.map_err(GatewayError::from_reqwest)?;
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            if !status.is_success() {
                return Err(crate::gateway::channels::codearts::refine(
                    status.as_u16(),
                    &text,
                    "",
                ));
            }
            serde_json::from_str(&text)
                .map_err(|error| GatewayError::network(format!("目录响应不是合法 JSON：{error}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;


    /// 控制台导出的 `credentials.csv`：只有 AK/SK，没有 STS token。
    ///
    /// 这是长期访问密钥的**合法形态**，早先因为要求三项齐全而导不进来。
    #[test]
    fn console_credentials_csv_is_accepted_without_an_sts_token() {
        let csv = "User Name,Access Key Id,Secret Access Key\n\
                   \"hid_to9t7v-tyvrj4uz\",HPUA2REATHC7Z05QOR2K,s8lJJBnmbauEW7RqsEmIM948poc0VHLQpGzPDcqv";
        let credential = parse_pasted_credential(csv).expect("控制台导出的 csv 必须能导入");
        assert_eq!(credential.access_key_id.as_deref(), Some("HPUA2REATHC7Z05QOR2K"));
        assert_eq!(
            credential.secret_key.as_deref(),
            Some("s8lJJBnmbauEW7RqsEmIM948poc0VHLQpGzPDcqv")
        );
        assert_eq!(credential.nickname.as_deref(), Some("hid_to9t7v-tyvrj4uz"));
        assert_eq!(credential.access_token, "", "长期 AK/SK 没有 STS token");
        // uid 必须由 AK/SK 派生，否则所有长期凭据会撞成同一个账号。
        assert!(credential.uid.as_deref().is_some_and(|uid| !uid.is_empty()));
    }

    /// 同一份 AK/SK 反复导入要落在同一个 uid 上（不然每点一次就多一个号）。
    #[test]
    fn the_same_key_pair_yields_the_same_uid() {
        let csv = "User Name,Access Key Id,Secret Access Key\n\
                   \"u\",AK1,SK1";
        let first = parse_pasted_credential(csv).expect("第一次");
        let second = parse_pasted_credential(csv).expect("第二次");
        assert_eq!(first.uid, second.uid);
    }

    /// 只有 AK、没有 SK：必须拒绝，并说清缺的是哪一项。
    #[test]
    fn a_key_pair_without_the_secret_is_rejected() {
        let error = parse_pasted_credential("User Name,Access Key Id\n\"u\",AK1")
            .expect_err("只有 AK 不能导入");
        assert!(error.message.contains("Secret Key"), "{}", error.message);
    }

    /// 三行纯文本形态仍然可用（AK/SK 两行 + 可选 user_id 不强制）。
    #[test]
    fn three_line_text_form_still_works() {
        let credential = parse_pasted_credential("TOKEN-1\nAK-1\nSK-1").expect("三行形态");
        assert_eq!(credential.access_token, "TOKEN-1");
        assert_eq!(credential.access_key_id.as_deref(), Some("AK-1"));
        assert_eq!(credential.secret_key.as_deref(), Some("SK-1"));
    }
    fn channel() -> CodeArtsChannel {
        CodeArtsChannel::from_options("codearts", &json!({}))
    }

    fn account() -> AccountSnapshot {
        AccountSnapshot {
            channel_id: "codearts".into(),
            uid: "uid-1".into(),
            access_token: Some("STS-TOKEN".into()),
            refresh_token: Some("rt".into()),
            access_key_id: Some("AKID".into()),
            secret_key: Some("SECRET".into()),
            ..Default::default()
        }
    }

    fn request() -> CanonicalRequest {
        CanonicalRequest {
            model: "snap-chat".into(),
            system: Vec::new(),
            messages: vec![crate::gateway::protocol::cir::Message {
                role: crate::gateway::protocol::cir::Role::User,
                content: vec![crate::gateway::protocol::cir::Block::text("hi")],
                tool_calls: Vec::new(),
                tool_call_id: None,
                name: None,
                reasoning: None,
            }],
            tools: Vec::new(),
            tool_choice: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream: true,
            reasoning: None,
            metadata: Default::default(),
            raw: json!({}),
        }
    }

    #[test]
    fn default_and_custom_base_url() {
        assert_eq!(
            channel().base_url,
            "https://snap-access.cn-north-4.myhuaweicloud.com"
        );
        let custom = CodeArtsChannel::from_options(
            "codearts-sg",
            &json!({"base_url": "https://snap-access.ap-southeast-1.myhuaweicloud.com/"}),
        );
        assert_eq!(
            custom.base_url,
            "https://snap-access.ap-southeast-1.myhuaweicloud.com"
        );
    }

    /// 缺 AK/SK 时必须**提前报错**，而不是发一个注定 401 的请求。
    #[test]
    fn missing_keys_fail_before_the_network_call() {
        let mut without = account();
        without.access_key_id = None;
        without.secret_key = None;
        let error = CodeArtsChannel::credentials(&without).expect_err("必须报错");
        assert_eq!(error.kind, ErrorKind::AuthExpired);
        assert!(error.message.contains("AK/SK"), "{}", error.message);
    }

    /// 签名头要齐全，且 `Authorization` 必须覆盖业务头。
    #[test]
    fn chat_request_is_fully_signed() {
        let upstream = channel()
            .build_chat_request(&account(), &request())
            .expect("应当能构建报文");
        assert!(upstream.url.ends_with("/api/v2/chat/completions"));
        let find = |name: &str| {
            upstream
                .headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.clone())
        };
        assert_eq!(find("x-auth-token").as_deref(), Some("STS-TOKEN"));
        assert_eq!(find("app-id").as_deref(), Some("CodeAgent3.0"));
        assert_eq!(find("X-Security-Token").as_deref(), Some("STS-TOKEN"));
        assert!(find("X-Sdk-Date").is_some());
        assert!(find("X-Sdk-Content-Sha256").is_some());
        let authorization = find("Authorization").expect("必须有签名头");
        assert!(authorization.starts_with("SDK-HMAC-SHA256 Access=AKID"));
        // 业务头必须进 SignedHeaders（顺序问题会让签名失效）。
        assert!(authorization.contains("app-id"), "{authorization}");
        assert!(authorization.contains("x-auth-token"), "{authorization}");
        // 请求体是 OpenAI 形状。
        assert_eq!(upstream.body["model"], "snap-chat");
        assert_eq!(upstream.body["stream"], true);
    }

    /// 真实目录响应（抓包）里的字段：倍率在 `credit[].ratio_display`，
    /// 视觉能力在 `supports_images`。
    #[test]
    fn builtin_models_read_credit_and_vision_from_the_real_shape() {
        let payload = json!({"count": 2, "builtinModels": [
            {
                "model_id": "GLM-5.2", "model_name": "GLM-5.2",
                "model_category": "文本生成", "supports_images": false,
                "credit": [{
                    "input_from": 0, "input_to": -1, "output_from": 0, "output_to": -1,
                    "ratio": "0.05", "input_price": "8", "cache_price": "2",
                    "output_price": "28", "ratio_display": "0.7x"
                }]
            },
            {
                "model_id": "Qwen3-VL-235B", "model_name": "Qwen3-VL-235B",
                "model_category": "图片理解", "supports_images": true, "credit": []
            }
        ]});
        let models = parse_builtin_models(&payload);
        assert_eq!(models.len(), 2);
        // `ratio_display: "0.7x"` → 面板认的 `x0.70`。
        assert_eq!(models[0].credits.as_deref(), Some("x0.70"));
        assert!(!models[0].capabilities.vision);
        assert!(models[0].capabilities.declared_vision, "声明过就要记下来");
        // 没有 credit 段 → 不编造倍率。
        assert_eq!(models[1].credits, None);
        assert!(models[1].capabilities.vision, "supports_images=true");
    }

    /// 倍率归一化：`0.7x` / `0.05` / `x0.5` 都要变成 `x…` 形式。
    #[test]
    fn rate_normalization_matches_the_panel_expectation() {
        assert_eq!(normalize_rate("0.7x"), "x0.70");
        assert_eq!(normalize_rate("0.05"), "x0.05");
        assert_eq!(normalize_rate("x0.5"), "x0.50");
        assert_eq!(normalize_rate("1.5X"), "x1.50");
        // 认不出的原样保留，不吞掉上游信息。
        assert_eq!(normalize_rate("面议"), "面议");
    }

    /// 目录接口必须用 `Accept: application/json`。
    ///
    /// 发 `text/event-stream` 会被上游当成 SSE 路由请求，回
    /// `TM.00001001 未找到合法可路由api, agentType:PromptCenter, 协议:SSE`
    /// —— 这个错误看起来像"没开通"，实际只是头写错了。
    #[test]
    fn catalog_headers_ask_for_json_not_sse() {
        let channel = channel();
        let account = account();
        let creds = CodeArtsChannel::credentials(&account).expect("credentials");
        let url = channel.url(BUILTIN_MODELS_PATH);
        let headers = channel.signed_headers(
            &account,
            &creds,
            "GET",
            &url,
            "application/json",
            &[("Agent-Type".to_string(), "PromptCenter".to_string())],
            &[],
        );
        let find = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.clone())
        };
        assert_eq!(find("Accept").as_deref(), Some("application/json"));
        assert_eq!(find("Agent-Type").as_deref(), Some("PromptCenter"));
        assert!(find("Authorization")
            .is_some_and(|value| value.starts_with("SDK-HMAC-SHA256 Access=")));
    }

    /// 长期 AK/SK 没有 STS token 时**不能发空的 `x-auth-token`**。
    #[test]
    fn empty_security_token_is_not_sent_as_an_empty_header() {
        let channel = channel();
        let mut long_term = account();
        long_term.access_token = None;
        let creds = CodeArtsChannel::credentials(&long_term).expect("credentials");
        let url = channel.url(BUILTIN_MODELS_PATH);
        let headers = channel.signed_headers(
            &long_term,
            &creds,
            "GET",
            &url,
            "application/json",
            &[],
            &[],
        );
        assert!(
            !headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("x-auth-token")),
            "没有 STS token 就不该发 x-auth-token"
        );

        // 有 STS token 时必须带上。
        let creds = CodeArtsChannel::credentials(&account()).expect("credentials");
        let headers =
            channel.signed_headers(&account(), &creds, "GET", &url, "application/json", &[], &[]);
        assert!(headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("x-auth-token") && value == "STS-TOKEN"));
    }

    #[test]
    fn builtin_models_parse_with_limits() {
        let payload = json!({"builtinModels": [
            {"model_id": "snap-chat", "model_name": "Snap Chat",
             "context_window": 128000, "max_tokens": 8192},
            {"model_name": "only-name", "context_window": 0},
            {"model_id": ""}
        ]});
        let models = parse_builtin_models(&payload);
        assert_eq!(models.len(), 2, "空 id 要被跳过");
        assert_eq!(models[0].id, "snap-chat");
        assert_eq!(models[0].context_length, Some(128_000));
        assert_eq!(models[0].max_output_tokens, Some(8_192));
        // `context_window: 0` 视为"未知"，不是"上限 0"。
        assert_eq!(models[1].context_length, None);
    }

    #[test]
    fn error_codes_map_to_the_right_scope() {
        // 限流：账号级，可换号重试。
        let limited = refine(200, r#"{"error_code":"TM.00001041","error_msg":"tpm"}"#, "m");
        assert_eq!(limited.kind, ErrorKind::RateLimited);
        assert_eq!(limited.scope, ErrorScope::Account);
        assert!(limited.retryable);

        // 模型未开通：只停这个模型，不能拉黑账号。
        let unregistered = refine(400, r#"{"error_code":"002002009","error_msg":"not registered"}"#, "glm-5.3");
        assert_eq!(unregistered.kind, ErrorKind::ModelNotFound);
        assert_eq!(
            unregistered.scope,
            ErrorScope::AccountModel("glm-5.3".into())
        );
        assert!(!unregistered.retryable, "换号也没用");

        // 参数错：请求问题。
        let invalid = refine(400, r#"{"error_code":"InferHub.001001005"}"#, "m");
        assert_eq!(invalid.kind, ErrorKind::InvalidRequest);
        assert!(!invalid.retryable);
    }

    #[test]
    fn unknown_body_falls_back_to_status_semantics() {
        let auth = refine(401, "unauthorized", "m");
        assert_eq!(auth.kind, ErrorKind::AuthExpired);
        let server = refine(503, "boom", "m");
        assert_eq!(server.kind, ErrorKind::Upstream5xx);
    }

    /// 自动刷新当前是"明确不支持"，不能假装成功（否则会静默用坏凭据）。
    #[tokio::test]
    async fn refresh_reports_unsupported_instead_of_lying() {
        let error = channel()
            .refresh(&reqwest::Client::new(), &account())
            .await
            .expect_err("必须报错");
        assert!(error.message.contains("DPoP"), "{}", error.message);
    }

    /// codearts2api 的 `auths/codearts-*.json` 原文可以直接粘进来。
    #[test]
    fn pasted_auth_file_parses_into_a_credential() {
        let text = r#"{
            "user_id": "u-42",
            "user_name": "张三",
            "cloud_dragon_token": "STS-TOKEN",
            "access_key_id": "AKID",
            "secret_access_key": "SECRET",
            "expiration": "2026-10-21T03:15:00Z",
            "refresh_token": "rt-1",
            "code_verifier": "cv-1"
        }"#;
        let credential = parse_pasted_credential(text).expect("应当解析成功");
        assert_eq!(credential.uid.as_deref(), Some("u-42"));
        assert_eq!(credential.nickname.as_deref(), Some("张三"));
        assert_eq!(credential.access_token, "STS-TOKEN");
        assert_eq!(credential.access_key_id.as_deref(), Some("AKID"));
        assert_eq!(credential.secret_key.as_deref(), Some("SECRET"));
        assert_eq!(credential.refresh_token.as_deref(), Some("rt-1"));
        assert_eq!(credential.code_verifier.as_deref(), Some("cv-1"));
        assert!(credential.expires_at_ms.is_some(), "Rfc3339 要能转成毫秒");
    }

    /// 缺 AK/SK 时必须报得清楚，而不是入库一个注定 401 的账号。
    #[test]
    fn incomplete_credential_is_rejected_with_the_missing_fields() {
        let error = parse_pasted_credential(r#"{"cloud_dragon_token":"t"}"#)
            .expect_err("必须报错");
        // 报错要用人话点出缺哪两项（不是内部字段名）。
        assert!(error.message.contains("Access Key"), "{}", error.message);
        assert!(error.message.contains("Secret Key"), "{}", error.message);
    }

    /// 纯文本三行也支持（临时 AK/SK 直接贴的那种）。
    #[test]
    fn three_line_text_credential_is_accepted() {
        let credential =
            parse_pasted_credential("STS-TOKEN\nAKID\nSECRET\n").expect("三行应当可用");
        assert_eq!(credential.access_token, "STS-TOKEN");
        assert_eq!(credential.access_key_id.as_deref(), Some("AKID"));
        assert_eq!(credential.secret_key.as_deref(), Some("SECRET"));
        // 没给 user_id 时用 token 派生稳定 uid。
        assert!(credential.uid.as_deref().is_some_and(|uid| !uid.is_empty()));
    }

    #[test]
    fn garbage_input_is_rejected() {
        assert!(parse_pasted_credential("   ").is_err());
        assert!(parse_pasted_credential("只有一行").is_err());
    }

    #[test]
    fn rfc3339_offsets_are_honoured() {
        let utc = parse_rfc3339_ms("2026-10-21T03:15:00Z").expect("UTC");
        let plus8 = parse_rfc3339_ms("2026-10-21T11:15:00+08:00").expect("+08:00");
        assert_eq!(utc, plus8, "同一时刻的两种写法要相等");
    }
}

#[cfg(test)]
mod probe_chat {
    use super::*;
    use crate::gateway::storage::Storage;
    use futures_util::StreamExt;

    #[tokio::test]
    #[ignore]
    async fn dump_chat() {
        let dir = std::path::PathBuf::from(
            r"C:\Users\issuser\AppData\Roaming\com.issuser.dynamic-island\gateway",
        );
        let store = crate::agent::secrets::OsCredentialStore;
        let storage = Storage::open(&dir, &store).expect("open");
        let record = storage
            .list_accounts()
            .expect("list")
            .into_iter()
            .find(|r| r.channel_id == "codearts")
            .expect("no codearts account");
        let credential = record.credential.expect("no credential");
        let account = AccountSnapshot {
            channel_id: record.channel_id.clone(),
            uid: record.uid.clone(),
            access_token: Some(credential.access_token.clone()).filter(|t| !t.is_empty()),
            refresh_token: credential.refresh_token.clone(),
            enterprise_id: credential.enterprise_id.clone(),
            access_key_id: credential.access_key_id.clone(),
            secret_key: credential.secret_key.clone(),
            code_verifier: credential.code_verifier.clone(),
        };
        let channel = CodeArtsChannel::from_options("codearts", &serde_json::json!({}));
        let client = reqwest::Client::new();

        // 先看目录解析结果。
        let models = channel
            .list_models(&client, &account)
            .await
            .expect("list_models")
            .unwrap_or_default();
        println!("=== 目录 {} 个 ===", models.len());
        for m in models.iter().take(10) {
            println!(
                "  {:<24} credits={:?} vision={}",
                m.id, m.credits, m.capabilities.vision
            );
        }
        let Some(first) = models.first() else { panic!("目录为空") };
        let model = first.id.clone();

        // 再打一次 chat。
        let request = CanonicalRequest {
            model: model.clone(),
            system: Vec::new(),
            messages: vec![crate::gateway::protocol::cir::Message {
                role: crate::gateway::protocol::cir::Role::User,
                content: vec![crate::gateway::protocol::cir::Block::text("只回复两个字：你好")],
                tool_calls: Vec::new(),
                tool_call_id: None,
                name: None,
                reasoning: None,
            }],
            tools: Vec::new(),
            tool_choice: None,
            temperature: None,
            top_p: None,
            max_tokens: Some(64),
            stream: true,
            reasoning: None,
            metadata: Default::default(),
            raw: serde_json::json!({}),
        };
        match channel.chat_stream(&account, &request, &client).await {
            Ok(mut frames) => {
                let mut text = String::new();
                let mut seen = 0usize;
                while let Some(frame) = frames.next().await {
                    seen += 1;
                    match frame {
                        Ok(frame) => {
                            for event in channel.parse_event(&frame) {
                                match event {
                                    CanonicalEvent::Text { text: chunk } => text.push_str(&chunk),
                                    CanonicalEvent::Error { message } => {
                                        panic!("上游 error 事件：{message}")
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Err(error) => panic!("帧失败：{}", error.describe()),
                    }
                    if text.chars().count() > 20 || seen > 300 {
                        break;
                    }
                }
                println!("=== chat 模型 {model} 帧数 {seen} ===");
                println!("=== 正文 ===\n{text}");
            }
            Err(error) => println!("=== chat 失败 ===\n{}", error.describe()),
        }
    }
}
