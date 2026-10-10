import { useCallback, useEffect, useRef, useState } from "react";
import { X } from "lucide-react";

import * as api from "@/api/gateway";
import type { GatewayChannel, GatewayLoginTicket } from "@/api/gateway";
import { toast } from "@/utils/toast";

/**
 * 添加账号弹窗：挑渠道 → 浏览器授权 → 自动入库。
 *
 * 成功后**不关闭**弹窗，方便连着加第二个渠道的账号；右下角是"完成"。
 */
export function AddAccountDialog({
  channels,
  onClose,
  onAdded,
}: {
  channels: GatewayChannel[];
  onClose: () => void;
  onAdded: () => void;
}) {
  const [pending, setPending] = useState<{ channelId: string; ticket: GatewayLoginTicket } | null>(
    null,
  );
  const [hint, setHint] = useState<string | null>(null);
  const [added, setAdded] = useState<string[]>([]);
  const pollTimer = useRef<number | null>(null);

  const stopPolling = useCallback(() => {
    if (pollTimer.current !== null) {
      window.clearInterval(pollTimer.current);
      pollTimer.current = null;
    }
  }, []);
  useEffect(() => stopPolling, [stopPolling]);

  const start = async (channelId: string) => {
    try {
      const ticket = await api.loginStart(channelId);
      setPending({ channelId, ticket });
      setHint("在浏览器里完成登录授权，这里会自动完成入库与签到");
      stopPolling();
      pollTimer.current = window.setInterval(async () => {
        try {
          const result = await api.loginPoll(channelId, ticket.state);
          if (result.status === "pending") return;
          stopPolling();
          if (result.status === "ok") {
            const name = result.account?.nickname ?? result.account?.uid ?? "";
            setAdded((previous) => [...previous, `${name}（${channelId}）`]);
            setPending(null);
            setHint(null);
            toast.success(`已加入账号：${name}`);
            onAdded();
          } else {
            setHint(result.message ?? "授权失败");
          }
        } catch (reason) {
          stopPolling();
          setHint(reason instanceof Error ? reason.message : "轮询失败");
        }
      }, 2500);
    } catch (reason) {
      setHint(reason instanceof Error ? reason.message : "无法开始登录");
    }
  };

  const enabled = channels.filter((channel) => channel.enabled);

  return (
    <div
      className="absolute inset-0 z-50 flex items-center justify-center bg-black/65 p-8"
      onClick={(event) => {
        event.stopPropagation();
        stopPolling();
        onClose();
      }}
    >
      <div
        className="w-full max-w-lg rounded-2xl border border-white/12 bg-zinc-900 p-4 shadow-2xl"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="flex items-center gap-2">
          <span className="text-[13px] font-medium text-zinc-100">添加账号</span>
          <span className="text-[10px] text-zinc-500">选一个渠道发起授权</span>
          <button
            type="button"
            onClick={() => {
              stopPolling();
              onClose();
            }}
            className="ml-auto flex h-6 w-6 items-center justify-center rounded-md text-zinc-400 hover:bg-white/12 hover:text-zinc-100"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </header>

        <div className="mt-3 flex flex-wrap gap-1.5">
          {enabled.map((channel) => (
            <button
              key={channel.id}
              type="button"
              disabled={pending !== null}
              onClick={() => void start(channel.id)}
              className={`rounded-lg border px-3 py-1.5 text-[11px] transition-colors ${
                pending?.channelId === channel.id
                  ? "border-sky-400/40 bg-sky-400/15 text-sky-100"
                  : "border-white/12 bg-white/[0.04] text-zinc-200 hover:bg-white/10 disabled:opacity-40"
              }`}
            >
              {channel.display_name || channel.id}
            </button>
          ))}
          {enabled.length === 0 && (
            <p className="text-[11px] text-zinc-500">
              没有启用的渠道：到「设置」里添加渠道配置后再来。
            </p>
          )}
        </div>

        {pending && (
          <div className="mt-3 rounded-lg border border-sky-400/25 bg-sky-400/8 px-2.5 py-2">
            <p className="text-[11px] text-sky-100">{hint}</p>
            <button
              type="button"
              onClick={() => {
                void navigator.clipboard?.writeText(pending.ticket.auth_url);
                toast.success("授权链接已复制");
              }}
              className="mt-1.5 block max-w-full truncate rounded-md bg-black/30 px-2 py-1 text-left font-mono text-[10px] text-sky-100/85"
            >
              {pending.ticket.auth_url}
            </button>
            <p className="mt-1 text-[10px] text-sky-200/70">
              正在等待授权（每 2.5 秒检查一次）…
            </p>
          </div>
        )}

        {added.length > 0 && (
          <div className="mt-3 rounded-lg border border-emerald-400/25 bg-emerald-400/8 px-2.5 py-2">
            <p className="text-[11px] text-emerald-100">本次新增 {added.length} 个账号</p>
            {added.map((entry) => (
              <div key={entry} className="text-[10px] text-emerald-200/80">
                {entry}
              </div>
            ))}
          </div>
        )}

        <footer className="mt-4 flex items-center gap-2">
          <span className="text-[10px] text-zinc-500">
            可以连续添加多个渠道；关闭弹窗前请先等授权完成
          </span>
          <button
            type="button"
            onClick={() => {
              stopPolling();
              onClose();
            }}
            className="ml-auto rounded-md bg-white/10 px-3 py-1 text-[11px] text-zinc-100 hover:bg-white/16"
          >
            完成
          </button>
        </footer>
      </div>
    </div>
  );
}
