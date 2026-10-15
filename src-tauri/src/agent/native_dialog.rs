//! 运行取消的统一结果。
//!
//! 这里曾承载执行前的原生确认对话框（`confirm` / `wait`）；
//! Agent 现在是全自主执行，不再向用户请求确认，
//! 因此本模块只剩“用户主动停止运行”的取消语义。

use super::tools::ToolExecutionResult;

pub fn cancelled_result() -> ToolExecutionResult {
    ToolExecutionResult::failure("CANCELLED", "运行已停止，未执行后续操作", false)
}
