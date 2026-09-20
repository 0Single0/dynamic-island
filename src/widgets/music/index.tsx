import type { LucideIcon } from "lucide-react";
import type { ComponentType } from "react";
import { Music2 } from "lucide-react";
import { MusicCompact } from "./components/MusicCompact";
import { MusicExpanded } from "./components/MusicExpanded";
import type { WidgetDefinition } from "../types";

/**
 * 网易云同步播放组件：CDP 直连网易云客户端。
 * 胶囊态显示封面+歌名，展开态为完整播放卡（进度/歌词/控制）。
 */
export const musicWidget: WidgetDefinition = {
  id: "music",
  name: "音乐",
  description: "网易云同步播放",
  icon: Music2,
  order: 20,
  compact: MusicCompact,
  expanded: MusicExpanded,
  size: "lg",
  enabledByDefault: true,
};

/** 供其他模块（如设置页）复用的图标引用 */
export const musicIcon: LucideIcon = Music2;
export type { WidgetDefinition, ComponentType };
