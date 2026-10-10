//! 调度：候选矩阵 → 亲和短路 → 分层打分选号 → 重试排除 → 冷却落地 → 并发槽。
//!
//! 这一层是「多号池」与「多渠道」的汇合点：候选来自矩阵，挑选只认账号运行态与统一错误语义。

use std::{collections::HashSet, sync::Arc};

use uuid::Uuid;

use crate::gateway::{
    channels::ChannelAdapter,
    core::{
        affinity::{AffinityMode, AffinityStore, Binding},
        matrix::Requirements,
        pool::{AccountKey, InflightGuard, PoolAccount},
        refresh,
        router::{self, Candidate, Selection},
        usage::estimate_request_tokens,
    },
    errors::{ErrorKind, ErrorScope, GatewayError},
    protocol::cir::CanonicalRequest,
    state::GatewayState,
    timeutil::now_ms,
    transport::FrameStream,
};

/// 一次已经连上上游的会话。持有账号在飞槽位，`drop` 即归还。
pub struct UpstreamSession {
    pub selection: Selection,
    pub adapter: Arc<dyn ChannelAdapter>,
    pub frames: FrameStream,
    pub attempts: usize,
    pub trace_id: String,
    /// 本次请求的亲和键（成功后写回/续期）。
    pub affinity_key: Option<String>,
    _guard: InflightGuard,
}

impl UpstreamSession {
    pub fn account(&self) -> &Arc<PoolAccount> {
        &self.selection.account
    }
}

/// 选号并连上上游；失败自动换号重试，全部失败返回最后一次的错误。
pub async fn open_chat_session(
    state: &GatewayState,
    request: &CanonicalRequest,
    scope: &str,
) -> Result<UpstreamSession, GatewayError> {
    let trace_id = short_id();

    // ① 候选：模型 × 渠道矩阵 + 请求级硬性要求过滤（本地就把不可能的发包剔掉）。
    let matrix = state.ensure_matrix(false).await;
    if matrix.routes(&request.model).is_empty() {
        // 没有任何渠道提供这个模型：这是调用方的问题（名字写错 / 没配覆盖），本地 400。
        return Err(GatewayError::invalid_request(format!(
            "没有渠道提供模型 {}：检查模型名，或用 aliases / routes 配置这条路由",
            request.model
        )));
    }
    let requirements = Requirements::from_request(request, estimate_request_tokens(request));
    let set = router::candidates(&matrix, &requirements, &state.pool, &request.model);
    let candidates = set.usable;
    let rejected: Vec<String> = set
        .rejected_requirements
        .iter()
        .cloned()
        .chain(
            set.channels_without_accounts
                .iter()
                .map(|channel| format!("{channel}：没有账号")),
        )
        .collect();
    if candidates.is_empty() {
        // 全被"请求要求"挡下（不支持图片/工具、上下文超限）：是请求的问题，报 400 说清原因。
        if !set.rejected_requirements.is_empty() {
            return Err(GatewayError::invalid_request(format!(
                "模型 {} 没有满足本次请求要求的渠道（{}）",
                request.model,
                set.rejected_requirements.join("；")
            )));
        }
        // 否则是容量问题：503 让调用方稍后重试。
        let detail = if set.channels_without_accounts.is_empty() {
            "先给渠道添加账号".to_string()
        } else {
            format!("{} 没有账号", set.channels_without_accounts.join("、"))
        };
        return Err(GatewayError::no_account(format!(
            "模型 {} 当前没有可用账号（{detail}）",
            request.model
        )));
    }

    // ② 亲和：命中且可用就直接用；不可用则阶梯式退化（同渠道 > 跨渠道），并清掉废绑定。
    let keys = AffinityStore::candidate_keys(scope, &request.model, request);
    let mut hint: Option<AccountKey> = None;
    let mut affinity_key: Option<String> = None;
    let mut planned: Option<Candidate> = None;
    let mut via_affinity = false;

    if state.affinity.enabled() {
        if let Some((key, binding)) = state.affinity.lookup(&keys, now_ms()) {
            hint = Some(AccountKey::new(
                binding.channel_id.clone(),
                binding.uid.clone(),
            ));
            if let Some(candidate) = router::affinity_pick(&candidates, &binding, &request.model) {
                if candidate
                    .account
                    .is_ready(&candidate.route.upstream_model, now_ms())
                    && !candidate.account.inflight_full()
                {
                    affinity_key = Some(key.clone());
                    planned = Some(candidate);
                }
            }
            if planned.is_none() {
                if state.affinity.mode() == AffinityMode::Strict {
                    return Err(GatewayError::no_account(
                        "会话绑定当前不可用（strict 模式：宁可直接失败，也不换号重写缓存）",
                    ));
                }
                state.affinity.clear(&key);
                let _ = state.storage.delete_affinity(&key);
            }
        }
    }

    // ③ 选号 + 重试。tried 保证一轮请求内绝不重复选同一账号。
    let policy = state.pool.policy();
    let scoring = state.config().scoring;
    let session_seed = router::seed_for(
        request.metadata.session_key().as_deref(),
        &request.model,
        now_ms(),
    );
    let max_attempts = candidates.len().max(2) + 1;
    let mut tried: HashSet<AccountKey> = HashSet::new();
    let mut attempts = 0usize;
    let mut last_error: Option<GatewayError> = None;

    // 攒下"每个候选为什么没被选中"：全池失败时要把真实原因告诉调用方，
    // 而不是笼统地说"冷却或并发已满"（那会误导排查）。
    let mut skip_reasons: Vec<String> = Vec::new();

    while attempts < max_attempts {
        let (candidate, guard) = match planned.take() {
            Some(candidate) => {
                // 亲和目标：槽位被抢走就退回常规挑选，不排队等它。
                match candidate.account.try_acquire() {
                    Some(guard) => {
                        via_affinity = true;
                        (candidate, guard)
                    }
                    None => continue,
                }
            }
            None => {
                via_affinity = false;
                let Some(picked) = router::pick(
                    &candidates,
                    &tried,
                    &request.model,
                    hint.as_ref(),
                    &scoring,
                    session_seed.wrapping_add(attempts as u64),
                ) else {
                    // 记下每个候选为什么不可用（冷却 / 熔断 / 额度停放 / 并发满）。
                    for candidate in &candidates {
                        if tried.contains(&candidate.account.key) {
                            continue;
                        }
                        let reasons = candidate
                            .account
                            .blocked_reasons(&candidate.route.upstream_model, now_ms());
                        let detail = if reasons.is_empty() {
                            if candidate.account.inflight_full() {
                                format!(
                                    "{}/{}：并发已满（{}/{}）",
                                    candidate.route.channel_id,
                                    candidate.account.key.uid,
                                    candidate.account.inflight(),
                                    candidate.account.max_inflight
                                )
                            } else {
                                continue;
                            }
                        } else {
                            format!(
                                "{}/{}：{}",
                                candidate.route.channel_id,
                                candidate.account.key.uid,
                                reasons.join("；")
                            )
                        };
                        if !skip_reasons.contains(&detail) {
                            skip_reasons.push(detail);
                        }
                    }
                    break;
                };
                picked
            }
        };

        tried.insert(candidate.account.key.clone());
        attempts += 1;
        if attempts > 1 {
            state
                .stats
                .retries
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        match try_open(state, &candidate, request).await {
            Ok((adapter, frames)) => {
                candidate.account.on_success(&request.model);
                let key = affinity_key.clone().or_else(|| keys.first().cloned());
                let affinity_key = match (state.affinity.enabled(), key) {
                    (true, Some(key)) => {
                        let binding = Binding {
                            channel_id: candidate.account.key.channel_id.clone(),
                            uid: candidate.account.key.uid.clone(),
                            upstream_model: candidate.route.upstream_model.clone(),
                            expires_at_ms: 0,
                            hits: 0,
                        };
                        let stored = state.affinity.bind(key.clone(), binding, now_ms());
                        // 只在绑定点变化时写库：命中续期不动磁盘。
                        if let Some(stored) = stored {
                            if let Err(error) = state.storage.save_affinity(&key, &stored) {
                                eprintln!("[gateway] 亲和绑定落库失败：{}", error.message);
                            }
                        }
                        Some(key)
                    }
                    _ => None,
                };

                eprintln!(
                    "[gateway] trace={trace_id} ok account={} upstream={} attempts={attempts} affinity={} {}",
                    candidate.account.key,
                    candidate.route.upstream_model,
                    affinity_key.is_some(),
                    request.trace_summary()
                );
                state.push_decision(decision_entry(
                    &trace_id,
                    request,
                    candidates.len(),
                    Some(&candidate),
                    attempts,
                    via_affinity,
                    &rejected,
                    None,
                ));

                return Ok(UpstreamSession {
                    selection: Selection {
                        route: candidate.route,
                        account: candidate.account,
                        via_affinity,
                    },
                    adapter,
                    frames,
                    attempts,
                    trace_id,
                    affinity_key,
                    _guard: guard,
                });
            }
            Err(error) => {
                let error = narrow_scope(error, &request.model);
                candidate.account.apply_error(&error, &policy, now_ms());
                eprintln!(
                    "[gateway] trace={trace_id} fail account={} kind={:?} retryable={} {}",
                    candidate.account.key, error.kind, error.retryable, error.message
                );
                let retryable = error.retryable;
                // 失败即解绑：下次重新挑，避免一直钉在一个坏账号上。
                if let Some(key) = affinity_key.take() {
                    state.affinity.clear(&key);
                    let _ = state.storage.delete_affinity(&key);
                }
                last_error = Some(error);
                // 先归还槽位再换号，否则这个账号在本轮请求里再也选不到。
                drop(guard);
                if !retryable {
                    break;
                }
                // P2：403 内容审核要在这里用降级提示词原号重试一次（换号无效）。
            }
        }
    }

    let error = last_error.unwrap_or_else(|| {
        if skip_reasons.is_empty() {
            // 没有任何一个候选落到这里：说明它们全被排除在候选集之外了。
            GatewayError::no_account(format!(
                "模型 {} 当前没有可用账号（本次请求的候选已被排除或已尝试）",
                request.model
            ))
        } else {
            // 说真话：把"每个账号为什么不可用"直接给出来。
            let mut message = format!("模型 {} 当前没有可用账号：", request.model);
            for (index, reason) in skip_reasons.iter().take(4).enumerate() {
                if index > 0 {
                    message.push_str("；");
                }
                message.push_str(reason);
            }
            if skip_reasons.len() > 4 {
                message.push_str(&format!("；等共 {} 个", skip_reasons.len()));
            }
            GatewayError::no_account(message)
        }
    });
    state.push_decision(decision_entry(
        &trace_id,
        request,
        candidates.len(),
        None,
        attempts,
        via_affinity,
        &rejected,
        Some(&error),
    ));
    Err(error)
}

/// 连一次上游：先保鲜凭据，再按渠道构建报文并取帧流。
async fn try_open(
    state: &GatewayState,
    candidate: &Candidate,
    request: &CanonicalRequest,
) -> Result<(Arc<dyn ChannelAdapter>, FrameStream), GatewayError> {
    if !refresh::ensure_credential(state, &candidate.account).await {
        return Err(GatewayError::new(
            ErrorKind::AuthExpired,
            ErrorScope::Account,
            401,
            "凭据刷新失败，需要重新登录",
            true,
        ));
    }
    let adapter = state
        .channel(&candidate.account.key.channel_id)
        .ok_or_else(|| {
            GatewayError::internal(format!("渠道 {} 没有可用适配器", candidate.account.key.channel_id))
        })?;

    // 上游模型名可能与对外名不同（矩阵的 upstream_model 就是干这个的）。
    let mut routed = request.clone();
    routed.model = candidate.route.upstream_model.clone();
    // 客户端常发一个远超上限的 max_tokens：上游是静默钳制而不是报错，
    // 网关照做（按该渠道目录里的上限钳），免得白跑一趟 400。
    if let (Some(limit), Some(requested)) = (candidate.route.limits.max_output, routed.max_tokens) {
        if limit > 0 && requested > limit {
            routed.max_tokens = Some(limit);
        }
    }

    let frames = adapter
        .chat_stream(&candidate.account.snapshot(), &routed, &state.client)
        .await?;
    Ok((adapter, frames))
}

/// 带 reset 时刻的 429 上游语义是「只限制这个模型」，把 scope 收窄到模型级，
/// 避免因为一个模型限流把整个账号（含其他模型）拉黑。
fn narrow_scope(mut error: GatewayError, model: &str) -> GatewayError {
    if error.reset_at_ms.is_some() && matches!(error.scope, ErrorScope::Account) {
        error.scope = ErrorScope::AccountModel(model.to_string());
    }
    error
}

fn decision_entry(
    trace_id: &str,
    request: &CanonicalRequest,
    candidate_count: usize,
    chosen: Option<&Candidate>,
    attempts: usize,
    via_affinity: bool,
    rejected: &[String],
    error: Option<&GatewayError>,
) -> serde_json::Value {
    serde_json::json!({
        "trace": trace_id,
        "ts": now_ms(),
        "model": request.model,
        "stream": request.stream,
        "session": request.metadata.session_key(),
        "candidates": candidate_count,
        "rejected": rejected,
        "attempts": attempts,
        "via_affinity": via_affinity,
        "channel": chosen.map(|candidate| candidate.route.channel_id.clone()),
        "account": chosen.map(|candidate| candidate.account.key.to_string()),
        "upstream_model": chosen.map(|candidate| candidate.route.upstream_model.clone()),
        "error": error.map(|error| serde_json::json!({
            "kind": error.kind,
            "message": error.message,
        })),
    })
}

fn short_id() -> String {
    Uuid::new_v4().simple().to_string()[..8].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_bearing_rate_limit_is_narrowed_to_the_model() {
        let error = GatewayError::new(
            crate::gateway::errors::ErrorKind::RateLimited,
            ErrorScope::Account,
            429,
            "x",
            true,
        )
        .with_reset_at(Some(1_000));
        match narrow_scope(error, "glm-5.3").scope {
            ErrorScope::AccountModel(model) => assert_eq!(model, "glm-5.3"),
            other => panic!("应当收窄到模型级，实际 {other:?}"),
        }
    }

    #[test]
    fn rate_limit_without_reset_stays_account_level() {
        let error = GatewayError::new(
            crate::gateway::errors::ErrorKind::RateLimited,
            ErrorScope::Account,
            429,
            "x",
            true,
        );
        assert!(matches!(
            narrow_scope(error, "glm-5.3").scope,
            ErrorScope::Account
        ));
    }
}
