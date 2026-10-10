import { useCallback, useEffect, useRef, useState } from "react";
import * as api from "@/api/gateway";
import type { GatewayAccount, GatewayChannel, GatewayStatus } from "@/api/gateway";

/** 面板轮询间隔：状态是"看着不动"的数字，2 秒足够，也不至于把 CPU 拉满。 */
const POLL_MS = 2000;

/**
 * 反代号池状态。
 *
 * 走 Tauri 命令轮询（不是事件推送）：面板只在展开时轮询，收起即停，
 * 换来的是 Rust 侧不用为面板维护事件通道。浏览器预览模式下给出明确提示。
 */
export function useGatewayState(active: boolean) {
  const [status, setStatus] = useState<GatewayStatus | null>(null);
  const [channels, setChannels] = useState<GatewayChannel[]>([]);
  const [accounts, setAccounts] = useState<GatewayAccount[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const mounted = useRef(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const [nextStatus, nextChannels, nextAccounts] = await Promise.all([
        api.status(),
        api.listChannels(),
        api.listAccounts(),
      ]);
      if (!mounted.current) return;
      setStatus(nextStatus);
      setChannels(nextChannels);
      setAccounts(nextAccounts);
      setError(null);
    } catch (reason) {
      if (!mounted.current) return;
      const message = reason instanceof Error ? reason.message : String(reason);
      setError(
        message.includes("__TAURI") || message.includes("undefined")
          ? "浏览器预览模式：反代号池需要桌面端运行"
          : message,
      );
    } finally {
      if (mounted.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    if (!active) {
      return () => {
        mounted.current = false;
      };
    }
    void refresh();
    const timer = window.setInterval(() => void refresh(), POLL_MS);
    return () => {
      mounted.current = false;
      window.clearInterval(timer);
    };
  }, [active, refresh]);

  return { status, channels, accounts, error, loading, refresh };
}
