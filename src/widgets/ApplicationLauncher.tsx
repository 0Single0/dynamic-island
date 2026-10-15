import { useEffect, useRef, useState } from "react";
import { widgetRegistry } from "@/widgets";
import { cn } from "@/lib/utils";
import type { WidgetDefinition } from "./types";
import { toolRegistry, type ToolIcon, type ToolId } from "./tools";

export type LauncherTarget =
  | { kind: "widget"; id: string }
  | { kind: "tool"; id: ToolId };

export function ApplicationLauncher({
  onOpen,
}: {
  onOpen: (target: LauncherTarget) => void;
}) {
  const [scrolling, setScrolling] = useState(false);
  const hideTimer = useRef<number | null>(null);

  // 滚动条只在滚动过程中出现，停下约 700ms 后淡出，避免静止时占位扎眼。
  const handleScroll = () => {
    setScrolling(true);
    if (hideTimer.current) window.clearTimeout(hideTimer.current);
    hideTimer.current = window.setTimeout(() => setScrolling(false), 700);
  };

  useEffect(
    () => () => {
      if (hideTimer.current) window.clearTimeout(hideTimer.current);
    },
    [],
  );

  const cards: Array<{
    id: string;
    name: string;
    icon: WidgetDefinition["icon"] | ToolIcon;
    target: LauncherTarget;
    accent: string;
  }> = [
    ...widgetRegistry.map((widget) => ({
      id: widget.id,
      name: widget.name,
      icon: widget.icon,
      target: { kind: "widget" as const, id: widget.id },
      accent: widget.id === "music" ? "text-emerald-300" : widget.id === "ai" ? "text-sky-200" : "text-sky-300",
    })),
    ...toolRegistry.map((tool) => ({
      id: tool.id,
      name: tool.name,
      icon: tool.icon,
      target: { kind: "tool" as const, id: tool.id },
      accent: tool.id === "proxy-pool" ? "text-amber-300" : "text-violet-300",
    })),
  ];

  return (
    <div
      onScroll={handleScroll}
      className={cn(
        "island-scroll h-full w-full overflow-y-auto px-3 pt-3",
        scrolling && "is-scrolling",
      )}
    >
      <div className="grid w-fit grid-cols-4 gap-2.5">
        {cards.map((card) => {
          const Icon = card.icon;
          return (
            <button
              key={card.id}
              type="button"
              onClick={(event) => {
                event.stopPropagation();
                onOpen(card.target);
              }}
              className="group flex h-[86px] w-[86px] flex-col items-center justify-center gap-1.5 rounded-2xl border border-white/10 bg-white/[0.045] p-2 text-center transition-all hover:-translate-y-0.5 hover:border-white/20 hover:bg-white/[0.09] active:translate-y-0"
            >
              <div className="flex items-center justify-center">
                <span className={`flex h-10 w-10 items-center justify-center rounded-[14px] bg-black/20 ${card.accent}`}>
                  <Icon className="h-6 w-6" strokeWidth={1.8} />
                </span>
              </div>
              <div className="w-full">
                <div className="truncate text-[11px] font-medium text-zinc-100">{card.name}</div>
              </div>
            </button>
          );
        })}
      </div>
    </div>
  );
}
