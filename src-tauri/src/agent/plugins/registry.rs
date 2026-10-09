//! 能力注册表：稳定 capability ID 到业务处理器的映射。
//!
//! 业务模块（netease / markdown / computer / files / web-search）只在这里注册实现；
//! 插件清单负责把能力暴露为模型工具，Agent Core 不感知具体应用。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::host::PluginEnv;
use super::manifest::CapabilitySpec;
use crate::agent::tools::ToolExecutionResult;

/// 单次调用的运行上下文；处理器不持有 run 状态。
#[derive(Clone)]
pub struct HandlerContext {
    pub session_id: String,
    pub run_id: String,
    pub cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
    pub env: PluginEnv,
}

impl HandlerContext {
    pub fn is_cancelled(&self) -> bool {
        (self.cancelled)()
    }
}

/// 一次解析完成的插件工具调用。
#[derive(Clone)]
pub struct PluginToolCall {
    pub plugin_id: String,
    pub namespace: String,
    pub tool_name: String,
    pub capability_id: String,
    pub arguments: Value,
    pub generation: u64,
}

impl PluginToolCall {
    /// 与审批快照一致的稳定参数哈希（键排序后计算）。
    pub fn args_hash(&self) -> String {
        let normalized = match self.arguments.as_object() {
            Some(object) => {
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort();
                keys.into_iter()
                    .map(|key| format!("{key}={}", object[key]))
                    .collect::<Vec<_>>()
                    .join(",")
            }
            None => self.arguments.to_string(),
        };
        crate::agent::permissions::args_hash(&normalized)
    }
}

#[async_trait::async_trait]
pub trait CapabilityHandler: Send + Sync {
    async fn call(&self, call: &PluginToolCall, ctx: &HandlerContext) -> ToolExecutionResult;
}

struct RegisteredCapability {
    spec: CapabilitySpec,
    plugin_id: String,
    generation: u64,
    /// 预留能力（available=false）只登记声明，允许没有处理器。
    handler: Option<Arc<dyn CapabilityHandler>>,
}

pub struct RegisteredEntry<'a> {
    pub spec: &'a CapabilitySpec,
    pub plugin_id: &'a str,
    pub generation: u64,
    pub handler: Option<Arc<dyn CapabilityHandler>>,
}

/// 只由 PluginRuntime 持有的注册表；挂载与卸载都走运行时。
#[derive(Default)]
pub struct CapabilityRegistry {
    entries: HashMap<String, RegisteredCapability>,
}

impl CapabilityRegistry {
    /// 注册一个能力；同名能力重复注册会被拒绝。
    pub fn insert(
        &mut self,
        spec: CapabilitySpec,
        plugin_id: &str,
        generation: u64,
        handler: Option<Arc<dyn CapabilityHandler>>,
    ) -> Result<(), String> {
        if let Some(existing) = self.entries.get(&spec.id) {
            return Err(format!(
                "能力 {} 已由插件 {} 注册",
                spec.id, existing.plugin_id
            ));
        }
        self.entries.insert(
            spec.id.clone(),
            RegisteredCapability {
                spec,
                plugin_id: plugin_id.into(),
                generation,
                handler,
            },
        );
        Ok(())
    }

    pub fn get(&self, capability_id: &str) -> Option<RegisteredEntry<'_>> {
        self.entries.get(capability_id).map(|entry| RegisteredEntry {
            spec: &entry.spec,
            plugin_id: &entry.plugin_id,
            generation: entry.generation,
            handler: entry.handler.clone(),
        })
    }

    pub fn remove_plugin(&mut self, plugin_id: &str) -> Vec<String> {
        let removed = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.plugin_id == plugin_id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in &removed {
            self.entries.remove(id);
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::permissions::RiskLevel;
    use serde_json::json;

    struct Fixture;
    #[async_trait::async_trait]
    impl CapabilityHandler for Fixture {
        async fn call(&self, call: &PluginToolCall, _ctx: &HandlerContext) -> ToolExecutionResult {
            ToolExecutionResult::success("fixture", json!({"tool": call.tool_name}))
        }
    }

    fn handler() -> Arc<dyn CapabilityHandler> {
        Arc::new(Fixture)
    }

    fn spec(id: &str) -> CapabilitySpec {
        CapabilitySpec::new(id, "fixture", RiskLevel::L1)
    }

    #[test]
    fn registration_is_unique_and_removal_is_scoped_to_the_plugin() {
        let mut registry = CapabilityRegistry::default();
        registry
            .insert(spec("music.player.control"), "netease", 1, Some(handler()))
            .unwrap();
        assert!(registry
            .insert(spec("music.player.control"), "other", 2, Some(handler()))
            .is_err());
        registry
            .insert(spec("computer.invoke"), "desktop", 1, Some(handler()))
            .unwrap();
        // 预留能力只登记声明，允许没有处理器。
        registry
            .insert(
                CapabilitySpec::reserved("computer.screenshot", "截图", RiskLevel::L2),
                "desktop",
                2,
                None,
            )
            .unwrap();
        assert_eq!(
            registry.remove_plugin("netease"),
            vec!["music.player.control"]
        );
        assert!(registry.get("music.player.control").is_none());
        let entry = registry.get("computer.invoke").unwrap();
        assert_eq!(entry.plugin_id, "desktop");
        assert_eq!(entry.generation, 1);
        assert!(entry.handler.is_some());
        let reserved = registry.get("computer.screenshot").unwrap();
        assert!(!reserved.spec.available);
        assert!(reserved.handler.is_none());
        assert_eq!(registry.remove_plugin("desktop").len(), 2);
    }

    #[test]
    fn argument_hash_is_order_independent_and_value_sensitive() {
        let call = PluginToolCall {
            plugin_id: "files".into(),
            namespace: "files".into(),
            tool_name: "read".into(),
            capability_id: "files.read".into(),
            arguments: json!({"b":2,"a":1}),
            generation: 1,
        };
        let reordered = PluginToolCall {
            arguments: json!({"a":1,"b":2}),
            ..call.clone()
        };
        assert_eq!(call.args_hash(), reordered.args_hash());
        assert_ne!(
            call.args_hash(),
            PluginToolCall {
                arguments: json!({"a":1,"b":3}),
                ..call.clone()
            }
            .args_hash()
        );
        assert!(call.args_hash().starts_with("args:"));
    }
}
