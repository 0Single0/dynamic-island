# Dynamic Island 桌面端 Agent Guide

## Project Facts

- Stack: Tauri 2 (Rust 1.96) + React 19.3 + TypeScript 6.0 + Vite 8 + Tailwind CSS 4.3
- Package manager: pnpm 11
- 核心库: framer-motion 13（动画）、lucide-react 1.46（图标）、react-router-dom 7（HashRouter）、sonner 2（toast）、axios 1.2（HTTP）
- Application commands:
  - Install: `pnpm install`
  - Development: `pnpm tauri dev`
  - Frontend dev only: `pnpm dev`
  - Type check: `pnpm typecheck`（等价 `tsc --noEmit`）
  - Build (frontend): `pnpm build`
  - Build (desktop bundle): `pnpm tauri build`
  - Rust check: `cd src-tauri && cargo check`
  - Test: NOT CONFIGURED

## Architecture

Tauri 2 桌面应用：`src/` 为 React 前端，`src-tauri/` 为 Rust 后端。窗口是无边框透明置顶的"灵动岛"。

### 目录职责（新增页面/API 必须使用目录入口形式）

```text
src/
  main.tsx              # 入口：HashRouter 挂载
  App.tsx               # 根组件：Routes + AppToaster
  routes.ts             # 路由表（唯一登记处）
  index.css             # Tailwind v4 入口 + 玻璃拟态表面样式
  pages/                # 页面，目录入口 index.tsx + 就近 type.ts / components/
    Island/index.tsx    #   岛体主页面（默认路由 /）
    Settings/index.tsx  #   设置页（/settings）
  components/
    common/             # 跨页面公共 UI：GlassCard、IconButton、AppToaster
    island/             # 灵动岛核心组件（Island.tsx 状态机与动画）
  widgets/              # 小组件注册中心（核心扩展点）
    types.ts            #   WidgetDefinition 契约：compact + expanded 双形态
    index.ts            #   注册表：新增组件只改这里
    clock.tsx           #   时钟组件
    music.tsx           #   网易云同步播放（占位）
  api/                  # 后端/外部接口层，目录入口 index.tsx
    window/index.ts     #   Tauri 窗口命令封装
    netease/index.ts    #   网易云 API（占位）
  utils/
    request.ts          # 全局唯一 Axios 实例与 ApiResponse<T> 解包
    toast.ts            # toast 统一包装（禁止直接 import sonner）
  config/
    island.ts           # 岛体尺寸常量（COMPACT_SIZE / EXPANDED_SIZE / WINDOW_PADDING）
  lib/
    utils.ts            # cn() 与 glassVariants
  types/
    api.ts              # ApiResponse<T> 信封类型
src-tauri/
  src/
    lib.rs              # Tauri Builder：插件注册 + 命令注册
    commands/           # Rust 命令按域分模块（window.rs …）
```

### 小组件接入规范（新增 widget 三步）

1. 在 `src/widgets/<name>.tsx` 实现 `WidgetDefinition`：`compact`（胶囊内紧凑展示）与 `expanded`（展开面板）两个 React 组件，声明 `id/name/icon/order/size`。
2. 在 `src/widgets/index.ts` 注册到 `widgetRegistry`。
3. 需要数据时在 `src/api/<name>/index.ts` 建接口层，禁止在组件内直接写 fetch/axios。

岛体的收起/展开/悬浮动画由 `src/components/island/Island.tsx` 统一驱动（framer-motion + 窗口尺寸同步），widget 只提供内容。

## Frontend Rules

- 使用 Tailwind CSS。玻璃拟态表面只用 `index.css` 里的 `.glass-island`（岛体）与 `.glass-panel`（面板），或 `components/common/GlassCard.tsx`，不要散落写 backdrop-filter。
- 新增或修改页面、组件的 UI 样式必须优先使用 Tailwind utility class；不要为单个页面继续新增 `*.css` 组件样式或 `agent-*` 这类散落的 CSS 类。
- 只有 Tailwind 无法稳定覆盖的浏览器细节（例如 WebView 编辑器伪元素、原生滚动条、第三方编辑器内部节点）才允许保留少量 CSS，并且应放到公共入口样式或第三方适配文件中，不能把普通布局、间距、颜色、尺寸写回组件 CSS。
- 修改既有页面时，顺手将本次触及区域从 CSS class 迁移为 Tailwind class；不要同时维护两套表达同一 UI 的样式系统。
- 使用 `components/common/` 下的公共组件；页面私有组件放 `pages/<page>/components/`。
- 布局用 Flexbox，外层容器 `h-screen w-screen`；岛体窗口本身透明，根元素 `bg-transparent`。
- 每个源文件保持在 1,000 行以内，超出前按职责拆分。
- 路由集中在 `src/routes.ts`；页面用目录入口 `src/pages/<Name>/index.tsx`。

## HTTP and Toasts

- 所有 HTTP 请求走 `utils/request.ts` 的 `request<T>()` 或共享 `axiosInstance`；禁止页面直接 import axios 或用 fetch。
- 响应体统一 `ApiResponse<T> = { code: number; data: T; msg: string }`（`src/types/api.ts`）；`code === 0` 视为成功。
- 失败提示统一经 `utils/toast.ts`；toast 挂载点为 `components/common/AppToaster.tsx`。

## Tauri / Rust Rules

- 前端调用 Rust 命令必须经 `src/api/window/index.ts` 封装，命令实现放 `src-tauri/src/commands/` 对应模块，并在 `lib.rs` 的 `generate_handler!` 注册。
- 新增 Tauri 插件时：`src-tauri/Cargo.toml` 加 crate + `lib.rs` 注册 + `capabilities/default.json` 加权限 + 前端装对应 `@tauri-apps/plugin-*` 包。
- 窗口尺寸变化必须同步调用 `set_window_compact` / `set_window_expanded`，保持顶部居中（`move_to_top_center`）。

## Change Checklist

完成改动前确认：页面是目录入口形式、widget 已注册、HTTP 走 request 层、toast 走包装层、无文件超 1,000 行。运行 `pnpm typecheck`、`pnpm build`（或 `cargo check`）验证。
