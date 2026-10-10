import { useEffect, type ComponentType } from "react";
import { useParams } from "react-router-dom";
import { Minus, Square, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getTool } from "@/widgets/tools";

/**
 * 工具窗口页（`#/tool/:id`）。
 *
 * 窗口是**无边框**的（`decorations: false`），所以这里自己画标题栏：
 * 一条可拖拽的横条 + 最小化/最大化/关闭 —— 与岛体的玻璃风格统一。
 */
export default function ToolPage() {
  const { id } = useParams();
  const tool = id ? getTool(id) : undefined;

  // index.css 把 html/body/#root 设成透明（岛体窗口要透出桌面），
  // 而工具窗口本身不透明 —— 不管的话 WebView 的白底就从边角透出来。
  useEffect(() => {
    const nodes = [document.documentElement, document.body];
    const previous = nodes.map((node) => node.style.background);
    nodes.forEach((node) => {
      node.style.background = "#0b0b0e";
    });
    return () => {
      nodes.forEach((node, index) => {
        node.style.background = previous[index];
      });
    };
  }, []);

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
        <span className="ml-auto flex items-center gap-0.5">
          <WindowButton label="最小化" onClick={() => void getCurrentWindow().minimize()}>
            <Minus className="h-3.5 w-3.5" />
          </WindowButton>
          <WindowButton label="最大化" onClick={() => void getCurrentWindow().toggleMaximize()}>
            <Square className="h-2.5 w-2.5" />
          </WindowButton>
          <WindowButton
            label="关闭"
            danger
            onClick={() => void getCurrentWindow().close()}
          >
            <X className="h-3.5 w-3.5" />
          </WindowButton>
        </span>
      </div>
      <div className="min-h-0 flex-1 overflow-hidden p-3">
        <Tool />
      </div>
    </div>
  );
}

function WindowButton({
  label,
  onClick,
  danger,
  children,
}: {
  label: string;
  onClick: () => void;
  danger?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      className={`flex h-6 w-8 items-center justify-center rounded-md text-zinc-400 transition-colors ${
        danger ? "hover:bg-rose-500/85 hover:text-white" : "hover:bg-white/12 hover:text-zinc-100"
      }`}
    >
      {children}
    </button>
  );
}
