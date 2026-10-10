# 岛内「反代号池」应用（工作名 proxy-pool）— 架构设计与实现计划

| 属性 | 值 |
|------|----|
| 依据 | 本文 §3–§9 为架构规格；用户已确认落地形态：**放进本仓库、Rust 写后端（含调度）** |
| 计划版本 | 0.1 |
| 生成日期 | 2026-10-09 |
| 目标项目 | `E:\code\dynamic-island` |
| 参考材料 | `D:\workbuddy2api-hub-main\国内版反代链路与接口文档.md`（MIT 参考实现，Python，仅供逻辑复用） |
| 参考策略 | new-api / one-api / sub2api / LiteLLM / vLLM / SGLang（只借算法与参数，见附录 A） |

## 0. 进度（滚动更新）

| 任务 | 状态 | 说明 |
|---|---|---|
| T-01 依赖与骨架 | ✅ | `src-tauri/src/gateway/` 22 文件 3620 行；`setup` 接入 `lib.rs`；监听 127.0.0.1:8790 起 11 个端口回退；`/health` |
| T-02 CIR + OpenAI 编解码 | ✅ | decode（developer→system、content 块数组、tool_calls、`max_completion_tokens`、reasoning）；SSE 帧编码 + 非流式折叠；usage 缓存字段别名归一 |
| T-04 渠道契约 + 注册中心 + 假渠道 | ✅ | `ChannelAdapter` + `build_registry`（未知适配器跳过并记警告）+ `FakeChannel`（可注入 401/429/5xx） |
| P0 调度切片 | ✅ | 账号池/环形轮询/重试排除/冷却策略（401·429·402·5xx·网络）/在飞槽 + Drop 归还/全局并发闸门/决策日志 |
| T-03 存储与凭据 | ✅ | SQLite（账号/用量/亲和）+ 单写线程批量落库 + master key 进系统凭据库 + AES-256-GCM 凭据信封；重启恢复账号与亲和绑定 |
| T-05 workbuddy 适配器 | ✅ | 登录轮询 / 刷新（串行锁在 `core::refresh`）/ 目录（桌面 UA + 空列表回落 + 清洗合并）/ 三套出站身份 / uid 派生指纹 / 卫生管道（指纹清洗·工具配对自愈·deepseek 思考两段式）/ 错误码表 |
| P1 矩阵 + 亲和 + 打分 | ✅ | 模型×渠道矩阵（实时 ∪ 快照 ∪ 覆盖、能力与请求级过滤）；会话亲和（账号级绑定、显式锚点 + 内容前缀链式指纹、TTL/续期/淘汰/落盘、prefer·strict·off）；选号（优先级分层 + 打分加权随机 + 排除集）；决策日志环形缓冲 |
| 岛内面板 | ✅ | `toolRegistry` 重构（消除 3 处硬编码）+ `proxy-pool` 五视图（账号/渠道/矩阵/调度/用量）+ 21 条 `gateway_*` 命令；登录流程（复制授权链接 → 面板轮询 → 落库进池 → 立即签到） |
| Anthropic 入口（`/v1/messages`、`count_tokens`） | ✅ | 解码（system/tool_use/tool_result/image/document/thinking、`tool_choice.any`→required、服务端工具降级）；有状态 SSE 编码器（message_start → content_block_start/delta/stop → message_delta → message_stop，文本块与工具块显式开合）；非流式折叠；Anthropic 错误信封；`count_tokens` 用本地 CJK 估算 |
| Responses 入口（`/v1/responses`） | ✅ | 解码（`input` 字符串/item 数组、`instructions`→system、`function_call`/`function_call_output`→tool 序列（相邻 call 自动合并）、`max_output_tokens`、`reasoning.effort`、服务端工具剔除、**`previous_response_id` 本地 400**）；有状态 SSE 编码器（`response.created` → `output_item.added/delta/done` → `response.completed`，reasoning/message/function_call 三类 item）；非流式折叠成 `object:"response"`；`finish_reason=length` → `status:"incomplete"` |
| 账号额度展示 | ✅ | 渠道无关的 `QuotaSnapshot`（label/unit/remaining/total/used/details/fetched_at）+ `ChannelAdapter::fetch_quota` 契约 + 账号行"额度"列（悬停看明细与抓取时间）+ 金币按钮手动刷新 + 登录成功后自动抓一次；workbuddy 接国内版计费接口（资源包汇总 → 免费/付费包明细 → 容量汇总，兼容 Precise 变体与 Size/Remain/Used 缺项修正） |
| 任务中心（账号行「任务」列） | ✅ | 任务清单挂在**账号**上（同一渠道不同账号进度不同）；事件族逐字对齐参考实现（canvas / playbook / expert / team / skill / automation / skin / chat / buddy / heartbeat，团队与专家按轮次换 id —— 上游按 `(eventCode, id)` 去重）；接取后**必须重读**清单（未接取时 progress 为 null）；上报 200 ≠ 计分（回读进度才认）；一键执行在 **Rust 后台**跑并逐条推进度表（`gateway_run_tasks`/`gateway_task_progress`），面板 1 秒轮询 —— 切页面/切账号都不丢 |
| 任务领取 | ✅ | 领取走 **Web 域** `/activity/growth/tasks/{code}/claim`；幂等语义实测：已领过 → `code:0 / already_claimed:true / credit:0`，未达标 → 400 `task not completed`。抽成 `claim_one`，`run()` 与行内「领取」共用；「已领取」只认 `accept_status == "claimed"`（早先读 `progress.claimed`/`claim_status` 都不存在，导致领过的显示"可领取"+每轮重复领） |
| 定时器（保活 / 定时签到 / 目录预热 / 探针） | ⬜ | **额度自动刷新已落地**：`gateway/ops.rs` 常驻任务，启动 20s 后每 60s 扫一遍，只刷超过 5 分钟未更新的账号（**不依赖面板打开**）；其余（签到/预热/探针）待做 |
| 额度守卫（保留积分 / 每日 Token / 每日积分 / 单模型） | ⬜ | 需要额度查询接口一起做 |
| 第二渠道（qoder） | ⬜ | 等 qoder 的接入资料；扩展机制与配置已就绪（写适配器 + registry + 配置，核心零改动） |
| 硬化（压测 / 故障演练 / 面板在真机上的视觉验收） | ⬜ | — |

当前验证：`cargo test --lib` 208 通过（gateway 150 条：真 socket 端到端、端口回退、凭据加密往返、矩阵候选与覆盖、亲和续期与降级、冷却策略、卫生管道、错误码翻译、任务字段与事件族、Anthropic 与 Responses 两条协议的事件序列与状态映射）；另有 12 条 `#[ignore]` 的**真实账号探针**（任务/额度/领取/账单，需要显式 `--ignored` 跑）；`cargo check --all-targets` 无 gateway 告警；`pnpm typecheck` / `pnpm build` 通过。

**探针的组织方式（2026-10-10）**：真实账号探针集中在 `gateway/channels/workbuddy/tasks.rs` 底部的 `mod probe`，共用一份 `Fixture`（从应用自己的库里读 wb-cn 账号）—— 早先 6 个探针各抄一份装配样板，光样板就 300 行，把文件顶到 1000 行上限以上。探针**刻意不写断言**：它们的用途是观察上游真实字段（accept_status 取值、claim 幂等返回），断言写死了反而会掩盖上游变化。铁律不变：**枚举值/字段名一律先探针看真的，再写进代码** —— `accept_status` 与 `already_claimed` 都是这么定的。

**新增渠道时额度怎么接**：实现 `ChannelAdapter::fetch_quota` 即可，把上游口径翻译成 `QuotaSnapshot`（`label` 写"积分 / 额度 / 试用期"，`unit` 写"点 / 元 / 天"）。面板只按这两个字段展示，**不认识任何具体渠道** —— 所以 qoder 接额度、kiro 接试用期都不需要动界面。查不到时返回 `Ok(None)`，界面显示"—"，不编造数字。

**实战排障记录（2026-10-09）**：ZCode 接网关报 `Cannot read properties of undefined (reading 'summaryParts')`——不是模型/账号问题，是 **Responses 流缺收尾事件**：AI SDK 要求 reasoning item 带 `reasoning_summary_part.added/.done`（它在 part 上维护 `summaryParts`），message item 带 `output_text.done`，且所有 `output_item.done` 按顺序配对。已照参考实现的协议形态补全并加回归测试锁死事件顺序。教训：**协议适配的对照物是"真实客户端消费的流"，不是文档**——同一个客户端走官方端点正常、走网关崩溃，第一件事就是逐事件 diff 两条流。

**已知偏差（与计划不同但有意为之）**：面板用 Tauri 命令轮询（2s，收起即停）而非事件推送 —— 省掉 Rust 侧为面板维护事件通道；若将来要看板常驻再换事件。

**号池表格的列宽策略（2026-10-10，试错后定型）**：**比例权重（`flex-[权重]`，操作列 `shrink-0` 定宽 168px）**。窗口多宽表格就多宽，**永不溢出**；窄了每列都窄一点（文字截断），宽了每列都宽一点。

试过并否掉的三版，记下来免得再走一遍：
- **按比例伸缩但操作列没定宽**：窄窗口时 5 个 32px 图标被 flex 压成 23px（"操作栏太挤"）。
- **固定宽度 + 表格内横向滚动 + 操作列 sticky 钉右侧**：sticky 本身生效（真 DOM 实测：未滚动时操作列就已被拉到可视右缘），但操作列 176px 钉住后会**盖住它左边那几列** —— 表格 1066px / 可视区 674px 时，"请求中~测试" 全被压在操作列底下。
- **最小宽度（min-w-*）**：硬下限，窗口比它们之和窄时整行照样溢出 —— 账号列被挤出可视区 + 底部多一条横向滚动条（"溢出就不管了？"）。
- 验证方式：真实组件在 1040/1200/1280/1440/1920 五档宽度下量 `scrollWidth == clientWidth`（溢出必须为 0）。
- 账号列宽度按**最长昵称**算（下限 96、上限 160），现在体现为**权重** 0.96..1.60：短昵称不浪费宽度，超长昵称也不会把整行顶变形。
- 凭据只显示天数（"26 天"）；额度进度条**不把比例夹到 1**：剩余超过总量（赠送/加包）时填满 + 百分比带 "+"（如 115%+），否则溢出了也看不见。

工具窗口默认尺寸从 1040×720 调到 1280×780（列多了，给足宽度更舒服；窗口再窄也不会溢出，只是每列更窄）。

## 1. 目标一句话

在灵动岛内新增一个常驻应用：**Rust 侧内嵌一个 OpenAI / Anthropic / Responses 兼容的 HTTP 网关 + 多渠道账号池调度器**，React 侧只做状态可视化与运维操作；对上游是多个订阅渠道（workbuddy 国内版 / 国际版、qoder、kiro …），对下游是一个标准 `http://127.0.0.1:<port>` 端点。**对外只有三个协议入口（Chat Completions / Anthropic Messages / Responses），与调用方是谁无关**：不做客户端识别、不按 UA 分支、不维护客户端白名单。

## 2. 范围边界

In scope：

- `src-tauri/src/gateway/`：内嵌 axum HTTP 服务（`/v1/*`）+ 渠道适配层 + 账号池 + 调度器 + 记账。
- 渠道适配器契约与注册中心；首个渠道 = workbuddy 国内版（同一适配器后续实例化出国际版）。
- 模型 × 渠道能力矩阵、会话亲和（保缓存命中）、统一错误语义与冷却/熔断、并发槽、决策追踪。
- 前端：岛内新应用面板（渠道 / 账号 / 矩阵 / 调度 / 用量五个视图）+ 顶栏状态与开关。
- 与既有设施复用：`agent::secrets::SecretStore`（系统凭据库）、`rusqlite` 存储约定、`netease` 的事件推送范式。

Out of scope：

- 多实例/集群部署、Redis 亲和共享（单进程内存 + SQLite 落盘足够）。
- 对外售卖用的多租户/计费/充值。
- 各渠道的运营自动化之外的玩法（签到、保活、任务等作为渠道可选 hook，不在核心）。
- 开机自启、托盘常驻形态改造（P5 再定）。
- 独立 headless 可执行文件（预留：核心写成库模块，将来加 `src/bin` 即可）。

## 3. 形态与集成点

### 3.1 为什么是「岛内嵌 HTTP 服务」而不是 Tauri command 转发

契约是**协议**，不是客户端：任意 SDK、脚本、别的机器、岛内自己的 agent，只要按三种协议之一说话就能用；网关不识别调用方身份，也不为某个客户端写分支。之所以必须是真 HTTP 服务而不是 Tauri command：Tauri IPC 只有岛内前端能用，而协议要求任何进程都能经 TCP 调用。Tauri 进程本身已跑 tokio 多线程运行时，直接 `tauri::async_runtime::spawn` 一个 axum server 即可，不需要额外进程。

```
任意协议客户端（SDK / CLI / 脚本 / 岛内 agent）──HTTP──┐
                                       ├─► axum (127.0.0.1:8790) ─► 路由调度 ─► 渠道适配 ─► 上游
岛内 React 面板 ──Tauri command/IPC──┘        （同一进程内的状态与服务层）
```

**UI 走 command，不走 HTTP**：面板与网关同进程，直接经 `#[tauri::command]` 读同一份 `SharedGateway` 状态，免去面板会话鉴权、端口未启动、CORS 三个坑；HTTP 面只留给外部客户端（`/v1/*` + `/health`）。将来若要浏览器打开看板，再加一个只读的 `/admin/*`（P5 可选）。

### 3.2 进程与生命周期

- 随岛启动时初始化（`lib.rs` 的 `setup`，与 `netease::setup(app.handle())` 同位置），是否开机即开服务由设置项 `gateway.enabled` 控制（默认开）。
- 隐藏到托盘时服务继续；**退出应用即停止**（这是内嵌形态的固有代价，UI 上要写清楚；P5 可提供 headless bin 或独立小进程兜底）。
- 端口：默认 `127.0.0.1:8790`（**避开参考实现占用的 8788**），被占用时在 `8790..8800` 顺序回退并把实际端口推到 UI；只绑回环，LAN 暴露需要显式开关 + 面板黄条警告（参考实现有 lan 启动脚本，本形态不做默认 LAN）。
- 面板顶栏常显「运行中 · 端口 · 可用账号 x/y」与一键复制 Base URL，避免"没起来却不知道"。

### 3.3 顺带收益

岛内 AI 应用（`agent`）的 provider 已支持 `chat-completions / responses / anthropic-messages` 三种协议。网关起来后，只需在设置里新增一个 provider 指向 `http://127.0.0.1:8790/v1`，岛内 agent 就能直接吃自己的号池——零代码。

## 4. Rust 架构与模块划分

依赖方向严格自上而下，`channels/` 不得引用 `core/`（渠道适配器不认识调度器）。

```
src-tauri/src/gateway/
  mod.rs              # setup(app) -> SharedGateway；启动 axum、事件推送、定时器
  state.rs            # SharedGateway = Arc<GatewayState>；内存运行态与快照
  config.rs           # 配置读写（app_data_dir/gateway/config.json）+ 热更新
  events.rs           # gateway-state 事件节流推送（照 netease 250ms→这里 1s）
  commands.rs         # #[tauri::command] 面（UI 专用，见 §8.3）
  server/
    mod.rs            # Router 组装、监听与端口回退、优雅停机
    openai.rs         # /v1/chat/completions、/v1/completions、/v1/models
    anthropic.rs      # /v1/messages、/v1/messages/count_tokens
    responses.rs      # /v1/responses
    auth.rs           # API Key 校验（Bearer / x-api-key / ?key=）
    sse.rs            # SSE 响应封装（Body::from_stream + 心跳注释帧）
  protocol/
    cir.rs            # CanonicalRequest / CanonicalEvent / Usage
    decode/{openai.rs, anthropic.rs, responses.rs}
    encode/{openai.rs, anthropic.rs, responses.rs, fold.rs}
    hygiene/{mod.rs, steps/*.rs}    # 请求卫生管道（纯函数族，可单测）
  core/
    router/{mod.rs, candidates.rs, affinity.rs, scoring.rs, retry.rs, trace.rs}
    pool/{mod.rs, account.rs, cooldown.rs, breaker.rs, slots.rs, quota.rs}
    matrix/{mod.rs, merge.rs, override.rs}
    errors.rs         # ErrorKind / ErrorScope / GatewayError / IntoResponse
    usage.rs          # 记账（writer task）
  channels/
    mod.rs            # ChannelAdapter trait + registry（★ 新增渠道只改这里）
    openai_compat.rs  # OpenAI 兼容模板基类（声明式四张表）
    workbuddy/{mod.rs, auth.rs, identity.rs, catalog.rs, ops.rs}
    fake.rs           # 测试用假渠道（可编程 429/401/超时/半截流）
  storage/
    mod.rs            # 迁移 + repo；单写线程（mpsc）+ 批量提交
  security/
    fingerprint.rs    # uid 派生设备指纹
    apikeys.rs        # 下游 API Key
    envelope.rs       # 凭据信封加密（master key 进 keyring）
```

**分层职责**

| 层 | 职责 | 禁止 |
|---|---|---|
| `server/` | 鉴权、解析协议 → CIR、编码 CIR → 协议、SSE 传输 | 不碰账号与调度 |
| `protocol/` | 三套协议 ↔ CIR 的双向转换；请求卫生管道 | 不做选择、不发网络 |
| `core/` | 候选推导、亲和、打分、重试编排、冷却/熔断、并发槽、记账、决策追踪 | 不认识具体渠道字段 |
| `channels/` | 登录/刷新/目录/身份头/指纹/出站报文/SSE 解析/错误码翻译 | 不引用 `core/` |
| `storage/` `/security/` | 持久化、密钥、指纹 | 无业务判断 |

## 5. 领域模型（Rust 类型草案）

```rust
pub struct GatewayState {
    pub config:  RwLock<Config>,
    pub pool:    RwLock<AccountPool>,          // 按渠道分片
    pub matrix:  RwLock<Matrix>,               // 模型 × 渠道
    pub affinity: AffinityStore,               // 内部 RwLock<HashMap>
    pub slots:   SlotLimiter,                  // 全局 + 每账号 Semaphore
    pub usage_tx: mpsc::Sender<UsageRecord>,   // 记账 writer
    pub stats:   Stats,                        // 原子计数
}

pub struct Account {
    pub channel_id: ChannelId,                 // 联合主键 = (channel_id, uid)
    pub uid: String,
    pub nickname: String,
    pub enabled: bool,
    pub route: RouteAttrs,                     // priority/weight/max_inflight/proxy_slot
    pub runtime: AccountRuntime,               // 内存态：见下
}

pub struct AccountRuntime {
    pub cooldown_until: AtomicU64,             // 账号级
    pub balance_until: AtomicU64,              // 402 停放
    pub breaker_until: AtomicU64,              // 5xx 熔断
    pub fails: AtomicU32,
    pub soft_streak: AtomicU32,
    pub inflight: AtomicU32,
    pub model_cooldowns: RwLock<HashMap<ModelId, u64>>,
    pub refresh_lock: tokio::sync::Mutex<()>,  // 刷新必须串行（上游轮换 refreshToken）
}

pub enum ErrorKind { AuthExpired, RateLimited, QuotaExhausted, ContentPolicy,
                     Upstream5xx, Network, InvalidRequest, ModelNotFound }
pub enum ErrorScope { Request, Account, AccountModel(ModelId), Channel }
pub struct UpstreamError { pub kind: ErrorKind, pub scope: ErrorScope,
                           pub reset_at: Option<u64>, pub detail: String }
```

关键约束：账号可用性始终是**实时计算值** `is_ready(account, model)`，不是落库状态；`(channel_id, uid)` 是唯一键，同一物理账号接两个渠道算两条账号。

## 6. 调度设计

### 6.1 候选推导（第一层过滤）

```
routes = matrix.routes(public_model)                     // 别名解析 → 候选渠道
  .filter(route.enabled && channel.enabled)
  .filter(satisfies(route.caps, req.requirements))       // vision / tools / 上下文 / max_tokens 上限
若为空 → 本地 400（不发上游、不耗额度），错误里带上"哪些渠道因为哪条约束被排除"
```

矩阵来源三层合并：各渠道实时目录（懒加载 + TTL 300s + 双检锁 + 内置快照回落）∪ 内置快照 ∪ 人工 override（`config.json` 里的 routes 覆盖，支持别名、优先级、上限、临时禁用）。**"glm 只走 workbuddy、ds 走 workbuddy+qoder" 不需要写死在代码里**——qoder 目录里没有 glm，候选自然只有一个；将来 qoder 上了 glm，拉一次目录即双候选。

### 6.2 会话亲和（保缓存命中，与负载均衡的张力显式化）

| 项 | 设计 |
|---|---|
| 绑定粒度 | **账号级**（上层前缀缓存大概率按账号隔离），键 = `(api_key_id, public_model, session_key)` |
| 键推导 | ① `metadata.user_id` 里的 session_id → ② 头 `x-conversation-id` / `session-id` → ③ body `conversation_id` / `prompt_cache_key` → ④ **内容前缀链式 sha256 + 断点回溯 ≤20 位置**（客户端不给锚点时也能粘住）→ ⑤ `api_key_id + ip + ua` 摘要 |
| TTL | 默认 1800s，命中续期；实测上游缓存 TTL 后调整（见 §13 实验 2） |
| 失效阶梯 | 原账号 → 同渠道其他账号 → 其他渠道（每往下一级缓存必丢，代价递增） |
| 稳定性 | 成功后改写绑定到真正成功的账号；重试有排除集，一轮请求内绝不重复选同一账号 |
| 逃逸 | TTFT > 15s / 绑定账号错误率 > 0.5 / 该账号并发槽满 → 允许一次性溢出（`preserve_binding`，不重写），账号恢复后自动回归 |
| 三态 | `prefer`（默认）/ `strict`（绑定不可用直接 503，用于 encrypted reasoning、超长上下文等）/ `off` |
| 观测 | **亲和命中率**与**真实缓存命中率**（`cached_tokens / prompt_tokens`）分开统计 |

### 6.3 选择算法（四步，顺序固定）

1. **亲和短路**：绑定账号仍可用且仍满足候选约束 → 直接用。
2. **候选账号集合**：`(route, account)` 展开，剔除不可用（冷却/停放/熔断/额度守卫/token 临期刷新失败/在飞超限）；全空 → 返回"最早恢复时间"对应的 429/503 + `Retry-After`。
3. **分层**：按 `route.priority` 降序，只取最高层（priority 表达成本档：免费 > 付费；同一模型在不同渠道可给不同 priority）。层内可选 weight 加权。
4. **打分 + 层内加权随机**：`priority_norm + weight_norm + quota_headroom − load + health + affinity_bonus − switch_penalty + cost_bonus`；对分数做平移后**无放回加权抽样**得到完整优先序（同会话重试顺序稳定）。随机种子 = `hash(session_key + model + 时间片)`：会话内稳定、会话间均衡；无会话锚点的纯负载请求混入时间熵。

`switch_penalty` 默认 0.35，与负载惩罚同量纲（参照 vLLM `LOADAWARE_BETA=1.0` 的语义：负载高出均值一倍 ≈ 抵掉一次完整 cache hit 的收益）。

### 6.4 失败分类、冷却与重试

| ErrorKind | Scope | 动作 |
|---|---|---|
| `AuthExpired`(401) | Account | 冷却 60s（池内仅 1 个可用账号时 3s），换号；**不触发刷新** |
| `RateLimited` 模型级 + reset | AccountModel(model) | 只冷该模型到 reset 时刻，同账号其他模型照用 |
| `RateLimited` 模型级无 reset | AccountModel(model) | 只冷该模型：`600s·2^(n-1)` ≤ 7200s（**按模型各自的 streak**），账号不连坐 |
| `RateLimited` 无 reset | Account | 软限流：`600s·2^(n-1)` ≤ 7200s，解绑亲和并换号 |
| `RateLimited` 账号级带 reset | Account | 停整个账号到那个时刻（上游没说哪个模型时的保守处理） |
| `QuotaExhausted`(402) | Account | 停放到次日本地 04:00 |
| `ContentPolicy`(403) | Request | **不换账号**；提示词降级后原号重试一次 |
| `Upstream5xx` | Account | 连续 3 次 → 熔断 `1800s·2^(n-3)` ≤ 21600s |
| `Network` | Account | 短睡指数退避重试（300ms 起），计入降级窗口（连续 5 次 → 600s 窗口） |
| `InvalidRequest`(400/413/422) | Request | 不重试、不改账号状态、直接透传 |
| 任一次成功 | — | 清空**该模型**的冷却与 streak、软限流计数、失败计数、熔断与降级窗口 |

尝试次数 `max(2, 该模型候选可用账号数) + 1`；全池失败按最后一次错误映射（OpenAI：`{"error":{...}}`；Anthropic：`{"type":"error",...}` 且 400/422→invalid_request_error、401→authentication_error、403→permission_error、429→rate_limit_error、503/529→overloaded_error）。

**限流粒度是渠道自己的事（2026-10-10 实现）**：`classify_error(status, body, upstream_model)` 会把**实际发出去的 upstream 模型名**交给渠道，由渠道决定落到哪个 `ErrorScope`；核心调度器不认识任何具体渠道。workbuddy 的落地：

- **6004 = 模型级限额**（参考实现原文 "usage exceeds frequency limit"）。正文会给 `your usage will reset at 2026-09-19 18:29:03 UTC+8`，我们解析成时刻 → 只停这个模型到那一刻；解析不出来也保持模型级（按模型退避），**绝不因为一个模型限额把整个账号拉黑**。裸 429（没有 6004）仍按参考实现当账号级软限流处理。
- **模型级冷却的 key 必须用 upstream 名**：写的时候（`narrow_scope`）和读的时候（`router::pick` / 亲和检查 / `on_success`）都用 `route.upstream_model`。早先写用对外名、读用 upstream 名 —— 结果是"明明记了冷却，调度器却查不到"，账号继续被选中、继续吃 429（这类 bug 不会报错，只会表现成"限流好像没用"）。
- 面板会把它显示出来：账号仍算可用（`is_ready("")`），但状态列的 `?` 与"限流等待"列会给出**模型级**限流（"模型 1" + 悬停看到是哪个模型、还有多久），避免把"某个模型限额"误判成"号废了"。

### 6.5 并发与生命周期（Rust 特有）

- `SlotLimiter`：全局 `Semaphore`（默认 32）+ 每账号 `Semaphore`（默认 3）。用 `OwnedSemaphorePermit` 组一个 **`SlotGuard`，`Drop` 时自动释放** —— 客户端断连导致 future 被 drop 时不会泄漏槽位。
- 上游读取加空闲超时：`tokio::time::timeout(300s, stream.next())`（reqwest 只管连接与首字节，不管流中空闲）。
- payload 过大（> 256KB）的 JSON 解析/改写走 `spawn_blocking`，避免阻塞 reactor。
- 客户端断连：`Body::from_stream` 的 receiver 被 drop → 上游流随之 drop，同时记 `client_aborted` 用量。

### 6.6 决策追踪

每请求一条 `decision`（JSONL + 内存环形缓冲最近 200 条供 UI）：

```
{trace, model, candidates, filtered:{qoder:"max_tokens>200000"},
 affinity:{hit,tier,key_src,age}, chosen:{channel,account,upstream},
 retries:[{account,kind,scope,reset}], usage:{prompt,cached,completion,credit},
 ttft_ms, total_ms}
```

多渠道路由没有这个就只能靠猜。

## 7. 渠道适配器契约

```rust
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    fn id(&self) -> &str;
    fn caps(&self) -> &ChannelCaps;                 // 支持的入口协议 / 是否非流式折叠 / 是否代理槽
    async fn login_start(&self, platform: &str) -> Result<LoginTicket, GatewayError>;
    async fn login_poll(&self, ticket: &LoginTicket) -> Result<LoginPoll, GatewayError>;
    async fn refresh(&self, cred: &Credential) -> Result<Credential, GatewayError>;
    async fn list_models(&self, cred: &Credential) -> Result<Option<Vec<UpstreamModel>>, GatewayError>;
    async fn build_request(&self, ctx: &BuildCtx<'_>, req: &CanonicalRequest) -> Result<UpstreamRequest, GatewayError>;
    fn parse_event(&self, frame: &str) -> Result<Vec<CanonicalEvent>, GatewayError>;
    fn classify_error(&self, status: u16, body: &str) -> UpstreamError;
    // 可选 hook：默认为不支持
    async fn fetch_quota(&self, _: &Credential) -> Result<Option<QuotaSnapshot>, GatewayError> { Ok(None) }
    async fn sign_in(&self, _: &Credential) -> Result<bool, GatewayError> { Ok(false) }
}
```

`OpenAICompatChannel` 模板基类把"OpenAI 兼容协议 + 私有鉴权头 + 私有错误码 + 指纹清洗"固化，子类只填四张表：

```rust
impl OpenAICompatChannel {
    const HOSTS: Hosts = Hosts { chat: "copilot.tencent.com", billing: Some("www.codebuddy.cn") };
    const AUTH: AuthStyle = AuthStyle::BearerUser { user_header: "X-User-Id", domain: "copilot.tencent.com" };
    const ERROR_MAP: &[(i64, ErrorKind)] = &[(11217, Pending), (11133, InvalidRequest),
        (11148, ToolPairBroken), (12403, UaRejected), (6004, RateLimitedModel), (10001, IdempotentOk)];
    const HYGIENE: &[HygieneStep] = &[NormalizeRoles, FingerprintScrub, ToolPairRepair,
        ReasoningBackfill, DeepSeekThinking, ForceStream, StreamOptions];
}
```

**请求卫生管道**（每步是纯函数，可单测、可按渠道开关）：`developer→system` 归一 → 提示词模式（passthrough/custom/append）→ 指纹清洗（第三方客户端特征串、`\r\n` 残片、`cc_*=` KV）→ 工具配对自愈 → deepseek `reasoning_content` 回填 → 思考注入（`thinking` + `reasoning_effort` 两段式）→ 强制 `stream:true` → 补 `stream_options.include_usage`。

**新增渠道 checklist（P4 验收标准 = 只做这 7 步，核心零改动）**：写适配器 → 声明四张表 → 注册进 `channels::registry` → 加渠道配置 → 加账号 → 拉目录进矩阵 → 跑契约测试。

## 8. 前端应用形态

### 8.1 注册方式：先补一个 `toolRegistry`（消除现有硬编码）

现状：`markdown-pdf` 这个"工具"在三处硬编码（`ApplicationLauncher` 的 cards、`panelState.ts` 的联合类型、`Island.tsx` 的标题三元表达式）。新增第二个工具前先把这层提成注册表：

```ts
// src/widgets/tools.ts
export type ToolId = "markdown-pdf" | "proxy-pool";
export const toolRegistry: ToolDefinition[] = [
  { id: "markdown-pdf", name: "MD 转 PDF", icon: MarkdownIcon, size: "lg", component: MarkdownPdfExpanded },
  { id: "proxy-pool",  name: "反代号池", icon: RadioTower,   size: "xl", component: ProxyPoolExpanded },
];
// panelState.ts:  ActivePanel 的 tool 分支改为 { kind: "tool"; id: ToolId }
// Island.tsx:     标题与尺寸改由 getTool(id) 提供；渲染改由 tool.component 提供
```

面板目录（`src/widgets/proxy-pool/`，与 `music/` 同为目录形态）：

```
index.tsx                    # 面板外壳：顶栏状态 + Tab 切换（≤200 行）
components/StatusBar.tsx     # 运行状态 / 端口 / Base URL 复制 / 开关
components/ChannelsView.tsx  # 渠道卡片：适配器/身份/优先级/权重/启停/账号数
components/AccountsView.tsx  # 账号表 + 登录流程（打开 authUrl → 轮询）
components/MatrixView.tsx    # 模型 × 渠道 网格（含覆盖编辑）
components/SchedulingView.tsx# 亲和统计 / 冷却倒计时 / 决策日志 tail
components/UsageView.tsx     # 今日 tokens / credit / 渠道与模型分布
```

尺寸约束：岛体展开面板最大 `xl = 780×480`（窗口 812×512 固定），内容区实际约 `740×420`。因此面板只做**紧凑表格 + 局部滚动**，不做大图与多列卡片墙；`grid` 单元格字号用 `text-[11px]` 级（与既有面板一致）。

### 8.2 状态流（照 `netease` 范式）

- Rust 侧 `events.rs` 以 **1s** 节流推送 `gateway-state` 事件（账号状态、冷却倒计时、在飞数、今日用量、亲和命中率）；状态变更类命令（启停/登录成功/切换渠道）立即补推一次。
- 前端 `src/stores/gateway/useGatewayState.ts`：挂载时 `invoke("gateway_status")` 拉一次全量，之后订阅事件；冷却倒计时在前端用本地时钟插值（照 netease 的 `positionAt` 手法），避免每秒发事件。
- 无 Tauri 环境（浏览器调试）时降级为静态占位，保持 UI 可独立调试（照 netease 的 `EMPTY` 约定）。

### 8.3 命令面（`gateway/commands.rs`，前端 `src/api/gateway/index.ts` 一一对应）

| 分组 | 命令 |
|---|---|
| 运行 | `gateway_status` · `gateway_set_enabled` · `gateway_base_url` |
| 渠道 | `gateway_list_channels` · `gateway_update_channel` |
| 账号 | `gateway_list_accounts` · `gateway_login_start/poll/cancel` · `gateway_set_account_enabled` · `gateway_delete_account` · `gateway_refresh_account` · `gateway_probe_account` · `gateway_checkin_account` |
| 矩阵 | `gateway_list_routes` · `gateway_set_route_override` |
| 调度 | `gateway_affinity_stats` · `gateway_clear_affinity` · `gateway_recent_decisions` |
| 用量 | `gateway_usage_summary` |
| 凭据 | `gateway_list_api_keys` · `gateway_upsert_api_key` · `gateway_delete_api_key` |

命名与前缀遵循仓库既有习惯（`agent_*` / `netease_*`），新前缀 `gateway_*`；窗口类命令保持 `commands/window.rs` 不动；命令全部并进 `lib.rs` 那唯一一个 `generate_handler!`。

**权限**：本方案不新增 Tauri 插件（axum 是普通 crate，出站请求走 Rust 侧 `reqwest`，不碰 JS 的 HTTP 权限）。唯一可能要动 `capabilities/default.json` 的地方是登录流程要在系统浏览器打开 `authUrl`：优先用已有的 opener 插件；若 `opener:default` 未覆盖 `open-url`，补一条 `opener:allow-open-url`（带 https 白名单），或改由 Rust 侧调 `tauri_plugin_opener` 直接打开，避免扩大 JS 权限面。

## 9. 数据、安全与复用

**SQLite（复用 `app_data_dir`，与 agent 同一库或独立 `gateway.db`）**

```sql
channels(id, adapter, display_name, options_json, priority, weight, enabled, created_at)
accounts(channel_id, uid, nickname, enterprise_id, credential_ref, encrypted_cred, expires_at,
         enabled, priority_override, weight_override, proxy_slot, added_at, source,
         PRIMARY KEY(channel_id, uid))
routes_override(public_model, channel_id, upstream_model, priority, weight, limits_json, enabled)
affinity(scope, model, session_key, channel_id, account_uid, upstream_model, expires_at, hits,
         PRIMARY KEY(scope, model, session_key))
usage(ts, model, channel_id, account_uid, api_key_id, stream, status,
      prompt_tokens, completion_tokens, reasoning_tokens, cached_tokens, total_tokens, credit,
      ttft_ms, total_ms, effort, message_hash, system_hash)
cooldowns_snapshot(account_key, scope, model, until, reason)   -- 重启不丢
```

写入走单写线程（`mpsc` → 批量事务，500ms/64 行）；用量另存一份 JSONL 镜像（`<app_data>/gateway/usage/usage.jsonl`）便于外部脚本消费。

**密钥**：单个 master key 存 `agent::secrets::SecretStore`（Windows 凭据管理器），账号 token 用 AES-256-GCM 信封加密后进 SQLite（`security/envelope.rs`，新增 `aes-gcm` 依赖）。理由：token 每次刷新都会轮换，逐条写系统凭据库既慢又污染凭据库；下游 API Key 数量少，可直接进凭据库。日志与面板一律只显示脱敏片段（token 只留 sha1 前 8 位）。

**设备指纹**：`machine_id / session_id / request_id` 由 `(channel_id, uid)` 加盐哈希稳定派生（同账号恒定、跨账号隔离）——这是多账号不被关联风控的关键，参考实现 `wb_fingerprint.py` 的逻辑可直接转写。

**新增依赖**（其余全部已有）：`axum` + `tower-http`（可选，超时/追踪）、`sha2`、`uuid`、`dashmap`（可选）、`aes-gcm`、`urlencoding`。`reqwest(stream)`、`tokio`、`rusqlite`、`serde`、`async-trait`、`base64`、`zeroize`、`keyring` 已在 `Cargo.toml` 中。

**复用已有代码**：`agent::providers::{parse_sse, pop_sse_frame}`（SSE 帧拼接，已处理分片中文）、`agent::secrets::{SecretStore, OsCredentialStore}`、`agent::storage` 的迁移/PRAGMA 约定、`netease` 的事件节流与 `setup(app)` 范式。

## 10. 任务分解

| 任务 | 描述 | 依赖 | 交付物 |
|------|------|------|--------|
| T-01 | 依赖与骨架：加 crate、`gateway/` 目录、`setup(app)` 接入 `lib.rs`、端口回退、`/health` | — | `gateway/{mod.rs,server/mod.rs,config.rs}`、`Cargo.toml` |
| T-02 | CIR 契约 + OpenAI 入口解码 + SSE 编码 + 非流式折叠 | T-01 | `protocol/cir.rs`、`protocol/{decode,encode}/openai.rs`、`server/{openai.rs,sse.rs}` |
| T-03 | 存储层：表结构、单写线程、账号/渠道/用量 repo；秘钥信封 | T-01 | `storage/mod.rs`、`security/envelope.rs` |
| T-04 | `ChannelAdapter` trait + `OpenAICompatChannel` + 注册中心 | T-01 | `channels/{mod.rs,openai_compat.rs}` |
| T-05 | workbuddy 适配器：登录(state 轮询)/刷新(串行锁)/目录(UA 敏感+空列表回落)/身份头/指纹/错误表/卫生管道 | T-02,T-03,T-04 | `channels/workbuddy/*` |
| T-06 | 账号池：可用性判定、冷却/熔断/停放、并发槽 guard、token 临期刷新 | T-05 | `core/pool/*`、`core/errors.rs` |
| T-07 | 矩阵：目录合并 + 能力/请求级过滤 + override | T-05 | `core/matrix/*` |
| T-08 | 调度器：候选 → 亲和 → 分层打分 → 重试编排；决策追踪 | T-06,T-07 | `core/router/*` |
| T-09 | 记账与聚合 + 定时器（保活/签到/目录预热/探针） | T-03,T-06 | `core/usage.rs`、`gateway/state.rs`、`channels/workbuddy/ops.rs` |
| T-10 | 前端注册表重构：`toolRegistry` 取代三处硬编码 | — | `src/widgets/tools.ts`、`panelState.ts`、`Island.tsx`、`ApplicationLauncher.tsx` |
| T-11 | 面板：状态栏 + 渠道/账号视图 + 命令封装 + 状态订阅 | T-09,T-10 | `src/widgets/proxy-pool/*`、`src/api/gateway/*`、`src/stores/gateway/*` |
| T-12 | 矩阵/调度/用量视图（含决策日志 tail 与亲和统计） | T-11 | 同上 |
| T-13 | Anthropic + Responses 两条入口线（含 `count_tokens` 估算与 SSE 事件序列） | T-02,T-08 | `protocol/{decode,encode}/anthropic.rs`、`responses.rs`、`server/{anthropic.rs,responses.rs}` |
| T-14 | 额度守卫（保留积分/每日 Token/每日积分/单模型 Token）+ API Key 管理 | T-09,T-12 | `core/pool/quota.rs`、`security/apikeys.rs` |
| T-15 | 国际版渠道实例化验证（同一适配器第二实例，核心零改动） | T-13 | 渠道配置 + 矩阵条目 |
| T-16 | P4 第二渠道（qoder）：spike + 适配器 + 契约测试 | T-15 | `channels/qoder/*` + fixtures |
| T-17 | 测试：router/pool/affinity 单测、FakeChannel 端到端、真实上游冒烟 | T-08,T-13 | `gateway/**/tests`、`channels/fake.rs` |
| T-18 | 硬化：压测、故障演练、日志脱敏审计、错误文案、README 运维段 | T-17 | 文档 + 修复 |

## 11. 阶段与验收

| 阶段 | 内容 | 验收 |
|---|---|---|
| **P0** | T-01…T-05（骨架 + workbuddy 单账号跑通） | 岛启动后 `curl 127.0.0.1:8790/v1/chat/completions` 流式/非流式各通一次；`/v1/models` 与上游一致；`cargo test` 绿 |
| **P1** | T-06…T-09（号池 + 矩阵 + 亲和调度 + 记账） | wb-cn 与 wb-intl 作为两个渠道同时挂上且**核心零改动**；同会话连续 20 次请求落同一账号（除冷却外 100%）；单账号故障 1 次请求内迁移；渠道整体 429 时流量迁移且不抖动 |
| **P2** | T-10…T-12（前端应用） | 岛内「反代号池」卡片可打开；五视图可用；登录流程能在面板完成；杀掉进程重启后亲和仍在（落盘生效） |
| **P3** | T-13…T-14（多协议 + 守卫） | 用两个真实客户端分别代表 Anthropic 与 Responses 协议各跑通一个真实任务（含工具调用；Claude Code / Codex CLI 仅作测试载体）；`cached_tokens` 与上游一致；守卫触发时账号在选号阶段即被排除 |
| **P4** | T-15…T-16（第二渠道） | `ds` 双渠道按权重分流；`glm` 只有 wb 候选（qoder 在本地被过滤，不发上游）；qoder 429 后自动回 wb 且请求体改写正确 |
| **P5** | T-17…T-18（硬化） | 压测（并发 32 长会话）无泄漏、无槽位悬挂；断网/目录空返回/token 全失效三类故障演练通过；README 运维段齐 |

点数建议：P0≈3–5 天、P1≈4–6 天、P2≈3–4 天、P3≈4–5 天、P4≈3–5 天（取决于 qoder 接入难度）、P5≈3–4 天。

## 12. 测试策略

- **纯逻辑单测**（无需网络）：候选过滤（能力/请求级约束）、亲和键推导与失效阶梯、打分与分层顺序、冷却/熔断状态迁移、错误分类表、卫生管道每一步（含 deepseek 思考两段式、工具配对自愈）。
- **FakeChannel 端到端**：`channels/fake.rs` 可编程返回 200/401/402/403/429/5xx、超时、半截流、乱序 SSE 帧；用它跑"亲和 → 429 → 换号 → 成功 → 绑定改写"整条链路，断言槽位不泄漏、排除集生效。
- **契约测试**：workbuddy 适配器配 fixture 回放（登录轮询的 11217/0 两态、`/v3/config` 的 UA 400、目录空列表回落、429 reset 文本解析）。
- **真实冒烟**（手动，进 `docs` 不作为 CI 门槛）：一个真账号流式 + 工具调用。
- 命令契约：沿用 `lib.rs` 的 `state_wiring` mock-runtime 测试思路，锁死 `SharedGateway` 的 `manage` 类型与命令签名。
- 前端：`pnpm typecheck` + `pnpm build`；面板纯函数（倒计时格式化、状态色映射）进 `tests/gateway-*.test.mjs`（沿用 `node --test` 现状）。
- 门禁：`cargo check`、`cargo test`、`pnpm typecheck`、`pnpm build`。

## 13. 风险与未决问题

**先做两个实验再定参数（各半天）**

1. **上游缓存的隔离粒度**：同账号、同前缀第二发的 `cached_tokens`；换账号同前缀呢？→ 决定亲和绑定粒度与 `switch_penalty` 权重。
2. **上游缓存 TTL**：同前缀间隔 1/5/15/60 分钟重发，看 `cached_tokens` 衰减 → 决定亲和 `ttl_seconds`（默认 1800s 是折中值）。

**风险表**

| 风险 | 影响 | 对策 |
|---|---|---|
| 端口冲突（参考实现占 8788） | 服务起不来 | 默认 8790 + 8790..8800 回退 + UI 显示实际端口 |
| 退出应用 = 网关停 | 客户端断流 | 托盘隐藏仍服务；UI 明示；P5 可加 headless bin/自启 |
| qoder/kiro 接入机制未知（可能有签名/客户端校验） | P4 工期与可行性 | 先 spike；不可复制就推迟渠道，架构不动 |
| 多账号同 IP 被关联风控 | 账号批量失效 | 稳定指纹 + 代理槽 + 单账号并发 ≤3 + 熔断 |
| 上游静默变更（目录/错误码/字段） | 突然全红 | 契约 fixture + 探针 + 目录快照回落 + 未知错误码语义兜底 |
| 亲和导致负载不均（老会话钉住少数账号） | 免费额度提前耗尽 | 逃逸 + 并发溢出（不重写绑定）+ 新会话加权分散 + 面板暴露单账号负载 |
| 岛窗口固定 780×480 | UI 拥挤 | 只做紧凑表格 + 局部滚动；不再加宽面板 |
| reqwest 无流空闲超时 | 半死连接挂住槽位 | `timeout(300s, stream.next())` + `SlotGuard` Drop 释放 |
| 合规/条款风险 | 封号 | 稳定指纹、不放大频率、保留余额保护、不上量对外售卖 |

**未决（需要你拍板）**

- 应用名与图标（暂用「反代号池」+ `RadioTower` 图标）。
- 默认是否随岛自动启动服务（建议是）。
- 是否需要 LAN 暴露开关（默认不做）。
- 用量与决策日志保留策略（建议 30 天滚动）。

## 附录 A · 默认参数表（对照开源实现）

| 项 | 本设计默认 | 参考来源 |
|---|---|---|
| 监听 | `127.0.0.1:8790`（回退 8790..8800） | 参考实现 8788 已被占用 |
| 模型目录缓存 | TTL 300s，懒加载 + 双检锁 + 失败回落 | 参考实现 |
| 亲和 TTL / 上限 | 1800s / 20000 条（可配 300–7200） | new-api 3600s·100k；LiteLLM prompt pin 300s；sub2api sticky 3600s |
| 亲和键 | 显式会话字段 → 内容前缀链式 hash（回溯 ≤20 位置）→ 兜底摘要 | new-api、LiteLLM、sub2api |
| 亲和三态 / 逃逸 / 成功后改写 | prefer / TTFT 15s·错误率 0.5 / 开 | new-api、sub2api |
| 重试排除 | 账号级 no-repeat 集合，贯穿亲和层 | sub2api `excludedIDs` |
| 最大尝试次数 | `max(2, 候选可用账号数) + 1` | 参考实现 2.5 |
| 401 / 429 / 402 / 5xx | 60s（唯一账号 3s）/ reset 或 `600·2^(n-1)`≤7200s / 次日 04:00 / 连续 3 次 `1800·2^(n-3)`≤21600s | 参考实现 2.5；不采用 LiteLLM 的 5s 激进冷却 |
| 网络降级 | 连续 5 次 → 600s 窗口 | 参考实现 2.5 |
| 并发 | 全局 32 / 每账号 3 / 排队 30s → 503 | 参考实现 4.1 |
| 选择算法 | priority 分层 → 层内加权随机 + 打分（含 switch_penalty 0.35） | new-api、sub2api、vLLM `LOADAWARE_BETA` |
| 上游流式 | 强制 `stream:true` + `stream_options.include_usage` | 参考实现 7.1 |
| 空闲超时 | 首字节 120s / 流中空闲 300s | 参考实现 4.3 |

**License**：参考实现 MIT，转写 Rust 需在 `NOTICE` 保留声明；**new-api 是 AGPL-3.0、sub2api 是 LGPL-3.0，只借算法与参数，不复制代码**；LiteLLM（MIT，`enterprise/` 另授权）、vLLM/SGLang（Apache-2.0）同理。

## 附录 B · 参考实现 → Rust 落点

| 参考实现（Python） | 本仓库落点 | 方式 |
|---|---|---|
| `wb_accounts.py` 登录/刷新/出站头 | `channels/workbuddy/{auth.rs,identity.rs}` | 逻辑转写（协议部分） |
| `wb_catalog.py` + 内置快照 | `channels/workbuddy/catalog.rs` | 解析/过滤/合并规则照搬为 Rust |
| `wb_fingerprint.py` | `security/fingerprint.rs` | 通用化为 `(channel, uid)` 派生 |
| `wb_prompt.py` 提示词模式 | `protocol/hygiene/steps/prompt_mode.rs` | 转写为管道步骤 |
| `wb_prox` 转发/SSE 清洗/折叠 | `protocol/*` + `channels/workbuddy/adapter.rs` | **参考逻辑重写**（要接 CIR，不能直接照搬） |
| `wb_atrest.py` 凭据解密 | `security/envelope.rs` | 若沿用其桌面端信封格式则需对齐算法 |
| `wb_tasks/scheduler/probes` | `channels/workbuddy/ops.rs` + `gateway/state.rs` 定时器 | 转写为可选 hook |
| `wb_settings.py` 面板鉴权 | `security/apikeys.rs` + `config.rs` | 改为 API Key + 设置模型 |
| `dashboard.html` 信息架构 | `src/widgets/proxy-pool/*` | 参考信息架构，UI 用 Tailwind 重做 |
