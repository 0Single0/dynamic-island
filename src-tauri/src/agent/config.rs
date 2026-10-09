use serde::{Deserialize, Serialize};
use tauri::State;

use super::{
    providers::ProviderProtocol,
    secrets::{OsCredentialStore, SecretStore},
    security::validate_outbound_url,
    storage::{ModelRecord, ProviderRecord},
};

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
        created_at: provider.created_at,
        updated_at: provider.updated_at,
    }
}

#[tauri::command]
pub fn agent_list_provider_configs(
    state: State<'_, super::AgentState>,
) -> Result<Vec<ProviderSummary>, String> {
    state
        .store
        .list_providers()
        .map(|providers| providers.into_iter().map(summarize).collect())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn agent_list_model_configs(
    state: State<'_, super::AgentState>,
) -> Result<Vec<ModelRecord>, String> {
    state.store.list_models().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn agent_save_provider_config(
    state: State<'_, super::AgentState>,
    input: ProviderInput,
) -> Result<ProviderSummary, String> {
    if input.name.trim().is_empty() {
        return Err("供应商名称不能为空".into());
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
    Ok(summarize(provider))
}

#[tauri::command]
pub fn agent_save_model_config(
    state: State<'_, super::AgentState>,
    input: ModelInput,
) -> Result<ModelRecord, String> {
    if input.provider_id.trim().is_empty() || input.model_id.trim().is_empty() {
        return Err("供应商和模型 ID 不能为空".into());
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
    Ok(model)
}

#[tauri::command]
pub fn agent_delete_model_config(
    state: State<'_, super::AgentState>,
    model_id: String,
) -> Result<(), String> {
    if model_id.trim().is_empty() {
        return Err("模型不存在".into());
    }
    state
        .store
        .delete_model(model_id.trim())
        .map_err(|error| error.to_string())
}

fn now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}
