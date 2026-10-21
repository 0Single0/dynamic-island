//! 自定义供应商相关的命令：拉模型、保存供应商、删除渠道。
//!
//! 单独成文件是因为它与"读状态"类命令不同：这里一条命令会**改配置 + 写账号 + 重建矩阵**，
//! 是一段完整流程，值得独立说明；`commands/mod.rs` 只负责转发与读取。

use serde_json::{json, Value};
use tauri::State;

use crate::gateway::{
    config::ChannelConfig, state::SharedGateway, storage::AccountRecord, timeutil::now_ms,
};

use super::message;

/// 读取用户选中的凭据文件（`.csv` / `.json` / `.txt`）。
///
/// 为什么由后端读而不是前端 `File.text()`：Tauri 的原生文件对话框只给**路径**，
/// 磁盘读取归后端；顺带统一处理 BOM 与 GBK —— 华为云控制台导出的 CSV 在中文
/// Windows 上可能是 GBK 编码，按 UTF-8 硬读会直接报错。
///
/// 限制：只接受文本类扩展名，且文件不超过 1 MiB（凭据文件就是几行）。
#[tauri::command]
pub fn gateway_read_credential_file(path: String) -> Result<Value, String> {
    let path = std::path::PathBuf::from(path.trim());
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    if !matches!(extension.as_str(), "csv" | "json" | "txt" | "text" | "conf") {
        return Err(format!(
            "不支持的文件类型 .{extension}：请选择 .csv、.json 或 .txt"
        ));
    }
    let meta = std::fs::metadata(&path).map_err(|error| format!("读不到文件：{error}"))?;
    if meta.len() > 1024 * 1024 {
        return Err("文件超过 1 MiB，凭据文件通常只有几行".into());
    }
    let bytes = std::fs::read(&path).map_err(|error| format!("读取失败：{error}"))?;
    let (text, encoding) = decode_credential_text(&bytes)?;
    Ok(json!({
        // 前端只拿文本 + 文件名做预览，不落盘。
        "text": text,
        "encoding": encoding,
        "name": path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default(),
    }))
}

/// 去掉 BOM 后按 UTF-8 解码。
///
/// 华为云控制台导出的 `credentials.csv` 是 **UTF-8 with BOM**（实测前三字节 EF BB BF），
/// 所以去 BOM 就够了，不需要为 GBK 引入额外依赖。
///
/// 解不出来时把话说清楚：让用户另存为 UTF-8，而不是抛一个"invalid utf-8"的天书。
fn decode_credential_text(bytes: &[u8]) -> Result<(String, &'static str), String> {
    let (body, had_bom) = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => (rest, true),
        None => (bytes, false),
    };
    match std::str::from_utf8(body) {
        Ok(text) => Ok((text.to_string(), if had_bom { "utf-8-bom" } else { "utf-8" })),
        Err(_) => Err(
            "文件不是 UTF-8 编码。请用记事本另存为 UTF-8，或直接把内容复制粘贴进来。".into(),
        ),
    }
}
/// 「获取模型」：按地址拉一次上游模型清单（填供应商时用，不落库）。
///
/// 通用实现放在渠道层（候选地址 + 协议鉴权 + 多方言解析），命令只做参数转发与
/// "顺手落进配置"的编排 —— 这样面板与未来的脚本入口共用同一份逻辑。
#[tauri::command]
pub async fn gateway_fetch_models(
    state: State<'_, SharedGateway>,
    base_url: String,
    api_key: Option<String>,
    models_url: Option<String>,
    protocol: Option<String>,
) -> Result<Value, String> {
    let models = crate::gateway::channels::custom::fetch_models(
        &state.client,
        &base_url,
        models_url.as_deref(),
        api_key.as_deref().unwrap_or_default(),
        protocol.as_deref().unwrap_or("openai-chat"),
    )
    .await
    // 失败要把原因说清（401 / 超时 / 没有该端点），面板才好提示用户改什么。
    .map_err(|error| error.describe())?;
    Ok(json!({
        "count": models.len(),
        "models": models
            .iter()
            .map(|model| json!({
                "id": model.id,
                "context_length": model.context_length,
                "max_output_tokens": model.max_output_tokens,
                "capabilities": {
                    "vision": model.capabilities.vision,
                    "tool_calls": model.capabilities.tool_calls,
                    "reasoning": model.capabilities.reasoning,
                },
            }))
            .collect::<Vec<_>>(),
    }))
}

/// 保存（或更新）一个自定义供应商：渠道配置 + 密钥账号一次写入。
///
/// 为什么渠道与账号一起写：自定义供应商的"账号"就是它的 API Key，
/// 分两步写入会出现"渠道建好了但没有可用账号"的中间态 —— 面板要能一次点完。
#[tauri::command]
pub async fn gateway_save_custom_provider(
    state: State<'_, SharedGateway>,
    channel_id: Option<String>,
    display_name: String,
    base_url: String,
    api_key: Option<String>,
    protocol: Option<String>,
    models_url: Option<String>,
    models: Vec<Value>,
    require_api_key: Option<bool>,
    priority: Option<u32>,
) -> Result<Value, String> {
    let display_name = display_name.trim().to_string();
    let base_url = base_url.trim().to_string();
    if display_name.is_empty() {
        return Err("请填写供应商名称".into());
    }
    if base_url.is_empty() {
        return Err("请填写供应商地址".into());
    }

    // 渠道 id：新建时派生一个稳定、可读、不与现有渠道冲突的；编辑时沿用。
    let channel_id = match channel_id.map(|id| id.trim().to_string()).filter(|id| !id.is_empty()) {
        Some(id) => id,
        None => {
            let existing: Vec<String> = state.channels().iter().map(|c| c.id().to_string()).collect();
            unique_channel_id(&display_name, &existing)
        }
    };

    let options = json!({
        "base_url": base_url,
        "protocol": protocol.unwrap_or_else(|| "openai-chat".into()),
        "models_url": models_url.filter(|url| !url.trim().is_empty()),
        "require_api_key": require_api_key.unwrap_or(true),
        "models": models,
    });
    let channel = ChannelConfig {
        id: channel_id.clone(),
        adapter: "custom".into(),
        display_name: display_name.clone(),
        enabled: true,
        priority: priority.unwrap_or(100),
        weight: 1,
        options,
    };
    state
        .edit_config(|config| {
            match config.channels.iter_mut().find(|entry| entry.id == channel.id) {
                Some(existing) => *existing = channel.clone(),
                None => config.channels.push(channel.clone()),
            }
        })
        .map_err(message)?;

    // 密钥账号：uid 固定为 `default`（一个供应商一把 Key）。
    // 留空表示"不改动已有密钥"——编辑时不想重填就得能留空。
    let uid = "default".to_string();
    let existing = state.storage.list_accounts().unwrap_or_default();
    let previous = existing
        .iter()
        .find(|record| record.channel_id == channel_id && record.uid == uid);

    let key = api_key
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .or_else(|| previous.and_then(|record| record.credential.as_ref()).map(|credential| credential.access_token.clone()));

    let requires = require_api_key.unwrap_or(true);
    if requires && key.as_deref().unwrap_or_default().is_empty() {
        return Err("请填写 API Key（本地服务可关闭「需要密钥」）".into());
    }

    state
        .storage
        .upsert_account(&AccountRecord {
            channel_id: channel_id.clone(),
            uid: uid.clone(),
            nickname: display_name,
            enabled: true,
            priority_override: None,
            weight_override: None,
            max_inflight: None,
            proxy_slot: None,
            added_at: previous.map(|record| record.added_at).unwrap_or_else(now_ms),
            source: "manual".into(),
            // 静态密钥没有过期时间：留 None，面板显示「已登录」而不是一个假的天数。
            // 保鲜链路靠 `static_credential()` 跳过刷新，不会因为 None 就去刷新。
            credential: key.map(|key| crate::gateway::channels::Credential {
                access_token: key,
                ..Default::default()
            }),
            model_allow: Vec::new(),
            model_rename: std::collections::BTreeMap::new(),
        })
        .map_err(message)?;
    state.reload_accounts().map_err(message)?;
    let matrix = state.ensure_matrix(true).await;
    Ok(json!({
        "channel_id": channel_id,
        "routes": matrix.route_count(),
        "accounts": state.pool.by_channel(&channel_id).len(),
    }))
}

/// 删除渠道：连它的账号与配置一起移除（渠道不存在也算成功，幂等）。
///
/// 账号先删、配置后删：反过来会出现"配置没了但账号还在"的中间态 ——
/// 那些账号会因为没有适配器而被跳过并刷警告（见 `build_accounts`）。
#[tauri::command]
pub async fn gateway_delete_channel(
    state: State<'_, SharedGateway>,
    channel_id: String,
) -> Result<Value, String> {
    let uids: Vec<String> = state
        .storage
        .list_accounts()
        .unwrap_or_default()
        .into_iter()
        .filter(|record| record.channel_id == channel_id)
        .map(|record| record.uid)
        .collect();
    let mut removed = 0usize;
    for uid in &uids {
        match state.storage.delete_account(&channel_id, uid) {
            Ok(true) => removed += 1,
            Ok(false) => {}
            Err(error) => return Err(error.message),
        }
    }
    state
        .edit_config(|config| config.channels.retain(|channel| channel.id != channel_id))
        .map_err(message)?;
    state.reload_accounts().map_err(message)?;
    Ok(json!({ "removed_accounts": removed }))
}

/// 由显示名派生一个未占用的渠道 id（保留中文/字母数字，其余折成 `-`）。
fn unique_channel_id(display_name: &str, existing: &[String]) -> String {
    let mut base: String = display_name
        .trim()
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_alphanumeric() || ch == '-' { ch } else { '-' })
        .collect();
    while base.contains("--") {
        base = base.replace("--", "-");
    }
    let base = base.trim_matches('-').to_string();
    let base = if base.is_empty() { "custom".to_string() } else { base };
    if !existing.iter().any(|id| id == &base) {
        return base;
    }
    for suffix in 2..1000 {
        let candidate = format!("{base}-{suffix}");
        if !existing.iter().any(|id| id == &candidate) {
            return candidate;
        }
    }
    format!("{base}-{}", now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 华为云控制台导出的 CSV 是 **UTF-8 with BOM**（实测前三字节 EF BB BF）。
    /// 不去掉 BOM 的话，表头会变成 `\u{feff}User Name`，列名匹配失败。
    #[test]
    fn utf8_bom_is_stripped_from_credential_files() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("User Name,Access Key Id,Secret Access Key".as_bytes());
        let (text, encoding) = decode_credential_text(&bytes).expect("bom 要能解");
        assert_eq!(text, "User Name,Access Key Id,Secret Access Key");
        assert_eq!(encoding, "utf-8-bom");
    }

    #[test]
    fn plain_utf8_is_decoded_as_is() {
        let (text, encoding) = decode_credential_text(b"token,ak,sk").expect("普通 utf-8");
        assert_eq!(text, "token,ak,sk");
        assert_eq!(encoding, "utf-8");
    }

    /// 非 UTF-8 要给出**可操作**的提示，而不是抛 "invalid utf-8"。
    #[test]
    fn non_utf8_input_gets_an_actionable_message() {
        // 0xFF 在任何编码下都不是合法的 UTF-8 起始字节。
        let error = decode_credential_text(&[0xFF, 0xFE, 0x00]).expect_err("必须报错");
        assert!(error.contains("UTF-8"), "{error}");
        assert!(error.contains("粘贴"), "要给出替代路径：{error}");
    }

    /// 解码后的 CSV 必须能一路走到凭据解析（端到端串起来）。
    #[test]
    fn a_bom_csv_round_trips_into_a_credential() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(
            "User Name,Access Key Id,Secret Access Key\n\"u\",AK1,SK1".as_bytes(),
        );
        let (text, _) = decode_credential_text(&bytes).expect("bom 要能解");
        let credential = crate::gateway::channels::codearts::parse_pasted_credential(&text)
            .expect("能解析成凭据");
        assert_eq!(credential.access_key_id.as_deref(), Some("AK1"));
        assert_eq!(credential.secret_key.as_deref(), Some("SK1"));
    }
}

#[cfg(test)]
mod rename_tests {
    /// 回归：路由覆盖的 `upstream_model` **只能**是本渠道目录里的名字。
    ///
    /// 早先界面把方向做反了，用户可以填出
    /// `{ model: "DeepSeek-V4-Flash-Official", upstream_model: "deepseek-v4.1-flash" }`，
    /// 于是 Trae 的上游名被改成一个**它不认识**的名字 → 上游回 995000。
    /// 这类条目现在会被 `set_account_models` 的清理逻辑删掉。
    #[test]
    fn a_route_override_pointing_at_a_foreign_name_is_stale() {
        let catalog: std::collections::HashSet<String> =
            ["DeepSeek-V4-Flash-Official".to_string()].into_iter().collect();
        let rename: std::collections::BTreeMap<String, String> =
            std::iter::empty().collect();

        // 脏条目：model 是目录原名，但 upstream 指向别的渠道的名字。
        let stale_model = "DeepSeek-V4-Flash-Official".to_string();
        let keep = rename.contains_key(&stale_model) || catalog.contains(&stale_model);
        assert!(keep, "目录里本来就有的名字不该被当成脏条目删掉");

        // 真正的脏条目：model 既不在改名表、也不在目录里（是被写坏的对外名）。
        let polluted = "deepseek-v4.1-flash".to_string();
        let keep_polluted = rename.contains_key(&polluted) || catalog.contains(&polluted);
        assert!(!keep_polluted, "既不在改名表也不在目录里 → 应当清掉");
    }
}
