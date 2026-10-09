import React from "react";
import { createRoot } from "react-dom/client";
import { AIExpanded } from "../src/widgets/ai/AIExpanded";
import "../src/index.css";
import type { ExecutionEvent, SessionRecord } from "../src/agent/api";
import { Select } from "../src/components/common/Select";

// Browser-only harness: exercises the production hook and rows without OS IPC.
const callbacks = new Map<number, (event: unknown) => void>();
const listeners = new Set<number>();
let callbackId = 0;
let events: ExecutionEvent[] = JSON.parse(sessionStorage.getItem("trace-test-events") || "[]");
const sessions: SessionRecord[] = JSON.parse(sessionStorage.getItem("trace-test-sessions") || "null") || [{ id: "test-session", title: "轨迹验收", createdAt: "0", updatedAt: "0", archived: false }, { id: "second-session", title: "另一段对话", createdAt: "0", updatedAt: "0", archived: false }];
const saveFixture = () => { sessionStorage.setItem("trace-test-events",JSON.stringify(events)); sessionStorage.setItem("trace-test-sessions",JSON.stringify(sessions)); };
let cancelled = false;
const delay = (ms: number) => new Promise(r => setTimeout(r, ms));
const emit = (input: { sessionId: string; runId: string }, eventType: string, payload: Record<string, unknown> = {}, blockId: string | null = null) => {
  const seq = events.filter(e => e.sessionId === input.sessionId).length + 1;
  const event: ExecutionEvent = { version: 1, id: `${input.runId}-${seq}`, sessionId: input.sessionId, runId: input.runId, seq, eventType, payload, blockId: blockId ? `${input.runId}:${blockId}` : null, callId: blockId === "tool" ? `${input.runId}:call-test` : null, createdAt: "0" };
  events.push(event);
  saveFixture();
  for (const id of listeners) callbacks.get(id)?.({ event: "agent://trace", id, payload: event });
};

Object.assign(window, {
  __TAURI_INTERNALS__: {
    transformCallback: (callback: (event: unknown) => void) => { const id = ++callbackId; callbacks.set(id, callback); return id; },
    invoke: async (command: string, args: Record<string, any> = {}) => {
      if (command === "plugin:event|listen") { listeners.add(args.handler); return args.handler; }
      if (command === "plugin:event|unlisten") { listeners.delete(args.eventId); return; }
      if (command === "agent_list_sessions") return sessions.filter(s => ["test-session", "second-session"].includes(s.id) || events.some(e => e.sessionId === s.id));
      if (command === "agent_list_model_configs") return ["deepseek-v4.1-flash", "glm-5.3-flash"].map((name,i)=>({ id: `test-model-${i}`, providerId: "test", modelId: name, displayName: name, enabled: true }));
      if (command === "agent_list_events") return events.filter(e => e.sessionId === args.sessionId && e.seq > (args.afterSeq || 0));
      if (command === "list_builtin_plugins") return [
        { id: "system", namespace: "system", name: "系统", version: "0.1.0", description: "读取当前系统时间", icon: "system", defaultScope: true, refAliases: ["系统"], risk: "L0", lifecycle: "active", generation: 1, tools: [{ name: "system__get_time", toolName: "get_time", description: "读取当前系统时间", capabilityId: "system.get_time", riskLevel: "L0", requiresConfirmation: false, legacyNames: ["system.get_time", "system_get_time"], available: true }], capabilities: [{ id: "system.get_time", description: "读取当前系统时间", riskLevel: "L0", available: true }], resources: [], events: [] },
        { id: "netease", namespace: "netease", name: "网易云音乐", version: "0.1.0", description: "读取歌曲、歌词和播放状态", icon: "netease", defaultScope: false, refAliases: ["网易云音乐", "music"], risk: "L1", lifecycle: "active", generation: 2, tools: [{ name: "netease__get_current_state", toolName: "get_current_state", description: "读取网易云当前歌曲、歌词和播放进度", capabilityId: "music.player.current_state", riskLevel: "L0", requiresConfirmation: false, legacyNames: ["netease.get_current_state", "netease_get_current_state"], available: true }], capabilities: [{ id: "music.player.current_state", description: "读取当前歌曲、歌词与播放进度", riskLevel: "L0", available: true }], resources: [], events: [] },
        { id: "desktop", namespace: "computer", name: "电脑", version: "0.1.0", description: "检查窗口控件，逐次确认后操作按钮和输入值", icon: "desktop", defaultScope: false, refAliases: ["电脑", "computer"], risk: "L3", lifecycle: "active", generation: 3, tools: [{ name: "computer__list_windows", toolName: "list_windows", description: "列出可操作的本机窗口", capabilityId: "computer.list_windows", riskLevel: "L0", requiresConfirmation: false, legacyNames: ["computer_list_windows"], available: true }], capabilities: [{ id: "computer.list_windows", description: "列出可操作的本机窗口", riskLevel: "L0", available: true }, { id: "computer.screenshot", description: "截取目标窗口图像", riskLevel: "L2", available: false }], resources: [], events: [] },
      ];
      if (command === "agent_list_timeline" || command === "agent_list_messages") return [];
      if (command === "agent_create_session") { const session = { ...sessions[0], id: crypto.randomUUID(), title: "新对话" }; sessions.unshift(session); saveFixture(); return session; }
      if (command === "agent_delete_session") { const index = sessions.findIndex(s => s.id === args.sessionId); if (index < 0) return false; sessions.splice(index, 1); events = events.filter(e => e.sessionId !== args.sessionId); saveFixture(); return true; }
      if (command === "agent_cancel_run") { cancelled = true; return; }
      if (command === "agent_send_message") {
        const input = args.input;
        const session = sessions.find(s=>s.id===input.sessionId); if(session?.title==="新对话") session.title=input.content.slice(0,32);
        cancelled = false;
        emit(input, "run_started"); emit(input, "user_message", { text: input.content });
        if (input.content.includes("400")) {
          const error = { category: "INVALID_REQUEST", status: 400, message: "工具名称包含无效字符", protocol: "chat-completions", model: "fixture", endpoint: "https://example.com/chat/completions", code: "InvalidParameter", param: "tools.0.function.name", requestId: "test-request", body: "invalid name" };
          emit(input, "run_failed", { message: "HTTP 400: 工具名称包含无效字符", error }); throw error;
        }
        emit(input, "block_started", { kind: "reasoning" }, "reasoning");
        for (const text of ["需要读取播放器。", "我会使用网易云工具。"] ) { emit(input, "reasoning_delta", { text }, "reasoning"); await delay(400); }
        emit(input, "block_ended", {}, "reasoning");
        emit(input, "block_started", { kind: "assistant" }, "before"); emit(input, "text_delta", { text: "我来读取当前播放状态。" }, "before"); emit(input, "block_ended", {}, "before");
        emit(input, "block_started", { kind: "tool" }, "tool"); emit(input, "tool_delta", { name: "netease.get_current_state", title: "读取网易云播放状态", argumentsDelta: "{" }, "tool"); await delay(400);
        emit(input, "tool_delta", { argumentsDelta: "}" }, "tool"); emit(input, "tool_started", { arguments: "{}" }, "tool"); await delay(1500);
        if (cancelled) { emit(input, "run_cancelled", { message: "运行已停止" }); throw "运行已停止"; }
        emit(input, "tool_completed", { output: { title: "测试歌曲", artist: "测试歌手" }, summary: "已读取播放状态", status: "completed" }, "tool");
        emit(input, "block_started", { kind: "assistant" }, "after");
        for (const text of ["当前播放的是 **测试歌曲**", "，歌手是测试歌手。", "\n\n这段正文在工具调用之后继续生成。"] ) { emit(input, "text_delta", { text }, "after"); await delay(350); }
        emit(input, "block_ended", {}, "after"); emit(input, "run_completed"); return {};
      }
      return null;
    },
  },
  __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: (_event: string, id: number) => listeners.delete(id) },
});

function WorkspaceHarness() {
  const [headerSlot,setHeaderSlot] = React.useState<HTMLSpanElement|null>(null);
  const [sample,setSample] = React.useState("one");
  return <main className="flex min-h-screen flex-col items-center bg-zinc-950 p-4">
    <div className="glass-island flex flex-col overflow-hidden rounded-lg p-4" style={{width:780,height:480,maxWidth:"100%"}}>
      <header className="flex h-7 shrink-0 items-center gap-2"><span ref={setHeaderSlot} className="contents" /><span className="text-sm font-semibold text-zinc-100">AI Agent</span></header>
      <div className="min-h-0 flex-1"><AIExpanded headerSlot={headerSlot} /></div>
    </div>
    <div className="mt-4 w-60"><Select value={sample} onChange={setSample} aria-label="测试公共选择器" options={[{value:"one",label:"选项一"},{value:"two",label:"选项二"}]} /></div>
  </main>;
}
const root = createRoot(document.getElementById("root")!);
root.render(<React.StrictMode><WorkspaceHarness /></React.StrictMode>);
import.meta.hot?.dispose(() => root.unmount());
