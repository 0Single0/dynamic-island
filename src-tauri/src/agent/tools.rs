use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::permissions::RiskLevel;

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

pub fn builtin_tools() -> Vec<ToolDefinition> {
    vec![
        tool(
            "netease.get_current_state",
            "读取网易云当前歌曲、歌词和播放进度",
            RiskLevel::L0,
            false,
            json!({"type":"object","properties":{}}),
            vec![],
        ),
        tool(
            "netease.control",
            "控制网易云播放、暂停、上一首或下一首",
            RiskLevel::L1,
            false,
            json!({"type":"object","required":["action"],"properties":{"action":{"type":"string","enum":["play","pause","next","previous"]}}}),
            vec!["修改播放状态"],
        ),
        tool(
            "netease.seek",
            "跳转网易云播放进度",
            RiskLevel::L1,
            false,
            json!({"type":"object","required":["positionMs"],"properties":{"positionMs":{"type":"integer","minimum":0}}}),
            vec!["修改播放进度"],
        ),
        tool(
            "markdown.read",
            "读取用户选中的 Markdown 文件",
            RiskLevel::L0,
            false,
            json!({"type":"object","required":["path"],"properties":{"path":{"type":"string"}}}),
            vec![],
        ),
        tool(
            "markdown.convert_to_pdf",
            "使用 WebView2 将 Markdown 渲染并打印为 PDF",
            RiskLevel::L1,
            false,
            json!({"type":"object","required":["html","outputPath"],"properties":{"html":{"type":"string"},"outputPath":{"type":"string"},"title":{"type":"string"}}}),
            vec!["创建 PDF 文件"],
        ),
        tool(
            "markdown.open_output",
            "打开生成的 PDF 文件",
            RiskLevel::L1,
            false,
            json!({"type":"object","required":["path"],"properties":{"path":{"type":"string"}}}),
            vec!["打开本地文件"],
        ),
    ]
}

fn tool(
    name: &str,
    description: &str,
    risk_level: RiskLevel,
    requires_confirmation: bool,
    input_schema: Value,
    side_effects: Vec<&str>,
) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        version: "0.1.0".into(),
        description: description.into(),
        input_schema,
        risk_level,
        requires_confirmation,
        supports_cancellation: true,
        supports_parallel: false,
        side_effects: side_effects.into_iter().map(str::to_owned).collect(),
        max_output_chars: 12_000,
    }
}

#[tauri::command]
pub fn list_builtin_tools() -> Vec<ToolDefinition> {
    builtin_tools()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtins_have_schema_and_risk_metadata() {
        let tools = builtin_tools();
        assert!(tools
            .iter()
            .any(|tool| tool.name == "netease.get_current_state"));
        assert!(tools
            .iter()
            .all(|tool| tool.input_schema["type"] == "object"));
        assert!(tools.iter().all(|tool| tool.max_output_chars > 0));
    }
}
