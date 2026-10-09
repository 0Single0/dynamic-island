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

pub fn manifest() -> PluginManifest {
    PluginManifest {
        id: "system".into(),
        namespace: "system".into(),
        name: "系统".into(),
        version: "0.1.0".into(),
        description: "读取当前系统时间".into(),
        icon: "system".into(),
        default_scope: true,
        ref_aliases: vec!["系统".into()],
        capabilities: vec![CapabilitySpec::new(
            CAPABILITY_TIME,
            "读取当前系统时间",
            RiskLevel::L0,
        )],
        bindings: vec![CapabilityBinding::direct(CAPABILITY_TIME)],
        tools: vec![ToolExposure {
            name: "get_time".into(),
            description: "读取当前系统时间，返回 UTC Unix 时间戳（秒和毫秒）。仅在问题需要准确的当前时间时使用。".into(),
            input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            capability_id: CAPABILITY_TIME.into(),
            risk_level: RiskLevel::L0,
            requires_confirmation: false,
            side_effects: vec![],
            legacy_names: vec!["system.get_time".into(), "system_get_time".into()],
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }],
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
