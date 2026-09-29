import { Check, Clock3, ShieldAlert, X } from "lucide-react";

export interface ApprovalCardProps {
  toolName: string;
  risk: "L0" | "L1" | "L2" | "L3";
  argsPreview: string;
  expiresAt?: string;
  onApprove: () => void;
  onReject: () => void;
}

export function ApprovalCard({ toolName, risk, argsPreview, expiresAt, onApprove, onReject }: ApprovalCardProps) {
  const highRisk = risk === "L2" || risk === "L3";
  return (
    <div className="rounded-xl border border-amber-300/20 bg-amber-300/[0.06] p-3 text-xs text-zinc-200">
      <div className="flex items-start gap-2">
        <span className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-lg bg-amber-300/15 text-amber-200"><ShieldAlert className="h-3.5 w-3.5" /></span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 font-medium">需要确认工具调用 <span className="rounded-full bg-amber-300/15 px-1.5 py-0.5 text-[10px] text-amber-100">{risk}</span></div>
          <div className="mt-1 text-zinc-400">{toolName}</div>
          <pre className="mt-2 max-h-20 overflow-auto rounded-lg bg-black/20 p-2 text-[10px] leading-4 text-zinc-400">{argsPreview}</pre>
          <div className="mt-2 flex items-center gap-1 text-[10px] text-zinc-600"><Clock3 className="h-3 w-3" /> {expiresAt ? `审批有效期至 ${expiresAt}` : "执行前会再次校验参数"}</div>
        </div>
      </div>
      <div className="mt-3 flex justify-end gap-2">
        <button type="button" onClick={onReject} className="flex items-center gap-1 rounded-lg bg-white/[0.06] px-2.5 py-1.5 text-[10px] text-zinc-400 hover:bg-white/[0.1]"><X className="h-3 w-3" />拒绝</button>
        <button type="button" onClick={onApprove} className={`flex items-center gap-1 rounded-lg px-2.5 py-1.5 text-[10px] ${highRisk ? "bg-amber-300/20 text-amber-100 hover:bg-amber-300/30" : "bg-sky-300/18 text-sky-100 hover:bg-sky-300/28"}`}><Check className="h-3 w-3" />允许一次</button>
      </div>
    </div>
  );
}