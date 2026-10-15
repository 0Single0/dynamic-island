import { useCallback, useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import { Settings, X } from "lucide-react";
import { widgetRegistry, getWidget } from "@/widgets";
import { ApplicationLauncher, type LauncherTarget } from "@/widgets/ApplicationLauncher";
import { getTool } from "@/widgets/tools";
import { openSettingsWindow, openToolWindow } from "@/api/window";
import { toast } from "@/utils/toast";
import {
  COMPACT_SIZE,
  EXPANDED_SIZE,
  HOVER_SCALE,
  LAUNCHER_SIZE,
  WINDOW_SIZE,
  WINDOW_MARGIN_TOP,
} from "@/config/island";
import {
  positionWindow,
  setAlwaysOnTop,
  setIslandRect,
} from "@/api/window";
import { IconButton } from "@/components/common/IconButton";
import { readIslandPanel, rememberIslandPanel, type ActivePanel } from "./panelState";

type IslandState = "collapsed" | "hovered" | "expanded";

/**
 * 灵动岛主体。
 * 窗口固定为最大尺寸的透明矩形、顶边钉死、水平居中，从不参与动画
 * （Windows 程序化窗口 resize 是瞬跳，参与动画必然抽搐）。
 * 所有伸缩动画都是 webview 内部渲染：岛体常驻，弹簧驱动宽高，
 * compact / expanded 两个内容层绝对定位、透明度交叉淡化。
 */
export function Island() {
  const [state, setState] = useState<IslandState>(() => (readIslandPanel().expanded ? "expanded" : "collapsed"));
  const [activePanel, setActivePanel] = useState<ActivePanel>(() => readIslandPanel().panel);
  const hoverTimer = useRef<number | null>(null);
  const pressedRef = useRef(false);
  const [widgetHeaderSlot, setWidgetHeaderSlot] = useState<HTMLSpanElement | null>(null);

  useEffect(() => {
    rememberIslandPanel(state === "expanded", activePanel);
  }, [activePanel, state]);

  const activeWidget = activePanel.kind === "widget" ? getWidget(activePanel.id) : undefined;
  const activeTool = activePanel.kind === "tool" ? getTool(activePanel.id) : undefined;
  const isLauncher = activePanel.kind === "launcher";
  const expandedSize = isLauncher
    ? LAUNCHER_SIZE
    : activeTool
      ? EXPANDED_SIZE[activeTool.size]
      : EXPANDED_SIZE[activeWidget?.size ?? "md"];

  // 启动时恢复固定的顶部居中位置，不读取旧的拖拽停靠记录。
  const initWindow = useCallback(() => {
    void positionWindow(WINDOW_MARGIN_TOP, WINDOW_SIZE.width, WINDOW_SIZE.height);
    void setAlwaysOnTop();
  }, []);

  useEffect(() => {
    initWindow();
  }, [initWindow]);

  const enter = () => {
    if (state === "expanded" || pressedRef.current) return;
    hoverTimer.current = window.setTimeout(() => {
      setState((prev) => (prev === "collapsed" ? "hovered" : prev));
    }, 120);
  };

  const leave = () => {
    if (hoverTimer.current) {
      window.clearTimeout(hoverTimer.current);
      hoverTimer.current = null;
    }
    if (state === "hovered") setState("collapsed");
  };

  const expand = (panel: ActivePanel) => {
    pressedRef.current = true;
    setActivePanel(panel);
    setState("expanded");
    window.setTimeout(() => {
      pressedRef.current = false;
    }, 300);
  };

  const collapse = () => {
    setState("collapsed");
    setActivePanel({ kind: "launcher" });
  };

  const isCollapsed = state !== "expanded";
  const islandRef = useRef<HTMLDivElement | null>(null);

  const openLauncherTarget = (target: LauncherTarget) => {
    if (target.kind === "widget") {
      expand({ kind: "widget", id: target.id });
      return;
    }
    // 声明了独立窗口的工具：开窗 + 收起岛体，不在 780×480 的胶囊里挤。
    const tool = getTool(target.id);
    if (tool?.window) {
      collapse();
      void openToolWindow(tool.id, tool.window.title, tool.window.width, tool.window.height).catch(
        (reason) =>
          toast.error(
            reason instanceof Error ? reason.message : "工具窗口打不开（改了 Rust 侧需要重新编译）",
          ),
      );
      return;
    }
    expand({ kind: "tool", id: target.id });
  };

  // 动画帧上把岛体视口逻辑坐标上报给 Rust（换算物理像素后驱动点击穿透）。
  // hover 缩放围绕岛体中心，需按 scale 反算实际占位矩形。
  const reportRect = useCallback(() => {
    const el = islandRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    void setIslandRect(rect.left, rect.top, rect.width, rect.height);
  }, []);

  // 初始上报一次（首个动画帧之前窗口仍需可点）
  useEffect(() => {
    reportRect();
    window.addEventListener("resize", reportRect);
    return () => window.removeEventListener("resize", reportRect);
  }, [reportRect]);

  return (
    <div
      className="flex h-screen w-screen items-start justify-center bg-transparent pt-2"
      onClick={() => {
        if (state === "expanded") collapse();
      }}
    >
      {/* 岛体：唯一常驻元素，尺寸弹簧驱动，与窗口尺寸无关 */}
      <motion.div
        ref={islandRef}
        onClick={(event) => {
          // 阻止冒泡到外层透明区域，避免展开态内部点击被判定为“点外部”。
          event.stopPropagation();
          if (isCollapsed) expand({ kind: "launcher" });
        }}
        onMouseEnter={enter}
        onMouseLeave={leave}
        onUpdate={reportRect}
        animate={{
          width: isCollapsed ? COMPACT_SIZE.width : expandedSize.width,
          height: isCollapsed ? COMPACT_SIZE.height : expandedSize.height,
          scale: state === "hovered" ? HOVER_SCALE : 1,
        }}
        transition={{
          type: "spring",
          stiffness: 480,
          damping: 34,
          mass: 0.9,
        }}
        className="glass-island relative cursor-pointer overflow-hidden"
        style={{ borderRadius: isCollapsed ? 22 : 12 }}
      >
        {/* 内容层一：compact（胶囊信息），仅透明度变化 */}
        <motion.div
          initial={false}
          animate={{
            opacity: isCollapsed ? 1 : 0,
            scale: isCollapsed ? 1 : 0.9,
          }}
          transition={{ duration: 0.16, ease: "easeOut" }}
          className="absolute inset-0 flex items-center justify-between gap-4 px-4"
          style={{ pointerEvents: isCollapsed ? "auto" : "none" }}
        >
          {widgetRegistry.filter((widget) => widget.id !== "ai").map((widget) => (
            <button
              key={widget.id}
              type="button"
              title={widget.name}
              onClick={(event) => {
                event.stopPropagation();
                expand({ kind: "widget", id: widget.id });
              }}
              className="cursor-pointer"
            >
              <widget.compact hovered={false} />
            </button>
          ))}
        </motion.div>

        {/* 内容层二：expanded（完整面板），仅透明度变化 */}
        <motion.div
          initial={false}
          animate={{
            opacity: isCollapsed ? 0 : 1,
            scale: isCollapsed ? 1.08 : 1,
          }}
          transition={{
            duration: 0.2,
            ease: "easeOut",
            delay: isCollapsed ? 0 : 0.08,
          }}
          className="absolute inset-0 flex flex-col p-4"
          style={{ pointerEvents: isCollapsed ? "none" : "auto" }}
        >
          {/* 顶栏只提供标题与操作按钮。 */}
          <div className="flex h-7 shrink-0 items-center justify-between">
            <div className="flex min-w-0 items-center gap-2">
              <span ref={setWidgetHeaderSlot} className="contents" />
              {isLauncher && <IconButton icon={Settings} label="打开设置中心" onClick={() => void openSettingsWindow()} />}
              <span className="text-sm font-semibold text-zinc-100">
                {activeWidget?.name ?? activeTool?.name ?? (isLauncher ? "应用" : "")}
              </span>
            </div>
            <div className="flex items-center gap-1">
              {(isLauncher || activeWidget?.id === "ai") && (
                <IconButton
                  icon={Settings}
                  label={isLauncher ? "打开设置中心" : "打开模型设置"}
                  onClick={() => void openSettingsWindow(isLauncher ? undefined : "ai-models")}
                />
              )}
              <IconButton icon={X} label="收起" onClick={collapse} />
            </div>
          </div>
          <div className="min-h-0 flex-1 overflow-hidden">
            {isLauncher ? (
              <ApplicationLauncher onOpen={openLauncherTarget} />
            ) : activeWidget ? (
              <activeWidget.expanded headerSlot={widgetHeaderSlot} />
            ) : activeTool ? (
              <activeTool.component />
            ) : null}
          </div>
        </motion.div>
      </motion.div>
    </div>
  );
}
