import { Music2 } from "lucide-react";
import type { WidgetDefinition } from "./types";

/**
 * 网易云同步播放组件（占位实现）。
 * 数据接入见 src/api/netease/，后续通过 usePlayerStore 对接真实播放状态。
 */
function MusicCompact() {
  return (
    <span className="flex items-center gap-1.5 text-[13px] text-zinc-300">
      <Music2 className="h-3.5 w-3.5 text-emerald-400" strokeWidth={2.2} />
      <span className="max-w-28 truncate">未在播放</span>
    </span>
  );
}

function MusicExpanded() {
  return (
    <div className="flex h-full w-full flex-col items-center justify-center gap-3">
      <div className="flex h-16 w-16 items-center justify-center rounded-xl bg-zinc-800">
        <Music2 className="h-8 w-8 text-zinc-500" />
      </div>
      <div className="text-sm font-medium text-zinc-200">网易云音乐同步</div>
      <div className="text-xs text-zinc-500">待接入：登录 / 播放状态 / 歌词</div>
    </div>
  );
}

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
