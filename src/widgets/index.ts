import { Clock, Music2 } from "lucide-react";
import type { WidgetDefinition } from "./types";
import { clockWidget } from "./clock";
import { musicWidget } from "./music";

/**
 * 小组件注册中心：新增组件只改这里。
 * 岛体收起/展开的渲染完全由注册表驱动，页面代码不感知具体组件。
 */
export const widgetRegistry: WidgetDefinition[] = [
  clockWidget,
  musicWidget,
].sort((a, b) => a.order - b.order);

export function getWidget(id: string): WidgetDefinition | undefined {
  return widgetRegistry.find((w) => w.id === id);
}

export { Clock, Music2 };
