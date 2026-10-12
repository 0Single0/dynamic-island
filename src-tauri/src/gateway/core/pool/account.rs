//! 账号运行态、凭据与冷却策略。
//!
//! 运行态全在内存（重启即清空），可用性永远是**实时计算值**而不是落库状态。

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU32, AtomicU64, Ordering},
        Arc, RwLock,
    },
};

use crate::gateway::{
    channels::{AccountSnapshot, Credential, QuotaSnapshot},
    config::CooldownConfig,
    errors::{ErrorKind, ErrorScope, GatewayError},
    timeutil::next_local_4am_ms,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountKey {
    pub channel_id: String,
    pub uid: String,
}

impl AccountKey {
    pub fn new(channel_id: impl Into<String>, uid: impl Into<String>) -> Self {
        Self {
            channel_id: channel_id.into(),
            uid: uid.into(),
        }
    }
}

impl std::fmt::Display for AccountKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.channel_id, self.uid)
    }
}

#[derive(Debug, Default)]
pub struct AccountRuntime {
    /// 账号级冷却（401 / 无 reset 的 429）。
    cooldown_until: AtomicU64,
    /// 402 余额停放，到次日本地 04:00。
    balance_until: AtomicU64,
    /// 连续 5xx 熔断。
    breaker_until: AtomicU64,
    /// 连续失败计数（5xx 与网络抖动共用），成功清零。
    fails: AtomicU32,
    /// 账号级软限流连续次数，决定退避倍数。
    soft_streak: AtomicU32,
    /// 在飞请求数。
    inflight: AtomicU32,
    /// 模型级冷却（key = **上游**模型名）：同账号其他模型照用。
    model_cooldowns: RwLock<HashMap<String, u64>>,
    /// 模型级软限流连续次数（上游没给恢复时刻时，按模型各自退避）。
    model_streaks: RwLock<HashMap<String, u32>>,
    /// 刷新串行锁：上游会轮换 refresh_token，并发刷新互相作废。
    pub refresh_lock: tokio::sync::Mutex<()>,
}

#[derive(Debug)]
pub struct PoolAccount {
    pub key: AccountKey,
    pub nickname: String,
    pub enabled: bool,
    /// 该渠道是否必须有凭据才可用（假渠道不需要）。
    pub requires_credential: bool,
    credentials: RwLock<Credential>,
    /// 额度快照：渠道无关，面板按 label/unit 展示（渠道自行翻译口径）。
    quota: RwLock<Option<QuotaSnapshot>>,
    /// 越大越先被选中：成本档（免费 > 付费）。
    pub priority: u32,
    /// 同档内的容量份额。
    pub weight: u32,
    pub max_inflight: u32,
    pub proxy_slot: Option<String>,
    pub runtime: AccountRuntime,
}

impl PoolAccount {
    pub fn new(channel_id: impl Into<String>, uid: impl Into<String>) -> Self {
        Self {
            key: AccountKey::new(channel_id, uid),
            nickname: String::new(),
            enabled: true,
            requires_credential: true,
            credentials: RwLock::new(Credential::default()),
            quota: RwLock::new(None),
            priority: 100,
            weight: 1,
            max_inflight: 1,
            proxy_slot: None,
            runtime: AccountRuntime::default(),
        }
    }

    pub fn snapshot(&self) -> AccountSnapshot {
        let credentials = self.credentials();
        AccountSnapshot {
            channel_id: self.key.channel_id.clone(),
            uid: self.key.uid.clone(),
            access_token: (!credentials.access_token.is_empty())
                .then(|| credentials.access_token.clone()),
            refresh_token: credentials.refresh_token.clone(),
            enterprise_id: credentials.enterprise_id.clone(),
        }
    }

    pub fn credentials(&self) -> Credential {
        self.credentials
            .read()
            .map(|credentials| credentials.clone())
            .unwrap_or_default()
    }

    pub fn quota(&self) -> Option<QuotaSnapshot> {
        self.quota
            .read()
            .ok()
            .and_then(|quota| quota.clone())
    }

    pub fn set_quota(&self, quota: Option<QuotaSnapshot>) {
        if let Ok(mut slot) = self.quota.write() {
            *slot = quota;
        }
    }

    /// 落库/刷新成功后整体替换凭据（refresh_token 会轮换，不能只换一半）。
    pub fn set_credentials(&self, credential: Credential) {
        if let Ok(mut slot) = self.credentials.write() {
            *slot = credential;
        }
    }

    /// 实时可用性：启用 ∧ 有凭据（若该渠道需要）∧ 不在冷却/停放/熔断窗口 ∧ 该模型没被单独冷掉。
    pub fn is_ready(&self, model: &str, now: u64) -> bool {
        if !self.enabled {
            return false;
        }
        if self.requires_credential && self.credentials().access_token.is_empty() {
            return false;
        }
        let blocked = self.runtime.cooldown_until.load(Ordering::SeqCst)
            | self.runtime.balance_until.load(Ordering::SeqCst)
            | self.runtime.breaker_until.load(Ordering::SeqCst);
        if now < blocked {
            return false;
        }
        if self
            .model_cooldowns()
            .get(model)
            .is_some_and(|until| now < *until)
        {
            return false;
        }
        true
    }

    /// 取一个在飞槽位。返回 None = 该账号并发已满（调度器应换号或让请求排队）。
    pub fn try_acquire(self: &Arc<Self>) -> Option<InflightGuard> {
        if self.runtime.inflight.load(Ordering::SeqCst) >= self.max_inflight {
            return None;
        }
        // 先加再看：避免并发下两个请求同时通过上限判断。
        if self.runtime.inflight.fetch_add(1, Ordering::SeqCst) >= self.max_inflight {
            self.runtime.inflight.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(InflightGuard {
            account: Arc::clone(self),
        })
    }

    /// 按统一语义落地冷却。策略参数见计划 §6.4，全部来自配置。
    pub fn apply_error(&self, error: &GatewayError, policy: &CooldownConfig, now: u64) {
        match error.kind {
            ErrorKind::AuthExpired => {
                self.raise_cooldown(&self.runtime.cooldown_until, now, policy.auth_secs);
            }
            ErrorKind::RateLimited => {
                // 三种形态，区别只在**范围**与**停多久**：
                // · 渠道说是模型级（AccountModel）：只停那个模型，同账号其他模型照用 ——
                //   上游给了恢复时刻就停到那一刻，没给就按模型各自的退避（不牵连账号）；
                // · 账号级但给了恢复时刻：停整个账号到那一刻；
                // · 账号级没给时刻：账号级指数退避（软限流，最保守）。
                let model = match &error.scope {
                    ErrorScope::AccountModel(model) => Some(model.clone()),
                    _ => None,
                };
                match (model, error.reset_at_ms) {
                    (Some(model), Some(reset_at)) => self.park_model(&model, reset_at),
                    (Some(model), None) => {
                        let streak = self.bump_model_streak(&model);
                        let wait = backoff_secs(
                            policy.soft_base_secs,
                            streak - 1,
                            policy.soft_cap_secs,
                        );
                        self.park_model(&model, now.saturating_add(wait * 1000));
                    }
                    (None, Some(reset_at)) => {
                        // 上游说了恢复时刻但没说哪个模型：按它停整个账号。
                        if reset_at > self.runtime.cooldown_until.load(Ordering::SeqCst) {
                            self.runtime.cooldown_until.store(reset_at, Ordering::SeqCst);
                        }
                    }
                    (None, None) => {
                        let streak = self.runtime.soft_streak.fetch_add(1, Ordering::SeqCst) + 1;
                        let wait =
                            backoff_secs(policy.soft_base_secs, streak - 1, policy.soft_cap_secs);
                        self.raise_cooldown(&self.runtime.cooldown_until, now, wait);
                    }
                }
            }
            // 上游渠道校验拦截：随机出现、重试即可 —— 不动任何冷却，
            // 否则"随机拦一次"会把一个健康账号白停十分钟。
            ErrorKind::ChannelBlocked => {}
            ErrorKind::QuotaExhausted => {
                self.runtime
                    .balance_until
                    .store(next_local_4am_ms(now), Ordering::SeqCst);
            }
            ErrorKind::Upstream5xx | ErrorKind::Network => {
                let fails = self.runtime.fails.fetch_add(1, Ordering::SeqCst) + 1;
                if fails >= policy.breaker_after {
                    let wait = backoff_secs(
                        policy.breaker_base_secs,
                        fails - policy.breaker_after,
                        policy.breaker_cap_secs,
                    );
                    let until = now.saturating_add(wait * 1000);
                    if until > self.runtime.breaker_until.load(Ordering::SeqCst) {
                        self.runtime.breaker_until.store(until, Ordering::SeqCst);
                    }
                }
            }
            // 内容审核换号也没用（原号降级重试在提示词管道里做）；
            // 请求类错误不改账号状态。
            ErrorKind::ContentPolicy
            | ErrorKind::InvalidRequest
            | ErrorKind::ModelNotFound
            | ErrorKind::NoAccount
            | ErrorKind::Unsupported
            | ErrorKind::Internal => {}
        }
    }

    /// 重建账号对象时把**运行态**搬过来：额度、冷却、熔断、失败计数。
    ///
    /// 为什么必须搬：面板改账号（启停、改优先级）走的是"重建整池"，
    /// 不搬就等于每次编辑都把全池的冷却和熔断清空一次 —— 保护机制形同虚设，
    /// 额度也会显示成空。
    ///
    /// 在飞数**不搬**：在途请求的守卫持有的是旧对象，搬过来只会让计数永久偏高。
    pub fn carry_runtime_from(&self, previous: &PoolAccount) {
        let load = |slot: &AtomicU64| slot.load(Ordering::SeqCst);
        self.runtime
            .cooldown_until
            .store(load(&previous.runtime.cooldown_until), Ordering::SeqCst);
        self.runtime
            .balance_until
            .store(load(&previous.runtime.balance_until), Ordering::SeqCst);
        self.runtime
            .breaker_until
            .store(load(&previous.runtime.breaker_until), Ordering::SeqCst);
        self.runtime
            .fails
            .store(previous.runtime.fails.load(Ordering::SeqCst), Ordering::SeqCst);
        self.runtime.soft_streak.store(
            previous.runtime.soft_streak.load(Ordering::SeqCst),
            Ordering::SeqCst,
        );
        if let (Ok(source), Ok(mut target)) = (
            previous.runtime.model_cooldowns.read(),
            self.runtime.model_cooldowns.write(),
        ) {
            *target = source.clone();
        }
        if let (Ok(source), Ok(mut target)) = (
            previous.runtime.model_streaks.read(),
            self.runtime.model_streaks.write(),
        ) {
            *target = source.clone();
        }
        if let Some(quota) = previous.quota() {
            self.set_quota(Some(quota));
        }
    }

    /// 清空全部惩罚状态（面板"重置状态"按钮）。
    ///
    /// 只清运行态：凭据、额度、启停都不动。
    pub fn reset_runtime(&self) {
        self.runtime.cooldown_until.store(0, Ordering::SeqCst);
        self.runtime.balance_until.store(0, Ordering::SeqCst);
        self.runtime.breaker_until.store(0, Ordering::SeqCst);
        self.runtime.fails.store(0, Ordering::SeqCst);
        self.runtime.soft_streak.store(0, Ordering::SeqCst);
        if let Ok(mut map) = self.runtime.model_cooldowns.write() {
            map.clear();
        }
        if let Ok(mut streaks) = self.runtime.model_streaks.write() {
            streaks.clear();
        }
    }

    /// 任一次成功都要把该账号的惩罚状态清空。
    pub fn on_success(&self, model: &str) {
        self.runtime.cooldown_until.store(0, Ordering::SeqCst);
        self.runtime.breaker_until.store(0, Ordering::SeqCst);
        self.runtime.fails.store(0, Ordering::SeqCst);
        self.runtime.soft_streak.store(0, Ordering::SeqCst);
        if let Ok(mut map) = self.runtime.model_cooldowns.write() {
            map.remove(model);
        }
        if let Ok(mut streaks) = self.runtime.model_streaks.write() {
            streaks.remove(model);
        }
    }

    /// 记住"某个模型被限流到什么时候"（同账号其他模型不受影响）。
    fn park_model(&self, model: &str, until: u64) {
        if let Ok(mut map) = self.runtime.model_cooldowns.write() {
            let entry = map.entry(model.to_string()).or_insert(0);
            // 只延长不缩短：并发下短冷却不能把长冷却覆盖掉。
            if until > *entry {
                *entry = until;
            }
        }
    }

    /// 模型级软限流计数 +1，返回当前值（决定退避倍数）。
    fn bump_model_streak(&self, model: &str) -> u32 {
        let mut streaks = match self.runtime.model_streaks.write() {
            Ok(streaks) => streaks,
            Err(_) => return 1,
        };
        let streak = streaks.entry(model.to_string()).or_insert(0);
        *streak = streak.saturating_add(1);
        *streak
    }

    /// 该账号当前**为什么不可用**（面板状态旁 `?` 的 tooltip 内容）。
    ///
    /// `model` 用于检查**模型级**冷却：上游限流常常只针对单个模型，
    /// 不传就会漏掉最常见的那种"用不了"。
    /// 可用时返回空 —— 不给用户看无意义的信息。
    pub fn blocked_reasons(&self, model: &str, now: u64) -> Vec<String> {
        let mut reasons = Vec::new();
        if !self.enabled {
            reasons.push("已停用：不会参与调度".to_string());
        }
        if self.requires_credential && self.credentials().access_token.is_empty() {
            reasons.push("没有凭据：该渠道需要先完成登录".to_string());
        }
        if let Some(until) = self.runtime_balance_after(now) {
            reasons.push(format!(
                "余额/额度不足被暂停，{} 后自动恢复",
                humanize(until - now)
            ));
        }
        if let Some(until) = self.runtime_breaker_after(now) {
            reasons.push(format!(
                "连续失败触发熔断，{} 后自动恢复（成功一次会立刻解除）",
                humanize(until - now)
            ));
        }
        if let Some(until) = self.runtime_cooldown_after(now) {
            reasons.push(format!(
                "被上游拒绝或限流，冷却 {} 后自动恢复",
                humanize(until - now)
            ));
        }
        let cooldowns = self.model_cooldowns();
        let mut limited: Vec<(&String, &u64)> = cooldowns
            .iter()
            .filter(|(name, until)| **until > now && (model.is_empty() || *name == model))
            .collect();
        limited.sort_by(|a, b| a.1.cmp(b.1));
        for (name, until) in limited {
            reasons.push(format!(
                "模型 {name} 被上游限流，{} 后恢复（同账号其他模型不受影响）",
                humanize(*until - now)
            ));
        }
        reasons
    }

    fn runtime_cooldown_after(&self, now: u64) -> Option<u64> {
        let value = self.runtime.cooldown_until.load(Ordering::SeqCst);
        (value > now).then_some(value)
    }

    fn runtime_breaker_after(&self, now: u64) -> Option<u64> {
        let value = self.runtime.breaker_until.load(Ordering::SeqCst);
        (value > now).then_some(value)
    }

    fn runtime_balance_after(&self, now: u64) -> Option<u64> {
        let value = self.runtime.balance_until.load(Ordering::SeqCst);
        (value > now).then_some(value)
    }

    pub fn cooldown_until_ms(&self) -> u64 {
        self.runtime
            .cooldown_until
            .load(Ordering::SeqCst)
            .max(self.runtime.balance_until.load(Ordering::SeqCst))
            .max(self.runtime.breaker_until.load(Ordering::SeqCst))
    }

    pub fn inflight(&self) -> u32 {
        self.runtime.inflight.load(Ordering::SeqCst)
    }

    /// 该账号在飞是否已达上限（调度器据此换号，而不是排队等它）。
    pub fn inflight_full(&self) -> bool {
        self.inflight() >= self.max_inflight
    }

    fn model_cooldowns(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, u64>> {
        self.runtime
            .model_cooldowns
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn raise_cooldown(&self, slot: &AtomicU64, now: u64, secs: u64) {
        let until = now.saturating_add(secs.saturating_mul(1000));
        if until > slot.load(Ordering::SeqCst) {
            slot.store(until, Ordering::SeqCst);
        }
    }
}

/// 把毫秒差说成人话：38s / 6m / 2h。
pub fn humanize(delta_ms: u64) -> String {
    let seconds = delta_ms / 1000;
    if seconds < 60 {
        return format!("{seconds} 秒");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes} 分钟");
    }
    format!("{} 小时", minutes / 60)
}

/// 指数退避：`base · 2^exponent`，封顶 `cap`。
fn backoff_secs(base: u64, exponent: u32, cap: u64) -> u64 {
    let exponent = exponent.min(16);
    base.saturating_mul(1u64 << exponent).min(cap)
}

/// 在飞槽位守卫：请求结束（含客户端断连导致 future 被 drop）时自动归还。
pub struct InflightGuard {
    account: Arc<PoolAccount>,
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.account.runtime.inflight.fetch_sub(1, Ordering::SeqCst);
    }
}

/// 单个模型的限流窗口（面板"限流等待"列 / 状态 `?` 用）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelLimit {
    pub model: String,
    pub until_ms: u64,
}

/// 供测试与面板使用的账号快照视图。
#[derive(Debug, Clone, serde::Serialize)]
pub struct AccountView {
    pub channel_id: String,
    pub uid: String,
    pub nickname: String,
    pub enabled: bool,
    pub ready: bool,
    pub has_credential: bool,
    pub credential_expires_at_ms: Option<u64>,
    pub cooldown_until_ms: u64,
    /// 不可用原因（冷却 / 熔断 / 额度停放 / 没凭据 / 停用），供面板 tooltip。
    pub blocked_reasons: Vec<String>,
    /// 模型级限流（key = 上游模型名）。账号本身还是可用的，只是这几个模型要等，
    /// 所以单独给面板一份，别和"账号不可用"混在一起。
    pub model_limits: Vec<ModelLimit>,
    /// 当前进行中的请求数。
    pub inflight: u32,
    /// 该账号的并发上限（面板显示"进行中 / 上限"）。
    pub max_inflight: u32,
    pub priority: u32,
    pub weight: u32,
    pub proxy_slot: Option<String>,
    /// 额度快照（渠道自行决定口径；没有额度概念的渠道是 None）。
    pub quota: Option<QuotaSnapshot>,
}

impl PoolAccount {
    pub fn view(&self, now: u64) -> AccountView {
        let credentials = self.credentials();
        AccountView {
            channel_id: self.key.channel_id.clone(),
            uid: self.key.uid.clone(),
            nickname: self.nickname.clone(),
            enabled: self.enabled,
            ready: self.is_ready("", now),
            has_credential: !credentials.access_token.is_empty(),
            credential_expires_at_ms: credentials.expires_at_ms,
            cooldown_until_ms: self.cooldown_until_ms(),
            blocked_reasons: self.blocked_reasons("", now),
            model_limits: {
                let mut limits: Vec<ModelLimit> = self
                    .model_cooldowns()
                    .iter()
                    .filter(|(_, until)| **until > now)
                    .map(|(model, until)| ModelLimit {
                        model: model.clone(),
                        until_ms: *until,
                    })
                    .collect();
                limits.sort_by(|a, b| a.until_ms.cmp(&b.until_ms));
                limits
            },
            inflight: self.inflight(),
            max_inflight: self.max_inflight,
            priority: self.priority,
            weight: self.weight,
            proxy_slot: self.proxy_slot.clone(),
            quota: self.quota(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::{channels::Credential, timeutil::now_ms};

    fn account() -> Arc<PoolAccount> {
        let mut account = PoolAccount::new("fake", "u1");
        account.nickname = "测试账号".into();
        account.requires_credential = false;
        account.max_inflight = 1;
        Arc::new(account)
    }

    #[test]
    fn missing_credential_blocks_only_channels_that_need_one() {
        let mut needs = PoolAccount::new("wb-cn", "u1");
        needs.enabled = true;
        assert!(!needs.is_ready("glm-5.3", now_ms()), "真实渠道没凭据不可用");
        needs.set_credentials(Credential {
            access_token: "token".into(),
            ..Credential::default()
        });
        assert!(needs.is_ready("glm-5.3", now_ms()));

        let open = {
            let mut account = PoolAccount::new("fake", "u1");
            account.requires_credential = false;
            account
        };
        assert!(open.is_ready("fake-model", now_ms()), "假渠道不需要凭据");
    }

    #[test]
    fn auth_failure_cools_account_for_the_configured_window() {
        let account = account();
        let policy = CooldownConfig::default();
        let now = now_ms();
        account.apply_error(
            &GatewayError::new(ErrorKind::AuthExpired, ErrorScope::Account, 401, "x", true),
            &policy,
            now,
        );
        assert!(!account.is_ready("m", now));
        assert!(account.is_ready("m", now + (policy.auth_secs + 1) * 1000));
    }

    /// 模型限额是本渠道的常态（workbuddy 的 6004）：一个模型被限，账号必须照常服务其他模型。
    #[test]
    fn model_scoped_limit_leaves_sibling_models_serviceable() {
        let account = account();
        let policy = CooldownConfig::default();
        let now = now_ms();
        account.apply_error(
            &GatewayError::new(
                ErrorKind::RateLimited,
                ErrorScope::AccountModel("deepseek-v4.1-flash".into()),
                429,
                "该模型被上游限流",
                true,
            ),
            &policy,
            now,
        );
        assert!(!account.is_ready("deepseek-v4.1-flash", now), "被限的模型不可用");
        assert!(account.is_ready("glm-5.3", now), "同账号其他模型照常可用");
        assert_eq!(account.cooldown_until_ms(), 0, "不该把整个账号拉黑");
        let reasons = account.blocked_reasons("", now);
        assert!(
            reasons.iter().any(|reason| reason.contains("deepseek-v4.1-flash")),
            "面板要能看到是哪个模型被限：{reasons:?}"
        );
    }

    /// 拿不到恢复时刻也要停在**模型**上：不能因为解析不出时间就停整个账号。
    #[test]
    fn model_scoped_limit_without_reset_still_only_cools_that_model() {
        let account = account();
        let policy = CooldownConfig::default();
        let now = now_ms();
        account.apply_error(
            &GatewayError::new(
                ErrorKind::RateLimited,
                ErrorScope::AccountModel("glm-5.3".into()),
                429,
                "该模型被上游限流",
                true,
            ),
            &policy,
            now,
        );
        assert!(!account.is_ready("glm-5.3", now));
        assert!(account.is_ready("deepseek-v4.1-flash", now));
        assert_eq!(account.cooldown_until_ms(), 0);
        // 退避窗口有效：到点自己恢复。
        assert!(account.is_ready("glm-5.3", now + (policy.soft_base_secs + 1) * 1000));
    }

    /// 上游说了恢复时刻但没说是哪个模型 → 按那个时刻停整个账号（旧行为是静默 no-op）。
    #[test]
    fn account_level_limit_with_reset_cools_until_that_moment() {
        let account = account();
        let now = now_ms();
        account.apply_error(
            &GatewayError::new(ErrorKind::RateLimited, ErrorScope::Account, 429, "x", true)
                .with_reset_at(Some(now + 120_000)),
            &CooldownConfig::default(),
            now,
        );
        assert!(!account.is_ready("glm-5.3", now));
        assert_eq!(account.cooldown_until_ms(), now + 120_000, "停到上游说的时刻");
    }

    #[test]
    fn rate_limit_with_reset_only_cools_that_model() {
        let account = account();
        let policy = CooldownConfig::default();
        let now = now_ms();
        let error = GatewayError::new(
            ErrorKind::RateLimited,
            ErrorScope::AccountModel("glm-5.3".into()),
            429,
            "x",
            true,
        )
        .with_reset_at(Some(now + 60_000));
        account.apply_error(&error, &policy, now);
        assert!(!account.is_ready("glm-5.3", now));
        assert!(account.is_ready("hy3", now), "其他模型不该被连坐");
        assert!(account.is_ready("glm-5.3", now + 61_000));
    }

    #[test]
    fn soft_rate_limit_backs_off_exponentially_and_then_clears() {
        let account = account();
        let policy = CooldownConfig::default();
        let now = now_ms();
        let error = || {
            GatewayError::new(ErrorKind::RateLimited, ErrorScope::Account, 429, "x", true)
        };
        account.apply_error(&error(), &policy, now);
        let first = account.cooldown_until_ms() - now;
        account.runtime.cooldown_until.store(0, Ordering::SeqCst);
        account.runtime.soft_streak.store(1, Ordering::SeqCst);
        account.apply_error(&error(), &policy, now);
        let second = account.cooldown_until_ms() - now;
        assert_eq!(first, policy.soft_base_secs * 1000);
        assert_eq!(second, policy.soft_base_secs * 2000);

        account.on_success("m");
        assert_eq!(account.cooldown_until_ms(), 0);
        assert!(account.is_ready("m", now));
    }

    #[test]
    fn quota_exhaustion_parks_until_next_local_4am() {
        let account = account();
        let policy = CooldownConfig::default();
        let now = now_ms();
        account.apply_error(
            &GatewayError::new(ErrorKind::QuotaExhausted, ErrorScope::Account, 402, "x", true),
            &policy,
            now,
        );
        let until = account.cooldown_until_ms();
        assert!(until > now);
        assert!(until <= now + 25 * 3600 * 1000);
    }

    #[test]
    fn repeated_5xx_trips_the_breaker_only_after_the_threshold() {
        let account = account();
        let policy = CooldownConfig::default();
        let now = now_ms();
        let error = || GatewayError::upstream(503, "boom");
        for _ in 0..policy.breaker_after - 1 {
            account.apply_error(&error(), &policy, now);
            assert_eq!(account.runtime.breaker_until.load(Ordering::SeqCst), 0);
        }
        account.apply_error(&error(), &policy, now);
        assert!(account.runtime.breaker_until.load(Ordering::SeqCst) > now);
    }

    #[test]
    fn inflight_slot_is_released_by_the_guard() {
        let account = account();
        let guard = account.try_acquire().expect("有空位");
        assert!(account.try_acquire().is_none(), "上限 1 时第二次必须失败");
        drop(guard);
        assert!(account.try_acquire().is_some(), "释放后应当能再取");
    }

    #[test]
    fn backlog_is_capped() {
        assert_eq!(backoff_secs(600, 0, 7200), 600);
        assert_eq!(backoff_secs(600, 3, 7200), 4800);
        assert_eq!(backoff_secs(600, 10, 7200), 7200);
    }
}
