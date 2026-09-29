use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderProtocol {
    ChatCompletions,
    Responses,
    AnthropicMessages,
}

impl ProviderProtocol {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "chat-completions" | "openai-chat" => Some(Self::ChatCompletions),
            "responses" | "openai-responses" => Some(Self::Responses),
            "anthropic-messages" | "anthropic" => Some(Self::AnthropicMessages),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<CanonicalMessage>,
    pub max_tokens: u32,
    pub stream: bool,
}

pub fn build_request(protocol: ProviderProtocol, request: &CanonicalRequest) -> Value {
    match protocol {
        ProviderProtocol::ChatCompletions => {
            let mut messages = Vec::new();
            if let Some(system) = request.system.as_ref() {
                messages.push(json!({"role": "system", "content": system}));
            }
            messages.extend(
                request
                    .messages
                    .iter()
                    .map(|message| json!({"role": message.role, "content": message.content})),
            );
            json!({
                "model": request.model,
                "messages": messages,
                "max_completion_tokens": request.max_tokens,
                "stream": request.stream,
                "stream_options": {"include_usage": true}
            })
        }
        ProviderProtocol::Responses => json!({
            "model": request.model,
            "instructions": request.system,
            "input": request.messages.iter().map(|message| json!({"role": message.role, "content": [{"type": "input_text", "text": message.content}]})).collect::<Vec<_>>(),
            "max_output_tokens": request.max_tokens,
            "stream": request.stream
        }),
        ProviderProtocol::AnthropicMessages => json!({
            "model": request.model,
            "system": request.system,
            "messages": request.messages.iter().filter(|message| message.role != "system").map(|message| json!({"role": message.role, "content": message.content})).collect::<Vec<_>>(),
            "max_tokens": request.max_tokens,
            "stream": request.stream
        }),
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

pub fn parse_sse(input: &str) -> Vec<SseEvent> {
    let mut events = Vec::new();
    let mut event = None;
    let mut data = Vec::new();
    let flush = |events: &mut Vec<SseEvent>, event: &mut Option<String>, data: &mut Vec<String>| {
        if !data.is_empty() || event.is_some() {
            events.push(SseEvent {
                event: event.take(),
                data: std::mem::take(data).join("\n"),
            });
        }
    };
    for line in input.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            flush(&mut events, &mut event, &mut data);
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => event = Some(value.into()),
            "data" => data.push(value.into()),
            _ => {}
        }
    }
    flush(&mut events, &mut event, &mut data);
    events
}

pub fn extract_text(protocol: ProviderProtocol, data: &Value) -> Option<String> {
    match protocol {
        ProviderProtocol::ChatCompletions => data["choices"][0]["delta"]["content"]
            .as_str()
            .map(str::to_owned),
        ProviderProtocol::Responses => data["delta"].as_str().map(str::to_owned),
        ProviderProtocol::AnthropicMessages => data["delta"]["text"].as_str().map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_three_protocol_shapes() {
        let request = CanonicalRequest {
            model: "demo".into(),
            system: Some("You are helpful".into()),
            messages: vec![CanonicalMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            max_tokens: 128,
            stream: true,
        };
        assert_eq!(
            build_request(ProviderProtocol::ChatCompletions, &request)["max_completion_tokens"],
            128
        );
        assert_eq!(
            build_request(ProviderProtocol::Responses, &request)["input"][0]["content"][0]["type"],
            "input_text"
        );
        assert_eq!(
            build_request(ProviderProtocol::AnthropicMessages, &request)["max_tokens"],
            128
        );
    }

    #[test]
    fn parses_multiline_sse_and_protocol_text() {
        let events =
            parse_sse("event: message\ndata: {\"delta\":{\"text\":\"a\"}}\n\ndata: [DONE]\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(
            extract_text(
                ProviderProtocol::AnthropicMessages,
                &serde_json::from_str(&events[0].data).unwrap()
            ),
            Some("a".into())
        );
    }
}
