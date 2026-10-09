use serde::Deserialize;
use serde_json::json;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use super::super::{native_dialog, tools::ToolExecutionResult};

const MAX_HTML_BYTES: usize = 512 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConvertArgs {
    pub html: String,
    #[serde(default)]
    pub title: Option<String>,
}

pub async fn convert(
    app: &AppHandle,
    args: ConvertArgs,
    cancelled: &(dyn Fn() -> bool + Sync),
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
    if let Err(error) = native_dialog::confirm(
        app,
        format!(
            "准备将 Markdown 内容打印为 PDF。\n\n标题：{}\n内容预览：\n{}\n\n确认后选择保存位置。不会覆盖已有文件。",
            if title.is_empty() { "Markdown PDF" } else { &title },
            args.html.chars().take(1200).collect::<String>()
        ),
        cancelled,
    )
    .await
    {
        return error;
    }

    let (send, receive) = tokio::sync::oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_title("保存 Markdown PDF（不覆盖）")
        .set_file_name(if title.is_empty() {
            "markdown.pdf".to_owned()
        } else {
            format!("{}.pdf", safe_filename(&title))
        });
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    dialog.save_file(move |path| {
        let _ = send.send(path);
    });
    let path = match native_dialog::wait(receive, cancelled).await {
        Ok(Some(path)) => match path.into_path() {
            Ok(path) => path,
            Err(_) => {
                return ToolExecutionResult::failure("INVALID_PATH", "无法解析 PDF 保存路径", false)
            }
        },
        Ok(None) => {
            return ToolExecutionResult::failure("USER_DENIED", "用户取消保存，未生成 PDF", false)
        }
        Err(error) => return error,
    };
    if cancelled() {
        return native_dialog::cancelled_result();
    }
    if path.exists() {
        return ToolExecutionResult::failure(
            "FILE_EXISTS",
            "目标 PDF 已存在，为避免覆盖请重新选择文件名",
            false,
        );
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

fn safe_filename(value: &str) -> String {
    let filtered = value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>()
        .trim_matches([' ', '.'])
        .to_owned();
    if filtered.is_empty() {
        "markdown".into()
    } else {
        filtered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filename_is_safe() {
        assert_eq!(safe_filename("a:b?.md"), "a_b_.md");
        assert_eq!(safe_filename("..."), "markdown");
    }
}
