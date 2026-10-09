//! Protocol-independent agent loop. Adapters own all provider wire formats.
use std::collections::HashMap;

use serde_json::{json, Value};

use super::{
    adapter::{ModelRequest, ProviderAdapter, ToolNames},
    builtin_executor::BuiltinToolExecutor,
    events::{self, EventDraft},
    model::{ModelBlock, ModelMessage, ModelStreamEvent},
    plugins::host::{PluginEnv, RunScope},
    plugins::PluginRuntime,
    provider_error::ProviderError,
    storage::AgentStore,
    tools::ModelToolExecutor,
    AgentState,
};
use crate::netease::SharedNetease;

pub struct DurableTrace<'a> {
    pub store: &'a AgentStore,
    pub app: Option<&'a tauri::AppHandle>,
    pub session_id: &'a str,
    pub run_id: &'a str,
}
impl DurableTrace<'_> {
    pub fn emit(&self, draft: EventDraft) -> Result<(), String> {
        if let Some(app) = self.app {
            events::publish(app, self.store, self.session_id, self.run_id, draft)?;
        } else {
            self.store
                .append_event(self.session_id, self.run_id, draft)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

/// 本轮模型工具名 → 插件/能力来源；写入轨迹便于核对“模型看到的名字”。
pub type ToolProvenance = HashMap<String, Value>;

/// 按本轮工具清单解析插件来源；未注册名称不出现在轨迹里。
pub async fn provenance_for(runtime: &PluginRuntime, names: &[String]) -> ToolProvenance {
    let mut provenance = ToolProvenance::new();
    for name in names {
        if let Ok(reference) = runtime.resolve(name).await {
            provenance.insert(
                name.clone(),
                json!({
                    "pluginId": reference.plugin_id,
                    "namespace": reference.namespace,
                    "toolName": reference.tool_name,
                    "capabilityId": reference.capability_id,
                    "pluginGeneration": reference.generation,
                }),
            );
        }
    }
    provenance
}

fn tool_payload(
    provenance: &ToolProvenance,
    name: &str,
    title: &str,
    arguments: &str,
) -> Value {
    let mut payload = json!({"name":name,"title":title,"arguments":arguments,"status":"running"});
    if let Some(reference) = provenance.get(name) {
        payload["pluginId"] = reference["pluginId"].clone();
        payload["namespace"] = reference["namespace"].clone();
        payload["toolName"] = reference["toolName"].clone();
        payload["capabilityId"] = reference["capabilityId"].clone();
        payload["pluginGeneration"] = reference["pluginGeneration"].clone();
        let normalized = serde_json::from_str::<Value>(arguments)
            .map(|value| value.to_string())
            .unwrap_or_else(|_| arguments.to_owned());
        payload["argsHash"] = json!(super::permissions::args_hash(&normalized));
    }
    payload
}

pub async fn run(
    trace: &DurableTrace<'_>,
    state: &AgentState,
    netease: &SharedNetease,
    client: &reqwest::Client,
    adapter: &ProviderAdapter,
    endpoint: &str,
    key: &str,
    request: ModelRequest,
) -> Result<String, ProviderError> {
    let env = PluginEnv {
        app: trace.app.cloned(),
        control: std::sync::Arc::new(state.run_control()),
        targets: std::sync::Arc::clone(&state.targets),
    };
    let scope = RunScope::new(
        trace.session_id,
        trace.run_id,
        env.cancellation(trace.run_id),
    );
    let _ = netease;
    let provenance = provenance_for(
        &state.plugins,
        &request.tools.iter().map(|tool| tool.name.clone()).collect::<Vec<_>>(),
    )
    .await;
    run_loop(
        trace,
        &BuiltinToolExecutor::new(std::sync::Arc::clone(&state.plugins), env, scope),
        &provenance,
        || state.is_cancelled(trace.run_id),
        client,
        adapter,
        endpoint,
        key,
        request,
    )
    .await
}

pub async fn run_loop(
    trace: &DurableTrace<'_>,
    executor: &impl ModelToolExecutor,
    provenance: &ToolProvenance,
    cancelled: impl Fn() -> bool,
    client: &reqwest::Client,
    adapter: &ProviderAdapter,
    endpoint: &str,
    key: &str,
    mut request: ModelRequest,
) -> Result<String, ProviderError> {
    let model = request.model.clone();
    let error = |category: &str, message: String| {
        ProviderError::local(category, message, adapter.protocol(), &model, endpoint)
    };
    let names = ToolNames::new(&request.tools).map_err(|e| error("INVALID_REQUEST", e))?;
    let mut combined = String::new();
    for step in 0..8u32 {
        if cancelled() {
            return Err(error("CANCELLED", "运行已取消".into()));
        }
        let mut started = EventDraft::new(
            "step_started",
            json!({"protocol":adapter.protocol(),"model":request.model}),
        );
        started.step = Some(step);
        trace.emit(started).map_err(|e| error("STORAGE", e))?;
        let mut call_indexes = std::collections::HashMap::new();
        let message=adapter.stream_step(client,endpoint,key,&request,&cancelled,|event| {
            let (event_type,index,payload,call_id)=match event {
                ModelStreamEvent::BlockStarted{index,kind}=>("block_started",Some(index),json!({"kind":kind}),None),
                ModelStreamEvent::TextDelta{index,text}=>("text_delta",Some(index),json!({"text":text}),None),
                ModelStreamEvent::ReasoningDelta{index,text}=>("reasoning_delta",Some(index),json!({"text":text}),None),
                ModelStreamEvent::ToolDelta{index,id,name,arguments}=>{
                    if let Some(id)=&id {call_indexes.insert(id.clone(),index);}
                    let internal=name.as_deref().and_then(|n|names.internal(n).ok());
                    let title=internal.as_deref().and_then(|n|request.tools.iter().find(|t|t.name==n)).map(|t|t.description.clone());
                    let mut payload=json!({"name":internal.or(name),"title":title,"argumentsDelta":arguments});
                    if let Some(name)=payload["name"].as_str().and_then(|n|provenance.get(n)) {
                        payload["pluginId"]=name["pluginId"].clone();
                        payload["capabilityId"]=name["capabilityId"].clone();
                    }
                    ("tool_delta",Some(index),payload,id)
                }
                ModelStreamEvent::BlockEnded{index}=>("block_ended",Some(index),json!({}),None),
                ModelStreamEvent::Usage{usage}=>("usage",None,json!({"usage":usage}),None),
                ModelStreamEvent::Finished{reason}=>("model_finished",None,json!({"reason":reason}),None),
            };
            trace.emit(EventDraft{step:Some(step),block_id:index.map(|i|format!("{}:{step}:block:{i}",trace.run_id)),call_id,event_type:event_type.into(),payload,..EventDraft::default()})
        }).await?;
        let calls = message
            .blocks
            .iter()
            .filter_map(|block| match block {
                ModelBlock::ToolCall {
                    id,
                    name,
                    arguments,
                } => Some((
                    call_indexes.get(id).copied(),
                    id.clone(),
                    name.clone(),
                    arguments.clone(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        let text = message.plain_text();
        if calls.is_empty() && text.trim().is_empty() {
            return Err(error("EMPTY_RESPONSE", "模型没有返回正文".into()));
        }
        combined.push_str(&text);
        let mut exchange = vec![message];
        for (index, id, name, arguments) in &calls {
            let index =
                index.ok_or_else(|| error("INVALID_STREAM", "工具块缺少关联事件".into()))?;
            if cancelled() {
                return Err(error("CANCELLED", "运行已取消".into()));
            }
            let tool = request
                .tools
                .iter()
                .find(|t| t.name == *name)
                .ok_or_else(|| error("INVALID_TOOL", "模型请求了未提供的工具".into()))?;
            let mut draft = EventDraft::new(
                "tool_started",
                tool_payload(provenance, name, &tool.description, arguments),
            );
            draft.step = Some(step);
            draft.block_id = Some(format!("{}:{step}:block:{index}", trace.run_id));
            draft.call_id = Some(id.clone());
            trace.emit(draft).map_err(|e| error("STORAGE", e))?;
            let args = serde_json::from_str(arguments)
                .map_err(|_| error("INVALID_TOOL", "工具参数不是合法 JSON".into()))?;
            let result = executor.execute(name, args).await;
            let mut completed = tool_payload(provenance, name, &tool.description, arguments);
            completed["output"] = result.data.clone();
            completed["summary"] = json!(result.summary);
            completed["status"] = json!(if result.ok { "completed" } else { "failed" });
            completed["errorCode"] = json!(result.error_code);
            let mut draft = EventDraft::new("tool_completed", completed);
            draft.step = Some(step);
            draft.block_id = Some(format!("{}:{step}:block:{index}", trace.run_id));
            draft.call_id = Some(id.clone());
            trace.emit(draft).map_err(|e| error("STORAGE", e))?;
            exchange.push(ModelMessage {
                role: "tool".into(),
                source_protocol: Some(adapter.protocol().into()),
                source_model: Some(model.clone()),
                source_endpoint: Some(super::provider_error::safe_endpoint(endpoint)),
                blocks: vec![ModelBlock::ToolResult {
                    call_id: id.clone(),
                    name: name.clone(),
                    output: serde_json::to_value(result)
                        .map_err(|e| error("INTERNAL", e.to_string()))?,
                }],
            });
        }
        // Commit whole exchanges so restart history never contains orphan calls.
        trace
            .store
            .append_context(trace.session_id, trace.run_id, &exchange)
            .map_err(|e| error("STORAGE", e.to_string()))?;
        request.messages.extend(exchange);
        let mut ended = EventDraft::new("step_completed", json!({}));
        ended.step = Some(step);
        trace.emit(ended).map_err(|e| error("STORAGE", e))?;
        if calls.is_empty() {
            return Ok(combined);
        }
    }
    Err(error(
        "STEP_LIMIT",
        "工具调用达到 8 步上限，已停止执行".into(),
    ))
}
