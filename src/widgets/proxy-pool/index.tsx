import { useCallback, useEffect, useState } from "react";
import { Play, Plus, RefreshCw } from "lucide-react";
import * as api from "@/api/gateway";
import type { GatewayAccount, GatewayChannel } from "@/api/gateway";
import { useGatewayState } from "@/stores/gateway/useGatewayState";
import { IconButton } from "@/components/common/IconButton";
import { Select } from "@/components/common/Select";
import { toast } from "@/utils/toast";
import {
  errorText,
  formatDuration,
  formatTokens,
  useChannelModels,
  useTaskRunState,
  writeTaskRun,
  type ModelChip,
} from "./shared";
import { AddAccountDialog } from "./AddAccountDialog";
import { accountCell, accountColumns } from "./accounts";
import { RequestLogView, SettingsView, UsageView } from "./views";

type Section = "accounts" | "logs" | "usage" | "settings";

const SECTIONS: Array<{ id: Section; label: string; hint: string }> = [
  { id: "accounts", label: "账号", hint: "号池列表" },
  { id: "logs", label: "请求日志", hint: "最近的调度结果" },
  { id: "usage", label: "用量", hint: "token 消耗" },
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

      <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-2">
        {error && (
          <div className="rounded-lg border border-amber-400/25 bg-amber-400/10 px-2 py-1 text-[11px] text-amber-100">
            {error}
          </div>
        )}
        {/* 日志页自带滚动容器 + 底部分页栏：分页不随表格滚动，所以不走外层滚动包裹。 */}
        {section === "logs" ? (
          <RequestLogView />
        ) : (
          <div className="gw-scroll gw-scroll-x min-h-0 min-w-0 flex-1 overflow-auto pr-1">
            {section === "accounts" ? (
              <AccountsView channels={channels} accounts={accounts} onChange={refresh} />
            ) : section === "usage" ? (
              <UsageView />
            ) : (
              <SettingsView status={status} channels={channels} onChange={refresh} />
            )}
          </div>
        )}
      </div>
    </div>
  );
}


/**
 * 模型卡片：点一下复制模型名；下面两行是**这个模型怎么用**——
 * 倍率、上下文/输出上限、能力徽标（图片/工具/思考）、思考档位与默认档。
 */
/**
 * 模型卡。`limit` 来自该**账号**的模型级限流（上游按模型限额时只停这个模型）——
 * 标在卡片上，一眼能看出"不是号废了，是这个模型在等"。
 */
function ModelCard({ model, limit }: { model: ModelChip; limit?: { until_ms: number } }) {
  const limited = limit !== undefined && limit.until_ms > Date.now();
  const limitedFor = limited ? formatDuration(limit.until_ms - Date.now()) : "";
  const badges = [
    model.capabilities.vision ? "图片" : null,
    model.capabilities.tool_calls ? "工具" : null,
    model.capabilities.reasoning ? "思考" : null,
  ].filter((badge): badge is string => badge !== null);

  return (
    <button
      type="button"
      title={
        limited
          ? `被上游限流，${limitedFor} 后恢复（同账号其他模型不受影响）· 点击复制模型名`
          : "点击复制模型名"
      }
      onClick={(event) => {
        event.stopPropagation();
        void navigator.clipboard?.writeText(model.id);
        toast.success(`已复制 ${model.id}`);
      }}
      className={`flex flex-col gap-1 rounded-lg border px-2 py-1.5 text-left transition-colors hover:border-sky-400/40 hover:bg-sky-400/10 ${
        limited ? "border-amber-400/35 bg-amber-400/[0.06]" : "border-white/10 bg-white/[0.03]"
      }`}
    >
      <span className="flex items-center gap-1.5">
        <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-zinc-100">
          {model.id}
        </span>
        {limited && (
          <span className="shrink-0 rounded bg-amber-400/20 px-1 text-[9px] text-amber-100">
            限流 {limitedFor}
          </span>
        )}
        <span
          className={`shrink-0 text-[9px] ${model.free ? "text-emerald-300" : "text-zinc-500"}`}
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
 * 账号行里展开的探针区：选模型 → 发一条真实请求 → 显示耗时/回复/用量。
 *
 * 与"看状态灯"不同，这是**真打上游**：能验证凭据、网络、模型准入是否都通。
 * 成功后账号的错误状态会被清掉（与调度路径一致）。
 */
function AccountProbe({ channelId, uid }: { channelId: string; uid: string }) {
  const [models, setModels] = useState<string[]>([]);
  const [model, setModel] = useState("");
  const [outcome, setOutcome] = useState<api.GatewayProbeOutcome | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [running, setRunning] = useState(false);

  useEffect(() => {
    void api
      .probeModels(channelId)
      .then((list) => {
        setModels(list);
        if (list.length > 0) setModel(list[0]);
      })
      .catch(() => setModels([]));
  }, [channelId]);

  const run = async () => {
    setRunning(true);
    setOutcome(null);
    setFailure(null);
    try {
      setOutcome(await api.probeAccount(channelId, uid, model || undefined));
    } catch (reason) {
      setFailure(errorText(reason, "探针失败"));
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="mt-0.5 rounded-lg border border-white/8 bg-black/25 px-2 py-1.5">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="text-[10px] text-zinc-500">测试模型</span>
        <Select
          value={model}
          options={
            models.length === 0
              ? [{ value: "", label: "该渠道没有可用模型" }]
              : models.map((entry) => ({ value: entry, label: entry }))
          }
          onChange={setModel}
          placeholder="选择模型"
          menuTitle="选择测试模型"
          className="h-6 w-48 text-[10px]"
          aria-label="选择测试模型"
        />
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            void run();
          }}
          disabled={running || models.length === 0}
          className="rounded-md bg-sky-400/15 px-2.5 py-0.5 text-[10px] text-sky-200 hover:bg-sky-400/25 disabled:opacity-40"
        >
          {running ? "测试中…" : "开始测试"}
        </button>
        <span className="text-[10px] text-zinc-600">
          真实发一条 "hi"，不走调度器、不影响其他账号
        </span>
      </div>

      {failure && (
        // 上游原文可能带换行/JSON：保留排版，能直接看清上游说了什么。
        <div className="mt-1 whitespace-pre-wrap break-all rounded-md border border-rose-400/25 bg-rose-400/8 px-2 py-1 font-mono text-[10px] leading-relaxed text-rose-100">
          {failure}
        </div>
      )}

      {outcome && (
        <div className="mt-1 flex flex-wrap items-center gap-3 rounded-md border border-emerald-400/20 bg-emerald-400/8 px-2 py-1 text-[10px] text-emerald-100">
          <span className="text-emerald-300">通过</span>
          <span>耗时 {outcome.latency_ms} ms</span>
          <span>
            用量 {outcome.prompt_tokens}/{outcome.completion_tokens}
          </span>
          <span className="min-w-0 flex-1 truncate" title={outcome.reply}>
            回复：{outcome.reply || "(空)"}
          </span>
          {outcome.upstream_model !== outcome.model && (
            <span className="text-emerald-200/70">上游 {outcome.upstream_model}</span>
          )}
        </div>
      )}
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
  // 列宽随账号昵称长度变化，表头与数据行共用这一份（所以永远对齐）。
  const columns = accountColumns(accounts);
  const [adding, setAdding] = useState(false);
  // 额度刷新已移到后端常驻定时任务（gateway/ops.rs）：不依赖面板打开、
  // 也不依赖停在哪个页面 —— 早期放在这里时，用网关跑对话积分变了界面也不刷新。
  const [expanded, setExpanded] = useState<string | null>(null);
  /** 展开任务清单的账号 uid（一次只开一个，避免表格被撑爆）。 */
  const [taskAccount, setTaskAccount] = useState<string | null>(null);
  /** 展开探针面板的账号 uid。 */
  const [probeAccount, setProbeAccount] = useState<string | null>(null);

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
          onClick={async (event) => {
            event.stopPropagation();
            // 全部执行：逐个账号启动，**已在执行的自动跳过**（后端会拒绝并回原因）。
            let started = 0;
            let skipped = 0;
            for (const account of accounts) {
              try {
                const outcome = await api.runTasks(account.channel_id, account.uid);
                if (outcome.started) {
                  started += 1;
                  writeTaskRun(`${account.channel_id}/${account.uid}`, {
                    running: true,
                    results: [],
                    error: null,
                  });
                } else {
                  skipped += 1;
                }
              } catch {
                skipped += 1;
              }
            }
            toast.success(`已启动 ${started} 个账号${skipped > 0 ? `，跳过 ${skipped} 个（执行中或失败）` : ""}`);
          }}
          disabled={accounts.length === 0}
          className="ml-auto flex items-center gap-1 rounded-md bg-sky-400/15 px-2.5 py-1 text-[11px] text-sky-200 hover:bg-sky-400/25 disabled:opacity-40"
        >
          <Play className="h-3.5 w-3.5" />
          全部执行
        </button>
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            setAdding(true);
          }}
          className="flex items-center gap-1 rounded-md bg-emerald-400/15 px-2.5 py-1 text-[11px] text-emerald-200 hover:bg-emerald-400/25"
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
          {/* 表头与数据行共用同一份列定义（`columns`）：宽度/间距/内边距必须一致，
              否则整列错开。整行铺满容器宽度，不做表格内横向滚动。 */}
          <div className="flex items-center gap-2 px-2 text-[10px] text-zinc-500">
            {columns.map((column) => (
              <span
                key={column.key}
                className={`${column.className} text-center`}
                style={column.style}
              >
                {column.label}
              </span>
            ))}
          </div>
          {accounts.map((account) => {
            const support = models[account.channel_id] ?? [];
            // 模型级限流的 key 是 upstream 名；退一步也用 id 兜一下（两者常常相同）。
            const limitsByModel = new Map(
              (account.model_limits ?? []).map((limit) => [limit.model, limit]),
            );
            return (
              <div
                key={`${account.channel_id}/${account.uid}`}
                className="flex flex-col"
              >
                <div className="flex items-center gap-2 rounded-lg border border-white/8 bg-white/[0.025] px-2 py-1.5">
                  {columns.map((column) => (
                    <span
                      key={column.key}
                      className={`${column.className} flex items-center justify-center gap-0.5`}
                      style={column.style}
                    >
                      {accountCell(account, column.key, {
                        support,
                        channelName: channelName(account.channel_id),
                        expanded: expanded === account.uid,
                        tasks: taskAccount === account.uid,
                        probe: probeAccount === account.uid,
                        onToggleModels: () =>
                          setExpanded(expanded === account.uid ? null : account.uid),
                        onToggleTasks: () =>
                          setTaskAccount(taskAccount === account.uid ? null : account.uid),
                        onToggleProbe: () =>
                          setProbeAccount(probeAccount === account.uid ? null : account.uid),
                        onChange,
                      })}
                    </span>
                  ))}
                </div>
                {expanded === account.uid && (
                  <div className="mt-0.5 rounded-lg border border-white/8 bg-black/25 px-2 py-1.5">
                    {support.length === 0 ? (
                      <span className="text-[10px] text-zinc-500">该渠道还没有模型目录</span>
                    ) : (
                      <div className="grid grid-cols-[repeat(auto-fill,minmax(230px,1fr))] gap-1.5">
                        {support.map((model) => (
                          <ModelCard
                            key={model.id}
                            model={model}
                            limit={limitsByModel.get(model.upstream) ?? limitsByModel.get(model.id)}
                          />
                        ))}
                      </div>
                    )}
                  </div>
                )}
                {taskAccount === account.uid && (
                  <AccountTasks channelId={account.channel_id} uid={account.uid} />
                )}
                {probeAccount === account.uid && (
                  <AccountProbe channelId={account.channel_id} uid={account.uid} />
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
 * 账号行里展开的任务区：任务清单 + 一键执行 + 行内领取。
 *
 * 任务挂在**账号**上（同一渠道不同账号的任务进度不同），所以它展开在账号行内。
 * 本轮执行结果**不另开区块**：每条任务的结果（状态/入账/原因）直接写回它自己那一行 ——
 * 一行就是一条任务的完整故事，不用两处对照着看。
 */
function AccountTasks({ channelId, uid }: { channelId: string; uid: string }) {
  const key = `${channelId}/${uid}`;
  // 执行状态来自模块级 store：切走再回来不会丢，也不会看起来像"被重置"。
  const runState = useTaskRunState(key);
  const [tasks, setTasks] = useState<api.GatewayTask[] | null>(null);
  /** 每日动作（签到 / 猫猫旅行 / 每日对话）：与任务分开列，语义不同。 */
  const [daily, setDaily] = useState<api.GatewayDailyAction[] | null>(null);
  const [claiming, setClaiming] = useState<string | null>(null);
  const [dailyBusy, setDailyBusy] = useState<string | null>(null);
  const [openNote, setOpenNote] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [nextTasks, nextDaily] = await Promise.all([
        api.listTasks(channelId),
        api.listDaily(channelId, uid).catch(() => null),
      ]);
      setTasks(nextTasks);
      setDaily(nextDaily);
      writeTaskRun(key, { error: null });
    } catch (reason) {
      setTasks(null);
      writeTaskRun(key, {
        error: errorText(reason, "读取任务失败"),
      });
    }
  }, [channelId, uid, key]);

  useEffect(() => {
    void load();
  }, [load]);

  // 后台执行：命令立即返回，进度靠轮询（逐条实时显示，切走也不丢）。
  const run = async () => {
    if (runState.running) return;
    try {
      const started = await api.runTasks(channelId, uid);
      if (!started.started) {
        toast.error(started.reason ?? "该账号的任务正在执行中");
        return;
      }
      writeTaskRun(key, { running: true, results: [], summary: null, error: null });
    } catch (reason) {
      const message = errorText(reason, "启动失败");
      writeTaskRun(key, { error: message });
      toast.error(message);
    }
  };

  // 执行中每秒拉一次进度；跑完自动刷新任务清单与额度（任务会真实改积分）。
  useEffect(() => {
    if (!runState.running) return;
    let cancelled = false;
    const timer = window.setInterval(async () => {
      try {
        const progress = await api.taskProgress(channelId, uid);
        if (cancelled) return;
        writeTaskRun(key, {
          running: progress.running,
          results: progress.results,
          summary: { done: progress.done, earned: progress.earned },
          error: progress.error ?? null,
        });
        if (!progress.running) {
          void load();
          void api.fetchQuota(channelId, uid).catch(() => null);
        }
      } catch {
        /* 轮询失败下一拍重试 */
      }
    }, 1000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [runState.running, channelId, uid, key, load]);

  /**
   * 单项领取：进度已达标但没领过的任务（含你在官方端手动做完的）。
   *
   * 结果并进同一份 results —— 行内渲染只有一条路径，不分"执行的"和"领取的"。
   */
  const claim = async (task: api.GatewayTask) => {
    if (claiming) return;
    setClaiming(task.code);
    try {
      const result = await api.claimTask(channelId, uid, task.code);
      const kept = (runState.results ?? []).filter((item) => item.code !== task.code);
      writeTaskRun(key, { results: [...kept, result] });
      if (result.earned > 0) {
        toast.success(`${task.name}：已领取 +${result.earned} 积分`);
      } else {
        toast.info(`${task.name}：${result.message}`);
      }
      void load();
      void api.fetchQuota(channelId, uid).catch(() => null);
    } catch (reason) {
      toast.error(errorText(reason, "领取失败"));
    } finally {
      setClaiming(null);
    }
  };

  /** 执行一个每日动作：单次请求，结果并进同一份 results（行内显示）。 */
  const runDailyAction = async (action: api.GatewayDailyAction) => {
    if (dailyBusy) return;
    setDailyBusy(action.code);
    try {
      const result = await api.runDaily(channelId, uid, action.code);
      const kept = (runState.results ?? []).filter((item) => item.code !== action.code);
      writeTaskRun(key, { results: [...kept, result] });
      if (result.status === "done" && result.earned > 0) {
        toast.success(`${action.name}：+${result.earned} 积分`);
      } else if (result.status === "done") {
        toast.info(`${action.name}：${result.message}`);
      } else {
        toast.error(`${action.name}：${result.message}`);
      }
      void load();
      // 签到/领奖会改积分：顺手刷一次额度，别等后端 5 分钟那轮。
      void api.fetchQuota(channelId, uid).catch(() => null);
    } catch (reason) {
      toast.error(errorText(reason, `${action.name}失败`));
    } finally {
      setDailyBusy(null);
    }
  };

  const { running, results, error } = runState;
  // 本轮结果按任务编号归位：每行只认自己那一条。
  const byCode = new Map((results ?? []).map((result) => [result.code, result]));
  const byCodeDaily = byCode;
  const automatable = (tasks ?? []).filter((task) => task.automatable).length;
  const claimable = (tasks ?? []).filter((task) => task.status === "claimable").length;

  return (
    <div className="mt-0.5 rounded-lg border border-white/8 bg-black/25 px-2 py-1.5">
      <div className="flex items-center gap-2">
        {running && (
          <span className="flex items-center gap-1 text-[10px] text-sky-300">
            <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-sky-400" />
            执行中 {(results ?? []).length} 项已完成
          </span>
        )}
        <span className="text-[10px] text-zinc-500" title={`账号 ${uid}`}>
          {tasks === null
            ? "读取中…"
            : tasks.length === 0
              ? "该账号没有任务清单"
              : `共 ${tasks.length} 项 · 可自动 ${automatable} 项${
                  claimable > 0 ? ` · 待领取 ${claimable} 项` : ""
                }`}
        </span>
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            void run();
          }}
          disabled={running || !tasks || tasks.length === 0}
          className="ml-auto rounded-md bg-emerald-400/15 px-2 py-0.5 text-[10px] text-emerald-200 hover:bg-emerald-400/25 disabled:opacity-40"
        >
          {running ? "执行中…" : "一键执行"}
        </button>
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            void load();
          }}
          className="rounded-md bg-white/8 px-2 py-0.5 text-[10px] text-zinc-300 hover:bg-white/14"
        >
          刷新
        </button>
      </div>

      {error && <div className="mt-1 text-[10px] text-amber-200">{error}</div>}

      {/* 每日动作：每天都要再做一次，所以**不混进任务清单**（那里是"做完就消失"）。
          清单由渠道按版本给：国内版签到+猫猫旅行，国际版每日活跃对话。 */}
      {daily !== null && daily.length > 0 && (
        <div className="mt-1 flex flex-col gap-0.5 rounded-md border border-white/8 bg-white/[0.02] px-1 py-0.5">
          <span className="px-0.5 text-[9px] uppercase tracking-wide text-zinc-600">每日</span>
          {daily.map((action) => {
            const outcome = byCodeDaily.get(action.code);
            return (
              <div key={action.code} className="flex items-center gap-2 text-[10px]">
                <span className="w-44 truncate text-zinc-200" title={action.name}>
                  {action.name}
                </span>
                <span
                  className={`w-20 ${outcome ? outcomeColor(outcome.status) : "text-zinc-400"}`}
                >
                  {outcome ? outcomeLabel(outcome.status) : action.done_today ? "今天已做" : "待做"}
                </span>
                <span
                  className="min-w-0 flex-1 truncate text-zinc-500"
                  title={outcome ? outcome.message : action.note}
                >
                  {outcome ? outcome.message : action.note}
                  {outcome && outcome.earned > 0 && (
                    <span className="text-emerald-300"> +{outcome.earned}</span>
                  )}
                </span>
                <button
                  type="button"
                  onClick={(event) => {
                    event.stopPropagation();
                    void runDailyAction(action);
                  }}
                  disabled={dailyBusy !== null || action.done_today}
                  className="shrink-0 rounded-md bg-white/8 px-2 py-0.5 text-[10px] text-zinc-300 hover:bg-white/16 disabled:opacity-40"
                >
                  {dailyBusy === action.code ? "执行中…" : action.action}
                </button>
              </div>
            );
          })}
        </div>
      )}

      {tasks !== null && tasks.length > 0 && (
        <div className="mt-1 flex flex-col gap-0.5">
          {tasks.map((task) => {
            const outcome = byCode.get(task.code);
            return (
              <div key={task.code} className="flex flex-col">
                <div className="flex items-center gap-2 rounded-md px-1 py-0.5 text-[10px] hover:bg-white/[0.05]">
                  <span className="w-44 truncate text-zinc-200" title={task.name}>
                    {task.name}
                  </span>
                  <span
                    className={`w-20 ${outcome ? outcomeColor(outcome.status) : "text-zinc-400"}`}
                    title={outcome ? "本轮执行结果" : "上游任务状态"}
                  >
                    {outcome ? outcomeLabel(outcome.status) : taskStatusLabel(task.status)}
                  </span>
                  <span className="w-14 font-mono text-zinc-500">{task.progress || "-"}</span>
                  <span className="w-14 text-emerald-300">
                    {task.reward > 0 ? `+${task.reward}` : "-"}
                  </span>
                  <span className="w-16">
                    {task.automatable ? (
                      <span className="text-zinc-500">可自动</span>
                    ) : (
                      <span className="text-amber-300">需手动</span>
                    )}
                  </span>
                  <span
                    className="min-w-0 flex-1 truncate text-zinc-500"
                    title={outcome ? outcome.message : task.note}
                  >
                    {outcome ? outcome.message : ""}
                    {outcome && outcome.earned > 0 && (
                      <span className="text-emerald-300"> +{outcome.earned}</span>
                    )}
                  </span>
                  {task.status === "claimable" ? (
                    <button
                      type="button"
                      onClick={(event) => {
                        event.stopPropagation();
                        void claim(task);
                      }}
                      disabled={claiming !== null}
                      className="shrink-0 rounded-md bg-amber-400/20 px-2 py-0.5 text-[10px] text-amber-100 hover:bg-amber-400/30 disabled:opacity-40"
                    >
                      {claiming === task.code ? "领取中…" : "领取"}
                    </button>
                  ) : (
                    task.note && (
                      <button
                        type="button"
                        onClick={(event) => {
                          event.stopPropagation();
                          setOpenNote(openNote === task.code ? null : task.code);
                        }}
                        title="为什么不能自动完成"
                        className="shrink-0 rounded-md bg-white/8 px-1.5 text-[10px] text-zinc-400 hover:bg-white/14"
                      >
                        ?
                      </button>
                    )
                  )}
                </div>
                {openNote === task.code && task.note && (
                  <div className="mb-0.5 ml-1 rounded-md border border-amber-400/25 bg-amber-400/8 px-2 py-0.5 text-[10px] text-amber-100">
                    {task.note}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}

      {running && (results ?? []).length === 0 && (
        <div className="mt-1 text-[10px] text-sky-200/80">
          正在逐项执行（每项可能需要多轮上报，请稍候）…
        </div>
      )}
    </div>
  );
}

/** 本轮执行结果的标签（上游任务自身状态另有一套文案）。 */
function outcomeLabel(status: string): string {
  if (status === "done") return "✓ 完成";
  if (status === "failed") return "✗ 失败";
  if (status === "not_in_window") return "不在窗口";
  if (status === "skipped") return "跳过";
  return status;
}

function outcomeColor(status: string): string {
  if (status === "done") return "text-emerald-300";
  if (status === "failed") return "text-rose-300";
  if (status === "not_in_window") return "text-amber-300";
  return "text-zinc-400";
}

function taskStatusLabel(status: string): string {
  if (status === "done") return "已完成";
  if (status === "claimable") return "可领取";
  if (status === "not_accepted") return "未接取";
  return "进行中";
}
