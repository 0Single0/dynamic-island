import React from "react";
import { createRoot } from "react-dom/client";
import "../src/index.css";
import { ProxyPoolExpanded } from "../src/widgets/proxy-pool";
import { ModelSettings } from "../src/pages/Settings/components/ModelSettings";

/**
 * 浏览器预览夹具：给网关面板与模型设置页喂确定性数据，
 * 用来在真 DOM 上量"滚动时高度如何变化"（不依赖 Tauri IPC）。
 */

const channels = [
  {
    id: "wb-cn",
    adapter: "workbuddy",
    display_name: "WorkBuddy 国内版",
    enabled: true,
    priority: 100,
    weight: 5,
    options: { realm: "cn" },
    accounts: 3,
    ready: 3,
  },
  {
    id: "wb-intl",
    adapter: "workbuddy",
    display_name: "WorkBuddy 国际版",
    enabled: true,
    priority: 100,
    weight: 5,
    options: { realm: "intl" },
    accounts: 1,
    ready: 1,
  },
  {
    id: "custom-1",
    adapter: "custom",
    display_name: "本机反代",
    enabled: true,
    priority: 50,
    weight: 3,
    options: {},
    accounts: 1,
    ready: 1,
  },
];

const accounts = Array.from({ length: 40 }, (_, index) => ({
  channel_id: index % 3 === 2 ? "custom-1" : "wb-cn",
  uid: `uid-${index}-9f8a7b6c5d4e3f2a1b0c9d8e7f6a5b4c`,
  nickname: index === 3 ? "一个特别特别长的昵称用来测列宽" : `test-${index}`,
  enabled: true,
  ready: index % 5 !== 4,
  has_credential: true,
  credential_expires_at_ms: Date.now() + 26 * 86_400_000,
  cooldown_until_ms: 0,
  blocked_reasons: index % 5 === 4 ? ["上游限流"] : [],
  model_limits: index % 4 === 3 ? [{ model: "glm-5.3-flash", until_ms: Date.now() + 600_000 }] : [],
  inflight: index % 4,
  max_inflight: 3,
  priority: 100,
  weight: 5,
  proxy_slot: null,
  quota: {
    label: "积分",
    unit: "点",
    remaining: 1200 - index * 70,
    total: 2000,
    used: 800 + index * 70,
    details: [["资源包", "标准包"]],
    fetched_at_ms: Date.now(),
  },
}));

const models = [
  {
    id: "glm-5.3-flash",
    upstream: "glm-5.3-flash",
    credits: "x0.20",
    free: false,
    context_length: 128000,
    max_output_tokens: 8192,
    capabilities: { vision: true, tool_calls: true, reasoning: true },
    caps_known: true,
    reasoning_efforts: ["low", "medium", "high"],
    reasoning_fixed_effort: null,
    reasoning_default_effort: "medium",
    reasoning_can_disable: true,
    always_reasoning: false,
  },
  {
    id: "glm-5.3",
    upstream: "glm-5.3",
    credits: "x1.00",
    free: false,
    context_length: 200000,
    max_output_tokens: 16384,
    capabilities: { vision: false, tool_calls: true, reasoning: true },
    caps_known: true,
    reasoning_efforts: ["medium", "high"],
    reasoning_fixed_effort: null,
    reasoning_default_effort: "high",
    reasoning_can_disable: false,
    always_reasoning: true,
  },
];

const series = Array.from({ length: 24 }, (_, hour) => ({
  ts: Date.now() - (23 - hour) * 3_600_000,
  requests: 3 + (hour % 7),
  total_tokens: 4000 + hour * 137,
  cached_tokens: 900 + hour * 31,
  completion_tokens: 700 + hour * 17,
  prompt_tokens: 2400 + hour * 89,
  credit: 0.4 + hour * 0.01,
}));

const usageRows = Array.from({ length: 120 }, (_, index) => ({
  ts: Date.now() - index * 60_000,
  model: "glm-5.3-flash",
  upstream_model: "glm-5.3-flash",
  channel_id: "wb-cn",
  account_uid: `uid-${index}`,
  status: index % 9 === 0 ? "upstream_aborted" : "completed",
  stream: true,
  attempts: 1,
  prompt_tokens: 1200,
  completion_tokens: 240,
  reasoning_tokens: 12,
  cached_tokens: 300,
  total_tokens: 1440,
  credit: 0.12,
  ttft_ms: 640,
  total_ms: 3200,
  tokens_per_second: 72,
  effort: "medium",
  estimated: false,
  error_detail: index % 9 === 0 ? "upstream 502: gateway exploded" : null,
}));

const usageSummary = {
  window_hours: 24,
  requests: 240,
  prompt_tokens: 320_000,
  completion_tokens: 61_000,
  cached_tokens: 120_000,
  total_tokens: 381_000,
  credit: 41.5,
  avg_ttft_ms: 700,
  cache_hit_rate: 0.31,
  by_channel: channels.map((channel, index) => ({
    key: channel.display_name,
    requests: 80 - index * 10,
    total_tokens: 120_000 - index * 20_000,
    cached_tokens: 40_000,
  })),
  by_model: models.map((model, index) => ({
    key: model.id,
    requests: 100 - index * 20,
    total_tokens: 200_000 - index * 40_000,
    cached_tokens: 60_000,
  })),
};

const providers = [
  {
    id: "builtin-gateway",
    name: "本机反代",
    protocol: "chat-completions",
    baseUrl: "http://127.0.0.1:8790/v1",
    hasApiKey: false,
    enabled: true,
    builtin: true,
    createdAt: "",
    updatedAt: "",
  },
  {
    id: "provider-wb",
    name: "wb",
    protocol: "responses",
    baseUrl: "http://127.0.0.1:8790/v1",
    hasApiKey: true,
    enabled: true,
    createdAt: "",
    updatedAt: "",
  },
];

const modelRecords = models.map((model, index) => ({
  id: `model-${index}`,
  providerId: index === 0 ? "builtin-gateway" : "provider-wb",
  modelId: model.id,
  displayName: model.id,
  contextWindow: model.context_length,
  supportsTools: true,
  supportsStreaming: true,
  supportsImages: false,
  enabled: true,
}));

const routes = {
  models: models.map((model, index) => ({
    model: model.id,
    routes: [
      {
        channel_id: index === 0 ? "wb-cn" : "wb-intl",
        upstream_model: model.upstream,
        priority: 100,
        weight: 5,
        free: false,
        credits: model.credits,
        context_windows: [model.context_length],
        capabilities: model.capabilities,
        reasoning_efforts: model.reasoning_efforts,
        reasoning_fixed_effort: null,
        reasoning_default_effort: model.reasoning_default_effort,
        reasoning_can_disable: true,
        always_reasoning: false,
        temperature: null,
        top_p: null,
        caps_known: true,
        source: "live",
        context: model.context_length,
        max_output: model.max_output_tokens,
      },
    ],
  })),
  live: 2,
  builtin: 1,
};

const status = {
  running: true,
  addr: "http://127.0.0.1:8790",
  error: null,
  enabled: true,
  port: 8790,
  actual_port: 8790,
  allow_lan: true,
  lan_addr: "http://10.8.13.67:8790",
  auth: "open" as const,
  api_keys: [],
  affinity: { mode: "prefer", entries: 12, capacity: 20000, ttl_seconds: 1800, lookups: 100, hits: 82, hit_rate: 0.82 },
  matrix: { routes: 6, live: 4, builtin: 2, ttl_seconds: 300 },
  concurrency: { global: 32, per_account: 3, available: 29 },
  stats: { requests: 240, failures: 12, retries: 3 },
  uptime_ms: 3_600_000,
  warnings: [],
};

const fixtures: Record<string, unknown> = {
  gateway_status: status,
  gateway_list_channels: channels,
  gateway_list_accounts: accounts,
  gateway_list_routes: routes,
  gateway_recent_usage: usageRows,
  gateway_usage_summary: usageSummary,
  gateway_usage_summary_range: usageSummary,
  gateway_usage_series: series,
  gateway_usage_series_range: series,
  gateway_paths: { data_dir: "C:/data/gateway", database: "C:/data/gateway/state.db" },
  gateway_recent_decisions: [],
  gateway_scheduler_snapshot: { accounts: [], affinity: status.affinity },
  gateway_probe_models: models.map((model) => model.id),
  gateway_list_tasks: [],
  gateway_list_daily: [],
  gateway_task_progress: { running: false, results: [], done: 0, earned: 0 },
  agent_list_provider_configs: providers,
  agent_list_model_configs: modelRecords,
};

Object.assign(window, {
  __TAURI_INTERNALS__: {
    transformCallback: (callback: (event: unknown) => void) => callback,
    invoke: async (command: string) => {
      if (command in fixtures) return fixtures[command];
      if (command.startsWith("plugin:event")) return 0;
      return null;
    },
  },
  __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
});

/**
 * 量一遍关键容器的高度，用于判断"滚动时高度是否塌陷"。
 * 只读真实 DOM（不改生产代码）：沿滚动容器向上取整条祖先链，
 * 任何一层高度随滚动变小/变负都会被抓到。
 */
function measure(label: string) {
  const scroller = document.querySelector<HTMLElement>(".gw-scroll");
  const chain: Array<{ cls: string; clientHeight: number; scrollHeight: number; height: number; top: number }> = [];
  for (let node: HTMLElement | null = scroller; node; node = node.parentElement) {
    const rect = node.getBoundingClientRect();
    chain.push({
      cls: (node.className || "").toString().slice(0, 60),
      clientHeight: node.clientHeight,
      scrollHeight: node.scrollHeight,
      height: Math.round(rect.height),
      top: Math.round(rect.top),
    });
    if (node === document.body) break;
  }
  return {
    label,
    scroller: scroller
      ? { clientHeight: scroller.clientHeight, scrollHeight: scroller.scrollHeight, scrollTop: scroller.scrollTop }
      : null,
    chain,
  };
}

function Harness() {
  const [view, setView] = React.useState<"pool" | "models">("pool");
  const [log, setLog] = React.useState<unknown[]>([]);

  React.useEffect(() => {
    const timer = window.setTimeout(() => {
      const scroller = document.querySelector<HTMLElement>(".gw-scroll");
      if (!scroller) return;
      const samples: unknown[] = [measure("initial")];
      const max = scroller.scrollHeight - scroller.clientHeight;
      for (let index = 1; index <= 5; index += 1) {
        scroller.scrollTop = Math.round((max * index) / 5);
        void scroller.offsetHeight;
        samples.push(measure(`top=${scroller.scrollTop}`));
      }
      setLog(samples);
      (window as unknown as Record<string, unknown>).__probe = samples;
    }, 1500);
    return () => window.clearTimeout(timer);
  }, [view]);

  return (
    <div className="flex w-screen flex-col overflow-hidden bg-[#0b0b0e] text-zinc-200" style={{ height: 820 }}>
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-white/10 px-2 text-[11px]">
        <button type="button" className="rounded bg-white/10 px-2 py-0.5" onClick={() => setView("pool")}>号池面板</button>
        <button type="button" className="rounded bg-white/10 px-2 py-0.5" onClick={() => setView("models")}>模型设置</button>
      </div>
      {/* 模拟工具窗口：外层定高，内容必须靠滚动容器吃掉溢出。 */}
      <div className="min-h-0 flex-1" style={{ padding: 12 }}>
        {view === "pool" ? <ProxyPoolExpanded /> : <ModelSettings />}
      </div>
      <pre className="hidden" data-log>{JSON.stringify(log, null, 2)}</pre>
    </div>
  );
}

const root = createRoot(document.getElementById("root")!);
root.render(<React.StrictMode><Harness /></React.StrictMode>);
import.meta.hot?.dispose(() => root.unmount());
