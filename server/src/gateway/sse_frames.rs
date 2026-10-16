//! SSE 帧重组：从任意网络字节块里切出**完整的一帧**。
//!
//! 原实现在 agent 模块（`agent::providers::pop_sse_frame`）。网关是独立服务后
//! 不能再依赖 agent，故整段搬进来（含单测）。
//!
//! 关键点：**先等分隔符到位再解 UTF-8** —— 否则中文被切在 chunk 边界上会乱码。

/// 从缓冲区里取出一帧（含分隔符），返回其 UTF-8 文本。
///
/// - `Ok(None)`：还没有完整的一帧，调用方应继续读网络。
/// - `Ok(Some(frame))`：一帧已消费（连同分隔符一起从 buffer 里移除）。
/// - `Err`：帧内容不是合法 UTF-8。
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

/// 解析一帧 SSE 里的 `event` / `data` 字段（多行 data 用换行拼接）。
/// 解析后的 SSE 帧（事件名 + 若干 data 行）。
///
/// 生产路径只用 [`pop_sse_frame`] 切帧、由渠道自己解释 JSON；这一层是给
/// 测试与排查用的完整解析（例如把上游原文贴进单测断言）。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[allow(dead_code)]
pub fn parse_sse(input: &str) -> Vec<SseEvent> {
    let mut events = Vec::new();
    let mut event = None;
    let mut data: Vec<String> = Vec::new();
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
    fn frame_reassembles_split_utf8_and_crlf_delimiter() {
        let frame = "data: {\"text\":\"你好\"}\r\n\r\n".as_bytes();
        let split = frame.iter().position(|b| *b >= 0x80).unwrap() + 1;
        let mut buffer = frame[..split].to_vec();
        assert!(pop_sse_frame(&mut buffer).unwrap().is_none());
        buffer.extend_from_slice(&frame[split..]);
        let parsed = pop_sse_frame(&mut buffer).unwrap().unwrap();
        assert_eq!(parse_sse(&parsed)[0].data, "{\"text\":\"你好\"}");
        assert!(buffer.is_empty());
    }

    #[test]
    fn lf_only_delimiter_is_accepted() {
        let mut buffer = b"data: hi\n\n".to_vec();
        assert_eq!(pop_sse_frame(&mut buffer).unwrap().as_deref(), Some("data: hi\n\n"));
        assert!(buffer.is_empty());
    }
}
