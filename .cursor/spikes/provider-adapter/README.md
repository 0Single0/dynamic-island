# Provider Adapter Mock Spike

状态：已完成（PLAN-00-01）；生产 Adapter 留在 TASK-07。

本 Spike 用本地静态 fixture 验证三种 API 的请求投影与服务端事件边界，不访问网络、不包含密钥，也不实现生产 Provider。Fixture 来源于公开协议形状，后续应以真实供应商契约测试补足。

## 暂定 canonical 映射

| Canonical | OpenAI Chat Completions | OpenAI Responses | Anthropic Messages |
|---|---|---|---|
| 请求指令 | messages 中的 system role | instructions | 顶层 system |
| 工具定义 | tools[].function | tools[]，type=function | tools[]，input_schema |
| 文本增量 | choices[].delta.content | response.output_text.delta | content_block_delta.text_delta |
| 工具开始 | choices[].delta.tool_calls，按 index 累积 id/name | response.output_item.added 的 function_call item | content_block_start 的 tool_use block |
| 工具参数增量 | tool_calls[].function.arguments | response.function_call_arguments.delta | input_json_delta.partial_json |
| 完成/用量 | finish_reason 与 usage chunk | response.completed 的 response.usage | message_delta.stop_reason/usage；message_stop 结束 |

## Fixture 布局

- fixtures/chat-completions-request.json 与 chat-completions-stream.sse
- fixtures/responses-request.json 与 responses-stream.sse
- fixtures/anthropic-messages-request.json 与 anthropic-messages-stream.sse
- src-tauri/tests/provider_protocol_fixtures.rs 对三组请求字段、SSE 帧边界、文本/工具/用量事件进行离线校验。

## Spike 决策与边界

1. Provider Adapter 负责协议转换与流解析；Agent Loop 只消费 canonical event。
2. SSE 必须按空行组帧，支持一个事件中多条 data 行；不能假设网络 chunk 与 SSE 事件边界一致。
3. 工具参数是分片 JSON。只有终止事件到达、完整缓冲并通过 JSON Schema 校验后，才可交给权限审批；增量片段不得执行。
4. 保留供应商原始 tool/call ID 用于结果关联；不同供应商的结束事件和 usage 位置不统一。
5. 请求 fixture 仅记录非敏感字段。认证头由安全凭据层注入，绝不写入 fixture、事件或日志。
6. 本 Spike 不决定各供应商所有可选字段、重试/错误策略和价格表；对应能力由 TASK-07/TASK-21 实现和验证。
7. Chat Completions 的输出 token 字段采用 max_completion_tokens；实际 Provider 若只兼容旧字段，应由显式兼容配置处理，不静默猜测。

## 验收

在 src-tauri 目录执行 cargo test --test provider_protocol_fixtures。测试完全离线，并验证三种请求形状、SSE 分帧、文本增量、工具调用关联/参数分片及用量或完成信息。

## 协议参考

- [OpenAI Chat Completions API Reference](https://platform.openai.com/docs/api-reference/chat/create)
- [OpenAI Responses streaming events](https://platform.openai.com/docs/api-reference/responses-streaming)
- [Anthropic Messages streaming](https://docs.anthropic.com/en/api/messages-streaming)
