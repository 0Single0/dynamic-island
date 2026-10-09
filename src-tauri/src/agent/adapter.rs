use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

use super::{
    model::{ModelBlock, ModelMessage, ModelStreamEvent},
    provider_error::{redact, ProviderError},
    providers::{parse_sse, pop_sse_frame, ProviderProtocol},
    stream::StreamDecoder,
    tools::ToolDefinition,
};

#[derive(Debug, Clone)]
pub struct ChatCompatibility {
    pub token_field: &'static str,
    pub include_usage: bool,
}

#[derive(Debug, Clone)]
pub enum ProviderAdapter {
    ChatCompletions(ChatCompatibility),
    Responses,
    AnthropicMessages,
}

impl ProviderAdapter {
    pub fn configured(protocol: ProviderProtocol, base: &str, model: &str) -> Self {
        match protocol {
            ProviderProtocol::ChatCompletions => {
                let is_openai = reqwest::Url::parse(base)
                    .ok()
                    .is_some_and(|url| url.host_str() == Some("api.openai.com"));
                let reasoning = model.starts_with("o1")
                    || model.starts_with("o3")
                    || model.starts_with("o4")
                    || model.starts_with("gpt-5");
                let is_ark = reqwest::Url::parse(base)
                    .ok()
                    .is_some_and(|url| url.host_str() == Some("ark.cn-beijing.volces.com"));
                Self::ChatCompletions(ChatCompatibility {
                    token_field: if is_openai && reasoning {
                        "max_completion_tokens"
                    } else {
                        "max_tokens"
                    },
                    include_usage: is_openai || is_ark,
                })
            }
            ProviderProtocol::Responses => Self::Responses,
            ProviderProtocol::AnthropicMessages => Self::AnthropicMessages,
        }
    }
    pub fn protocol(&self) -> &'static str {
        match self {
            Self::ChatCompletions(_) => "chat-completions",
            Self::Responses => "responses",
            Self::AnthropicMessages => "anthropic-messages",
        }
    }
    pub fn endpoint(&self, base: &str) -> String {
        let suffix = match self {
            Self::ChatCompletions(_) => "/chat/completions",
            Self::Responses => "/responses",
            Self::AnthropicMessages => "/messages",
        };
        let base = base.trim().trim_end_matches('/');
        if base.ends_with(suffix) {
            base.into()
        } else {
            format!("{base}{suffix}")
        }
    }
    pub fn build_body(&self, request: &ModelRequest) -> Result<Value, String> {
        let names = ToolNames::new(&request.tools)?;
        let wire = |name: &str| names.wire(name);
        let mut body = match self {
            Self::ChatCompletions(compat) => {
                let mut messages = vec![json!({"role":"system","content":request.system})];
                for message in &request.messages {
                    let mut calls = Vec::new();
                    let mut reasoning = String::new();
                    for block in &message.blocks {
                        match block {
                            ModelBlock::ToolCall{id,name,arguments} => calls.push(json!({"id":id,"type":"function","function":{"name":wire(name)?,"arguments":arguments}})),
                            ModelBlock::ToolResult{call_id,output,..} => messages.push(json!({"role":"tool","tool_call_id":call_id,"content":output.to_string()})),
                            ModelBlock::Reasoning{text,..} if message.source_protocol.as_deref()==Some(self.protocol()) => reasoning.push_str(text),
                            _ => {}
                        }
                    }
                    let text = message.plain_text();
                    if !text.is_empty() || !calls.is_empty() || message.role == "user" {
                        let mut item = json!({"role":message.role,"content":text});
                        let has_calls = !calls.is_empty();
                        if !calls.is_empty() {
                            item["tool_calls"] = json!(calls);
                        }
                        if has_calls && !reasoning.is_empty() {
                            item["reasoning_content"] = json!(reasoning);
                        }
                        messages.push(item);
                    }
                }
                let mut body =
                    json!({"model":request.model,"messages":messages,"stream":request.stream});
                body[compat.token_field] = json!(request.max_tokens);
                if compat.include_usage && request.stream {
                    body["stream_options"] = json!({"include_usage":true});
                }
                if !request.tools.is_empty() {
                    body["tools"]=json!(request.tools.iter().map(|t| Ok(json!({"type":"function","function":{"name":wire(&t.name)?,"description":t.description,"parameters":t.input_schema}}))).collect::<Result<Vec<_>,String>>()?);
                }
                body
            }
            Self::Responses => {
                let mut input = Vec::new();
                for message in &request.messages {
                    // Preserve item order, including provider-native signed reasoning.
                    for block in &message.blocks {
                        match block {
                            ModelBlock::Text{text} => input.push(json!({"role":message.role,"content":[{"type":if message.role=="assistant" {"output_text"} else {"input_text"},"text":text}]})),
                            ModelBlock::ToolCall{id,name,arguments} => input.push(json!({"type":"function_call","call_id":id,"name":wire(name)?,"arguments":arguments})),
                            ModelBlock::ToolResult{call_id,output,..} => input.push(json!({"type":"function_call_output","call_id":call_id,"output":output.to_string()})),
                            ModelBlock::Opaque{value} if message.source_protocol.as_deref()==Some(self.protocol()) => input.push(value.clone()),
                            _ => {}
                        }
                    }
                }
                let mut body = json!({"model":request.model,"instructions":request.system,"input":input,"max_output_tokens":request.max_tokens,"stream":request.stream,"store":false,"include":["reasoning.encrypted_content"]});
                if !request.tools.is_empty() {
                    body["tools"]=json!(request.tools.iter().map(|t| Ok(json!({"type":"function","name":wire(&t.name)?,"description":t.description,"parameters":t.input_schema,"strict":false}))).collect::<Result<Vec<_>,String>>()?);
                }
                body
            }
            Self::AnthropicMessages => {
                let mut messages: Vec<Value> = Vec::new();
                for message in &request.messages {
                    let mut content = Vec::new();
                    for block in &message.blocks {
                        match block {
                            ModelBlock::Text{text} if !text.is_empty() => content.push(json!({"type":"text","text":text})),
                            ModelBlock::Reasoning{text,signature} if !signature.is_empty() && message.source_protocol.as_deref()==Some(self.protocol()) => content.push(json!({"type":"thinking","thinking":text,"signature":signature})),
                            ModelBlock::ToolCall{id,name,arguments} => content.push(json!({"type":"tool_use","id":id,"name":wire(name)?,"input":serde_json::from_str::<Value>(arguments).map_err(|_| "工具参数不是合法 JSON")?})),
                            ModelBlock::ToolResult{call_id,output,..} => content.push(json!({"type":"tool_result","tool_use_id":call_id,"content":output.to_string(),"is_error":output["ok"]==false})),
                            ModelBlock::Opaque{value} if message.source_protocol.as_deref()==Some(self.protocol()) => content.push(value.clone()),
                            _=>{}
                        }
                    }
                    if content.is_empty() {
                        continue;
                    }
                    let role = if message.role == "assistant" {
                        "assistant"
                    } else {
                        "user"
                    };
                    if let Some(last) = messages.last_mut().filter(|v| v["role"] == role) {
                        last["content"]
                            .as_array_mut()
                            .expect("content array")
                            .extend(content);
                    } else {
                        messages.push(json!({"role":role,"content":content}));
                    }
                }
                let mut body = json!({"model":request.model,"system":request.system,"messages":messages,"max_tokens":request.max_tokens,"stream":request.stream});
                if !request.tools.is_empty() {
                    body["tools"]=json!(request.tools.iter().map(|t| Ok(json!({"name":wire(&t.name)?,"description":t.description,"input_schema":t.input_schema}))).collect::<Result<Vec<_>,String>>()?);
                }
                body
            }
        };
        if !request.tools.is_empty() && !matches!(self, Self::AnthropicMessages) {
            body["tool_choice"] = json!("auto");
        }
        Ok(body)
    }

    pub async fn stream_step(
        &self,
        client: &reqwest::Client,
        endpoint: &str,
        key: &str,
        request: &ModelRequest,
        cancelled: impl Fn() -> bool,
        mut publish: impl FnMut(ModelStreamEvent) -> Result<(), String>,
    ) -> Result<ModelMessage, ProviderError> {
        let local = |category: &str, message: String| {
            ProviderError::local(
                category,
                redact(&message, key),
                self.protocol(),
                &request.model,
                endpoint,
            )
        };
        if cancelled() {
            return Err(local("CANCELLED", "运行已取消".into()));
        }
        let mut request = request.clone();
        let safe_endpoint = super::provider_error::safe_endpoint(endpoint);
        for message in &mut request.messages {
            if message.source_model.as_deref() != Some(request.model.as_str())
                || message.source_endpoint.as_deref() != Some(safe_endpoint.as_str())
                || message.source_protocol.as_deref() != Some(self.protocol())
            {
                message.blocks.retain(|block| {
                    !matches!(
                        block,
                        ModelBlock::Reasoning { .. } | ModelBlock::Opaque { .. }
                    )
                });
            }
        }
        let body = self
            .build_body(&request)
            .map_err(|e| local("INVALID_REQUEST", e))?;
        let mut builder = client.post(endpoint).json(&body);
        builder = match self {
            Self::AnthropicMessages => builder
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01"),
            _ => builder.bearer_auth(key),
        };
        let cancellation = async {
            loop {
                if cancelled() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            }
        };
        tokio::pin!(cancellation);
        let response = tokio::select! {
            _=&mut cancellation => return Err(local("CANCELLED","运行已取消".into())),
            result=builder.send() => result.map_err(|e|local(if e.is_timeout(){"TIMEOUT"}else{"TRANSPORT"},e.without_url().to_string()))?,
        };
        if !response.status().is_success() {
            return tokio::select! {
                _=&mut cancellation => Err(local("CANCELLED","运行已取消".into())),
                error=ProviderError::from_response(response,self.protocol(),&request.model,endpoint,key) => Err(error),
            };
        }
        let names = ToolNames::new(&request.tools).map_err(|e| local("INVALID_REQUEST", e))?;
        let mut decoder = StreamDecoder::new(self.protocol(), names);
        let mut network = response.bytes_stream();
        let mut buffer = Vec::new();
        let decode_error = |data: &str| {
            let value: Value = serde_json::from_str(data).unwrap_or(Value::Null);
            let error = value
                .get("error")
                .filter(|v| !v.is_null())
                .or_else(|| value["response"].get("error").filter(|v| !v.is_null()))
                .or_else(|| (value["type"] == "error").then_some(&value));
            error.map(|value| {
                let mut error = ProviderError::from_body(
                    400,
                    &json!({"error":value}).to_string(),
                    None,
                    self.protocol(),
                    &request.model,
                    endpoint,
                    key,
                );
                error.status = None;
                error.category = "PROVIDER_STREAM".into();
                error
            })
        };
        loop {
            let chunk = tokio::select! {
                _=&mut cancellation => return Err(local("CANCELLED","运行已取消".into())),
                next=network.next()=>next,
            };
            let Some(chunk) = chunk else {
                break;
            };
            buffer.extend_from_slice(&chunk.map_err(|e| local("TRANSPORT", e.to_string()))?);
            if buffer.len() > 8 * 1024 * 1024 {
                return Err(local("INVALID_STREAM", "模型流事件过大".into()));
            }
            while let Some(frame) =
                pop_sse_frame(&mut buffer).map_err(|e| local("INVALID_STREAM", e))?
            {
                for event in parse_sse(&frame) {
                    if let Some(error) = decode_error(&event.data) {
                        return Err(error);
                    }
                    for item in decoder
                        .accept(&event.data)
                        .map_err(|e| local("INVALID_STREAM", e))?
                    {
                        publish(item).map_err(|e| local("STORAGE", e))?;
                    }
                }
            }
        }
        if !buffer.is_empty() {
            let tail = String::from_utf8(buffer)
                .map_err(|_| local("INVALID_STREAM", "模型流包含无效 UTF-8".into()))?;
            for event in parse_sse(&tail) {
                if let Some(error) = decode_error(&event.data) {
                    return Err(error);
                }
                for item in decoder
                    .accept(&event.data)
                    .map_err(|e| local("INVALID_STREAM", e))?
                {
                    publish(item).map_err(|e| local("STORAGE", e))?;
                }
            }
        }
        let mut message = decoder
            .finish()
            .map_err(|e| local("INCOMPLETE_RESPONSE", e))?;
        message.source_model = Some(request.model);
        message.source_endpoint = Some(safe_endpoint);
        Ok(message)
    }
}

#[derive(Clone)]
pub struct ModelRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<ModelMessage>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub stream: bool,
}

#[derive(Clone, Default)]
pub struct ToolNames {
    internal_to_wire: HashMap<String, String>,
    wire_to_internal: HashMap<String, String>,
}
impl ToolNames {
    pub fn new(tools: &[ToolDefinition]) -> Result<Self, String> {
        let mut names = Self::default();
        let mut seen = HashSet::new();
        for tool in tools {
            let wire = tool.name.replace('.', "_");
            if wire.is_empty()
                || wire.len() > 64
                || !wire
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                || !seen.insert(wire.clone())
            {
                return Err(format!("工具名称无效或冲突: {}", tool.name));
            }
            names
                .internal_to_wire
                .insert(tool.name.clone(), wire.clone());
            names.wire_to_internal.insert(wire, tool.name.clone());
        }
        // 历史工具名（含旧的点号/单下划线写法）解析到当前工具；
        // 当前名称优先，别名冲突时保留先注册的映射。
        for tool in tools {
            for legacy in &tool.legacy_names {
                let alias = legacy.replace('.', "_");
                if alias.is_empty()
                    || alias.len() > 64
                    || !alias
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    continue;
                }
                names
                    .wire_to_internal
                    .entry(alias)
                    .or_insert_with(|| tool.name.clone());
            }
        }
        Ok(names)
    }
    pub fn wire(&self, internal: &str) -> Result<String, String> {
        // Historical calls remain representable even if a tool is not offered now.
        let name = self
            .internal_to_wire
            .get(internal)
            .cloned()
            .unwrap_or_else(|| internal.replace('.', "_"));
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err("历史工具名无效".into());
        }
        Ok(name)
    }
    pub fn internal(&self, wire: &str) -> Result<String, String> {
        self.wire_to_internal
            .get(wire)
            .cloned()
            .ok_or_else(|| format!("模型请求了未提供的工具: {wire}"))
    }
}
