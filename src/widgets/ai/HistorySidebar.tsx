import { useEffect, useState } from "react";
import { Check, Clock3, MessageSquare, MessageSquarePlus, Trash2, X } from "lucide-react";
import { motion, useReducedMotion } from "framer-motion";
import type { SessionRecord } from "@/agent/api";
import { cn } from "@/lib/utils";
function sessionTitle(session: SessionRecord) {
  return session.title.trim() || "新对话";
}

function shortDate(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  const now = new Date();
  if (date.toDateString() === now.toDateString()) {
    return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  return date.toLocaleDateString([], { month: "numeric", day: "numeric" });
}

export function HistorySidebar({
  sessions,
  activeSessionId,
  collapsed,
  disabled,
  onSelect,
  onCreate,
  onDelete,
}: {
  sessions: SessionRecord[];
  activeSessionId: string | null;
  collapsed: boolean;
  disabled?: boolean;
  onSelect: (session: SessionRecord) => void;
  onCreate: () => void;
  onDelete: (session: SessionRecord) => void;
}) {
  const reducedMotion = useReducedMotion();
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  // 会话在别处消失（或运行期间被禁用）时收起确认态，避免停在无效的二次确认上。
  useEffect(() => {
    setPendingDeleteId(current => current && !disabled && sessions.some(session => session.id === current) ? current : null);
  }, [disabled, sessions]);
  return (
    <motion.aside initial={false} animate={{ width: collapsed ? 0 : 184, opacity: collapsed ? 0 : 1 }} transition={{ duration: reducedMotion ? 0 : 0.22, ease: [0.22, 1, 0.36, 1] }}
      aria-hidden={collapsed} inert={collapsed} className="shrink-0 overflow-hidden bg-black/[0.1]">
    <div className="flex h-full w-[184px] flex-col border-r border-white/[0.08] px-2.5 py-3">
      <div className="flex h-5 shrink-0 items-center justify-between px-1 text-[10px] text-white/[0.34]">
        <div className="flex items-center gap-1.5"><Clock3 className="h-3 w-3" /><span>历史对话</span></div>
        <button type="button" onClick={onCreate} title="新建对话" aria-label="新建对话" className="grid h-5 w-5 place-items-center rounded-md text-white/[0.48] transition hover:bg-white/[0.08] hover:text-white"><MessageSquarePlus className="h-3 w-3" /></button>
      </div>
      <div className="mt-1 min-h-0 flex-1 space-y-0.5 overflow-y-auto pr-0.5 [scrollbar-color:rgba(255,255,255,.18)_transparent] [scrollbar-width:thin]">
        {sessions.length ? sessions.map((session) => {
          const title = sessionTitle(session);
          if (pendingDeleteId === session.id) {
            return (
              <div key={session.id} className="flex items-center gap-1 rounded-lg bg-rose-400/[0.12] px-2 py-2">
                <span className="min-w-0 flex-1 truncate text-[10px] text-rose-100/[0.85]">删除这条对话？</span>
                <button type="button" onClick={() => { setPendingDeleteId(null); onDelete(session); }} title="确认删除" aria-label={`确认删除 ${title}`} className="grid h-5 w-5 shrink-0 place-items-center rounded-md text-rose-100/80 transition hover:bg-rose-400/25 hover:text-white"><Check className="h-3 w-3" /></button>
                <button type="button" onClick={() => setPendingDeleteId(null)} title="取消删除" aria-label="取消删除" className="grid h-5 w-5 shrink-0 place-items-center rounded-md text-white/[0.45] transition hover:bg-white/[0.08] hover:text-white"><X className="h-3 w-3" /></button>
              </div>
            );
          }
          return (
            <div key={session.id} className="group relative">
              <button
                type="button"
                onClick={() => onSelect(session)}
                className={cn(
                  "flex w-full min-w-0 items-center gap-2 rounded-lg py-2 pl-2 pr-8 text-left transition-colors",
                  session.id === activeSessionId ? "bg-white/[0.1] text-white/[0.9]" : "text-white/[0.53] hover:bg-white/[0.06] hover:text-white/[0.78]",
                )}
              >
                <span className="grid h-5 w-5 shrink-0 place-items-center rounded-md bg-white/[0.06] text-white/[0.38]"><MessageSquare className="h-3 w-3" /></span>
                <span className="min-w-0 flex-1 truncate text-[10px]">{title}</span>
              </button>
              <span aria-hidden className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 text-[9px] text-white/[0.24] transition-opacity group-hover:opacity-0 group-focus-within:opacity-0">{shortDate(session.updatedAt)}</span>
              <button
                type="button"
                onClick={() => setPendingDeleteId(session.id)}
                disabled={disabled}
                title="删除对话"
                aria-label={`删除对话 ${title}`}
                className="absolute right-1.5 top-1/2 grid h-5 w-5 -translate-y-1/2 place-items-center rounded-md text-white/[0.42] opacity-0 transition hover:bg-rose-400/20 hover:text-rose-100 focus-visible:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100 disabled:hidden"
              >
                <Trash2 className="h-3 w-3" />
              </button>
            </div>
          );
        }) : (
          <div className="px-2 py-4 text-[10px] leading-5 text-white/[0.28]">还没有历史对话</div>
        )}
      </div>
    </div>
    </motion.aside>
  );
}
