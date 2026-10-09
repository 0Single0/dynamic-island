import type { LucideIcon } from "lucide-react";
import type { ComponentType } from "react";

/**
 * 小组件的展示形态。
 * compact —— 岛体收起时在胶囊内显示的极简信息（时间、频谱、封面等）。
 * expanded —— 岛体展开后渲染的完整面板。
 */
export type WidgetSize = "sm" | "md" | "lg" | "xl";
export interface WidgetExpandedRenderProps { headerSlot?: HTMLElement | null }

export interface WidgetCompactRenderProps {
  /** 是否处于悬浮预览状态（岛体微展时） */
  hovered: boolean;
}

export interface WidgetDefinition {
  /** 全局唯一 id，同时作为路由与配置键 */
  id: string;
  name: string;
  description?: string;
  icon: LucideIcon;
  /** 收起态在胶囊中的排列顺序，越小越靠前 */
  order: number;
  /** 收起态胶囊内的紧凑展示 */
  compact: ComponentType<WidgetCompactRenderProps>;
  /** 展开态完整面板 */
  expanded: ComponentType<WidgetExpandedRenderProps>;
  /** 展开面板的期望宽度档位，影响岛体展开尺寸 */
  size?: WidgetSize;
  /** 是否默认启用；后续接入设置页后由配置驱动 */
  enabledByDefault?: boolean;
}
