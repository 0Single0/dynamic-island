# Agent 本地存储方案 Spike

状态：已决策（2026-09-29）
关联计划：`PLAN-00-02`
关联规格：`desktop-agent-spec.md` §12

## 决策

采用分层存储，但 Agent 业务数据只有一个权威数据源：

1. **SQLite 作为 Agent 持久化主库**。Rust 侧直接使用 SQLx（SQLite driver），通过 Tauri commands 暴露受校验的领域操作；前端不获得任意 SQL 执行入口。
2. **Tauri Store 仅保存非敏感、纯展示层偏好**，例如面板是否折叠、最近选择的设置子页。它不存 Provider/Model 的权威配置、插件权限、会话状态或任何凭据。
3. **凭据独立安全存储**。API Key 和敏感自定义 Header 不写入 SQLite、Store、日志或导出文件；Provider 行只保存 `secret_ref`。凭据后端由 `PLAN-00-03` 决定。

## 方案比较

| 方案 | 适合的数据 | 优点 | 不适合作为 Agent 主存储的原因 |
|------|------------|------|------------------------------|
| Tauri Store | 少量、低风险键值偏好 | 集成简单；可从 WebView 或 Rust 异步读写；可跨重启保存 | 会话、消息、工具调用、Run Journal、插件权限是有关联且会持续增长的数据；用 JSON 键值容器实现过滤、关系约束、事务和恢复会变复杂 |
| SQLite | Provider、Model、Session、Message、ToolCall、Run、Plugin、Permission、Task、Artifact 元数据 | 关系查询、索引、约束和事务适合领域数据；单机无需数据库服务；可用有版本的迁移演进 | 需要维护迁移、数据库生命周期、备份和锁/事务策略 |
| WebView 直接调用 Tauri SQL 插件 | 前端主导的普通数据 CRUD | 官方插件以 WebView SQL API 为入口，支持 SQLite 并提供迁移机制 | 本产品将权限与 Agent 执行集中在 Rust；不应让 UI 直接获得任意 SQL 写入能力，领域规则应留在 Rust storage/service 层 |

## 数据归属

| 数据 | 位置 | 规则 |
|------|------|------|
| Agent 会话、消息、工具调用和压缩摘要 | SQLite | 事务化追加，按会话/时间查询 |
| Run、预算、事件序号、资源锁和恢复信息 | SQLite | 运行状态变化和 Journal 记录保持一致；恢复后重新校验外部资源及高风险审批 |
| Provider/Model（不含秘密）、插件配置、启停和权限 | SQLite | 由 Rust command 校验后读写 |
| Task、Artifact 元数据、附件索引 | SQLite | 文件内容仍按 Artifact 生命周期保存；清理元数据不得误删用户原文件 |
| 外观和纯 UI 导航偏好 | Tauri Store | 可丢失后恢复默认；不作为 Agent 安全判断依据 |
| API Key、敏感 Header | `PLAN-00-03` 的安全凭据后端 | SQLite 只保存不透明引用；日志/诊断不得输出秘密 |

## 实施约束（供 TASK-05）

- 数据库放在 Tauri `app_data_dir` 下，文件名固定为 `agent.sqlite3`；路径由 Rust 获取，禁止由前端传入任意数据库路径。
- Rust 提供单一 storage/service 边界；业务模块不能自行拼接数据库路径或绕过权限规则写表。
- 用 SQLx 的 SQLite 支持，在运行时连接（不依赖构建机器上存在数据库）；迁移放入 `src-tauri/migrations/`，按递增版本管理，并在应用启动阶段完成后才开放 Agent commands。
- 使用参数化查询；对会话删除、消息/工具调用写入、Run 状态与事件日志等需要原子性的操作显式使用事务。
- SQLite 调用保持异步，不在 Tokio 核心线程执行长时间同步 I/O；连接池大小按单用户桌面负载保持保守，并启用外键约束。写入繁忙时使用有限等待/明确错误，不无界重试。
- 数据库迁移失败时不得启动半初始化的 Agent；返回可诊断错误并保留原数据库。迁移需有从空库初始化、重复启动和失败回滚测试。
- 数据导出/删除、附件清理和数据库备份由后续对应任务实现；备份期间需保证一致性，不将密钥纳入导出。
- Tauri Store 若在后续启用，只允许保存无权限含义的 UI 值，并使用最小 capability 权限；不能把 Store 的默认宽权限当作安全边界。

## 验收结论

- 方案能覆盖规格 §12 的当前表清单和独立 Run/资源锁要求。
- SQLite 是会话/插件/任务数据的唯一权威来源；Tauri Store 不与 SQLite 重复保存同一业务字段。
- API Key 存储方案明确留给 `PLAN-00-03`，不因本 Spike 被误认为已解决。
- 当前仅完成架构 Spike；不新增 SQLx、Tauri Store 或迁移依赖，正式实现由 `TASK-05` 完成。

## 官方参考

- [Tauri Store 插件](https://v2.tauri.app/plugin/store/)：持久化键值存储，可从 WebView 或 Rust 异步访问。
- [Tauri SQL 插件](https://v2.tauri.app/plugin/sql/)：向前端提供 SQL 数据库接口，基于 SQLx，支持 SQLite 等驱动及迁移。
- [SQLx 文档](https://docs.rs/sqlx/latest/sqlx/)：Rust 异步 SQL 工具包及 SQLite driver 文档。
