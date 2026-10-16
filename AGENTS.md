# 反代号池 Agent Guide

## Project Facts

- 形态：**Web 服务**（单容器部署），不是桌面应用
- Stack: Rust 1.96 + axum 0.8（后端）/ React 19.3 + TypeScript 6.0 + Vite 8 + Tailwind CSS 4.3（面板）
- Package manager: pnpm 10
- 核心库: lucide-react（图标）、recharts（图表）、sonner（toast）、axios（HTTP）
- Application commands:
  - Install: `pnpm install`
  - Development（面板）: `pnpm dev`（:5273，`/api` 代理到 127.0.0.1:8791）
  - Type check: `pnpm typecheck`（等价 `tsc --noEmit`）
  - Build (frontend): `pnpm build`（产物 `dist/`，给 Rust 静态托管）
  - 界面冒烟: `pnpm mount-check <面板地址> [账号] [密码]`（真跑打包产物，抓白屏）
  - Rust check: `cd server && cargo check`
  - Rust test: `cd server && cargo test`（175 个用例）
  - 容器: `docker compose up -d --build`
  - 无本地 Rust 工具链时用 Docker 验证（见 README）

## Architecture

一个 Rust 进程起**两个 HTTP 面**，共享同一份 `SharedGateway` 状态：

| 面 | 默认端口 | 路由 | 鉴权 |
|---|---|---|---|
| 下游面 | 8790 | `/v1/*`、`/health` | `config.api_keys`（空 = 免鉴权） |
| 面板面 | 8791 | `/api/auth/*`、`/api/gateway/*`、静态 `dist` | 会话 Cookie（env 账号密码） |

### 目录职责

```text
src/                      面板前端
  main.tsx                  入口（无 Router：单页，登录态决定显示什么）
  App.tsx                   根组件：登录页 / 号池面板二选一 + AppToaster
  index.css                 Tailwind v4 入口 + .gw-scroll 滚动条
  pages/                    页面，目录入口 index.tsx
    ProxyPool/index.tsx       号池主界面（含 accounts/usage/views 子组件）
    Login/index.tsx           面板登录页
  components/common/        跨页面公共 UI：IconButton、Select、DateRangePicker、AppToaster
  api/gateway/index.ts      面板 API 封装（唯一出口，页面禁止直接 axios/fetch）
  stores/
    panel/usePanelSession.ts  登录态（401 自动回登录页）
    gateway/useGatewayState.ts 号池轮询状态
  utils/
    request.ts              全局 axios 实例 + ApiResponse 解包 + 401 订阅
    toast.ts                toast 包装（禁止直接 import sonner）
  types/api.ts              ApiResponse<T> 信封类型
server/                   Rust 网关（axum）
  src/main.rs               入口：装配 env、起下游面 + 面板面、--healthcheck
  src/secrets.rs            凭据主密钥（GATEWAY_MASTER_KEY 或文件）
  src/panel/
    mod.rs                    面板路由（/api/gateway/*）+ 静态资源 fallback
    auth.rs                   登录、会话 Cookie、鉴权守卫
  src/gateway/
    mod.rs                    setup(dir) → SharedGateway（不再依赖 Tauri）
    commands.rs               面板业务逻辑（纯函数，参数是 &SharedGateway）
    server/                   下游 HTTP：openai / anthropic / responses / sse
    core/                     调度、亲和、模型矩阵、号池
    channels/                 上游适配器：workbuddy / openai_compat / fake
    protocol/                 CIR 中间表示 + 三向编解码
    storage.rs                SQLite（账号 / 用量 / 亲和）
    timeutil.rs               本地时区（读 TZ 环境变量，不依赖 tzdata）
```

## 关键约定

### 面板 API

- 路径 `POST /api/gateway/<name>`，`<name>` = `commands.rs` 里 `gateway_*` 去掉前缀并改成 kebab-case
  （`gateway_list_channels` → `/api/gateway/list-channels`）。
- 请求体字段用**下划线转驼峰**（`channel_id` → `channelId`），与 `commands.rs` 的参数名一一对应。
- 响应统一 `{ code, data, msg }`：`code === 0` 成功。
  **业务失败也返回 HTTP 200**，靠 `msg` 带出上游原文 —— 前端 `errorText()` 直接展示它。
  只有未登录是 401（前端据此回登录页）。

### 环境变量（服务器部署口径）

- `PANEL_PASSWORD` 必填；空则**拒绝启动**（`PANEL_ALLOW_OPEN=1` 是本地调试逃生口）。
- `GATEWAY_MASTER_KEY` 丢了 = 已存凭据全部解不开。
- `TZ` 决定"今天"与每日动作；`timeutil` 内置常用时区表，不需要系统 tzdata。
- `GATEWAY_ALLOW_FAKE=1` 保留假渠道（仅供部署自检链路）。

### 面板静态资源

- `/assets/*` 由独立 `ServeDir` 托管：**miss 就是 404，绝不回落 `index.html`**
  （回落 HTML 会让浏览器按 `text/html` 解析 JS，报 `Unexpected token '<'`）。
- 只给 200 加 `Cache-Control`：`/assets/*` 是 `immutable`，入口是 `no-cache`；
  404 不带缓存头（否则一次"暂时缺失"会被缓存一整年）。
- 改 `full_router` 前先看 `panel::static_tests` 的 4 个回归测试。
- 静态托管只证明"服务器吐了字节"；界面是否真能渲染用 `pnpm mount-check` 验
  （入口包报错、chunk 404、渲染期异常都只表现为白屏，`curl` 看不出来）。

## Frontend Rules

- Tailwind utility class 优先；不新增组件级 `*.css`。
- 所有 HTTP 走 `api/gateway/index.ts` → `utils/request.ts`；禁止页面直接 import axios 或 fetch。
- 失败提示统一经 `utils/toast.ts`。
- 每个源文件保持在 1,000 行以内，超出前按职责拆分。
- 页面用目录入口 `src/pages/<Name>/index.tsx`，页面私有组件放 `src/pages/<Name>/components/`。
- 剪贴板 API 只在安全上下文可用（HTTPS/localhost）：`http://<内网IP>` 下必须降级
  （真链接 / `window.prompt`），不能只写 `navigator.clipboard?.writeText`。

## Rust Rules

- 模块依赖方向：`server → core → channels → transport`；`channels` **不得**引用 `core`。
- `channels` 里不许出现 `windows::` / 平台专有 API（服务器版跑 Linux 容器）。
- 面板业务逻辑写在 `gateway/commands.rs`，参数用 `&SharedGateway`，不要在 `panel/` 里直接摸 storage。

## 构建规则

- `package.json` 的 `packageManager: "pnpm@10.25.0"` **不能删**：Dockerfile 走
  corepack，不钉版本它会拉最新 pnpm，新版可能拒绝旧 lockfile 直接构建失败。
- 改前端依赖后必须 `pnpm install` 更新 `pnpm-lock.yaml`（Docker 用 `--frozen-lockfile`）。
- 运行层刻意不装 apt 包：TLS 根证书走 `webpki-roots`（编进二进制）、
  时区由 `timeutil` 自己解析 `TZ`。别顺手加 `apt-get`，那会让构建依赖 Debian 镜像源。

## Change Checklist

完成改动前确认：HTTP 走 request 层、toast 走包装层、无文件超 1,000 行、
面板新增端点已在 `panel/mod.rs` 注册且前端 `api/gateway/index.ts` 同步。
运行 `pnpm typecheck && pnpm build`、`cd server && cargo test`（无本地工具链用 Docker）。
改了静态托管或前端入口，再跑一次 `pnpm mount-check <地址>` 确认界面没白屏。
