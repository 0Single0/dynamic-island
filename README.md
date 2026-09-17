# Dynamic Island

桌面端「灵动岛」小组件应用 —— Tauri 2 + React 19 + Tailwind CSS 4 + Rust。

屏幕顶部居中一条黑色玻璃拟态胶囊（类似 iPhone 灵动岛），鼠标悬浮时轻微放大预览，点击后弹性展开为完整面板。内部采用小组件注册中心架构，首个规划组件为网易云同步播放。

## 快速开始

```bash
pnpm install
pnpm tauri dev     # 桌面端开发（Rust + 前端热更）
pnpm tauri build   # 打包安装程序
```

仅前端开发（浏览器调试，无 Tauri 窗口）：

```bash
pnpm dev
```

## 交互

- 收起：顶部居中胶囊，显示各组件紧凑信息（时间、播放状态…）
- 悬浮：120ms 延迟后微放大（1.06x）预览
- 点击：弹簧动画展开为完整面板，系统窗口尺寸同步变化
- 面板右上角：收起 / 设置 / 置顶

## 新增小组件

1. `src/widgets/<name>.tsx` 实现 `WidgetDefinition`（`compact` + `expanded` 双形态）
2. 在 `src/widgets/index.ts` 注册
3. 数据接口放 `src/api/<name>/index.ts`

详细规范见 [AGENTS.md](./AGENTS.md)。

## 目录职责

```text
src/
  pages/        页面（目录入口 index.tsx）
  components/
    common/     跨页面公共 UI
    island/     灵动岛核心组件
  widgets/      小组件注册中心
  api/          接口层（Tauri 命令 / 外部 HTTP）
  utils/        request / toast
  config/       岛体尺寸常量
  lib/          cn() 等工具
  types/        共享类型
src-tauri/
  src/commands/ Rust 命令（按域分模块）
```

## Roadmap

- [ ] 网易云同步播放（登录 / 播放状态 / 歌词）
- [ ] 组件启用开关与配置持久化
- [ ] 系统托盘
- [ ] 鼠标穿透（收起态仅胶囊区域可交互）
