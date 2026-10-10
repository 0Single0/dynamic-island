import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import {
  Activity,
  ArrowDownToLine,
  ArrowUpFromLine,
  CircleDollarSign,
  Layers,
  Percent,
} from "lucide-react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  Legend,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import * as api from "@/api/gateway";
import type { GatewaySeriesPoint, GatewayUsage } from "@/api/gateway";
import { Select } from "@/components/common/Select";
import { formatTokens } from "./shared";

/**
 * 用量页：上面指标卡、中间趋势图、下面两张分布饼图。
 *
 * 趋势图用 recharts：X 轴带时间刻度，鼠标悬浮显示**该时段的整列数值**
 * （每个序列一行 + 合计），这是纯手写 SVG 做不到的交互。
 */
export function UsageView() {
  const [hours, setHours] = useState(24);
  const [model, setModel] = useState("");
  const [usage, setUsage] = useState<GatewayUsage | null>(null);
  const [series, setSeries] = useState<GatewaySeriesPoint[]>([]);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const [summary, points] = await Promise.all([
          api.usageSummary(hours),
          api.usageSeries(hours, model || undefined),
        ]);
        if (cancelled) return;
        setUsage(summary);
        setSeries(points);
      } catch {
        /* 浏览器预览模式忽略 */
      }
    };
    void load();
    const timer = window.setInterval(() => void load(), 5000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [hours, model]);

  const prompt = usage?.prompt_tokens ?? 0;
  const cached = usage?.cached_tokens ?? 0;
  const completion = usage?.completion_tokens ?? 0;

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-1.5">
        {[
          { value: 24, label: "近 24 小时" },
          { value: 168, label: "近 7 天" },
          { value: 720, label: "近 30 天" },
        ].map((entry) => (
          <button
            key={entry.value}
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              setHours(entry.value);
            }}
            className={`rounded-md px-2 py-0.5 text-[10px] ${
              hours === entry.value ? "bg-white/14 text-zinc-100" : "bg-white/6 text-zinc-400"
            }`}
          >
            {entry.label}
          </button>
        ))}
        <Select
          value={model}
          options={[
            { value: "", label: "全部模型" },
            ...(usage?.by_model ?? []).map((row) => ({ value: row.key, label: row.key })),
          ]}
          onChange={setModel}
          placeholder="全部模型"
          menuTitle="按模型筛选"
          className="ml-1 h-6 w-44 text-[10px]"
          aria-label="按模型筛选"
        />
      </div>

      {/* 主指标卡：左边一个大的，右边两个小的 */}
      <div className="flex items-stretch gap-3 rounded-xl border border-white/8 bg-white/[0.03] p-3">
        <div className="flex min-w-0 flex-1 flex-col justify-center">
          <span className="text-[11px] text-zinc-400">总 token 消耗</span>
          <div className="flex items-baseline gap-2">
            <span className="text-[26px] font-semibold leading-tight text-zinc-50">
              {formatTokens(usage?.total_tokens ?? 0)}
            </span>
            <span className="text-[10px] text-zinc-500">
              输入 {formatTokens(prompt)} · 输出 {formatTokens(completion)}
            </span>
          </div>
        </div>
        <div className="flex shrink-0 gap-2">
          <MiniStat
            icon={<Activity className="h-3.5 w-3.5 text-sky-300" />}
            label="总请求数"
            value={String(usage?.requests ?? 0)}
          />
          <MiniStat
            icon={<CircleDollarSign className="h-3.5 w-3.5 text-emerald-300" />}
            label="总积分"
            value={(usage?.credit ?? 0).toFixed(2)}
          />
        </div>
      </div>

      <div className="grid grid-cols-4 gap-2">
        <SubCard
          icon={<ArrowDownToLine className="h-3.5 w-3.5 text-violet-300" />}
          label="输入"
          value={formatTokens(prompt)}
        />
        <SubCard
          icon={<ArrowUpFromLine className="h-3.5 w-3.5 text-rose-300" />}
          label="输出"
          value={formatTokens(completion)}
        />
        <SubCard
          icon={<Layers className="h-3.5 w-3.5 text-sky-300" />}
          label="缓存命中"
          value={formatTokens(cached)}
        />
        <SubCard
          icon={<Percent className="h-3.5 w-3.5 text-emerald-300" />}
          label="缓存命中率"
          value={`${Math.round((usage?.cache_hit_rate ?? 0) * 100)}%`}
          emphasis
        />
      </div>

      <Panel title="使用趋势">
        <TrendChart points={series} hours={hours} />
      </Panel>

      <div className="grid grid-cols-2 gap-3">
        <Panel title="按模型分布">
          <Donut rows={usage?.by_model ?? []} />
        </Panel>
        <Panel title="按渠道分布">
          <Donut rows={usage?.by_channel ?? []} />
        </Panel>
      </div>
    </div>
  );
}

function MiniStat({ icon, label, value }: { icon: ReactNode; label: string; value: string }) {
  return (
    <div className="flex min-w-[96px] flex-col gap-0.5 rounded-lg border border-white/8 bg-black/20 px-2.5 py-2">
      <span className="flex items-center gap-1 text-[10px] text-zinc-500">
        {icon}
        {label}
      </span>
      <span className="text-[16px] font-medium text-zinc-100">{value}</span>
    </div>
  );
}

function SubCard({
  icon,
  label,
  value,
  emphasis,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  emphasis?: boolean;
}) {
  return (
    <div className="rounded-xl border border-white/8 bg-white/[0.025] px-2.5 py-2">
      <span className="flex items-center gap-1 text-[10px] text-zinc-500">
        {icon}
        {label}
      </span>
      <span
        className={`mt-0.5 block text-[15px] font-medium ${
          emphasis ? "text-emerald-300" : "text-zinc-100"
        }`}
      >
        {value}
      </span>
    </div>
  );
}

interface SeriesDef {
  key: keyof GatewaySeriesPoint;
  label: string;
  color: string;
}

const SERIES: SeriesDef[] = [
  { key: "prompt_tokens", label: "输入", color: "rgb(129 140 248)" },
  { key: "cached_tokens", label: "缓存命中", color: "rgb(56 189 248)" },
  { key: "completion_tokens", label: "输出", color: "rgb(244 114 182)" },
  { key: "total_tokens", label: "总量", color: "rgb(52 211 153)" },
];

/** X 轴刻度：跨度超过两天只到"日"，否则精确到小时。 */
function axisLabel(ts: number, hours: number): string {
  const date = new Date(ts);
  if (hours > 48) return `${date.getMonth() + 1}/${date.getDate()}`;
  return `${String(date.getHours()).padStart(2, "0")}:00`;
}

function tooltipLabel(ts: number, hours: number): string {
  const date = new Date(ts);
  return hours > 48
    ? `${date.getMonth() + 1}月${date.getDate()}日`
    : `${date.getMonth() + 1}月${date.getDate()}日 ${String(date.getHours()).padStart(2, "0")}:00`;
}

/** 趋势图：面积图 + 悬浮显示该时段整列数值。 */
function TrendChart({ points, hours }: { points: GatewaySeriesPoint[]; hours: number }) {
  if (points.length === 0) {
    return <div className="py-8 text-center text-[10px] text-zinc-600">这段时间还没有请求</div>;
  }

  return (
    <div className="h-[215px] w-full">
      <ResponsiveContainer width="100%" height="100%">
        <AreaChart data={points} margin={{ top: 6, right: 8, bottom: 0, left: -12 }}>
          <defs>
            {SERIES.map((series) => (
              <linearGradient key={series.key} id={`fill-${series.key}`} x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor={series.color} stopOpacity={0.35} />
                <stop offset="100%" stopColor={series.color} stopOpacity={0.02} />
              </linearGradient>
            ))}
          </defs>
          <CartesianGrid stroke="rgba(255,255,255,0.06)" vertical={false} />
          <XAxis
            dataKey="ts"
            tickFormatter={(value: number) => axisLabel(value, hours)}
            tick={{ fontSize: 10, fill: "rgb(113 113 122)" }}
            tickLine={false}
            axisLine={{ stroke: "rgba(255,255,255,0.08)" }}
            minTickGap={28}
          />
          <YAxis
            tickFormatter={(value: number) => formatTokens(value)}
            tick={{ fontSize: 10, fill: "rgb(113 113 122)" }}
            tickLine={false}
            axisLine={false}
            width={52}
          />
          <Tooltip
            cursor={{ stroke: "rgba(255,255,255,0.25)", strokeDasharray: "3 3" }}
            content={<TrendTooltip hours={hours} />}
          />
          <Legend
            verticalAlign="bottom"
            height={20}
            iconType="plainline"
            wrapperStyle={{ fontSize: 10, color: "rgb(161 161 170)" }}
          />
          {SERIES.map((series) => (
            <Area
              key={series.key}
              type="monotone"
              dataKey={series.key}
              name={series.label}
              stroke={series.color}
              strokeWidth={2}
              fill={`url(#fill-${series.key})`}
              dot={false}
              activeDot={{ r: 3.5, strokeWidth: 0 }}
              isAnimationActive={false}
            />
          ))}
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}

interface TooltipEntry {
  name?: string;
  value?: number;
  color?: string;
  dataKey?: string | number;
}

/** 悬浮卡片：标题是该时段与合计，下面每个序列一行。 */
function TrendTooltip({
  active,
  payload,
  label,
  hours,
}: {
  active?: boolean;
  payload?: TooltipEntry[];
  label?: number;
  hours: number;
}) {
  if (!active || !payload || payload.length === 0 || label === undefined) return null;
  const rows = payload.filter((entry) => entry.dataKey !== "ts");
  const total = rows.reduce((sum, entry) => sum + (entry.value ?? 0), 0);

  return (
    <div className="rounded-lg border border-white/12 bg-zinc-900/95 px-2.5 py-2 shadow-xl">
      <div className="text-[10px] font-medium text-zinc-200">
        {tooltipLabel(label, hours)} · {formatTokens(total)} tokens
      </div>
      <div className="mt-1 flex flex-col gap-0.5">
        {rows.map((entry) => (
          <div key={String(entry.dataKey)} className="flex items-center gap-2 text-[10px]">
            <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: entry.color }} />
            <span className="text-zinc-400">{entry.name}</span>
            <span className="ml-auto pl-3 font-medium text-zinc-100">
              {formatTokens(entry.value ?? 0)} tokens
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

const PIE_COLORS = [
  "rgb(56 189 248)",
  "rgb(52 211 153)",
  "rgb(129 140 248)",
  "rgb(244 114 182)",
  "rgb(251 191 36)",
  "rgb(248 113 113)",
];

/** 分布饼图（环形）：占比一眼可见，右侧配图例与数值。 */
function Donut({
  rows,
}: {
  rows: Array<{ key: string; requests: number; total_tokens: number; cached_tokens: number }>;
}) {
  const total = rows.reduce((sum, row) => sum + row.total_tokens, 0);
  if (rows.length === 0 || total <= 0) {
    return <div className="py-4 text-center text-[10px] text-zinc-600">暂无数据</div>;
  }

  const top = rows.slice(0, 6);
  const radius = 42;
  const circumference = 2 * Math.PI * radius;
  let offset = 0;

  return (
    <div className="flex items-center gap-4">
      <svg viewBox="0 0 120 120" className="h-32 w-32 shrink-0 -rotate-90">
        {top.map((row, index) => {
          const share = row.total_tokens / total;
          const segment = (
            <circle
              key={row.key}
              cx="60"
              cy="60"
              r={radius}
              fill="none"
              stroke={PIE_COLORS[index % PIE_COLORS.length]}
              strokeWidth="18"
              strokeDasharray={`${share * circumference} ${circumference}`}
              strokeDashoffset={-offset * circumference}
            >
              <title>
                {row.key}：{formatTokens(row.total_tokens)}（{Math.round(share * 100)}%）
              </title>
            </circle>
          );
          offset += share;
          return segment;
        })}
        <circle cx="60" cy="60" r="28" fill="rgb(9 9 11)" />
      </svg>

      <div className="flex min-w-0 flex-1 flex-col gap-1">
        {top.map((row, index) => (
          <div key={row.key} className="flex items-center gap-2 text-[10px]">
            <span
              className="h-2 w-2 shrink-0 rounded-sm"
              style={{ background: PIE_COLORS[index % PIE_COLORS.length] }}
            />
            <span className="min-w-0 flex-1 truncate text-zinc-300">{row.key}</span>
            <span className="shrink-0 text-zinc-500">
              {Math.round((row.total_tokens / total) * 100)}%
            </span>
            <span className="w-14 shrink-0 text-right text-zinc-400">
              {formatTokens(row.total_tokens)}
            </span>
          </div>
        ))}
        {rows.length > top.length && (
          <span className="text-[10px] text-zinc-600">
            其余 {rows.length - top.length} 个渠道/模型未显示
          </span>
        )}
      </div>
    </div>
  );
}

function Panel({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="rounded-xl border border-white/8 bg-white/[0.025] p-2.5">
      <div className="mb-2 text-[11px] text-zinc-400">{title}</div>
      {children}
    </section>
  );
}

