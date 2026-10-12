//! 网关的常驻定时任务（不依赖面板是否打开）。
//!
//! 为什么放在后端：这些是**系统该自己做的事**，不是"打开面板才做"。
//! 早期版本把额度刷新写在前端（面板停在账号页、每 60 秒检查一次、快照超过
//! 10 分钟才补），结果是：
//! · 用网关跑对话消耗了积分，界面上看不到变化（没人触发刷新）；
//! · 切到「请求日志」「用量」等页面后，刷新彻底停止；
//! · 工具窗口一关，什么都没跑。
//!
//! 现在统一由这里驱动，前端只负责显示。

use std::time::Duration;

use crate::gateway::{state::SharedGateway, timeutil::now_ms};

/// 额度快照超过这个时长就刷新（任务/对话都会改积分，5 分钟粒度够用又不打扰上游）。
const QUOTA_STALE_MS: u64 = 5 * 60 * 1000;
/// 巡检周期。
const TICK: Duration = Duration::from_secs(60);
/// 首次巡检前的等待：把启动时那波请求让过去（登录、目录拉取）。
const STARTUP_DELAY: Duration = Duration::from_secs(20);
/// 账号之间的间隔：避免整点齐发被上游当成异常流量。
const PER_ACCOUNT_GAP: Duration = Duration::from_millis(800);

/// 每日动作的补跑档位（参考实现：09:00 / 21:00 各巡检一次）。
const DAILY_HOURS: [u32; 2] = [9, 21];
/// 账号之间的间隔：别让签到/旅行请求挤在一起。
const DAILY_GAP: Duration = Duration::from_millis(1200);

/// 启动常驻定时任务（`setup` 时调用一次）。
pub fn spawn(state: SharedGateway) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        // 每 (账号, 动作, 日期) 只自动跑一次；失败项留到 09:00/21:00 再补一次。
        let mut daily: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        loop {
            refresh_stale_quotas(&state).await;
            run_daily_actions(&state, &mut daily).await;
            tokio::time::sleep(TICK).await;
        }
    });
}

/// 跑一遍每日动作：只做**今天还没做**的，且不重复尝试。
///
/// 为什么不只在 09:00/21:00 跑：应用不是常开的（关了就不会触发整点）。
/// 所以启动后先补一次，整点档再给"上次失败"的账号补一次机会。
async fn run_daily_actions(
    state: &SharedGateway,
    done: &mut std::collections::HashMap<String, bool>,
) {
    let (year, month, day, hour) = crate::gateway::timeutil::local_date_hour();
    let today = format!("{year:04}{month:02}{day:02}");
    let slot = DAILY_HOURS.contains(&hour);

    for account in state.pool.accounts() {
        if !account.enabled {
            continue;
        }
        if account.requires_credential && account.credentials().access_token.is_empty() {
            continue;
        }
        if account.cooldown_until_ms() > crate::gateway::timeutil::now_ms() {
            continue;
        }
        let Some(adapter) = state.channel(&account.key.channel_id) else {
            continue;
        };
        let snapshot = account.snapshot();
        let Ok(Some(actions)) = adapter.daily_actions(&state.client, &snapshot).await else {
            continue;
        };
        for action in actions {
            if action.done_today || !action.automatable {
                continue;
            }
            let key = format!("{}|{}|{today}", account.key, action.code);
            match done.get(&key) {
                // 今天试过且成功 → 不再打扰上游。
                Some(true) => continue,
                // 今天试过但失败：只在整点档再给一次机会，避免每 60 秒重试。
                Some(false) if !slot => continue,
                _ => {}
            }
            let outcome = adapter
                .run_daily_action(&state.client, &snapshot, &action.code)
                .await;
            let ok = outcome.as_ref().map(|one| one.status == "done").unwrap_or(false);
            match outcome {
                Ok(one) => {
                    eprintln!(
                        "[gateway] 每日动作 {} {}/{} → {}：{}",
                        one.name, account.key.channel_id, account.key.uid, one.status, one.message
                    );
                }
                Err(error) => eprintln!(
                    "[gateway] 每日动作 {} {}/{} 失败：{}",
                    action.name, account.key.channel_id, account.key.uid, error.describe()
                ),
            }
            done.insert(key, ok);
            // 签到/领奖都会改积分：顺手刷新额度，面板不用等 5 分钟。
            if ok && action.code != "daily_chat" {
                if let Ok(Some(quota)) = adapter.fetch_quota(&state.client, &snapshot).await {
                    account.set_quota(Some(quota));
                }
            }
            tokio::time::sleep(DAILY_GAP).await;
        }
    }
}

/// 刷新"过期"账号的额度快照。串行 + 间隔，冷却中的账号跳过。
async fn refresh_stale_quotas(state: &SharedGateway) {
    let now = now_ms();
    for account in state.pool.accounts() {
        if !account.enabled {
            continue;
        }
        // 没凭据（还没登录）或正在冷却的账号不打扰上游。
        if account.requires_credential && account.credentials().access_token.is_empty() {
            continue;
        }
        if account.cooldown_until_ms() > now {
            continue;
        }
        let fresh = account
            .quota()
            .map(|quota| now.saturating_sub(quota.fetched_at_ms) <= QUOTA_STALE_MS)
            .unwrap_or(false);
        if fresh {
            continue;
        }
        let Some(adapter) = state.channel(&account.key.channel_id) else {
            continue;
        };
        match adapter
            .fetch_quota(&state.client, &account.snapshot())
            .await
        {
            Ok(snapshot) => {
                account.set_quota(snapshot);
            }
            // 额度刷新失败不该吵用户：下一轮会再试。
            Err(error) => {
                eprintln!(
                    "[gateway] 账号 {} 额度刷新失败（下轮重试）：{}",
                    account.key, error.message
                );
            }
        }
        tokio::time::sleep(PER_ACCOUNT_GAP).await;
    }
}
