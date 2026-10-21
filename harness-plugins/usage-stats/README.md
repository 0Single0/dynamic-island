# Usage Statistics — 用量统计插件

Web GUI 的全局用量面板：从持久化会话日志汇总 Token 消耗，展示渠道用量饼图、总量、使用趋势、缓存命中与命中率，以及按模型分布。

## 面板内容

| 区域 | 说明 |
|---|---|
| KPI 行 | 总 Token、输入（提示词）、输出、缓存命中、缓存写入、缓存命中率 |
| 使用趋势 | 多系列平滑面积图（总量 / 缓存命中 / 输入 / 输出），悬停有十字准线、圆点与浮层 |
| 渠道用量 | 环形饼图，按 provider（渠道）统计总 Token 占比 |
| 按模型分布 | 按总 Token 排序的模型表，含渠道、占比条、命中率、调用次数 |

顶部的时间范围可切换近 24 小时 / 7 / 30 / 90 天与全部；右上角「刷新」重新读取。

趋势图在 **24 小时**窗口按小时分桶（浮层标题显示「10月20日 10:00」），更长的窗口按日分桶。切到 24 小时即可看到小时级曲线。

## 数据来源

唯一真源是**持久化会话日志**：每次模型调用结算为 `assistant/message`（重试时为 `assistant/attempt`），其中带服务商上报的 `TokenUsage`（`inputTokens`、`outputTokens`、`cacheReadTokens`、`cacheWriteTokens`）。插件折叠这些结算记录：

- 同一 `turn:step` 的重复结算**替换**而非累加（流式先到、最终消息后到的情形只计一次）；
- `llm/retry-started` 结束替换范围，同一 step 的重试是一次独立计费；
- 模型归属取结算消息的 `message.source`，缺失时回落到最近的 `request/header`。

命中率 = 缓存命中读 ÷ 提示词 Token（未缓存输入 + 缓存读 + 缓存写）。

## 结构

```text
harness-plugins/usage-stats/
  package.json        # bundle 清单：dsh.bundle.patch + dsh.client
  cordis.patch.yml    # 插入 usage-stats 行
  index.js            # Host 半：扫描会话日志、按 revision 缓存、注册 RPC 路由
  rpc-route.js        # 与 connection.rpc.call 兼容的 wire 协议（鉴权 + 信封）
  stats.js            # 纯函数折叠：usage 提取、按会话折叠、聚合读模型
  client.js           # Client 半：locale、侧栏图标、主面板组件
  locale/en.json      # 插件清单的展示标题与描述（英文）
  locale/zh.json      # 同上（中文）
  icon.svg            # 插件图标
  tests/              # 四个 Node 测试套件
```

### Host 半

`index.js` 的 `inject` 为 `['sessionPersistence', 'connection', 'webServer']`：

1. 用 `sessionPersistence.list()` 取全部会话快照；`formatStatus === 'migration-required'` 的跳过。
2. 对每个会话按 `revision` 复用缓存，仅对变化的会话调用 `open(id, 'read')` 读取，并裁掉 fork 继承的前缀。
3. 用 `stats.js` 折叠出该会话的用量贡献并缓存。
4. 通过 `webServer.register({ kind: 'prefix', path: '/usage-stats' })` 暴露 RPC，先经 `connection.requestRejection(req)` 做 cookie 与来源校验。

> 为什么不使用 `connection.rpc.handle()`：该方法用 Connection 插件自身 fiber 上的 `webServer` 服务挂载物理路由，其他插件调用时会抛 `cannot get property "webServer" without inject`。因此本插件按同一 wire 格式自建路由（`client-task` → `server-response` 信封），这样既能被浏览器的 `ctx.connection.rpc.call()` 直接调用，也能复用 Connection 的准入逻辑。

单个会话读取失败只记为空贡献，不会让整页报错。

### Client 半

`client.js` 是 `window.__ModuleLoader__.load(...)` 形式的单文件 bundle：

- `ctx.locale.register('usageStats', { en, zh })` 注册文案；
- `ctx.slots.register({ name: 'sidebar.panellist', id: 'usage-stats', order: 20 })` 注册侧栏入口；
- `ctx.slots.register({ name: 'main', key: 'usage-stats' })` 注册主面板；
- 数据经 `ctx.connection.rpc.call('/usage-stats', 'summary', { days, tzOffsetMinutes })` 获取。

样式全部使用宿主主题 token（`--dsw-alias-*`），仅图表系列色为固定数据可视化配色。

### 为什么图表是手写 SVG 而不是 recharts / echarts

动态客户端插件的 bundle 只能 `require('react')`——模块表里没有 recharts/echarts，`fetch`/`import()` 在浏览器半也被封禁。本仓库 `package.json` 里的 recharts 是打包进 Tauri 前端的，插件运行时取不到。

所以趋势图用原生 SVG 复刻了同类图表库的观感：

- **单调三次插值**（d3 `curveMonotoneX` 的切线规则），即 `type="monotone"`。关键性质：相邻两端斜率异号时该点是局部极值，切线强制为 0，所以曲线在峰值处变平而不会冲过去；其余位置切线被两侧割线夹住，因此曲线始终落在数据范围内。**不要改回 Catmull-Rom**——它在「尖峰紧邻零值」处会过冲，曾导致曲线跌穿 0 基线（`tests/curve.test.mjs` 锁住了这个回归，旧实现会过冲 13.33px）；
- 每系列一层半透明渐变面积（`<linearGradient>`，26% → 2% 透明度），总量线最后绘制压在最上层；
- 悬停时画虚线十字准线 + 每系列一个圆点 + 浮层，浮层靠近右边缘时自动翻到左侧；
- 图例在图表下方，横线色块 + 系列名；
- 坐标轴用「好看的刻度」算法选步长（1/2/5 × 10^n），所以是 0 / 100K / 200K 而不是随机小数。

浮层与网格线全部走主题 token，浅色主题下自动是浅底深字。

## 安装与验证

```powershell
# 已在当前 web profile 中安装（link 到本目录）：
#   @local/usage-stats  →  E:\code\dynamic-island\harness-plugins\usage-stats
# 侧栏底部「用量」图标打开面板。
```

测试（无需 Harness 运行时）：

```powershell
cd harness-plugins/usage-stats
node tests/all.mjs
```

五个套件：

- `stats.test.mjs` — 折叠与聚合的纯逻辑（替换、重试、范围过滤、日/小时分桶、空态）；
- `host.test.mjs` — 以桩 Context 驱动 `apply`，覆盖鉴权、信封、缓存复用、revision 失效、坏请求、bucket 默认值，并用真实 HTTP server 跑一遍完整往返；
- `client.test.mjs` — 按页面加载器的格式求值 bundle，校验 slot 注册、RPC 调用形状、中英字典键一致；
- `curve.test.mjs` — 曲线几何回归：尖峰不得跌穿基线、单调斜坡不得越过端点、极值点切线为 0；
- `render.test.mjs` — 用按组件实例保存 hook 状态的轻量渲染器真实渲染面板，覆盖数据态、悬停浮层、移出后清除、小时粒度、空态、失败态，并断言不出现 `NaN`。

## 已知限制

- 只统计**持久化日志里有 usage 的结算**；服务商未上报 usage 的调用不计入。
- 全部范围（`days: 0`）最多渲染最近 120 个自然日，早于此的仍计入总计与分组；小时粒度最多 31 天。
- 面板读取的是磁盘日志，不是实时流；正在进行的 turn 需其结算落盘后刷新才出现。
- 趋势图是日/小时聚合，不支持缩放或框选。
