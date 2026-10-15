use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use super::{
    providers::ProviderProtocol,
    secrets::{OsCredentialStore, SecretStore},
    security::validate_outbound_url,
    storage::{ModelRecord, ProviderRecord},
};

/// 模型/供应商配置变更广播。
///
/// 设置中心是独立窗口，岛体那个窗口**不再因为打开设置而被卸载**，
/// 所以它不会自动重读模型列表 —— 改了配置必须主动通知它，
/// 否则输入框的模型下拉会一直显示旧数据。
pub const CONFIG_CHANGED_EVENT: &str = "agent://config-changed";

pub fn emit_config_changed(app: &tauri::AppHandle) {
    let _ = app.emit(CONFIG_CHANGED_EVENT, ());
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInput {
    pub id: Option<String>,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInput {
    pub id: Option<String>,
    pub provider_id: String,
    pub model_id: String,
    pub display_name: String,
    pub context_window: Option<u64>,
    pub supports_tools: Option<bool>,
    pub supports_streaming: Option<bool>,
    pub supports_images: Option<bool>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSummary {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub has_api_key: bool,
    pub enabled: bool,
    /// 内置「本机反代」：由网关实时合成，不可编辑/删除。
    pub builtin: bool,
    pub created_at: String,
    pub updated_at: String,
}

fn summarize(provider: ProviderRecord) -> ProviderSummary {
    ProviderSummary {
        id: provider.id,
        name: provider.name,
        protocol: provider.protocol,
        base_url: provider.base_url,
        has_api_key: provider
            .secret_ref
            .as_deref()
            .and_then(|reference| OsCredentialStore.get(reference).ok().flatten())
            .is_some(),
        enabled: provider.enabled,
        builtin: false,
        created_at: provider.created_at,
        updated_at: provider.updated_at,
    }
}

#[tauri::command]
pub fn agent_list_provider_configs(
    state: State<'_, super::AgentState>,
) -> Result<Vec<ProviderSummary>, String> {
    let mut providers = state
        .store
        .list_providers()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(summarize)
        .collect::<Vec<_>>();
    // 内置反代供应商排在最前：它是固定项，不落库、不可删。
    if let Some(gateway) = state.gateway() {
        let builtin = super::gateway_link::builtin_provider(gateway);
        providers.insert(
            0,
            ProviderSummary {
                id: builtin.id,
                name: builtin.name,
                protocol: builtin.protocol,
                base_url: builtin.base_url,
                has_api_key: true,
                enabled: true,
                builtin: true,
                created_at: String::new(),
                updated_at: String::new(),
            },
        );
    }
    Ok(providers)
}

#[tauri::command]
pub async fn agent_list_model_configs(
    state: State<'_, super::AgentState>,
) -> Result<Vec<ModelRecord>, String> {
    let mut models = state.store.list_models().map_err(|error| error.to_string())?;
    if let Some(gateway) = state.gateway() {
        models.extend(super::gateway_link::builtin_models(gateway).await);
    }
    Ok(models)
}

#[tauri::command]
pub fn agent_save_provider_config(
    app: tauri::AppHandle,
    state: State<'_, super::AgentState>,
    input: ProviderInput,
) -> Result<ProviderSummary, String> {
    if input.name.trim().is_empty() {
        return Err("供应商名称不能为空".into());
    }
    // 内置反代供应商由网关状态实时合成，不能写库、不能改名。
    if input
        .id
        .as_deref()
        .is_some_and(super::gateway_link::is_builtin_id)
    {
        return Err("内置「本机反代」供应商不可编辑".into());
    }
    if ProviderProtocol::parse(&input.protocol).is_none() {
        return Err("不支持的 Provider 协议".into());
    }
    validate_outbound_url(&input.base_url)
        .map_err(|error| format!("Base URL 不允许：{error:?}"))?;

    let id = input.id.unwrap_or_else(|| format!("provider-{}", now()));
    let secret_ref = format!("provider/{id}/api-key");
    if let Some(api_key) = input
        .api_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        OsCredentialStore
            .put(&secret_ref, api_key)
            .map_err(|error| error.to_string())?;
    }

    let existing = state
        .store
        .get_provider(&id)
        .map_err(|error| error.to_string())?;
    let now = now();
    let provider = ProviderRecord {
        id,
        name: input.name.trim().to_owned(),
        protocol: input.protocol,
        base_url: input.base_url.trim().trim_end_matches('/').to_owned(),
        secret_ref: Some(secret_ref),
        enabled: input.enabled.unwrap_or(true),
        created_at: existing
            .as_ref()
            .map(|value| value.created_at.clone())
            .unwrap_or_else(|| now.clone()),
        updated_at: now,
    };
    state
        .store
        .upsert_provider(&provider)
        .map_err(|error| error.to_string())?;
    emit_config_changed(&app);
    Ok(summarize(provider))
}

#[tauri::command]
pub fn agent_save_model_config(
    app: tauri::AppHandle,
    state: State<'_, super::AgentState>,
    input: ModelInput,
) -> Result<ModelRecord, String> {
    if input.provider_id.trim().is_empty() || input.model_id.trim().is_empty() {
        return Err("供应商和模型 ID 不能为空".into());
    }
    // 反代目录里的模型是实时合成的，不能当普通模型落库。
    if input.provider_id == super::gateway_link::BUILTIN_PROVIDER_ID
        || input
            .id
            .as_deref()
            .is_some_and(super::gateway_link::is_builtin_id)
    {
        return Err("内置反代模型由网关目录实时提供，无需手动配置".into());
    }
    if state
        .store
        .get_provider(&input.provider_id)
        .map_err(|error| error.to_string())?
        .is_none()
    {
        return Err("供应商不存在".into());
    }
    let model = ModelRecord {
        id: input.id.unwrap_or_else(|| format!("model-{}", now())),
        provider_id: input.provider_id,
        model_id: input.model_id.trim().to_owned(),
        display_name: if input.display_name.trim().is_empty() {
            input.model_id.trim().to_owned()
        } else {
            input.display_name.trim().to_owned()
        },
        context_window: input.context_window,
        supports_tools: input.supports_tools.unwrap_or(true),
        supports_streaming: input.supports_streaming.unwrap_or(true),
        supports_images: input.supports_images.unwrap_or(false),
        enabled: input.enabled.unwrap_or(true),
    };
    state
        .store
        .upsert_model(&model)
        .map_err(|error| error.to_string())?;
    emit_config_changed(&app);
    Ok(model)
}

#[tauri::command]
pub fn agent_delete_model_config(
    app: tauri::AppHandle,
    state: State<'_, super::AgentState>,
    model_id: String,
) -> Result<(), String> {
    if model_id.trim().is_empty() {
        return Err("模型不存在".into());
    }
    if super::gateway_link::is_builtin_id(model_id.trim()) {
        return Err("内置反代模型由网关目录实时提供，不可删除".into());
    }
    state
        .store
        .delete_model(model_id.trim())
        .map_err(|error| error.to_string())?;
    emit_config_changed(&app);
    Ok(())
}

fn now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}
