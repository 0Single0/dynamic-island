# 桌面 Agent 应用 — 实现计划

| 属性 | 值 |
|------|----|
| 依据规格 | `.cursor/docs/desktop-agent-spec.md` |
| 计划版本 | 0.3-draft |
| 生成日期 | 2026-09-28 |
| 生成说明 | 由 spec-plan 根据规格说明推导；实现时若与规格冲突，以最新规格为准 |
| 目标项目 | `E:\code\dynamic-island` |

## 1. 规格摘要

### 1.1 目标一句话

在现有 Tauri 灵动岛中增加一个通用桌面 Agent：用户可以对话、配置任意模型供应商、通过插件使用文件/网络/应用/窗口/桌面能力，并安全地调用网易云和 Markdown 转 PDF 等项目内置能力。

### 1.2 范围边界

In scope：

- AI 应用卡片、AI 对话页、任务中心。
- 全局设置中心，而不只是模型设置页。
- Provider/Model 配置和三种 API Adapter。
- 自研 Agent Loop、流式事件、取消、重试和权限审批。
- 插件 Registry、内置插件、`/` 动作目录、`@` 插件目录。
- 文件、网络搜索、浏览器、系统、剪贴板、网易云、Markdown、桌面插件。
- 文件 Diff/备份/撤销。
- 上下文压缩。
- UIA 优先、SendInput 兜底的 Computer Use。

Out of scope：

- 多 Agent 协作。
- 语音输入和语音播报。
- 云同步和团队共享。
- 默认开放 Shell 或不受限制的脚本执行。
- 对所有 Windows 应用提供稳定的自动化保证。
- 第一版公开插件市场。

MVP 发布边界：

- 必须完成 AI 对话、全局设置、Provider/Model、Plugin Registry、Slash/Mention 目录、网易云、Markdown、文件读取/受控写入、网络搜索、上下文压缩、权限审批、会话保存和基础诊断。
- 浏览器/CDP、任务中心、后台任务、UIA、SendInput、截图/OCR、本地插件包、MCP、Sidecar 和本地模型允许提前做 Spike，但不作为 MVP 发布阻塞项。

### 1.3 关键交付物

| 类别 | 交付物 |
|------|--------|
| UI | AI 卡片、AI 对话页、输入目录浮层、全局设置中心、插件页、审批卡、任务中心 |
| Agent | Agent Loop、任务状态、取消、重试、流式事件、上下文压缩 |
| 模型 | Chat Completions、Responses、Anthropic Messages 三种适配器 |
| 插件 | Plugin Manifest、Registry、权限、内置 system/files/web-search/browser/desktop/clipboard/netease/markdown |
| 存储 | Provider、Model、Session、Message、ToolCall、Plugin、Permission、Task、Artifact 数据 |
| 桌面 | UIA 检查和控件操作、SendInput、窗口校验、截图兜底 |
| 质量 | Mock Provider、工具调用、权限、压缩、文件 Diff、Computer Use 集成测试 |

## 2. 与规格追溯矩阵

| 规格 ID / 章节 | 实现概要 | 计划任务 |
|----------------|----------|----------|
| §1.2、§5.1 | AI 卡片位于时钟前；应用列表设置按钮 | TASK-01、TASK-02 |
| §5.2 | AI 对话页、模型选择、消息区、工具状态、审批和停止 | TASK-08、TASK-09 |
| §5.3、§5.6 | 全局设置中心和 AI/模型分类 | TASK-02、TASK-06 |
| §5.4、§5.5 | Tiptap 输入框、`/` 动作目录、`@` 插件目录、浮层和 chip | TASK-04 |
| §5.7、§6 | 通用能力范围、插件 Registry、Manifest、插件权限 | TASK-03、TASK-10、TASK-12、TASK-13、TASK-14 |
| §7 | Agent 状态机、Agent Loop、取消、重试、最大步数 | TASK-08 |
| API-01～API-03 | 会话、发送消息、取消运行 | TASK-05、TASK-08 |
| API-04～API-05 | 工具审批、手动上下文压缩 | TASK-09、TASK-18 |
| API-06～API-09 | Provider/Model 配置和测试连接 | TASK-06、TASK-07 |
| API-10～API-13 | Tool/Plugin 查询、启停和权限 | TASK-03、TASK-09 |
| API-14～API-15 | 任务列表和任务恢复 | TASK-19 |
| API-16～API-23 | Run 状态/事件重放、重试、任务暂停取消、预算、会话导出、备用模型、插件测试和 Artifact | TASK-08、TASK-19、TASK-21、TASK-24、TASK-25 |
| §8 | 三种模型 API Adapter 和统一流式事件 | TASK-07 |
| §7.4、§14～§15 | 运行预算、费用、重试、降级、并发锁和事件重放 | TASK-08、TASK-21、TASK-25 |
| §9 | 内置工具、文件工具、网络工具、浏览器工具 | TASK-10～TASK-14 |
| §10 | UIA、SendInput、截图、目标窗口安全策略 | TASK-15～TASK-17 |
| §11 | Context Compaction 和摘要 UI | TASK-18 |
| §12 | SQLite、Store、安全密钥存储、插件/任务表 | TASK-03、TASK-05 |
| §14～§16 | 事件协议、配置开关、普通/高风险调用链 | TASK-08、TASK-09、TASK-19 |
| §17～§19 | NFR、风险、单测/集成/E2E/验收 | TASK-20 |

## 3. 前置条件与阻塞

### 3.1 可直接采用的默认假设

以下内容不阻塞第一阶段开发，可以先按规格中的假设实现：

- AI 卡片名称暂定为 `AI`，从左到右位于时钟之前。
- Enter 发送，Shift+Enter 换行。
- 默认不启用 Computer Use 和 Shell 工具。
- 只允许用户明确选择的文件，或设置中心明确授权的目录。
- 会话默认本地持久化。
- 第一版使用内置插件和手动导入的本地受信任插件，不做公开插件市场。
- 远程供应商使用 HTTPS；localhost 可以使用 HTTP。

### 3.2 需要在 Phase 0 关闭的 GAP

| GAP | 影响 | 处理方式 | 是否阻塞 |
|-----|------|----------|----------|
| GAP-03 默认供应商 | 首次使用引导 | 无默认供应商，先实现手动添加 | 否 |
| GAP-04 API Key 存储 | 生产安全 | Phase 0 选择 Windows Credential Manager 或 DPAPI；未决定前可用抽象接口 | 生产阻塞 |
| GAP-05 Enter 行为 | 输入体验 | 按 Enter 发送、Shift+Enter 换行实现，后续可改快捷键 | 否 |
| GAP-06 会话保存 | 数据模型 | 默认本地保存，提供清理入口 | 否 |
| GAP-07 文件范围 | 文件工具安全 | 先做用户选择路径和授权目录 | 文件写入阻塞 |
| GAP-08 Computer Use 默认状态 | 桌面风险 | 默认关闭 | 否 |
| GAP-09 Shell 工具 | 高风险 | 第一版不实现 Shell | 否 |
| GAP-10 图片输入 | 多模态 | 先保留附件结构，按模型能力决定是否发送 | 否 |
| GAP-11 自定义 Header | 供应商兼容 | 支持非敏感 Header；敏感 Header 必须安全存储 | Provider 发布阻塞 |
| GAP-12 模型发现 | 设置体验 | 支持获取失败后手动添加 | 否 |
| GAP-13 HTTP 策略 | 网络安全 | 远程 HTTPS、localhost HTTP | 网络功能阻塞 |
| GAP-14 插件来源 | 插件安全 | 先只做内置插件；本地插件导入延后 | 否 |
| GAP-15 文件编辑范围 | 文件安全 | 用户选中文件 + 授权目录 | 文件编辑阻塞 |
| GAP-16 搜索供应商 | 网络搜索 | 先定义 `WebSearchProvider` 接口，接入一个实现 | 搜索功能阻塞 |
| GAP-17 后台任务 | 生命周期 | 第一版默认前台任务，关闭窗口停止或暂停 | 否 |

### 3.3 环境和依赖

- Windows + Tauri 2 + React + TypeScript + Rust。
- 复用当前 `src-tauri/src/netease/` 和 `src-tauri/src/markdown.rs`。
- 前端增加 Tiptap 相关依赖。
- 后端需要 HTTP/SSE、SQLite、Windows UIA 和 SendInput 能力。
- API Key 存储需要 Windows 安全存储实现或依赖。
- 网络搜索接口先提供 Mock；接入真实 Provider 时需要用户配置搜索服务凭据或本地代理，缺少凭据不阻塞其他 MVP 能力。

## 4. 阶段划分

### Phase 0 — 决策与 Spike

目标：在正式开发前关闭高风险技术选择，不写大规模业务代码。

| 任务 | 产出 | 负责 |
|------|------|------|
| PLAN-00-01 | Provider Adapter Mock：三种 API 请求/流式响应 fixture | 开发 |
| PLAN-00-02 | SQLite vs Tauri Store 试验；确定会话/插件数据方案 | 开发 |
| PLAN-00-03 | Windows Credential Manager/DPAPI 抽象试验 | 开发 |
| PLAN-00-04 | Tiptap `/`、`@` 浮层交互原型 | 前端 |
| PLAN-00-05 | UIA 读取记事本控件树并执行按钮/输入框操作 | Rust |
| PLAN-00-06 | 网络搜索 Provider 接口和一个真实/Mock 实现 | 后端 |
| PLAN-00-07 | 浏览器/CDP 会话：Target 发现、临时 Profile、调试端口校验、下载上传和生命周期 | Rust |
| PLAN-00-08 | Agent 预算、费用统计、重试、备用模型和并发锁 Spike | Rust |
| PLAN-00-09 | 外部内容不可信边界、提示词注入和 SSRF 测试 | Rust/测试 |
| PLAN-00-10 | 文件编码、原子写入、外部修改检测、Junction/UNC 路径 Spike | Rust |

退出条件：

- 三种 API 的 canonical request/event 映射方案确定。
- Provider、Plugin、Tool、Permission 的 Rust/TypeScript 类型确定。
- API Key 不需要明文落盘。
- Tiptap 浮层能完成搜索、键盘选择、Tab 补全和 chip 删除。
- UIA 能在记事本中完成读取文本和设置文本的最小验证。
- 搜索结果可以保留来源 URL、标题和摘要。
- CDP 只能连接已校验的浏览器/应用 Target，并能正确创建、恢复和销毁会话。
- 单次 Run 的时间、token、费用、工具次数和重试预算能由后端强制终止。
- 网页、文件和剪贴板中的提示词注入不能改变权限或触发高风险工具。
- 文件写入能检测外部修改并在失败时恢复备份。

### Phase 1 — 产品骨架、全局设置与插件 Registry

目标：先让产品有正确的入口、全局设置和插件目录，不接复杂 Agent。

交付：

- AI 卡片和 AI panel 状态。
- 全局设置中心。
- Plugin Manifest、Registry、启停和权限数据结构。
- 插件/动作目录的静态数据渲染。
- Provider/Model 设置页骨架。

### Phase 2 — Provider Adapter 与基础 Agent Loop

目标：完成普通对话、流式输出、工具调用和取消。

交付：

- 三种 API Adapter。
- Provider/Model 保存和连接测试。
- Session/Message/Run 数据。
- Agent Loop 状态机。
- Tauri 事件协议。
- 最小 `system.get_time` 和 `system.get_clipboard` 工具。

### Phase 3 — 内置插件和通用文件能力

目标：让 Agent 能使用当前项目已有能力，并能安全处理文件。

交付：

- `netease` 内置插件。
- `markdown` 内置插件。
- `files` 插件的搜索、读取、Diff、写入、备份、撤销。
- `/` 动作目录和 `@` 插件目录与 Registry 联动。
- 工具审批卡片。

### Phase 4 — 上下文压缩、网络搜索、浏览器和任务中心

目标：从单轮对话升级为可处理复杂任务的通用 Agent。

交付：

- 自动/手动上下文压缩。
- 网络搜索和来源卡片。
- 浏览器插件/CDP 适配。
- 任务计划、任务中心、产物和任务恢复。
- 文件、网页和工具结果引用。

### Phase 5 — Computer Use

目标：在安全边界内实现受控 Windows 桌面操作。

交付：

- UIA 窗口枚举、控件树、查找、读取和控件操作。
- SendInput 键盘/鼠标和前台窗口校验。
- 窗口/元素短期 ID。
- 截图工具和兼容性错误。
- 逐次审批、停止、按键释放和动作日志。

### Phase 6 — 稳定性、打包和扩展

目标：完成正式发布前的安全、性能、兼容性和可维护性工作。

交付：

- 全量单测、集成测试和 E2E。
- 权限审计、API Key 脱敏和日志清理。
- 插件版本校验和本地受信任插件导入；不受信任插件包不在 MVP 中执行。
- Sidecar 接口预留，不默认启用。
- 正式打包和无 Python/Node 环境验证。

## 5. 任务分解

| ID | 描述 | 依赖 | 涉及模块 / 路径 | 验收参照 |
|----|------|------|-----------------|----------|
| TASK-01 | 增加 AI 应用卡片，放在时钟之前；扩展 ActivePanel 为 AI 页面 | 无 | `src/widgets/index.ts`、`src/widgets/ai/`、`src/components/island/Island.tsx`、`src/config/island.ts` | §5.1 |
| TASK-02 | 将设置按钮和页面升级为全局设置中心 | TASK-01 | `src/pages/Settings/`、`src/routes.ts`、`src/App.tsx`、`src/components/island/Island.tsx` | §5.3、§5.6 |
| TASK-03 | 实现 Plugin Manifest、Plugin Registry、启停和权限数据结构 | TASK-02 | `src/agent/plugins/`、`src-tauri/src/agent/plugins/`、`src-tauri/src/lib.rs` | §6.1～§6.4、API-11～API-13 |
| TASK-04 | 使用 Tiptap 实现输入框、Slash 动作目录和 @ 插件目录 | TASK-01、TASK-03 | `src/components/agent/Composer.tsx`、`src/components/agent/CapabilityPopover.tsx`、`package.json` | §5.4、§5.5 |
| TASK-05 | 实现会话、任务、消息和本地存储 | PLAN-00-02 | `src-tauri/src/agent/storage/`、`src-tauri/migrations/`、`src-tauri/src/lib.rs` | §12、API-01 |
| TASK-06 | 实现 ProviderConfig、ModelConfig 和全局 AI 设置页 | TASK-02、TASK-05、PLAN-00-03 | `src/pages/Settings/ai/`、`src/agent/providers.ts`、`src-tauri/src/agent/config.rs` | §5.3、§8、API-06～API-09 |
| TASK-07 | 实现三种 API Adapter、SSE 解析和统一流式事件 | PLAN-00-01、TASK-06 | `src-tauri/src/agent/providers/`、`src-tauri/src/agent/model.rs` | API-06～API-09、§8 |
| TASK-08 | 实现 Agent Loop、任务状态、取消、重试、预算、并发和事件协议 | TASK-05、TASK-07、PLAN-00-08 | `src-tauri/src/agent/loop.rs`、`state.rs`、`events.rs`、`src/agent/` | §7、§7.4、§14、API-02～API-03、API-16～API-19 |
| TASK-09 | 实现 Permission Policy、审批快照、参数哈希、过期处理和 Approval UI | TASK-03、TASK-08 | `src-tauri/src/agent/permissions.rs`、`src/components/agent/ApprovalCard.tsx` | §4、§7、API-04、§18 |
| TASK-10 | 将网易云和 Markdown 转 PDF 接入为内置插件 | TASK-03、TASK-08 | `src-tauri/src/agent/plugins/netease.rs`、`markdown.rs`、`src/widgets/music/`、`src/widgets/MarkdownPdfExpanded.tsx` | §9.2、§23 |
| TASK-11 | 实现文件编码/路径安全、外部修改检测、Diff、原子写入、备份和撤销 | TASK-09、PLAN-00-10 | `src-tauri/src/agent/plugins/files.rs`、`src/components/agent/DiffPreview.tsx` | §6.6、§9.3、§17 |
| TASK-12 | 实现网络搜索插件和来源记录 | PLAN-00-06、TASK-03、TASK-08 | `src-tauri/src/agent/plugins/web_search.rs`、`src/components/agent/SourceCard.tsx` | §5.7、§6.7 |
| TASK-13 | 实现浏览器插件和 CDP 适配 | TASK-12、PLAN-00-07 | `src-tauri/src/agent/plugins/browser.rs`、`src-tauri/src/agent/cdp/` | §6.7、§10 |
| TASK-14 | 实现通用应用、窗口和剪贴板插件 | TASK-09 | `src-tauri/src/agent/plugins/system.rs`、`clipboard.rs`、`window.rs` | §5.7、§9.3 |
| TASK-15 | 实现 UIA Computer Use 基础层 | PLAN-00-05、TASK-09、TASK-14 | `src-tauri/src/computer/`、`Cargo.toml` Windows features | §10.2～§10.3 |
| TASK-16 | 实现 SendInput、窗口前台校验和安全策略 | TASK-15 | `src-tauri/src/computer/input.rs`、`window_guard.rs` | §10.4 |
| TASK-17 | 实现截图工具和兼容性错误展示 | TASK-15 | `src-tauri/src/computer/screenshot.rs`、`src/components/agent/ComputerUseCard.tsx` | §10.2、§10.5 |
| TASK-18 | 实现上下文压缩和摘要 UI | TASK-08 | `src-tauri/src/agent/compaction.rs`、`src/components/agent/CompactionCard.tsx` | §11、API-05 |
| TASK-19 | 实现任务中心、后台任务、暂停/取消、恢复和通知 | TASK-05、TASK-08 | `src/pages/AgentTasks/`、`src-tauri/src/agent/tasks.rs`、通知模块 | §6.8、API-14～API-18 |
| TASK-20 | 集成测试、权限测试和正式打包验证 | TASK-01～TASK-19、TASK-21～TASK-26 | `src-tauri/tests/`、`src/components/agent/__tests__/`、CI/打包脚本 | §17～§19 |

补充治理任务：

| ID | 描述 | 依赖 | 验收参照 |
|----|------|------|----------|
| TASK-21 | 实现 Run 预算、费用统计、重试、备用模型、模型能力矩阵和降级策略 | TASK-07、PLAN-00-08 | §7.4、§8.5、API-19、API-21 |
| TASK-22 | 实现外部内容隔离、提示词注入标记、SSRF 和敏感数据脱敏 | TASK-09、PLAN-00-09 | §4、§6.7、§10.4、§17～§19 |
| TASK-23 | 实现浏览器会话、CDP Target 校验、临时 Profile、下载上传和恢复策略 | TASK-12、PLAN-00-07 | §6.7、§10、API-16 |
| TASK-24 | 实现附件、Artifact、临时文件清理和会话导出/删除 | TASK-05、TASK-19 | §12、API-20、API-23 |
| TASK-25 | 实现 Run Journal、会话/文件/窗口/浏览器锁和崩溃恢复 | TASK-05、TASK-08、TASK-19 | §7.4、§12、API-16～API-18 |
| TASK-26 | 实现诊断页、事件重放、日志导出和安全审计测试 | TASK-08、TASK-19 | §14、§17～§19 |

## 6. 数据与迁移

### 6.1 建议迁移顺序

1. 创建 `agent_plugins`、`agent_plugin_permissions`。
2. 创建 `ai_providers`、`ai_models`。
3. 创建 `ai_sessions`、`ai_messages`、`ai_tool_calls`。
4. 创建 `ai_compactions`、`ai_attachments`。
5. 创建 `agent_tasks`、`agent_artifacts`。
6. 增加索引：`session_id + sequence`、`provider_id`、`task status`、`plugin_id`。

### 6.2 迁移和回滚

- 每个迁移有递增版本号和向前迁移函数。
- 启动时先完成迁移，再初始化 Agent Registry。
- 迁移失败时禁止启动 Agent，保留原数据库并显示恢复提示。
- 数据库 schema 失败不影响网易云和时钟基础功能。
- API Key 不参与数据库回滚；安全存储引用失效时显示“需要重新配置 API Key”。

### 6.3 数据量和清理

- 消息正文和工具结果支持按会话清理。
- 附件和截图默认存放在应用数据目录，可由设置中心清理。
- 日志按保留天数滚动清理。
- 删除会话前需要确认；删除后不自动删除用户原文件。

## 7. 接口实现顺序

建议按以下顺序联调，避免前端先依赖不稳定的模型协议：

1. `plugin_list`、`tool_list`：先让 UI 能展示静态/内置能力。
2. `ai_list_providers`、`ai_upsert_provider`、`ai_upsert_model`：完成配置存储。
3. `ai_test_provider`：使用 Mock Provider 和真实最小请求。
4. `agent_create_session`、`agent_send_message`：先实现只返回文本的 Loop。
5. `agent://run/{runId}` 事件：接入流式文本和状态。
6. `agent_approve_action`：接入审批暂停/恢复。
7. 内置 `system.get_time`、`netease.get_current_state`、`markdown.convert_to_pdf`。
8. 文件、网络搜索和浏览器插件。
9. `agent_compact_session` 和自动压缩。
10. `agent_list_tasks`、`agent_resume_task`。
11. UIA、SendInput 和截图工具。

12. agent_get_run_events、agent_get_usage 和 agent_set_run_limits：接入断线重连和后端预算强制。
13. agent_retry_run、agent_pause_task、agent_cancel_task：接入有限重试、暂停和取消。
14. artifact_list、agent_export_session、agent_delete_session：接入产物和数据清理。

## 8. 测试与验收执行

### 8.1 单测批次

| 批次 | 内容 |
|------|------|
| A | Provider URL 归一化、请求体映射、SSE/流式事件解析 |
| B | Plugin Manifest 校验、Registry、权限合并和能力目录过滤 |
| C | Agent Loop 状态、工具调用、重试、取消、最大步数 |
| D | Context Compaction 触发、摘要结构、恢复未完成工具链 |
| E | 文件路径安全、Diff、备份、撤销 |
| F | UIA 查询条件、elementId 生命周期、SendInput 停止释放 |
| G | Run 预算、费用、备用模型、重试分类和并发锁 |
| H | 外部内容隔离、提示词注入、SSRF、敏感数据脱敏 |
| I | CDP Target 校验、Profile、下载上传、任务恢复 |
| J | Artifact 生命周期、会话导出删除、崩溃恢复 |

### 8.2 集成/E2E 批次

1. Mock Provider 普通回答和流式回答。
2. Mock Provider 返回工具调用，Agent 完成二次请求。
3. OpenAI Chat、Responses、Anthropic Messages 各跑成功/失败/超时/工具调用。
4. `/` 目录搜索、键盘选择、Tab 补全、动作草稿。
5. `@` 插件目录选择、chip 删除、按需工具暴露。
6. 全局设置中心的供应商、插件、权限和数据清理。
7. 网易云、Markdown 转 PDF、文件 Diff/撤销。
8. 网络搜索来源卡片和浏览器安全确认。
9. 上下文压缩前后继续任务。
10. 记事本/计算器/文件管理器的 UIA 和 SendInput。
11. 超出 token、费用、时长和工具次数预算时后端停止 Run。
12. Provider 限流、超时、认证错误和上下文超限按分类处理，不能无限重试。
13. 网页或文件中的恶意指令不能改变权限或触发高风险动作。
14. CDP 只能连接已校验的 Target，不能访问默认禁止的内网地址。
15. 任务崩溃恢复时重新校验文件、窗口、浏览器和 L2/L3 审批。

### 8.3 冒烟清单

- [ ] 应用列表可看到 AI 卡片且位于时钟之前。
- [ ] 点击设置进入全局设置中心。
- [ ] 添加一个供应商和模型并测试连接。
- [ ] AI 可以流式回答普通问题。
- [ ] `/搜索` 可以生成动作草稿。
- [ ] `@文件`、`@网页搜索`、`@网易云` 可以选择并显示 chip。
- [ ] AI 可以查询当前歌曲并控制暂停。
- [ ] AI 可以调用 Markdown 转 PDF。
- [ ] 文件修改显示 Diff，拒绝确认时不写入。
- [ ] 网络搜索结果带来源。
- [ ] 上下文压缩后任务仍能继续。
- [ ] Computer Use 默认关闭，打开后高风险动作需要确认。
- [ ] 停止 Agent 后不再执行工具。
- [ ] 正式安装包不依赖 Python 或 Node。

## 9. 风险与缓解

| 风险 | 关联规格 | 缓解 |
|------|----------|------|
| “通用”范围过大导致第一版不可控 | §1、§5.7 | 先实现插件架构和内置插件，按阶段增加能力 |
| 插件权限过宽 | §6.4、§10.4 | manifest 声明、默认 ask、L3 禁止永久授权 |
| 模型错误修改或删除文件 | §6.6、§9.3 | Diff、备份、原子写入、撤销、确认 |
| 网络页面提示词注入 | §6.7 | 页面内容与系统指令隔离，工具输出标记为不可信来源 |
| 浏览器登录/提交误操作 | §6.7、§10.4 | 登录/提交/购买/上传逐次确认 |
| API 协议差异 | §8 | canonical adapter、Mock fixture、原始错误保留 |
| 上下文压缩丢失任务状态 | §11 | 固定摘要 schema，保留未完成工具链和最近原始轮次 |
| UIA 对自绘应用失效 | §10.5 | CDP、快捷键、截图降级，返回明确兼容性错误 |
| SendInput 被 UIPI 阻断 | §10.3～§10.4 | 前台窗口和权限校验，不自动提权 |
| Agent 后台任务残留 | §6.8、GAP-17 | 第一版默认前台，关闭窗口时停止/暂停 |
| API Key 泄露 | §12、GAP-04 | Windows 安全存储、日志脱敏、前端只显示已配置 |
| 插件供应链风险 | §6.3、GAP-14 | 第一版只内置插件和本地受信任包，校验 manifest |
| Agent 无限循环和费用失控 | §7.4、GAP-18 | 后端强制 token、费用、时间、工具次数和重试预算 |
| 备用模型能力不兼容 | §8.5、GAP-19 | 切换前重新检查工具、图片、上下文和剩余预算能力 |
| SSRF、网页下载和上传越权 | §6.7、GAP-20/21 | 默认阻止内网和元数据地址；上传下载逐次确认并限制目录 |
| 审批参数被模型替换 | §4.1 | 绑定参数哈希、目标和过期时间，执行前二次校验 |
| 任务恢复重复副作用 | §7.4、GAP-23 | Run Journal、资源锁、幂等标记和 L2/L3 重新确认 |
| Artifact 和附件泄露 | §12、API-20/API-23 | MIME/大小/来源记录，日志脱敏，按保留策略清理 |

## 10. 建议 PR / 提交切分

- PR1：AI 应用卡片、全局设置中心骨架、ActivePanel 状态扩展。
- PR2：Plugin Manifest/Registry、内置插件注册和静态能力目录。
- PR3：会话/消息/Provider/Model 持久化和 AI 设置页。
- PR4：三种 Provider Adapter、Mock Provider 和连接测试。
- PR5：Agent Loop、流式事件、取消、重试和普通对话。
- PR6：Tiptap Composer、`/` 动作目录和 `@` 插件目录。
- PR7：权限策略、Approval UI、内置网易云/Markdown 插件。
- PR8：文件搜索、Diff、备份、撤销和剪贴板/系统插件。
- PR9：上下文压缩、任务中心和任务恢复。
- PR10：网络搜索、来源卡片和浏览器/CDP 插件。
- PR11：UIA Computer Use、SendInput、窗口校验和截图。
- PR12：全量测试、权限审计、正式打包和安装包验收。

每个 PR 要求：

- 保持 `pnpm typecheck`、`pnpm build` 和 `cargo check` 通过。
- 新增工具必须有风险等级、Schema、失败结果和最少一个测试。
- 不允许把 API Key、密码、完整敏感输入写入日志。
- UI 改动必须在 Tauri 开发运行中验证，不只依赖浏览器预览。
