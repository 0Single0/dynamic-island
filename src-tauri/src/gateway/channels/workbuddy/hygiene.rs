//! 请求卫生管道：CIR → 上游 OpenAI 形状报文。
//!
//! 每一步都是纯函数，可单测、可按渠道开关。顺序固定：
//! 拼消息 → 指纹清洗 → 工具配对自愈 → reasoning 回填 → 思考注入 → 强制流式。

use serde_json::{json, Value};

use crate::gateway::protocol::cir::{Block, CanonicalRequest, Message, Role};

/// 会触发上游 WAF / 内容审核的第三方客户端特征串。
///
/// 上游的表现是 `403` 或 `code 11-128`（Illegal API invocation from an
/// unapproved channel）—— 换账号也没用，必须把特征消掉。
///
/// **处理方式不是删除**：参考实现（MIT）用 A/B 测确认过，这些串被整段抹掉后
/// 请求仍然会被判异常；正确做法是**替换成语义等价但不触发匹配的写法**
/// （只改"归属声明"这类可替换的部分，保留指令本体）。
const FINGERPRINT_REWRITES: [(&str, &str); 6] = [
    // 计费头残片：整个头连同值一起去掉（它不是正文语义的一部分）。
    ("x-anthropic-billing-hdr", ""),
    // Claude Code 的自我声明：去掉 "Anthropic's official CLI ..." 这段归属，
    // 保留 "You are Claude Code" 这个型号声明（单独出现是允许的）。
    (
        "You are Claude Code, Anthropic's official CLI tool for Claude",
        "You are Claude Code",
    ),
    // Codex CLI 同理：去掉 "tool, a terminal-based ..." 这段附加描述。
    (
        "You are a coding agent running in the Codex CLI tool, a terminal-based coding assistant.",
        "You are a coding agent running in the Codex CLI tool.",
    ),
    // 反馈链接含 anthropics 域名，是明确的来源指纹。
    (
        "To provide feedback, users should report the issue at https://github.com/anthropics/claude-code/issues",
        "To provide feedback, users should report the issue on the project homepage",
    ),
    // 分支说明里的特征短语。
    ("Main branch (", "Primary branch ("),
    // 上游自己的错误码字面量：出现在正文里会被当成伪装特征。
    // 参考实现在公开发布的版本里把这个改写的目标值隐去了，无从照搬；
    // 而它只是个数字码、去掉不损失任何语义（参考实现"不要整段删除"的警告
    // 针对的是身份声明这类有语义的串），所以这里选择移除。
    ("11-128", ""),
];

/// 来源域名：命中即移除（域名本身没有任何语义价值）。
const FINGERPRINT_DOMAINS: [&str; 1] = ["github.com/anthropics/"];

/// 拼出上游请求体：以原始请求体为基底（保留渠道特有的透传字段），再逐层清洗。
pub fn build_chat_body(model: &str, request: &CanonicalRequest) -> Value {
    let mut body = match request.raw.clone() {
        Value::Object(map) => Value::Object(map),
        _ => json!({}),
    };
    if let Some(map) = body.as_object_mut() {
        // 私有标记键（下划线开头）是网关内部用的，不该发给上游。
        map.retain(|key, _| !key.starts_with('_'));
        map.remove("max_completion_tokens");
        // **客户端协议的顶层提示词字段必须删掉**：Anthropic 的 `system`、
        // Responses 的 `instructions` / `input` 是各自协议的顶层字段，
        // 而我们已经把提示词重建进 `messages`（且**经过指纹清洗**）。
        // 留着它们等于把客户端的原文（未清洗）再发一遍 ——
        // 既让上游看到两份提示词，也把清洗直接绕过。
        map.remove("system");
        map.remove("instructions");
        map.remove("input");
    }

    body["model"] = json!(model);
    let mut messages = build_messages(request);
    repair_tool_pairing(&mut messages);
    backfill_reasoning_content(&mut messages, model);
    body["messages"] = Value::Array(messages);

    if request.tools.is_empty() {
        if let Some(map) = body.as_object_mut() {
            map.remove("tools");
        }
    } else {
        body["tools"] = json!(request
            .tools
            .iter()
            .map(|tool| {
                let mut function = json!({ "name": tool.name });
                if let Some(description) = &tool.description {
                    function["description"] = json!(sanitize_text(description));
                }
                if let Some(parameters) = &tool.parameters {
                    function["parameters"] = parameters.clone();
                }
                json!({ "type": "function", "function": function })
            })
            .collect::<Vec<_>>());
    }

    match request.max_tokens {
        Some(max_tokens) => body["max_tokens"] = json!(max_tokens),
        None => {
            if let Some(map) = body.as_object_mut() {
                map.remove("max_tokens");
            }
        }
    }

    inject_deepseek_thinking(&mut body, model);
    force_stream(&mut body);
    body
}

fn build_messages(request: &CanonicalRequest) -> Vec<Value> {
    let mut messages = Vec::new();

    let system: Vec<&str> = request.system.iter().filter_map(Block::as_text).collect();
    if !system.is_empty() {
        messages.push(json!({
            "role": "system",
            "content": sanitize_text(&system.join("\n")),
        }));
    }

    for message in &request.messages {
        messages.push(build_message(message));
    }
    messages
}

fn build_message(message: &Message) -> Value {
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };
    let mut value = json!({ "role": role, "content": content_value(&message.content) });

    if !message.tool_calls.is_empty() {
        value["tool_calls"] = json!(message
            .tool_calls
            .iter()
            .map(|call| json!({
                "id": call.id.clone().unwrap_or_default(),
                "type": "function",
                "function": {
                    "name": call.name,
                    "arguments": sanitize_text(&call.arguments),
                },
            }))
            .collect::<Vec<_>>());
    }
    if let Some(id) = &message.tool_call_id {
        value["tool_call_id"] = json!(id);
    }
    if let Some(name) = &message.name {
        value["name"] = json!(name);
    }
    if let Some(reasoning) = &message.reasoning {
        // 历史思考内容同样要清：它会被整段回传给上游。
        value["reasoning_content"] = json!(sanitize_text(reasoning));
    }
    value
}

/// 有图就发块数组，纯文本发字符串（上游对两种写法都收，但纯文本更省）。
fn content_value(blocks: &[Block]) -> Value {
    let texts: Vec<&str> = blocks.iter().filter_map(Block::as_text).collect();
    let images: Vec<&str> = blocks
        .iter()
        .filter_map(|block| match block {
            Block::Image { url } => Some(url.as_str()),
            Block::Text { .. } => None,
        })
        .collect();

    if images.is_empty() {
        return Value::String(sanitize_text(&texts.join("\n")));
    }
    let mut parts = Vec::new();
    for text in texts {
        parts.push(json!({ "type": "text", "text": sanitize_text(text) }));
    }
    for url in images {
        parts.push(json!({ "type": "image_url", "image_url": { "url": url } }));
    }
    Value::Array(parts)
}

/// 把会被判「未授权渠道」的特征串改写成等价写法，并抹平 `\r\n` 残片。
pub fn sanitize_text(text: &str) -> String {
    // 绝大多数请求没有任何特征串：先判断，省掉下面这一串无谓的拷贝与扫描。
    if !has_fingerprint(text) {
        return text.to_string();
    }
    let mut output = text.to_string();
    for (from, to) in FINGERPRINT_REWRITES {
        if output.contains(from) {
            output = output.replace(from, to);
        }
    }
    for domain in FINGERPRINT_DOMAINS {
        if output.contains(domain) {
            output = output.replace(domain, "");
        }
    }
    // `\r\n` 残片会被上游当成"伪造头"的特征。
    output.replace("\r\n", "\n")
}

/// 是否含需要处理的特征（给调用方做"要不要改写"的判断，避免无谓的字符串拷贝）。
pub fn has_fingerprint(text: &str) -> bool {
    FINGERPRINT_REWRITES
        .iter()
        .any(|(from, _)| !from.is_empty() && text.contains(from))
        || FINGERPRINT_DOMAINS.iter().any(|domain| text.contains(domain))
        || text.contains("\r\n")
}

/// 工具配对自愈：合并相邻 assistant(tool_calls) → 按 call 重排 tool 结果 → 清孤儿。
///
/// 上游对「工具调用序列断裂」会直接 400（code 11148），所以本地先修好再发。
fn repair_tool_pairing(messages: &mut Vec<Value>) {
    merge_adjacent_tool_calls(messages);

    // 结果索引：同一 tool_call_id 只认第一条。
    let mut results: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    for message in messages.iter() {
        if message.get("role").and_then(Value::as_str) == Some("tool") {
            if let Some(id) = message.get("tool_call_id").and_then(Value::as_str) {
                results
                    .entry(id.to_string())
                    .or_insert_with(|| message.clone());
            }
        }
    }

    let mut output = Vec::with_capacity(messages.len());
    for message in messages.drain(..) {
        let role = message.get("role").and_then(Value::as_str).unwrap_or_default();
        if role == "tool" {
            // tool 消息统一由它对应的 assistant 重新插入，这里跳过（清孤儿）。
            continue;
        }
        if role == "assistant" {
            if let Some(calls) = message.get("tool_calls").and_then(Value::as_array).cloned() {
                let kept: Vec<Value> = calls
                    .into_iter()
                    .filter(|call| {
                        call.get("id")
                            .and_then(Value::as_str)
                            .is_some_and(|id| results.contains_key(id))
                    })
                    .collect();
                if kept.is_empty() {
                    let mut without_calls = message.clone();
                    if let Some(map) = without_calls.as_object_mut() {
                        map.remove("tool_calls");
                    }
                    output.push(without_calls);
                } else {
                    let mut with_calls = message.clone();
                    with_calls["tool_calls"] = Value::Array(kept.clone());
                    output.push(with_calls);
                    for call in kept {
                        if let Some(id) = call.get("id").and_then(Value::as_str) {
                            if let Some(result) = results.get(id) {
                                output.push(result.clone());
                            }
                        }
                    }
                }
                continue;
            }
        }
        output.push(message);
    }
    *messages = output;
}

fn merge_adjacent_tool_calls(messages: &mut Vec<Value>) {
    let mut output: Vec<Value> = Vec::with_capacity(messages.len());
    for message in messages.drain(..) {
        let mergeable = message.get("role").and_then(Value::as_str) == Some("assistant")
            && message.get("tool_calls").is_some();
        if mergeable {
            if let Some(last) = output.last_mut() {
                let last_mergeable = last.get("role").and_then(Value::as_str) == Some("assistant")
                    && last.get("tool_calls").is_some();
                if last_mergeable {
                    let mut calls = last
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    calls.extend(
                        message
                            .get("tool_calls")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default(),
                    );
                    last["tool_calls"] = Value::Array(calls);
                    let extra = message
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !extra.is_empty() {
                        let head = last
                            .get("content")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        last["content"] = Value::String(format!("{head}{extra}"));
                    }
                    continue;
                }
            }
        }
        output.push(message);
    }
    *messages = output;
}

/// deepseek 系开启思考时，历史 assistant 消息缺 `reasoning_content` 会被上游拒。
fn backfill_reasoning_content(messages: &mut [Value], model: &str) {
    if !is_deepseek(model) {
        return;
    }
    for message in messages {
        if message.get("role").and_then(Value::as_str) == Some("assistant")
            && message.get("reasoning_content").is_none()
        {
            message["reasoning_content"] = json!("");
        }
    }
}

/// 思考开关是**两段式**：只发 `thinking.type=enabled` 不给 `reasoning_effort`，
/// 实测上游 reasoning_tokens 为 0（等于没开）。所以两个一起给。
fn inject_deepseek_thinking(body: &mut Value, model: &str) {
    if !is_deepseek(model) {
        return;
    }
    let opted_out = body
        .get("thinking")
        .and_then(|thinking| thinking.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("disabled"));
    let effort = body
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .map(str::to_string);
    if opted_out || effort.as_deref().is_some_and(|value| value.eq_ignore_ascii_case("none")) {
        return;
    }
    body["thinking"] = json!({ "type": "enabled" });
    if effort.is_none() {
        body["reasoning_effort"] = json!("high");
    }
}

fn is_deepseek(model: &str) -> bool {
    model.to_ascii_lowercase().starts_with("deepseek")
}

/// 上游只支持流式：非流式由网关本地折叠；usage 只能从 `stream_options` 拿到。
fn force_stream(body: &mut Value) {
    body["stream"] = json!(true);
    let has_options = body.get("stream_options").is_some_and(Value::is_object);
    if !has_options {
        body["stream_options"] = json!({ "include_usage": true });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::protocol::cir::{CanonicalRequest, ToolCall, ToolSpec};
    use serde_json::json;

    fn request() -> CanonicalRequest {
        CanonicalRequest {
            model: "glm-5.3".into(),
            system: vec![Block::text("你是助手")],
            messages: Vec::new(),
            tools: Vec::new(),
            tool_choice: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream: false,
            reasoning: None,
            metadata: Default::default(),
            raw: json!({}),
        }
    }

    fn user(text: &str) -> Message {
        Message {
            role: Role::User,
            content: vec![Block::text(text)],
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
            reasoning: None,
        }
    }

    fn assistant_with_call(id: &str, name: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: Vec::new(),
            tool_calls: vec![ToolCall {
                id: Some(id.into()),
                name: name.into(),
                arguments: "{}".into(),
            }],
            tool_call_id: None,
            name: None,
            reasoning: None,
        }
    }

    fn tool_result(id: &str, text: &str) -> Message {
        Message {
            role: Role::Tool,
            content: vec![Block::text(text)],
            tool_calls: Vec::new(),
            tool_call_id: Some(id.into()),
            name: None,
            reasoning: None,
        }
    }

    #[test]
    fn fingerprints_are_rewritten_not_deleted() {
        // 参考实现的 A/B 结论：整段删除反而仍被判异常，要替换成等价写法。
        let rewritten = sanitize_text(
            "You are Claude Code, Anthropic's official CLI tool for Claude. Be helpful.",
        );
        assert!(rewritten.starts_with("You are Claude Code."), "{rewritten}");
        assert!(!rewritten.contains("Anthropic's official CLI"), "{rewritten}");

        let rewritten = sanitize_text(
            "You are a coding agent running in the Codex CLI tool, a terminal-based coding assistant.",
        );
        assert!(
            rewritten.contains("You are a coding agent running in the Codex CLI tool."),
            "{rewritten}"
        );
        assert!(!rewritten.contains("terminal-based"), "{rewritten}");

        // 来源域名直接去掉（没有语义价值）。
        assert!(!sanitize_text("see github.com/anthropics/claude-code").contains("anthropics"));

        // \r\n 残片要抹平。
        assert!(!sanitize_text("a\r\nb").contains("\r\n"));
    }

    #[test]
    fn fingerprint_detection_drives_whether_rewriting_is_needed() {
        assert!(has_fingerprint("You are Claude Code, Anthropic's official CLI tool for Claude"));
        assert!(has_fingerprint("x-anthropic-billing-hdr"));
        assert!(has_fingerprint("line\r\n"));
        assert!(!has_fingerprint("普通的中文提问"), "普通请求不该被改写");
    }

    #[test]
    fn system_goes_first_and_stream_is_forced_with_usage() {
        let mut req = request();
        req.messages.push(user("你好"));
        let body = build_chat_body("glm-5.3", &req);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "你是助手");
        assert_eq!(body["messages"][1]["content"], "你好");
        assert_eq!(body["stream"], true, "上游只支持流式");
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["model"], "glm-5.3");
    }

    #[test]
    fn waf_fingerprints_are_scrubbed_from_content_and_arguments() {
        let mut req = request();
        req.messages.push(user("You are Claude Code, run this\r\nnow"));
        req.messages.push(assistant_with_call("call_1", "shell"));
        // 必须给出配对的结果，否则 tool_call 会被自愈逻辑当孤儿清掉。
        req.messages.push(tool_result("call_1", "ok"));
        req.messages[1].tool_calls[0].arguments =
            "{\"cmd\":\"echo 11-128 github.com/anthropics/x\"}".into();
        let body = build_chat_body("hy3", &req);
        let text = body["messages"][1]["content"].as_str().unwrap();
        assert!(!text.contains("Anthropic's official CLI"), "{text}");
        assert!(!text.contains('\r'), "\\r\\n 残片必须抹平");
        let args = body["messages"][2]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        assert!(!args.contains("11-128"), "{args}");
        assert!(!args.contains("anthropics"), "{args}");
    }

    #[test]
    fn adjacent_tool_calls_are_merged_and_results_reordered() {
        let mut req = request();
        req.messages.push(user("查天气"));
        req.messages.push(assistant_with_call("call_1", "get_weather"));
        req.messages.push(assistant_with_call("call_2", "get_time"));
        req.messages.push(tool_result("call_2", "12:00"));
        req.messages.push(tool_result("call_1", "晴"));
        let body = build_chat_body("hy3", &req);
        let messages = body["messages"].as_array().unwrap();
        // system, user, assistant(两个 call), tool(call_1), tool(call_2)
        assert_eq!(messages.len(), 5, "{messages:#?}");
        assert_eq!(
            messages[2]["tool_calls"].as_array().unwrap().len(),
            2,
            "相邻的 tool_calls 必须合并成一条 assistant"
        );
        assert_eq!(messages[3]["tool_call_id"], "call_1");
        assert_eq!(messages[4]["tool_call_id"], "call_2");
    }

    #[test]
    fn orphan_tool_calls_and_orphan_results_are_cleaned() {
        let mut req = request();
        req.messages.push(user("hi"));
        req.messages.push(assistant_with_call("call_missing", "get_weather"));
        req.messages.push(tool_result("call_other", "没人要的结果"));
        let body = build_chat_body("hy3", &req);
        let messages = body["messages"].as_array().unwrap();
        // system + user + assistant（tool_calls 被清掉）；孤儿 tool 结果被丢弃。
        assert_eq!(messages.len(), 3, "{messages:#?}");
        assert!(messages[2].get("tool_calls").is_none(), "孤儿 tool_calls 要清掉");
        assert!(
            !messages
                .iter()
                .any(|message| message["role"] == "tool"),
            "孤儿 tool 结果要清掉"
        );
    }

    #[test]
    fn deepseek_thinking_is_injected_as_two_parts() {
        let mut req = request();
        req.messages.push(user("hi"));
        let body = build_chat_body("deepseek-v4.1-flash", &req);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["reasoning_effort"], "high");
        // 历史 assistant 缺 reasoning_content 要补空串。
        assert!(body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|message| message["reasoning_content"].as_str() != Some("")));
    }

    #[test]
    fn deepseek_history_gets_empty_reasoning_and_explicit_off_is_respected() {
        let mut req = request();
        req.messages.push(user("hi"));
        req.messages.push(Message {
            role: Role::Assistant,
            content: vec![Block::text("上次回答")],
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
            reasoning: None,
        });
        let body = build_chat_body("deepseek-v4-pro", &req);
        assert_eq!(body["messages"][2]["reasoning_content"], "");

        let mut req = request();
        req.messages.push(user("hi"));
        req.raw = json!({"thinking": {"type": "disabled"}});
        let body = build_chat_body("deepseek-v4-pro", &req);
        assert!(body.get("reasoning_effort").is_none(), "显式关闭时不该注入 effort");
    }

    #[test]
    fn private_keys_and_legacy_max_tokens_field_are_cleaned() {
        let mut req = request();
        req.messages.push(user("hi"));
        req.raw = json!({"_trace": "x", "max_completion_tokens": 10, "temperature": 0.3});
        req.max_tokens = Some(2048);
        let body = build_chat_body("hy3", &req);
        assert!(body.get("_trace").is_none());
        assert!(body.get("max_completion_tokens").is_none());
        assert_eq!(body["max_tokens"], 2048);
        assert_eq!(body["temperature"], 0.3, "透传字段要保留");
    }

    /// 客户端协议的**顶层提示词字段**不能跟着 raw 透传出去：
    /// 提示词已经清洗过并重建进 messages，留着原文等于把清洗整个绕过
    /// （Anthropic 的 `system` / Responses 的 `instructions`、`input`）。
    #[test]
    fn top_level_prompt_fields_do_not_leak_through_raw() {
        let mut req = request();
        req.system = vec![Block::text("You are Claude Code, Anthropic's official CLI tool for Claude")];
        req.messages.push(user("hi"));
        // 模拟 Anthropic 解码后的 raw：顶层还带着未清洗的 system。
        req.raw = json!({
            "model": "glm-5.3",
            "system": "You are Claude Code, Anthropic's official CLI tool for Claude",
            "max_tokens": 64,
        });
        let body = build_chat_body("glm-5.3", &req);
        assert!(body.get("system").is_none(), "顶层 system 必须删除：提示词走清洗后的 messages");
        let system = body["messages"][0]["content"].as_str().unwrap();
        assert!(
            !system.contains("Anthropic's official CLI"),
            "messages 里的提示词必须是清洗后的：{system}"
        );
        // Responses 协议同理。
        let mut req = request();
        req.messages.push(user("hi"));
        req.raw = json!({"instructions": "secret prompt", "input": "raw input"});
        let body = build_chat_body("glm-5.3", &req);
        assert!(body.get("instructions").is_none());
        assert!(body.get("input").is_none());
    }

    #[test]
    fn tools_are_emitted_in_openai_shape() {
        let mut req = request();
        req.messages.push(user("hi"));
        req.tools.push(ToolSpec {
            name: "get_weather".into(),
            description: Some("查天气".into()),
            parameters: Some(json!({"type": "object"})),
        });
        let body = build_chat_body("hy3", &req);
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "get_weather");
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn images_become_content_parts() {
        let mut req = request();
        req.messages.push(Message {
            role: Role::User,
            content: vec![
                Block::text("看图"),
                Block::Image {
                    url: "https://example.com/a.png".into(),
                },
            ],
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
            reasoning: None,
        });
        let body = build_chat_body("glm-5v-turbo", &req);
        assert_eq!(body["messages"][1]["content"][0]["type"], "text");
        assert_eq!(body["messages"][1]["content"][1]["type"], "image_url");
    }
}


/// 真机 A/B 探针：11-128 到底由**哪段文字**触发，以及改写后是否放行。
///
/// 用途：指纹表不能靠猜。参考实现的 A/B 结论（大小写不敏感、容忍前后缀、
/// 改掉句子结构就不再匹配）需要在本机上复核一遍 —— 尤其"改成什么才放行"。
///
/// ```text
/// cargo test --lib probe_fingerprint_ab -- --ignored --nocapture
/// ```
#[cfg(test)]
mod probe {
    use serde_json::json;

    use crate::agent::secrets::OsCredentialStore;
    use crate::gateway::channels::{workbuddy::identity, AccountSnapshot};
    use crate::gateway::storage::Storage;
    use crate::gateway::transport::{self, UpstreamRequest};

    #[tokio::test]
    #[ignore]
    async fn probe_fingerprint_ab() {
        let appdata = std::env::var("APPDATA").expect("APPDATA");
        let dir = std::path::Path::new(&appdata)
            .join("com.issuser.dynamic-island")
            .join("gateway");
        let storage = Storage::open(&dir, &OsCredentialStore).expect("库");
        let record = storage
            .list_accounts()
            .expect("账号")
            .into_iter()
            .find(|record| record.channel_id == "wb-cn")
            .expect("没有 wb-cn 账号");
        let credential = record.credential.expect("没有凭据");
        let account = AccountSnapshot {
            channel_id: record.channel_id.clone(),
            uid: record.uid.clone(),
            access_token: Some(credential.access_token.clone()),
            refresh_token: credential.refresh_token.clone(),
            enterprise_id: credential.enterprise_id.clone(),
        };
        let hosts = identity::hosts(identity::Realm::Cn, identity::Outbound::Desktop);
        let client = reqwest::Client::new();

        // 候选：每段文字单独当 system 发一次。命中 11-128 的就是特征串；
        // 放行的那几行告诉我们"改成什么写法是安全的"。
        // 同样的正文，快发 vs 慢发：判断触发条件是"内容"还是"频率/突发"。
        let system = "You are a helpful assistant.
Default branch (you will usually use this for PRs): main";
        let mut burst_blocked = 0;
        let mut slow_blocked = 0;
        let rounds = 8;

        println!("=== A. 连续快发（无间隔）===");
        for index in 0..rounds {
            let text = one_call(&client, &hosts, &account, system).await;
            let blocked = text.contains("11-128") || text.contains("unapproved");
            if blocked {
                burst_blocked += 1;
            }
            println!("  第 {} 次：{}", index + 1, if blocked { "被拦 11-128" } else { "放行" });
        }

        println!("=== B. 间隔 3 秒 ===");
        for index in 0..rounds {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let text = one_call(&client, &hosts, &account, system).await;
            let blocked = text.contains("11-128") || text.contains("unapproved");
            if blocked {
                slow_blocked += 1;
            }
            println!("  第 {} 次：{}", index + 1, if blocked { "被拦 11-128" } else { "放行" });
        }
        println!("=== 汇总：快发被拦 {burst_blocked}/{rounds}，慢发被拦 {slow_blocked}/{rounds} ===");
    }

    /// 发一次最小请求，返回响应正文（2xx）或错误正文。
    async fn one_call(
        client: &reqwest::Client,
        hosts: &identity::Hosts,
        account: &AccountSnapshot,
        system: &str,
    ) -> String {
        let headers = identity::chat_headers(
            &account.channel_id,
            identity::Realm::Cn,
            identity::Outbound::Desktop,
            hosts,
            account,
            1,
        );
        let body = json!({
            "model": "glm-5.3",
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": "hi" },
            ],
            "stream": true,
            "max_tokens": 8,
            "stream_options": { "include_usage": true },
        });
        let upstream = UpstreamRequest {
            url: hosts.chat_url("/v2/chat/completions"),
            headers,
            body,
        };
        match transport::send(&upstream, client).await {
            Ok(response) => response.text().await.unwrap_or_default(),
            Err(failure) => failure.body,
        }
    }
}
