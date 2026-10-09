mod adapter;
mod builtin_executor;
pub mod config;
pub mod events;
pub mod filesystem;
pub mod limits;
mod model;
pub(crate) mod native_dialog;
mod native_trace;
pub mod permissions;
pub mod plugins;
pub mod provider_error;
#[cfg(test)]
mod provider_probe;
pub mod providers;
pub mod search;
pub mod secrets;
pub mod security;
pub mod storage;
mod stream;
#[cfg(test)]
mod tests;
pub mod tools;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, State};

use crate::agent::permissions::{args_hash, ApprovalSnapshot, PermissionPolicy, RiskLevel};
use crate::agent::builtin_executor::BuiltinToolExecutor;
use crate::agent::plugins::host::{self as plugin_host, PluginEnv};
use crate::agent::plugins::PluginRuntime;
use crate::agent::providers::ProviderProtocol;
use crate::agent::secrets::SecretStore;
use crate::agent::storage::{AgentStore, MessageRecord, SessionRecord, TimelineRecord};
use crate::agent::tools::ToolExecutionResult;
use crate::computer;
use crate::netease::SharedNetease;

pub struct AgentState {
    pub store: AgentStore,
    /// 进程内插件运行时：启动时挂载全部内置插件后长期可用。
    pub plugins: Arc<PluginRuntime>,
    /// Windows UI Automation 短期目标注册表；跨调用共享以便窗口/元素 ID 复用。
    pub targets: Arc<Mutex<computer::Targets>>,
    cancelled_runs: RunSet,
    pending_approvals: Mutex<HashMap<String, ApprovalSnapshot>>,
}

/// 取消集合共享句柄：插件环境与 Agent 循环看到同一份状态。
#[derive(Clone, Default)]
pub(crate) struct RunSet(Arc<Mutex<HashSet<String>>>);

impl RunSet {
    fn insert(&self, run_id: &str) {
        if let Ok(mut runs) = self.0.lock() {
            runs.insert(run_id.to_owned());
        }
    }

    fn remove(&self, run_id: &str) {
        if let Ok(mut runs) = self.0.lock() {
            runs.remove(run_id);
        }
    }

    fn contains(&self, run_id: &str) -> bool {
        self.0
            .lock()
            .map(|runs| runs.contains(run_id))
            .unwrap_or(true)
    }
}

impl plugin_host::RunControl for RunSet {
    fn cancelled(&self, run_id: &str) -> bool {
        self.contains(run_id)
    }
}

impl AgentState {
    pub fn initialize(app_data_dir: &std::path::Path) -> Result<Self, String> {
        let store = AgentStore::open(app_data_dir).map_err(|e| e.to_string())?;
        store
            .recover_interrupted_runs()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            store,
            plugins: Arc::new(PluginRuntime::new()),
            targets: Arc::new(Mutex::new(computer::Targets::default())),
            cancelled_runs: RunSet::default(),
            pending_approvals: Mutex::new(HashMap::new()),
        })
    }

    /// 运行取消查询的共享句柄，插件环境与 Agent 循环共用同一份状态。
    pub(crate) fn run_control(&self) -> RunSet {
        self.cancelled_runs.clone()
    }

    pub(crate) fn is_cancelled(&self, run_id: &str) -> bool {
        self.cancelled_runs.contains(run_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageInput {
    pub session_id: String,
    pub content: String,
    #[serde(default)]
    pub plugin_ids: Vec<String>,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunResult {
    pub run_id: String,
    pub message: MessageRecord,
    pub status: String,
}

#[tauri::command]
pub fn agent_list_sessions(state: State<'_, AgentState>) -> Result<Vec<SessionRecord>, String> {
    state
        .store
        .list_sessions()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn agent_create_session(
    state: State<'_, AgentState>,
    title: Option<String>,
) -> Result<SessionRecord, String> {
    state
        .store
        .create_session(title.as_deref().unwrap_or("新会话"))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn agent_delete_session(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<bool, String> {
    state
        .store
        .delete_session(&session_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn agent_list_messages(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<Vec<MessageRecord>, String> {
    state
        .store
        .list_messages(&session_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn agent_list_timeline(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<Vec<TimelineRecord>, String> {
    state
        .store
        .list_timeline(&session_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn agent_list_events(
    state: State<'_, AgentState>,
    session_id: String,
    after_seq: Option<i64>,
) -> Result<Vec<events::ExecutionEvent>, String> {
    state
        .store
        .list_events(&session_id, after_seq.unwrap_or(0).max(0))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn agent_send_message(
    app: AppHandle,
    state: State<'_, AgentState>,
    netease: State<'_, SharedNetease>,
    input: SendMessageInput,
) -> Result<AgentRunResult, provider_error::ProviderError> {
    use events::EventDraft;
    use provider_error::ProviderError;
    use serde_json::json;
    let local =
        |category: &str, message: String| ProviderError::local(category, message, "", "", "");
    let content = input.content.trim();
    if content.is_empty() {
        return Err(local("INVALID_INPUT", "消息不能为空".into()));
    }
    state
        .store
        .seed_context(&input.session_id)
        .map_err(|e| local("STORAGE", e.to_string()))?;
    let run_id = input.run_id.clone().unwrap_or_else(|| new_id("run"));
    state
        .store
        .create_run(&run_id, &input.session_id)
        .map_err(|e| local("STORAGE", e.to_string()))?;
    let trace = native_trace::DurableTrace {
        store: &state.store,
        app: Some(&app),
        session_id: &input.session_id,
        run_id: &run_id,
    };
    let result: Result<AgentRunResult, ProviderError> = async {
        let user = state
            .store
            .add_message(&input.session_id, "user", content)
            .map_err(|e| local("STORAGE", e.to_string()))?;
        state
            .store
            .append_context(
                &input.session_id,
                &run_id,
                &[model::ModelMessage::text("user", content)],
            )
            .map_err(|e| local("STORAGE", e.to_string()))?;
        trace
            .emit(EventDraft::new("run_started", json!({})))
            .map_err(|e| local("STORAGE", e))?;
        trace
            .emit(EventDraft::new(
                "user_message",
                json!({"messageId":user.id,"text":content}),
            ))
            .map_err(|e| local("STORAGE", e))?;
        let text = resolve_provider_request(&input, &state, &netease, &trace).await?;
        let message = state
            .store
            .add_message(&input.session_id, "assistant", &text)
            .map_err(|e| local("STORAGE", e.to_string()))?;
        trace
            .emit(EventDraft::new(
                "run_completed",
                json!({"messageId":message.id}),
            ))
            .map_err(|e| local("STORAGE", e))?;
        state
            .store
            .finish_run(&run_id, "completed", None)
            .map_err(|e| local("STORAGE", e.to_string()))?;
        Ok(AgentRunResult {
            run_id: run_id.clone(),
            message,
            status: "completed".into(),
        })
    }
    .await;
    if let Err(error) = &result {
        let cancelled = error.category == "CANCELLED";
        let _ = trace.emit(EventDraft::new(
            if cancelled {
                "run_cancelled"
            } else {
                "run_failed"
            },
            json!({"message":error.to_string(),"error":error}),
        ));
        let _ = state.store.finish_run(
            &run_id,
            if cancelled { "cancelled" } else { "failed" },
            Some(&error.category),
        );
    }
    state.cancelled_runs.remove(&run_id);
    result
}

#[tauri::command]
pub fn agent_cancel_run(state: State<'_, AgentState>, run_id: String) -> Result<(), String> {
    state.cancelled_runs.insert(&run_id);
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalInput {
    pub run_id: String,
    pub tool_name: String,
    pub args_json: String,
    pub risk: RiskLevel,
}

#[tauri::command]
pub fn agent_preview_approval(
    state: State<'_, AgentState>,
    input: ApprovalInput,
) -> Result<ApprovalSnapshot, String> {
    let policy = PermissionPolicy::default();
    if !policy.requires_confirmation(input.risk) {
        return Err("该风险等级按当前策略无需审批".into());
    }
    let normalized_args = serde_json::from_str::<Value>(&input.args_json)
        .map(|value| value.to_string())
        .unwrap_or_else(|_| input.args_json.clone());
    let snapshot = ApprovalSnapshot {
        id: new_id("approval"),
        run_id: input.run_id,
        tool_name: input.tool_name,
        args_hash: args_hash(&normalized_args),
        args_json: input.args_json,
        risk: input.risk,
        expires_at: (unix_seconds() + 300).to_string(),
        status: "pending".into(),
    };
    state
        .pending_approvals
        .lock()
        .map_err(|_| "审批状态不可用".to_string())?
        .insert(snapshot.id.clone(), snapshot.clone());
    Ok(snapshot)
}

/// 统一执行入口：所有内置工具（含手动触发）都经插件运行时转发到 capability。
#[tauri::command]
pub async fn agent_execute_builtin_tool(
    app: AppHandle,
    state: State<'_, AgentState>,
    tool_name: String,
    arguments: Value,
) -> Result<ToolExecutionResult, String> {
    use tools::ModelToolExecutor;
    let env = PluginEnv {
        app: Some(app),
        control: Arc::new(state.run_control()),
        targets: Arc::clone(&state.targets),
    };
    let scope = plugin_host::RunScope::manual(&env);
    Ok(BuiltinToolExecutor::new(Arc::clone(&state.plugins), env, scope)
        .execute(&tool_name, arguments)
        .await)
}

async fn resolve_provider_request(
    input: &SendMessageInput,
    state: &AgentState,
    netease: &SharedNetease,
    trace: &native_trace::DurableTrace<'_>,
) -> Result<String, provider_error::ProviderError> {
    use adapter::{ModelRequest, ProviderAdapter};
    use provider_error::ProviderError;
    let local = |message: String| ProviderError::local("CONFIGURATION", message, "", "", "");
    let providers = state
        .store
        .list_providers()
        .map_err(|e| local(e.to_string()))?;
    let models = state
        .store
        .list_models()
        .map_err(|e| local(e.to_string()))?;
    let selected_model = match input.model_id.as_deref() {
        Some(id) => Some(
            state
                .store
                .get_model(id)
                .map_err(|e| local(e.to_string()))?
                .ok_or_else(|| local("所选模型不存在".into()))?,
        ),
        None => None,
    };
    let provider_id = input
        .provider_id
        .as_deref()
        .or_else(|| selected_model.as_ref().map(|m| m.provider_id.as_str()));
    let provider = providers
        .iter()
        .find(|p| p.enabled && provider_id.map_or(true, |id| p.id == id))
        .ok_or_else(|| local("请先配置可用的供应商和模型".into()))?;
    let model = selected_model
        .as_ref()
        .or_else(|| {
            models
                .iter()
                .find(|m| m.enabled && m.provider_id == provider.id)
        })
        .ok_or_else(|| local("供应商没有启用的模型".into()))?;
    if !model.enabled || model.provider_id != provider.id {
        return Err(local("所选模型未启用或不属于当前供应商".into()));
    }
    if !model.supports_streaming {
        return Err(local("当前 Agent 需要支持流式输出的模型".into()));
    }
    let protocol = ProviderProtocol::parse(&provider.protocol)
        .ok_or_else(|| local("不支持的供应商协议".into()))?;
    let adapter = ProviderAdapter::configured(protocol, &provider.base_url, &model.model_id);
    let endpoint = adapter.endpoint(&provider.base_url);
    security::validate_outbound_url(&endpoint)
        .map_err(|e| local(format!("请求地址不允许: {e:?}")))?;
    let reference = provider
        .secret_ref
        .as_deref()
        .ok_or_else(|| local("供应商尚未配置密钥".into()))?;
    let key = secrets::OsCredentialStore
        .get(reference)
        .map_err(|e| local(e.to_string()))?
        .ok_or_else(|| local("供应商尚未配置密钥".into()))?;
    let request = ModelRequest {
        model: model.model_id.clone(),
        system: model::SYSTEM_PROMPT.into(),
        messages: state
            .store
            .list_context(&input.session_id)
            .map_err(|e| local(e.to_string()))?,
        tools: if model.supports_tools {
            // `@` 只决定本轮工具范围；插件本身始终处于已挂载状态。
            state.plugins.tools_for(&input.plugin_ids).await
        } else {
            vec![]
        },
        max_tokens: 8192,
        stream: true,
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|e| local(e.without_url().to_string()))?;
    native_trace::run(
        trace,
        state,
        netease,
        &client,
        &adapter,
        &endpoint,
        key.as_str(),
        request,
    )
    .await
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn new_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}-{nanos}-{}", std::process::id())
}
