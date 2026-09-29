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
