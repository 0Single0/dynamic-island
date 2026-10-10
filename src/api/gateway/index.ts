import { invoke } from "@tauri-apps/api/core";

/**
 * 反代号池网关 API：与 src-tauri/src/gateway/commands.rs 一一对应。
 *
 * 面板走 Tauri command（同进程直连状态），不走本地 HTTP —— 免鉴权、免 CORS，
 * 也不依赖 HTTP 服务是否已经起来。
 */

export interface GatewayChannel {
  id: string;
  adapter: string;
  display_name: string;
  enabled: boolean;
  priority: number;
  weight: number;
  options: unknown;
  accounts: number;
  ready: number;
}

/**
 * 渠道无关的额度快照：面板只按 label + unit 展示，渠道自己翻译口径
 * （workbuddy = 积分，将来 qoder 可能是额度、kiro 可能是试用期）。
 */
export interface GatewayQuota {
  label: string;
  unit: string;
  remaining: number | null;
  total: number | null;
  used: number | null;
  details: Array<[string, string]>;
  fetched_at_ms: number;
}

export interface GatewayAccount {
  channel_id: string;
  uid: string;
  nickname: string;
  enabled: boolean;
  ready: boolean;
  has_credential: boolean;
  credential_expires_at_ms: number | null;
  cooldown_until_ms: number;
  /** 不可用原因（冷却 / 熔断 / 额度停放 / 没凭据 / 停用），供状态旁的 tooltip。 */
  blocked_reasons: string[];
  inflight: number;
  max_inflight: number;
  priority: number;
  weight: number;
  proxy_slot: string | null;
  quota: GatewayQuota | null;
}

export interface GatewayStatus {
  running: boolean;
  addr: string | null;
  error: string | null;
  enabled: boolean;
  port: number;
  auth: "open" | "api_key";
  api_keys: string[];
  affinity: {
    mode: string;
    entries: number;
    capacity: number;
    ttl_seconds: number;
    lookups: number;
    hits: number;
    hit_rate: number;
  };
  matrix: {
    routes: number;
    live: number;
    builtin: number;
    ttl_seconds: number;
  };
  concurrency: { global: number; per_account: number; available: number };
  stats: { requests: number; failures: number; retries: number };
  uptime_ms: number;
  warnings: string[];
}

export interface GatewayRouteEntry {
  channel_id: string;
  upstream_model: string;
  priority: number;
  weight: number;
  free: boolean;
  /** 倍率字符串：x0.00 = 免费。 */
  credits: string | null;
  context_windows: number[];
  capabilities: { vision: boolean; tool_calls: boolean; reasoning: boolean };
  reasoning_efforts: string[];
  reasoning_fixed_effort: string | null;
  reasoning_default_effort: string | null;
  reasoning_can_disable: boolean | null;
  always_reasoning: boolean;
  temperature: number | null;
  top_p: number | null;
  caps_known: boolean;
  source: string;
  context: number | null;
  max_output: number | null;
}

export interface GatewayModelRoutes {
  model: string;
  routes: GatewayRouteEntry[];
}

export interface GatewayDecision {
  trace: string;
  ts: number;
  model: string;
  stream: boolean;
  session: string | null;
  candidates: number;
  rejected: string[];
  attempts: number;
  via_affinity: boolean;
  channel: string | null;
  account: string | null;
  upstream_model: string | null;
  error: { kind: string; message: string } | null;
}

export interface GatewayUsage {
  window_hours: number;
  requests: number;
  prompt_tokens: number;
  completion_tokens: number;
  cached_tokens: number;
  total_tokens: number;
  credit: number;
  avg_ttft_ms: number;
  cache_hit_rate: number;
  by_channel: Array<{ key: string; requests: number; total_tokens: number; cached_tokens: number }>;
  by_model: Array<{ key: string; requests: number; total_tokens: number; cached_tokens: number }>;
}

export interface GatewayLoginTicket {
  state: string;
  auth_url: string;
}

export interface GatewayLoginResult {
  status: "pending" | "ok" | "failed";
  message?: string;
  account?: { channel_id: string; uid: string; nickname: string };
}

export interface GatewaySchedulerSnapshot {
  accounts: Array<{
    key: string;
    enabled: boolean;
    ready: boolean;
    inflight: number;
    cooldown_ms: number;
    priority: number;
    weight: number;
  }>;
  affinity: GatewayStatus["affinity"];
}

export function status() {
  return invoke<GatewayStatus>("gateway_status");
}

export function setRuntime(patch: { enabled?: boolean; apiKeys?: string[] }) {
  return invoke<{ enabled: boolean; api_keys: number }>("gateway_set_runtime", patch);
}

export function refreshMatrix() {
  return invoke<{ routes: number; live: number; builtin: number }>("gateway_refresh_matrix");
}

export function listChannels() {
  return invoke<GatewayChannel[]>("gateway_list_channels");
}

export function updateChannel(patch: {
  channelId: string;
  enabled?: boolean;
  priority?: number;
  weight?: number;
}) {
  return invoke<{ ok: boolean }>("gateway_update_channel", patch);
}

export function listAccounts() {
  return invoke<GatewayAccount[]>("gateway_list_accounts");
}

export function loginStart(channelId: string) {
  return invoke<GatewayLoginTicket>("gateway_login_start", { channelId });
}

export function loginPoll(channelId: string, loginState: string) {
  return invoke<GatewayLoginResult>("gateway_login_poll", { channelId, loginState });
}

export function setAccountEnabled(channelId: string, uid: string, enabled: boolean) {
  return invoke<{ ok: boolean }>("gateway_set_account_enabled", { channelId, uid, enabled });
}

export function deleteAccount(channelId: string, uid: string) {
  return invoke<{ removed: boolean }>("gateway_delete_account", { channelId, uid });
}

export function refreshAccount(channelId: string, uid: string) {
  return invoke<{ expires_at_ms: number | null }>("gateway_refresh_account", { channelId, uid });
}

/** 刷新额度（渠道无关；渠道不支持时返回 null）。 */
export function fetchQuota(channelId: string, uid: string) {
  return invoke<GatewayQuota | null>("gateway_fetch_quota", { channelId, uid });
}

/** 重置账号运行态：清冷却 / 熔断 / 失败计数 / 模型级限流（不动凭据与额度）。 */
export function resetAccountState(channelId: string, uid: string) {
  return invoke<{ ok: boolean; ready: boolean }>("gateway_reset_account_state", {
    channelId,
    uid,
  });
}

export function checkinAccount(channelId: string, uid: string) {
  return invoke<{ ok: boolean }>("gateway_checkin_account", { channelId, uid });
}

export function listRoutes() {
  return invoke<{ models: GatewayModelRoutes[]; live: number; builtin: number }>(
    "gateway_list_routes",
  );
}

export function clearAffinity() {
  return invoke<{ cleared: number }>("gateway_clear_affinity");
}

export function recentDecisions() {
  return invoke<GatewayDecision[]>("gateway_recent_decisions");
}

export function usageSummary(hours = 24) {
  return invoke<GatewayUsage>("gateway_usage_summary", { hours });
}

export interface GatewayUsageRow {
  ts: number;
  model: string;
  upstream_model: string;
  channel_id: string;
  account_uid: string;
  status: string;
  stream: boolean;
  attempts: number;
  prompt_tokens: number;
  completion_tokens: number;
  reasoning_tokens: number;
  cached_tokens: number;
  total_tokens: number;
  credit: number | null;
  ttft_ms: number | null;
  total_ms: number;
  tokens_per_second: number;
  effort: string | null;
  estimated: boolean;
  error_detail: string | null;
}

export interface GatewaySeriesPoint {
  ts: number;
  requests: number;
  total_tokens: number;
  cached_tokens: number;
  completion_tokens: number;
  prompt_tokens: number;
  credit: number;
}

/** 请求明细（日志表格）。 */
export function recentUsage(limit = 200) {
  return invoke<GatewayUsageRow[]>("gateway_recent_usage", { limit });
}

/** 按小时分桶的时间序列（折线图）。 */
export function usageSeries(hours: number, model?: string) {
  return invoke<GatewaySeriesPoint[]>("gateway_usage_series", {
    hours,
    model: model ? model : null,
  });
}

export function schedulerSnapshot() {
  return invoke<GatewaySchedulerSnapshot>("gateway_scheduler_snapshot");
}

export function paths() {
  return invoke<{ data_dir: string; database: string }>("gateway_paths");
}
