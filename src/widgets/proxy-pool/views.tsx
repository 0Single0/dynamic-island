import { useEffect, useState, type ReactNode } from "react";
import * as api from "@/api/gateway";
import type { GatewayChannel, GatewayStatus, GatewayUsageRow } from "@/api/gateway";
import { toast } from "@/utils/toast";
import { errorText, formatTokens } from "./shared";
import { CustomProviderDialog } from "./CustomProviderDialog";

// 用量视图独立成文件（图多、结构不同），这里只做转发。
export { UsageView } from "./usage";

/**
 * 请求日志：一条请求一行（时间/模型/强度/账号/模式/结果/耗时/首字/速度/各类 token/积分）。
 *
 * 列定义只有**这一份**（表头与数据行共用）：以前是两处各自硬编码，
 * 改一处忘另一处就会整表错位 —— 已经错过一次了。
 * 行容器 w-max + 横向滚动：列宽按内容撑开，看不全就左右滚，不压缩截断。
 *
 * 失败的行左侧泛红，并带一个 `?`：点开就是**上游返回的原文**。
 */
/** 每页行数：日志一行信息密度高，50 条在展开面板里大约两屏。 */
const LOG_PAGE_SIZE = 50;

export function RequestLogView() {
  const [rows, setRows] = useState<GatewayUsageRow[]>([]);
  const [opened, setOpened] = useState<string | null>(null);
  const [page, setPage] = useState(1);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const next = await api.recentUsage(0);
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

  // 新数据到达时页数可能收缩（后端保留窗口有限），把页码夹回有效范围。
  const pageCount = Math.max(1, Math.ceil(rows.length / LOG_PAGE_SIZE));
  const safePage = Math.min(page, pageCount);
  const pageRows = rows.slice((safePage - 1) * LOG_PAGE_SIZE, safePage * LOG_PAGE_SIZE);

  if (rows.length === 0) {
    return (
      <p className="px-1 py-2 text-[11px] text-zinc-500">
        还没有请求。把客户端的 base URL 指到本机端口，请求就会出现在这里。
      </p>
    );
  }

  /** 唯一的列定义：宽度类 + 表头。行渲染按同一顺序取 cell。 */
  const columns: Array<{ key: string; width: string; label: string }> = [
    { key: "ts", width: "w-16", label: "时间" },
    { key: "model", width: "w-40", label: "模型" },
    { key: "effort", width: "w-12", label: "强度" },
    { key: "account", width: "w-24", label: "账号" },
    { key: "stream", width: "w-12", label: "模式" },
    { key: "status", width: "w-16", label: "结果" },
    { key: "total_ms", width: "w-16", label: "耗时" },
    { key: "ttft", width: "w-16", label: "首字" },
    { key: "speed", width: "w-14", label: "速度" },
    { key: "prompt", width: "w-16", label: "输入" },
    { key: "completion", width: "w-14", label: "输出" },
    { key: "reasoning", width: "w-12", label: "思考" },
    { key: "cache", width: "w-12", label: "缓存" },
    { key: "total", width: "w-16", label: "总 token" },
    { key: "credit", width: "w-12", label: "积分" },
  ];

  /** 单元格渲染：key 对齐列定义，样式统一（等宽数字用 font-mono）。 */
  const cell = (row: GatewayUsageRow, key: string): ReactNode => {
    const failed = row.status !== "completed" || row.error_detail !== null;
    const mono = "font-mono";
    switch (key) {
      case "ts":
        return <span className={`${mono} text-zinc-500`}>{formatClock(row.ts)}</span>;
      case "model":
        return (
          <span className="text-zinc-200" title={row.upstream_model}>
            {row.model}
          </span>
        );
      case "effort":
        return <span className="text-zinc-500">{row.effort ?? "-"}</span>;
      case "account":
        return (
          <span className="text-zinc-400" title={row.account_uid}>
            {row.account_uid.slice(0, 8)}
          </span>
        );
      case "stream":
        return <span className="text-zinc-500">{row.stream ? "流式" : "非流"}</span>;
      case "status":
        return (
          <span className={`flex items-center justify-center gap-0.5 ${failed ? "text-rose-300" : "text-emerald-300"}`}>
            {failed ? statusLabel(row.status) : "完成"}
            {failed && row.error_detail && (
              <span
                role="button"
                tabIndex={0}
                onClick={(event) => {
                  event.stopPropagation();
                  setOpened(opened === `${row.ts}-${row.account_uid}` ? null : `${row.ts}-${row.account_uid}`);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    setOpened(opened === `${row.ts}-${row.account_uid}` ? null : `${row.ts}-${row.account_uid}`);
                  }
                }}
                title="查看上游返回"
                className="flex h-3.5 w-3.5 cursor-pointer items-center justify-center rounded-full bg-rose-400/25 text-[8px] text-rose-100 hover:bg-rose-400/45"
              >
                ?
              </span>
            )}
          </span>
        );
      case "total_ms":
        return <span className={mono}>{row.total_ms} ms</span>;
      case "ttft":
        return (
          <span className={mono}>{row.ttft_ms === null ? "-" : `${row.ttft_ms} ms`}</span>
        );
      case "speed":
        return (
          <span className={mono}>
            {row.tokens_per_second > 0 ? `${row.tokens_per_second}/s` : "-"}
          </span>
        );
      case "prompt":
        return <span className={mono}>{formatTokens(row.prompt_tokens)}</span>;
      case "completion":
        return <span className={mono}>{formatTokens(row.completion_tokens)}</span>;
      case "reasoning":
        return (
          <span className="text-zinc-500">
            {row.reasoning_tokens > 0 ? formatTokens(row.reasoning_tokens) : "-"}
          </span>
        );
      case "cache":
        return (
          <span className="text-zinc-500">
            {row.total_tokens > 0
              ? `${Math.round((row.cached_tokens / row.total_tokens) * 100)}%`
              : "-"}
          </span>
        );
      case "total":
        return <span className={`font-medium ${mono}`}>{formatTokens(row.total_tokens)}</span>;
      case "credit":
        return (
          <span
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
        );
      default:
        return null;
    }
  };

  return (
    // 外层 h-full + overflow-hidden：分页栏钉在底部，不跟表格一起滚。
    <div className="flex h-full min-h-0 flex-col">
      <div className="gw-scroll gw-scroll-x min-h-0 flex-1 overflow-auto pr-1">
        <div className="w-max min-w-full flex flex-col gap-0.5">
          <div className="flex w-max min-w-full items-center gap-2 px-2 pb-1 text-[10px] text-zinc-500">
            {columns.map((column) => (
              <span key={column.key} className={`${column.width} shrink-0 text-center`}>
                {column.label}
              </span>
            ))}
          </div>
          {pageRows.map((row) => {
            const key = `${row.ts}-${row.account_uid}-${row.model}`;
            const failed = row.status !== "completed" || row.error_detail !== null;
            return (
              <div key={key} className="flex flex-col">
                <div
                  className={`flex w-max min-w-full items-center gap-2 rounded-md px-2 py-1 text-[10px] ${
                    failed ? "bg-rose-400/[0.06]" : "bg-white/[0.02]"
                  }`}
                >
                  {columns.map((column) => (
                    <span
                      key={column.key}
                      className={`${column.width} flex shrink-0 items-center justify-center text-center`}
                    >
                      {cell(row, column.key)}
                    </span>
                  ))}
                </div>
                {opened === `${row.ts}-${row.account_uid}` && row.error_detail && (
                  <pre className="mb-1 mt-0.5 max-h-40 overflow-auto whitespace-pre-wrap rounded-md border border-rose-400/25 bg-black/45 px-2 py-1.5 text-[10px] leading-relaxed text-rose-100">
                    {row.error_detail}
                  </pre>
                )}
              </div>
            );
          })}
        </div>
      </div>

      <div className="mt-1 flex shrink-0 items-center justify-end gap-2 px-1 text-[10px] text-zinc-500">
        <span className="mr-auto ml-1">共 {rows.length} 条</span>
        <span>
          共 {rows.length} 条 · 第 {safePage}/{pageCount} 页
        </span>
        <button
          type="button"
          onClick={() => setPage(Math.max(1, safePage - 1))}
          disabled={safePage <= 1}
          className="rounded-md bg-white/8 px-2 py-0.5 text-zinc-300 hover:bg-white/14 disabled:opacity-35"
        >
          上一页
        </button>
        <button
          type="button"
          onClick={() => setPage(Math.min(pageCount, safePage + 1))}
          disabled={safePage >= pageCount}
          className="rounded-md bg-white/8 px-2 py-0.5 text-zinc-300 hover:bg-white/14 disabled:opacity-35"
        >
          下一页
        </button>
      </div>
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
  /** 正在编辑的自定义供应商（null = 未开弹窗）。 */
  const [editing, setEditing] = useState<GatewayChannel | null>(null);

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
          {channels.map((channel) => {
            // 自定义供应商可以改地址/密钥/模型，也能删掉（连账号）；内置渠道只能启停。
            const custom = channel.adapter === "custom";
            return (
              <div key={channel.id} className="flex items-center gap-2 text-[11px]">
                <span
                  className={`h-1.5 w-1.5 rounded-full ${
                    channel.enabled ? "bg-emerald-400" : "bg-zinc-500"
                  }`}
                />
                <span className="min-w-0 flex-[1.2] truncate text-zinc-200">
                  {channel.display_name || channel.id}
                </span>
                <span className="w-20 shrink-0 text-[10px] text-zinc-500">
                  {custom ? "自定义" : channel.adapter}
                </span>
                <span className="w-24 shrink-0 text-[10px] text-zinc-500">
                  账号 {channel.accounts} · 可用 {channel.ready}
                </span>
                <span className="ml-auto flex shrink-0 items-center gap-1">
                  {custom && (
                    <button
                      type="button"
                      onClick={(event) => {
                        event.stopPropagation();
                        setEditing(channel);
                      }}
                      className="rounded-md bg-white/10 px-2 py-0.5 text-[10px] text-zinc-200 hover:bg-white/16"
                    >
                      编辑
                    </button>
                  )}
                  {custom && (
                    <button
                      type="button"
                      onClick={async (event) => {
                        event.stopPropagation();
                        try {
                          const result = await api.deleteChannel(channel.id);
                          toast.success(
                            `已删除 ${channel.display_name || channel.id}（含 ${result.removed_accounts} 个账号）`,
                          );
                          onChange();
                        } catch (reason) {
                          toast.error(errorText(reason, "删除失败"));
                        }
                      }}
                      className="rounded-md bg-rose-400/15 px-2 py-0.5 text-[10px] text-rose-200 hover:bg-rose-400/25"
                    >
                      删除
                    </button>
                  )}
                  <button
                    type="button"
                    onClick={async (event) => {
                      event.stopPropagation();
                      await api.updateChannel({ channelId: channel.id, enabled: !channel.enabled });
                      toast.success(channel.enabled ? `已停用 ${channel.id}` : `已启用 ${channel.id}`);
                      onChange();
                    }}
                    className="rounded-md bg-white/10 px-2 py-0.5 text-[10px] hover:bg-white/16"
                  >
                    {channel.enabled ? "停用" : "启用"}
                  </button>
                </span>
              </div>
            );
          })}
        </div>
      </Panel>

      <Panel title="HTTP 服务">
        {/* 地址显示**实际监听**的（端口被占用会回退，配置端口≠实际端口）；
            点击即复制。内网开关开着时再给一行局域网地址。 */}
        {(() => {
          const addr = status?.addr ?? "-";
          return (
            <div className="flex flex-col gap-1.5">
              <div className="flex items-center gap-2 text-[11px]">
                <button
                  type="button"
                  onClick={(event) => {
                    event.stopPropagation();
                    void navigator.clipboard?.writeText(addr);
                    toast.success(`已复制 ${addr}`);
                  }}
                  title="点击复制"
                  className="min-w-0 flex-1 truncate rounded-md bg-white/[0.04] px-2 py-1 text-left font-mono text-[11px] text-zinc-100 hover:bg-white/10"
                >
                  {addr}
                </button>
                <button
                  type="button"
                  onClick={(event) => {
                    event.stopPropagation();
                    void navigator.clipboard?.writeText(addr);
                    toast.success(`已复制 ${addr}`);
                  }}
                  className="shrink-0 rounded-md bg-white/10 px-2 py-1 text-[10px] text-zinc-300 hover:bg-white/16"
                >
                  复制
                </button>
                <button
                  type="button"
                  onClick={async (event) => {
                    event.stopPropagation();
                    await api.setRuntime({ enabled: !status?.enabled });
                    toast.success("已保存，重启岛后生效");
                    onChange();
                  }}
                  className="shrink-0 rounded-md bg-white/10 px-2 py-0.5 text-[10px] hover:bg-white/16"
                >
                  {status?.enabled ? "关闭服务" : "开启服务"}
                </button>
              </div>

              <div className="flex items-center gap-2 text-[10px] text-zinc-500">
                <span>
                  {status?.enabled ? "随岛启动" : "已关闭"} · 实际端口{" "}
                  {status?.actual_port ?? status?.port ?? "-"}
                  {(status?.actual_port ?? status?.port) !== status?.port &&
                    status?.port !== undefined &&
                    `（配置的 ${status.port} 被占用，已回退）`}
                </span>
              </div>

              {/* 内网访问：绑 0.0.0.0，给出可复制的局域网地址 */}
              <div className="flex items-center gap-2 text-[10px]">
                <button
                  type="button"
                  onClick={async (event) => {
                    event.stopPropagation();
                    await api.setRuntime({ allowLan: !status?.allow_lan });
                    toast.success(
                      status?.allow_lan ? "已关闭内网访问，重启岛后生效" : "已开启内网访问，重启岛后生效",
                    );
                    onChange();
                  }}
                  className={`shrink-0 rounded-md px-2 py-0.5 text-[10px] transition-colors ${
                    status?.allow_lan
                      ? "bg-emerald-400/20 text-emerald-200 hover:bg-emerald-400/30"
                      : "bg-white/8 text-zinc-400 hover:bg-white/14"
                  }`}
                >
                  {status?.allow_lan ? "内网访问：开" : "内网访问：关"}
                </button>
                {status?.allow_lan && status?.lan_addr && (
                  <>
                    <button
                      type="button"
                      onClick={(event) => {
                        event.stopPropagation();
                        void navigator.clipboard?.writeText(status.lan_addr ?? "");
                        toast.success(`已复制 ${status.lan_addr}`);
                      }}
                      title="点击复制"
                      className="min-w-0 flex-1 truncate rounded-md bg-white/[0.04] px-2 py-1 text-left font-mono text-sky-200/90 hover:bg-white/10"
                    >
                      {status.lan_addr}
                    </button>
                    <button
                      type="button"
                      onClick={(event) => {
                        event.stopPropagation();
                        void navigator.clipboard?.writeText(status.lan_addr ?? "");
                        toast.success(`已复制 ${status.lan_addr}`);
                      }}
                      className="shrink-0 rounded-md bg-white/10 px-2 py-1 text-zinc-300 hover:bg-white/16"
                    >
                      复制
                    </button>
                  </>
                )}
                {status?.allow_lan && !status?.lan_addr && (
                  <span className="text-amber-200/80">重启岛后显示内网地址</span>
                )}
              </div>
              {status?.allow_lan && (status?.api_keys ?? []).length === 0 && (
                <p className="text-[10px] text-amber-200/80">
                  内网开放时建议在下方配置 API Key —— 否则同一局域网内的任何人都能调用号池。
                </p>
              )}
              <p className="text-[10px] text-zinc-500">
                客户端 base URL 指过来即可：`/v1/chat/completions`、`/v1/messages`、`/v1/responses`。
              </p>
            </div>
          );
        })()}
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

      {editing && (
        <CustomProviderDialog
          editing={{
            channelId: editing.id,
            displayName: editing.display_name || editing.id,
            options: editing.options,
            priority: editing.priority,
          }}
          onClose={() => setEditing(null)}
          onSaved={onChange}
        />
      )}
    </div>
  );
}

export function Panel({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="rounded-xl border border-white/8 bg-white/[0.025] p-2.5">
      <div className="mb-1.5 text-[11px] text-zinc-400">{title}</div>
      {children}
    </section>
  );
}
