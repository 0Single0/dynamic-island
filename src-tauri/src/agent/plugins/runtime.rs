//! 插件运行时：发现、校验、挂载、解析工具并统一执行能力。
//!
//! Agent Loop 只能通过 `PluginRuntime::tools_for` / `PluginRuntime::call` 使用业务能力，
//! 不再对具体应用做 `match tool_name`。

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;

use super::host::HandlerMap;
use super::manifest::{
    CapabilityBinding, PluginLifecycle, PluginManifest, PluginStatus, ToolExposure, ToolStatus,
    INTERACTIVE_TIMEOUT_MS,
};
use super::registry::{CapabilityRegistry, HandlerContext, PluginToolCall};
use crate::agent::tools::{ToolDefinition, ToolExecutionResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginError {
    pub code: String,
    pub message: String,
    pub plugin_id: Option<String>,
    pub retryable: bool,
}

impl PluginError {
    pub fn new(
        code: &str,
        message: impl Into<String>,
        plugin_id: Option<&str>,
        retryable: bool,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            plugin_id: plugin_id.map(str::to_owned),
            retryable,
        }
    }

    pub fn to_result(self) -> ToolExecutionResult {
        ToolExecutionResult::failure(self.code, self.message, self.retryable)
    }
}

/// 结构化工具引用：模型名 → 插件 → 内部工具 → 能力。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolRef {
    pub plugin_id: String,
    pub namespace: String,
    pub tool_name: String,
    pub capability_id: String,
    pub generation: u64,
}

struct MountedPlugin {
    manifest: PluginManifest,
    generation: u64,
    lifecycle: PluginLifecycle,
    tools: Vec<ToolExposure>,
}

#[derive(Default)]
struct RuntimeState {
    plugins: HashMap<String, MountedPlugin>,
    /// 模型工具名与历史别名 → 插件 id，用于冲突检测与解析。
    model_names: HashMap<String, String>,
    capabilities: CapabilityRegistry,
    next_generation: u64,
}

/// 进程内插件运行时；启动时挂载全部内置插件后长期可用。
pub struct PluginRuntime {
    state: RwLock<RuntimeState>,
}

impl Default for PluginRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginRuntime {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(RuntimeState {
                next_generation: 1,
                ..RuntimeState::default()
            }),
        }
    }

    /// discover → validate → mount：校验清单、注册能力和工具名。
    pub async fn mount(
        &self,
        manifest: PluginManifest,
        handlers: HandlerMap,
    ) -> Result<u64, PluginError> {
        let mut state = self.state.write().await;
        Self::validate(&manifest, &handlers)?;
        if state.plugins.contains_key(&manifest.id) {
            return Err(PluginError::new(
                "PLUGIN_ALREADY_MOUNTED",
                format!("插件 {} 已挂载", manifest.id),
                Some(&manifest.id),
                false,
            ));
        }
        for name in exposure_names(&manifest) {
            if let Some(owner) = state.model_names.get(&name) {
                return Err(PluginError::new(
                    "TOOL_NAME_CONFLICT",
                    format!("工具名 {name} 已由插件 {owner} 占用"),
                    Some(&manifest.id),
                    false,
                ));
            }
        }
        // 处理器齐备性与能力冲突都在写入前检查：挂载失败不留半注册状态。
        let mut prepared = Vec::with_capacity(manifest.capabilities.len());
        for capability in &manifest.capabilities {
            let handler = handlers.get(&capability.id).cloned();
            // 已实现能力（available）必须有处理器，缺处理器是宿主装配缺陷；
            // 预留能力只登记声明，允许没有处理器且永远不会被暴露或调用。
            if handler.is_none() && capability.available {
                return Err(PluginError::new(
                    "HANDLER_MISSING",
                    format!("能力 {} 缺少处理器", capability.id),
                    Some(&manifest.id),
                    false,
                ));
            }
            if state.capabilities.get(&capability.id).is_some() {
                return Err(PluginError::new(
                    "CAPABILITY_CONFLICT",
                    format!("能力 {} 已注册", capability.id),
                    Some(&manifest.id),
                    false,
                ));
            }
            prepared.push((capability.clone(), handler));
        }
        let generation = state.next_generation;
        state.next_generation += 1;
        for (capability, handler) in prepared {
            state
                .capabilities
                .insert(capability, &manifest.id, generation, handler)
                .map_err(|error| {
                    PluginError::new("CAPABILITY_CONFLICT", error, Some(&manifest.id), false)
                })?;
        }
        for name in exposure_names(&manifest) {
            state.model_names.insert(name, manifest.id.clone());
        }
        let tools = manifest.tools.clone();
        state.plugins.insert(
            manifest.id.clone(),
            MountedPlugin {
                manifest,
                generation,
                lifecycle: PluginLifecycle::Active,
                tools,
            },
        );
        Ok(generation)
    }

    fn validate(manifest: &PluginManifest, _handlers: &HandlerMap) -> Result<(), PluginError> {
        let invalid =
            |message: String| PluginError::new("INVALID_MANIFEST", message, Some(&manifest.id), false);
        if manifest.id.is_empty()
            || !manifest
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(invalid("插件 id 只能包含小写字母、数字和连字符".into()));
        }
        if manifest.namespace.is_empty()
            || !manifest
                .namespace
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(invalid(
                "插件 namespace 只能包含字母、数字、下划线和连字符".into(),
            ));
        }
        let mut capability_ids = HashSet::new();
        for capability in &manifest.capabilities {
            if capability.id.is_empty() || !capability_ids.insert(capability.id.as_str()) {
                return Err(invalid(format!("能力 ID 为空或重复: {}", capability.id)));
            }
        }
        for binding in &manifest.bindings {
            if !capability_ids.contains(binding.capability_id.as_str())
                || binding.handler_id != binding.capability_id
            {
                return Err(invalid(format!(
                    "能力绑定无效或指向未知能力: {}",
                    binding.capability_id
                )));
            }
        }
        let mut names = HashSet::new();
        for tool in &manifest.tools {
            let exposed = manifest.tool_exposed_name(tool);
            let valid = exposed.len() <= 64
                && exposed
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
            if !valid {
                return Err(invalid(format!("工具名不合法或过长: {exposed}")));
            }
            if !names.insert(exposed) {
                return Err(invalid("同一插件内工具名或历史名重复".into()));
            }
            // 历史名只用于解析旧会话/旧客户端发来的调用，不发给模型，因此允许点号。
            for legacy in &tool.legacy_names {
                let valid = !legacy.is_empty()
                    && legacy.len() <= 128
                    && legacy
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
                if !valid {
                    return Err(invalid(format!("历史工具名不合法: {legacy}")));
                }
                if !names.insert(legacy.clone()) {
                    return Err(invalid("同一插件内工具名或历史名重复".into()));
                }
            }
        }
        for tool in &manifest.tools {
            if tool.input_schema["type"] != Value::String("object".into()) {
                return Err(invalid(format!("工具 {} 的参数必须是 JSON 对象", tool.name)));
            }
            if !capability_ids.contains(tool.capability_id.as_str()) {
                return Err(invalid(format!(
                    "工具 {} 引用了未声明能力 {}",
                    tool.name, tool.capability_id
                )));
            }
        }
        Ok(())
    }

    /// 挂载状态快照：UI 只读展示，不含任何启停控件。
    pub async fn statuses(&self) -> Vec<PluginStatus> {
        let state = self.state.read().await;
        let mut statuses = state
            .plugins
            .values()
            .map(|plugin| {
                let manifest = &plugin.manifest;
                PluginStatus {
                    id: manifest.id.clone(),
                    namespace: manifest.namespace.clone(),
                    name: manifest.name.clone(),
                    version: manifest.version.clone(),
                    description: manifest.description.clone(),
                    icon: manifest.icon.clone(),
                    default_scope: manifest.default_scope,
                    ref_aliases: manifest.ref_aliases.clone(),
                    risk: manifest.highest_risk(),
                    lifecycle: plugin.lifecycle,
                    generation: plugin.generation,
                    tools: manifest
                        .tools
                        .iter()
                        .map(|tool| ToolStatus {
                            name: manifest.tool_exposed_name(tool),
                            tool_name: tool.name.clone(),
                            description: tool.description.clone(),
                            capability_id: tool.capability_id.clone(),
                            risk_level: tool.risk_level,
                            legacy_names: tool.legacy_names.clone(),
                            available: capability_available(&state, &tool.capability_id),
                        })
                        .collect(),
                    capabilities: manifest.capabilities.clone(),
                    resources: manifest.resources.clone(),
                    events: manifest.events.clone(),
                }
            })
            .collect::<Vec<_>>();
        statuses.sort_by(|a, b| a.id.cmp(&b.id));
        statuses
    }

    /// 把 `@` 引用与历史别名解析为插件 id；未知引用原样回报。
    pub async fn resolve_refs(&self, refs: &[String]) -> (Vec<String>, Vec<String>) {
        let state = self.state.read().await;
        let mut resolved = Vec::new();
        let mut unknown = Vec::new();
        for raw in refs {
            let key = raw.trim().trim_start_matches('@');
            let found = state
                .plugins
                .values()
                .find(|plugin| {
                    plugin.manifest.id == key
                        || plugin.manifest.namespace == key
                        || plugin.manifest.ref_aliases.iter().any(|alias| alias == key)
                })
                .map(|plugin| plugin.manifest.id.clone());
            match found {
                Some(id) if !resolved.contains(&id) => resolved.push(id),
                Some(_) => {}
                None => unknown.push(raw.clone()),
            }
        }
        (resolved, unknown)
    }

    /// 本轮模型工具：默认范围插件 + `@` 引用的插件；未实现能力不暴露。
    pub async fn tools_for(&self, refs: &[String]) -> Vec<ToolDefinition> {
        let (selected, _) = self.resolve_refs(refs).await;
        let state = self.state.read().await;
        let mut plugins = state.plugins.values().collect::<Vec<_>>();
        plugins.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        plugins
            .into_iter()
            .filter(|plugin| plugin.lifecycle == PluginLifecycle::Active)
            .filter(|plugin| {
                plugin.manifest.default_scope || selected.contains(&plugin.manifest.id)
            })
            .flat_map(|plugin| {
                plugin
                    .manifest
                    .tools
                    .iter()
                    .filter(|tool| capability_available(&state, &tool.capability_id))
                    .map(|tool| to_definition(&plugin.manifest, tool))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// 全部已实现工具（测试与本地诊断用），不区分本轮引用。
    pub async fn all_tools(&self) -> Vec<ToolDefinition> {
        let refs = self
            .statuses()
            .await
            .into_iter()
            .map(|status| status.id)
            .collect::<Vec<_>>();
        self.tools_for(&refs).await
    }

    /// 解析模型工具名：新双下划线名称与历史别名都可用。
    pub async fn resolve(&self, requested: &str) -> Result<ToolRef, PluginError> {
        let state = self.state.read().await;
        let plugin_id = state
            .model_names
            .get(requested)
            .cloned()
            .ok_or_else(|| {
                PluginError::new(
                    "TOOL_NOT_FOUND",
                    format!("工具未注册或不属于任何已挂载插件: {requested}"),
                    None,
                    false,
                )
            })?;
        let plugin = state.plugins.get(&plugin_id).ok_or_else(|| {
            PluginError::new(
                "PLUGIN_UNAVAILABLE",
                format!("插件 {plugin_id} 未挂载"),
                Some(&plugin_id),
                false,
            )
        })?;
        if plugin.lifecycle != PluginLifecycle::Active {
            return Err(PluginError::new(
                "PLUGIN_UNAVAILABLE",
                format!("插件 {} 正在卸载，暂不接受新调用", plugin_id),
                Some(&plugin_id),
                true,
            ));
        }
        let manifest = &plugin.manifest;
        let tool = plugin
            .tools
            .iter()
            .find(|tool| {
                manifest.tool_exposed_name(tool) == requested
                    || tool.legacy_names.iter().any(|legacy| legacy == requested)
            })
            .ok_or_else(|| {
                PluginError::new(
                    "TOOL_NOT_FOUND",
                    format!("插件 {plugin_id} 没有工具 {requested}"),
                    Some(&plugin_id),
                    false,
                )
            })?;
        let capability = state
            .capabilities
            .get(&tool.capability_id)
            .ok_or_else(|| {
                PluginError::new(
                    "PLUGIN_UNAVAILABLE",
                    format!("能力 {} 未注册", tool.capability_id),
                    Some(&plugin_id),
                    true,
                )
            })?;
        if !capability.spec.available {
            return Err(PluginError::new(
                "CAPABILITY_NOT_IMPLEMENTED",
                format!("能力 {} 尚未实现", tool.capability_id),
                Some(&plugin_id),
                false,
            ));
        }
        Ok(ToolRef {
            plugin_id,
            namespace: manifest.namespace.clone(),
            tool_name: tool.name.clone(),
            capability_id: tool.capability_id.clone(),
            generation: capability.generation,
        })
    }

    /// 单次调用的超时预算；需要原生确认的工具留出确认窗口余量。
    pub async fn timeout_for(&self, requested: &str) -> Option<u64> {
        let state = self.state.read().await;
        state.plugins.values().find_map(|plugin| {
            plugin
                .tools
                .iter()
                .find(|tool| {
                    plugin.manifest.tool_exposed_name(tool) == requested
                        || tool.legacy_names.iter().any(|legacy| legacy == requested)
                })
                .map(|tool| {
                    if tool.timeout_ms == 0 {
                        INTERACTIVE_TIMEOUT_MS
                    } else {
                        tool.timeout_ms
                    }
                })
        })
    }

    /// 统一执行入口：解析 → 取处理器 → 带超时调用。
    pub async fn call(
        &self,
        requested: &str,
        arguments: Value,
        ctx: &HandlerContext,
    ) -> ToolExecutionResult {
        let reference = match self.resolve(requested).await {
            Ok(reference) => reference,
            Err(error) => return error.to_result(),
        };
        let timeout_ms = self
            .timeout_for(requested)
            .await
            .unwrap_or(crate::agent::plugins::manifest::DEFAULT_TIMEOUT_MS);
        let handler = {
            let state = self.state.read().await;
            state
                .capabilities
                .get(&reference.capability_id)
                .and_then(|entry| entry.handler.clone())
        };
        let Some(handler) = handler else {
            return PluginError::new(
                "PLUGIN_UNAVAILABLE",
                format!("能力 {} 已卸载", reference.capability_id),
                Some(&reference.plugin_id),
                true,
            )
            .to_result();
        };
        if ctx.is_cancelled() {
            return ToolExecutionResult::failure("CANCELLED", "运行已取消", false);
        }
        let call = PluginToolCall {
            plugin_id: reference.plugin_id.clone(),
            namespace: reference.namespace.clone(),
            tool_name: reference.tool_name.clone(),
            capability_id: reference.capability_id.clone(),
            arguments,
            generation: reference.generation,
        };
        match tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            handler.call(&call, ctx),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => PluginError::new(
                "PLUGIN_TIMEOUT",
                format!(
                    "插件 {} 的工具 {} 超过 {timeout_ms} 毫秒未返回",
                    reference.plugin_id, reference.tool_name
                ),
                Some(&reference.plugin_id),
                true,
            )
            .to_result(),
        }
    }

    /// drain → unmount：停止接受新调用并注销能力与工具名。
    pub async fn unmount(&self, plugin_id: &str) -> Result<(), PluginError> {
        let mut state = self.state.write().await;
        let Some(plugin) = state.plugins.remove(plugin_id) else {
            return Err(PluginError::new(
                "PLUGIN_NOT_MOUNTED",
                format!("插件 {plugin_id} 未挂载"),
                Some(plugin_id),
                false,
            ));
        };
        drop(plugin);
        state.capabilities.remove_plugin(plugin_id);
        state
            .model_names
            .retain(|_, owner| owner.as_str() != plugin_id);
        Ok(())
    }

    /// reload：先 drain 再按新清单重新挂载，generation 递增。
    pub async fn reload(
        &self,
        manifest: PluginManifest,
        handlers: HandlerMap,
    ) -> Result<u64, PluginError> {
        let plugin_id = manifest.id.clone();
        self.unmount(&plugin_id).await?;
        self.mount(manifest, handlers).await
    }

    pub async fn mounted_ids(&self) -> Vec<String> {
        let state = self.state.read().await;
        let mut ids = state.plugins.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        ids
    }

    pub async fn generation(&self, plugin_id: &str) -> Option<u64> {
        let state = self.state.read().await;
        state.plugins.get(plugin_id).map(|plugin| plugin.generation)
    }

    /// 清单声明的能力绑定关系，供诊断与测试核对。
    pub async fn bindings(&self, plugin_id: &str) -> Vec<CapabilityBinding> {
        let state = self.state.read().await;
        state
            .plugins
            .get(plugin_id)
            .map(|plugin| plugin.manifest.bindings.clone())
            .unwrap_or_default()
    }
}

/// 一个插件占用的所有模型可见名称：当前名 + 历史名。
fn exposure_names(manifest: &PluginManifest) -> Vec<String> {
    manifest
        .tools
        .iter()
        .flat_map(|tool| {
            let mut names = vec![manifest.tool_exposed_name(tool)];
            names.extend(tool.legacy_names.iter().cloned());
            names
        })
        .collect()
}

fn capability_available(state: &RuntimeState, capability_id: &str) -> bool {
    state
        .capabilities
        .get(capability_id)
        .map(|entry| entry.spec.available)
        .unwrap_or(false)
}

fn to_definition(manifest: &PluginManifest, tool: &ToolExposure) -> ToolDefinition {
    ToolDefinition {
        name: manifest.tool_exposed_name(tool),
        version: format!("{}@{}", manifest.id, manifest.version),
        description: tool.description.clone(),
        input_schema: tool.input_schema.clone(),
        risk_level: tool.risk_level,
        supports_cancellation: true,
        supports_parallel: false,
        side_effects: tool.side_effects.clone(),
        max_output_chars: 12_000,
        legacy_names: tool.legacy_names.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::permissions::RiskLevel as L;
    use crate::agent::plugins::manifest::{CapabilitySpec, INTERACTIVE_TIMEOUT_MS};
    use crate::agent::plugins::host::{bind, CapabilityHandler, HandlerMap};
    use serde_json::json;
    use std::sync::Arc;

    struct Slow;
    #[async_trait::async_trait]
    impl CapabilityHandler for Slow {
        async fn call(&self, _call: &PluginToolCall, _ctx: &HandlerContext) -> ToolExecutionResult {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            ToolExecutionResult::success("late", json!({}))
        }
    }

    fn echo(handlers: &mut HandlerMap, capability_id: &str, name: &'static str) {
        let id = capability_id.to_owned();
        bind(handlers, &id, move |_call, _ctx| {
            let name = name;
            async move { ToolExecutionResult::success(name, json!({})) }
        });
    }

    fn tool(name: &str, capability_id: &str, legacy: &[&str], timeout_ms: u64) -> ToolExposure {
        ToolExposure {
            name: name.into(),
            description: format!("fixture {name}"),
            input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            capability_id: capability_id.into(),
            risk_level: L::L0,
            side_effects: vec![],
            legacy_names: legacy.iter().map(|v| (*v).into()).collect(),
            timeout_ms,
        }
    }

    fn fixture_manifest(id: &str, namespace: &str, default_scope: bool) -> PluginManifest {
        let capability = format!("{id}.ping");
        PluginManifest {
            id: id.into(),
            namespace: namespace.into(),
            name: id.into(),
            version: "0.1.0".into(),
            description: "fixture".into(),
            icon: "system".into(),
            default_scope,
            ref_aliases: vec![],
            capabilities: vec![CapabilitySpec::new(&capability, "ping", L::L0)],
            bindings: vec![CapabilityBinding::direct(&capability)],
            tools: vec![tool(
                "ping",
                &capability,
                &[format!("{id}.ping")].iter().map(String::as_str).collect::<Vec<_>>(),
                INTERACTIVE_TIMEOUT_MS,
            )],
            resources: vec![],
            events: vec![],
        }
    }

    fn fixture_handlers(id: &str) -> HandlerMap {
        let mut handlers = HandlerMap::new();
        let capability = format!("{id}.ping");
        match id {
            "slow" => {
                handlers.insert(capability, Arc::new(Slow) as Arc<dyn CapabilityHandler>);
            }
            _ => echo(&mut handlers, &capability, "alpha"),
        }
        handlers
    }

    /// 已实现能力与预留能力共存：预留能力只注册，不暴露为工具。
    fn reserved_handlers() -> HandlerMap {
        let mut handlers = HandlerMap::new();
        echo(&mut handlers, "alpha.ping", "alpha");
        echo(&mut handlers, "alpha.screenshot", "screenshot");
        handlers
    }

    fn ctx(cancelled: bool) -> HandlerContext {
        HandlerContext {
            session_id: "session".into(),
            run_id: "run".into(),
            cancelled: Arc::new(move || cancelled),
            env: crate::agent::plugins::host::PluginEnv::default(),
        }
    }

    #[tokio::test]
    async fn mount_is_unique_namespaced_and_idempotent_checked() {
        let runtime = PluginRuntime::new();
        let generation = runtime
            .mount(fixture_manifest("alpha", "alpha", true), fixture_handlers("alpha"))
            .await
            .unwrap();
        assert_eq!(generation, 1);
        let again = runtime
            .mount(fixture_manifest("alpha", "alpha", true), fixture_handlers("alpha"))
            .await
            .unwrap_err();
        assert_eq!(again.code, "PLUGIN_ALREADY_MOUNTED");
        let tools = runtime.tools_for(&[]).await;
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "alpha__ping");
        assert_eq!(tools[0].version, "alpha@0.1.0");
        assert_eq!(runtime.mounted_ids().await, vec!["alpha"]);
        assert_eq!(runtime.bindings("alpha").await.len(), 1);
    }

    #[tokio::test]
    async fn name_conflicts_and_scope_rules_are_enforced() {
        let runtime = PluginRuntime::new();
        runtime
            .mount(fixture_manifest("alpha", "shared", true), fixture_handlers("alpha"))
            .await
            .unwrap();
        let conflict = runtime
            .mount(fixture_manifest("beta", "shared", false), fixture_handlers("beta"))
            .await
            .unwrap_err();
        assert_eq!(conflict.code, "TOOL_NAME_CONFLICT");
        let mut scoped = fixture_manifest("gamma", "gamma", false);
        scoped.ref_aliases = vec!["电脑".into(), "computer-use".into()];
        runtime.mount(scoped, fixture_handlers("gamma")).await.unwrap();
        assert_eq!(runtime.tools_for(&[]).await.len(), 1);
        let selected = runtime.tools_for(&["@电脑".into(), "computer-use".into()]).await;
        assert_eq!(selected.len(), 2);
        assert!(selected.iter().any(|tool| tool.name == "gamma__ping"));
        let (ids, unknown) = runtime
            .resolve_refs(&["电脑".into(), "computer-use".into(), "missing".into()])
            .await;
        assert_eq!(ids, vec!["gamma"]);
        assert_eq!(unknown, vec!["missing"]);
    }

    #[tokio::test]
    async fn calls_route_through_capabilities_with_legacy_names_and_timeout() {
        let runtime = PluginRuntime::new();
        runtime
            .mount(fixture_manifest("alpha", "alpha", true), fixture_handlers("alpha"))
            .await
            .unwrap();
        let result = runtime.call("alpha__ping", json!({}), &ctx(false)).await;
        assert!(result.ok);
        assert_eq!(result.summary, "alpha");
        assert_eq!(result.data, json!({}));
        assert!(runtime.call("alpha.ping", json!({}), &ctx(false)).await.ok);
        assert_eq!(
            runtime
                .call("unknown__ping", json!({}), &ctx(false))
                .await
                .error_code
                .as_deref(),
            Some("TOOL_NOT_FOUND")
        );
        assert_eq!(
            runtime
                .call("alpha__ping", json!({}), &ctx(true))
                .await
                .error_code
                .as_deref(),
            Some("CANCELLED")
        );

        let mut slow = fixture_manifest("slow", "slow", true);
        slow.tools[0].timeout_ms = 20;
        slow.tools[0].legacy_names.clear();
        runtime
            .mount(slow, fixture_handlers("slow"))
            .await
            .unwrap();
        let timed_out = runtime.call("slow__ping", json!({}), &ctx(false)).await;
        assert_eq!(timed_out.error_code.as_deref(), Some("PLUGIN_TIMEOUT"));
        assert!(timed_out.retryable);
    }

    #[tokio::test]
    async fn reserved_capabilities_stay_hidden_and_unmount_clears_registration() {
        let runtime = PluginRuntime::new();
        let mut manifest = fixture_manifest("alpha", "alpha", true);
        manifest
            .capabilities
            .push(CapabilitySpec::reserved("alpha.screenshot", "截图", L::L2));
        manifest.bindings.push(CapabilityBinding::direct("alpha.screenshot"));
        runtime
            .mount(manifest, reserved_handlers())
            .await
            .unwrap();
        assert_eq!(runtime.tools_for(&[]).await.len(), 1);
        let statuses = runtime.statuses().await;
        let reserved = statuses[0]
            .capabilities
            .iter()
            .find(|capability| capability.id == "alpha.screenshot")
            .unwrap();
        assert!(!reserved.available);
        assert_eq!(
            runtime.unmount("missing").await.unwrap_err().code,
            "PLUGIN_NOT_MOUNTED"
        );
        runtime.unmount("alpha").await.unwrap();
        assert!(runtime.statuses().await.is_empty());
        assert!(runtime.mounted_ids().await.is_empty());
        assert_eq!(
            runtime
                .call("alpha__ping", json!({}), &ctx(false))
                .await
                .error_code
                .as_deref(),
            Some("TOOL_NOT_FOUND")
        );
    }

    #[tokio::test]
    async fn reload_increments_generation_and_restores_active_lifecycle() {
        let runtime = PluginRuntime::new();
        let first = runtime
            .mount(fixture_manifest("alpha", "alpha", true), fixture_handlers("alpha"))
            .await
            .unwrap();
        let second = runtime
            .reload(
                fixture_manifest("alpha", "alpha", true),
                fixture_handlers("alpha"),
            )
            .await
            .unwrap();
        assert!(second > first);
        assert_eq!(runtime.generation("alpha").await, Some(second));
        let statuses = runtime.statuses().await;
        assert_eq!(statuses[0].lifecycle, PluginLifecycle::Active);
        assert_eq!(statuses[0].tools[0].name, "alpha__ping");
    }

    #[tokio::test]
    async fn invalid_manifests_are_rejected_before_any_registration() {
        let runtime = PluginRuntime::new();
        let mut bad_namespace = fixture_manifest("alpha", "bad namespace", true);
        bad_namespace.tools[0].legacy_names.clear();
        assert_eq!(
            runtime
                .mount(bad_namespace, fixture_handlers("alpha"))
                .await
                .unwrap_err()
                .code,
            "INVALID_MANIFEST"
        );
        let mut unknown = fixture_manifest("alpha", "alpha", true);
        unknown.tools[0].capability_id = "alpha.missing".into();
        assert_eq!(
            runtime
                .mount(unknown, fixture_handlers("alpha"))
                .await
                .unwrap_err()
                .code,
            "INVALID_MANIFEST"
        );
        let mut missing = fixture_manifest("alpha", "alpha", true);
        missing
            .capabilities
            .push(CapabilitySpec::new("alpha.extra", "extra", L::L0));
        missing.bindings.push(CapabilityBinding::direct("alpha.extra"));
        // 已实现能力必须由宿主提供处理器，缺失属于装配缺陷。
        assert_eq!(
            runtime
                .mount(missing, fixture_handlers("alpha"))
                .await
                .unwrap_err()
                .code,
            "HANDLER_MISSING"
        );
        // 预留能力只声明不实现，允许没有处理器。
        let mut reserved = fixture_manifest("alpha", "alpha", true);
        reserved
            .capabilities
            .push(CapabilitySpec::reserved("alpha.future", "未来能力", L::L2));
        reserved.bindings.push(CapabilityBinding::direct("alpha.future"));
        runtime.mount(reserved, fixture_handlers("alpha")).await.unwrap();
        runtime.unmount("alpha").await.unwrap();
        assert!(runtime.mounted_ids().await.is_empty());
    }
}
