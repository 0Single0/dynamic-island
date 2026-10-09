use super::{
    adapter::{ChatCompatibility, ModelRequest, ProviderAdapter, ToolNames},
    builtin_executor::BuiltinToolExecutor,
    events::EventDraft,
    model::{ModelBlock, ModelMessage, ModelStreamEvent},
    native_trace::{run_loop, DurableTrace},
    plugins::host::{PluginEnv, RunScope},
    plugins::PluginRuntime,
    providers::ProviderProtocol,
    storage::AgentStore,
    stream::StreamDecoder,
    tools::{ModelToolExecutor, ToolExecutionResult},
};
/// 在同步测试与 `#[tokio::test]` 内部都能安全等待异步操作：
/// 运行时内部不允许再调用 `block_on`，所以统一把 future 送到独立线程执行。
fn block_on_runtime<F: std::future::Future + Send + 'static>(future: F) -> F::Output
where
    F::Output: Send + 'static,
{
    std::thread::spawn(move || tauri::async_runtime::block_on(future))
        .join()
        .expect("async test bridge thread panicked")
}

/// 全部内置插件已挂载的运行时：Agent 工具目录的唯一来源。
async fn mounted_runtime() -> Arc<PluginRuntime> {
    let runtime = Arc::new(PluginRuntime::new());
    super::plugins::mount_builtin_plugins(&runtime, None)
        .await
        .expect("builtin plugins mount during tests");
    runtime
}

/// 指定 @ 范围内的模型工具目录（默认范围 + 选中插件）。
fn model_tools_for(plugin_ids: &[String]) -> Vec<super::tools::ToolDefinition> {
    let ids = plugin_ids.to_vec();
    block_on_runtime(async move { mounted_runtime().await.tools_for(&ids).await })
}

fn model_tools() -> Vec<super::tools::ToolDefinition> {
    block_on_runtime(async { mounted_runtime().await.all_tools().await })
}
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

struct Server {
    endpoint: String,
    requests: Arc<Mutex<Vec<Value>>>,
    done: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new(responses: Vec<(u16, String)>, pause: bool) -> Self {
        Self::at("/chat/completions", responses, pause)
    }
    fn at(route: &str, responses: Vec<(u16, String)>, pause: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}{route}", listener.local_addr().unwrap());
        let route = route.to_owned();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let thread = std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0u8; 4096];
                let boundary = loop {
                    let n = socket.read(&mut chunk).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..boundary]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                assert!(headers.starts_with(&format!("POST {route} ")));
                let lower = headers.to_ascii_lowercase();
                if route == "/messages" {
                    assert!(lower.contains("x-api-key: key"));
                    assert!(lower.contains("anthropic-version: 2023-06-01"));
                } else {
                    assert!(lower.contains("authorization: bearer "));
                }
                while bytes.len() < boundary + length {
                    let n = socket.read(&mut chunk).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                }
                captured
                    .lock()
                    .unwrap()
                    .push(serde_json::from_slice(&bytes[boundary..boundary + length]).unwrap());
                let header=format!("HTTP/1.1 {status} Test\r\nContent-Type: {}\r\nContent-Length: {}\r\nX-Request-Id: local-test\r\nConnection: close\r\n\r\n",if status==200{"text/event-stream"}else{"application/json"},body.len());
                if socket.write_all(header.as_bytes()).is_err() {
                    continue;
                }
                let split = body
                    .find("\r\n\r\n")
                    .map(|i| i + 4)
                    .or_else(|| body.find("\n\n").map(|i| i + 2))
                    .unwrap_or(body.len());
                if socket.write_all(&body.as_bytes()[..split]).is_err() {
                    continue;
                }
                if pause {
                    std::thread::sleep(Duration::from_millis(250));
                }
                let _ = socket.write_all(&body.as_bytes()[split..]);
            }
            finished.store(true, Ordering::SeqCst);
        });
        Self {
            endpoint,
            requests,
            done,
            thread: Some(thread),
        }
    }
    fn join(&mut self) {
        self.thread.take().unwrap().join().unwrap();
    }
}

fn frame(value: Value) -> String {
    format!("data: {value}\r\n\r\n")
}
fn chat_answer(text: &str) -> String {
    frame(json!({"choices":[{"delta":{"content":text}}]}))
        + &frame(json!({"choices":[{"delta":{},"finish_reason":"stop"}]}))
        + "data: [DONE]\r\n\r\n"
}
fn request() -> ModelRequest {
    ModelRequest {
        model: "test".into(),
        system: "System".into(),
        messages: vec![ModelMessage::text("user", "hi")],
        tools: model_tools_for(&["desktop".into(), "netease".into()]),
        max_tokens: 128,
        stream: true,
    }
}
fn chat() -> ProviderAdapter {
    ProviderAdapter::configured(
        ProviderProtocol::ChatCompletions,
        "https://compatible.example/v1",
        "test",
    )
}

#[test]
fn model_tool_catalog_has_unique_names_and_current_pdf_schema() {
    // 全量目录（含非默认范围的 markdown）用于校验工具名唯一与 PDF schema。
    let tools = model_tools();
    let mut names = std::collections::HashSet::new();
    for definition in &tools {
        assert!(
            names.insert(definition.name.clone()),
            "duplicate tool: {}",
            definition.name
        );
    }
    let pdf = tools
        .iter()
        .find(|definition| definition.name == "markdown__convert_to_pdf")
        .expect("markdown PDF tool is registered");
    assert_eq!(pdf.input_schema["required"], json!(["html"]));
    assert!(pdf.input_schema["properties"].get("outputPath").is_none());
    assert!(pdf.input_schema["properties"].get("html").is_some());
}

#[tokio::test]
async fn production_prompt_and_real_tool_catalog_reach_the_http_request() {
    let mut server = Server::new(vec![(200, chat_answer("你好，有什么需要帮忙的？"))], false);
    let mut input = request();
    input.system = super::model::SYSTEM_PROMPT.into();
    input.messages = vec![ModelMessage::text("user", "你好")];
    chat()
        .stream_step(
            &reqwest::Client::new(),
            &server.endpoint,
            "key",
            &input,
            || false,
            |_| Ok(()),
        )
        .await
        .unwrap();
    server.join();
    let captured = server.requests.lock().unwrap();
    let body = &captured[0];
    let system = body["messages"][0]["content"].as_str().unwrap();
    assert_eq!(system, super::model::SYSTEM_PROMPT);
    assert!(!system.contains("网易云"));
    assert!(!system.to_lowercase().contains("netease"));
    assert!(system.contains("普通问候"));
    let names = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.len() > 3);
    assert!(names.contains(&"system__get_time"));
    assert!(!names.contains(&"system__launch_app"));
    assert!(names.contains(&"netease__start_or_connect"));
    assert!(names.contains(&"files__create_text"));
    assert!(names.contains(&"computer__list_windows"));
    assert!(names.contains(&"netease__get_current_state"));
    let launch = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["function"]["name"] == "netease__start_or_connect")
        .unwrap();
    assert_eq!(launch["function"]["parameters"]["properties"], json!({}));
}

#[tokio::test]
async fn actual_requests_and_text_are_streamed_before_completion() {
    let mut server = Server::new(vec![(200, chat_answer("你好"))], true);
    let mut deltas = String::new();
    let mut seen = false;
    let result = chat()
        .stream_step(
            &reqwest::Client::new(),
            &server.endpoint,
            "test-secret",
            &request(),
            || false,
            |e| {
                if let ModelStreamEvent::TextDelta { text, .. } = e {
                    assert!(!server.done.load(Ordering::SeqCst));
                    seen = true;
                    deltas.push_str(&text);
                }
                Ok(())
            },
        )
        .await
        .unwrap();
    server.join();
    assert!(seen);
    assert_eq!(deltas, "你好");
    assert_eq!(result.plain_text(), "你好");
    let captured = server.requests.lock().unwrap();
    let body = &captured[0];
    let tool_names = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(tool_names.contains(&"netease__get_current_state"));
    assert_eq!(body["max_tokens"], 128);
    assert!(body.get("max_completion_tokens").is_none());
    assert!(body.get("stream_options").is_none());
}

#[tokio::test]
async fn http_errors_are_real_bounded_and_redacted() {
    for status in [400, 401, 429, 500] {
        let body=json!({"error":{"code":"InvalidParameter","message":"bad test-secret","param":"tools.0.function.name"}}).to_string();
        let mut server = Server::new(vec![(status, body)], false);
        let error = chat()
            .stream_step(
                &reqwest::Client::new(),
                &server.endpoint,
                "test-secret",
                &request(),
                || false,
                |_| Ok(()),
            )
            .await
            .unwrap_err();
        server.join();
        assert_eq!(error.status, Some(status));
        assert_eq!(error.request_id.as_deref(), Some("local-test"));
        assert!(!serde_json::to_string(&error)
            .unwrap()
            .contains("test-secret"));
    }
    let mut server = Server::new(vec![(400, "x".repeat(32000))], false);
    let error = chat()
        .stream_step(
            &reqwest::Client::new(),
            &server.endpoint,
            "key",
            &request(),
            || false,
            |_| Ok(()),
        )
        .await
        .unwrap_err();
    server.join();
    assert_eq!(error.body.unwrap().len(), 16 * 1024);
}

struct ReadTool;
#[async_trait::async_trait]
impl ModelToolExecutor for ReadTool {
    async fn execute(&self, name: &str, args: Value) -> ToolExecutionResult {
        assert_eq!(name, "netease__get_current_state");
        assert_eq!(args, json!({}));
        ToolExecutionResult::success("test fixture only", json!({"title":"test song"}))
    }
}

#[tokio::test]
async fn tool_exchange_is_durable_and_sent_back_with_native_ids() {
    let first = frame(json!({"choices":[{"delta":{"reasoning_content":"先读取"}}]}))
        + &frame(json!({"choices":[{"delta":{"content":"我来查询。"}}]}))
        + &frame(
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1","function":{"name":"netease_get_current_state","arguments":"{"}}]}}]}),
        )
        + &frame(
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"}"}}]},"finish_reason":"tool_calls"}]}),
        )
        + "data: [DONE]\n\n";
    let mut server = Server::new(
        vec![(200, first), (200, chat_answer("歌曲是 test song。"))],
        false,
    );
    let path = std::env::temp_dir().join(format!(
        "agent-loop-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let store = AgentStore::open(&path).unwrap();
    let session = store.create_session("test").unwrap();
    store.create_run("run-1", &session.id).unwrap();
    store.seed_context(&session.id).unwrap();
    store
        .append_context(&session.id, "run-1", &request().messages)
        .unwrap();
    let trace = DurableTrace {
        store: &store,
        app: None,
        session_id: &session.id,
        run_id: "run-1",
    };
    let answer = run_loop(
        &trace,
        &ReadTool,
        &super::native_trace::ToolProvenance::new(),
        || false,
        &reqwest::Client::new(),
        &chat(),
        &server.endpoint,
        "key",
        request(),
    )
    .await
    .unwrap();
    server.join();
    assert_eq!(answer, "我来查询。歌曲是 test song。");
    let captures = server.requests.lock().unwrap();
    let messages = captures[1]["messages"].as_array().unwrap();
    assert_eq!(
        messages[2]["tool_calls"][0]["function"]["name"],
        "netease__get_current_state"
    );
    assert_eq!(messages[2]["reasoning_content"], "先读取");
    assert_eq!(messages[3]["tool_call_id"], "call-1");
    let events = store.list_events(&session.id, 0).unwrap();
    let text = events
        .iter()
        .position(|e| e.event_type == "text_delta")
        .unwrap();
    let tool = events
        .iter()
        .position(|e| e.event_type == "tool_started")
        .unwrap();
    let later = events
        .iter()
        .rposition(|e| e.event_type == "text_delta")
        .unwrap();
    assert!(text < tool && tool < later);
    assert_eq!(events[tool].call_id.as_deref(), Some("call-1"));
    let context = store.list_context(&session.id).unwrap();
    assert_eq!(context.len(), 4);
    drop(store);
    let reopened = AgentStore::open(&path).unwrap();
    assert_eq!(reopened.list_context(&session.id).unwrap(), context);
    reopened.recover_interrupted_runs().unwrap();
    assert_eq!(
        reopened
            .list_events(&session.id, events.last().unwrap().seq)
            .unwrap()[0]
            .event_type,
        "run_interrupted"
    );
    drop(reopened);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn adapter_profiles_names_and_protocol_history_are_validated() {
    assert_eq!(
        chat().endpoint("https://example.com/v1/chat/completions/"),
        "https://example.com/v1/chat/completions"
    );
    let openai = ProviderAdapter::configured(
        ProviderProtocol::ChatCompletions,
        "https://api.openai.com/v1",
        "o3",
    );
    assert_eq!(
        openai.build_body(&request()).unwrap()["max_completion_tokens"],
        128
    );
    let usage = ProviderAdapter::ChatCompletions(ChatCompatibility {
        token_field: "max_tokens",
        include_usage: true,
    });
    assert_eq!(
        usage.build_body(&request()).unwrap()["stream_options"]["include_usage"],
        true
    );
    let mut tools = model_tools();
    let mut collision = tools[0].clone();
    collision.name = tools[1].name.clone();
    tools.push(collision);
    assert!(ToolNames::new(&tools).is_err());
    let names = ToolNames::new(&model_tools()).unwrap();
    assert!(names.internal("unknown").is_err());
    // 旧会话里的工具名仍能解析到当前工具。
    assert_eq!(
        names.internal("netease_get_current_state").unwrap(),
        "netease__get_current_state"
    );
    let mut input = request();
    input.messages.push(ModelMessage {
        role: "assistant".into(),
        source_protocol: Some("chat-completions".into()),
        source_model: Some("test".into()),
        source_endpoint: None,
        blocks: vec![
            ModelBlock::Reasoning {
                text: "private".into(),
                signature: "sig".into(),
            },
            ModelBlock::Text {
                text: "hello".into(),
            },
        ],
    });
    let anthropic = ProviderAdapter::AnthropicMessages
        .build_body(&input)
        .unwrap();
    assert!(!anthropic.to_string().contains("private"));
    let responses = ProviderAdapter::Responses.build_body(&input).unwrap();
    assert_eq!(responses["input"][1]["content"][0]["type"], "output_text");
}

#[test]
fn three_decoders_keep_reasoning_tool_ids_and_completion_boundaries() {
    let names = ToolNames::new(&model_tools()).unwrap();
    let mut response = StreamDecoder::new("responses", names.clone());
    for value in [
        json!({"type":"response.output_item.added","output_index":0,"item":{"id":"r","type":"reasoning"}}),
        json!({"type":"response.reasoning_summary_text.delta","output_index":0,"delta":"摘要"}),
        json!({"type":"response.output_item.done","output_index":0,"item":{"id":"r","type":"reasoning","summary":[{"type":"summary_text","text":"摘要"}],"encrypted_content":"opaque"}}),
        json!({"type":"response.output_item.added","output_index":2,"item":{"id":"item","type":"function_call","call_id":"call-2","name":"netease_get_current_state","arguments":""}}),
        json!({"type":"response.function_call_arguments.delta","item_id":"item","delta":"{}"}),
        json!({"type":"response.completed","response":{"usage":{"input_tokens":1}}}),
    ] {
        response.accept(&value.to_string()).unwrap();
    }
    let message = response.finish().unwrap();
    assert!(
        matches!(&message.blocks[1],ModelBlock::ToolCall{id,name,..} if id=="call-2" && name=="netease__get_current_state")
    );
    let mut input = request();
    input.messages.push(message);
    let body = ProviderAdapter::Responses.build_body(&input).unwrap();
    assert_eq!(body["input"][1]["encrypted_content"], "opaque");
    let mut anthropic = StreamDecoder::new("anthropic-messages", names.clone());
    for value in [
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"思考"}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":"答复"}}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
        json!({"type":"message_stop"}),
    ] {
        anthropic.accept(&value.to_string()).unwrap();
    }
    let message = anthropic.finish().unwrap();
    assert_eq!(message.plain_text(), "答复");
    input.messages = vec![message];
    assert_eq!(
        ProviderAdapter::AnthropicMessages
            .build_body(&input)
            .unwrap()["messages"][0]["content"][0]["signature"],
        "sig"
    );
    let mut broken = StreamDecoder::new("chat-completions", names);
    broken
        .accept(&json!({"choices":[{"delta":{"content":"partial"}}]}).to_string())
        .unwrap();
    assert!(broken.finish().is_err());
}

#[tokio::test]
async fn cancellation_interrupts_a_waiting_stream() {
    let mut server = Server::new(vec![(200, chat_answer("partial"))], true);
    let flag = Arc::new(AtomicBool::new(false));
    let cancel = flag.clone();
    let task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(60)).await;
        cancel.store(true, Ordering::SeqCst);
    });
    let error = chat()
        .stream_step(
            &reqwest::Client::new(),
            &server.endpoint,
            "key",
            &request(),
            || flag.load(Ordering::SeqCst),
            |_| Ok(()),
        )
        .await
        .unwrap_err();
    assert_eq!(error.category, "CANCELLED");
    task.await.unwrap();
    server.join();
}

fn temporary_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "agent-regression-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[tokio::test]
async fn responses_and_anthropic_use_actual_transport_and_tool_context() {
    for adapter in [
        ProviderAdapter::Responses,
        ProviderAdapter::AnthropicMessages,
    ] {
        let (route, first, last) = match adapter {
            ProviderAdapter::Responses => (
                "/responses",
                frame(
                    json!({"type":"response.output_item.added","output_index":3,"item":{"id":"item","type":"function_call","call_id":"native-call","name":"netease_get_current_state","arguments":""}}),
                ) + &frame(
                    json!({"type":"response.function_call_arguments.delta","item_id":"item","delta":"{}"}),
                ) + &frame(json!({"type":"response.completed"})),
                frame(json!({"type":"response.output_text.delta","output_index":0,"delta":"好了"}))
                    + &frame(json!({"type":"response.completed"})),
            ),
            _ => (
                "/messages",
                frame(
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"读取"}}),
                ) + &frame(
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signature"}}),
                ) + &frame(json!({"type":"content_block_stop","index":0}))
                    + &frame(
                        json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"native-call","name":"netease_get_current_state","input":{}}}),
                    )
                    + &frame(json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}))
                    + &frame(json!({"type":"message_stop"})),
                frame(
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":"好了"}}),
                ) + &frame(json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}))
                    + &frame(json!({"type":"message_stop"})),
            ),
        };
        let mut server = Server::at(route, vec![(200, first), (200, last)], false);
        assert_eq!(adapter.endpoint(&server.endpoint), server.endpoint);
        let path = temporary_path();
        let store = AgentStore::open(&path).unwrap();
        let session = store.create_session("protocol").unwrap();
        store.create_run("run", &session.id).unwrap();
        let trace = DurableTrace {
            store: &store,
            app: None,
            session_id: &session.id,
            run_id: "run",
        };
        assert_eq!(
            run_loop(
                &trace,
                &ReadTool,
                &super::native_trace::ToolProvenance::new(),
                || false,
                &reqwest::Client::new(),
                &adapter,
                &server.endpoint,
                "key",
                request()
            )
            .await
            .unwrap(),
            "好了"
        );
        server.join();
        let captured = server.requests.lock().unwrap();
        if route == "/responses" {
            assert_eq!(captured[0]["max_output_tokens"], 128);
            let tool_names = captured[0]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["name"].as_str().unwrap())
                .collect::<Vec<_>>();
            assert!(tool_names.contains(&"netease__get_current_state"));
            assert_eq!(captured[1]["input"][1]["call_id"], "native-call");
            assert_eq!(captured[1]["input"][2]["type"], "function_call_output");
        } else {
            assert_eq!(captured[0]["max_tokens"], 128);
            assert_eq!(
                captured[1]["messages"][1]["content"][0]["signature"],
                "signature"
            );
            assert_eq!(
                captured[1]["messages"][2]["content"][0]["tool_use_id"],
                "native-call"
            );
        }
        let records = store.list_events(&session.id, 0).unwrap();
        assert_eq!(
            records
                .iter()
                .find(|e| e.event_type == "tool_completed")
                .unwrap()
                .call_id
                .as_deref(),
            Some("native-call")
        );
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[tokio::test]
async fn signatures_are_not_replayed_across_models_or_endpoints() {
    let mut server = Server::new(
        vec![(200, chat_answer("ok")), (200, chat_answer("ok"))],
        false,
    );
    let mut input = request();
    input.messages.push(ModelMessage {
        role: "assistant".into(),
        source_protocol: Some("chat-completions".into()),
        source_model: Some("other-model".into()),
        source_endpoint: Some(server.endpoint.clone()),
        blocks: vec![
            ModelBlock::Reasoning {
                text: "do-not-replay".into(),
                signature: String::new(),
            },
            ModelBlock::Text {
                text: "prior answer".into(),
            },
        ],
    });
    chat()
        .stream_step(
            &reqwest::Client::new(),
            &server.endpoint,
            "key",
            &input,
            || false,
            |_| Ok(()),
        )
        .await
        .unwrap();
    input.messages[1].source_model = Some(input.model.clone());
    input.messages[1].source_endpoint = Some("https://other.example/v1/chat/completions".into());
    chat()
        .stream_step(
            &reqwest::Client::new(),
            &server.endpoint,
            "key",
            &input,
            || false,
            |_| Ok(()),
        )
        .await
        .unwrap();
    server.join();
    for body in server.requests.lock().unwrap().iter() {
        assert!(!body.to_string().contains("do-not-replay"));
    }
}

#[tokio::test]
async fn stream_errors_retain_provider_code_and_partial_text() {
    let body = frame(json!({"choices":[{"delta":{"content":"部分正文"}}]}))
        + &frame(
            json!({"error":{"code":"quota_exhausted","message":"bad secret-value","param":"budget"}}),
        );
    let mut server = Server::new(vec![(200, body)], false);
    let mut text = String::new();
    let error = chat()
        .stream_step(
            &reqwest::Client::new(),
            &server.endpoint,
            "secret-value",
            &request(),
            || false,
            |e| {
                if let ModelStreamEvent::TextDelta { text: delta, .. } = e {
                    text.push_str(&delta);
                }
                Ok(())
            },
        )
        .await
        .unwrap_err();
    server.join();
    assert_eq!(text, "部分正文");
    assert_eq!(error.category, "PROVIDER_STREAM");
    assert_eq!(error.code.as_deref(), Some("quota_exhausted"));
    assert!(!serde_json::to_string(&error)
        .unwrap()
        .contains("secret-value"));
}

#[test]
fn multiple_calls_invalid_arguments_and_unknown_names_are_checked() {
    let names = ToolNames::new(&model_tools()).unwrap();
    for (name, args, valid) in [
        ("netease_get_current_state", "{}", true),
        ("unknown", "{}", false),
        ("netease_get_current_state", "{", false),
        ("netease_get_current_state", "[]", false),
    ] {
        let mut decoder = StreamDecoder::new("chat-completions", names.clone());
        decoder.accept(&json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":name,"arguments":args}},{"index":1,"id":"b","function":{"name":name,"arguments":args}}]},"finish_reason":"tool_calls"}]}).to_string()).unwrap();
        decoder.accept("[DONE]").unwrap();
        let result = decoder.finish();
        assert_eq!(result.is_ok(), valid);
        if valid {
            assert_eq!(result.unwrap().blocks.len(), 2);
        }
    }
}

#[tokio::test]
async fn netease_and_markdown_adapters_handle_hostless_contexts() {
    let env = PluginEnv::default();
    let executor = BuiltinToolExecutor::new(
        mounted_runtime().await,
        env.clone(),
        RunScope::manual(&env),
    );
    // 网易云适配器：当前名与两个历史名都解析到同一能力，无宿主时返回结构化错误。
    for name in [
        "netease__get_current_state",
        "netease_get_current_state",
        "netease.get_current_state",
    ] {
        let result = executor.execute(name, json!({})).await;
        assert!(!result.ok, "{name}");
        assert_eq!(
            result.error_code.as_deref(),
            Some("HOST_UNAVAILABLE"),
            "{name}"
        );
    }
    // Markdown 适配器：参数校验先于宿主需求，超限参数直接拒绝。
    let oversized = executor
        .execute(
            "markdown_convert_to_pdf",
            json!({"html": "x".repeat(512 * 1024 + 1)}),
        )
        .await;
    assert_eq!(oversized.error_code.as_deref(), Some("INVALID_TOOL_ARGS"));
    let no_host = executor
        .execute("markdown.convert_to_pdf", json!({"html": "<h1>hi</h1>"}))
        .await;
    assert_eq!(no_host.error_code.as_deref(), Some("HOST_UNAVAILABLE"));
}

#[test]
fn concurrent_sequences_legacy_seeding_and_terminal_recovery_are_durable() {
    let path = temporary_path();
    let store = Arc::new(AgentStore::open(&path).unwrap());
    let session = store.create_session("legacy").unwrap();
    store.add_message(&session.id, "user", "old user").unwrap();
    store
        .add_message(&session.id, "assistant", "old answer")
        .unwrap();
    store.seed_context(&session.id).unwrap();
    store.seed_context(&session.id).unwrap();
    assert_eq!(store.list_context(&session.id).unwrap().len(), 2);
    store.create_run("run", &session.id).unwrap();
    assert!(store.create_run("conflict", &session.id).is_err());
    let workers = (0..4)
        .map(|_| {
            let store = store.clone();
            let session_id = session.id.clone();
            std::thread::spawn(move || {
                for _ in 0..20 {
                    store
                        .append_event(&session_id, "run", EventDraft::new("usage", json!({})))
                        .unwrap();
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }
    let events = store.list_events(&session.id, 0).unwrap();
    assert_eq!(
        events.iter().map(|e| e.seq).collect::<Vec<_>>(),
        (1..=80).collect::<Vec<_>>()
    );
    store
        .append_event(
            &session.id,
            "run",
            EventDraft::new("run_completed", json!({})),
        )
        .unwrap();
    drop(store);
    let store = AgentStore::open(&path).unwrap();
    store.recover_interrupted_runs().unwrap();
    assert_eq!(store.list_events(&session.id, 80).unwrap().len(), 1);
    store.create_run("next-run", &session.id).unwrap();
    store.recover_interrupted_runs().unwrap();
    store.recover_interrupted_runs().unwrap();
    assert_eq!(store.list_events(&session.id, 81).unwrap().len(), 1);
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

fn tool_reply(id: usize) -> String {
    frame(
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":format!("call-{id}"),"function":{"name":"netease_get_current_state","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}),
    ) + "data: [DONE]\r\n\r\n"
}

struct FailedRead;
#[async_trait::async_trait]
impl ModelToolExecutor for FailedRead {
    async fn execute(&self, _name: &str, _args: Value) -> ToolExecutionResult {
        ToolExecutionResult::failure("NETEASE_NOT_CONNECTED", "未连接", true)
    }
}

#[tokio::test]
async fn failed_tools_are_sent_back_and_step_limit_stops_the_loop() {
    for limit in [false, true] {
        let responses = if limit {
            (0..8).map(|i| (200, tool_reply(i))).collect()
        } else {
            vec![(200, tool_reply(0)), (200, chat_answer("播放器未连接。"))]
        };
        let mut server = Server::new(responses, false);
        let path = temporary_path();
        let store = AgentStore::open(&path).unwrap();
        let session = store.create_session("failure").unwrap();
        store.create_run("run", &session.id).unwrap();
        let trace = DurableTrace {
            store: &store,
            app: None,
            session_id: &session.id,
            run_id: "run",
        };
        let result = run_loop(
            &trace,
            &FailedRead,
            &super::native_trace::ToolProvenance::new(),
            || false,
            &reqwest::Client::new(),
            &chat(),
            &server.endpoint,
            "key",
            request(),
        )
        .await;
        server.join();
        if limit {
            assert_eq!(result.unwrap_err().category, "STEP_LIMIT");
        } else {
            assert_eq!(result.unwrap(), "播放器未连接。");
        }
        let requests = server.requests.lock().unwrap();
        let returned: Value =
            serde_json::from_str(requests[1]["messages"][3]["content"].as_str().unwrap()).unwrap();
        assert_eq!(returned["ok"], false);
        assert_eq!(returned["errorCode"], "NETEASE_NOT_CONNECTED");
        let events = store.list_events(&session.id, 0).unwrap();
        let completed = events
            .iter()
            .filter(|e| e.event_type == "tool_completed")
            .collect::<Vec<_>>();
        assert_eq!(completed.len(), if limit { 8 } else { 1 });
        assert!(completed.iter().all(|e| e.payload["status"] == "failed"));
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }
}
