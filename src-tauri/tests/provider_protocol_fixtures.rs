use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq)]
struct SseFrame {
    event: Option<String>,
    data: String,
}

fn parse_sse(input: &str) -> Vec<SseFrame> {
    let mut frames = Vec::new();
    let mut event = None;
    let mut data = Vec::new();

    let flush = |frames: &mut Vec<SseFrame>, event: &mut Option<String>, data: &mut Vec<String>| {
        if !data.is_empty() || event.is_some() {
            frames.push(SseFrame {
                event: event.take(),
                data: std::mem::take(data).join("\n"),
            });
        }
    };

    for line in input.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            flush(&mut frames, &mut event, &mut data);
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => event = Some(value.to_owned()),
            "data" => data.push(value.to_owned()),
            _ => {}
        }
    }
    flush(&mut frames, &mut event, &mut data);
    frames
}

fn json_frames(frames: &[SseFrame]) -> Vec<(&SseFrame, Value)> {
    frames
        .iter()
        .filter(|frame| frame.data != "[DONE]")
        .map(|frame| {
            (
                frame,
                serde_json::from_str(&frame.data).expect("fixture data must be JSON"),
            )
        })
        .collect()
}

fn read_json(input: &str) -> Value {
    serde_json::from_str(input).expect("request fixture must be valid JSON")
}

#[test]
fn request_fixtures_capture_protocol_differences_without_credentials() {
    let chat = read_json(include_str!(
        "fixtures/providers/chat-completions-request.json"
    ));
    let responses = read_json(include_str!("fixtures/providers/responses-request.json"));
    let anthropic = read_json(include_str!(
        "fixtures/providers/anthropic-messages-request.json"
    ));

    assert_eq!(chat["messages"][0]["role"], "system");
    assert_eq!(chat["tools"][0]["type"], "function");
    assert_eq!(
        chat["tools"][0]["function"]["parameters"]["properties"]["id"]["type"],
        "integer"
    );
    assert_eq!(chat["max_completion_tokens"], 128);
    assert_eq!(chat["stream_options"]["include_usage"], true);

    assert_eq!(responses["instructions"], "You are a helpful assistant.");
    assert_eq!(responses["input"][0]["content"][0]["type"], "input_text");
    assert_eq!(responses["tools"][0]["type"], "function");
    assert_eq!(responses["max_output_tokens"], 128);

    assert_eq!(anthropic["system"], "You are a helpful assistant.");
    assert!(anthropic["messages"][0].get("system").is_none());
    assert_eq!(anthropic["tools"][0]["input_schema"]["required"][0], "id");
    assert_eq!(anthropic["max_tokens"], 128);

    for fixture in [
        include_str!("fixtures/providers/chat-completions-request.json"),
        include_str!("fixtures/providers/responses-request.json"),
        include_str!("fixtures/providers/anthropic-messages-request.json"),
    ] {
        let value = read_json(fixture);
        assert!(value.get("api_key").is_none());
        assert!(value.get("Authorization").is_none());
        assert!(value.get("x-api-key").is_none());
    }
}

#[test]
fn chat_completions_stream_yields_text_tool_call_usage_and_done() {
    let frames = parse_sse(include_str!(
        "fixtures/providers/chat-completions-stream.sse"
    ));
    let parsed = json_frames(&frames);
    let mut text = String::new();
    let mut calls: BTreeMap<u64, (String, String, String)> = BTreeMap::new();
    let mut finish_reason = None;
    let mut usage = None;

    for (_, frame) in parsed {
        if let Some(frame_usage) = frame.get("usage") {
            usage = Some(frame_usage.clone());
        }
        for choice in frame["choices"].as_array().into_iter().flatten() {
            if let Some(delta_text) = choice["delta"]["content"].as_str() {
                text.push_str(delta_text);
            }
            for call in choice["delta"]["tool_calls"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let index = call["index"].as_u64().expect("tool call index");
                let entry = calls.entry(index).or_default();
                if let Some(id) = call["id"].as_str() {
                    entry.0.push_str(id);
                }
                if let Some(name) = call["function"]["name"].as_str() {
                    entry.1.push_str(name);
                }
                if let Some(args) = call["function"]["arguments"].as_str() {
                    entry.2.push_str(args);
                }
            }
            if let Some(reason) = choice["finish_reason"].as_str() {
                finish_reason = Some(reason.to_owned());
            }
        }
    }

    assert!(frames.iter().any(|frame| frame.data == "[DONE]"));
    assert_eq!(text, "Looking up ");
    assert_eq!(finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(calls.len(), 1);
    let (id, name, arguments) = calls.get(&0).expect("index 0 call");
    assert_eq!(id, "call_mock_1");
    assert_eq!(name, "lookup");
    assert_eq!(read_json(arguments)["id"], 42);
    assert_eq!(usage.as_ref().unwrap()["total_tokens"], 24);
}

#[test]
fn responses_stream_correlates_function_item_and_call_id() {
    let frames = parse_sse(include_str!("fixtures/providers/responses-stream.sse"));
    let parsed = json_frames(&frames);
    let mut text = String::new();
    let mut item_id = String::new();
    let mut call_id = String::new();
    let mut name = String::new();
    let mut arguments = String::new();
    let mut event_types = Vec::new();
    let mut usage = None;

    for (sse, frame) in parsed {
        let kind = frame["type"]
            .as_str()
            .or(sse.event.as_deref())
            .unwrap_or_default();
        event_types.push(kind.to_owned());
        match kind {
            "response.output_text.delta" => {
                text.push_str(frame["delta"].as_str().unwrap_or_default())
            }
            "response.output_item.added" if frame["item"]["type"] == "function_call" => {
                item_id = frame["item"]["id"].as_str().unwrap_or_default().to_owned();
                call_id = frame["item"]["call_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                name = frame["item"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
            }
            "response.function_call_arguments.delta" => {
                assert_eq!(frame["item_id"], item_id);
                arguments.push_str(frame["delta"].as_str().unwrap_or_default());
            }
            "response.completed" => usage = Some(frame["response"]["usage"].clone()),
            _ => {}
        }
    }

    assert_eq!(text, "Looking up ");
    assert_eq!(item_id, "fc_item_1");
    assert_eq!(call_id, "call_mock_2");
    assert_eq!(name, "lookup");
    assert_eq!(read_json(&arguments)["id"], 42);
    assert!(event_types
        .iter()
        .any(|kind| kind == "response.function_call_arguments.done"));
    assert_eq!(usage.unwrap()["total_tokens"], 24);
}

#[test]
fn anthropic_stream_uses_block_indexes_and_tool_use_ids() {
    let frames = parse_sse(include_str!(
        "fixtures/providers/anthropic-messages-stream.sse"
    ));
    let parsed = json_frames(&frames);
    let mut text = String::new();
    let mut tool_index = None;
    let mut tool_id = String::new();
    let mut name = String::new();
    let mut arguments = String::new();
    let mut stop_reason = None;
    let mut input_tokens = None;
    let mut output_tokens = None;

    for (sse, frame) in parsed {
        let kind = frame["type"]
            .as_str()
            .or(sse.event.as_deref())
            .unwrap_or_default();
        match kind {
            "message_start" => input_tokens = frame["message"]["usage"]["input_tokens"].as_u64(),
            "content_block_start" if frame["content_block"]["type"] == "text" => {}
            "content_block_start" if frame["content_block"]["type"] == "tool_use" => {
                tool_index = frame["index"].as_u64();
                tool_id = frame["content_block"]["id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                name = frame["content_block"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
            }
            "content_block_delta" if frame["delta"]["type"] == "text_delta" => {
                text.push_str(frame["delta"]["text"].as_str().unwrap_or_default());
            }
            "content_block_delta" if frame["delta"]["type"] == "input_json_delta" => {
                assert_eq!(frame["index"].as_u64(), tool_index);
                arguments.push_str(frame["delta"]["partial_json"].as_str().unwrap_or_default());
            }
            "message_delta" => {
                stop_reason = frame["delta"]["stop_reason"].as_str().map(str::to_owned);
                output_tokens = frame["usage"]["output_tokens"].as_u64();
            }
            _ => {}
        }
    }

    assert_eq!(text, "Looking up ");
    assert_eq!(tool_index, Some(1));
    assert_eq!(tool_id, "toolu_mock_1");
    assert_eq!(name, "lookup");
    assert_eq!(read_json(&arguments)["id"], 42);
    assert_eq!(stop_reason.as_deref(), Some("tool_use"));
    assert_eq!(input_tokens, Some(18));
    assert_eq!(output_tokens, Some(6));
}
