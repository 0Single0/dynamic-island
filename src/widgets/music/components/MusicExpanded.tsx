import { Pause, Play, SkipBack, SkipForward, Loader2, RefreshCw } from "lucide-react";
import { useEffect, useRef, useState, type CSSProperties } from "react";
import { AnimatePresence, motion } from "framer-motion";
import * as netease from "@/api/netease";
import type { NeteaseControlAction } from "@/api/netease/types";
import { useNeteaseState } from "@/stores/netease/useNeteaseState";
import { CoverImage } from "./CoverImage";
import { cn } from "@/lib/utils";

/** 展开面板：连接状态行 + 封面/曲目信息 + 进度条 + 歌词 + 控制按钮。 */
export function MusicExpanded() {
  const { state, positionMs } = useNeteaseState();
  const track = state.track;

  return (
    <div className="flex h-full w-full flex-col gap-3">
      {state.connection !== "synchronizing" && state.connection !== "degraded" ? (
        <ConnectionPanel state={state} />
      ) : (
        <>
          <div className="flex min-h-0 flex-1 gap-4">
            {/* 左：封面 + 控制 */}
            <div className="flex w-40 shrink-0 flex-col items-center gap-2">
              <Cover url={track?.coverUrl} playing={state.isPlaying} />
              <TrackTitle title={track?.title ?? "未在播放"} />
              <span className="max-w-full truncate text-xs text-zinc-400">
                {track?.artist ?? "—"}
              </span>
              <Controls isPlaying={state.isPlaying} connected={track != null} />
            </div>
            {/* 右：进度 + 歌词 */}
            <div className="flex min-w-0 flex-1 flex-col justify-center gap-3">
              <ProgressBar
                positionMs={positionMs}
                durationMs={track?.durationMs ?? 0}
              />
              <LyricsView state={state} />
            </div>
          </div>
        </>
      )}
    </div>
  );
}

function ConnectionPanel({ state }: { state: ReturnType<typeof useNeteaseState>["state"] }) {
  const [starting, setStarting] = useState(false);

  const handleStart = async () => {
    setStarting(true);
    await netease.startOrConnect();
    // 启动/连接是异步过程，结果通过 netease-state 事件回流
    window.setTimeout(() => setStarting(false), 20000);
  };

  const handleRestart = async () => {
    setStarting(true);
    await netease.restartWithSync();
    window.setTimeout(() => setStarting(false), 20000);
  };

  const needsRestart = state.connection === "needs-restart";
  const isError = state.connection === "error";
  const idle = state.connection === "idle" || state.connection === "waiting-for-cdp";

  return (
    <div className="flex h-full w-full flex-col items-center justify-center gap-3">
      <div className="flex h-16 w-16 items-center justify-center rounded-xl bg-zinc-800">
        {starting || state.connection === "waiting-for-cdp" ? (
          <Loader2 className="h-8 w-8 animate-spin text-emerald-400" />
        ) : (
          <RefreshCw className="h-7 w-7 text-zinc-500" />
        )}
      </div>
      <div className="text-sm font-medium text-zinc-200">网易云音乐同步</div>
      <div className="max-w-80 text-center text-xs leading-5 text-zinc-500">
        {needsRestart
          ? (state.message ?? "需要由本工具重启网易云以启用精确同步")
          : isError
            ? (state.message ?? "连接出错")
            : idle
              ? "通过调试端口同步播放状态与逐行歌词"
              : (state.message ?? "")}
      </div>
      {needsRestart ? (
        <div className="flex gap-2">
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              void handleRestart();
            }}
            className="rounded-full bg-emerald-500 px-4 py-1.5 text-xs font-medium text-black transition-colors hover:bg-emerald-400"
          >
            保存并重启网易云
          </button>
        </div>
      ) : (
        <button
          type="button"
          disabled={starting}
          onClick={(e) => {
            e.stopPropagation();
            void handleStart();
          }}
          className="rounded-full bg-emerald-500 px-4 py-1.5 text-xs font-medium text-black transition-colors hover:bg-emerald-400 disabled:opacity-50"
        >
          {starting ? "连接中…" : "启动网易云同步"}
        </button>
      )}
    </div>
  );
}

function Cover({ url, playing }: { url?: string; playing: boolean }) {
  return (
    <div className="relative h-24 w-24 overflow-hidden rounded-full bg-zinc-800 shadow-lg ring-2 ring-white/10">
      {url ? (
        <CoverImage
          url={url}
          className={cn(
            "h-full w-full rounded-full object-cover animate-disc-spin",
            !playing && "animate-disc-spin-paused",
          )}
        />
      ) : null}
      {!url && (
        <div className="flex h-full w-full items-center justify-center">
          <RefreshCw
            className={cn(
              "h-8 w-8 text-zinc-600 animate-disc-spin",
              !playing && "animate-disc-spin-paused",
            )}
          />
        </div>
      )}
    </div>
  );
}

/**
 * 歌名仅在宽度溢出时启用跑马灯。两份连续文本保证循环交界不会闪回开头。
 */
function TrackTitle({ title }: { title: string }) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const textRef = useRef<HTMLSpanElement>(null);
  const [scrollDistance, setScrollDistance] = useState(0);
  const shouldScroll = scrollDistance > 0;

  useEffect(() => {
    const measure = () => {
      const viewportWidth = viewportRef.current?.clientWidth ?? 0;
      const textWidth = textRef.current?.getBoundingClientRect().width ?? 0;
      const overflow = textWidth - viewportWidth;
      // 文字之间留一段呼吸感，再接上下一轮同名文本。
      setScrollDistance(overflow > 1 ? Math.ceil(textWidth + 32) : 0);
    };

    measure();
    const observer = new ResizeObserver(measure);
    if (viewportRef.current) observer.observe(viewportRef.current);
    if (textRef.current) observer.observe(textRef.current);
    return () => observer.disconnect();
  }, [title]);

  const duration = Math.max(7, scrollDistance / 28);
  const marqueeStyle: CSSProperties & { "--track-title-shift": string } = {
    "--track-title-shift": `${scrollDistance}px`,
    animationDuration: `${duration}s`,
  };

  return (
    <div
      ref={viewportRef}
      className="w-full overflow-hidden text-center text-[15px] font-medium leading-6 text-zinc-100"
      title={title}
    >
      <div
        className={cn(
          "inline-flex w-max whitespace-nowrap",
          shouldScroll && "animate-track-title-marquee",
        )}
        style={shouldScroll ? marqueeStyle : undefined}
      >
        <span ref={textRef}>{title}</span>
        {shouldScroll && (
          <span className="ml-8" aria-hidden="true">
            {title}
          </span>
        )}
      </div>
    </div>
  );
}

function Controls({ isPlaying, connected }: { isPlaying: boolean; connected: boolean }) {
  const press = (action: NeteaseControlAction) => () => {
    if (!connected) return;
    void netease.control(action);
  };

  return (
    <div className="mt-1 flex items-center gap-1">
      <ControlButton onClick={press("previous")} disabled={!connected} label="上一首">
        <SkipBack className="h-4 w-4" fill="currentColor" />
      </ControlButton>
      <ControlButton
        onClick={press("toggle")}
        disabled={!connected}
        label={isPlaying ? "暂停" : "播放"}
        primary
      >
        {isPlaying ? (
          <Pause className="h-4.5 w-4.5" fill="currentColor" />
        ) : (
          <Play className="h-4.5 w-4.5 translate-x-[1px]" fill="currentColor" />
        )}
      </ControlButton>
      <ControlButton onClick={press("next")} disabled={!connected} label="下一首">
        <SkipForward className="h-4 w-4" fill="currentColor" />
      </ControlButton>
    </div>
  );
}

function ControlButton({
  children,
  onClick,
  disabled,
  label,
  primary,
}: {
  children: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
  label: string;
  primary?: boolean;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      className={cn(
        "flex items-center justify-center rounded-full transition-all",
        primary ? "h-9 w-9" : "h-8 w-8",
        disabled
          ? "cursor-not-allowed text-zinc-600"
          : primary
            ? "bg-white text-black hover:scale-105 active:scale-95"
            : "text-zinc-300 hover:bg-white/10 hover:text-white",
      )}
    >
      {children}
    </button>
  );
}

function fmt(ms: number) {
  const total = Math.floor(ms / 1000);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function ProgressBar({ positionMs, durationMs }: { positionMs: number; durationMs: number }) {
  const [draftPositionMs, setDraftPositionMs] = useState<number | null>(null);
  const [pendingSeekMs, setPendingSeekMs] = useState<number | null>(null);
  const draggingRef = useRef(false);
  // 松手到网易云实际完成 seek 之间，持续显示用户指定的位置，避免旧进度闪回。
  const displayPositionMs =
    draftPositionMs ?? pendingSeekMs ?? Math.min(positionMs, durationMs || positionMs);
  const pct = durationMs > 0 ? Math.min(100, (displayPositionMs / durationMs) * 100) : 0;

  useEffect(() => {
    if (!draggingRef.current) setDraftPositionMs(null);
  }, [positionMs]);

  useEffect(() => {
    if (pendingSeekMs === null) return;
    // 后端按整秒采样，允许约一秒的确认误差；超时则放弃乐观展示，回到真实状态。
    if (Math.abs(positionMs - pendingSeekMs) <= 1_200) {
      setPendingSeekMs(null);
      return;
    }
    const timeout = window.setTimeout(() => setPendingSeekMs(null), 1_800);
    return () => window.clearTimeout(timeout);
  }, [pendingSeekMs, positionMs]);

  const commitSeek = () => {
    if (!draggingRef.current) return;
    draggingRef.current = false;
    const nextPosition = draftPositionMs;
    if (nextPosition !== null && durationMs > 0) {
      setPendingSeekMs(nextPosition);
    }
    setDraftPositionMs(null);
    if (nextPosition !== null && durationMs > 0) {
      void netease.seek(nextPosition).then((accepted) => {
        // 页面未找到原始进度条或指令失败时，不要永久停在乐观位置。
        if (!accepted) {
          setPendingSeekMs((pending) => (pending === nextPosition ? null : pending));
        }
      });
    }
  };

  return (
    <div className="flex items-center gap-2 text-[11px] tabular-nums text-zinc-400">
      <span className="w-9 text-right">{fmt(displayPositionMs)}</span>
      <div className="relative flex h-5 flex-1 items-center">
        <div className="pointer-events-none absolute left-0 right-0 h-1 overflow-hidden rounded-full bg-white/10">
          <div
            className="h-full rounded-full bg-emerald-400 transition-[width] duration-150 ease-linear"
            style={{ width: `${pct}%` }}
          />
        </div>
        <input
          type="range"
          min={0}
          max={Math.max(durationMs, 1)}
          step={100}
          value={displayPositionMs}
          disabled={durationMs <= 0}
          aria-label="播放进度"
          onPointerDown={(event) => {
            event.stopPropagation();
            draggingRef.current = true;
          }}
          onPointerUp={(event) => {
            event.stopPropagation();
            commitSeek();
          }}
          onPointerCancel={commitSeek}
          onBlur={commitSeek}
          onChange={(event) => setDraftPositionMs(Number(event.target.value))}
          className="music-progress-range absolute inset-x-0 h-5 w-full cursor-pointer appearance-none bg-transparent disabled:cursor-default"
        />
      </div>
      <span className="w-9">{fmt(durationMs)}</span>
    </div>
  );
}

function LyricsView({ state }: { state: ReturnType<typeof useNeteaseState>["state"] }) {
  const lyrics = state.lyrics;

  if (!lyrics || lyrics.kind === "none") {
    return (
      <div className="flex flex-1 items-center justify-center text-xs text-zinc-600">
        {state.track?.songId ? "该歌曲暂无可同步歌词" : "未能精确匹配歌曲，已停用歌词"}
      </div>
    );
  }

  const lines = lyrics.visibleLines ?? [];

  if (lines.length === 0) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center text-xs text-zinc-600">
        正在定位当前歌词…
      </div>
    );
  }

  return (
    <div className="music-lyrics-mask min-h-0 flex-1 overflow-hidden">
      {/* 以当前高亮句为视觉中心：固定行高后，歌词轨道需额外上移。 */}
      <div className="flex min-h-full -translate-y-12 flex-col justify-center gap-1 py-4 text-center">
        <AnimatePresence initial={false} mode="popLayout">
          {lines.map((line) => (
            <motion.div
              layout="position"
              key={line.startMs}
              initial={{ opacity: 0 }}
              animate={{ opacity: line.active ? 1 : 0.64 }}
              exit={{ opacity: 0 }}
              transition={{
                layout: { duration: 0.42, ease: [0.22, 1, 0.36, 1] },
                opacity: { duration: 0.2, ease: "easeOut" },
              }}
              className={cn(
                "flex h-10 flex-col justify-start transition-[color,font-size,line-height] duration-300 ease-out",
                line.active
                  ? "text-[18px] font-semibold leading-6 text-zinc-50"
                  : "text-[14px] leading-6 text-zinc-500",
              )}
            >
              <div className="truncate px-2">{line.text || "　"}</div>
              {/* 每行固定留出翻译高度，当前行切换时不会再撑开/收起列表。 */}
              <div
                className={cn(
                  "h-4 truncate px-2 text-[11px] font-normal leading-4",
                  line.active ? "text-zinc-400" : "invisible",
                )}
              >
                {line.active ? (line.translation || "　") : "　"}
              </div>
            </motion.div>
          ))}
        </AnimatePresence>
      </div>
      {lyrics.kind === "translation" && (
        <div className="absolute bottom-0 right-0 bg-[#0a0a0c]/70 px-1.5 py-0.5 text-[10px] text-zinc-600">
          翻译歌词
        </div>
      )}
    </div>
  );
}
