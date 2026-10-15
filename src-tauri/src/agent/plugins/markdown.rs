use serde::Deserialize;
use serde_json::json;
use tauri::AppHandle;

use crate::agent::tools::ToolExecutionResult;

const MAX_HTML_BYTES: usize = 512 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConvertArgs {
    pub html: String,
    #[serde(default)]
    pub title: Option<String>,
    pub output_path: String,
}

pub async fn convert(
    app: &AppHandle,
    args: ConvertArgs,
    _cancelled: &(dyn Fn() -> bool + Sync),
) -> ToolExecutionResult {
    if args.html.trim().is_empty() || args.html.len() > MAX_HTML_BYTES || args.html.contains('\0') {
        return ToolExecutionResult::failure(
            "INVALID_TOOL_ARGS",
            "HTML 不能为空、不能包含控制字符，且不能超过 512 KiB",
            false,
        );
    }
    let title = args
        .title
        .as_deref()
        .unwrap_or("Markdown PDF")
        .trim()
        .chars()
        .take(120)
        .collect::<String>();
    // 路径由模型给出；不再弹保存窗口。仅做技术校验：绝对路径、.pdf 结尾、不覆盖。
    let path = std::path::PathBuf::from(args.output_path.trim());
    if !path.is_absolute() {
        return ToolExecutionResult::failure("INVALID_PATH", "outputPath 必须是本机绝对路径", false);
    }
    if path.to_string_lossy().starts_with("\\\\")
        || path.to_string_lossy().starts_with("//")
    {
        return ToolExecutionResult::failure("INVALID_PATH", "不支持网络共享路径", false);
    }
    if path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        != "pdf"
    {
        return ToolExecutionResult::failure("INVALID_PATH", "输出文件必须使用 .pdf 扩展名", false);
    }
    if path.exists() {
        return ToolExecutionResult::failure(
            "FILE_EXISTS",
            "目标 PDF 已存在，请换一个文件名",
            false,
        );
    }
    match crate::markdown::markdown_to_pdf(
        app.clone(),
        args.html,
        path.to_string_lossy().into(),
        title,
    )
    .await
    {
        Ok(()) => ToolExecutionResult::success(
            "Markdown 已转换为 PDF",
            json!({ "outputPath": path, "format": "A4" }),
        ),
        Err(error) => ToolExecutionResult::failure("MARKDOWN_PDF_FAILED", error, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pdf_path_must_be_absolute_and_pdf() {
        // 非绝对路径与错误扩展名都在写入前拒绝。
        assert!(!std::path::Path::new("out.pdf").is_absolute());
        let args = ConvertArgs {
            html: "<h1>hi</h1>".into(),
            title: Some("t".into()),
            output_path: "relative/out.pdf".into(),
        };
        assert!(!std::path::PathBuf::from(args.output_path.trim()).is_absolute());
    }
}
