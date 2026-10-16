//! 凭据保鲜：选号前检查，临期则**按账号串行**刷新并落库。
//!
//! 为什么必须串行：上游每次刷新都会轮换 refresh_token，
//! 并发刷新会让后一次拿着已作废的旧 token 去换，结果是账号被刷废。

use crate::gateway::{
    channels::Credential, core::pool::PoolAccount, errors::GatewayError, state::GatewayState,
    timeutil::now_ms,
};

/// 安全边距：剩余寿命不足 2 分钟就先刷新（与参考实现一致）。
pub const SAFETY_MARGIN_MS: u64 = 120_000;

/// 确保账号凭据可用。返回 false = 该账号当前不可用（没凭据或刷新失败）。
pub async fn ensure_credential(state: &GatewayState, account: &PoolAccount) -> bool {
    let credential = account.credentials();
    if credential.access_token.is_empty() {
        return !account.requires_credential;
    }
    if !credential.is_expiring(now_ms(), SAFETY_MARGIN_MS) {
        return true;
    }
    match refresh(state, account).await {
        Ok(_) => true,
        Err(error) => {
            eprintln!(
                "[gateway] 账号 {} 刷新失败（本轮换号）：{}",
                account.key, error.message
            );
            false
        }
    }
}

/// 强制刷新（面板的"刷新凭据"按钮也走它）。
pub async fn refresh(
    state: &GatewayState,
    account: &PoolAccount,
) -> Result<Credential, GatewayError> {
    let _serial = account.runtime.refresh_lock.lock().await;

    // 等锁期间别的请求可能已经刷好了：加锁后再确认一次，避免重复刷新把 token 轮换两次。
    let current = account.credentials();
    if !current.access_token.is_empty() && !current.is_expiring(now_ms(), 0) {
        return Ok(current);
    }

    let adapter = state.channel(&account.key.channel_id).ok_or_else(|| {
        GatewayError::internal(format!("渠道 {} 没有可用适配器", account.key.channel_id))
    })?;
    let refreshed = adapter
        .refresh(&state.client, &account.snapshot())
        .await?
        .ok_or_else(|| GatewayError::unsupported("该渠道不支持刷新凭据，请重新登录"))?;

    account.set_credentials(refreshed.clone());
    if let Err(error) = state.storage.set_credential(
        &account.key.channel_id,
        &account.key.uid,
        Some(&refreshed),
    ) {
        // 内存里已生效，落库失败只影响重启后的可用性，不该让本次请求失败。
        eprintln!("[gateway] 新凭据落库失败：{}", error.message);
    }
    eprintln!(
        "[gateway] 账号 {} 凭据已刷新，{}",
        account.key,
        expiry_hint(&refreshed)
    );
    Ok(refreshed)
}

fn expiry_hint(credential: &Credential) -> String {
    match credential.expires_at_ms {
        Some(expires_at) => {
            let remaining = expires_at.saturating_sub(now_ms()) / 60_000;
            format!("剩余 {remaining} 分钟")
        }
        None => "上游未给过期时间".to_string(),
    }
}
