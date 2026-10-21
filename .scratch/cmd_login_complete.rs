/// 完成一次「需要粘贴回调地址」的登录（Trae 这类渠道）。
///
/// 返回值与 `gateway_login_poll` 的成功分支同形，前端可以复用同一段入库提示。
#[tauri::command]
pub async fn gateway_login_complete(
    state: State<'_, SharedGateway>,
    channel_id: String,
    callback: String,
) -> Result<Value, String> {
    if callback.trim().is_empty() {
        return Err("请粘贴授权完成后浏览器地址栏里的完整 URL".into());
    }
    let adapter = state
        .channel(&channel_id)
        .ok_or_else(|| format!("渠道 {channel_id} 不存在"))?;
    let credential = adapter
        .login_complete(&state.client, callback.trim())
        .await
        .map_err(message)?;
    let uid = credential
        .uid
        .clone()
        .filter(|uid| !uid.is_empty())
        .ok_or_else(|| "上游没有返回账号标识（JWT sub 为空）".to_string())?;
    let nickname = credential
        .nickname
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| uid.chars().take(8).collect());
    state
        .storage
        .upsert_account(&AccountRecord {
            channel_id: channel_id.clone(),
            uid: uid.clone(),
            nickname: nickname.clone(),
            enabled: true,
            priority_override: None,
            weight_override: None,
            max_inflight: None,
            proxy_slot: None,
            added_at: now_ms(),
            source: "oauth".into(),
            credential: Some(credential),
            model_allow: Vec::new(),
            model_rename: std::collections::BTreeMap::new(),
        })
        .map_err(message)?;
    let count = state.reload_accounts().map_err(message)?;
    // 新号顺手签一次到、抓一次额度；失败都不影响可用性。
    if let Some(account) = state.account(&channel_id, &uid) {
        let snapshot = account.snapshot();
        let _ = adapter.sign_in(&state.client, &snapshot).await;
        if let Ok(quota) = adapter.fetch_quota(&state.client, &snapshot).await {
            account.set_quota(quota);
        }
    }
    let _ = state.ensure_matrix(true).await;
    Ok(json!({
        "status": "ok",
        "account": { "channel_id": channel_id, "uid": uid, "nickname": nickname },
        "accounts": count,
    }))
}
