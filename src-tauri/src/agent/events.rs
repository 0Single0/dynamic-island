use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionEvent {
    pub version: u32,
    pub id: String,
    pub session_id: String,
    pub run_id: String,
    pub step: Option<u32>,
    pub block_id: Option<String>,
    pub call_id: Option<String>,
    pub parent_call_id: Option<String>,
    pub seq: i64,
    pub event_type: String,
    pub payload: Value,
    pub created_at: String,
}

#[derive(Default)]
pub struct EventDraft {
    pub step: Option<u32>,
    pub block_id: Option<String>,
    pub call_id: Option<String>,
    pub parent_call_id: Option<String>,
    pub event_type: String,
    pub payload: Value,
}

impl EventDraft {
    pub fn new(event_type: &str, payload: Value) -> Self {
        Self {
            event_type: event_type.into(),
            payload,
            ..Self::default()
        }
    }
}

pub fn publish(
    app: &tauri::AppHandle,
    store: &super::storage::AgentStore,
    session_id: &str,
    run_id: &str,
    draft: EventDraft,
) -> Result<ExecutionEvent, String> {
    use tauri::Emitter;
    let event = store
        .append_event(session_id, run_id, draft)
        .map_err(|e| e.to_string())?;
    // Persistence is authoritative. Missed broadcasts are replayed by sequence.
    let _ = app.emit("agent://trace", &event);
    Ok(event)
}
