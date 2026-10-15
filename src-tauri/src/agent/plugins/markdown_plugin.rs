//! Markdown 插件：把 WebView2 打印 PDF 的既有实现暴露为能力。
//!
//! 渲染与打印逻辑仍在 `crate::markdown`，此处只做清单与参数校验。

use serde_json::json;

use super::host::{app, bind, HandlerMap};
use super::manifest::{
    CapabilityBinding, CapabilitySpec, PluginManifest, ToolExposure, INTERACTIVE_TIMEOUT_MS,
};
use super::markdown as service;
use crate::agent::permissions::RiskLevel;
use crate::agent::tools::ToolExecutionResult;

pub const CAPABILITY_PDF: &str = "markdown.convert_to_pdf";

const MAX_HTML_BYTES: usize = 512 * 1024;

pub fn manifest() -> PluginManifest {
    PluginManifest {
        id: "markdown".into(),
        namespace: "markdown".into(),
        name: "Markdown 转 PDF".into(),
        version: "0.2.0".into(),
        description: "把 Markdown 文件转换为 PDF".into(),
        icon: "markdown".into(),
        default_scope: false,
        ref_aliases: vec!["markdown".into(), "markdown-pdf".into()],
        capabilities: vec![CapabilitySpec::new(
            CAPABILITY_PDF,
            "把 Markdown HTML 打印为 PDF",
            RiskLevel::L1,
        )],
        bindings: vec![CapabilityBinding::direct(CAPABILITY_PDF)],
        tools: vec![ToolExposure {
            name: "convert_to_pdf".into(),
            description:
                "将 Markdown HTML 渲染并打印为 PDF，写入绝对路径 outputPath。需绝对路径、.pdf 结尾且目标不存在；不覆盖已有文件。"
                    .into(),
            input_schema: json!({"type":"object","required":["html","outputPath"],"properties":{"html":{"type":"string","maxLength":524288},"title":{"type":"string","maxLength":120},"outputPath":{"type":"string","description":"本机绝对路径，以 .pdf 结尾"}},"additionalProperties":false}),
            capability_id: CAPABILITY_PDF.into(),
            risk_level: RiskLevel::L1,
            side_effects: vec!["创建 PDF 文件".into()],
            legacy_names: vec!["markdown.convert_to_pdf".into(), "markdown_convert_to_pdf".into()],
            timeout_ms: INTERACTIVE_TIMEOUT_MS,
        }],
        resources: vec![],
        events: vec![],
    }
}

pub fn handlers() -> HandlerMap {
    let mut handlers = HandlerMap::new();
    bind(&mut handlers, CAPABILITY_PDF, |call, ctx| async move {
        // 先做参数校验再要求宿主：非法参数在任何环境下都能得到明确错误。
        let args = match serde_json::from_value::<service::ConvertArgs>(call.arguments.clone()) {
            Ok(args) => args,
            Err(_) => {
                return ToolExecutionResult::failure(
                    "INVALID_TOOL_ARGS",
                    "参数类型或字段不符合工具定义",
                    false,
                )
            }
        };
        if args.html.trim().is_empty()
            || args.html.len() > MAX_HTML_BYTES
            || args.html.contains('\0')
        {
            return ToolExecutionResult::failure(
                "INVALID_TOOL_ARGS",
                "HTML 不能为空、不能包含控制字符，且不能超过 512 KiB",
                false,
            );
        }
        let app = match app(&ctx) {
            Ok(app) => app,
            Err(error) => return error,
        };
        service::convert(&app, args, ctx.cancelled.as_ref()).await
    });
    handlers
}
