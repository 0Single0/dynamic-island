# 内置插件自动注册与 Agent 能力运行时改造 — 实现计划

| 属性 | 值 |
|------|----|
| 依据规格 | `.cursor/docs/plugin-runtime-spec.md`（本计划配套规范文档） |
| 计划版本 | 0.1 |
| 生成日期 | 2026-10-08 |
| 生成说明 | 由用户确认的改造方案推导；实现与规格冲突时以规格为准 |
| 目标项目 | `E:\code\dynamic-island` |

## 1. 目标一句话

把「每个应用一套 Agent 执行分支」改为「宿主统一注册 capability、插件清单自动挂载、按请求解析工具、统一经 PluginRuntime 调用」。

## 2. 范围边界

In scope：

- Capability Registry、Plugin Manifest、Plugin Runtime、Tool Resolver 四层契约。
- system / files / web-search / markdown / netease / desktop 六个内置插件迁移到统一注册。
- 模型工具名统一 `<namespace>__<tool>`，保留点号与单下划线旧名解析兼容。
- 启动时自动 mount 全部内置插件，`@` 只影响当前请求的工具范围。
- 统一结构化插件错误、generation、drain/unmount/reload、调用超时。
- Agent Trace 记录插件 ID、工具名、capability ID、参数哈希与结果状态。

Out of scope：

- 插件启停/组合/Profile 等用户配置页面（明确不做）。
- DLL 级动态加载、外部应用 Bridge 传输实现（只预留接口与结构）。
- 截图、OCR、SendInput 能力实现（只预留 capability ID）。

## 3. 前置条件与阻塞

无 GAP。用户已确认：不做插件配置页面、全部内置插件启动即挂载、`@` 仅作当前请求选择。

## 4. 任务分解

| 任务 | 描述 | 依赖 | 交付物 |
|------|------|------|--------|
| T-01 | 冻结契约：`CapabilityRegistry`、`CapabilityHandler`、`PluginManifest`、`ToolExposure`、`ToolRef`、`PluginRuntime`、`PluginToolCall`、`PluginError` | — | `src-tauri/src/agent/plugins/{manifest.rs,registry.rs,runtime.rs}` |
| T-02 | 迁移 system / files / web-search / markdown，执行器改为 runtime 转发 | T-01 | `plugins/{system.rs,files.rs,web_search.rs,markdown.rs}`、`builtin_executor.rs`、`tools.rs` |
| T-03 | 接入网易云 NeteasePlugin（CDP 逻辑保留在 `src-tauri/src/netease/`） | T-01 | `plugins/netease.rs` |
| T-04 | 新增 DesktopPlugin 统一电脑能力，预留截图/OCR/输入 capability | T-01 | `plugins/desktop.rs` |
| T-05 | 启动自动挂载全部插件；删除应用专用 `match` 分支与临时审批分支 | T-02..T-04 | `lib.rs`、`agent/mod.rs`、`plugins/mod.rs` |
| T-06 | 工具名兼容层：双下划线新名 + 旧名 alias，trace 记录插件来源 | T-02 | `adapter.rs`、`stream.rs`、`native_trace.rs`、`tools.rs` |
| T-07 | 前端：插件目录来自运行时清单，`@` 只生成 `pluginRefs`；设置中心只读能力列表 | T-05 | `src/agent/plugins/types.ts`、`src/agent/api.ts`、`src/widgets/ai/AIExpanded.tsx`、`src/pages/Settings/index.tsx` |
| T-08 | 测试与文档：挂载/冲突/`@` 范围/转发/drain/超时/旧名兼容 | T-01..T-07 | `agent/tests.rs`、`plugins/runtime.rs` 内测试、`.cursor/docs/plugin-runtime-spec.md` |

## 5. 验收与回归

- `pnpm typecheck`、`pnpm build`、`cargo check`、`cargo test`、`git diff --check`。
- 新增测试覆盖：自动 mount、重复 mount 拒绝、工具名冲突、`@` 工具范围、capability 统一转发、drain/unmount/reload、不可用与超时、旧工具名兼容、trace 插件来源字段。

## 6. 默认约定

- 新工具名 `<namespace>__<tool>`；旧 `a.b` 与 `a_b` 名称继续可解析。
- 无 `@` 默认暴露 system / files / web-search。
- 插件始终可用，不提供用户启停。
- Agent Core 不保存应用专用业务逻辑；业务实现只保留一份。
