use super::{
    adapter::ToolNames,
    model::{ModelBlock, ModelMessage, ModelStreamEvent},
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

pub struct StreamDecoder {
    protocol: String,
    names: ToolNames,
    blocks: BTreeMap<usize, ModelBlock>,
    closed: HashSet<usize>,
    active: Option<(String, usize)>,
    chat_tools: HashMap<usize, usize>,
    response_items: HashMap<String, usize>,
    next_index: usize,
    terminal: bool,
    reason: Option<String>,
}

impl StreamDecoder {
    pub fn new(protocol: &str, names: ToolNames) -> Self {
        Self {
            protocol: protocol.into(),
            names,
            blocks: BTreeMap::new(),
            closed: HashSet::new(),
            active: None,
            chat_tools: HashMap::new(),
            response_items: HashMap::new(),
            next_index: 0,
            terminal: false,
            reason: None,
        }
    }
    fn open(&mut self, index: usize, kind: &str, events: &mut Vec<ModelStreamEvent>) {
        if self.blocks.contains_key(&index) {
            return;
        }
        self.next_index = self.next_index.max(index + 1);
        self.blocks.insert(
            index,
            match kind {
                "reasoning" => ModelBlock::Reasoning {
                    text: String::new(),
                    signature: String::new(),
                },
                "tool" => ModelBlock::ToolCall {
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                },
                _ => ModelBlock::Text {
                    text: String::new(),
                },
            },
        );
        events.push(ModelStreamEvent::BlockStarted {
            index,
            kind: kind.into(),
        });
    }
    fn end(&mut self, index: usize, events: &mut Vec<ModelStreamEvent>) {
        if self.closed.insert(index) {
            events.push(ModelStreamEvent::BlockEnded { index });
        }
    }
    fn chat_index(&mut self, kind: &str, events: &mut Vec<ModelStreamEvent>) -> usize {
        if let Some((current, index)) = &self.active {
            if current == kind {
                return *index;
            }
        }
        if let Some((_, index)) = self.active.take() {
            self.end(index, events);
        }
        let index = self.next_index;
        self.open(index, kind, events);
        self.active = Some((kind.into(), index));
        index
    }
    fn text(&mut self, index: usize, kind: &str, text: &str, events: &mut Vec<ModelStreamEvent>) {
        if text.is_empty() {
            return;
        }
        self.open(index, kind, events);
        match self.blocks.get_mut(&index) {
            Some(ModelBlock::Text { text: current }) => {
                current.push_str(text);
                events.push(ModelStreamEvent::TextDelta {
                    index,
                    text: text.into(),
                });
            }
            Some(ModelBlock::Reasoning { text: current, .. }) => {
                current.push_str(text);
                events.push(ModelStreamEvent::ReasoningDelta {
                    index,
                    text: text.into(),
                });
            }
            _ => {}
        }
    }
    fn tool(
        &mut self,
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        args: &str,
        events: &mut Vec<ModelStreamEvent>,
    ) {
        self.open(index, "tool", events);
        if let Some(ModelBlock::ToolCall {
            id: current_id,
            name: current_name,
            arguments,
        }) = self.blocks.get_mut(&index)
        {
            if let Some(id) = id {
                *current_id = id.into();
            }
            if let Some(name) = name {
                current_name.push_str(name);
            }
            arguments.push_str(args);
            events.push(ModelStreamEvent::ToolDelta {
                index,
                id: id.map(str::to_owned),
                name: if current_name.is_empty() {
                    None
                } else {
                    Some(current_name.clone())
                },
                arguments: args.into(),
            });
        }
    }
    fn complete(&mut self, reason: &str, events: &mut Vec<ModelStreamEvent>) {
        self.terminal = true;
        self.reason = Some(reason.into());
        let indexes = self.blocks.keys().copied().collect::<Vec<_>>();
        for index in indexes {
            self.end(index, events);
        }
        events.push(ModelStreamEvent::Finished {
            reason: reason.into(),
        });
    }
    pub fn accept(&mut self, data: &str) -> Result<Vec<ModelStreamEvent>, String> {
        let mut events = Vec::new();
        if data == "[DONE]" {
            if self.protocol == "chat-completions" {
                let reason = self
                    .reason
                    .clone()
                    .ok_or("模型流结束但缺少 finish_reason")?;
                self.complete(&reason, &mut events);
            }
            return Ok(events);
        }
        let value: Value = serde_json::from_str(data).map_err(|_| "模型流返回无效 JSON")?;
        if let Some(error) = value.get("error").filter(|v| !v.is_null()) {
            return Err(error["message"].as_str().unwrap_or("模型流返回错误").into());
        }
        if self.terminal {
            return Err("完成标记之后收到额外模型内容".into());
        }
        match self.protocol.as_str() {
            "chat-completions" => self.chat(&value, &mut events)?,
            "responses" => self.responses(&value, &mut events)?,
            "anthropic-messages" => self.anthropic(&value, &mut events)?,
            _ => return Err("不支持的流式协议".into()),
        }
        Ok(events)
    }
    fn chat(&mut self, v: &Value, events: &mut Vec<ModelStreamEvent>) -> Result<(), String> {
        if let Some(usage) = v.get("usage").filter(|u| !u.is_null()) {
            events.push(ModelStreamEvent::Usage {
                usage: usage.clone(),
            });
        }
        let delta = &v["choices"][0]["delta"];
        for (field, kind) in [("reasoning_content", "reasoning"), ("content", "assistant")] {
            if let Some(text) = delta[field].as_str().filter(|t| !t.is_empty()) {
                let index = self.chat_index(kind, events);
                self.text(index, kind, text, events);
            }
        }
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            let provider_index = call["index"].as_u64().ok_or("工具调用缺少 index")? as usize;
            let index = match self.chat_tools.get(&provider_index).copied() {
                Some(index) => index,
                None => {
                    if let Some((_, index)) = self.active.take() {
                        self.end(index, events);
                    }
                    let index = self.next_index;
                    self.chat_tools.insert(provider_index, index);
                    self.open(index, "tool", events);
                    index
                }
            };
            self.tool(
                index,
                call["id"].as_str(),
                call["function"]["name"].as_str(),
                call["function"]["arguments"].as_str().unwrap_or(""),
                events,
            );
        }
        if let Some(reason) = v["choices"][0]["finish_reason"].as_str() {
            self.reason = Some(reason.into());
        }
        Ok(())
    }
    fn responses(&mut self, v: &Value, events: &mut Vec<ModelStreamEvent>) -> Result<(), String> {
        let index = v["output_index"].as_u64().unwrap_or(0) as usize;
        match v["type"].as_str().unwrap_or("") {
            "response.output_item.added" => {
                let item = &v["item"];
                if let Some(id) = item["id"].as_str() {
                    self.response_items.insert(id.into(), index);
                }
                match item["type"].as_str().unwrap_or("") {
                    "function_call" => self.tool(
                        index,
                        item["call_id"].as_str(),
                        item["name"].as_str(),
                        item["arguments"].as_str().unwrap_or(""),
                        events,
                    ),
                    "reasoning" => self.open(index, "reasoning", events),
                    "message" => self.open(index, "assistant", events),
                    _ => {}
                }
            }
            "response.output_text.delta" => self.text(
                index,
                "assistant",
                v["delta"].as_str().unwrap_or(""),
                events,
            ),
            "response.reasoning_summary_text.delta" => self.text(
                index,
                "reasoning",
                v["delta"].as_str().unwrap_or(""),
                events,
            ),
            "response.function_call_arguments.delta" => {
                let index = v["item_id"]
                    .as_str()
                    .and_then(|id| self.response_items.get(id))
                    .copied()
                    .ok_or("工具参数缺少关联的 item_id")?;
                self.tool(index, None, None, v["delta"].as_str().unwrap_or(""), events);
            }
            "response.output_item.done" => {
                let item = &v["item"];
                if item["type"] == "reasoning" {
                    self.blocks.insert(
                        index,
                        ModelBlock::Opaque {
                            value: item.clone(),
                        },
                    );
                }
                if item["type"] == "function_call" {
                    if !self.blocks.contains_key(&index) {
                        self.tool(
                            index,
                            item["call_id"].as_str(),
                            item["name"].as_str(),
                            "",
                            events,
                        );
                    }
                    if let Some(ModelBlock::ToolCall {
                        id,
                        name,
                        arguments,
                    }) = self.blocks.get_mut(&index)
                    {
                        if let Some(value) = item["call_id"].as_str() {
                            // 上游可能在收尾时才给/换 call_id（added 里是占位）。
                            // 换 id 时补一个 ToolDelta 把新 id 广播出去，
                            // 否则执行循环按旧 id 查不到块 → "工具块缺少关联事件"。
                            if *id != value {
                                *id = value.into();
                                events.push(ModelStreamEvent::ToolDelta {
                                    index,
                                    id: Some(value.into()),
                                    name: if name.is_empty() {
                                        None
                                    } else {
                                        Some(name.clone())
                                    },
                                    arguments: String::new(),
                                });
                            }
                        }
                        if let Some(value) = item["name"].as_str() {
                            *name = value.into();
                        }
                        if let Some(value) = item["arguments"].as_str() {
                            *arguments = value.into();
                        }
                    }
                }
                self.end(index, events);
            }
            "response.completed" => {
                if let Some(usage) = v["response"].get("usage") {
                    events.push(ModelStreamEvent::Usage {
                        usage: usage.clone(),
                    });
                }
                let reason = if self
                    .blocks
                    .values()
                    .any(|b| matches!(b, ModelBlock::ToolCall { .. }))
                {
                    "tool_calls"
                } else {
                    "stop"
                };
                self.complete(reason, events);
            }
            "response.incomplete" | "response.failed" => {
                return Err(v["response"]["error"]["message"]
                    .as_str()
                    .unwrap_or("模型响应失败或输出不完整")
                    .into())
            }
            "error" => return Err(v["message"].as_str().unwrap_or("模型流返回错误").into()),
            _ => {}
        }
        Ok(())
    }
    fn anthropic(&mut self, v: &Value, events: &mut Vec<ModelStreamEvent>) -> Result<(), String> {
        let index = v["index"].as_u64().unwrap_or(0) as usize;
        match v["type"].as_str().unwrap_or("") {
            "message_start" => {
                if let Some(usage) = v["message"].get("usage") {
                    events.push(ModelStreamEvent::Usage {
                        usage: usage.clone(),
                    });
                }
            }
            "content_block_start" => {
                let block = &v["content_block"];
                match block["type"].as_str().unwrap_or("") {
                    "text" => {
                        self.open(index, "assistant", events);
                        self.text(
                            index,
                            "assistant",
                            block["text"].as_str().unwrap_or(""),
                            events,
                        );
                    }
                    "thinking" => {
                        self.open(index, "reasoning", events);
                        self.text(
                            index,
                            "reasoning",
                            block["thinking"].as_str().unwrap_or(""),
                            events,
                        );
                    }
                    "tool_use" => {
                        let initial = block["input"]
                            .as_object()
                            .filter(|o| !o.is_empty())
                            .map(|_| block["input"].to_string())
                            .unwrap_or_default();
                        self.tool(
                            index,
                            block["id"].as_str(),
                            block["name"].as_str(),
                            &initial,
                            events,
                        );
                    }
                    "redacted_thinking" => {
                        self.blocks.insert(
                            index,
                            ModelBlock::Opaque {
                                value: block.clone(),
                            },
                        );
                    }
                    _ => {}
                }
            }
            "content_block_delta" => {
                let delta = &v["delta"];
                match delta["type"].as_str().unwrap_or("") {
                    "text_delta" => self.text(
                        index,
                        "assistant",
                        delta["text"].as_str().unwrap_or(""),
                        events,
                    ),
                    "thinking_delta" => self.text(
                        index,
                        "reasoning",
                        delta["thinking"].as_str().unwrap_or(""),
                        events,
                    ),
                    "input_json_delta" => self.tool(
                        index,
                        None,
                        None,
                        delta["partial_json"].as_str().unwrap_or(""),
                        events,
                    ),
                    "signature_delta" => {
                        if let Some(ModelBlock::Reasoning { signature, .. }) =
                            self.blocks.get_mut(&index)
                        {
                            signature.push_str(delta["signature"].as_str().unwrap_or(""));
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => self.end(index, events),
            "message_delta" => {
                if let Some(reason) = v["delta"]["stop_reason"].as_str() {
                    self.reason = Some(
                        match reason {
                            "end_turn" | "stop_sequence" => "stop",
                            "tool_use" => "tool_calls",
                            "max_tokens" => "length",
                            other => other,
                        }
                        .into(),
                    );
                }
                if let Some(usage) = v.get("usage") {
                    events.push(ModelStreamEvent::Usage {
                        usage: usage.clone(),
                    });
                }
            }
            "message_stop" => {
                let reason = self.reason.clone().ok_or("模型缺少 stop_reason")?;
                self.complete(&reason, events);
            }
            _ => {}
        }
        Ok(())
    }
    pub fn finish(mut self) -> Result<ModelMessage, String> {
        if !self.terminal {
            return Err("模型流意外中断，未收到完成标记".into());
        }
        match self.reason.as_deref() {
            Some("stop" | "tool_calls") => {}
            Some("length") => return Err("模型输出达到长度上限，本轮结果不完整".into()),
            other => return Err(format!("模型提前停止: {other:?}")),
        }
        let mut ids = HashSet::new();
        for block in self.blocks.values_mut() {
            if let ModelBlock::ToolCall {
                id,
                name,
                arguments,
            } = block
            {
                if id.is_empty() || !ids.insert(id.clone()) {
                    return Err("工具调用 ID 缺失或重复".into());
                }
                *name = self.names.internal(name)?;
                if arguments.is_empty() {
                    *arguments = json!({}).to_string();
                }
                let args: Value =
                    serde_json::from_str(arguments).map_err(|_| "工具参数不是合法 JSON")?;
                if !args.is_object() {
                    return Err("工具参数必须是对象".into());
                }
            }
        }
        if self.reason.as_deref() == Some("tool_calls")
            && !self
                .blocks
                .values()
                .any(|b| matches!(b, ModelBlock::ToolCall { .. }))
        {
            return Err("模型声明调用工具但没有有效工具调用".into());
        }
        if self.blocks.is_empty() {
            return Err("模型没有返回内容".into());
        }
        Ok(ModelMessage {
            role: "assistant".into(),
            blocks: self.blocks.into_values().collect(),
            source_protocol: Some(self.protocol),
            source_model: None,
            source_endpoint: None,
        })
    }
}
