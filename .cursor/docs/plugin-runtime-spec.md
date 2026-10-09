# 内置插件自动注册与 Agent 能力运行时 — 规格

| 属性 | 值 |
|------|----|
| 规格版本 | 1.0 |
| 生效日期 | 2026-10-08 |
| 实现计划 | `.cursor/plan/plugin-runtime-plan.md` |
| 状态 | 已实现并通过 `cargo test` / `pnpm build` 验收 |

## 1. 目标

宿主把可复用的业务能力注册成稳定的 capability ID；每个内置插件只声明「如何把已有能力暴露给模型」。
Agent Loop 不再包含任何应用专用分支，只通过 `PluginRuntime` 解析工具并统一调用能力。

## 2. 四层契约

```text
业务模块 ──register──▶ CapabilityRegistry        （capability ID → CapabilityHandler）
插件清单 ──mount────▶ PluginRuntime                （PluginManifest → 工具名 / 绑定 / 状态）
请求     ──resolve──▶ ToolResolver（tools_for）    （默认范围 + @ 引用 → 模型工具目录）
调用     ──call─────▶ PluginRuntime::call          （模型工具名 → capability → 处理器）
```

| 类型 | 位置 | 职责 |
|------|------|------|
| `CapabilitySpec` / `CapabilityBinding` / `ToolExposure` / `PluginManifest` / `PluginStatus` | `src-tauri/src/agent/plugins/manifest.rs` | 能力声明、绑定、工具暴露、清单与只读状态 |
| `CapabilityRegistry` / `CapabilityHandler` / `HandlerContext` / `PluginToolCall` | `src-tauri/src/agent/plugins/registry.rs` | capability ID → 处理器的唯一注册表 |
| `PluginRuntime` / `PluginError` / `ToolRef` | `src-tauri/src/agent/plugins/runtime.rs` | 校验、挂载、解析、调用、超时、drain/unmount/reload |
| `PluginEnv` / `HandlerMap` / `bind` / `RunControl` | `src-tauri/src/agent/plugins/host.rs` | 宿主环境（AppHandle、取消集合、UIA 目标）、处理器装配 |

## 3. 工具命名与解析

- 模型可见名：`<namespace>__<tool>`，例如 `computer__list_windows`、`markdown__convert_to_pdf`。
  名称必须满足 `[A-Za-z0-9_-]{1,64}`（适配 OpenAI / Anthropic / Responses 的工具名约束）。
- 历史名：清单里的 `legacy_names`（点号写法 `a.b`、单下划线写法 `a_b`）只用于解析旧会话/旧客户端
  发来的调用，**不会发给模型**；`ToolNames` 在解码阶段把历史名解析回当前名。
- capability ID 保持点号风格（`music.player.current_state`、`files.read`、`web.search`…），
  它是宿主与业务模块之间的稳定契约，与模型可见名解耦。

## 4. 挂载与生命周期

- `mount_builtin_plugins(runtime, netease)` 在应用启动时按固定顺序挂载六个内置插件；
  挂载失败视为程序缺陷，启动中止（`lib.rs`）。
- 校验发生在写入之前，失败不留半注册状态：
  插件 id / namespace 字符集、能力 ID 唯一、绑定指向已声明能力、工具名合法且不重复、
  工具引用的能力存在、参数 schema 是 JSON object。
- 已实现能力（`available = true`）必须提供处理器，缺失返回 `HANDLER_MISSING`；
  预留能力（`available = false`，如 `computer.screenshot` / `computer.ocr` / `computer.send_input`）
  只登记声明，可以没有处理器，永远不会暴露为工具或被调用。
- 生命周期：`Active → Draining → Unmounted`；`reload` = unmount + mount，`generation` 单调递增。
- 每次调用都有超时预算：交互类工具 330s（原生确认窗口），其余默认 60s；超时返回
  `PLUGIN_TIMEOUT`（retryable）。

## 5. 请求范围（`@` 语义）

- 默认范围：`system`、`files`、`web_search`（清单 `default_scope = true`）。
- `@` 只为**当前这一条消息**追加工具：`@电脑` / `@computer` / `@computer-use` 等别名与插件 id、
  namespace 等价；解析结果写入 `pluginIds`，不存在的引用被忽略。
- 插件从不因 `@` 启动或停止；没有插件启停 UI，也没有插件配置文件。
- `tools_for(refs)` 跳过 `available = false` 的能力，工具目录按插件 id 稳定排序。

## 6. 错误模型

`PluginError { code, message, plugin_id, retryable }`，错误码：

| 码 | 含义 |
|----|------|
| `PLUGIN_ALREADY_MOUNTED` | 重复挂载同一插件 |
| `INVALID_MANIFEST` | 清单校验失败（含工具名非法/重复） |
| `TOOL_NAME_CONFLICT` | 工具名（含历史名）与已挂载插件冲突 |
| `CAPABILITY_CONFLICT` | 能力 ID 重复注册 |
| `HANDLER_MISSING` | 已实现能力缺少处理器 |
| `PLUGIN_NOT_MOUNTED` / `PLUGIN_UNAVAILABLE` | 插件未挂载 / 正在卸载 |
| `TOOL_NOT_FOUND` | 名称无法解析到任何工具 |
| `CAPABILITY_NOT_IMPLEMENTED` | 预留能力被调用 |
| `PLUGIN_TIMEOUT` | 超过超时预算（retryable） |
| `CANCELLED` / `USER_DENIED` / `APPROVAL_EXPIRED` | 运行取消 / 用户拒绝 / 确认超时 |

## 7. 轨迹（Agent Trace）

`tool_started` / `tool_completed` 载荷记录：`pluginId`、`namespace`、`toolName`、`capabilityId`、
`pluginGeneration`、`argsHash`（键排序后的稳定哈希）、`status`。来源信息由
`native_trace::provenance_for(runtime, names)` 生成，前端轨迹行按同一份事件渲染。

## 8. 内置插件清单

| 插件 id | namespace | 默认范围 | 能力（capability） |
|---------|-----------|----------|--------------------|
| `system` | `system` | 是 | `system.get_time` |
| `files` | `files` | 是 | `files.read`、`files.create_text`（原生对话框选择、仅文本、≤128 KiB、不覆盖、拒绝符号链接） |
| `web-search` | `web_search` | 是 | `web.search`、`web.fetch`（保留 SSRF 防护） |
| `markdown` | `markdown` | 否（`@`） | `markdown.convert_to_pdf` |
| `netease` | `netease` | 否（`@`） | `music.player.current_state`、`music.player.control`、`music.player.seek`、`music.player.start_or_connect`（CDP 逻辑保留在 `src-tauri/src/netease/`） |
| `desktop` | `computer` | 否（`@`） | `computer.list_windows`、`computer.inspect_window`、`computer.get_text`、`computer.invoke`、`computer.set_value`；预留 `computer.screenshot`、`computer.ocr`、`computer.send_input` |

前端只读展示这些插件与能力：`list_builtin_plugins`（`src/agent/plugins/usePlugins.ts` →
设置中心 `PluginCatalog`），没有任何启停、排序或权限开关。

## 9. 验收与测试基线

- `pnpm typecheck`、`pnpm build`、`cargo check`、`cargo test`、`git diff --check` 全部通过。
- `cargo test`：lib 单元测试 55 通过 / 3 ignored，集成测试 5 通过。
- 覆盖点：自动挂载、重复挂载拒绝、工具名冲突、能力冲突、预留能力隐藏、`@` 作用域与别名解析、
  能力统一转发、drain/unmount/reload 与 generation、超时、旧名兼容（点号 + 单下划线）、
  轨迹来源字段、文件工具安全策略、desktop 参数校验、netease / markdown 适配器（无宿主降级与参数拒绝）。

## 10. 状态注册契约（命令 `State<T>`）

命令签名是 `State<'_, AgentState>` / `State<'_, Arc<PluginRuntime>>`，`manage` 的注册类型必须一一对应：
类型不一致时编译与普通单元测试都不会报错，只有运行时第一条 IPC 才会返回 "state not managed"。
因此 `lib.rs` 把注册抽成 `register_agent_state`，并用 `state_wiring` 测试（tauri `test` feature 的 mock
runtime，`[dev-dependencies]`）真正调用 `agent_create_session` / `agent_list_sessions` /
`list_builtin_plugins`，锁住这一契约；改坏注册类型该测试会直接失败。

## 11. 已知边界

- Windows 上 `tauri-plugin-dialog`（rfd `common-controls-v6`）静态导入 `TaskDialogIndirect`：
  测试可执行文件需要 Common-Controls v6 清单。主程序与 bins 由 tauri-build 资源提供，
  集成测试由 `build.rs` 的 `rustc-link-arg-tests` 提供，`cargo test --lib` 的单元测试
  由 `lib.rs` 中 `cfg(test)` 的 `.drectve` 段提供（`/MANIFESTDEPENDENCY`）。
- `src-tauri/src/agent/filesystem.rs`、`limits.rs` 是改造前遗留的未接线模块，当前不在运行时路径上。
