//! 内置「本机反代」供应商：把岛内网关当作一个一等供应商接入 Agent。
//!
//! 网关已经在 `127.0.0.1:<port>/v1` 暴露 OpenAI / Anthropic 兼容面，
//! 所以「供应商用反代」不需要新的传输层 —— 只需要：
//! 1. 合成一个指向网关的供应商记录（Base URL 随实际端口，端口会回退，不能手写死）；
//! 2. 把网关的模型目录实时映射成模型列表，随渠道目录变化自动更新。
//!
//! 这两者都不落库：网关是活的状态，写进 SQLite 只会过期。

use crate::gateway::SharedGateway;

use super::storage::{ModelRecord, ProviderRecord};

/// 内置供应商 id：固定且保留，数据库里不会出现同 id 的记录。
pub const BUILTIN_PROVIDER_ID: &str = "builtin-gateway";
/// 合成模型 id 前缀：`gateway:<对外模型名>`。用于把它和数据库自建模型区分开。
pub const BUILTIN_MODEL_PREFIX: &str = "gateway:";

/// 内置供应商的展示名。
pub const BUILTIN_PROVIDER_NAME: &str = "本机反代";

/// 网关实际监听地址；未跑起来时回落到配置端口（请求会在传输层报错，提示更明确）。
fn gateway_base_url(gateway: &SharedGateway) -> String {
    let origin = gateway
        .server_status()
        .addr
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", gateway.config().port));
    // 端口取实际监听的，主机**钉死回环**：allow_lan 开着时网关绑 0.0.0.0，
    // addr 会变成 "http://0.0.0.0:8790" —— 既过不了 agent 的出站安全策略
    // （只放行 https / http+localhost，0.0.0.0 会被拒成 LocalhostOnlyForHttp），
    // 作为目标地址也不可连。回环访问绑 0.0.0.0 的服务始终可用。
    let port = origin.rsplit(':').next().and_then(|p| p.parse::<u16>().ok());
    match port {
        Some(port) => format!("http://127.0.0.1:{port}/v1"),
        None => format!("{origin}/v1"),
    }
}

/// 合成的内置供应商记录。`secret_ref` 为空：密钥在解析时直接取网关配置（见 `builtin_credential`）。
pub fn builtin_provider(gateway: &SharedGateway) -> ProviderRecord {
    let now = String::new();
    ProviderRecord {
        id: BUILTIN_PROVIDER_ID.into(),
        name: BUILTIN_PROVIDER_NAME.into(),
        protocol: "chat-completions".into(),
        base_url: gateway_base_url(gateway),
        secret_ref: None,
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    }
}

/// 访问本机网关要带的密钥：配了就用第一把；未配（回环开放）时用占位符，
/// 网关在 `api_keys` 为空时不校验，占位符不会造成 401。
///
/// 返回 `Zeroizing<String>` 与系统凭据库的取值类型一致，便于调用处统一处理。
pub fn builtin_credential(gateway: &SharedGateway) -> zeroize::Zeroizing<String> {
    zeroize::Zeroizing::new(
        gateway
            .config()
            .api_keys
            .first()
            .cloned()
            .unwrap_or_else(|| "local".into()),
    )
}

/// 从网关模型目录合成模型列表：对外名即模型 ID，能力/上下文取该模型所有路由的并集。
pub async fn builtin_models(gateway: &SharedGateway) -> Vec<ModelRecord> {
    let matrix = gateway.ensure_matrix(false).await;
    matrix
        .public_models()
        .into_iter()
        .map(|model| {
            let routes = matrix.routes(&model);
            let caps_known = routes.iter().any(|route| route.caps_known);
            let vision = routes.iter().any(|route| route.capabilities.vision);
            // 能力未知时乐观放行工具调用：由上游报错，避免"因为未知就一律禁用"。
            let tools = !caps_known || routes.iter().any(|route| route.capabilities.tool_calls);
            let context = routes.iter().filter_map(|route| route.limits.context).max();
            ModelRecord {
                id: format!("{BUILTIN_MODEL_PREFIX}{model}"),
                provider_id: BUILTIN_PROVIDER_ID.into(),
                model_id: model.clone(),
                display_name: model,
                context_window: context,
                supports_tools: tools,
                supports_streaming: true,
                supports_images: vision,
                enabled: true,
            }
        })
        .collect()
}

/// 合成模型 id → 对外模型名。
pub fn public_model_of(model_id: &str) -> Option<&str> {
    model_id.strip_prefix(BUILTIN_MODEL_PREFIX)
}

/// 该 id 是否属于内置供应商（供应商 id 或合成模型 id）。
pub fn is_builtin_id(id: &str) -> bool {
    id == BUILTIN_PROVIDER_ID || id.starts_with(BUILTIN_MODEL_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_ids_are_recognized_and_prefixed() {
        assert!(is_builtin_id(BUILTIN_PROVIDER_ID));
        assert!(is_builtin_id("gateway:glm-5.3-flash"));
        assert!(!is_builtin_id("provider-123"));
        assert_eq!(public_model_of("gateway:glm-5.3-flash"), Some("glm-5.3-flash"));
        assert_eq!(public_model_of("provider-123"), None);
    }
}
