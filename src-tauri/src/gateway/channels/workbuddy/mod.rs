//! WorkBuddy 渠道适配器。国内版与国际版**共用这一份代码**，靠 `options` 区分：
//!
//! ```json
//! { "id": "wb-cn",   "adapter": "workbuddy", "options": {"realm": "cn",   "outbound_identity": "workbuddy"} }
//! { "id": "wb-intl", "adapter": "workbuddy", "options": {"realm": "intl", "outbound_identity": "workbuddy"} }
//! ```
//!
//! 同一个适配器实例化两次就是两个独立渠道 —— 这是「新增渠道核心零改动」的第一道验证。

pub mod auth;
pub mod billing;
pub mod daily;
pub mod tasks;
pub mod catalog;
pub mod hygiene;
pub mod identity;

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde_json::Value;

use crate::gateway::channels::openai_compat;
use crate::gateway::{
    channels::{
        AccountSnapshot, ChannelAdapter, Credential, LoginPoll, LoginTicket, UpstreamModel,
    },
    errors::{ErrorKind, ErrorScope, GatewayError},
    protocol::cir::{CanonicalEvent, CanonicalRequest},
    transport::UpstreamRequest,
};

use identity::{Hosts, Outbound, Realm};

pub struct WorkbuddyChannel {
    id: String,
    realm: Realm,
    outbound: Outbound,
    hosts: Hosts,
    /// 单调序号：给 `X-Request-ID` 用，避免同账号高并发下撞 ID。
    sequence: AtomicU64,
}

impl WorkbuddyChannel {
    pub fn from_options(id: &str, options: &Value) -> Self {
        let realm = Realm::parse(
            options
                .get("realm")
                .and_then(Value::as_str)
                .unwrap_or("cn"),
        );
        let outbound = Outbound::parse(
            options
                .get("outbound_identity")
                .and_then(Value::as_str)
                .unwrap_or("workbuddy"),
        );
        Self {
            id: id.to_string(),
            realm,
            outbound,
            hosts: identity::hosts(realm, outbound),
            sequence: AtomicU64::new(0),
        }
    }

    fn snapshot_catalog(&self) -> Vec<UpstreamModel> {
        match self.realm {
            Realm::Cn => catalog::snapshot_cn(),
            // 国际版目录与国内版差异较大，没有经过验证的快照就不编造。
            Realm::Intl => Vec::new(),
        }
    }
}

#[async_trait]
impl ChannelAdapter for WorkbuddyChannel {
    fn id(&self) -> &str {
        &self.id
    }

    fn adapter_kind(&self) -> &str {
        "workbuddy"
    }

    fn build_chat_request(
        &self,
        account: &AccountSnapshot,
        request: &CanonicalRequest,
    ) -> Result<UpstreamRequest, GatewayError> {
        let body = hygiene::build_chat_body(&request.model, request);
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        Ok(UpstreamRequest {
            url: self.hosts.chat_url(catalog::CHAT_PATH),
            headers: identity::chat_headers(
                &self.id,
                self.realm,
                self.outbound,
                &self.hosts,
                account,
                sequence,
            ),
            body,
        })
    }

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
        let snapshot = self.snapshot_catalog();
        let Some((ids, meta)) = catalog::fetch_remote(client, &self.hosts, self.realm, account).await
        else {
            // 拉取失败（网络 / 非 2xx / 空列表）：用快照顶，绝不对外报"没有模型"。
            return Ok((!snapshot.is_empty()).then_some(snapshot));
        };
        let snapshot_credits = snapshot
            .iter()
            .map(|entry| (entry.id.clone(), entry.credits.clone()))
            .collect();
        let curated = catalog::curate(&ids, &meta, &snapshot_credits);
        Ok(Some(catalog::merge(&curated, &meta, &snapshot)))
    }

    async fn refresh(
        &self,
        client: &reqwest::Client,
        account: &AccountSnapshot,
    ) -> Result<Option<Credential>, GatewayError> {
        let Some(refresh_token) = account.refresh_token.as_deref().filter(|t| !t.is_empty()) else {
            return Ok(None);
        };
        auth::refresh(client, &self.hosts, account, refresh_token)
            .await
            .map(Some)
    }

    async fn login_start(
        &self,
        client: &reqwest::Client,
        platform: &str,
    ) -> Result<LoginTicket, GatewayError> {
        auth::login_start(client, &self.hosts, platform).await
    }

    async fn login_poll(
        &self,
        client: &reqwest::Client,
        state: &str,
    ) -> Result<LoginPoll, GatewayError> {
        auth::login_poll(client, &self.hosts, state).await
    }

    async fn list_tasks(
        &self,
        client: &reqwest::Client,
        account: &AccountSnapshot,
    ) -> Result<Option<Vec<crate::gateway::channels::TaskInfo>>, GatewayError> {
        if self.realm != Realm::Cn {
            // 国际版没有国内成长任务中心。
            return Ok(None);
        }
        tasks::list(client, &self.hosts, account).await.map(Some)
    }

    async fn run_tasks(
        &self,
        client: &reqwest::Client,
        account: &AccountSnapshot,
        progress: tokio::sync::mpsc::UnboundedSender<crate::gateway::channels::TaskRunResult>,
    ) -> Result<Vec<crate::gateway::channels::TaskRunResult>, GatewayError> {
        if self.realm != Realm::Cn {
            return Ok(Vec::new());
        }
        tasks::run(client, &self.hosts, account, progress).await
    }

    async fn claim_task(
        &self,
        client: &reqwest::Client,
        account: &AccountSnapshot,
        code: &str,
    ) -> Result<crate::gateway::channels::TaskRunResult, GatewayError> {
        if self.realm != Realm::Cn {
            return Err(GatewayError::unsupported("国际版没有成长任务体系"));
        }
        // 领奖的失败原因（未达标/已领过）是**行内文案**，不是异常：返回 Ok 交给面板展示。
        Ok(tasks::claim(client, &self.hosts, account, code).await)
    }

    async fn fetch_quota(
        &self,
        client: &reqwest::Client,
        account: &AccountSnapshot,
    ) -> Result<Option<crate::gateway::channels::QuotaSnapshot>, GatewayError> {
        billing::fetch(client, &self.hosts, account).await
    }

    async fn daily_actions(
        &self,
        client: &reqwest::Client,
        account: &AccountSnapshot,
    ) -> Result<Option<Vec<crate::gateway::channels::DailyAction>>, GatewayError> {
        // 动作按 realm 分开（CN：签到 + 猫猫旅行；Intl：每日活跃对话）。
        Ok(Some(
            daily::actions(client, &self.hosts, self.realm, account).await,
        ))
    }

    async fn run_daily_action(
        &self,
        client: &reqwest::Client,
        account: &AccountSnapshot,
        code: &str,
    ) -> Result<crate::gateway::channels::TaskRunResult, GatewayError> {
        Ok(daily::run(client, &self.hosts, self.realm, account, code).await)
    }

    async fn sign_in(
        &self,
        client: &reqwest::Client,
        account: &AccountSnapshot,
    ) -> Result<bool, GatewayError> {
        Ok(auth::sign_in(client, &self.hosts, account).await)
    }
}

/// 上游错误语义：先在正文里找已知数字码，找不到就按状态码兜底。
///
/// 数字码比状态码精确得多 —— 例如 12403 是「UA 不被接受」，按状态码只会看到 400。
///
/// `upstream_model` 只用来描述**范围**：这个渠道的限额是按模型的，
/// 6004 要落到 [`ErrorScope::AccountModel`]，不能拉黑整个账号。
pub fn refine(status: u16, body: &str, upstream_model: &str) -> GatewayError {
    // 渠道校验拦截要单独认出来：它不是"请求不合法"，重试就能过
    // （真机 A/B 见 `ErrorKind::ChannelBlocked` 的注释）。识别用的是**字符串码**
    // `11-128`（带连字符，不是数字），所以不能走 known_code 的数值扫描。
    let lowered = body.to_ascii_lowercase();
    if lowered.contains("11-128") || lowered.contains("unapproved channel") {
        return GatewayError::new(
            ErrorKind::ChannelBlocked,
            ErrorScope::Request,
            429,
            "上游渠道校验拦截（该判定带随机性，重试即可；换账号重试更稳）",
            true,
        )
        .with_upstream(openai_compat::raw(body));
    }
    let refined = refine_kind(status, body, upstream_model);
    refined.with_upstream(openai_compat::raw(body))
}

fn refine_kind(status: u16, body: &str, upstream_model: &str) -> GatewayError {
    match known_code(body) {
        Some(12403) => GatewayError::invalid_request(
            "上游拒绝了当前客户端标识（UA）：检查渠道的出站身份配置",
        ),
        Some(11135) => {
            GatewayError::invalid_request("图片数据非法：换一张真图或新开会话再试")
        }
        Some(11133) => GatewayError::invalid_request(openai_compat::short(body, "请求参数不合法")),
        Some(11148) => {
            GatewayError::invalid_request("工具调用序列断裂（本地自愈未覆盖），建议重开会话")
        }
        // 6004 = **模型级**限额（参考实现的原话："usage exceeds frequency limit"，
        // 正文会给 "your usage will reset at <时间>"）。必须落到该模型：
        // 同账号的其他模型照常能跑 —— 早先写成 Account，等于一个模型限额把整个账号拉黑。
        // 拿不到 reset 时刻也算模型级（池子会给它一个模型退避），不能因此停整个账号。
        Some(6004) => {
            let reset_at = openai_compat::parse_rate_limit_reset(body);
            // 恢复时刻写进提示里：客户端/面板要能用它决定"等还是换模型"。
            let when = match reset_at {
                Some(until) => {
                    let remaining = until.saturating_sub(crate::gateway::timeutil::now_ms());
                    format!(
                        "，预计 {} 后恢复",
                        crate::gateway::core::pool::humanize(remaining)
                    )
                }
                // 上游没给时刻：如实说"我们自己按模型退避"，别让人以为是上游承诺。
                None => "，恢复时刻未知（上游未给出，按模型退避重试）".to_string(),
            };
            GatewayError::new(
                ErrorKind::RateLimited,
                ErrorScope::AccountModel(upstream_model.to_string()),
                429,
                format!("该模型被上游限流{when}（同账号其他模型不受影响）"),
                true,
            )
            .with_reset_at(reset_at)
        }
        _ => openai_compat::classify_openai_error(status, body),
    }
}

/// 按独立数字 token 匹配错误码，避免把正文里的 token 计数当成错误码。
fn known_code(body: &str) -> Option<i64> {
    const CODES: [i64; 5] = [12403, 11135, 11133, 11148, 6004];
    CODES
        .into_iter()
        .find(|code| contains_code(body, *code))
}

fn contains_code(body: &str, code: i64) -> bool {
    let needle = code.to_string();
    let bytes = body.as_bytes();
    let mut start = 0;
    while let Some(index) = body[start..].find(&needle) {
        let position = start + index;
        let before_ok = position == 0 || !bytes[position - 1].is_ascii_digit();
        let after = position + needle.len();
        let after_ok = after >= bytes.len() || !bytes[after].is_ascii_digit();
        if before_ok && after_ok {
            return true;
        }
        start = position + needle.len();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn realm_and_identity_come_from_options() {
        let cn = WorkbuddyChannel::from_options("wb-cn", &json!({"realm": "cn"}));
        assert_eq!(cn.realm, Realm::Cn);
        assert_eq!(cn.hosts.chat, "copilot.tencent.com");
        let intl = WorkbuddyChannel::from_options(
            "wb-intl",
            &json!({"realm": "intl", "outbound_identity": "vscode"}),
        );
        assert_eq!(intl.realm, Realm::Intl);
        assert_eq!(intl.outbound, Outbound::Vscode);
        assert_eq!(intl.hosts.chat, "www.workbuddy.cn");
        // 默认身份是桌面端，默认区域是国内版。
        let default = WorkbuddyChannel::from_options("wb", &json!({}));
        assert_eq!(default.outbound, Outbound::Desktop);
        assert_eq!(default.realm, Realm::Cn);
    }

    #[test]
    fn numeric_codes_are_translated_more_precisely_than_status_codes() {
        let ua = refine(400, r#"{"code":12403,"msg":"unknown user agent"}"#, "m");
        assert!(ua.message.contains("客户端标识"), "{}", ua.message);
        assert!(!ua.retryable, "配置问题重试没用");

        let tools = refine(400, "code 11148 tool call sequence broken", "m");
        assert!(tools.message.contains("工具调用序列断裂"), "{}", tools.message);

        let image = refine(400, "{\"code\":11135}", "m");
        assert!(image.message.contains("图片数据非法"), "{}", image.message);

        // 6004 是模型级限流，带 reset 时刻 —— 范围必须落到那个模型。
        let limited = refine(
            429,
            "code 6004 busy, your usage will reset at 1970-01-02 00:00:00 UTC+0",
            "deepseek-v4.1-flash",
        );
        assert_eq!(limited.kind, ErrorKind::RateLimited);
        assert_eq!(limited.reset_at_ms, Some(86_400_000));
        assert_eq!(
            limited.scope,
            ErrorScope::AccountModel("deepseek-v4.1-flash".into()),
            "6004 只停这一个模型，不是整个账号"
        );

        // 参考实现里 6004 的真实措辞：恢复时刻用 "your usage will reset at ... UTC+8"。
        let real = refine(
            429,
            r#"{"code":6004,"msg":"your usage exceeds frequency limit, your usage will reset at 2026-09-19 18:29:03 UTC+8"}"#,
            "deepseek-v4.1-flash",
        );
        assert_eq!(
            real.reset_at_ms,
            Some(1_789_813_743_000),
            "要能读懂上游给的恢复时刻（北京时间）"
        );

        // 真机实测报文（中文！我们发 Accept-Language: zh-CN，上游就把文案本地化了）：
        // 认死英文 "reset at" 会解析不出恢复时刻，退化成 10m/20m 退避 —— 而真实恢复在第二天。
        let real_cn = refine(
            429,
            r#"{"code":6004,"msg":"您的使用量已超出频率限制，将在 2026-10-11 14:46:40 UTC+8 重置，您也可以切换其他模型继续使用。","requestId":"a9d6fe19_000001"}"#,
            "deepseek-v4.1-flash",
        );
        assert_eq!(
            real_cn.reset_at_ms,
            Some(1_791_701_200_000),
            "中文文案里的恢复时刻也要能解析出来（2026-10-11 14:46:40 +08:00）"
        );
        assert!(
            real_cn.message.contains("后恢复"),
            "提示要带上剩余时间：{}",
            real_cn.message
        );
        assert!(!real_cn.message.contains("未知"), "{}", real_cn.message);

        // 没有 reset 时刻也要保持模型级（池子给模型退避，账号照用）。
        let bare = refine(429, r#"{"code":6004,"msg":"usage exceeds frequency limit"}"#, "glm-5.3");
        assert_eq!(bare.reset_at_ms, None);
        assert_eq!(bare.scope, ErrorScope::AccountModel("glm-5.3".into()));

        // 裸 429（没有 6004）= 账号级软限流，仍按账号退避（与参考实现一致）。
        let soft = refine(429, "too many requests", "glm-5.3");
        assert_eq!(soft.scope, ErrorScope::Account);
    }

    /// 分类后的错误必须**留着上游原文**：面板/请求日志/探针要能看到上游到底说了什么。
    /// （只留一句人话提示，等于出事时把线索丢掉。）
    #[test]
    fn classified_errors_keep_the_upstream_body() {
        let body = r#"{"code":6004,"msg":"usage exceeds frequency limit, your usage will reset at 1970-01-02 00:00:00 UTC+0"}"#;
        let error = refine(429, body, "deepseek-v4.1-flash");
        assert!(
            error.upstream.as_deref().is_some_and(|raw| raw.contains("6004")),
            "原文要留着"
        );
        let described = error.describe();
        assert!(described.contains("上游返回"), "{described}");
        assert!(described.contains("reset at"), "{described}");
        // 能算出恢复时刻时，提示里要写清楚（客户端据此决定等还是换模型）。
        assert!(error.message.contains("后恢复"), "{}", error.message);

        // 通用分类路径同样保留原文。
        let generic = refine(429, "slow down please", "glm-5.3");
        assert_eq!(generic.upstream.as_deref(), Some("slow down please"));
    }

    /// 11-128（渠道校验拦截）：**字符串码**，带连字符，数值扫描认不到。
    /// 真机 A/B：同一报文一次放行一次被拦、随后 16 连发 0 拦 —— 随机判定，
    /// 所以正确分类是"可重试"，而不是"请求不合法"（换号重试能过）。
    #[test]
    fn channel_block_11_128_is_retryable_request_scope() {
        let body = r#"{"code":"11-128","msg":"Illegal API invocation from an unapproved channel","displayTips":{"zh":"请求被拦截，请重新发送"}}"#;
        let error = refine(400, body, "glm-5.3");
        assert_eq!(error.kind, ErrorKind::ChannelBlocked);
        assert_eq!(error.scope, ErrorScope::Request);
        assert!(error.retryable, "重试（最好换号）就能过");
        assert!(error.upstream.as_deref().is_some_and(|raw| raw.contains("unapproved")));
        // 英文措辞也要认。
        let plain = refine(400, "Illegal API invocation from an unapproved channel", "glm-5.3");
        assert_eq!(plain.kind, ErrorKind::ChannelBlocked);
    }

    #[test]
    fn unknown_body_falls_back_to_status_semantics() {
        let error = refine(401, "unauthorized", "m");
        assert_eq!(error.kind, ErrorKind::AuthExpired);
        assert!(error.retryable);
    }

    #[test]
    fn code_matching_requires_token_boundaries() {
        // 正文里的 token 计数 124030 不该被当成 12403。
        assert!(known_code("code 12403").is_some());
        assert!(known_code("124030").is_none());
        assert!(known_code("tokens: 111335").is_none());
    }
}
