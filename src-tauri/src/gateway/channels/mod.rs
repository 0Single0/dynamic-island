//! 渠道适配层：把某个上游订阅服务封装成 [`ChannelAdapter`]。
//!
//! `channels` **不得引用 `core`**：适配器不认识调度器、冷却、亲和，
//! 它只负责「怎么登录 / 怎么刷新 / 怎么发一次请求 / 怎么读回事件 / 这个错误是什么意思」。
//! 新增渠道只改本模块 + 配置。

pub mod fake;
pub mod openai_compat;
pub mod workbuddy;

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;

use crate::gateway::{
    config::Config,
    errors::GatewayError,
    protocol::cir::{CanonicalEvent, CanonicalRequest},
    transport::{self, FrameStream, UpstreamRequest},
};

/// 渠道看到的最小账号视图：只有出站需要的字段，拿不到运行态（冷却/在飞）。
#[derive(Debug, Clone, Default)]
pub struct AccountSnapshot {
    pub channel_id: String,
    pub uid: String,
    pub access_token: Option<String>,
    /// 刷新凭据要用；上游每次刷新都会轮换它。
    pub refresh_token: Option<String>,
    pub enterprise_id: Option<String>,
}

/// 落库的凭据。刷新会轮换 refresh_token，所以整块一起换。
///
/// 需要序列化：它以 JSON 形式装进加密信封再进 SQLite。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Credential {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at_ms: Option<u64>,
    pub uid: Option<String>,
    pub nickname: Option<String>,
    pub enterprise_id: Option<String>,
    pub domain: Option<String>,
}

impl Credential {
    /// 剩余寿命不足 `margin_ms` 就要刷新（选号时的安全边距）。
    pub fn is_expiring(&self, now_ms: u64, margin_ms: u64) -> bool {
        match self.expires_at_ms {
            Some(expires_at) => expires_at.saturating_sub(now_ms) <= margin_ms,
            // 上游没给过期时间时按"需要刷新"处理：宁可多刷一次，不能拿死 token 打上游。
            None => true,
        }
    }
}

/// 渠道模型目录里的一条。能力/长度用于矩阵的候选过滤。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct UpstreamModel {
    pub id: String,
    /// 倍率字符串：`x0.00` / `0` 视为免费；缺失表示未知（按收费处理）。
    pub credits: Option<String>,
    pub context_length: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub vendor: Option<String>,
    pub capabilities: ModelCapabilities,
    /// 上下文档位（上游 `contextWindow.supportedLengths`）。
    pub context_windows: Vec<u64>,
    /// 思考档位：可选档位 / 固定档位 / 默认档 / 能否关闭 / 是否强制思考。
    pub reasoning_efforts: Vec<String>,
    pub reasoning_fixed_effort: Option<String>,
    pub reasoning_default_effort: Option<String>,
    pub reasoning_can_disable: Option<bool>,
    pub always_reasoning: bool,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    /// 这条来自内置快照（而非实时目录）：能力字段与"现在是免费的"都可能过期，
    /// 所以矩阵会把它标成 Builtin 来源，并且不拿它的能力去做硬性过滤。
    pub from_snapshot: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ModelCapabilities {
    pub vision: bool,
    pub tool_calls: bool,
    pub reasoning: bool,
}

/// 渠道无关的额度快照。
///
/// 各渠道口径天然不同（workbuddy 是积分/资源包，将来 qoder 可能是额度、
/// kiro 可能是试用期或请求数），所以这里只做**归一**，不做解释：
/// 面板按 `label` + `unit` 展示，渠道特有的补充信息放 `details` 原样透出。
/// 新渠道接入时只要实现 `ChannelAdapter::fetch_quota`，面板不用改。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct QuotaSnapshot {
    /// 主口径名称：积分 / 额度 / 余额 / 请求数 …
    pub label: String,
    /// 单位：点 / 元 / 次 / token …
    pub unit: String,
    /// 剩余量。
    pub remaining: Option<f64>,
    /// 周期总量（拿不到就不给）。
    pub total: Option<f64>,
    /// 已用量（拿不到就不给）。
    pub used: Option<f64>,
    /// 渠道特有的补充行，面板按原样展示（键值对）。
    pub details: Vec<(String, String)>,
    /// 抓取时间（epoch ms）：面板据此显示"多久之前"。
    pub fetched_at_ms: u64,
}

/// 登录会话票据：state 由适配器保管在上游，authUrl 交给用户浏览器打开。
#[derive(Debug, Clone)]
pub struct LoginTicket {
    pub state: String,
    pub auth_url: String,
}

/// 登录轮询结果。
#[derive(Debug, Clone)]
pub enum LoginPoll {
    /// 用户还没完成授权（上游 pending 码）。
    Pending,
    /// 授权完成，凭据可以落库。
    Ready(Box<Credential>),
    /// 上游给了明确失败。
    Failed(String),
}

#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// 渠道实例 id，与 `ChannelConfig::id` 一致。
    fn id(&self) -> &str;
    /// 适配器类型（`workbuddy` / `fake` …），用于面板显示。
    fn adapter_kind(&self) -> &str;

    /// 该渠道的账号是否必须有凭据才可用（假渠道不需要，用来空手验证链路）。
    fn requires_credential(&self) -> bool {
        true
    }

    /// 构建上游报文。纯函数，不碰网络 —— 它是渠道层最值得单测的部分。
    fn build_chat_request(
        &self,
        account: &AccountSnapshot,
        request: &CanonicalRequest,
    ) -> Result<UpstreamRequest, GatewayError>;

    /// 一帧上游 SSE → 若干 CIR 事件。未知帧返回空 vec（不因为多一个字段就中断整条流）。
    fn parse_event(&self, frame: &str) -> Vec<CanonicalEvent>;

    /// 把上游状态码/错误码翻译成统一语义。调度器只认 [`GatewayError::kind`] 与 `scope`。
    fn classify_error(&self, status: u16, body: &str) -> GatewayError;

    /// 拉模型目录。`Ok(None)` 表示"拉取失败，请回落快照"，**不是**"这个渠道没有模型"。
    async fn list_models(
        &self,
        client: &reqwest::Client,
        account: &AccountSnapshot,
    ) -> Result<Option<Vec<UpstreamModel>>, GatewayError>;

    /// 默认传输链路：构建报文 → HTTP → SSE 帧流。就地吐帧的渠道（假渠道）覆盖它。
    async fn chat_stream(
        &self,
        account: &AccountSnapshot,
        request: &CanonicalRequest,
        client: &reqwest::Client,
    ) -> Result<FrameStream, GatewayError> {
        let upstream = self.build_chat_request(account, request)?;
        match transport::send(&upstream, client).await {
            Ok(response) => Ok(transport::frame_stream(response, STREAM_IDLE_TIMEOUT)),
            Err(transport::UpstreamFailure { status: 0, body }) => {
                Err(GatewayError::network(body))
            }
            Err(transport::UpstreamFailure { status, body }) => {
                Err(self.classify_error(status, &body))
            }
        }
    }

    /// 刷新凭据。`Ok(None)` = 该渠道不需要刷新（或没有 refresh_token）。
    ///
    /// **调用方必须按账号串行调用**：上游会轮换 refresh_token，并发刷新互相作废。
    async fn refresh(
        &self,
        _client: &reqwest::Client,
        _account: &AccountSnapshot,
    ) -> Result<Option<Credential>, GatewayError> {
        Ok(None)
    }

    /// 开始一次登录：拿到 state 与给用户打开的授权链接。
    async fn login_start(
        &self,
        _client: &reqwest::Client,
        _platform: &str,
    ) -> Result<LoginTicket, GatewayError> {
        Err(GatewayError::unsupported("该渠道不支持登录流程，请手工填写凭据"))
    }

    /// 轮询登录结果。面板每 2.5 秒调一次。
    async fn login_poll(
        &self,
        _client: &reqwest::Client,
        _state: &str,
    ) -> Result<LoginPoll, GatewayError> {
        Err(GatewayError::unsupported("该渠道不支持登录流程"))
    }

    /// 额度/余额快照。`Ok(None)` = 该渠道没有额度概念，或这次没查到。
    ///
    /// 面板只消费归一后的 [`QuotaSnapshot`]，所以新渠道接额度不需要动界面。
    async fn fetch_quota(
        &self,
        _client: &reqwest::Client,
        _account: &AccountSnapshot,
    ) -> Result<Option<QuotaSnapshot>, GatewayError> {
        Ok(None)
    }

    /// 运营类动作：每日签到（国内版积分）。返回是否算成功（含幂等）。
    async fn sign_in(
        &self,
        _client: &reqwest::Client,
        _account: &AccountSnapshot,
    ) -> Result<bool, GatewayError> {
        Ok(false)
    }
}

/// 上游目录里哪些 id 算「可对外发布的聊天模型」。
///
/// 这些命名约定在该上游家族里是共通的：`lite`/`auto` 等是内部虚拟模型与 picker 别名，
/// `codewise-*`/`completion-*` 是 IDE 补全通道，带 `-image-alpha*`/`-taco-completion`
/// 后缀的不是通用对话模型。矩阵构建与渠道目录清洗共用同一套规则。
pub fn is_chat_model(id: &str) -> bool {
    const NON_CHAT: [&str; 7] = [
        "lite",
        "default-model",
        "fast-model",
        "balanced-model",
        "primary-model",
        "deep-model",
        "auto",
    ];
    const NON_CHAT_PREFIXES: [&str; 2] = ["codewise-", "completion-"];
    const NON_CHAT_SUFFIXES: [&str; 3] = ["-image-alpha", "-image-alpha-edit", "-taco-completion"];

    !id.is_empty()
        && !NON_CHAT.contains(&id)
        && !NON_CHAT_PREFIXES.iter().any(|prefix| id.starts_with(prefix))
        && !NON_CHAT_SUFFIXES.iter().any(|suffix| id.ends_with(suffix))
}

/// 倍率字符串判定免费：`x0.00` / `x0` / `0` / `0.00`。
/// 缺失或解析不了**不算免费**（继续受额度守卫约束）。
pub fn credits_is_free(credits: Option<&str>) -> bool {
    matches!(
        credits.map(|value| value.trim().to_ascii_lowercase()),
        Some(ref value) if matches!(value.as_str(), "x0.00" | "x0" | "0" | "0.00")
    )
}

/// 上游连上后允许的空闲时间（参考实现：流中 300 秒无数据即视为断流）。
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// 渠道注册中心：配置里每个渠道实例化一个适配器。
///
/// 未知适配器只记警告并跳过 —— 配置写错不该让整个岛起不来。
pub fn build_registry(
    config: &Config,
    warnings: &mut Vec<String>,
) -> HashMap<String, Arc<dyn ChannelAdapter>> {
    let mut registry: HashMap<String, Arc<dyn ChannelAdapter>> = HashMap::new();
    for channel in &config.channels {
        let adapter: Arc<dyn ChannelAdapter> = match channel.adapter.as_str() {
            "fake" => Arc::new(fake::FakeChannel::from_options(&channel.id, &channel.options)),
            "workbuddy" => Arc::new(workbuddy::WorkbuddyChannel::from_options(
                &channel.id,
                &channel.options,
            )),
            other => {
                warnings.push(format!(
                    "渠道 {} 的适配器 {other} 尚未实现，已跳过",
                    channel.id
                ));
                continue;
            }
        };
        registry.insert(channel.id.clone(), adapter);
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::config::ChannelConfig;

    #[test]
    fn unknown_adapter_is_skipped_with_a_warning() {
        let mut config = Config::default();
        // 默认配置自带渠道；这个测试只关心"未知适配器被跳过"。
        config.channels.clear();
        config.channels.push(ChannelConfig {
            id: "qoder".into(),
            adapter: "qoder".into(),
            ..ChannelConfig::default()
        });
        config.channels.push(ChannelConfig {
            id: "fake".into(),
            adapter: "fake".into(),
            ..ChannelConfig::default()
        });
        config.channels.push(ChannelConfig {
            id: "wb-cn".into(),
            adapter: "workbuddy".into(),
            ..ChannelConfig::default()
        });
        let mut warnings = Vec::new();
        let registry = build_registry(&config, &mut warnings);
        assert_eq!(registry.len(), 2);
        assert!(registry.contains_key("fake"));
        assert!(registry.contains_key("wb-cn"));
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn credential_expiry_margin_is_enforced() {
        let credential = Credential {
            access_token: "t".into(),
            expires_at_ms: Some(1_000_000),
            ..Credential::default()
        };
        // 剩 5 分钟 > 2 分钟边距：还不刷。
        assert!(!credential.is_expiring(1_000_000 - 300_000, 120_000));
        // 剩 60 秒 < 2 分钟边距：要刷。
        assert!(credential.is_expiring(1_000_000 - 60_000, 120_000));
        // 没有过期时间：按需要刷新处理。
        assert!(Credential::default().is_expiring(0, 120_000));
    }
}
