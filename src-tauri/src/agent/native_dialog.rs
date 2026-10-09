use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

use super::tools::ToolExecutionResult;

pub async fn wait<T>(
    mut receive: tokio::sync::oneshot::Receiver<T>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<T, ToolExecutionResult> {
    let deadline = tokio::time::sleep(Duration::from_secs(300));
    tokio::pin!(deadline);
    let mut check = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            result = &mut receive => {
                if cancelled() { return Err(cancelled_result()); }
                return result.map_err(|_| ToolExecutionResult::failure("DIALOG_CLOSED", "确认窗口已关闭", false));
            },
            _ = &mut deadline => return Err(ToolExecutionResult::failure("APPROVAL_EXPIRED", "等待用户确认超时，未执行操作", false)),
            _ = check.tick() => if cancelled() { return Err(cancelled_result()); },
        }
    }
}

pub fn cancelled_result() -> ToolExecutionResult {
    ToolExecutionResult::failure("CANCELLED", "运行已停止，未执行后续操作", false)
}

pub async fn confirm(
    app: &AppHandle,
    message: String,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), ToolExecutionResult> {
    if cancelled() {
        return Err(cancelled_result());
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    let mut dialog = app
        .dialog()
        .message(message)
        .title("确认 Agent 操作")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "允许本次".into(),
            "取消".into(),
        ));
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.parent(&window);
    }
    dialog.show(move |allowed| {
        let _ = send.send(allowed);
    });
    if wait(receive, cancelled).await? {
        Ok(())
    } else {
        Err(ToolExecutionResult::failure(
            "USER_DENIED",
            "用户拒绝本次操作。不要重复请求或尝试绕过。",
            false,
        ))
    }
}
