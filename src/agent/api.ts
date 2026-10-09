import { invoke } from "@tauri-apps/api/core";
import type { JSONContent } from "@tiptap/core";

export interface SessionRecord {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  archived: boolean;
}

export interface MessageRecord {
  id: string;
  sessionId: string;
  role: "user" | "assistant" | "system" | string;
  content: string;
  createdAt: string;
}

export interface TimelineRecord {
  id: string;
  sessionId: string;
  kind: "user" | "assistant" | "reasoning" | "tool" | string;
  text?: string | null;
  title?: string | null;
  detail?: string | null;
  status: "running" | "completed" | "failed" | string;
  position: number;
  createdAt: string;
}

export interface ProviderError {
  category: string;
  message: string;
  status?: number | null;
  protocol: string;
  model: string;
  endpoint: string;
  code?: string | null;
  param?: string | null;
  requestId?: string | null;
  body?: string | null;
}

export interface ExecutionEvent {
  version: number;
  id: string;
  sessionId: string;
  runId: string;
  step?: number | null;
  blockId?: string | null;
  callId?: string | null;
  parentCallId?: string | null;
  seq: number;
  eventType: string;
  payload: Record<string, unknown>;
  createdAt: string;
}

export function listExecutionEvents(sessionId: string, afterSeq = 0) {
  return tauriInvoke<ExecutionEvent[]>("agent_list_events", { sessionId, afterSeq });
}

/** 删除会话：后端级联清理该会话的消息、轨迹与事件；返回是否命中了会话。 */
export function deleteSession(sessionId: string) {
  return tauriInvoke<boolean>("agent_delete_session", { sessionId });
}

/** 已加载插件目录（只读）：来自插件运行时，用于 @ 选择与设置页展示。 */
export function listBuiltinPlugins() {
  return tauriInvoke<import("./plugins/types").PluginStatus[]>("list_builtin_plugins");
}

export interface ProviderRecord {
  id: string;
  name: string;
  protocol: string;
  baseUrl: string;
  hasApiKey: boolean;
  enabled: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface ModelRecord {
  id: string;
  providerId: string;
  modelId: string;
  displayName: string;
  contextWindow?: number | null;
  supportsTools: boolean;
  supportsStreaming: boolean;
  supportsImages: boolean;
  enabled: boolean;
}

export interface AgentRunResult {
  runId: string;
  message: MessageRecord;
  status: string;
}

export function tauriInvoke<T>(command: string, args?: Record<string, unknown>) {
  return invoke<T>(command, args);
}

export function documentToPlainText(document: JSONContent) {
  const parts: string[] = [];
  const visit = (node: JSONContent) => {
    if (node.type === "mention" && typeof node.attrs?.label === "string") {
      parts.push(`@${node.attrs.label}`);
    } else if (typeof node.text === "string") {
      parts.push(node.text);
    }
    node.content?.forEach(visit);
  };
  visit(document);
  return parts.join(" ").replace(/\s+/g, " ").trim();
}

export function documentToPluginIds(document: JSONContent) {
  const ids = new Set<string>();
  const visit = (node: JSONContent) => {
    if (node.type === "mention" && typeof node.attrs?.id === "string") ids.add(node.attrs.id);
    node.content?.forEach(visit);
  };
  visit(document);
  return [...ids];
}
