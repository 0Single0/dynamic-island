import { useMemo, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { Bot, Loader2, Square, PanelLeftClose, PanelLeftOpen } from "lucide-react";
import { Composer, type AgentComposerItem } from "@/components/agent/Composer";
import { AgentIcon, toAgentIconId } from "@/components/agent/AgentIcons";
import { ModelPicker } from "@/components/agent/ModelPicker";
import { useBuiltinPlugins } from "@/agent/plugins";
import { cn } from "@/lib/utils";
import { tauriInvoke } from "@/agent/api";
import { openSettingsWindow } from "@/api/window";
import { HistorySidebar } from "./HistorySidebar";
import { ErrorRow, MarkdownBody, ReasoningRow, ToolRow } from "./ConversationRows";
import { useAgentConversation } from "./useAgentConversation";
import { getDraft, setDraft, subscribeDrafts } from "@/agent/drafts";
import type { WidgetExpandedRenderProps } from "@/widgets/types";
import type { ConversationEntry } from "@/agent/trace";

export function AIExpanded({ headerSlot }: WidgetExpandedRenderProps = {}) {
  const { entries, sessions, sessionId, isLoading, models, providers, selectedModelId, setSelectedModelId, isSending, activeRunId, viewportRef, followTailRef, loadSession, createNewSession, deleteSession, sendMessage } = useAgentConversation();
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const draft = useSyncExternalStore(subscribeDrafts, () => getDraft(sessionId));
  const toggleSidebar = <button type="button" onClick={() => setSidebarCollapsed(value => !value)} title={sidebarCollapsed ? "展开历史对话" : "折叠历史对话"} aria-label={sidebarCollapsed ? "展开历史对话" : "折叠历史对话"} aria-expanded={!sidebarCollapsed} className="grid h-6 w-6 shrink-0 place-items-center rounded-md text-white/50 transition hover:bg-white/[0.08] hover:text-white">{sidebarCollapsed ? <PanelLeftOpen className="h-3.5 w-3.5" /> : <PanelLeftClose className="h-3.5 w-3.5" />}</button>;
  const { plugins } = useBuiltinPlugins();
  const composerPlugins = useMemo<AgentComposerItem[]>(() => plugins.map(plugin => ({
    id: plugin.id, label: plugin.name, description: plugin.description,
    icon: <AgentIcon id={toAgentIconId(plugin.icon)} />, group: "插件", pickerKind: "plugin",
  })), [plugins]);
  const runningEntry = useMemo(() => {
    if (!isSending) return undefined;
    return [...entries].reverse().find((entry): entry is ConversationEntry =>
      (!activeRunId || entry.runId === activeRunId) &&
      (entry.kind === "reasoning" || entry.kind === "tool" || entry.kind === "assistant") &&
      ["receiving", "pending", "running"].includes(entry.status ?? "running"),
    );
  }, [activeRunId, entries, isSending]);
  const runningLabel = runningEntry?.kind === "reasoning"
    ? "正在思考..."
    : runningEntry?.kind === "tool"
      ? runningEntry.status === "receiving" || runningEntry.status === "pending"
        ? `准备${runningEntry.title || "工具调用"}...`
        : `正在执行 ${runningEntry.title || "工具"}...`
      : runningEntry?.kind === "assistant"
        ? "正在生成回复..."
        : "正在处理...";

  return (
    <div className="flex h-full min-h-0 overflow-hidden rounded-lg" onClick={(event) => event.stopPropagation()}>
      <HistorySidebar
        sessions={sessions}
        activeSessionId={sessionId}
        collapsed={sidebarCollapsed}
        disabled={isSending}
        onSelect={(session) => { if (!isSending) void loadSession(session); }}
        onCreate={() => { if (!isSending) void createNewSession(); }}
        onDelete={(session) => { if (!isSending) void deleteSession(session); }}
      />
      <section className="flex min-w-0 flex-1 flex-col bg-white/[0.015]">
        <header className="flex h-10 shrink-0 items-center justify-between border-b border-white/[0.07] px-4">
          <div className="flex min-w-0 items-center gap-2">
            {headerSlot ? createPortal(toggleSidebar, headerSlot) : toggleSidebar}
            <div className="min-w-0 truncate text-[11px] font-medium text-white/[0.72]">{sessions.find((session) => session.id === sessionId)?.title || "新对话"}</div>
          </div>
        </header>
        {isSending && <div className="flex h-8 shrink-0 items-center gap-1.5 border-b border-white/[0.06] px-5 text-[10px] text-white/[0.48]" role="status" aria-live="polite"><Loader2 className="h-3 w-3 animate-spin text-sky-200/70" /><span>{runningLabel}</span></div>}
        <div ref={viewportRef} onScroll={(event) => {
          const node = event.currentTarget;
          followTailRef.current = node.scrollHeight - node.scrollTop - node.clientHeight < 90;
        }} className="min-h-0 flex-1 overflow-y-auto px-5 py-4 [scrollbar-color:rgba(255,255,255,.2)_transparent] [scrollbar-width:thin]">
          <div className="mx-auto flex w-full max-w-[720px] flex-col gap-1">
            {entries.length === 0 ? (
              <div className="flex min-h-[180px] flex-col items-center justify-center text-center">
                <div className="grid h-9 w-9 place-items-center rounded-xl bg-white/[0.07] text-sky-200/[0.85]"><Bot className="h-5 w-5" /></div>
                <p className="mt-3 text-xs text-white/[0.5]">直接说你想完成什么</p>
              </div>
            ) : entries.map((entry) => {
              if (entry.kind === "reasoning") return <ReasoningRow key={entry.id} entry={entry} />;
              if (entry.kind === "tool") return <ToolRow key={entry.id} entry={entry} />;
              if (entry.kind === "error") return <ErrorRow key={entry.id} entry={entry} />;
              return <div key={entry.id} className={cn("flex py-1.5", entry.kind === "user" ? "justify-end" : "justify-start")}><div className={cn("max-w-[92%] min-w-0", entry.kind === "user" ? "whitespace-pre-wrap break-words rounded-xl bg-white/[0.09] px-3 py-2 text-[12px] leading-[1.75] text-white/[0.88]" : "px-1")}>{entry.kind === "user" ? entry.text : <MarkdownBody text={entry.text} />}</div></div>;
            })}
          </div>
        </div>
        <div className="shrink-0 px-4 pb-3 pt-1">
          <Composer
            key={sessionId}
            document={draft}
            onDocumentChange={document => setDraft(sessionId, document)}
            disabled={isSending || isLoading}
            plugins={composerPlugins}
            placeholder="输入消息，使用 @ 选择插件"
            footer={(
              <div className="flex min-w-0 items-center gap-1.5 text-[10px] text-zinc-500">
                <ModelPicker providers={providers} models={models} value={selectedModelId} onChange={setSelectedModelId} onManage={() => void openSettingsWindow("ai-models")} className="max-w-[240px] py-1 text-[11px]" aria-label="选择模型" />
                {activeRunId && <button type="button" onClick={() => void tauriInvoke("agent_cancel_run", { runId: activeRunId })} className="ml-2 flex items-center gap-1 rounded-md px-2 py-1 text-[10px] text-zinc-400 hover:bg-white/[0.07] hover:text-zinc-200"><Square className="h-3 w-3 fill-current" /> 停止</button>}
              </div>
            )}
            onSubmit={(document) => { void sendMessage(document); }}
          />
        </div>
      </section>
    </div>
  );
}
