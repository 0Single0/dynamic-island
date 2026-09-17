import type { WidgetSize } from "@/widgets/types";

/** 展开面板按 size 档位映射的像素尺寸（岛体渲染尺寸） */
export const EXPANDED_SIZE: Record<WidgetSize, { width: number; height: number }> =
  {
    sm: { width: 320, height: 180 },
    md: { width: 420, height: 260 },
    lg: { width: 520, height: 320 },
  };

/** 收起态胶囊尺寸 */
export const COMPACT_SIZE = { width: 300, height: 44 };

/** hover 预览放大系数 1.06 的放大余量 */
export const HOVER_SCALE = 1.06;

/**
 * 窗口固定尺寸 = 最大展开档位 + 四周留白。
 * 窗口尺寸从不参与动画（Windows 程序化 resize 无平滑动画，会造成抽搐），
 * 所有伸缩动画都在这个透明窗口内部由 framer-motion 完成。
 */
export const WINDOW_SIZE = {
  width: EXPANDED_SIZE.lg.width + 32,
  height: EXPANDED_SIZE.lg.height + 32,
};

/** 窗口顶边距（屏幕顶部到窗口顶边） */
export const WINDOW_MARGIN_TOP = 4;
