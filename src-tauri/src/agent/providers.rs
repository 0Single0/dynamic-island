use serde::{Deserialize, Serialize};
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

/// Extract one complete SSE frame from arbitrary network byte chunks. Waiting
/// for the delimiter before UTF-8 decoding preserves split Chinese characters.
pub fn pop_sse_frame(buffer: &mut Vec<u8>) -> Result<Option<String>, String> {
    let boundary = (0..buffer.len()).find_map(|index| {
        if buffer[index..].starts_with(b"\r\n\r\n") {
            Some((index, 4))
        } else if buffer[index..].starts_with(b"\n\n") {
            Some((index, 2))
        } else {
            None
        }
    });
    let Some((index, width)) = boundary else {
        return Ok(None);
    };
    let bytes = buffer.drain(..index + width).collect();
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "模型流包含无效 UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sse_preserves_multiline_data_and_comments() {
        let events = parse_sse(": heartbeat\r\nevent: chunk\r\ndata: one\r\ndata: two\r\n\r\n");
        assert_eq!(events[0].data, "one\ntwo");
        assert_eq!(events[0].event.as_deref(), Some("chunk"));
    }
    #[test]
    fn sse_frame_reassembles_split_utf8_and_crlf_delimiter() {
        let frame = "data: {\"text\":\"你好\"}\r\n\r\n".as_bytes();
        let split = frame.iter().position(|b| *b >= 0x80).unwrap() + 1;
        let mut buffer = frame[..split].to_vec();
        assert!(pop_sse_frame(&mut buffer).unwrap().is_none());
        buffer.extend_from_slice(&frame[split..]);
        let parsed = pop_sse_frame(&mut buffer).unwrap().unwrap();
        assert_eq!(parse_sse(&parsed)[0].data, "{\"text\":\"你好\"}");
        assert!(buffer.is_empty());
    }
}
