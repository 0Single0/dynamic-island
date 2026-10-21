import type { JSONContent } from "@tiptap/core";

export const NEW_CONVERSATION = "draft:new-conversation";
const emptyDocument: JSONContent = { type: "doc", content: [{ type: "paragraph" }] };
const storageKey = "agent.workspace.v1";
const listeners = new Set<() => void>();
let loaded = false;
let active = NEW_CONVERSATION;
let drafts: Record<string, JSONContent> = {};

function load() {
  if (loaded) return;
  loaded = true;
  try {
    const saved = JSON.parse(localStorage.getItem(storageKey) || "null");
    if (saved?.version !== 1) return;
    if (typeof saved.active === "string") active = saved.active;
    for (const [key, value] of Object.entries(saved.drafts || {})) {
      if (value && typeof value === "object" && "type" in value && value.type === "doc") drafts[key] = value as JSONContent;
    }
  } catch { /* Memory-only drafts still work when browser storage is unavailable. */ }
}
function publish() {
  try { localStorage.setItem(storageKey, JSON.stringify({ version: 1, active, drafts })); } catch { /* Keep the in-memory copy. */ }
  for (const listener of listeners) listener();
}
export function hasDraftContent(document: JSONContent): boolean {
  return Boolean(document.text || document.type === "mention" || document.type === "image" || document.content?.some(hasDraftContent));
}
export function getDraft(key: string): JSONContent { load(); return drafts[key] || emptyDocument; }
export function setDraft(key: string, document: JSONContent) {
  load();
  if (hasDraftContent(document)) drafts = { ...drafts, [key]: document };
  else { const next = { ...drafts }; delete next[key]; drafts = next; }
  publish();
}
export function promoteDraft(sessionId: string) {
  load();
  const draft = getDraft(NEW_CONVERSATION);
  const next = { ...drafts }; delete next[NEW_CONVERSATION];
  if (hasDraftContent(draft)) next[sessionId] = draft;
  drafts = next; active = sessionId; publish();
}
export function clearDraft(key: string) {
  load();
  if (!(key in drafts)) return;
  const next = { ...drafts }; delete next[key]; drafts = next; publish();
}
export function getActiveConversation() { load(); return active; }
export function setActiveConversation(id: string) { load(); active = id; publish(); }
export function subscribeDrafts(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; }

const modelKey = "agent.selectedModel.v1";
/** 记住上次使用的模型：重启应用 / 新建会话时恢复，而不是每次都跳回列表第一个。 */
export function loadSelectedModel(): string {
  try { return localStorage.getItem(modelKey) || ""; } catch { return ""; }
}
export function saveSelectedModel(modelId: string) {
  try {
    if (modelId) localStorage.setItem(modelKey, modelId);
    else localStorage.removeItem(modelKey);
  } catch { /* 记不住也不影响功能 */ }
}
