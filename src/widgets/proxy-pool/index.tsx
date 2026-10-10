import { useCallback, useEffect, useRef, useState } from "react";
import { Coins, Plus, Power, RefreshCw, RotateCcw, Trash2, X } from "lucide-react";
import * as api from "@/api/gateway";
import type { GatewayAccount, GatewayChannel, GatewayLoginTicket } from "@/api/gateway";
import { useGatewayState } from "@/stores/gateway/useGatewayState";
import { IconButton } from "@/components/common/IconButton";
import { toast } from "@/utils/toast";
import {
  expiryText,
  formatClock,
  formatDuration,
  formatTokens,
  useChannelModels,
  type ModelChip,
} from "./shared";
import { RequestLogView, SettingsView, UsageView } from "./views";

type Section = "accounts" | "logs" | "usage" | "settings";

const SECTIONS: Array<{ id: Section; label: string; hint: string }> = [
  { id: "accounts", label: "账号", hint: "号池列表" },
  { id: "logs", label: "请求日志", hint: "最近的调度结果" },
  { id: "usage", label: "用量", hint: "token · 缓存命中" },
  { id: "settings", label: "设置", hint: "渠道 · API Key · 亲和" },
];

/**
 * 反代号池面板（独立窗口主界面）。
 *
 * 账号页是**一个扁平的号池列表**：一行一个账号，能一眼看出它在哪个渠道、可用与否、
 * 支持哪些模型。加号走「添加账号」弹窗 —— 在弹窗里挑渠道发起授权，
 * 而不是把每个渠道的登录按钮摊在页面上。
 */
export function ProxyPoolExpanded() {
  const { status, channels, accounts, error, loading, refresh } = useGatewayState(true);
  const [section, setSection] = useState<Section>("accounts");

  return (
    <div className="relative flex h-full w-full gap-3 text-zinc-200">
      <nav className="flex w-36 shrink-0 flex-col gap-1">
        {SECTIONS.map((entry) => (
          <button
            key={entry.id}
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              setSection(entry.id);
            }}
            className={`rounded-lg px-2.5 py-1.5 text-left transition-colors ${
              section === entry.id
                ? "bg-white/12 text-zinc-50"
                : "text-zinc-400 hover:bg-white/6 hover:text-zinc-200"
            }`}
          >
            <div className="text-[12px] font-medium">{entry.label}</div>
            <div className="mt-0.5 text-[10px] text-zinc-500">{entry.hint}</div>
          </button>
        ))}

        <div className="mt-auto rounded-lg border border-white/8 bg-white/[0.03] px-2 py-1.5">
          <div className="flex items-center gap-1.5">
            <span
              className={`h-1.5 w-1.5 rounded-full ${
                status?.running ? "bg-emerald-400" : "bg-zinc-500"
              }`}
            />
            <span className="text-[10px] text-zinc-300">
              {status?.running ? "运行中" : "已停止"}
            </span>
            <span className="ml-auto">
              <IconButton
                icon={RefreshCw}
                label={loading ? "刷新中" : "刷新"}
                onClick={() => refresh()}
              />
            </span>
          </div>
          <div className="mt-1 truncate font-mono text-[9px] text-zinc-500">
            {status?.addr ?? "-"}
          </div>
          <div className="text-[9px] text-zinc-500">
            {status?.auth === "open" ? "本机免鉴权" : "需 API Key"} · 并发余{" "}
            {status?.concurrency.available ?? 0}
          </div>
        </div>
      </nav>

      <div className="flex min-h-0 flex-1 flex-col gap-2">
        {error && (
          <div className="rounded-lg border border-amber-400/25 bg-amber-400/10 px-2 py-1 text-[11px] text-amber-100">
            {error}
          </div>
        )}
        <div className="min-h-0 flex-1 overflow-y-auto pr-1">
          {section === "accounts" ? (
            <AccountsView channels={channels} accounts={accounts} onChange={refresh} />
          ) : section === "logs" ? (
            <RequestLogView />
          ) : section === "usage" ? (
            <UsageView />
          ) : (
            <SettingsView status={status} channels={channels} onChange={refresh} />
          )}
        </div>
      </div>
    </div>
  );
}

function AccountsView({
  channels,
  accounts,
  onChange,
}: {
  channels: GatewayChannel[];
  accounts: GatewayAccount[];
  onChange: () => void;
}) {
  const models = useChannelModels();
  const [adding, setAdding] = useState(false);
  // 额度自动刷新：过期（10 分钟）就补一次，串行且不并发，避免把上游刷成风控。
  const quotaRefreshing = useRef(false);
  const accountsRef = useRef(accounts);
  accountsRef.current = accounts;
  useEffect(() => {
    // 依赖只留 onChange：账号列表每 2 秒就换新数组，若把它写进依赖，
    // 定时器会被反复重建，永远等不到 60 秒。
    const timer = window.setInterval(async () => {
      if (quotaRefreshing.current) return;
      const stale = accountsRef.current.filter(
        (account) =>
          account.has_credential &&
          (!account.quota || Date.now() - account.quota.fetched_at_ms > 10 * 60 * 1000),
      );
      if (stale.length === 0) return;
      quotaRefreshing.current = true;
      try {
        for (const account of stale) {
          await api.fetchQuota(account.channel_id, account.uid).catch(() => null);
        }
        onChange();
      } finally {
        quotaRefreshing.current = false;
      }
    }, 60_000);
    return () => window.clearInterval(timer);
  }, [onChange]);
  const [expanded, setExpanded] = useState<string | null>(null);

  const channelName = (id: string) =>
    channels.find((channel) => channel.id === id)?.display_name || id;

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-2 pb-2">
        <span className="text-[12px] font-medium text-zinc-100">号池</span>
        <span className="text-[10px] text-zinc-500">
          {accounts.length} 个账号 · 可用 {accounts.filter((account) => account.ready).length}
        </span>
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            setAdding(true);
          }}
          className="ml-auto flex items-center gap-1 rounded-md bg-emerald-400/15 px-2.5 py-1 text-[11px] text-emerald-200 hover:bg-emerald-400/25"
        >
          <Plus className="h-3.5 w-3.5" />
          添加账号
        </button>
      </div>

      {accounts.length === 0 ? (
        <div className="mt-6 rounded-xl border border-dashed border-white/12 px-4 py-8 text-center">
          <p className="text-[12px] text-zinc-400">号池是空的</p>
          <p className="mt-1 text-[11px] text-zinc-500">
            点右上角「添加账号」，选一个渠道完成授权即可入库。
          </p>
        </div>
      ) : (
        <div className="flex flex-col gap-1">
          <div className="flex items-center gap-2 px-2 text-[10px] text-zinc-500">
            <span className="min-w-0 flex-[1.2] text-center">账号</span>
            <span className="min-w-0 flex-[1.2] text-center">渠道</span>
            <span className="min-w-0 flex-[0.6] text-center">状态</span>
            <span className="min-w-0 flex-[0.9] text-center">凭据</span>
            <span className="min-w-0 flex-[1] text-center">额度</span>
            <span className="min-w-0 flex-[0.7] text-center">请求中</span>
            <span className="min-w-0 flex-[0.7] text-center">限流等待</span>
            <span className="min-w-0 flex-1 text-center">可用模型</span>
            <span className="w-32 shrink-0 text-center">操作</span>
          </div>
          {accounts.map((account) => {
            const support = models[account.channel_id] ?? [];
            return (
              <div key={`${account.channel_id}/${account.uid}`} className="flex flex-col">
              <div className="flex items-center gap-2 overflow-hidden rounded-lg border border-white/8 bg-white/[0.025] px-2 py-1.5">
                <span className="flex min-w-0 flex-[1.2] items-center justify-center gap-1.5">
                  <span
                    className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                      account.ready
                        ? "bg-emerald-400"
                        : account.enabled
                          ? "bg-amber-400"
                          : "bg-zinc-500"
                    }`}
                  />
                  <span className="truncate text-[11px] text-zinc-100">
                    {account.nickname || account.uid.slice(0, 8)}
                  </span>
                </span>
                <span className="min-w-0 flex-[1.2] truncate text-center text-[10px] text-zinc-400">
                  {channelName(account.channel_id)}
                </span>
                <span className="flex min-w-0 flex-[0.6] items-center justify-center gap-0.5 text-[10px]">
                  <span className={account.ready ? "text-emerald-300" : "text-amber-300"}>
                    {account.ready ? "可用" : account.enabled ? "受限" : "停用"}
                  </span>
                  {!account.ready && (
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
                <span
                  className="min-w-0 flex-[0.9] truncate text-center text-[10px] text-zinc-500"
                  title={account.credential_expires_at_ms ? expiryText(account.credential_expires_at_ms) : ""}
                >
                  {account.has_credential
                    ? account.credential_expires_at_ms
                      ? expiryText(account.credential_expires_at_ms)
                      : "已登录"
                    : "无凭据"}
                </span>
                <span className="min-w-0 flex-[1]">
                  <QuotaCell quota={account.quota} />
                </span>
                <span
                  className="min-w-0 flex-[0.7] text-center text-[10px] text-zinc-400"
                  title={`正在处理的请求 ${account.inflight} 个 / 该账号上限 ${account.max_inflight} 个`}
                >
                  {account.inflight}/{account.max_inflight}
                </span>
                <span
                  className="min-w-0 flex-[0.7] text-center text-[10px] text-zinc-400"
                  title={
                    account.cooldown_until_ms > Date.now()
                      ? "被上游拒绝后进入冷却，到点自动恢复"
                      : "没有冷却"
                  }
                >
                  {account.cooldown_until_ms > Date.now()
                    ? formatDuration(account.cooldown_until_ms - Date.now())
                    : "—"}
                </span>
                <button
                  type="button"
                  onClick={(event) => {
                    event.stopPropagation();
                    setExpanded(expanded === account.uid ? null : account.uid);
                  }}
                  className="min-w-0 flex-1 truncate text-center text-[10px] text-zinc-400 hover:text-zinc-100"
                  title="展开可用模型（点模型名即复制）"
                >
                  {support.length === 0
                    ? "暂无模型"
                    : `${support.length} 个模型 ${expanded === account.uid ? "▴" : "▾"}`}
                </button>
                <span className="flex w-32 shrink-0 items-center justify-center gap-0.5 px-1">
                  <IconButton
                    icon={RotateCcw}
                    label="重置状态（清冷却/熔断）"
                    onClick={async () => {
                      try {
                        await api.resetAccountState(account.channel_id, account.uid);
                        toast.success("已重置该账号的冷却与熔断");
                        onChange();
                      } catch (reason) {
                        toast.error(reason instanceof Error ? reason.message : "重置失败");
                      }
                    }}
                  />
                  <IconButton
                    icon={RefreshCw}
                    label="刷新凭据"
                    onClick={async () => {
                      try {
                        await api.refreshAccount(account.channel_id, account.uid);
                        toast.success("凭据已刷新");
                        onChange();
                      } catch (reason) {
                        toast.error(reason instanceof Error ? reason.message : "刷新失败");
                      }
                    }}
                  />
                  <IconButton
                    icon={Coins}
                    label="刷新额度"
                    onClick={async () => {
                      try {
                        await api.fetchQuota(account.channel_id, account.uid);
                        onChange();
                      } catch (reason) {
                        toast.error(reason instanceof Error ? reason.message : "额度查询失败");
                      }
                    }}
                  />
                  <IconButton
                    icon={Power}
                    label={account.enabled ? "停用账号" : "启用账号"}
                    onClick={async () => {
                      await api.setAccountEnabled(account.channel_id, account.uid, !account.enabled);
                      onChange();
                    }}
                  />
                  <IconButton
                    icon={Trash2}
                    label="删除账号"
                    onClick={async () => {
                      await api.deleteAccount(account.channel_id, account.uid);
                      toast.success("已删除");
                      onChange();
                    }}
                  />
                </span>
              </div>
              {expanded === account.uid && (
                <div className="mt-0.5 rounded-lg border border-white/8 bg-black/25 px-2 py-1.5">
                  {support.length === 0 ? (
                    <span className="text-[10px] text-zinc-500">该渠道还没有模型目录</span>
                  ) : (
                    <div className="grid grid-cols-[repeat(auto-fill,minmax(230px,1fr))] gap-1.5">
                      {support.map((model) => (
                        <ModelCard key={model.id} model={model} />
                      ))}
                    </div>
                  )}
                </div>
              )}
            </div>
            );
          })}
        </div>
      )}

      {adding && (
        <AddAccountDialog
          channels={channels}
          onClose={() => setAdding(false)}
          onAdded={onChange}
        />
      )}
    </div>
  );
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
      <span className="block text-[10px] text-zinc-600" title="还没有额度数据：点左侧金币按钮刷新">
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
  const ratio =
    total !== null && total > 0 && remaining !== null
      ? Math.max(0, Math.min(1, remaining / total))
      : null;
  const tone =
    ratio === null
      ? "bg-zinc-500"
      : ratio < 0.1
        ? "bg-rose-400"
        : ratio < 0.3
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
        {ratio !== null && (
          <span className="shrink-0 text-zinc-500">{Math.round(ratio * 100)}%</span>
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

/**
 * 模型卡片：点一下复制模型名；下面两行是**这个模型怎么用**——
 * 倍率、上下文/输出上限、能力徽标（图片/工具/思考）、思考档位与默认档。
 */
function ModelCard({ model }: { model: ModelChip }) {
  const badges = [
    model.capabilities.vision ? "图片" : null,
    model.capabilities.tool_calls ? "工具" : null,
    model.capabilities.reasoning ? "思考" : null,
  ].filter((badge): badge is string => badge !== null);

  return (
    <button
      type="button"
      title="点击复制模型名"
      onClick={(event) => {
        event.stopPropagation();
        void navigator.clipboard?.writeText(model.id);
        toast.success(`已复制 ${model.id}`);
      }}
      className="flex flex-col gap-1 rounded-lg border border-white/10 bg-white/[0.03] px-2 py-1.5 text-left transition-colors hover:border-sky-400/40 hover:bg-sky-400/10"
    >
      <span className="flex items-center gap-1.5">
        <span className="truncate font-mono text-[10px] text-zinc-100">{model.id}</span>
        <span
          className={`ml-auto shrink-0 text-[9px] ${
            model.free ? "text-emerald-300" : "text-zinc-500"
          }`}
        >
          {model.free ? "免费" : (model.credits ?? "—")}
        </span>
      </span>

      <span className="flex flex-wrap items-center gap-1 text-[9px] text-zinc-500">
        <span>上下文 {model.context_length ? formatTokens(model.context_length) : "?"}</span>
        <span className="text-zinc-700">·</span>
        <span>输出 {model.max_output_tokens ? formatTokens(model.max_output_tokens) : "?"}</span>
        {badges.map((badge) => (
          <span key={badge} className="rounded bg-white/8 px-1 text-zinc-300">
            {badge}
          </span>
        ))}
      </span>

      <span className="flex flex-wrap items-center gap-1 text-[9px] text-zinc-500">
        <span>
          档位{" "}
          {model.reasoning_efforts.length > 0
            ? model.reasoning_efforts.join("/")
            : (model.reasoning_fixed_effort ?? "无")}
        </span>
        {model.reasoning_default_effort && (
          <span className="text-zinc-400">默认 {model.reasoning_default_effort}</span>
        )}
        {model.always_reasoning && <span className="text-amber-300">强制思考</span>}
        {model.reasoning_can_disable === false && <span className="text-zinc-400">不可关闭</span>}
      </span>
    </button>
  );
}

/**
 * 添加账号弹窗：挑渠道 → 浏览器授权 → 自动入库。
 *
 * 成功后**不关闭**弹窗，方便连着加第二个渠道的账号；右下角是"完成"。
 */
function AddAccountDialog({
  channels,
  onClose,
  onAdded,
}: {
  channels: GatewayChannel[];
  onClose: () => void;
  onAdded: () => void;
}) {
  const [pending, setPending] = useState<{ channelId: string; ticket: GatewayLoginTicket } | null>(
    null,
  );
  const [hint, setHint] = useState<string | null>(null);
  const [added, setAdded] = useState<string[]>([]);
  const pollTimer = useRef<number | null>(null);

  const stopPolling = useCallback(() => {
    if (pollTimer.current !== null) {
      window.clearInterval(pollTimer.current);
      pollTimer.current = null;
    }
  }, []);
  useEffect(() => stopPolling, [stopPolling]);

  const start = async (channelId: string) => {
    try {
      const ticket = await api.loginStart(channelId);
      setPending({ channelId, ticket });
      setHint("在浏览器里完成登录授权，这里会自动完成入库与签到");
      stopPolling();
      pollTimer.current = window.setInterval(async () => {
        try {
          const result = await api.loginPoll(channelId, ticket.state);
          if (result.status === "pending") return;
          stopPolling();
          if (result.status === "ok") {
            const name = result.account?.nickname ?? result.account?.uid ?? "";
            setAdded((previous) => [...previous, `${name}（${channelId}）`]);
            setPending(null);
            setHint(null);
            toast.success(`已加入账号：${name}`);
            onAdded();
          } else {
            setHint(result.message ?? "授权失败");
          }
        } catch (reason) {
          stopPolling();
          setHint(reason instanceof Error ? reason.message : "轮询失败");
        }
      }, 2500);
    } catch (reason) {
      setHint(reason instanceof Error ? reason.message : "无法开始登录");
    }
  };

  const enabled = channels.filter((channel) => channel.enabled);

  return (
    <div
      className="absolute inset-0 z-50 flex items-center justify-center bg-black/65 p-8"
      onClick={(event) => {
        event.stopPropagation();
        stopPolling();
        onClose();
      }}
    >
      <div
        className="w-full max-w-lg rounded-2xl border border-white/12 bg-zinc-900 p-4 shadow-2xl"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="flex items-center gap-2">
          <span className="text-[13px] font-medium text-zinc-100">添加账号</span>
          <span className="text-[10px] text-zinc-500">选一个渠道发起授权</span>
          <button
            type="button"
            onClick={() => {
              stopPolling();
              onClose();
            }}
            className="ml-auto flex h-6 w-6 items-center justify-center rounded-md text-zinc-400 hover:bg-white/12 hover:text-zinc-100"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </header>

        <div className="mt-3 flex flex-wrap gap-1.5">
          {enabled.map((channel) => (
            <button
              key={channel.id}
              type="button"
              disabled={pending !== null}
              onClick={() => void start(channel.id)}
              className={`rounded-lg border px-3 py-1.5 text-[11px] transition-colors ${
                pending?.channelId === channel.id
                  ? "border-sky-400/40 bg-sky-400/15 text-sky-100"
                  : "border-white/12 bg-white/[0.04] text-zinc-200 hover:bg-white/10 disabled:opacity-40"
              }`}
            >
              {channel.display_name || channel.id}
            </button>
          ))}
          {enabled.length === 0 && (
            <p className="text-[11px] text-zinc-500">
              没有启用的渠道：到「设置」里添加渠道配置后再来。
            </p>
          )}
        </div>

        {pending && (
          <div className="mt-3 rounded-lg border border-sky-400/25 bg-sky-400/8 px-2.5 py-2">
            <p className="text-[11px] text-sky-100">{hint}</p>
            <button
              type="button"
              onClick={() => {
                void navigator.clipboard?.writeText(pending.ticket.auth_url);
                toast.success("授权链接已复制");
              }}
              className="mt-1.5 block max-w-full truncate rounded-md bg-black/30 px-2 py-1 text-left font-mono text-[10px] text-sky-100/85"
            >
              {pending.ticket.auth_url}
            </button>
            <p className="mt-1 text-[10px] text-sky-200/70">
              正在等待授权（每 2.5 秒检查一次）…
            </p>
          </div>
        )}

        {added.length > 0 && (
          <div className="mt-3 rounded-lg border border-emerald-400/25 bg-emerald-400/8 px-2.5 py-2">
            <p className="text-[11px] text-emerald-100">本次新增 {added.length} 个账号</p>
            {added.map((entry) => (
              <div key={entry} className="text-[10px] text-emerald-200/80">
                {entry}
              </div>
            ))}
          </div>
        )}

        <footer className="mt-4 flex items-center gap-2">
          <span className="text-[10px] text-zinc-500">
            可以连续添加多个渠道；关闭弹窗前请先等授权完成
          </span>
          <button
            type="button"
            onClick={() => {
              stopPolling();
              onClose();
            }}
            className="ml-auto rounded-md bg-white/10 px-3 py-1 text-[11px] text-zinc-100 hover:bg-white/16"
          >
            完成
          </button>
        </footer>
      </div>
    </div>
  );
}
