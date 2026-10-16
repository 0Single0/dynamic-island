# 反代号池（Gateway Panel）

把上游 AI 服务的多个账号聚合成一个统一入口的**反代网关**，带 Web 管理面板，单容器部署。

- **下游面**（默认 `:8790`）：`/v1/*`，兼容 OpenAI / Anthropic / Responses 三种协议，客户端只要改 `base_url`。
- **面板面**（默认 `:8791`）：`/api/*` + 静态管理界面，账号池、请求日志、用量、渠道设置都在这里。

一个进程、两个端口、共享同一份状态。

## 快速开始（Docker）

```bash
cp .env.example .env
# 必改：PANEL_PASSWORD；建议同时填 PANEL_SECRET 与 PUBLIC_BASE_URL
docker compose up -d --build
```

打开 `http://<服务器>:8791`，用 `.env` 里的账号密码登录。

客户端这样接：

```bash
export OPENAI_BASE_URL=http://<服务器>:8790/v1
export OPENAI_API_KEY=<面板「设置」里配置的 API Key>
```

> 面板「设置」里默认没有 API Key（`auth: open`）。**暴露到公网前务必先加一个 API Key**，
> 否则任何能访问 8790 的人都能白用你的账号池。

## 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `PANEL_USERNAME` | `admin` | 面板登录账号 |
| `PANEL_PASSWORD` | — | **必填**。留空则容器拒绝启动（`PANEL_ALLOW_OPEN=1` 可显式开放，仅限本地调试） |
| `PANEL_SECRET` | 随机 | 会话签名盐。固定它，重启后已登录的浏览器才不会掉线 |
| `GATEWAY_MASTER_KEY` | 自动生成 | 32 字节 base64，凭据主密钥。**丢了已存的账号凭据就解不开** |
| `PUBLIC_BASE_URL` | 空 | 客户端该指向的对外地址，面板会显示它 |
| `GATEWAY_PORT` / `GATEWAY_BIND` | `8790` / `0.0.0.0` | 下游监听 |
| `PANEL_PORT` / `PANEL_BIND` | `8791` / `0.0.0.0` | 面板监听 |
| `GATEWAY_DATA_DIR` | `/data` | 配置、SQLite、主密钥所在目录 |
| `TZ` | `UTC` | 本地时区。签到/每日动作按**本地日期**判定，容器里务必设对 |
| `GATEWAY_ALLOW_FAKE` | 空 | `1` = 保留自检用的假渠道（无凭据、不碰网络）。**仅供部署自检，别长期开** |
| `CARGO_MIRROR` | `rsproxy` | 仅构建时用；官方源可留空 |

数据全在 `./data`（compose 已挂载）：`config.json`、`state.db`、`secrets/`。**备份这个目录就是备份全部状态。**

## 账号登录是怎么完成的

添加账号不需要在服务器上开浏览器，也不需要回调地址：

1. 面板点「添加账号」→ 选渠道，服务端向上游申请一个 `state` 并返回授权链接；
2. 你在**任意一台能上网的机器**的浏览器里打开这个链接，完成登录；
3. 服务端每 2.5 秒轮询一次上游，拿到凭据后自动入库、签到、抓额度。

面板里那个授权链接是可直接点击的真链接（不是"复制到剪贴板"）——
部署在 `http://<内网 IP>` 时浏览器没有剪贴板 API，复制按钮会退化成弹出原文让你手选。

## 本地开发

```bash
pnpm install
pnpm dev            # 面板前端，:5273，/api 自动代理到 127.0.0.1:8791
```

后端（需要 Rust 工具链）：

```bash
cd server
GATEWAY_DATA_DIR=../data PANEL_PASSWORD=dev cargo run
```

没装 Rust 也可以只靠 Docker 验证：

```bash
docker run --rm -v "$PWD/server:/src" -w /src rust:1.96-bookworm \
  -v gateway-cargo-registry:/usr/local/cargo/registry cargo test
```

## 目录职责

```text
src/                     面板前端（React 19 + Tailwind 4）
  pages/ProxyPool/         号池主界面（目录入口 index.tsx）
  pages/Login/             面板登录页
  components/common/       跨页面公共 UI
  api/gateway/             面板 API 封装（全部走 utils/request.ts）
  stores/                  panel 会话 / gateway 轮询状态
  utils/                   request（含 401 回登录）/ toast
server/                  Rust 网关（axum）
  src/main.rs              入口：起下游面 + 面板面
  src/panel/               面板路由与登录鉴权
  src/secrets.rs           凭据主密钥（env 或文件）
  src/gateway/             网关本体
    server/                  下游 HTTP（/v1/*，三种协议）
    core/                    调度、亲和、矩阵、号池
    channels/                各上游渠道适配（workbuddy / openai_compat / fake）
    protocol/                CIR 中间表示与三向编解码
    storage.rs               SQLite（账号 / 用量 / 亲和）
```

## 验证

```bash
cd server && cargo test      # 175 个用例：协议编解码、调度、存储、鉴权、静态资源
pnpm typecheck && pnpm build # 面板类型检查与打包
```

部署完想确认真的通了，跑自检脚本（只读探测 + 一次登录，不改配置）：

```bash
sh scripts/selfcheck.sh http://<服务器>:8791 http://<服务器>:8790 <账号> <密码>
```

它会检查静态资源、未登录拦截、登录、管理 API、**下游是否还是 `auth: open`**、
以及 `/v1/models` 是否可达。`auth: open` 会明确报红 —— 那是公网部署最危险的状态。

接口全通**不等于界面能用**：入口包执行报错、chunk 加载失败、渲染期抛异常，
都表现为白屏，而 `curl` 只看得到 200。所以另有一个界面冒烟测试，
它用 jsdom + Node 的 ESM 加载器**真的把线上打包产物跑一遍**：

```bash
pnpm mount-check http://<服务器>:8791                # 验未登录时渲染出登录页
pnpm mount-check http://<服务器>:8791 <账号> <密码>   # 验登录后号池界面渲染
```

它抓的正是"服务器 200 但页面白屏"这一类问题，报错会直指
`加载 /assets/index-xxx.js 失败：HTTP 404` 这种真正的原因。

想连 `/v1/*` 的转发链路一起验（没有真实账号时）：给容器加 `GATEWAY_ALLOW_FAKE=1`，
配置里放一个 `{"id":"fake","adapter":"fake"}` 渠道，然后：

```bash
curl -X POST http://<服务器>:8790/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"fake-model","messages":[{"role":"user","content":"hi"}]}'
```

## 静态资源的两个坑（已处理，改这块前先读）

`index.html` 里的资源名带内容哈希，所以：

1. **`/assets/*` 找不到必须 404，不能回落 `index.html`。**
   回落到 HTML 会让浏览器拿 `text/html` 去解析 JS，报 `Unexpected token '<'` ——
   一个和真实原因（资源已被新版本替换）毫无关系的错。旧版 `index.html` 被缓存时尤其常见。
2. **404 不能带 `Cache-Control`。** 部署瞬间 `assets/` 可能还没铺好，
   若这次 404 被 `immutable` 记上一整年，就成了一年的白屏。

入口文件用 `no-cache`（每次回源校验），`/assets/*` 用
`public, max-age=31536000, immutable`。这两条都有回归测试兜着。

## pnpm 版本必须钉住

`package.json` 里的 `packageManager: "pnpm@10.25.0"` **不能删**。

Dockerfile 用 `corepack enable && pnpm install --frozen-lockfile` 装前端依赖，
而 corepack 默认会去拉**最新版** pnpm。踩过这个坑：pnpm 12 拿 pnpm 10 写的
lockfile 直接拒绝安装（`ERR_PNPM_MINIMUM_RELEASE_AGE_VIOLATION`），
构建在 `pnpm install` 这一步就挂了。钉住版本后 corepack 会严格用 10.25.0。

## Roadmap

- [ ] 面板支持修改渠道 `options`（出站身份等目前只能改 `config.json`）
- [ ] 用量页导出 CSV
- [ ] 多用户面板账号（当前是单账号 + 会话 Cookie）
