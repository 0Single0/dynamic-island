import type { ComponentType } from "react";
import { useParams } from "react-router-dom";
import { getTool } from "@/widgets/tools";
import { WindowControls, useOpaqueWindowBackground } from "@/components/common/WindowChrome";

/**
 * 工具窗口页（`#/tool/:id`）。
 *
 * 窗口是**无边框**的（`decorations: false`），所以这里自己画标题栏：
 * 一条可拖拽的横条 + 最小化/最大化/关闭 —— 与岛体的玻璃风格统一。
 */
export default function ToolPage() {
  const { id } = useParams();
  const tool = id ? getTool(id) : undefined;

  // 独立窗口不透明；index.css 把根节点设成了透明（为岛体服务）。
  useOpaqueWindowBackground();

  if (!tool) {
    return (
      <div className="flex h-screen w-screen items-center justify-center bg-zinc-950 text-sm text-zinc-400">
        未知工具：{id ?? "(空)"}
      </div>
    );
  }

  const Tool = tool.component;
  const Icon = tool.icon as ComponentType<{ className?: string; strokeWidth?: number }>;

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-[#0b0b0e] text-zinc-200">
      <div
        data-tauri-drag-region
        className="flex h-9 shrink-0 cursor-grab items-center gap-2 border-b border-white/8 px-3 active:cursor-grabbing"
      >
        <Icon className="h-3.5 w-3.5 text-amber-300" strokeWidth={1.8} />
        <span className="text-[12px] font-medium text-zinc-200">{tool.name}</span>
        <span className="ml-auto">
          <WindowControls />
        </span>
      </div>
      <div className="min-h-0 flex-1 overflow-hidden p-3">
        <Tool />
      </div>
    </div>
  );
}
