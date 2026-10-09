# 桌面 Agent 请求与实时轨迹实现记录

日期：2026-10-01。范围：供应商请求、网易云只读工具循环、实时轨迹、持久化与历史回放。文件修改、网页搜索、电脑控制、上下文压缩的完整实现不属于本次范围。

## 1. HTTP 400 的实测证据

供应商：火山方舟。地址：`https://ark.cn-beijing.volces.com/api/coding/v3/chat/completions`。使用现有启用配置，密钥从 Windows 凭据库直接读取，不打印、不写入报告。没有修改用户供应商配置或历史会话。

| 对照请求 | deepseek-v4.1-flash | glm-5.3-flash |
| --- | --- | --- |
| 最小 model/messages/stream | 200，流正常结束 | 200，流正常结束 |
| 原始点号工具名与原有参数组合 | **400，InvalidParameter** | 200，流正常结束 |
| 合法下划线工具名 | 200，流正常结束 | 200，流正常结束 |
| 单独添加 max_tokens | 200，流正常结束 | 200，流正常结束 |
| 单独添加 max_completion_tokens | 200，流正常结束 | 200，流正常结束 |
| 单独添加 stream_options.include_usage | 200，流正常结束 | 200，流正常结束 |

DeepSeek 返回的真实错误正文关键字段：

```text
code: InvalidParameter
message: tools.0.function.name: expected 1-128 ASCII letters, digits, underscores or hyphens
requestId: 0217908613467587093713527b8c530401a403eec8fc1525078b7
```

结论：已经确认 DeepSeek 拒绝 `netease.get_current_state` 中的点号，改为 `netease_get_current_state` 后通过。GLM 接受该次原始请求，不能把它之前所有报错都归因于点号。其他参数在当前两个模型上均通过，并非这次已确认的 400 原因。

生产请求保留内部点号命名，使用双向工具名映射；对外按三种接口共同的保守约束验证：1-64 个 ASCII 字母、数字、下划线或连字符，名称必须唯一。未知工具、映射冲突和无效参数均拒绝，不通过重试 400 或静默删除工具掩盖问题。

## 2. 供应商与工具循环

参考 DSH（DeepSeek Harness）的模块分工，而不是复制其框架或 UI 样式：

- `E:/deepseek-harness/packages/llm/llm-deepseek/src/serialize.ts`：结构化内容到请求的转换；工具回传保留原生调用 ID；只有工具调用轮次才回传 Chat 的 `reasoning_content`；无正文工具轮次使用空字符串而不是 null。
- `E:/deepseek-harness/packages/llm/llm-deepseek/src/translate.ts`：正文、推理、工具参数按块逐片转为统一事件，不靠解析最终文本执行工具。
- `E:/deepseek-harness/packages/client/ui-conversation/src/client/conversation-nodes/assistant.ts`：同一投影逻辑处理实时增量、完成和中断，按动画帧合并刷新。
- `E:/deepseek-harness/packages/client/ui-conversation/src/client/conversation-nodes/tool.ts`：工具调用与结果用 callId 关联，预留父子调用关系。

本项目职责：

| 模块 | 责任 |
| --- | --- |
| `src-tauri/src/agent/adapter.rs` | ProviderAdapter（模型供应商适配器）：地址、认证、请求转换、兼容档案、网络传输 |
| `src-tauri/src/agent/stream.rs` | 三个协议的 SSE（服务器推送事件）解码，保留原生调用 ID、签名与加密推理内容 |
| `src-tauri/src/agent/model.rs` | ModelMessage、ModelBlock、ModelStreamEvent 三套内部契约 |
| `src-tauri/src/agent/provider_error.rs` | 有界错误读取、错误分类与脱敏 |
| `src-tauri/src/agent/native_trace.rs` | 唯一生产循环，最多 8 步；取消、工具分发、上下文写入与事件发布 |
| `src-tauri/src/agent/tools.rs` | 真实可用工具与权限检查；当前只暴露网易云 L0 只读状态工具 |

Chat Completions 默认使用 `max_tokens`；官方 OpenAI 推理模型选择 `max_completion_tokens`。官方 OpenAI 与已验证的方舟启用流式用量参数；其他兼容地址默认不发送该参数。Responses 使用 `max_output_tokens`，Anthropic Messages 使用 `max_tokens`。

HTTP 非成功响应最多读取 16 KiB，保留状态、协议、模型、地址、错误码、参数、请求 ID 与脱敏正文。流中供应商错误也保留 code/param。密钥与认证信息过滤后才写入诊断事件；地址移除用户名、密码、查询参数与片段，不额外记录完整请求内容。

持久化模型消息包含正文、推理、原生工具调用及 JSON 结果；assistant 与本步所有工具结果在同一事务内写入，取消中途不留下孤立工具调用。供应商推理块绑定来源协议、模型和脱敏地址，切换模型或地址时不重放不兼容的签名/加密推理。其余正文和工具历史仍保留。

每次运行使用已解析的供应商、模型、兼容档案快照；选中模型必须启用并属于所选供应商。设置或模型选择的后续变化不会影响已开始的运行。

## 3. 执行事件与前端接口

执行事件 `ExecutionEvent`：

```typescript
{
  version: 1,
  id, sessionId, runId,
  step, blockId, callId, parentCallId,
  seq, eventType, payload, createdAt
}
```

`seq` 在会话范围内从 1 连续递增，由 SQLite 事务分配。事件提交成功后广播 `agent://trace`；终态事件与 runs.status 在同一事务中更新。

主要事件：`run_started`、`user_message`、`step_started`、`block_started`、`text_delta`、`reasoning_delta`、`tool_delta`、`block_ended`、`model_finished`、`tool_started`、`tool_completed`、`step_completed`、`usage`、`run_completed`、`run_failed`、`run_cancelled`、`run_interrupted`。

补取接口：`agent_list_events({ sessionId, afterSeq })`。返回 `seq > afterSeq` 的持久化事件，按 seq 排序。前端先订阅再发送，按事件 ID 去重，只投影连续序号；有缺口时暂存，定期补取。历史初始化、实时通知、缺口恢复使用同一个纯 reducer，不在结束后把已显示内容替换为一个汇总消息。

前端分工：

- `src/agent/trace.ts`：纯事件归并、排序、去重、终态恢复。
- `src/widgets/ai/useAgentConversation.ts`：会话加载、订阅、补取、动画帧合并、发送与自动滚动。
- `src/widgets/ai/ConversationRows.tsx`：Markdown 正文、可展开的紧凑思考行、工具行、错误诊断。
- `src/widgets/ai/HistorySidebar.tsx`：历史列表、新对话、折叠侧栏。
- `src/widgets/ai/legacy.ts`：旧 timeline/messages 的兼容显示，不伪造丢失的调用轨迹。

轨迹按真实发生顺序显示：思考 → 正文 → 工具 → 后续正文。工具名称与参数接收立即可见，随后更新执行中、成功或失败状态。思考只展示实际返回的内容，不生成假的思考摘要。UI 使用 Tailwind，工具图标 14px。用户上滚后停止自动追随；发送期间保留草稿、禁止误发，中文输入法确认不触发发送。

增量迁移新增 `agent_events`、`agent_context`、`agent_context_seeded`，保留旧表。旧消息只初始化结构化上下文一次。应用初始化将遗留 running 运行标为 interrupted，保留已写入事件；重复恢复不会再次生成中断事件。

## 4. 已执行验收

| 检查 | 结果 |
| --- | --- |
| `pnpm typecheck` | 通过 |
| `pnpm build` | 通过；保留 bundle >500 KiB 提示 |
| `cargo check` | 通过；有现有未使用脚手架代码警告 |
| `cargo test` | 32 个单元测试、5 个集成测试通过；2 个真实接口测试默认忽略 |
| `pnpm test:agent` | 6 个事件投影/历史兼容测试通过 |
| `git diff --check` | 通过，无空白错误 |
| `live_ark_request_matrix`，显式运行 | 两个真实配置模型完成上述对照 |
| `live_ark_production_tool_loop`，显式运行 | 两个模型通过生产工具循环及重新打开数据库后的下一轮请求 |
| 浏览器加载生产 hook/组件，替换桌面通信为测试事件 | 已验证完成前出现内容、交错显示、工具详情、400 详情、新会话、历史切换、折叠侧栏、取消、运行中草稿保留 |

真实循环最后一轮验证：DeepSeek 产生 1 个工具结果、32 个正文增量、33 个推理增量；GLM 产生 1 个工具结果、61 个正文增量、64 个推理增量。两个模型从重新打开的数据库恢复上下文后继续回答成功。增量数量是本次样本，不是固定指标。

真实模型测试的工具执行器返回明确标注的只读测试数据，并非用户实际音乐。用户数据库仅用于只读取得现有配置；会话和事件写在独立临时测试数据库，测试结束后清理。

自动测试覆盖实际 HTTP 请求的三种地址和认证、名称映射、token 参数、原生工具 ID 与结果回传、HTTP 400/401/429/500 与脱敏、流中错误、中文分包/CRLF/多行 SSE、多工具、非法参数、未知工具、意外中断、取消、8 步上限、工具失败后继续回答、并发 seq、重复回放、缺口恢复、旧会话及重启恢复。

## 5. 验收边界与复测

**原生 Tauri 桌面点击验收尚未完成。** 开发程序可以运行和截屏，但当前桌面自动化无法可靠取得该透明置顶窗口的前台进程身份，点击被判为落到外部窗口，键盘输入也被拒绝。没有绕过输入保护，也没有把浏览器模拟验收写成真实桌面验收。

仍需在桌面端手动确认：选择两个现有模型分别普通聊天；连接网易云后请求当前状态；运行中看到正文/思考/调用增量；取消保留部分内容；关闭并重新打开应用后恢复历史且无重复轨迹。Responses 与 Anthropic 本次通过生产适配器的本地 HTTP 服务验收，未使用真实账号验证这两个协议。

复测命令：

```powershell
pnpm typecheck
pnpm build
pnpm test:agent
cd src-tauri
cargo check
cargo test
cargo test live_ark_request_matrix -- --ignored --nocapture
cargo test live_ark_production_tool_loop -- --ignored --nocapture
```

后两个命令会读取现有方舟配置并消耗少量模型额度，默认测试不运行它们。浏览器测试入口为 `http://localhost:1420/tests/agent-trace-browser.html`，只用于开发服务，不进入正式应用路由或生产构建。

本次未提交 Git；用户原有配置和历史数据保留。
