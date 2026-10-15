import { useEffect, type ReactNode } from "react";
import { Minus, Square, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * 独立窗口（工具窗口 / 设置窗口）的公共外观。
 *
 * 这些窗口是 `decorations: false` 的，没有系统标题栏，所以拖拽手柄和
 * 最小化/最大化/关闭都得应用自己画 —— 统一放这里，避免每个窗口各写一份。
 */

/**
 * index.css 把 html/body/#root 设成透明（岛体窗口要透出桌面），
 * 而独立窗口本身不透明 —— 不管的话 WebView 的白底会从边角透出来。
 */
export function useOpaqueWindowBackground(color = "#0b0b0e") {
  useEffect(() => {
    const nodes = [document.documentElement, document.body];
    const previous = nodes.map((node) => node.style.background);
    nodes.forEach((node) => {
      node.style.background = color;
    });
    return () => {
      nodes.forEach((node, index) => {
        node.style.background = previous[index];
      });
    };
  }, [color]);
}

/**
 * 无边框窗口的标题栏控件。
 *
 * 单独渲染按钮组；整条标题栏（含拖拽区域）由调用方布局，
 * 因为各窗口左侧放什么（图标、标题、标签）并不一样。
 */
export function WindowControls() {
  return (
    <span className="flex items-center gap-0.5">
      <WindowButton label="最小化" onClick={() => void getCurrentWindow().minimize()}>
        <Minus className="h-3.5 w-3.5" />
      </WindowButton>
      <WindowButton label="最大化" onClick={() => void getCurrentWindow().toggleMaximize()}>
        <Square className="h-2.5 w-2.5" />
      </WindowButton>
      <WindowButton label="关闭" danger onClick={() => void getCurrentWindow().close()}>
        <X className="h-3.5 w-3.5" />
      </WindowButton>
    </span>
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
  children: ReactNode;
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
