import type { MessageRecord, TimelineRecord } from "@/agent/api";
import type { ConversationEntry, TraceStatus } from "@/agent/trace";

export function visibleLegacyText(text: string) {
  const marker = text.toUpperCase().indexOf("DSML");
  if (marker < 0) return text;
  const start = text.lastIndexOf("<", marker);
  if (start < 0 || marker - start > 24) return text;
  const prose = text.slice(0, start).trimEnd();
  return `${prose}${prose ? "\n\n" : ""}模型返回了不支持的文本工具语法，未执行其中的命令。`;
}

export function legacyEntries(timeline: TimelineRecord[], messages: MessageRecord[]): ConversationEntry[] {
  const status = (s: string): TraceStatus => s === "running" ? "interrupted" : s === "failed" ? "failed" : "completed";
  if (!timeline.length) return messages.map((m, i) => ({ kind: m.role === "user" ? "user" : "assistant", id: m.id, text: visibleLegacyText(m.content), position: i - messages.length, createdAt: m.createdAt }));
  return timeline.flatMap((r, i): ConversationEntry[] => {
    const base = { id: r.id, position: i - timeline.length, createdAt: r.createdAt };
    if ((r.kind === "reasoning" && r.detail === "整理上下文并准备执行") || (r.kind === "tool" && r.detail?.startsWith("插件："))) return [];
    if (r.kind === "user" || r.kind === "assistant") return [{ ...base, kind: r.kind, text: visibleLegacyText(r.text || "") }];
    if (r.kind === "reasoning") return [{ ...base, kind: "reasoning", title: r.title || "思考", detail: r.text || r.detail || "", status: status(r.status) }];
    if (r.kind === "tool") return [{ ...base, kind: "tool", title: r.title || "调用工具", detail: r.detail || "", status: status(r.status) }];
    return [];
  });
}
