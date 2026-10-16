/**
 * 把任意抛出的东西变成可读文本。
 *
 * 面板失败分两种来源：
 * ① `request()` 抛的是 `Error`（信封 `code !== 0` 时用 `msg` 构造，或网络/超时）；
 * ② 少数地方直接抛字符串。
 * 两种都要原样带出**后端/上游的原文** —— 探针、任务这些地方
 * `reason instanceof Error ? reason.message : "失败"` 会把原文丢掉。
 */
export function errorText(reason: unknown, fallback: string): string {
  if (typeof reason === "string" && reason.trim() !== "") return reason;
  if (reason instanceof Error && reason.message.trim() !== "") return reason.message;
  return fallback;
}
