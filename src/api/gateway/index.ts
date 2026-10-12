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
  /** 模型级限流：账号可用，但这几个模型在等到期（上游按模型限额）。 */
  model_limits: Array<{ model: string; until_ms: number }>;
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

export interface GatewayTask {
  code: string;
  name: string;
  status: string;
  progress: string;
  reward: number;
  automatable: boolean;
  note: string;
}

export interface GatewayTaskRunResult {
  code: string;
  name: string;
  status: string;
  message: string;
  earned: number;
}

export interface GatewayProbeOutcome {
  model: string;
  upstream_model: string;
  reply: string;
  latency_ms: number;
  prompt_tokens: number;
  completion_tokens: number;
}

/** 单账号真实探针：发一条最小请求，验证这个账号能不能用。 */
export function probeAccount(channelId: string, uid: string, model?: string) {
  return invoke<GatewayProbeOutcome>("gateway_probe_account", {
    channelId,
    uid,
    model: model || null,
  });
}

/** 该渠道可用于探针的模型列表。 */
export function probeModels(channelId: string) {
  return invoke<string[]>("gateway_probe_models", { channelId });
}

/** 渠道任务清单（渠道没有任务体系时返回 null）。 */
export function listTasks(channelId: string) {
  return invoke<GatewayTask[] | null>("gateway_list_tasks", { channelId });
}

/** 启动该账号的任务执行（后台跑，立即返回；已在执行则 started=false）。 */
export function runTasks(channelId: string, uid: string) {
  return invoke<{ started: boolean; reason?: string }>("gateway_run_tasks", {
    channelId,
    uid,
  });
}

/**
 * 每日动作（签到 / 猫猫旅行 / 每日活跃对话）。
 *
 * 与任务分开：任务是"一次性、做完消失"，每日动作每天都要再做一次 ——
 * 动作清单由**渠道按版本**给出（国内版签到+猫猫、国际版每日对话），前端不认识具体渠道。
 */
export interface GatewayDailyAction {
  code: string;
  name: string;
  /** 用户能做的动作名（"签到" / "领取奖励" / "派出旅行" / "去对话"）。 */
  action: string;
  done_today: boolean;
  note: string;
  automatable: boolean;
}

/** 该账号今天的每日动作清单（查状态，不做任何写操作）。 */
export function listDaily(channelId: string, uid: string) {
  return invoke<GatewayDailyAction[] | null>("gateway_list_daily", { channelId, uid });
}

/** 执行一个每日动作（单次请求，同步返回结果）。 */
export function runDaily(channelId: string, uid: string, code: string) {
  return invoke<GatewayTaskRunResult>("gateway_run_daily", { channelId, uid, code });
}

export interface GatewayTaskProgress {
  running: boolean;
  started_at?: number;
  finished_at?: number;
  results: GatewayTaskRunResult[];
  done: number;
  earned: number;
  error?: string;
}

/** 任务执行进度（前端轮询，逐条实时显示）。 */
export function taskProgress(channelId: string, uid: string) {
  return invoke<GatewayTaskProgress>("gateway_task_progress", { channelId, uid });
}

/**
 * 领取单项已达标任务的奖励（任务行上的「领取」）。
 *
 * 只调领奖接口，不上报任何事件 —— 用于"进度早就达标、只是没领过"的任务。
 * 失败原因（未达标 / 已领过）在返回的 message 里，不当异常抛。
 */
export function claimTask(channelId: string, uid: string, code: string) {
  return invoke<GatewayTaskRunResult>("gateway_claim_task", { channelId, uid, code });
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
