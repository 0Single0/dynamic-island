import { Music2 } from "lucide-react";
import { useNeteaseState } from "@/stores/netease/useNeteaseState";
import { CoverImage } from "./CoverImage";

/** 胶囊态：封面（播放中旋转；无封面用图标占位）+ 歌名 + 播放中的跳动音条。 */
export function MusicCompact() {
  const { state } = useNeteaseState();
  const track = state.track;
  const hasTrack = Boolean(track?.title);
  const playing = state.isPlaying;

  if (!hasTrack) {
    return (
      <span className="flex items-center gap-1.5 text-[13px] text-zinc-300">
        <Music2 className="h-3.5 w-3.5 text-emerald-400" strokeWidth={2.2} />
        <span className="max-w-28 truncate">未在播放</span>
      </span>
    );
  }

  return (
    <span className="flex items-center gap-1.5 text-[13px] text-zinc-300">
      <span className="relative h-4 w-4 shrink-0">
        {track?.coverUrl && (
          <CoverImage
            url={track.coverUrl}
            className={`absolute inset-0 h-4 w-4 rounded-full object-cover ${
              playing ? "animate-disc-spin" : ""
            }`}
          />
        )}
        {!track?.coverUrl && (
          <Music2
            className={`absolute inset-0 h-4 w-4 text-emerald-400 ${
              playing ? "animate-disc-spin" : ""
            }`}
            strokeWidth={2.2}
          />
        )}
      </span>
      <span className="max-w-28 truncate">{track?.title}</span>
      {playing && <EqualizerBars />}
    </span>
  );
}

/** 三根跳动音条：纯 CSS 动画，播放中显示。 */
function EqualizerBars() {
  return (
    <span className="flex h-3 w-3.5 items-end justify-center gap-[2px]">
      {[0, 1, 2].map((i) => (
        <span
          key={i}
          className="w-[2.5px] rounded-full bg-emerald-400"
          style={{
            height: "40%",
            animation: `eq-bounce 900ms ease-in-out ${i * 180}ms infinite alternate`,
          }}
        />
      ))}
    </span>
  );
}
