import { useCallback, useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import { X, Settings, Pin } from "lucide-react";
import { widgetRegistry, getWidget } from "@/widgets";
import {
  COMPACT_SIZE,
  EXPANDED_SIZE,
  HOVER_SCALE,
  WINDOW_SIZE,
  WINDOW_MARGIN_TOP,
} from "@/config/island";
import {
  positionWindow,
  setAlwaysOnTop,
  setIslandRect,
} from "@/api/window";
import { IconButton } from "@/components/common/IconButton";

type IslandState = "collapsed" | "hovered" | "expanded";

/**
 * 灵动岛主体。
 * 窗口固定为最大尺寸的透明矩形、顶边钉死、水平居中，从不参与动画
 * （Windows 程序化窗口 resize 是瞬跳，参与动画必然抽搐）。
 * 所有伸缩动画都是 webview 内部渲染：岛体常驻，弹簧驱动宽高，
 * compact / expanded 两个内容层绝对定位、透明度交叉淡化。
 */
export function Island() {
  const [state, setState] = useState<IslandState>("collapsed");
  const [activeId, setActiveId] = useState<string | null>(null);
  const hoverTimer = useRef<number | null>(null);
  const pressedRef = useRef(false);

  const activeWidget = activeId ? getWidget(activeId) : undefined;
  const expandedSize = activeWidget
    ? EXPANDED_SIZE[activeWidget.size ?? "md"]
    : EXPANDED_SIZE.md;

  // 启动：置顶 + 固定窗口位置（只调一次）
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

  const expand = (widgetId: string | null) => {
    pressedRef.current = true;
    setActiveId(widgetId);
    setState("expanded");
    window.setTimeout(() => {
      pressedRef.current = false;
    }, 300);
  };

  const collapse = () => {
    setState("collapsed");
    setActiveId(null);
  };

  const isCollapsed = state !== "expanded";
  // 点击胶囊体（而非具体小组件）时展开的默认面板
  const defaultWidget = widgetRegistry[0];

  // 动画帧上把岛体视口逻辑坐标上报给 Rust（换算物理像素后驱动点击穿透）。
  // hover 缩放围绕岛体中心，需按 scale 反算实际占位矩形。
  const islandRef = useRef<HTMLDivElement | null>(null);
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
      className="flex h-screen w-screen items-start justify-center bg-transparent"
      style={{ paddingTop: 8 }}
      onClick={() => {
        if (state === "expanded") collapse();
      }}
    >
      {/* 岛体：唯一常驻元素，尺寸弹簧驱动，与窗口尺寸无关 */}
      <motion.div
        ref={islandRef}
        onClick={(event) => {
          // 阻止冒泡到外层透明区域，避免展开态内部点击被判定为“点外部”
          event.stopPropagation();
          if (isCollapsed) expand(defaultWidget?.id ?? null);
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
          {widgetRegistry.map((widget) => (
            <button
              key={widget.id}
              type="button"
              title={widget.name}
              onClick={(event) => {
                event.stopPropagation();
                expand(widget.id);
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
          <div className="flex items-center justify-between">
            <span className="text-xs font-medium text-zinc-400">
              {activeWidget?.name ?? "灵动岛"}
            </span>
            <div className="flex items-center gap-0.5">
              <IconButton icon={Pin} label="置顶" />
              <IconButton icon={Settings} label="设置" />
              <IconButton icon={X} label="收起" onClick={collapse} />
            </div>
          </div>
          <div className="flex-1 overflow-hidden">
            {activeWidget && <activeWidget.expanded />}
          </div>
        </motion.div>
      </motion.div>
    </div>
  );
}
