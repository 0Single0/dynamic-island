import type { ComponentType } from "react";
import IslandPage from "@/pages/Island";
import SettingsPage from "@/pages/Settings";
import ToolPage from "@/pages/Tool";

export interface AppRoute {
  path: string;
  Component: ComponentType;
}

/**
 * 路由表：新增页面时在此登记。
 * 页面文件必须使用目录入口形式，如 src/pages/Chat/index.tsx。
 */
export const routes: AppRoute[] = [
  { path: "/", Component: IslandPage },
  { path: "/settings", Component: SettingsPage },
  // 工具窗口页：同一个工具组件，给它整个窗口
  { path: "/tool/:id", Component: ToolPage },
];
