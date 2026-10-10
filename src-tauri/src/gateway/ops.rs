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

/// 启动常驻定时任务（`setup` 时调用一次）。
pub fn spawn(state: SharedGateway) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        loop {
            refresh_stale_quotas(&state).await;
            tokio::time::sleep(TICK).await;
        }
    });
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
