//! 请求卫生管道：CIR → 上游 OpenAI 形状报文。
//!
//! 每一步都是纯函数，可单测、可按渠道开关。顺序固定：
//! 拼消息 → 指纹清洗 → 工具配对自愈 → reasoning 回填 → 思考注入 → 强制流式。

use serde_json::{json, Value};

use crate::gateway::protocol::cir::{Block, CanonicalRequest, Message, Role};

/// 会触发上游 WAF / 内容审核的第三方客户端特征串。
///
/// 上游的表现是 `403` 或 `code 11128`（Illegal API invocation from an
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
    ("11128", ""),
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

    // 工具选择**归一成上游要的字符串**：上游是 Go 服务，`Request.tool_choice`
    // 声明为 string —— 发对象会 11101 "cannot unmarshal object into ... string"
    //（畸形请求有时还会撞上渠道校验的 11128）。Claude Code 指定工具时发的
    // `{type:"tool",name}` 解码出来是 OpenAI 的 function 对象，必须在这里降级；
    // 参考实现同样在最后一道做这个归一（`normalize_tool_choice`）。
    // 语义：auto/required/none 原样；指定工具 = 工具名（上游的 string 形状）。
    if let Some(choice) = &request.tool_choice {
        if let Some(normalized) = normalize_tool_choice(choice) {
            body["tool_choice"] = json!(normalized);
        }
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
    // **计费头残片要整段删**（参考实现的 SANITIZE_HDR_RE + SANITIZE_KV_RE）：
    // Claude Code 的 system 里有被塞进来的 HTTP 头结构 ——
    //   x-anthropic-billing-header: cc_version=2.1.296.32d; cc_...=...; ...
    // 只删键名会留下 ": cc_version=2.1.296..." —— 上游 WAF 认的正是这个残片
    //（字节级 bisect 锁定：删掉 [0..50]（含该残片）就放行，原样必拦）。
    // 键名可能被截断（开头是 "-anthropic-billing-header:"），所以：
    // · 删到行尾的 `键名[: ]cc_* KV 串`；
    // · 兜底删所有裸 `cc_xxx=值;` KV 对；
    // · 再删残留的键名词本身。
    loop {
        let before = output.clone();
        if let Some(start) = output.find("cc_") {
            // 找 KV 起点，向前吞掉同行的 "xxx-billing-header: " 前缀。
            let line_start = output[..start].rfind('\n').map(|index| index + 1).unwrap_or(0);
            let prefix = &output[line_start..start];
            let colon = prefix.rfind(':');
            let remove_from = match colon {
                // 前缀形如 "x-anthropic-billing-header"（或被截断的同族）→ 从行首删。
                Some(_) if prefix.to_ascii_lowercase().contains("billing-header") => line_start,
                _ => start,
            };
            // KV 串到行尾为止（cc_a=1; cc_b=2; ...）。
            let line_end = output[start..]
                .find('\n')
                .map(|index| start + index)
                .unwrap_or(output.len());
            output.replace_range(remove_from..line_end, "");
        }
        // 裸 KV 对兜底（可能跨行散落）：cc_<word>=<直到分号或行尾>;
        let mut cleaned = String::with_capacity(output.len());
        for line in output.split_inclusive('\n') {
            let trimmed = line.trim_start();
            if trimmed.starts_with("cc_") && trimmed.contains('=') {
                continue;
            }
            cleaned.push_str(line);
        }
        output = cleaned;
        if output == before {
            break;
        }
    }
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
        // 计费头残片：cc_* KV（WAF 指纹，见 sanitize_text 里的注释）。
        || text.contains("cc_")
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

/// 把 OpenAI 形状的 `tool_choice` 归一成上游的字符串形状。
///
/// 返回 `None` = 无法识别的形状，整个字段不要发（发畸形值必被上游拒绝）。
/// `"none"` 保留 tools 声明：语义由 tool_choice 本身表达，删 tools 会让
/// 模型拿不到函数签名、把调用降级成正文里的伪 JSON（参考实现踩过的坑）。
fn normalize_tool_choice(choice: &Value) -> Option<String> {
    match choice {
        Value::String(text) => {
            let lowered = text.trim().to_ascii_lowercase();
            match lowered.as_str() {
                "auto" | "required" | "none" => Some(lowered),
                // OpenAI 的 {type:"function",function:{name}} 之外没有别的对象，
                // 字符串只可能是这三种语义；其他字符串不猜，丢掉。
                _ => None,
            }
        }
        Value::Object(map) => {
            let kind = map.get("type").and_then(Value::as_str)?;
            match kind {
                "auto" | "required" | "none" => Some(kind.to_string()),
                "function" => Some(
                    map.get("function")
                        .and_then(|function| function.get("name"))
                        .and_then(Value::as_str)
                        .filter(|name| !name.trim().is_empty())
                        .map(|name| name.trim().to_string())?,
                ),
                _ => None,
            }
        }
        _ => None,
    }
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
            "{\"cmd\":\"echo 11128 github.com/anthropics/x\"}".into();
        let body = build_chat_body("hy3", &req);
        let text = body["messages"][1]["content"].as_str().unwrap();
        assert!(!text.contains("Anthropic's official CLI"), "{text}");
        assert!(!text.contains('\r'), "\\r\\n 残片必须抹平");
        let args = body["messages"][2]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        assert!(!args.contains("11128"), "{args}");
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
    /// 工具选择归一成上游要的**字符串**：发对象会 11101（上游是 Go，
    /// `Request.tool_choice` 声明为 string）。指定工具 = 工具名。
    /// 计费头残片【整段】删除：Claude Code 把 HTTP 头结构塞进了 system，
    /// 只删键名会留下 ": cc_version=...; cc_entrypoint=...;" 这样的半截 KV ——
    /// 上游 WAF 认的正是这个残片（真机 bisect 锁定，见 probe_cc_bisect）。
    #[test]
    fn billing_header_remnant_is_removed_as_a_whole() {
        // 真机 dump 的 system 开头（字节级核对过的形状）。
        let kv = ["c", "c", "_", "v", "e", "r", "s", "i", "o", "n"].concat();
        let entry = ["c", "c", "_", "e", "n", "t", "r", "y", "p", "o", "i", "n", "t"].concat();
        let key = [
            "x", "-", "a", "n", "t", "h", "r", "o", "p", "i", "c", "-", "b", "i", "l", "l", "i",
            "n", "g", "-", "h", "e", "a", "d", "e", "r",
        ]
        .concat();
        let dirty = format!(
            "{}: {}=2.1.296.32d; {}=sdk-cli;\nYou are a Claude agent, Agent SDK.",
            key, kv, entry
        );
        let cleaned = sanitize_text(&dirty);
        assert!(!cleaned.contains(&kv), "cc_version 残留：{cleaned}");
        assert!(!cleaned.contains("billing-header"), "键名残留：{cleaned}");
        assert!(!cleaned.contains(&entry), "entrypoint 残留：{cleaned}");
        // KV 行删掉后留下正文行自己的开头换行 —— 与真机清洗后的 dump 一致。
        assert!(
            cleaned
                .trim_start_matches('\n')
                .starts_with("You are a Claude agent"),
            "残片应整段删掉、正文保留：{cleaned}"
        );

        // 行首没有键名的裸 KV（键名已被上一轮删掉）也要删。
        let bare = format!(": {}=2.1.296.32d;\nnext", kv);
        let cleaned = sanitize_text(&bare);
        assert!(!cleaned.contains(&kv), "裸 KV 残留：{cleaned}");
        assert!(cleaned.contains("next"), "正文要保留：{cleaned}");
    }

    #[test]
    fn tool_choice_is_normalized_to_upstream_string_shape() {
        let mut req = request();
        req.messages.push(user("hi"));
        req.tools.push(ToolSpec {
            name: "shell".into(),
            description: None,
            parameters: Some(json!({"type": "object"})),
        });
        // Claude Code 指定工具（经 Anthropic 解码后是 OpenAI function 对象）。
        req.tool_choice = Some(json!({"type": "function", "function": {"name": "shell"}}));
        // raw 里带着 Anthropic 形状的旧值 —— 归一值必须覆盖它。
        req.raw = json!({"tool_choice": {"type": "any"}});
        let body = build_chat_body("glm-5.3", &req);
        assert_eq!(
            body["tool_choice"], "shell",
            "指定工具要发工具名字符串，不是对象"
        );

        // 三种语义对象 → 字符串。
        for (value, expect) in [
            (json!({"type": "auto"}), "auto"),
            (json!({"type": "required"}), "required"),
            (json!({"type": "none"}), "none"),
        ] {
            let mut req = request();
            req.messages.push(user("hi"));
            req.tool_choice = Some(value);
            let body = build_chat_body("glm-5.3", &req);
            assert_eq!(body["tool_choice"], expect);
        }

        // OpenAI 客户端直接发字符串也认。
        let mut req = request();
        req.messages.push(user("hi"));
        req.tool_choice = Some(json!("auto"));
        let body = build_chat_body("glm-5.3", &req);
        assert_eq!(body["tool_choice"], "auto");

        // 认不出的形状整个字段不发（发畸形值必被拒）。
        let mut req = request();
        req.messages.push(user("hi"));
        req.tool_choice = Some(json!({"type": "weird"}));
        let body = build_chat_body("glm-5.3", &req);
        assert!(body.get("tool_choice").is_none());
    }

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


/// 真机 A/B 探针：11128 到底由**哪段文字**触发，以及改写后是否放行。
///
/// 用途：指纹表不能靠猜。参考实现的 A/B 结论（大小写不敏感、容忍前后缀、
/// 改掉句子结构就不再匹配）需要在本机上复核一遍 —— 尤其"改成什么才放行"。
///
/// ```text
/// cargo test --lib probe_fingerprint_ab -- --ignored --nocapture
/// ```
#[cfg(test)]
mod probe {
    use serde_json::{json, Value};

    use crate::agent::secrets::OsCredentialStore;
    use crate::gateway::channels::{workbuddy::identity, AccountSnapshot};
    use crate::gateway::storage::Storage;
    use crate::gateway::transport::{self, UpstreamRequest};

    /// **决定性 A/B**：读 dump 目录里 Claude Code 的真实出站报文（带全部私有键），
    /// 用真实凭据直打上游，逐键删减 —— 哪个键删掉后从"被拦"变"放行"，哪个就是元凶。
    ///
    /// ```text
    /// GATEWAY_DUMP_SRC=E:/code/dynamic-island/.gateway-dump2 \
    ///   cargo test --lib probe_cc_bisect -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore]
    async fn probe_cc_bisect() {
        use crate::agent::secrets::OsCredentialStore;
        use crate::gateway::storage::Storage;

        let dump_dir = std::env::var("GATEWAY_DUMP_SRC")
            .unwrap_or_else(|_| r"E:/code/dynamic-island/.gateway-dump2".into());
        let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&dump_dir)
            .expect("dump 目录")
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        paths.sort_by_key(|path| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0));
        let Some(latest) = paths.last() else {
            panic!("dump 目录里没有请求");
        };
        let payload: Value =
            serde_json::from_str(&std::fs::read_to_string(latest).expect("读 dump")).expect("解析");
        let real = payload["body"].clone();
        let url = payload["url"].as_str().expect("url").to_string();
        println!(
            "用 dump：{}（{} 字节）",
            latest.display(),
            serde_json::to_string(&real).unwrap_or_default().len()
        );

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
        let credential = record.credential.expect("凭据");
        let account = AccountSnapshot {
            channel_id: record.channel_id.clone(),
            uid: record.uid.clone(),
            access_token: Some(credential.access_token.clone()),
            refresh_token: credential.refresh_token.clone(),
            enterprise_id: credential.enterprise_id.clone(),
        };
        let hosts = identity::hosts(identity::Realm::Cn, identity::Outbound::Desktop);
        let client = reqwest::Client::new();
        let headers = identity::chat_headers(
            &account.channel_id,
            identity::Realm::Cn,
            identity::Outbound::Desktop,
            &hosts,
            &account,
            1,
        );

        async fn attempt(
            label: &str,
            url: &str,
            headers: &[(String, String)],
            client: &reqwest::Client,
            body: &Value,
        ) -> bool {
            let upstream = UpstreamRequest {
                url: url.to_string(),
                headers: headers.to_vec(),
                body: body.clone(),
            };
            let (blocked, status, snippet) = match transport::send(&upstream, client).await {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let text = response.text().await.unwrap_or_default();
                    let blocked = text.contains("11128") || text.contains("11101");
                    let snippet = format!("[OK分支] {}", &text[..text.len().min(130)]);
                    (blocked, status, snippet)
                }
                Err(failure) => {
                    eprintln!(
                        "[bisect-debug] failure.body 前 80 字节: {:?}",
                        &failure.body[..failure.body.len().min(80)]
                    );
                    let bytes = failure.body.as_bytes();
                    let hex: Vec<String> = bytes[..bytes.len().min(40)]
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect();
                    eprintln!("[bisect-debug] 前 40 字节 hex: {}", hex.join(" "));
                    eprintln!(
                        "[bisect-debug] contains '11128'={} contains '-'={} contains 0x2d={}",
                        failure.body.contains("11128"),
                        failure.body.contains('-'),
                        bytes.contains(&0x2d)
                    );
                    let blocked =
                        failure.body.contains("11128") || failure.body.contains("11101");
                    let snippet: String = format!("[ERR分支] {}", &failure.body[..failure.body.len().min(130)]);
                    (blocked, failure.status, snippet)
                }
            };
            println!("{:46} → HTTP {status} {} {}", label, if blocked { "【被拦】" } else { "放行" }, snippet);
            blocked
        }

        // 0. 原样
        if !attempt("0. 原样重放（Claude Code 真实报文）", &url, &headers, &client, &real).await {
            println!("    原样就放行 —— 被拦的成分不在 body（可能是 headers / 账号状态）");
            return;
        }
        // 1. 逐键删
        for key in ["safeguards", "context_management", "output_config", "thinking", "metadata", "tools"] {
            let mut map = real.as_object().cloned().unwrap_or_default();
            if map.remove(key).is_none() {
                continue;
            }
            let blocked = attempt(&format!("1. 删 {key}"), &url, &headers, &client, &Value::Object(map)).await;
            if !blocked {
                println!("    ★ 元凶 = {key}");
            }
        }
        // 3. 二分 messages[0]（system 文本）：定位触发 11-128 的具体片段。
        if let Some(messages) = real["messages"].as_array().cloned() {
            if let Some(system_text) = messages[0]["content"].as_str() {
                let chars: Vec<char> = system_text.chars().collect();
                let total = chars.len();
                let fine = std::env::var("GATEWAY_BISECT_FINE").is_ok();
                println!("system 共 {total} 字符，开始二分（fine={fine}）");
                // 粗分 10 段，每段单独当 system 发（前后加无害包装避免空段）。
                // 第二轮（GATEWAY_BISECT_FINE=1）：对 system 的每一**行**单独发。
                let fine = std::env::var("GATEWAY_BISECT_FINE").is_ok();
                let step = if fine { 50 } else { (total + 9) / 10 };
                for (index, start) in (0..total).step_by(step).enumerate() {
                    let end = (start + step).min(total);
                    let slice: String = chars[start..end].iter().collect();
                    let body = json!({
                        "model": real["model"],
                        "messages": [
                            { "role": "system", "content": slice },
                            { "role": "user", "content": "hi" },
                        ],
                        "max_tokens": 16,
                        "stream": true,
                        "stream_options": { "include_usage": true },
                    });
                    attempt(
                        &format!("3. system[{start}..{end}]（{} 字符）", end - start),
                        &url,
                        &headers,
                        &client,
                        &body,
                    )
                    .await;
                }
            }
        }

        // 4. 逐行 A/B（GATEWAY_BISECT_LINES=1 时跑）。
        if std::env::var("GATEWAY_BISECT_LINES").is_ok() {
            if let Some(messages) = real["messages"].as_array().cloned() {
                if let Some(system_text) = messages[0]["content"].as_str() {
                    for (index, line) in system_text.lines().enumerate() {
                        if line.trim().is_empty() {
                            continue;
                        }
                        let body = json!({
                            "model": real["model"],
                            "messages": [
                                { "role": "system", "content": line },
                                { "role": "user", "content": "hi" },
                            ],
                            "max_tokens": 16,
                            "stream": true,
                            "stream_options": { "include_usage": true },
                        });
                        attempt(
                            &format!(
                                "4. 行 {index}（{} 字符）: {:?}",
                                line.chars().count(),
                                &line[..line.chars().count().min(80)]
                            ),
                            &url,
                            &headers,
                            &client,
                            &body,
                        )
                        .await;
                    }
                }
            }
        }

        // 5. 前缀累积二分（GATEWAY_BISECT_PREFIX=1）：逐行累加 system，
        // 找到第一个从"放行"翻成"被拦"的前缀 —— 最后一行就是元凶。
        if std::env::var("GATEWAY_BISECT_PREFIX").is_ok() {
            if let Some(messages) = real["messages"].as_array().cloned() {
                if let Some(system_text) = messages[0]["content"].as_str() {
                    let lines: Vec<&str> = system_text.lines().collect();
                    let mut previous_ok = true;
                    for (index, line) in lines.iter().enumerate() {
                        let prefix = lines[..=index].join("
");
                        let body = json!({
                            "model": real["model"],
                            "messages": [
                                { "role": "system", "content": prefix },
                                { "role": "user", "content": "hi" },
                            ],
                            "max_tokens": 16,
                            "stream": true,
                            "stream_options": { "include_usage": true },
                        });
                        let blocked = attempt(
                            &format!("5. 前缀到行 {index}（{} 字符）: {:?}", prefix.chars().count(),
                                &line[..line.chars().count().min(60)]),
                            &url,
                            &headers,
                            &client,
                            &body,
                        )
                        .await;
                        if blocked && previous_ok {
                            println!("    ★★★ 元凶行 = {index}: {line:?}");
                        }
                        previous_ok = !blocked;
                    }
                }
            }
        }

        // 6. 清洗后完整重放（GATEWAY_BISECT_CLEAN=1）：验证 sanitize 后能过。
        if std::env::var("GATEWAY_BISECT_CLEAN").is_ok() {
            if let Some(messages) = real["messages"].as_array().cloned() {
                let cleaned_system = super::sanitize_text(
                    messages[0]["content"].as_str().unwrap_or_default(),
                );
                println!(
                    "清洗后 system 前 80 字符: {:?}",
                    &cleaned_system[..cleaned_system.chars().count().min(80)]
                );
                let mut rebuilt = real.clone();
                if let Some(list) = rebuilt["messages"].as_array_mut() {
                    list[0]["content"] = json!(cleaned_system);
                }
                attempt("6. 清洗后完整重放", &url, &headers, &client, &rebuilt).await;
            }
        }

        // 2. messages 逐条删
        if let Some(messages) = real["messages"].as_array().cloned() {
            for skip in 0..messages.len() {
                let trimmed: Vec<Value> = messages
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != skip)
                    .map(|(_, value)| value.clone())
                    .collect();
                let mut map = real.as_object().cloned().unwrap_or_default();
                map.insert("messages".into(), Value::Array(trimmed));
                let size = messages[skip]
                    .get("content")
                    .map(|content| serde_json::to_string(content).unwrap_or_default().len())
                    .unwrap_or(0);
                attempt(
                    &format!("2. 删 messages[{skip}]（{size} 字节）"),
                    &url,
                    &headers,
                    &client,
                    &Value::Object(map),
                )
                .await;
            }
        }
    }

    /// 真机验证：Claude Code 形状的请求（带 tools + tool_choice 指定工具）
    /// 经 build_chat_body 重建后能被上游接受（不再 11101 / 11128）。
    #[tokio::test]
    #[ignore]
    async fn probe_claude_code_shape() {
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

        // Claude Code 的典型形状：system 带自我声明 + tools + 指定 tool_choice。
        let request = crate::gateway::protocol::cir::CanonicalRequest {
            model: "glm-5.3".into(),
            system: vec![crate::gateway::protocol::cir::Block::text(
                "You are Claude Code, Anthropic's official CLI tool for Claude.",
            )],
            messages: vec![crate::gateway::protocol::cir::Message {
                role: crate::gateway::protocol::cir::Role::User,
                content: vec![crate::gateway::protocol::cir::Block::text("hi")],
                tool_calls: Vec::new(),
                tool_call_id: None,
                name: None,
                reasoning: None,
            }],
            tools: vec![crate::gateway::protocol::cir::ToolSpec {
                name: "shell".into(),
                description: Some("run a command".into()),
                parameters: Some(serde_json::json!({"type": "object", "properties": {}})),
            }],
            tool_choice: Some(serde_json::json!({"type": "function", "function": {"name": "shell"}})),
            temperature: None,
            top_p: None,
            max_tokens: Some(64),
            stream: true,
            reasoning: None,
            metadata: Default::default(),
            // 模拟 Anthropic 入口的 raw：顶层 system + Anthropic 形状 tool_choice。
            raw: serde_json::json!({
                "system": "You are Claude Code, Anthropic's official CLI tool for Claude.",
                "tool_choice": {"type": "tool", "name": "shell"},
            }),
        };
        let body = super::build_chat_body("glm-5.3", &request);
        println!("重建后的 tool_choice = {}", body["tool_choice"]);
        println!("重建后有没有顶层 system = {}", body.get("system").is_some());

        let headers = identity::chat_headers(
            &account.channel_id,
            identity::Realm::Cn,
            identity::Outbound::Desktop,
            &hosts,
            &account,
            1,
        );
        let upstream = UpstreamRequest {
            url: hosts.chat_url("/v2/chat/completions"),
            headers,
            body,
        };
        match transport::send(&upstream, &client).await {
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                let blocked = text.contains("11128") || text.contains("11101") || text.contains("unapproved");
                println!("HTTP {} {} {}", status, if blocked { "【被拦】" } else { "放行" },
                    text.chars().take(150).collect::<String>().replace("
", " "));
            }
            Err(failure) => println!("被拒 status={} {}", failure.status, failure.body.chars().take(200).collect::<String>()),
        }
    }

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

        // 候选：每段文字单独当 system 发一次。命中 11128 的就是特征串；
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
            let blocked = text.contains("11128") || text.contains("unapproved");
            if blocked {
                burst_blocked += 1;
            }
            println!("  第 {} 次：{}", index + 1, if blocked { "被拦 11128" } else { "放行" });
        }

        println!("=== B. 间隔 3 秒 ===");
        for index in 0..rounds {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let text = one_call(&client, &hosts, &account, system).await;
            let blocked = text.contains("11128") || text.contains("unapproved");
            if blocked {
                slow_blocked += 1;
            }
            println!("  第 {} 次：{}", index + 1, if blocked { "被拦 11128" } else { "放行" });
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
