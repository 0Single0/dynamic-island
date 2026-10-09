use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::permissions::RiskLevel;

#[async_trait::async_trait]
pub trait ModelToolExecutor: Sync {
    async fn execute(&self, name: &str, arguments: Value) -> ToolExecutionResult;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    pub version: String,
    pub description: String,
    pub input_schema: Value,
    pub risk_level: RiskLevel,
    pub requires_confirmation: bool,
    pub supports_cancellation: bool,
    pub supports_parallel: bool,
    pub side_effects: Vec<String>,
    pub max_output_chars: u32,
    /// 历史工具名（旧会话/旧客户端用过）：只用于解析收到的调用，不发给模型。
    #[serde(default)]
    pub legacy_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecutionResult {
    pub ok: bool,
    pub summary: String,
    pub data: Value,
    pub display: Option<Value>,
    pub error_code: Option<String>,
    pub retryable: bool,
}

impl ToolExecutionResult {
    pub fn success(summary: impl Into<String>, data: Value) -> Self {
        Self {
            ok: true,
            summary: summary.into(),
            data,
            display: None,
            error_code: None,
            retryable: false,
        }
    }

    pub fn failure(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            ok: false,
            summary: message.into(),
            data: Value::Null,
            display: None,
            error_code: Some(code.into()),
            retryable,
        }
    }
}

/// 只读工具目录命令：内容来自插件运行时，不包含任何启停语义。
#[tauri::command]
pub async fn list_builtin_tools(
    runtime: tauri::State<'_, std::sync::Arc<super::plugins::PluginRuntime>>,
) -> Result<Vec<ToolDefinition>, String> {
    Ok(runtime.all_tools().await)
}
