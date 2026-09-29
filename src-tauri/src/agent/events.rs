use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    RunStarted {
        run_id: String,
        session_id: String,
    },
    RunStatus {
        run_id: String,
        phase: String,
        title: String,
        detail: Option<String>,
    },
    MessageDelta {
        run_id: String,
        message_id: String,
        delta: String,
    },
    ApprovalRequired {
        run_id: String,
        approval_id: String,
        tool_name: String,
        risk: String,
    },
    RunCompleted {
        run_id: String,
        message_id: String,
    },
    RunCancelled {
        run_id: String,
    },
    RunFailed {
        run_id: String,
        message: String,
    },
}
