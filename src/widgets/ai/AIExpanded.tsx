import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { JSONContent } from "@tiptap/core";
import { Check, ChevronDown, ChevronRight, Circle, Loader2, Square, TerminalSquare } from "lucide-react";
import { Composer, type AgentComposerItem } from "@/components/agent/Composer";
import { AgentIcon } from "@/components/agent/AgentIcons";
import { builtinPluginRegistry } from "@/agent/plugins";
import { Select } from "@/components/common/Select";
import { cn } from "@/lib/utils";
import {
  documentToPlainText,
  documentToPluginIds,
  tauriInvoke,
  type MessageRecord,
  type ModelRecord,
} from "@/agent/api";

interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  text: string;
}

interface AgentStreamPayload {
  type?: string;
  run_id?: string;
  delta?: string;
  message?: string;
  phase?: string;
  title?: string;
  detail?: string;
}

interface RunStep {
  id: string;
  kind: "thinking" | "tool" | "result";
  title: string;
  detail?: string;
  status: "running" | "completed" | "failed";
}

function RunActivity({ steps, running }: { steps: RunStep[]; running: boolean }) {
  const [openSteps, setOpenSteps] = useState<Record<string, boolean>>({});
  if (!steps.length) return null;
  const completed = !running && steps.every((step) => step.status === "completed");

  const toggleStep = (id: string) => {
    setOpenSteps((current) => ({ ...current, [id]: !current[id] }));
  };

  const getStepIcon = (step: RunStep) => {
    if (step.status === "running") return Loader2;
    if (step.status === "failed") return Circle;
    if (step.kind === "tool") return TerminalSquare;
    return Check;
  };

  return (
    <div className="my-1 w-full max-w-[620px] text-white/[0.56]">
      <div className="flex min-h-6 items-center gap-2 px-0 py-0.5 text-left">
        <span className={cn(
          "h-[7px] w-[7px] rounded-full border bg-transparent",
          running && "border-sky-300/95 shadow-[0_0_0_3px_rgba(125,211,252,.1)]",
          completed && "border-emerald-300/65 bg-emerald-300/70",
          !running && !completed && "border-rose-300/75 bg-rose-300/70",
        )} />
        <span className="text-[10px] text-white/[0.52]">{running ? "正在处理" : completed ? "已完成" : "运行结束"}</span>
      </div>
      <div className="grid gap-0 pb-0.5 pl-px">
        {steps.map((step) => {
          const Icon = getStepIcon(step);
          const expandable = Boolean(step.detail);
          const expanded = Boolean(openSteps[step.id]);
          return (
            <div key={step.id} className="flex min-h-6 items-start gap-2 py-0.5">
              <div className="grid w-4 shrink-0 place-items-center pt-0.5"><Icon className={`h-3.5 w-3.5 shrink-0 ${step.status === "running" ? "animate-spin text-sky-300" : step.status === "failed" ? "text-rose-300" : "text-zinc-500"}`} /></div>
              {expandable ? (
                <button type="button" className="inline-flex min-w-0 items-center gap-[5px] bg-transparent p-0 text-left" onClick={() => toggleStep(step.id)}>
                  <span className="min-w-0 overflow-hidden text-ellipsis whitespace-nowrap text-[11px] leading-[1.45] text-white/[0.53] hover:text-white/[0.76]">{step.title}</span>
                  {expanded ? <ChevronDown className="h-3 w-3 shrink-0 text-white/[0.34]" /> : <ChevronRight className="h-3 w-3 shrink-0 text-white/[0.34]" />}
                  {expanded && <span className="max-w-[380px] overflow-hidden text-ellipsis whitespace-nowrap text-[10px] text-white/[0.35]">{step.detail}</span>}
                </button>
              ) : (
                <span className="min-w-0 overflow-hidden text-ellipsis whitespace-nowrap text-[11px] leading-[1.45] text-white/[0.53]">{step.title}</span>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function AIExpanded() {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [models, setModels] = useState<ModelRecord[]>([]);
  const [selectedModelId, setSelectedModelId] = useState("");
  const [isSending, setIsSending] = useState(false);
  const [activeRunId, setActiveRunId] = useState<string | null>(null);
  const [runSteps, setRunSteps] = useState<RunStep[]>([]);
  const composerPlugins = useMemo<AgentComposerItem[]>(
    () => builtinPluginRegistry.map((plugin) => ({
      id: plugin.id,
      label: plugin.name,
      description: plugin.description,
      icon: <AgentIcon id={plugin.icon} />,
      group: "插件",
      pickerKind: "plugin",
    })),
    [],
  );

  useEffect(() => {
    let active = true;
    void Promise.all([
      tauriInvoke<{ id: string; title: string }[]>("agent_list_sessions"),
      tauriInvoke<ModelRecord[]>("agent_list_model_configs"),
    ]).then(async ([sessions, modelRecords]) => {
      const session = sessions[0] ?? await tauriInvoke<{ id: string }>("agent_create_session", { title: "桌面 Agent" });
      if (!active) return;
      setSessionId(session.id);
      const enabledModels = modelRecords.filter((model) => model.enabled);
      setModels(enabledModels);
      setSelectedModelId(enabledModels[0]?.id ?? "");
      const stored = await tauriInvoke<MessageRecord[]>("agent_list_messages", { sessionId: session.id });
      if (active && stored.length) {
        setMessages(stored
          .filter((message) => message.role === "user" || message.role === "assistant")
          .map((message) => ({
            id: message.id,
            role: message.role === "user" ? "user" : "assistant",
            text: message.content,
          })));
      }
    }).catch(() => undefined);
    return () => { active = false; };
  }, []);

  useEffect(() => {
    let stop: (() => void) | undefined;
    void listen<AgentStreamPayload>("agent://event", ({ payload }) => {
      if (!payload.run_id) return;
      if (payload.type === "message_delta" && payload.delta) {
        const delta = payload.delta;
        setMessages((current) => {
          const id = `stream-${payload.run_id}`;
          const existing = current.find((message) => message.id === id);
          if (existing) return current.map((message) => message.id === id ? { ...message, text: `${message.text}${delta}` } : message);
          return [...current, { id, role: "assistant", text: delta }];
        });
      }
      if (payload.type === "run_status" && payload.phase && payload.title) {
        setRunSteps((current) => {
          const id = `${payload.run_id}-${payload.phase}`;
          const existing = current.find((step) => step.id === id);
          const nextStep: RunStep = { id, kind: payload.phase === "tool" ? "tool" : "thinking", title: payload.title ?? "处理中", detail: payload.detail, status: "running" };
          return existing ? current.map((step) => step.id === id ? { ...step, ...nextStep } : step) : [...current, nextStep];
        });
      }
      if (payload.type === "run_completed") setRunSteps((current) => current.map((step) => ({ ...step, status: "completed" })));
      if (payload.type === "run_failed" || payload.type === "run_cancelled") setRunSteps((current) => current.map((step) => ({ ...step, status: "failed" })));
    }).then((unlisten) => { stop = unlisten; }).catch(() => undefined);
    return () => { stop?.(); };
  }, []);

  const sendMessage = async (document: JSONContent) => {
    const text = documentToPlainText(document);
    if (!text || isSending) return;
    const runId = `run-${Date.now()}`;
    const selectedModel = models.find((model) => model.id === selectedModelId);
    const pluginIds = documentToPluginIds(document);
    const selectedPluginNames = pluginIds.map((id) => builtinPluginRegistry.find((plugin) => plugin.id === id)?.name).filter(Boolean) as string[];
    setMessages((current) => [...current, { id: `user-${Date.now()}`, role: "user", text }]);
    setIsSending(true);
    setActiveRunId(runId);
    setRunSteps([
      { id: `${runId}-thinking`, kind: "thinking", title: "分析请求", detail: "整理上下文并准备执行", status: "running" },
      ...(selectedPluginNames.length ? [{ id: `${runId}-tool`, kind: "tool" as const, title: "已选择插件", detail: selectedPluginNames.join("、"), status: "running" as const }] : []),
    ]);
    try {
      let currentSessionId = sessionId;
      if (!currentSessionId) {
        const session = await tauriInvoke<{ id: string }>("agent_create_session", { title: text.slice(0, 32) });
        currentSessionId = session.id;
        setSessionId(currentSessionId);
      }
      const result = await tauriInvoke<{ message: MessageRecord }>("agent_send_message", {
        input: {
          sessionId: currentSessionId,
          content: text,
          pluginIds,
          providerId: selectedModel?.providerId,
          modelId: selectedModel?.id,
          runId,
        },
      });
      setMessages((current) => [
        ...current.filter((message) => message.id !== `stream-${runId}`),
        { id: result.message.id, role: "assistant", text: result.message.content },
      ]);
      setRunSteps((current) => current.map((step) => ({ ...step, status: "completed" })));
    } catch {
      setMessages((current) => [
        ...current.filter((message) => message.id !== `stream-${runId}`),
        { id: `assistant-${Date.now()}`, role: "assistant", text: "当前未连接到可用模型，请先在设置中心配置供应商和模型。" },
      ]);
      setRunSteps((current) => current.map((step) => ({ ...step, status: "failed", detail: step.id.endsWith("thinking") ? "运行失败，请检查模型或供应商配置" : step.detail })));
    } finally {
      setIsSending(false);
      setActiveRunId(null);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-2" onClick={(event) => event.stopPropagation()}>
      <div className="min-h-0 flex-1 overflow-y-auto px-1 py-1 [scrollbar-color:rgba(255,255,255,.2)_transparent] [scrollbar-width:thin]">
        <div className="space-y-3">
      {messages.map((message) => (
            <div key={message.id} className={`flex ${message.role === "user" ? "justify-end" : "justify-start"}`}>
              <div className={`max-w-[88%] text-xs leading-5 ${message.role === "user" ? "rounded-xl bg-sky-400/14 px-3 py-2 text-sky-50" : "whitespace-pre-wrap px-1 py-1 text-zinc-300"}`}>{message.text}</div>
            </div>
          ))}
          <RunActivity steps={runSteps} running={isSending} />
        </div>
      </div>
      <Composer
        plugins={composerPlugins}
        placeholder="输入消息，使用 @ 选择插件"
        footer={(
          <div className="flex min-w-0 items-center gap-1.5 text-[10px] text-zinc-500">
            <span className="shrink-0">模型</span>
            <div className="relative min-w-0">
              <Select
                value={selectedModelId}
                options={models.map((model) => ({ value: model.id, label: model.displayName || model.modelId, description: model.modelId }))}
                onChange={setSelectedModelId}
                placeholder="未配置模型"
                className="max-w-[220px] py-1 text-[10px]"
                aria-label="选择模型"
              />
            </div>
            {activeRunId && (
              <button
                type="button"
                onClick={() => void tauriInvoke("agent_cancel_run", { runId: activeRunId })}
                className="ml-2 flex items-center gap-1 rounded-md px-2 py-1 text-[10px] text-zinc-400 hover:bg-white/[0.07] hover:text-zinc-200"
              >
                <Square className="h-3 w-3 fill-current" /> 停止
              </button>
            )}
          </div>
        )}
        onSubmit={(document) => { void sendMessage(document); }}
      />
    </div>
  );
}
