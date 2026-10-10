import { useCallback, useEffect, useState } from "react";
import * as api from "@/api/gateway";
import type { GatewayTaskRunResult } from "@/api/gateway";

export interface ModelChip {
  id: string;
  credits: string | null;
  free: boolean;
  context_length: number | null;
  max_output_tokens: number | null;
  capabilities: { vision: boolean; tool_calls: boolean; reasoning: boolean };
  caps_known: boolean;
  reasoning_efforts: string[];
  reasoning_fixed_effort: string | null;
  reasoning_default_effort: string | null;
  reasoning_can_disable: boolean | null;
  always_reasoning: boolean;
}

/** 渠道 → 可用模型（含倍率）。从矩阵按渠道反向索引："这个渠道能用哪些模型、各自多少倍率"。 */
export function useChannelModels() {
  const [map, setMap] = useState<Record<string, ModelChip[]>>({});

  const load = useCallback(async () => {
    try {
      const data = await api.listRoutes();
      const next: Record<string, ModelChip[]> = {};
      for (const entry of data.models) {
        for (const route of entry.routes) {
          const list = (next[route.channel_id] ??= []);
          if (!list.some((model) => model.id === entry.model)) {
            list.push({
              id: entry.model,
              credits: route.credits,
              free: route.free,
              context_length: route.context,
              max_output_tokens: route.max_output,
              capabilities: route.capabilities,
              caps_known: route.caps_known,
              reasoning_efforts: route.reasoning_efforts,
              reasoning_fixed_effort: route.reasoning_fixed_effort,
              reasoning_default_effort: route.reasoning_default_effort,
              reasoning_can_disable: route.reasoning_can_disable,
              always_reasoning: route.always_reasoning,
            });
          }
        }
      }
      setMap(next);
    } catch {
      /* 浏览器预览模式忽略 */
    }
  }, []);

  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), 30_000);
    return () => window.clearInterval(timer);
  }, [load]);

  return map;
}

export function formatTokens(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}

export function formatDuration(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000));
  return seconds < 60 ? `${seconds}s` : `${Math.round(seconds / 60)}m`;
}

export function formatClock(ts: number): string {
  const date = new Date(ts);
  return [date.getHours(), date.getMinutes(), date.getSeconds()]
    .map((part) => String(part).padStart(2, "0"))
    .join(":");
}

export function expiryText(expiresAt: number): string {
  const remaining = expiresAt - Date.now();
  if (remaining <= 0) return "凭据已过期";
  const minutes = remaining / 60_000;
  if (minutes >= 1440) return `凭据 ${Math.round(minutes / 1440)} 天后过期`;
  if (minutes >= 60) return `凭据 ${Math.round(minutes / 60)} 小时后过期`;
  return `凭据 ${Math.max(1, Math.round(minutes))} 分钟后过期`;
}

/** 错误语义 → 人话（与 Rust 侧 ErrorKind 对应）。 */
export function errorLabel(kind: string): string {
  const labels: Record<string, string> = {
    auth_expired: "凭据失效",
    rate_limited: "限流",
    quota_exhausted: "额度用尽",
    content_policy: "内容审核",
    upstream_5xx: "上游异常",
    network: "网络",
    no_account: "无可用账号",
    invalid_request: "请求不合法",
    model_not_found: "模型不存在",
    unsupported: "不支持",
    internal: "网关内部错误",
  };
  return labels[kind] ?? kind;
}

/**
 * 任务执行状态：放在**模块级**而不是组件里。
 *
 * 为什么：任务执行是长任务（18 项 × 最多 10 轮 × 1.1 秒间隔），
 * 存在组件 state 里的话，切到别的账号/页面会卸载组件 → 状态与结果全丢，
 * 看起来就像"执行被重置了"（其实上游还在跑，只是界面忘了）。
 */
export interface TaskRunState {
  running: boolean;
  results: GatewayTaskRunResult[] | null;
  summary: { done: number; earned: number } | null;
  error: string | null;
}

const EMPTY_RUN: TaskRunState = { running: false, results: null, summary: null, error: null };
const runStates = new Map<string, TaskRunState>();
const runListeners = new Set<() => void>();

export function readTaskRun(key: string): TaskRunState {
  return runStates.get(key) ?? EMPTY_RUN;
}

export function writeTaskRun(key: string, patch: Partial<TaskRunState>) {
  runStates.set(key, { ...readTaskRun(key), ...patch });
  for (const listener of runListeners) listener();
}

/** 订阅任务执行状态（跨组件挂载存活）。 */
export function useTaskRunState(key: string): TaskRunState {
  const [, force] = useState(0);
  useEffect(() => {
    const listener = () => force((value) => value + 1);
    runListeners.add(listener);
    return () => {
      runListeners.delete(listener);
    };
  }, []);
  return readTaskRun(key);
}
