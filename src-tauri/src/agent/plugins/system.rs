//! 系统插件：宿主基础能力适配层。
//!
//! 只声明能力和清单，不复制业务实现。

use serde_json::json;

use super::host::{bind, HandlerMap};
use super::manifest::{
    CapabilityBinding, CapabilitySpec, PluginManifest, ToolExposure, DEFAULT_TIMEOUT_MS,
};
use crate::agent::permissions::RiskLevel;
use crate::agent::tools::ToolExecutionResult;

pub const CAPABILITY_TIME: &str = "system.get_time";
pub const CAPABILITY_LAUNCH: &str = "system.launch_app";

pub fn manifest() -> PluginManifest {
    PluginManifest {
        id: "system".into(),
        namespace: "system".into(),
        name: "系统".into(),
        version: "0.2.0".into(),
        description: "读取当前系统时间并启动本机应用".into(),
        icon: "system".into(),
        default_scope: true,
        ref_aliases: vec!["系统".into()],
        capabilities: vec![
            CapabilitySpec::new(CAPABILITY_TIME, "读取当前系统时间", RiskLevel::L0),
            CapabilitySpec::new(CAPABILITY_LAUNCH, "启动本机应用", RiskLevel::L1),
        ],
        bindings: vec![
            CapabilityBinding::direct(CAPABILITY_TIME),
            CapabilityBinding::direct(CAPABILITY_LAUNCH),
        ],
        tools: vec![
            ToolExposure {
                name: "get_time".into(),
                description: "读取当前系统时间，返回 UTC Unix 时间戳（秒和毫秒）。仅在问题需要准确的当前时间时使用。".into(),
                input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
                capability_id: CAPABILITY_TIME.into(),
                risk_level: RiskLevel::L0,
                side_effects: vec![],
                legacy_names: vec!["system.get_time".into(), "system_get_time".into()],
                timeout_ms: DEFAULT_TIMEOUT_MS,
            },
            ToolExposure {
                name: "launch_app".into(),
                description: "启动本机应用或打开文件。给出绝对路径 path（.exe 直接启动，.lnk/.url/文档走系统关联），或给出开始菜单里的友好名 name（如“记事本”“Visual Studio Code”）。二者至少给一个，优先 path。不弹确认，直接执行。".into(),
                input_schema: json!({"type":"object","properties":{"path":{"type":"string","description":"应用或文件的绝对路径"},"name":{"type":"string","description":"开始菜单中的友好名，仅在不知道路径时使用"},"args":{"type":"array","items":{"type":"string"},"description":"可选启动参数"}},"additionalProperties":false}),
                capability_id: CAPABILITY_LAUNCH.into(),
                risk_level: RiskLevel::L1,
                side_effects: vec!["启动本机应用或打开文件".into()],
                legacy_names: vec!["system.launch_app".into(), "system_launch_app".into()],
                timeout_ms: DEFAULT_TIMEOUT_MS,
            },
        ],
        resources: vec![],
        events: vec![],
    }
}

pub fn handlers() -> HandlerMap {
    let mut handlers = HandlerMap::new();
    bind(&mut handlers, CAPABILITY_TIME, |_call, _ctx| async move {
        match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(time) => ToolExecutionResult::success(
                "已读取系统时间",
                json!({
                    "unixSeconds": time.as_secs(),
                    "unixMilliseconds": time.as_millis() as u64,
                    "timeZone": "UTC"
                }),
            ),
            Err(_) => ToolExecutionResult::failure("CLOCK_ERROR", "系统时间早于 Unix 起点", false),
        }
    });
    bind(&mut handlers, CAPABILITY_LAUNCH, |call, _ctx| async move {
        let path = call
            .arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let name = call
            .arguments
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let args = call
            .arguments
            .get("args")
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let target = match (path, name) {
            (Some(path), _) => std::path::PathBuf::from(path),
            (None, Some(name)) => match crate::agent::launcher::resolve_by_name(&name) {
                Ok(path) => path,
                Err(error) => {
                    return ToolExecutionResult::failure("APP_NOT_FOUND", error, false)
                }
            },
            (None, None) => {
                return ToolExecutionResult::failure(
                    "INVALID_TOOL_ARGS",
                    "必须提供 path 或 name 之一",
                    false,
                )
            }
        };
        match crate::agent::launcher::launch_path(&target, &args) {
            Ok(outcome) => ToolExecutionResult::success(
                format!("已启动：{}", outcome.target),
                json!({"target": outcome.target, "via": outcome.via}),
            ),
            Err(error) => ToolExecutionResult::failure("LAUNCH_FAILED", error, false),
        }
    });
    handlers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::plugins::host::{HandlerContext, PluginEnv, PluginToolCall, RunScope};
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn time_capability_returns_utc_epoch_fields() {
        let handlers = handlers();
        let handler = handlers.get(CAPABILITY_TIME).unwrap();
        let env = PluginEnv::default();
        let scope = RunScope::manual(&env);
        let ctx = HandlerContext {
            session_id: scope.session_id,
            run_id: scope.run_id,
            cancelled: scope.cancelled,
            env,
        };
        let call = PluginToolCall {
            plugin_id: "system".into(),
            namespace: "system".into(),
            tool_name: "get_time".into(),
            capability_id: CAPABILITY_TIME.into(),
            arguments: json!({}),
            generation: 1,
        };
        let result = handler.call(&call, &ctx).await;
        assert!(result.ok);
        assert!(result.data["unixSeconds"].as_u64().unwrap() > 0);
        assert!(result.data["unixMilliseconds"].as_u64().unwrap() > 0);
        assert_eq!(result.data["timeZone"], "UTC");
        assert!(Arc::strong_count(&ctx.env.control) >= 1);
    }
}
