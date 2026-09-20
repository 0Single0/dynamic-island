import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import * as api from "@/api/netease";
import type { NeteaseState } from "@/api/netease/types";

/**
 * 网易云同步状态 Hook。
 *
 * - 订阅 Rust `netease-state` 事件（节流后的真实快照，250ms 一拍）。
 * - 用 sampledAtEpochMs 在两次快照之间做本地插值，进度条/歌词平滑走秒。
 * - 挂载时拉取一次 current state（事件是推送制，不保证订阅前的状态）。
 * - 浏览器环境（无 Tauri）保持 idle 占位，UI 可独立调试。
 */

const EMPTY: NeteaseState = {
  connection: "idle",
  isPlaying: false,
  positionMs: 0,
  sampledAtEpochMs: 0,
};

/** 每 tick 刷新一次本地插值的时间（ms）。 */
const INTERPOLATION_TICK_MS = 200;
/**
 * 网易云页面的时间文本精度只有整秒。轮询恰好落在同一秒内时，后端会再次
 * 传来较旧的位置；小于此阈值的回退视为采样精度误差，而不是用户 seek。
 */
const SNAPSHOT_BACKTRACK_TOLERANCE_MS = 900;

function trackIdentity(state: NeteaseState) {
  const track = state.track;
  return [track?.songId ?? "", track?.title ?? "", track?.artist ?? "", track?.durationMs ?? 0].join("\u0000");
}

function positionAt(state: NeteaseState, now: number) {
  if (!state.isPlaying) return state.positionMs;
  const elapsed = now - state.sampledAtEpochMs;
  return elapsed > 0 && elapsed < 5000 ? state.positionMs + elapsed : state.positionMs;
}

/**
 * 合并后端快照时保持本地播放时钟单调前进，避免 4.8s -> 4.0s 这种整秒回跳。
 * 真正的跳转通常大于 0.9 秒，因此仍会马上回写到正确位置。
 */
function stabilizeSnapshot(previous: NeteaseState, incoming: NeteaseState): NeteaseState {
  const now = Date.now();
  const sameTrack = trackIdentity(previous) === trackIdentity(incoming);
  if (!sameTrack || !previous.isPlaying || !incoming.isPlaying) return incoming;

  const predictedPositionMs = positionAt(previous, now);
  const backtrackMs = predictedPositionMs - incoming.positionMs;
  if (backtrackMs <= 0 || backtrackMs > SNAPSHOT_BACKTRACK_TOLERANCE_MS) return incoming;

  return {
    ...incoming,
    positionMs: Math.round(predictedPositionMs),
    sampledAtEpochMs: now,
  };
}

export function useNeteaseState() {
  const [state, setState] = useState<NeteaseState>(EMPTY);
  const latestRef = useRef<NeteaseState>(EMPTY);
  const [, forceTick] = useState(0);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let timer: number | null = null;
    let disposed = false;

    const receiveSnapshot = (incoming: NeteaseState) => {
      const next = stabilizeSnapshot(latestRef.current, incoming);
      latestRef.current = next;
      setState(next);
    };

    void api.currentState().then((initial) => {
      if (disposed || !initial) return;
      receiveSnapshot(initial);
    });

    void listen<NeteaseState>("netease-state", (event) => {
      receiveSnapshot(event.payload);
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlisten = fn;
    });

    // 播放中按本地时钟插值，让进度平滑推进（仅推动渲染，不回写 Rust）
    timer = window.setInterval(() => {
      const latest = latestRef.current;
      if (!latest.isPlaying) return;
      const elapsed = Date.now() - latest.sampledAtEpochMs;
      if (elapsed <= 0 || elapsed > 5000) return;
      forceTick((n) => n + 1);
    }, INTERPOLATION_TICK_MS);

    return () => {
      disposed = true;
      if (timer !== null) window.clearInterval(timer);
      unlisten?.();
    };
  }, []);

  // 对外暴露插值后的进度
  const positionMs = positionAt(state, Date.now());

  return { state, positionMs };
}
