/**
 * 日期区间选择器（自绘，非官方控件）：点输入框弹月历，选起点再选终点。
 *
 * 为什么不用 <input type="date">：原生控件样式无法与玻璃拟态统一（Tailwind 管不到
 * 弹出层），而且一次只能选一天 —— 用量筛选要的是"起点→终点"。
 * 月历本身是纯 div/button 网格：样式全 Tailwind，键盘可用（button 天然可聚焦）。
 */
import { useEffect, useRef, useState } from "react";
import { CalendarDays, ChevronLeft, ChevronRight, X } from "lucide-react";

export interface DayRange {
  start: number;
  end: number;
}

/** 本地日期 → 当天 00:00 / 24:00 的毫秒。 */
function dayStart(year: number, month: number, day: number): number {
  return new Date(year, month, day).getTime();
}

function sameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

const WEEKDAYS = ["日", "一", "二", "三", "四", "五", "六"];

export function DateRangePicker({
  value,
  onChange,
  onClear,
}: {
  value: DayRange | null;
  onChange: (range: DayRange) => void;
  onClear: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [cursor, setCursor] = useState(() => {
    const now = new Date();
    return { year: now.getFullYear(), month: now.getMonth() };
  });
  /** 选中的起点（还没选终点时的暂存）。 */
  const [pendingStart, setPendingStart] = useState<number | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  // 点外面关掉
  useEffect(() => {
    if (!open) return;
    const onDown = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) {
        setOpen(false);
        setPendingStart(null);
      }
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);

  const today = new Date();
  const label = value
    ? `${formatDay(value.start)} ~ ${formatDay(value.end)}`
    : "选择日期区间";

  const pick = (year: number, month: number, day: number) => {
    const time = dayStart(year, month, day);
    if (pendingStart === null) {
      setPendingStart(time);
      return;
    }
    // 第二次点击 = 终点；反选就交换。
    const [start, end] =
      time >= pendingStart ? [pendingStart, time + 86_399_999] : [time, pendingStart + 86_399_999];
    onChange({ start, end });
    setPendingStart(null);
    setOpen(false);
  };

  const firstDay = new Date(cursor.year, cursor.month, 1);
  const daysInMonth = new Date(cursor.year, cursor.month + 1, 0).getDate();
  const leadBlanks = firstDay.getDay();

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        onClick={(event) => {
          event.stopPropagation();
          setOpen((previous) => !previous);
        }}
        className={`flex h-6 items-center gap-1 rounded-md px-2 text-[10px] transition-colors ${
          value
            ? "bg-sky-400/20 text-sky-100"
            : "bg-white/6 text-zinc-400 hover:bg-white/12"
        }`}
        aria-label="选择日期区间"
      >
        <CalendarDays className="h-3 w-3" />
        {label}
        {value && (
          <span
            role="button"
            tabIndex={0}
            onClick={(event) => {
              event.stopPropagation();
              onClear();
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") onClear();
            }}
            className="ml-0.5 rounded p-0.5 hover:bg-white/20"
            aria-label="清除日期筛选"
          >
            <X className="h-2.5 w-2.5" />
          </span>
        )}
      </button>

      {open && (
        <div
          className="absolute left-0 top-8 z-40 w-60 rounded-lg border border-white/12 bg-zinc-900 p-2 shadow-2xl"
          onClick={(event) => event.stopPropagation()}
        >
          <div className="flex items-center justify-between px-1 pb-1">
            <button
              type="button"
              onClick={() =>
                setCursor((previous) =>
                  previous.month === 0
                    ? { year: previous.year - 1, month: 11 }
                    : { ...previous, month: previous.month - 1 },
                )
              }
              className="rounded p-0.5 text-zinc-400 hover:bg-white/10 hover:text-zinc-100"
              aria-label="上个月"
            >
              <ChevronLeft className="h-3.5 w-3.5" />
            </button>
            <span className="text-[11px] font-medium text-zinc-200">
              {cursor.year} 年 {cursor.month + 1} 月
            </span>
            <button
              type="button"
              onClick={() =>
                setCursor((previous) =>
                  previous.month === 11
                    ? { year: previous.year + 1, month: 0 }
                    : { ...previous, month: previous.month + 1 },
                )
              }
              className="rounded p-0.5 text-zinc-400 hover:bg-white/10 hover:text-zinc-100"
              aria-label="下个月"
            >
              <ChevronRight className="h-3.5 w-3.5" />
            </button>
          </div>

          <div className="grid grid-cols-7 gap-0.5 text-center">
            {WEEKDAYS.map((weekday) => (
              <span key={weekday} className="py-0.5 text-[9px] text-zinc-600">
                {weekday}
              </span>
            ))}
            {Array.from({ length: leadBlanks }).map((_, index) => (
              <span key={`blank-${index}`} />
            ))}
            {Array.from({ length: daysInMonth }).map((_, index) => {
              const day = index + 1;
              const time = dayStart(cursor.year, cursor.month, day);
              const isToday = sameDay(new Date(time), today);
              const rangeStart = pendingStart ?? value?.start ?? null;
              const rangeEnd = pendingStart === null ? (value?.end ?? null) : null;
              const inRange =
                rangeStart !== null &&
                rangeEnd !== null &&
                time >= rangeStart &&
                time <= rangeEnd;
              const isEdge = time === rangeStart || (rangeEnd !== null && time === rangeEnd - 86_399_000 + 86_399_999 - 86_399_999);
              const isStart = rangeStart !== null && sameDay(new Date(time), new Date(rangeStart));
              return (
                <button
                  key={day}
                  type="button"
                  onClick={() => pick(cursor.year, cursor.month, day)}
                  className={`rounded py-0.5 text-[10px] transition-colors ${
                    inRange || isEdge || isStart
                      ? "bg-sky-400/25 text-sky-100"
                      : isToday
                        ? "bg-white/10 text-zinc-100"
                        : "text-zinc-400 hover:bg-white/12 hover:text-zinc-100"
                  }`}
                >
                  {day}
                </button>
              );
            })}
          </div>

          <div className="mt-1 border-t border-white/8 pt-1 text-center text-[9px] text-zinc-600">
            {pendingStart === null ? "点选起点，再点终点" : "已选起点，点选终点"}
          </div>
        </div>
      )}
    </div>
  );
}

function formatDay(ms: number): string {
  const date = new Date(ms);
  return `${date.getMonth() + 1}/${date.getDate()}`;
}
