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
  /**
   * 该渠道的登录是否需要「用户把回调地址粘回来」。
   *
   * 后端按适配器给（Trae 为 true）：授权回调被上游强制成回环地址，
   * 网关没有那个监听端口。弹窗据此切到粘贴框，而不是轮询一个永远
   * 不会到达的结果。
   */
  login_needs_callback: boolean;
  /**
   * 该渠道有没有**真正的授权页**。
   *
   * `false` 表示它要求粘贴凭据（CodeArts 这类），此时 `auth_url` 只是一行
   * 填写说明 —— 面板不能拿它去 `openUrl`，那会打开一个不存在的地址。
   */
  login_has_auth_page: boolean;
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
  /** 生效值（已按"覆盖 → 渠道 → 默认"解开）。 */
  priority: number;
  weight: number;
  /** 账号级覆盖值：`null` = 跟随渠道。编辑弹窗据此决定输入框留空还是填数。 */
  priority_override: number | null;
  weight_override: number | null;
  proxy_slot: string | null;
  /** 这个账号只服务哪些对外模型名。空 = 不限。 */
  model_allow: string[];
  /** 账号级模型改名：`对外名 -> 该账号的上游名`。 */
  model_rename: Record<string, string>;
  quota: GatewayQuota | null;
}

export interface GatewayStatus {
  running: boolean;
  addr: string | null;
  error: string | null;
  enabled: boolean;
  port: number;
  /** 实际监听端口（配置端口被占用回退后，与 port 可能不同）。 */
  actual_port: number | null;
  /** 是否允许局域网访问（绑 0.0.0.0）。 */
  allow_lan: boolean;
  /** 内网访问地址（allow_lan 开着才给）。 */
  lan_addr: string | null;
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
  /**
   * 这条路由的元数据来源：`live`（实时目录）/ `builtin`（拉取失败回落的快照）/
   * `override`（人工覆盖）。面板据此区分"上游没给"与"这次没拉到"。
   */
  source: "live" | "builtin" | "override";
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

export function setRuntime(patch: {
  enabled?: boolean;
  apiKeys?: string[];
  allowLan?: boolean;
}) {
  return invoke<{ enabled: boolean; allow_lan: boolean; api_keys: number }>(
    "gateway_set_runtime",
    patch,
  );
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

/** 「获取模型」拉回的一条（各家方言已归一成这几个字段）。 */
export interface GatewayFetchedModel {
  id: string;
  context_length: number | null;
  max_output_tokens: number | null;
  capabilities: { vision: boolean; tool_calls: boolean; reasoning: boolean };
}

/**
 * 拉取某个自定义供应商的模型清单（通用实现：候选地址 + 协议鉴权 + 多方言解析）。
 *
 * 只读、不落库 —— 面板拿到后让用户勾选，再随供应商一起保存。
 */
export function fetchModels(input: {
  baseUrl: string;
  apiKey?: string;
  modelsUrl?: string;
  protocol?: string;
}) {
  return invoke<{ count: number; models: GatewayFetchedModel[] }>("gateway_fetch_models", input);
}

/** 随供应商一起落库的一条模型（矩阵/面板要的元数据）。 */
export interface CustomProviderModel {
  id: string;
  context_length?: number | null;
  max_output_tokens?: number | null;
  capabilities?: { vision?: boolean; tool_calls?: boolean; reasoning?: boolean };
  reasoning_efforts?: string[];
  reasoning_default_effort?: string | null;
  always_reasoning?: boolean;
}

/**
 * 保存一个自定义供应商：渠道配置 + 密钥账号一次写入。
 *
 * 不传 `channelId` 是新建（渠道 id 由后端按名称派生）；传了就是更新。
 * `apiKey` 留空表示不改动已保存的密钥。
 */
export function saveCustomProvider(input: {
  channelId?: string;
  displayName: string;
  baseUrl: string;
  apiKey?: string;
  protocol?: string;
  modelsUrl?: string;
  models: CustomProviderModel[];
  requireApiKey?: boolean;
  priority?: number;
}) {
  return invoke<{ channel_id: string; routes: number; accounts: number }>(
    "gateway_save_custom_provider",
    input,
  );
}

/** 删除渠道（连同它的账号）。 */
export function deleteChannel(channelId: string) {
  return invoke<{ removed_accounts: number }>("gateway_delete_channel", { channelId });
}

export function listAccounts() {
  return invoke<GatewayAccount[]>("gateway_list_accounts");
}

/**
 * 读取用户选中的凭据文件（.csv / .json / .txt）。
 *
 * Tauri 的原生文件对话框只给路径，磁盘读取归后端；后端顺带处理 BOM
 * （华为云控制台导出的 CSV 是 UTF-8 with BOM）。
 */
export function readCredentialFile(path: string) {
  return invoke<{ text: string; encoding: string; name: string }>(
    "gateway_read_credential_file",
    { path },
  );
}
export function loginStart(channelId: string) {
  return invoke<GatewayLoginTicket>("gateway_login_start", { channelId });
}

export function loginPoll(channelId: string, loginState: string) {
  return invoke<GatewayLoginResult>("gateway_login_poll", { channelId, loginState });
}

/**
 * 完成一次「需要粘贴回调地址」的登录（Trae 这类渠道）。
 *
 * 为什么单独一个命令：Trae 强制授权回调为 `http://127.0.0.1:<port>/authorize`，
 * 网关没有那个本地监听端口，所以用户必须把授权完成后地址栏里的完整 URL 交回来。
 * 弹窗据 `loginNeedsCallback` 决定走这条路径还是轮询。
 */
export function loginComplete(channelId: string, callback: string) {
  return invoke<GatewayLoginResult>("gateway_login_complete", { channelId, callback });
}

export function setAccountEnabled(channelId: string, uid: string, enabled: boolean) {
  return invoke<{ ok: boolean }>("gateway_set_account_enabled", { channelId, uid, enabled });
}

/**
 * 改账号画像：昵称 / 优先级 / 权重。
 *
 * 三个字段都是**可选覆盖**（与后端语义一致）：
 * · `nickname` 省略 = 不动；空串 = 清空（面板回落到 uid 前 8 位）；
 * · `priority / weight` 省略或 null = 清掉覆盖、回落渠道值，只有数字才是覆盖。
 *
 * 凭据、并发上限、额度、冷却一概不动 —— 编辑账号不该有"顺手把登录态弄丢"的副作用。
 */
export function setAccountProfile(
  channelId: string,
  uid: string,
  patch: {
    nickname?: string;
    priority?: number | null;
    weight?: number | null;
    maxInflight?: number | null;
  },
) {
  return invoke<{
    ok: boolean;
    nickname: string;
    priority: number;
    weight: number;
    max_inflight: number;
  }>("gateway_set_account_profile", {
    channelId,
    uid,
    nickname: patch.nickname,
    priority: patch.priority ?? null,
    weight: patch.weight ?? null,
    maxInflight: patch.maxInflight ?? null,
  });
}

/**
 * 改账号的模型清单：白名单（空 = 不限）+ 账号级模型改名。
 *
 * `rename` 是 `对外名 -> 该账号的上游名`：Trae 把 `deepseek-v4.1-flash` 叫
 * `DeepSeek-V4-Flash-Official`，配一条映射就能让客户端继续用统一的名字，
 * 出站时自动换成该账号的上游名。
 */
export function setAccountModels(
  channelId: string,
  uid: string,
  allow: string[],
  rename: Record<string, string>,
) {
  return invoke<{
    ok: boolean;
    allow: string[];
    rename: Record<string, string>;
    /** 后端为"本渠道目录里没有这个对外名"自动补的路由覆盖。 */
    routes_added: string[];
    /** 因上游名不在渠道目录里而被跳过的条目（带原因）。 */
    skipped: string[];
  }>("gateway_set_account_models", { channelId, uid, allow, rename });
}

export function deleteAccount(channelId: string, uid: string) {
  return invoke<{ removed: boolean }>("gateway_delete_account", { channelId, uid });
}

export function refreshAccount(channelId: string, uid: string) {
  return invoke<{ expires_at_ms: number | null }>("gateway_refresh_account", { channelId, uid });
}

/**
 * 刷新额度（渠道无关）。
 *
 * `supported = false` 表示该渠道**没有额度概念**（CodeArts 按 token 计量、
 * 只在华为云控制台看），不是"查询失败"—— 面板据此给出不同的提示文案。
 */
export function fetchQuota(channelId: string, uid: string) {
  return invoke<{ supported: boolean; quota: GatewayQuota | null }>(
    "gateway_fetch_quota",
    { channelId, uid },
  );
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

/**
 * 设置单个账号的并发上限（"请求中"列点开改）。`null` = 回落渠道/全局默认。
 * 立即生效；重启后保留（写在账号配置里）。
 */
export function setAccountMaxInflight(channelId: string, uid: string, maxInflight: number | null) {
  return invoke<{ ok: boolean; max_inflight: number }>("gateway_set_account_max_inflight", {
    channelId,
    uid,
    maxInflight,
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
/** 请求日志：limit 省略（或 0）= 全量。 */
export function recentUsage(limit = 0) {
  return invoke<GatewayUsageRow[]>("gateway_recent_usage", { limit });
}

/** 按小时分桶的时间序列（折线图）。 */
/** 指定毫秒区间的汇总（日期选择器用）。 */
export function usageSummaryRange(fromMs: number, toMs: number) {
  return invoke<GatewayUsage>("gateway_usage_summary_range", { fromMs, toMs });
}

/** 指定毫秒区间的趋势序列（桶宽自适应）。 */
export function usageSeriesRange(fromMs: number, toMs: number, model?: string) {
  return invoke<GatewaySeriesPoint[]>("gateway_usage_series_range", { fromMs, toMs, model });
}

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
