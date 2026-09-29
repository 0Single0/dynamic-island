import { widgetRegistry } from "@/widgets";
import type { WidgetDefinition } from "./types";
import { MarkdownIcon } from "./MarkdownIcon";

export type LauncherTarget =
  | { kind: "widget"; id: string }
  | { kind: "tool"; id: "markdown-pdf" };

export function ApplicationLauncher({
  onOpen,
}: {
  onOpen: (target: LauncherTarget) => void;
}) {
  const cards: Array<{
    id: string;
    name: string;
    icon: WidgetDefinition["icon"] | typeof MarkdownIcon;
    target: LauncherTarget;
    accent: string;
  }> = [
    ...widgetRegistry.map((widget) => ({
      id: widget.id,
      name: widget.name,
      icon: widget.icon,
      target: { kind: "widget" as const, id: widget.id },
      accent: widget.id === "music" ? "text-emerald-300" : "text-sky-300",
    })),
    {
      id: "markdown-pdf",
      name: "MD 转 PDF",
      icon: MarkdownIcon,
      target: { kind: "tool", id: "markdown-pdf" },
      accent: "text-violet-300",
    },
  ];

  return (
    <div className="flex h-full w-full items-start justify-start px-3 pt-3">
      <div className="grid w-fit grid-cols-3 gap-2.5">
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
              className="group flex h-[116px] w-[116px] flex-col items-center justify-center gap-1.5 rounded-2xl border border-white/10 bg-white/[0.045] p-2 text-center transition-all hover:-translate-y-0.5 hover:border-white/20 hover:bg-white/[0.09] active:translate-y-0"
            >
              <div className="flex items-center justify-center">
                <span className={`flex h-12 w-12 items-center justify-center rounded-[16px] bg-black/20 ${card.accent}`}>
                  <Icon className="h-7 w-7" strokeWidth={1.8} />
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
