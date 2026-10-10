import { RadioTower } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { ComponentType } from "react";
import type { WidgetSize } from "./types";
import { MarkdownIcon } from "./MarkdownIcon";
import { MarkdownPdfExpanded } from "./MarkdownPdfExpanded";
import { ProxyPoolExpanded } from "./proxy-pool";

/**
 * 工具注册中心：与 widget 不同，工具没有"收起态胶囊"，只能从应用列表打开。
 * 新增工具只改这里 —— 应用列表、岛体标题、面板尺寸都由注册表驱动。
 */
export type ToolId = "proxy-pool" | "markdown-pdf";

/** 工具图标：lucide 图标或自定义 SVG 组件都能用（应用列表统一按组件渲染）。 */
export type ToolIcon =
  | LucideIcon
  | ComponentType<{ className?: string; strokeWidth?: number }>;

export interface ToolDefinition {
  id: ToolId;
  name: string;
  icon: ToolIcon;
  /** 展开面板尺寸档位（在岛体内渲染时用） */
  size: WidgetSize;
  /**
   * 声明后该工具在**独立窗口**里打开，而不是把岛体放大。
   * 岛体面板最大 780×480，只够看；要操作表格/日志就得给个真窗口。
   */
  window?: { title: string; width: number; height: number };
  component: ComponentType;
}

export const toolRegistry: ToolDefinition[] = [
  {
    id: "proxy-pool",
    name: "反代号池",
    icon: RadioTower,
    size: "xl",
    window: { title: "反代号池", width: 1040, height: 720 },
    component: ProxyPoolExpanded,
  },
  {
    id: "markdown-pdf",
    name: "MD 转 PDF",
    icon: MarkdownIcon,
    size: "lg",
    component: MarkdownPdfExpanded,
  },
];

export function getTool(id: string): ToolDefinition | undefined {
  return toolRegistry.find((tool) => tool.id === id);
}
