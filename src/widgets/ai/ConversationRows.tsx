import { useMemo, useState } from "react";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { BrainCircuit, ChevronDown, ChevronRight, CircleAlert, CircleCheck, FileText, Loader2, Monitor, Music2, Search, TerminalSquare, Wrench } from "lucide-react";
import type { ConversationEntry, TraceStatus } from "@/agent/trace";
import { visibleLegacyText } from "./legacy";

export function MarkdownBody({ text }: { text: string }) {
  const html = useMemo(() => DOMPurify.sanitize(marked.parse(visibleLegacyText(text), { async: false }) as string), [text]);
  return <div className="min-w-0 break-words text-[12px] leading-[1.75] text-white/[0.82] [&_a]:break-all [&_a]:text-sky-300 [&_a]:underline [&_blockquote]:border-l-2 [&_blockquote]:border-white/[0.25] [&_blockquote]:pl-3 [&_code]:rounded [&_code]:bg-white/[0.09] [&_code]:px-1 [&_h1]:mb-2 [&_h1]:text-lg [&_h2]:mb-2 [&_h2]:text-base [&_li]:ml-4 [&_li]:list-disc [&_p]:mb-2 [&_pre]:overflow-x-auto [&_pre]:rounded-lg [&_pre]:bg-black/[0.25] [&_pre]:p-3 [&_table]:w-full [&_td]:border [&_td]:border-white/10 [&_td]:p-1 [&_th]:border [&_th]:border-white/10 [&_th]:p-1" dangerouslySetInnerHTML={{ __html: html }} />;
}

function StatusIcon({ status }: { status: TraceStatus }) {
  if (["running", "receiving", "pending"].includes(status)) return <Loader2 className="h-3.5 w-3.5 animate-spin text-white/50" />;
  if (status !== "completed") return <CircleAlert className="h-3.5 w-3.5 text-rose-300" />;
  return <CircleCheck className="h-3.5 w-3.5 text-emerald-300/70" />;
}

export function ReasoningRow({ entry }: { entry: Extract<ConversationEntry, { kind: "reasoning" }> }) {
  const [expanded, setExpanded] = useState(false);
  const lines = entry.detail?.trimEnd().split("\n");
  const summary = entry.status === "running" ? lines?.[lines.length - 1] : lines?.[0];
  return <div className="min-w-0 py-1">
    <button type="button" className="flex w-full min-w-0 items-center gap-2 rounded-md px-1 py-1 text-left text-[11px] text-white/55 transition-colors hover:bg-white/5" onClick={() => setExpanded(v => !v)} aria-expanded={expanded}>
      <BrainCircuit className="h-3.5 w-3.5 shrink-0" /><span className="shrink-0">{entry.title}</span>
      <span className="min-w-0 flex-1 truncate text-white/35">{summary}</span><StatusIcon status={entry.status} />{expanded ? <ChevronDown className="h-3 w-3 shrink-0" /> : <ChevronRight className="h-3 w-3 shrink-0" />}
    </button>
    {expanded && <div className="ml-6 max-h-72 overflow-y-auto whitespace-pre-wrap break-words border-l border-white/10 py-2 pl-3 text-[11px] leading-[1.65] text-white/50 [scrollbar-color:rgba(255,255,255,.2)_transparent] [scrollbar-width:thin]">{entry.detail}</div>}
  </div>;
}

function toolIcon(name: string) {
  if (name.includes("netease") || name.includes("网易云")) return Music2;
  if (name.includes("search") || name.includes("搜索")) return Search;
  if (name.includes("file") || name.includes("markdown") || name.includes("文件")) return FileText;
  if (name.includes("computer") || name.includes("电脑")) return Monitor;
  if (name.includes("shell") || name.includes("终端")) return TerminalSquare;
  return Wrench;
}

export function ToolRow({ entry }: { entry: Extract<ConversationEntry, { kind: "tool" }> }) {
  const [expanded, setExpanded] = useState(false);
  const Icon = toolIcon(entry.name || entry.title);
  const status = { receiving: "接收参数", pending: "等待执行", running: "执行中", completed: "已完成", failed: "失败", cancelled: "已停止", interrupted: "已中断" }[entry.status];
  return <div className="min-w-0 py-1">
    <button type="button" className="flex w-full min-w-0 items-center gap-2 rounded-md px-1 py-1 text-left text-[11px] text-white/65 transition-colors hover:bg-white/5" onClick={() => setExpanded(v => !v)} aria-expanded={expanded}>
      <Icon className="h-3.5 w-3.5 shrink-0 text-white/50" /><span className="min-w-0 truncate">{entry.title}</span>
      <span className="min-w-0 flex-1 truncate text-white/35">{entry.summary}</span><span className="shrink-0 text-[10px] text-white/40">{status}</span><StatusIcon status={entry.status} />{expanded ? <ChevronDown className="h-3 w-3 shrink-0" /> : <ChevronRight className="h-3 w-3 shrink-0" />}
    </button>
    {expanded && <div className="ml-6 max-h-72 space-y-2 overflow-auto border-l border-white/10 py-2 pl-3 text-[11px] leading-5 text-white/50 [scrollbar-color:rgba(255,255,255,.2)_transparent] [scrollbar-width:thin]">
      {entry.arguments && <div><div className="text-white/35">参数</div><pre className="whitespace-pre-wrap break-all">{entry.arguments}</pre></div>}
      {entry.output !== undefined && <div><div className="text-white/35">结果</div><pre className="whitespace-pre-wrap break-all">{typeof entry.output === "string" ? entry.output : JSON.stringify(entry.output, null, 2)}</pre></div>}
      {entry.detail && <pre className="whitespace-pre-wrap break-all">{entry.detail}</pre>}
    </div>}
  </div>;
}

export function ErrorRow({ entry }: { entry: Extract<ConversationEntry, { kind: "error" }> }) {
  const [expanded, setExpanded] = useState(false);
  const error = entry.error;
  return <div className="min-w-0 py-2 text-[11px] leading-5 text-rose-200/85" role="status">
    <button type="button" className="flex w-full min-w-0 items-start gap-2 text-left" onClick={() => setExpanded(v => !v)} aria-expanded={expanded}>
      <CircleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" /><span className="min-w-0 flex-1 break-words">{entry.text}</span>{error && (expanded ? <ChevronDown className="mt-1 h-3 w-3 shrink-0" /> : <ChevronRight className="mt-1 h-3 w-3 shrink-0" />)}
    </button>
    {expanded && error && <dl className="ml-5 mt-2 grid grid-cols-[64px_minmax(0,1fr)] gap-x-2 text-white/50">
      {Object.entries({ 类别: error.category, 状态: error.status, 模型: error.model, 协议: error.protocol, 地址: error.endpoint, 错误码: error.code, 参数: error.param, 请求ID: error.requestId, 响应: error.body }).filter(([, v]) => v !== undefined && v !== null && v !== "").map(([key, value]) => <div key={key} className="contents"><dt>{key}</dt><dd className="whitespace-pre-wrap break-all">{String(value)}</dd></div>)}
    </dl>}
  </div>;
}
