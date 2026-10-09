import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { JSONContent } from "@tiptap/core";
import { documentToPlainText, documentToPluginIds, deleteSession as deleteStoredSession, listExecutionEvents, tauriInvoke, type ExecutionEvent, type MessageRecord, type ModelRecord, type ProviderError, type SessionRecord, type TimelineRecord } from "@/agent/api";
import { emptyTrace, reduceTrace, type TraceState } from "@/agent/trace";
import { toast } from "@/utils/toast";
import { legacyEntries } from "./legacy";
import { clearDraft, getActiveConversation, NEW_CONVERSATION, promoteDraft, setActiveConversation, setDraft } from "@/agent/drafts";

function asProviderError(value: unknown): ProviderError {
  if (typeof value === "object" && value !== null && "message" in value) return value as ProviderError;
  return { category: "CLIENT", message: typeof value === "string" ? value : "无法完成请求", protocol: "", model: "", endpoint: "" };
}

export function useAgentConversation() {
  const [trace, setTrace] = useState<TraceState>(emptyTrace);
  const [sessions, setSessions] = useState<SessionRecord[]>([]);
  const [sessionId, setSessionId] = useState(NEW_CONVERSATION);
  const [isLoading, setIsLoading] = useState(true);
  const [models, setModels] = useState<ModelRecord[]>([]);
  const [selectedModelId, setSelectedModelId] = useState("");
  const [isSending, setIsSending] = useState(false);
  const [activeRunId, setActiveRunId] = useState<string | null>(null);
  const state = useRef(trace);
  const currentSession = useRef(NEW_CONVERSATION);
  const activeRun = useRef<string | null>(null);
  const sending = useRef(false);
  const loading = useRef(false);
  const loadVersion = useRef(0);
  const mounted = useRef(true);
  const queue = useRef<ExecutionEvent[]>([]);
  const frame = useRef<number | null>(null);
  const listenerReady = useRef<Promise<void> | null>(null);
  const invokedRun = useRef<string | null>(null);
  const viewportRef = useRef<HTMLDivElement>(null);
  const followTailRef = useRef(true);

  const apply = useCallback((events: ExecutionEvent[]) => {
    state.current = reduceTrace(state.current, events);
    if (!mounted.current) return;
    setTrace(state.current);
    if (activeRun.current && invokedRun.current !== activeRun.current && state.current.terminalRuns.has(activeRun.current)) {
      activeRun.current = null; sending.current = false;
      setActiveRunId(null); setIsSending(false);
    }
  }, []);

  const flush = useCallback(() => {
    if (frame.current !== null) cancelAnimationFrame(frame.current);
    frame.current = null;
    const events = queue.current; queue.current = [];
    apply(events);
  }, [apply]);

  const catchUp = useCallback(async (id: string) => {
    const events = await listExecutionEvents(id, state.current.lastSeq);
    if (mounted.current && currentSession.current === id && !loading.current) apply(events);
  }, [apply]);

  const loadSession = useCallback(async (session: SessionRecord) => {
    if (sending.current) return;
    const version = ++loadVersion.current;
    loading.current = true; setIsLoading(true);
    if (frame.current !== null) cancelAnimationFrame(frame.current);
    frame.current = null; queue.current = [];
    currentSession.current = session.id;
    setSessionId(session.id);
    setActiveConversation(session.id);
    try {
      const [events, timeline, messages] = await Promise.all([
        listExecutionEvents(session.id),
        tauriInvoke<TimelineRecord[]>("agent_list_timeline", { sessionId: session.id }),
        tauriInvoke<MessageRecord[]>("agent_list_messages", { sessionId: session.id }),
      ]);
      if (!mounted.current || currentSession.current !== session.id || loadVersion.current !== version) return;
      const newIds = new Set(events.flatMap(e => typeof e.payload.messageId === "string" ? [e.payload.messageId] : []));
      const legacy = legacyEntries(timeline, messages.filter(m => !newIds.has(m.id)));
      state.current = reduceTrace(emptyTrace(legacy), events);
      loading.current = false; setIsLoading(false);
      flush();
      const pendingRun = [...events].reverse().find(e => e.eventType === "run_started" && !state.current.terminalRuns.has(e.runId));
      activeRun.current = pendingRun?.runId || null;
      sending.current = Boolean(pendingRun);
      setActiveRunId(activeRun.current); setIsSending(sending.current);
      followTailRef.current = true;
    } catch (error) {
      if (!mounted.current || loadVersion.current !== version) return;
      loading.current = false; setIsLoading(false);
      const failure = asProviderError(error);
      state.current = emptyTrace([{ kind: "error", id: "history-error", text: failure.message, error: failure, position: 0 }]);
      setTrace(state.current);
    }
  }, [flush]);

  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    let stop: (() => void) | undefined;
    listenerReady.current = listen<ExecutionEvent>("agent://trace", ({ payload }) => {
      if (disposed) return;
      if (payload.sessionId !== currentSession.current) return;
      queue.current.push(payload);
      if (!loading.current && frame.current === null) frame.current = requestAnimationFrame(flush);
    }).then(unlisten => { if (!disposed) stop = unlisten; else unlisten(); });
    void listenerReady.current.then(async () => {
      if (disposed) return;
      const [storedSessions, modelRecords] = await Promise.all([
        tauriInvoke<SessionRecord[]>("agent_list_sessions"), tauriInvoke<ModelRecord[]>("agent_list_model_configs"),
      ]);
      if (disposed) return;
      setSessions(storedSessions); setModels(modelRecords.filter(m => m.enabled));
      setSelectedModelId(modelRecords.find(m => m.enabled)?.id || "");
      const active = getActiveConversation();
      const session = storedSessions.find(s => s.id === active);
      if (session) await loadSession(session);
      else { loading.current = false; setIsLoading(false); setActiveConversation(NEW_CONVERSATION); }
    }).catch(error => {
      if (disposed) return;
      loading.current = false; setIsLoading(false);
      const failure = asProviderError(error);
      state.current = emptyTrace([{ kind: "error", id: "load-error", text: failure.message, error: failure, position: 0 }]);
      setTrace(state.current);
    });
    const poll = window.setInterval(() => {
      const id = currentSession.current;
      if (id !== NEW_CONVERSATION && !loading.current && (sending.current || state.current.pending.size > 0)) void catchUp(id).catch(() => undefined);
    }, 1000);
    return () => {
      disposed = true; mounted.current = false; stop?.(); window.clearInterval(poll);
      if (frame.current !== null) cancelAnimationFrame(frame.current);
    };
  }, [catchUp, flush, loadSession]);

  useEffect(() => {
    if (followTailRef.current && viewportRef.current) viewportRef.current.scrollTop = viewportRef.current.scrollHeight;
  }, [trace]);

  const createNewSession = async () => {
    if (sending.current) return;
    ++loadVersion.current;
    if (frame.current !== null) cancelAnimationFrame(frame.current);
    frame.current = null; queue.current = [];
    currentSession.current = NEW_CONVERSATION; setSessionId(NEW_CONVERSATION); setActiveConversation(NEW_CONVERSATION);
    state.current = emptyTrace(); setTrace(state.current);
    loading.current = false; setIsLoading(false); followTailRef.current = true;
  };

  const deleteSession = async (session: SessionRecord) => {
    if (sending.current) return;
    try {
      await deleteStoredSession(session.id);
    } catch (error) {
      toast.error(asProviderError(error).message);
      return;
    }
    clearDraft(session.id);
    if (!mounted.current) return;
    setSessions(current => current.filter(item => item.id !== session.id));
    // 删掉的是当前会话时要立刻离开它，否则界面还停留在已不存在的历史上。
    if (currentSession.current === session.id) await createNewSession();
  };

  const sendMessage = async (document: JSONContent) => {
    const content = documentToPlainText(document);
    let id = currentSession.current;
    if (!content || sending.current || loading.current) return;
    const runId = `run-${crypto.randomUUID()}`;
    const model = models.find(m => m.id === selectedModelId);
    activeRun.current = runId; invokedRun.current = runId; sending.current = true;
    setIsSending(true); setActiveRunId(runId); followTailRef.current = true;
    try {
      await listenerReady.current;
      if (id === NEW_CONVERSATION) {
        const session = await tauriInvoke<SessionRecord>("agent_create_session", { title: "新对话" });
        id = session.id; promoteDraft(id);
        currentSession.current = id;
        if (mounted.current) { setSessionId(id); setSessions(current => [session, ...current]); }
      }
      state.current = { ...state.current, entries: [...state.current.entries, { kind: "user", id: `local-user-${runId}`, runId, text: content, position: state.current.lastSeq + 1 }] };
      if (mounted.current) setTrace(state.current);
      await tauriInvoke("agent_send_message", { input: { sessionId: id, content, pluginIds: documentToPluginIds(document), providerId: model?.providerId, modelId: model?.id, runId } });
    } catch (error) {
      flush();
      if (id !== NEW_CONVERSATION) await catchUp(id).catch(() => undefined);
      else setDraft(NEW_CONVERSATION, document);
      if (!state.current.terminalRuns.has(runId)) {
        const failure = asProviderError(error);
        state.current = { ...state.current, entries: [...state.current.entries, { kind: "error", id: `${runId}:error`, runId, text: failure.status ? `HTTP ${failure.status}: ${failure.message}` : failure.message, error: failure, position: state.current.lastSeq + 1 }] };
        if (mounted.current) setTrace(state.current);
      }
    } finally {
      flush();
      if (id !== NEW_CONVERSATION) await catchUp(id).catch(() => undefined);
      invokedRun.current = null; activeRun.current = null; sending.current = false;
      if (mounted.current) {
        setIsSending(false); setActiveRunId(null);
        void tauriInvoke<SessionRecord[]>("agent_list_sessions").then(setSessions).catch(() => undefined);
      }
    }
  };

  return { entries: trace.entries, sessions, sessionId, isLoading, models, selectedModelId, setSelectedModelId, isSending, activeRunId, viewportRef, followTailRef, loadSession, createNewSession, deleteSession, sendMessage };
}
