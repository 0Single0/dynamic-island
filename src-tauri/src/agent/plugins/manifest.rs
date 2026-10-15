//! 插件契约：能力声明、插件清单、工具暴露与生命周期。
//!
//! 业务模块只实现 capability handler；插件清单只描述“如何把已有能力暴露给 Agent”。
//! 模型侧工具名统一为 `<namespace>__<tool>`，旧名称作为 alias 保留解析兼容。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::permissions::RiskLevel;

/// 需要原生对话框的能力自行管理 300 秒确认窗口，运行时超时必须留出余量。
pub const INTERACTIVE_TIMEOUT_MS: u64 = 330_000;
pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

fn default_true() -> bool {
    true
}

/// 稳定能力 ID + 语义；实现由后台业务模块提供。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitySpec {
    pub id: String,
    pub description: String,
    pub risk_level: RiskLevel,
    /// 预留能力（截图、OCR、输入模拟）注册但尚未实现。
    #[serde(default = "default_true")]
    pub available: bool,
}

impl CapabilitySpec {
    pub fn new(id: &str, description: &str, risk_level: RiskLevel) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            risk_level,
            available: true,
        }
    }

    pub fn reserved(id: &str, description: &str, risk_level: RiskLevel) -> Self {
        Self {
            available: false,
            ..Self::new(id, description, risk_level)
        }
    }
}

/// 能力到处理器的绑定：插件不复制业务逻辑，只声明绑定关系。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityBinding {
    pub capability_id: String,
    pub handler_id: String,
}

impl CapabilityBinding {
    /// 默认绑定：能力 ID 即处理器 ID。
    pub fn direct(capability_id: &str) -> Self {
        Self {
            capability_id: capability_id.into(),
            handler_id: capability_id.into(),
        }
    }
}

/// 暴露给模型的工具：名称、参数结构、风险等级和超时。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolExposure {
    /// 插件内工具名，例如 `inspect_window`。
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub capability_id: String,
    pub risk_level: RiskLevel,
    #[serde(default)]
    pub side_effects: Vec<String>,
    /// 历史模型工具名，例如 `computer.inspect_window`、`computer_inspect_window`。
    #[serde(default)]
    pub legacy_names: Vec<String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

/// 资源与事件暴露：第一阶段只描述关系，热插拔时随插件一起挂载和注销。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceExposure {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EventExposure {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PluginLifecycle {
    Discovered,
    Validated,
    Mounted,
    Active,
    Draining,
    Unmounted,
    Failed,
}

/// 插件清单：宿主注册能力的唯一入口。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub id: String,
    /// 模型工具命名空间；未单独指定时等于 id。
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub icon: String,
    /// 无 `@` 引用时默认暴露给模型。
    pub default_scope: bool,
    /// `@`/历史别名到本插件的映射。
    #[serde(default)]
    pub ref_aliases: Vec<String>,
    pub capabilities: Vec<CapabilitySpec>,
    pub bindings: Vec<CapabilityBinding>,
    pub tools: Vec<ToolExposure>,
    #[serde(default)]
    pub resources: Vec<ResourceExposure>,
    #[serde(default)]
    pub events: Vec<EventExposure>,
}

impl PluginManifest {
    pub fn tool_exposed_name(&self, tool: &ToolExposure) -> String {
        format!("{}__{}", self.namespace, tool.name)
    }

    pub fn highest_risk(&self) -> RiskLevel {
        self.tools
            .iter()
            .map(|tool| tool.risk_level)
            .fold(RiskLevel::L0, |current, risk| match (current, risk) {
                (RiskLevel::L3, _) | (_, RiskLevel::L3) => RiskLevel::L3,
                (RiskLevel::L2, _) | (_, RiskLevel::L2) => RiskLevel::L2,
                (RiskLevel::L1, _) | (_, RiskLevel::L1) => RiskLevel::L1,
                _ => RiskLevel::L0,
            })
    }
}

/// 挂载后暴露给 UI 的只读状态；不提供任何启停开关。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginStatus {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub icon: String,
    pub default_scope: bool,
    pub ref_aliases: Vec<String>,
    pub risk: RiskLevel,
    pub lifecycle: PluginLifecycle,
    pub generation: u64,
    pub tools: Vec<ToolStatus>,
    pub capabilities: Vec<CapabilitySpec>,
    pub resources: Vec<ResourceExposure>,
    pub events: Vec<EventExposure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    /// 模型看到的工具名：`<namespace>__<tool>`。
    pub name: String,
    /// 插件内部工具名。
    pub tool_name: String,
    pub description: String,
    pub capability_id: String,
    pub risk_level: RiskLevel,
    pub legacy_names: Vec<String>,
    pub available: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposed_names_use_double_underscore_and_risk_folds_upwards() {
        let manifest = PluginManifest {
            id: "desktop".into(),
            namespace: "computer".into(),
            name: "电脑".into(),
            version: "0.1.0".into(),
            description: String::new(),
            icon: "desktop".into(),
            default_scope: false,
            ref_aliases: vec![],
            capabilities: vec![],
            bindings: vec![],
            tools: vec![ToolExposure {
                name: "inspect_window".into(),
                description: String::new(),
                input_schema: serde_json::json!({"type":"object"}),
                capability_id: "computer.inspect_window".into(),
                risk_level: RiskLevel::L1,
                side_effects: vec![],
                legacy_names: vec!["computer.inspect_window".into()],
                timeout_ms: INTERACTIVE_TIMEOUT_MS,
            }],
            resources: vec![],
            events: vec![],
        };
        assert_eq!(
            manifest.tool_exposed_name(&manifest.tools[0]),
            "computer__inspect_window"
        );
        assert_eq!(manifest.highest_risk(), RiskLevel::L1);
        assert!(!CapabilitySpec::reserved("computer.screenshot", "截图", RiskLevel::L2).available);
    }
}
