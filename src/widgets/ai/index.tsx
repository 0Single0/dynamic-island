import { Bot } from "lucide-react";
import type { WidgetDefinition } from "@/widgets/types";
import { AIExpanded } from "./AIExpanded";

function AICompact() {
  // AI 仍然作为应用保留在应用列表中，但不在顶部胶囊里占用一个图标位。
  return <span className="h-8 w-8" aria-hidden="true" />;
}

export const aiWidget: WidgetDefinition = {
  id: "ai",
  name: "AI Agent",
  description: "通用桌面 Agent",
  icon: Bot,
  order: 0,
  compact: AICompact,
  expanded: AIExpanded,
  size: "xl",
  enabledByDefault: true,
};
