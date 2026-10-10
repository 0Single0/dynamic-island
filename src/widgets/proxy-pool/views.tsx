import { useEffect, useState } from "react";
import * as api from "@/api/gateway";
import type { GatewayChannel, GatewayStatus, GatewayUsageRow } from "@/api/gateway";
import { toast } from "@/utils/toast";
import { formatTokens } from "./shared";

// 用量视图独立成文件（图多、结构不同），这里只做转发。
export { UsageView } from "./usage";

/**
 * 请求日志：一条请求一行（时间/模型/强度/账号/模式/结果/耗时/首字/速度/各类 token/积分）。
 *
 * 失败的行左侧泛红，并带一个 `?`：点开就是**上游返回的原文** —— 排查时最想知道的就是这个。
 */
export function RequestLogView() {
  const [rows, setRows] = useState<GatewayUsageRow[]>([]);
  const [opened, setOpened] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const next = await api.recentUsage(200);
        if (!cancelled) setRows(next);
      } catch {
        /* 浏览器预览模式忽略 */
      }
    };
    void load();
    const timer = window.setInterval(() => void load(), 2000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  if (rows.length === 0) {
    return (
      <p className="px-1 py-2 text-[11px] text-zinc-500">
        还没有请求。把客户端的 base URL 指到本机端口，请求就会出现在这里。
      </p>
    );
  }

  const columns: Array<[string, string]> = [
    ["w-14", "时间"],
    ["min-w-0 flex-[1.4]", "模型"],
    ["w-10", "强度"],
    ["min-w-0 flex-[0.8]", "账号"],
    ["w-10", "模式"],
    ["w-14", "结果"],
    ["w-14", "耗时"],
    ["w-14", "首字"],
    ["w-14", "速度"],
    ["w-14", "输入"],
    ["w-12", "输出"],
    ["w-10", "思考"],
    ["w-12", "缓存"],
    ["w-16", "总 token"],
    ["w-10", "积分"],
    ["w-6", "?"],
  ];

  return (
    <div className="flex flex-col gap-0.5">
      <div className="flex items-center gap-2 px-2 pb-1 text-[10px] text-zinc-500">
        {columns.map(([width, label]) => (
          <span key={label} className={`${width} shrink-0 text-center`}>
            {label}
          </span>
        ))}
      </div>
      {rows.map((row) => {
        const key = `${row.ts}-${row.account_uid}-${row.model}`;
        const failed = row.status !== "completed" || row.error_detail !== null;
        return (
          <div key={key} className="flex flex-col">
            <div
              className={`flex items-center gap-2 overflow-hidden rounded-md px-2 py-1 text-[10px] ${
                failed ? "bg-rose-400/[0.06]" : "bg-white/[0.02]"
              }`}
            >
              <span className="w-14 shrink-0 font-mono text-zinc-500">{formatClock(row.ts)}</span>
              <span className="min-w-0 flex-[1.4] truncate text-zinc-200" title={row.upstream_model}>
                {row.model}
              </span>
              <span className="w-10 shrink-0 text-zinc-500">{row.effort ?? "-"}</span>
              <span className="min-w-0 flex-[0.8] truncate text-zinc-400" title={row.account_uid}>
                {row.account_uid.slice(0, 8)}
              </span>
              <span className="w-10 shrink-0 text-zinc-500">{row.stream ? "流式" : "非流"}</span>
              <span className={`w-14 shrink-0 ${failed ? "text-rose-300" : "text-emerald-300"}`}>
                {failed ? statusLabel(row.status) : "完成"}
              </span>
              <span className="w-14 shrink-0 text-zinc-400">{row.total_ms} ms</span>
              <span className="w-14 shrink-0 text-zinc-400">
                {row.ttft_ms === null ? "-" : `${row.ttft_ms} ms`}
              </span>
              <span className="w-14 shrink-0 text-zinc-400">
                {row.tokens_per_second > 0 ? `${row.tokens_per_second}/s` : "-"}
              </span>
              <span className="w-14 shrink-0 text-zinc-400">{formatTokens(row.prompt_tokens)}</span>
              <span className="w-12 shrink-0 text-zinc-400">
                {formatTokens(row.completion_tokens)}
              </span>
              <span className="w-10 shrink-0 text-zinc-500">
                {row.reasoning_tokens > 0 ? formatTokens(row.reasoning_tokens) : "-"}
              </span>
              <span className="w-12 shrink-0 text-zinc-500">
                {row.total_tokens > 0
                  ? `${Math.round((row.cached_tokens / row.total_tokens) * 100)}%`
                  : "-"}
              </span>
              <span className="w-16 shrink-0 font-medium text-zinc-100">
                {formatTokens(row.total_tokens)}
              </span>
              <span
                className="w-10 shrink-0 text-center text-zinc-500"
                title={
                  row.credit !== null
                    ? `上游积分：${row.credit}`
                    : row.estimated
                      ? "上游没给 usage：这一行的 token 是本地估算，积分不可用"
                      : "上游这次没返回积分"
                }
              >
                {row.credit === null ? (row.estimated ? "估" : "-") : row.credit.toFixed(2)}
              </span>
              <span className="w-6 shrink-0">
                {row.error_detail && (
                  <button
                    type="button"
                    onClick={(event) => {
                      event.stopPropagation();
                      setOpened(opened === key ? null : key);
                    }}
                    title="查看上游返回"
                    className="flex h-4 w-4 items-center justify-center rounded-full bg-rose-400/25 text-[9px] text-rose-100 hover:bg-rose-400/45"
                  >
                    ?
                  </button>
                )}
              </span>
            </div>
            {opened === key && row.error_detail && (
              <pre className="mb-1 mt-0.5 max-h-40 overflow-auto whitespace-pre-wrap rounded-md border border-rose-400/25 bg-black/45 px-2 py-1.5 text-[10px] leading-relaxed text-rose-100">
                {row.error_detail}
              </pre>
            )}
          </div>
        );
      })}
    </div>
  );
}

function statusLabel(status: string): string {
  if (status === "client_aborted") return "客户端中断";
  if (status === "upstream_aborted") return "上游中断";
  return "失败";
}

function formatClock(ts: number): string {
  const date = new Date(ts);
  return [date.getHours(), date.getMinutes(), date.getSeconds()]
    .map((part) => String(part).padStart(2, "0"))
    .join(":");
}

export function SettingsView({
  status,
  channels,
  onChange,
}: {
  status: GatewayStatus | null;
  channels: GatewayChannel[];
  onChange: () => void;
}) {
  const [paths, setPaths] = useState<{ data_dir: string; database: string } | null>(null);
  const [keyInput, setKeyInput] = useState("");

  useEffect(() => {
    void api
      .paths()
      .then(setPaths)
      .catch(() => setPaths(null));
  }, []);

  return (
    <div className="flex flex-col gap-3">
      <Panel title="渠道">
        {channels.length === 0 && <div className="text-[10px] text-zinc-600">没有配置渠道</div>}
        <div className="flex flex-col gap-1">
          {channels.map((channel) => (
            <div key={channel.id} className="flex items-center gap-2 text-[11px]">
              <span
                className={`h-1.5 w-1.5 rounded-full ${
                  channel.enabled ? "bg-emerald-400" : "bg-zinc-500"
                }`}
              />
              <span className="min-w-0 flex-[1.2] truncate text-zinc-200">
                {channel.display_name || channel.id}
              </span>
              <span className="w-20 shrink-0 text-[10px] text-zinc-500">{channel.adapter}</span>
              <span className="w-24 shrink-0 text-[10px] text-zinc-500">
                账号 {channel.accounts} · 可用 {channel.ready}
              </span>
              <button
                type="button"
                onClick={async (event) => {
                  event.stopPropagation();
                  await api.updateChannel({ channelId: channel.id, enabled: !channel.enabled });
                  toast.success(channel.enabled ? `已停用 ${channel.id}` : `已启用 ${channel.id}`);
                  onChange();
                }}
                className="ml-auto shrink-0 rounded-md bg-white/10 px-2 py-0.5 text-[10px] hover:bg-white/16"
              >
                {channel.enabled ? "停用" : "启用"}
              </button>
            </div>
          ))}
        </div>
      </Panel>

      <Panel title="HTTP 服务">
        <div className="flex items-center gap-2 text-[11px]">
          <span className="text-zinc-300">{status?.addr ?? "-"}</span>
          <span className="text-zinc-500">
            {status?.enabled ? "随岛启动" : "已关闭"} · 端口 {status?.port ?? "-"}
          </span>
          <button
            type="button"
            onClick={async (event) => {
              event.stopPropagation();
              await api.setRuntime({ enabled: !status?.enabled });
              toast.success("已保存，重启岛后生效");
              onChange();
            }}
            className="ml-auto shrink-0 rounded-md bg-white/10 px-2 py-0.5 text-[10px] hover:bg-white/16"
          >
            {status?.enabled ? "关闭服务" : "开启服务"}
          </button>
        </div>
        <p className="mt-1.5 text-[10px] text-zinc-500">
          客户端 base URL 指过来即可：`/v1/chat/completions`、`/v1/messages`、`/v1/responses`。
        </p>
      </Panel>

      <Panel title="下游 API Key">
        <p className="text-[10px] text-zinc-500">
          留空 = 仅本机免鉴权（只绑回环）。填了之后客户端必须带 Bearer 或 x-api-key。
        </p>
        <div className="mt-2 flex items-center gap-1.5">
          <input
            value={keyInput}
            onChange={(event) => setKeyInput(event.target.value)}
            placeholder="sk-..."
            className="h-6 min-w-0 flex-1 rounded-md border border-white/10 bg-black/30 px-2 font-mono text-[10px] text-zinc-200 outline-none focus:border-sky-400/50"
          />
          <button
            type="button"
            onClick={async (event) => {
              event.stopPropagation();
              if (!keyInput.trim()) return;
              await api.setRuntime({ apiKeys: [...(status?.api_keys ?? []), keyInput.trim()] });
              setKeyInput("");
              toast.success("已添加，重启岛后生效");
              onChange();
            }}
            className="shrink-0 rounded-md bg-white/10 px-2 py-0.5 text-[10px] hover:bg-white/16"
          >
            添加
          </button>
          <button
            type="button"
            onClick={async (event) => {
              event.stopPropagation();
              await api.setRuntime({ apiKeys: [] });
              toast.success("已清空（恢复本机免鉴权）");
              onChange();
            }}
            className="shrink-0 rounded-md bg-white/10 px-2 py-0.5 text-[10px] hover:bg-white/16"
          >
            清空
          </button>
        </div>
        <div className="mt-2 flex flex-wrap gap-1">
          {(status?.api_keys ?? []).map((hash) => (
            <span
              key={hash}
              className="rounded bg-white/6 px-1.5 py-0.5 font-mono text-[10px] text-zinc-400"
            >
              {hash}…
            </span>
          ))}
        </div>
      </Panel>

      <Panel title="会话亲和">
        <div className="flex items-center gap-3 text-[10px] text-zinc-500">
          <span>模式 {status?.affinity.mode ?? "-"}</span>
          <span>绑定 {status?.affinity.entries ?? 0}</span>
          <span>命中率 {Math.round((status?.affinity.hit_rate ?? 0) * 100)}%</span>
          <span>TTL {status?.affinity.ttl_seconds ?? 0}s</span>
          <button
            type="button"
            onClick={async (event) => {
              event.stopPropagation();
              const result = await api.clearAffinity();
              toast.success(`已清空 ${result.cleared} 条绑定`);
              onChange();
            }}
            className="ml-auto shrink-0 rounded-md bg-white/10 px-2 py-0.5 text-[10px] hover:bg-white/16"
          >
            清空绑定
          </button>
        </div>
        <p className="mt-1.5 text-[10px] text-zinc-500">
          亲和把同一会话钉在同一账号上，保住上游前缀缓存命中；清空后下次请求重新选号。
        </p>
      </Panel>

      <Panel title="数据位置">
        <div className="font-mono text-[10px] text-zinc-500">{paths?.data_dir ?? "-"}</div>
        <div className="font-mono text-[10px] text-zinc-500">{paths?.database ?? "-"}</div>
      </Panel>
    </div>
  );
}

function Panel({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="rounded-xl border border-white/8 bg-white/[0.025] p-2.5">
      <div className="mb-1.5 text-[11px] text-zinc-400">{title}</div>
      {children}
    </section>
  );
}
