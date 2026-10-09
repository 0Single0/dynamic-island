import type { ExecutionEvent, ProviderError } from "./api";

export type TraceStatus = "receiving" | "pending" | "running" | "completed" | "failed" | "cancelled" | "interrupted";
interface EntryBase { id: string; runId?: string; position: number; createdAt?: string; status?: TraceStatus }
export type ConversationEntry =
  | (EntryBase & { kind: "user" | "assistant"; text: string })
  | (EntryBase & { kind: "reasoning"; title: string; detail?: string; status: TraceStatus })
  | (EntryBase & { kind: "tool"; title: string; name?: string; arguments?: string; callId?: string; parentCallId?: string; output?: unknown; summary?: string; detail?: string; status: TraceStatus })
  | (EntryBase & { kind: "error"; text: string; error?: ProviderError });

export interface TraceState {
  entries: ConversationEntry[];
  lastSeq: number;
  pending: Map<number, ExecutionEvent>;
  seen: Set<string>;
  terminalRuns: Set<string>;
}

export function emptyTrace(legacy: ConversationEntry[] = []): TraceState {
  return { entries: legacy, lastSeq: 0, pending: new Map(), seen: new Set(), terminalRuns: new Set() };
}

function project(entries: ConversationEntry[], event: ExecutionEvent): ConversationEntry[] {
  const { payload: p, blockId, runId, seq, eventType, createdAt } = event;
  const base = { id: blockId || event.id, runId, position: seq, createdAt };
  const text = typeof p.text === "string" ? p.text : "";
  if (eventType === "user_message") return [...entries.filter(e => e.id !== `local-user-${runId}`), { ...base, kind: "user", text }];
  if (eventType === "block_started") {
    if (!blockId || entries.some(e => e.id === blockId)) return entries;
    if (p.kind === "tool") return [...entries, { ...base, kind: "tool", title: "准备工具调用", arguments: "", status: "receiving" }];
    if (p.kind === "reasoning") return [...entries, { ...base, kind: "reasoning", title: "思考", detail: "", status: "running" }];
    return [...entries, { ...base, kind: "assistant", text: "", status: "running" }];
  }
  if (eventType === "text_delta" || eventType === "reasoning_delta") {
    return entries.map(e => e.id !== blockId ? e : e.kind === "assistant" ? { ...e, text: e.text + text } : e.kind === "reasoning" ? { ...e, detail: (e.detail || "") + text } : e);
  }
  if (eventType === "tool_delta" || eventType === "tool_started" || eventType === "tool_completed") {
    return entries.map(e => e.id !== blockId || e.kind !== "tool" ? e : {
      ...e,
      name: typeof p.name === "string" ? p.name : e.name,
      title: typeof p.title === "string" ? p.title : typeof p.name === "string" ? p.name : e.title,
      callId: event.callId || e.callId,
      parentCallId: event.parentCallId || e.parentCallId,
      arguments: typeof p.arguments === "string" ? p.arguments : (e.arguments || "") + (typeof p.argumentsDelta === "string" ? p.argumentsDelta : ""),
      output: eventType === "tool_completed" ? p.output : e.output,
      summary: typeof p.summary === "string" ? p.summary : e.summary,
      status: eventType === "tool_started" ? "running" : eventType === "tool_completed" ? p.status === "failed" ? "failed" : "completed" : e.status,
    });
  }
  if (eventType === "block_ended") return entries.map(e => e.id === blockId ? { ...e, status: e.kind === "tool" ? "pending" : "completed" } : e);
  if (["run_completed", "run_failed", "run_cancelled", "run_interrupted"].includes(eventType)) {
    const status: TraceStatus = eventType === "run_completed" ? "completed" : eventType === "run_cancelled" ? "cancelled" : eventType === "run_interrupted" ? "interrupted" : "failed";
    const result = entries.map(e => e.runId === runId && ["running", "pending", "receiving"].includes(e.status || "") ? { ...e, status } : e);
    if (eventType === "run_completed") return result;
    return [...result, { ...base, id: `${runId}:error`, kind: "error", text: typeof p.message === "string" ? p.message : "运行中断", status, error: p.error as ProviderError | undefined }];
  }
  return entries;
}

// Only contiguous events are projected. A later sequence triggers a replay request.
export function reduceTrace(state: TraceState, events: ExecutionEvent[]): TraceState {
  const pending = new Map(state.pending);
  const seen = new Set(state.seen);
  const terminalRuns = new Set(state.terminalRuns);
  let { entries, lastSeq } = state;
  for (const event of events) {
    if (event.seq > lastSeq && !seen.has(event.id)) pending.set(event.seq, event);
  }
  for (;;) {
    const event = pending.get(lastSeq + 1);
    if (!event) break;
    pending.delete(event.seq);
    seen.add(event.id);
    entries = project(entries, event);
    if (["run_completed", "run_failed", "run_cancelled", "run_interrupted"].includes(event.eventType)) terminalRuns.add(event.runId);
    lastSeq = event.seq;
  }
  return { entries, lastSeq, pending, seen, terminalRuns };
}
