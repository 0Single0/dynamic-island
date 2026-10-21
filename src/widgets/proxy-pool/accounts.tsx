import { Coins, Pencil, Power, RefreshCw, RotateCcw, Trash2 } from "lucide-react";
import { useState, type ReactNode } from "react";

import * as api from "@/api/gateway";
import type { GatewayAccount } from "@/api/gateway";
import { IconButton } from "@/components/common/IconButton";
import { toast } from "@/utils/toast";

import { errorText, expiryText, formatClock, formatDuration, type ModelChip } from "./shared";

/** 表格列名（顺序即渲染顺序）。 */
type AccountColumn =
  | "account"
  | "channel"
  | "state"
  | "credential"
  | "quota"
  | "inflight"
  | "cooldown"
  | "models"
  | "tasks"
  | "probe"
  | "ops";

export interface AccountColumnSpec {
  key: AccountColumn;
  label: string;
  /** 宽度相关的 Tailwind 类（固定宽度 + grow/shrink 策略）。 */
  className: string;
  /**
   * 动态权重（只有账号列用）：按最长昵称算出来的 `flex-grow`，
   * 夹在 0.96..1.6，短昵称不浪费宽度、超长昵称也不会把整行顶变形。
   */
  style?: { flexGrow: number };
}

/**
 * 列宽策略：**比例权重 + 操作列定宽**。
 *
 * 走过的两条弯路，记在这里免得再试：
 * · 单一列独吞（`flex-1`）：窗口一大额度列被撑成一整条（"额度太宽了"）；
 * · 最小宽度：硬下限，窗口比它们之和窄时整行就溢出 —— 账号列被挤出可视区、
 *   底部多一条横向滚动条（"溢出就不管了？"）。
 *
 * 现在按权重分：所有列按比例分享可用宽度，**窗口多宽表格就多宽，永不溢出**。
 * 窄了每列都窄一点（文字截断），宽了每列都宽一点。
 * 唯一的硬宽度是操作列：6 个 32px 图标 = 192px（编辑 + 重置/凭据/额度/启停/删除），
 * `shrink-0` 钉死 —— 否则窗口一窄图标先被 flex 压扁，那是这个表最早也最反复的毛病。
 */
export function accountColumns(
  accounts: ReadonlyArray<{ nickname: string; uid: string }>,
): AccountColumnSpec[] {
  // 按 11px 字号估宽：CJK 约 12px、其余约 7px。只用来定权重，不需要精确。
  const widest = accounts.reduce((width, account) => {
    const name = account.nickname || account.uid.slice(0, 8);
    const measured = [...name].reduce(
      (sum, char) => sum + (char.charCodeAt(0) > 0x2e7f ? 12 : 7),
      0,
    );
    return Math.max(width, measured);
  }, 0);
  // +26 = 状态圆点 + 间隔 + 单元格内边距；96..160 换算成 0.96..1.60 的权重。
  const accountWeight = Math.min(160, Math.max(96, widest + 26)) / 100;

  return [
    { key: "account", label: "账号", className: "flex-1 min-w-0", style: { flexGrow: accountWeight } },
    { key: "channel", label: "渠道", className: "flex-[1] min-w-0" },
    { key: "state", label: "状态", className: "flex-[0.62] min-w-0" },
    { key: "credential", label: "凭据", className: "flex-[0.72] min-w-0" },
    { key: "quota", label: "额度", className: "flex-[1.22] min-w-0" },
    { key: "inflight", label: "请求中", className: "flex-[0.62] min-w-0" },
    { key: "cooldown", label: "限流等待", className: "flex-[0.66] min-w-0" },
    { key: "models", label: "可用模型", className: "flex-[0.88] min-w-0" },
    { key: "tasks", label: "任务", className: "flex-[0.92] min-w-0" },
    { key: "probe", label: "测试", className: "flex-[0.92] min-w-0" },
    // 5 个图标（重置/凭据/额度/启停/删除）+ 编辑 = 192px，定宽、不参与伸缩。
    { key: "ops", label: "操作", className: "w-[12.5rem] shrink-0" },
  ];
}


/** "请求中"单元格：`在飞/上限`，点上限可改（改完立即生效并落库）。 */
function InflightCell({
  channelId,
  uid,
  inflight,
  maxInflight,
  onChange,
}: {
  channelId: string;
  uid: string;
  inflight: number;
  maxInflight: number;
  onChange: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");

  const commit = async () => {
    setEditing(false);
    const value = Number.parseInt(draft, 10);
    if (Number.isNaN(value) || value === maxInflight || value < 1) return;
    try {
      await api.setAccountMaxInflight(channelId, uid, value);
      toast.success(`并发上限已改为 ${value}`);
      onChange();
    } catch (reason) {
      toast.error(errorText(reason, "修改失败"));
    }
  };

  if (!editing) {
    return (
      <span
        className="text-[10px] text-zinc-400"
        title={`正在处理的请求 ${inflight} 个 / 上限 ${maxInflight} 个（点数字修改上限；该上限是网关默认值，上游没有每账号并发限制）`}
      >
        {inflight}/
        <span
          role="button"
          tabIndex={0}
          onClick={(event) => {
            event.stopPropagation();
            setDraft(String(maxInflight));
            setEditing(true);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              setDraft(String(maxInflight));
              setEditing(true);
            }
          }}
          className="cursor-pointer border-b border-dashed border-zinc-500 hover:text-sky-200"
        >
          {maxInflight}
        </span>
      </span>
    );
  }
  return (
    <input
      autoFocus
      value={draft}
      onClick={(event) => event.stopPropagation()}
      onChange={(event) => setDraft(event.target.value.replace(/\D/g, ""))}
      onBlur={() => void commit()}
      onKeyDown={(event) => {
        if (event.key === "Enter") void commit();
        if (event.key === "Escape") setEditing(false);
      }}
      className="h-4 w-8 rounded border border-sky-400/50 bg-black/40 text-center font-mono text-[10px] text-zinc-100 outline-none"
      aria-label="并发上限"
    />
  );
}

/** 凭据剩余天数：只给天数，过期就直说。 */
function credentialDays(expiresAtMs: number): string {
  const remaining = expiresAtMs - Date.now();
  if (remaining <= 0) return "已过期";
  return `${Math.ceil(remaining / 86_400_000)} 天`;
}

/** 单元格上下文：账号行的展开态与回调，省得在 cell 里层层闭包。 */
interface AccountCellContext {
  support: ModelChip[];
  channelName: string;
  /** 渠道适配器类型：自定义供应商的密钥是静态的，操作与内置渠道不同。 */
  channelAdapter: string;
  expanded: boolean;
  tasks: boolean;
  probe: boolean;
  onToggleModels: () => void;
  onToggleTasks: () => void;
  onToggleProbe: () => void;
  /** 打开「编辑账号」弹窗（昵称 / 优先级 / 权重）。 */
  onEdit: () => void;
  onChange: () => void;
}

/**
 * 单元格内容：`key` 与 `ACCOUNT_COLUMNS` 一一对应（顺序也一致）。
 *
 * 放在组件外是让"列定义 → 内容"一眼能对上；宽度由外层列宽决定，
 * 这里只管内容与对齐，不再自带宽度类。
 */
export function accountCell(
  account: GatewayAccount,
  key: AccountColumn,
  context: AccountCellContext,
): ReactNode {
  /** 图标动作的统一收尾：成功/失败都提示，成功后刷新账号列表。 */
  const apply = async (action: () => Promise<unknown>, done: string, failure: string) => {
    try {
      await action();
      toast.success(done);
      context.onChange();
    } catch (reason) {
      toast.error(errorText(reason, failure));
    }
  };

  switch (key) {
    case "account":
      return (
        // 整格可点：昵称是这一行里最像"可编辑"的地方，点它开编辑弹窗；
        // 悬停给虚线底边（与"请求中"列改上限的交互一致），别让人靠猜。
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            context.onEdit();
          }}
          title={`${account.uid}\n点击编辑昵称 / 优先级 / 权重`}
          className="flex w-full min-w-0 items-center justify-center gap-1.5 rounded px-1 hover:bg-white/8"
        >
          <span
            className={`h-1.5 w-1.5 shrink-0 rounded-full ${
              account.ready ? "bg-emerald-400" : account.enabled ? "bg-amber-400" : "bg-zinc-500"
            }`}
          />
          <span className="truncate border-b border-dashed border-white/20 text-[11px] text-zinc-100">
            {account.nickname || account.uid.slice(0, 8)}
          </span>
        </button>
      );
    case "channel":
      return (
        <span className="w-full truncate text-center text-[10px] text-zinc-400">
          {context.channelName}
        </span>
      );
    case "state":
      return (
        <span className="flex items-center justify-center gap-0.5 text-[10px]">
          <span className={account.ready ? "text-emerald-300" : "text-amber-300"}>
            {account.ready ? "可用" : account.enabled ? "受限" : "停用"}
          </span>
          {/* 可用也要给 ?：账号能被调度，但可能"某个模型在限流"——
              那是最容易被误判成"号废了"的情形，必须能一眼看到原因。 */}
          {(!account.ready || account.blocked_reasons.length > 0) && (
            <span
              title={
                account.blocked_reasons.length > 0
                  ? account.blocked_reasons.join("\n")
                  : "当前不在冷却中，但没有可用候选（例如该渠道无模型）"
              }
              className="flex h-3.5 w-3.5 shrink-0 cursor-help items-center justify-center rounded-full bg-amber-400/25 text-[8px] text-amber-100"
            >
              ?
            </span>
          )}
        </span>
      );
    case "credential": {
      // 只显示天数（"26 天"）—— 表头已经写了"凭据"，"多少天过期"那串字纯属占地方。
      const text = account.has_credential
        ? account.credential_expires_at_ms
          ? credentialDays(account.credential_expires_at_ms)
          : "已登录"
        : "无凭据";
      const title = account.credential_expires_at_ms
        ? expiryText(account.credential_expires_at_ms)
        : text;
      return (
        <span className="w-full truncate text-center text-[10px] text-zinc-500" title={title}>
          {text}
        </span>
      );
    }
    case "quota":
      return <QuotaCell quota={account.quota} />;
    case "inflight":
      // 上限是**我们自己定的默认值**（上游没有每账号并发限制），点开就能改。
      return (
        <InflightCell
          channelId={account.channel_id}
          uid={account.uid}
          inflight={account.inflight}
          maxInflight={account.max_inflight}
          onChange={context.onChange}
        />
      );
    case "cooldown": {
      // 账号级冷却优先；没有账号冷却但有**模型级**限流时如实显示 ——
      // 上游按模型限额是常态（workbuddy 6004），账号本身还能跑其他模型。
      const accountCooldown = account.cooldown_until_ms - Date.now();
      if (accountCooldown > 0) {
        return (
          <span className="text-[10px] text-zinc-400" title="被上游拒绝后进入冷却，到点自动恢复">
            {formatDuration(accountCooldown)}
          </span>
        );
      }
      const limits = account.model_limits ?? [];
      if (limits.length === 0) {
        return (
          <span className="text-[10px] text-zinc-400" title="没有冷却">
            —
          </span>
        );
      }
      const detail = limits
        .map(
          (limit) =>
            `${limit.model}：${formatDuration(Math.max(0, limit.until_ms - Date.now()))} 后恢复`,
        )
        .join("\n");
      return (
        <span
          className="text-[10px] text-sky-300/90"
          title={`上游按模型限额（账号仍可用，其他模型不受影响）\n${detail}`}
        >
          模型 {limits.length}
        </span>
      );
    }
    case "models": {
      // 折叠着也要能看出"有模型在限流"：数字后面点一个小黄点。
      const limitedModels = new Set((account.model_limits ?? []).map((limit) => limit.model));
      const limitedCount = context.support.filter(
        (model) => limitedModels.has(model.upstream) || limitedModels.has(model.id),
      ).length;
      return (
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            context.onToggleModels();
          }}
          className="w-full truncate text-center text-[10px] text-zinc-400 hover:text-zinc-100"
          title={
            limitedCount > 0
              ? `展开可用模型（${limitedCount} 个正被上游限流，展开看是哪个）· 点模型名即复制`
              : "展开可用模型（点模型名即复制）"
          }
        >
          {context.support.length === 0
            ? "暂无模型"
            : `${context.support.length} 个模型 ${context.expanded ? "▴" : "▾"}`}
          {limitedCount > 0 && (
            <span className="ml-0.5 inline-block h-1.5 w-1.5 rounded-full bg-amber-400 align-middle" />
          )}
        </button>
      );
    }
    case "tasks":
      // 自定义供应商没有"成长任务/签到"体系（那是内置渠道的运营能力）。
      if (context.channelAdapter === "custom") {
        return <span className="text-[10px] text-zinc-600">—</span>;
      }
      return (
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            context.onToggleTasks();
          }}
          title="查看并一键执行该账号的成长任务"
          className="rounded-md bg-white/8 px-2 py-0.5 text-[10px] text-zinc-300 hover:bg-white/16"
        >
          任务 {context.tasks ? "▴" : "▾"}
        </button>
      );
    case "probe":
      return (
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            context.onToggleProbe();
          }}
          title="真实探针：指定模型发一条最小请求"
          className="rounded-md bg-white/8 px-2 py-0.5 text-[10px] text-zinc-300 hover:bg-white/16"
        >
          测试 {context.probe ? "▴" : "▾"}
        </button>
      );
    case "ops":
      return (
        <>
          {/* 编辑放第一个：它是"改这个号"的入口，其余都是瞬时动作。 */}
          <IconButton icon={Pencil} label="编辑账号（昵称 / 优先级 / 权重）" onClick={context.onEdit} />
          <IconButton
            icon={RotateCcw}
            label="重置状态（清冷却/熔断）"
            onClick={() =>
              apply(
                () => api.resetAccountState(account.channel_id, account.uid),
                "已重置该账号的冷却与熔断",
                "重置失败",
              )
            }
          />
          {/* 自定义供应商的密钥是静态的：没有刷新端点，也没有额度口径，
              这两个按钮按下去只会报错 —— 直接不显示。 */}
          {context.channelAdapter !== "custom" && (
            <IconButton
              icon={RefreshCw}
              label="刷新凭据"
              onClick={() =>
                apply(
                  () => api.refreshAccount(account.channel_id, account.uid),
                  "凭据已刷新",
                  "刷新失败",
                )
              }
            />
          )}
          {context.channelAdapter !== "custom" && (
            <IconButton
              icon={Coins}
              label="刷新额度"
              onClick={async () => {
                // 这个渠道**没有额度概念**时要说清楚，别让用户以为查询坏了：
                // CodeArts 按 token 计量，额度只在华为云控制台看。
                try {
                  const result = await api.fetchQuota(account.channel_id, account.uid);
                  if (result.supported) {
                    toast.success("额度已刷新");
                  } else {
                    toast.info("该渠道没有额度接口：用量请在服务商控制台查看");
                  }
                  context.onChange();
                } catch (reason) {
                  toast.error(errorText(reason, "额度查询失败"));
                }
              }}
            />
          )}
          <IconButton
            icon={Power}
            label={account.enabled ? "停用账号" : "启用账号"}
            onClick={() =>
              apply(
                () => api.setAccountEnabled(account.channel_id, account.uid, !account.enabled),
                account.enabled ? "已停用" : "已启用",
                "启停失败",
              )
            }
          />
          <IconButton
            icon={Trash2}
            label="删除账号"
            onClick={() =>
              apply(
                () => api.deleteAccount(account.channel_id, account.uid),
                "已删除",
                "删除失败",
              )
            }
          />
        </>
      );
    default:
      return null;
  }
}


/**
 * 额度单元格：进度条 + 剩余/总量。
 *
 * 条形表示**剩余占比**（用户关心的就是"还能用多少"），颜色分档：
 * 低于 10% 红、低于 30% 黄，其余绿 —— 一眼看出该不该补号。
 * 渠道无关：标签与单位都用渠道给的值（积分 / 额度 / 试用期天数）。
 */
function QuotaCell({ quota }: { quota: GatewayAccount["quota"] }) {
  if (!quota) {
    return (
      <span
        className="block text-[10px] text-zinc-600"
        title={"该渠道没有额度接口（或还没取到）：点左侧金币按钮刷新。\n" +
          "按 token 计量的渠道（如 CodeArts）的用量只能在服务商控制台看。"}
      >
        —
      </span>
    );
  }

  const number = (value: number | null) =>
    value === null
      ? "?"
      : Math.abs(value) >= 1000
        ? `${(value / 1000).toFixed(1)}k`
        : `${Math.round(value * 10) / 10}`;

  const total = quota.total;
  const remaining = quota.remaining;
  // 原始比例**不夹到 1**：上游给赠送/加包时剩余会超过总量，夹掉就看不到溢出了。
  const raw = total !== null && total > 0 && remaining !== null ? remaining / total : null;
  const ratio = raw === null ? null : Math.max(0, Math.min(1, raw));
  // 超过总量：进度条填满，百分比带 "+"（如 115%+），颜色按"充裕"而不是报警。
  const over = raw !== null && raw > 1;
  const tone =
    raw === null
      ? "bg-zinc-500"
      : over
        ? "bg-sky-400"
        : raw < 0.1
          ? "bg-rose-400"
          : raw < 0.3
            ? "bg-amber-400"
            : "bg-emerald-400";

  return (
    <span className="flex min-w-0 flex-col items-center gap-0.5" title={quotaTooltip(quota)}>
      <span className="flex items-baseline gap-1 truncate text-[10px]">
        <span className="shrink-0 text-zinc-400">{quota.label}</span>
        <span className="truncate text-zinc-200">
          {number(remaining)}
          {total === null ? ` ${quota.unit}` : ` / ${number(total)}`}
        </span>
        {raw !== null && (
          <span className={over ? "shrink-0 text-sky-300" : "shrink-0 text-zinc-500"}>
            {Math.round(raw * 100)}%{over ? "+" : ""}
          </span>
        )}
      </span>
      <span className="h-1 w-full overflow-hidden rounded-full bg-white/8">
        <span
          className={`block h-full rounded-full ${tone}`}
          style={{ width: `${ratio === null ? 0 : Math.round(ratio * 100)}%` }}
        />
      </span>
    </span>
  );
}

/** 悬停明细：抓取时间 + 渠道自定义的补充行。 */
function quotaTooltip(quota: GatewayAccount["quota"]): string {
  if (!quota) return "还没有额度数据：点左侧金币按钮刷新";
  const lines = [`抓取于 ${formatClock(quota.fetched_at_ms)}`];
  if (quota.used !== null) lines.push(`已用 ${quota.used} ${quota.unit}`);
  for (const [key, value] of quota.details) lines.push(`${key}：${value}`);
  return lines.join("\n");
}
