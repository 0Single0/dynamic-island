/// 改账号画像：昵称 / 优先级 / 权重 / 并发上限（面板「编辑账号」）。
///
/// 四个字段都是"可选覆盖"：
/// · `nickname = None` 不动昵称，`Some("")` 清空（面板回落到 uid 前 8 位）；
/// · `priority / weight / max_inflight = None` 清掉覆盖、回落到渠道或全局默认。
///
/// 为什么不复用 `gateway_add_account`：那条要整条记录，**凭据、加入时间全都会被覆盖**。
/// 编辑只该动这几样，登录态必须原样留着。改完 `reload_accounts()` 会继承冷却/额度/在飞等运行态。
#[tauri::command]
pub async fn gateway_set_account_profile(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
    nickname: Option<String>,
    priority: Option<u32>,
    weight: Option<u32>,
    max_inflight: Option<u32>,
) -> Result<Value, String> {
    if state.account(&channel_id, &uid).is_none() {
        return Err(format!("账号 {channel_id}/{uid} 不存在"));
    }
    // 昵称清洗：去首尾空白；超长会顶坏面板的固定列宽，截到 40 个字符。
    // **空串是有意义的**（= 清空，视图回落到 uid 前 8 位），所以不能把它折成 None：
    // None 在这里表示"别动昵称"。
    let nickname = nickname.map(|value| value.trim().chars().take(40).collect::<String>());
    let changed = state
        .storage
        .set_account_profile(
            &channel_id,
            &uid,
            nickname.as_deref(),
            priority,
            weight,
            max_inflight,
        )
        .map_err(message)?;
    state.reload_accounts().map_err(message)?;
    // 回读一次：面板要拿"回落之后实际生效的值"，而不是它刚才填的值。
    let account = state
        .account(&channel_id, &uid)
        .ok_or_else(|| format!("账号 {channel_id}/{uid} 在重建后消失了"))?;
    let view = account.view(now_ms());
    Ok(json!({
        "ok": changed,
        "nickname": view.nickname,
        "priority": view.priority,
        "weight": view.weight,
        "max_inflight": view.max_inflight,
    }))
}

/// 改账号的模型清单：白名单（空 = 不限）+ 账号级模型改名。
///
/// 白名单语义是"这个号**只**服务这些对外模型名"，空列表 = 该渠道有什么就能跑什么。
/// 改名解决的是同名不同姓：Trae 把 `deepseek-v4.1-flash` 叫 `DeepSeek-V4-Flash-Official`，
/// 配上 `{ "deepseek-v4.1-flash": "DeepSeek-V4-Flash-Official" }` 之后，
/// 对外仍然是统一的 `deepseek-v4.1-flash`，出站自动换成该账号的上游名。
///
/// 两样都只影响调度，不改凭据、额度和冷却。
#[tauri::command]
pub async fn gateway_set_account_models(
    state: State<'_, SharedGateway>,
    channel_id: String,
    uid: String,
    allow: Vec<String>,
    rename: std::collections::BTreeMap<String, String>,
) -> Result<Value, String> {
    if state.account(&channel_id, &uid).is_none() {
        return Err(format!("账号 {channel_id}/{uid} 不存在"));
    }
    // 清掉空白项：面板里删到只剩空格时，别让它变成一条匹配不上的白名单。
    let allow: Vec<String> = allow
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    // 改名两边都去空白；只映射到空名的条目没有意义（等于让请求不带模型名），直接丢掉。
    let rename: std::collections::BTreeMap<String, String> = rename
        .into_iter()
        .map(|(from, to)| (from.trim().to_string(), to.trim().to_string()))
        .filter(|(from, to)| !from.is_empty() && !to.is_empty())
        .collect();
    let changed = state
        .storage
        .set_account_models(&channel_id, &uid, &allow, &rename)
        .map_err(message)?;
    state.reload_accounts().map_err(message)?;
    Ok(json!({ "ok": changed, "allow": allow, "rename": rename }))
}
