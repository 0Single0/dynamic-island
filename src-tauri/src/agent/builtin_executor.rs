//! Agent 工具执行器：只负责把模型调用转发给插件运行时。
//!
//! 这里不再有任何应用专用分支；业务能力由 `plugins` 下的清单 + capability 注册提供。

use std::sync::Arc;

use serde_json::Value;

use super::plugins::host::{PluginEnv, RunScope};
use super::plugins::registry::HandlerContext;
use super::plugins::PluginRuntime;
use super::tools::{ModelToolExecutor, ToolExecutionResult};

pub struct BuiltinToolExecutor {
    runtime: Arc<PluginRuntime>,
    env: PluginEnv,
    scope: RunScope,
}

impl BuiltinToolExecutor {
    pub fn new(runtime: Arc<PluginRuntime>, env: PluginEnv, scope: RunScope) -> Self {
        Self {
            runtime,
            env,
            scope,
        }
    }
}

#[async_trait::async_trait]
impl ModelToolExecutor for BuiltinToolExecutor {
    async fn execute(&self, name: &str, arguments: Value) -> ToolExecutionResult {
        let ctx = HandlerContext {
            session_id: self.scope.session_id.clone(),
            run_id: self.scope.run_id.clone(),
            cancelled: Arc::clone(&self.scope.cancelled),
            env: self.env.clone(),
        };
        self.runtime.call(name, arguments, &ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::plugins::host::{bind, HandlerMap};
    use crate::agent::plugins::manifest::{
        CapabilityBinding, CapabilitySpec, PluginManifest, ToolExposure, DEFAULT_TIMEOUT_MS,
    };
    use crate::agent::permissions::RiskLevel;
    use serde_json::json;

    fn runtime_with_fixture() -> Arc<PluginRuntime> {
        let mut handlers = HandlerMap::new();
        bind(&mut handlers, "fixture.echo", |call, _ctx| async move {
            ToolExecutionResult::success("已转发到能力处理器", call.arguments)
        });
        let manifest = PluginManifest {
            id: "fixture".into(),
            namespace: "fixture".into(),
            name: "fixture".into(),
            version: "0.1.0".into(),
            description: String::new(),
            icon: "system".into(),
            default_scope: true,
            ref_aliases: vec![],
            capabilities: vec![CapabilitySpec::new("fixture.echo", "echo", RiskLevel::L0)],
            bindings: vec![CapabilityBinding::direct("fixture.echo")],
            tools: vec![ToolExposure {
                name: "echo".into(),
                description: "echo".into(),
                input_schema: json!({"type":"object","properties":{"value":{"type":"string"}},"additionalProperties":false}),
                capability_id: "fixture.echo".into(),
                risk_level: RiskLevel::L0,
                requires_confirmation: false,
                side_effects: vec![],
                legacy_names: vec!["fixture.echo".into()],
                timeout_ms: DEFAULT_TIMEOUT_MS,
            }],
            resources: vec![],
            events: vec![],
        };
        // `#[tokio::test]` 内部不能再调用 block_on，把挂载放到独立线程执行。
        std::thread::spawn(move || {
            let runtime = Arc::new(PluginRuntime::new());
            tauri::async_runtime::block_on(runtime.mount(manifest, handlers)).unwrap();
            runtime
        })
        .join()
        .expect("fixture mount thread panicked")
    }

    #[tokio::test]
    async fn executor_forwards_new_and_legacy_names_without_app_branches() {
        let env = PluginEnv::default();
        let scope = RunScope::manual(&env);
        let executor = BuiltinToolExecutor::new(runtime_with_fixture(), env, scope);
        let result = executor
            .execute("fixture__echo", json!({"value":"a"}))
            .await;
        assert!(result.ok);
        assert_eq!(result.summary, "已转发到能力处理器");
        assert_eq!(result.data["value"], "a");
        assert!(executor
            .execute("fixture.echo", json!({"value":"b"}))
            .await
            .ok);
        let unknown = executor.execute("netease.control", json!({})).await;
        assert_eq!(unknown.error_code.as_deref(), Some("TOOL_NOT_FOUND"));
    }
}
