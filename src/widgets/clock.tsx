import { useEffect, useState } from "react";
import { Clock } from "lucide-react";
import type { WidgetDefinition } from "./types";

function useNow() {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  return now;
}

function ClockCompact() {
  const now = useNow();
  return (
    <span className="flex items-center gap-1.5 text-[13px] font-semibold tabular-nums tracking-wide text-zinc-100">
      <Clock className="h-3.5 w-3.5 text-zinc-400" strokeWidth={2.2} />
      {now.toLocaleTimeString("zh-CN", {
        hour: "2-digit",
        minute: "2-digit",
      })}
    </span>
  );
}

function ClockExpanded() {
  const now = useNow();
  return (
    <div className="flex h-full w-full flex-col items-center justify-center gap-1">
      <div className="text-4xl font-bold tabular-nums tracking-tight text-white">
        {now.toLocaleTimeString("zh-CN", {
          hour: "2-digit",
          minute: "2-digit",
          second: "2-digit",
        })}
      </div>
      <div className="text-sm text-zinc-400">
        {now.toLocaleDateString("zh-CN", {
          year: "numeric",
          month: "long",
          day: "numeric",
          weekday: "long",
        })}
      </div>
    </div>
  );
}

export const clockWidget: WidgetDefinition = {
  id: "clock",
  name: "时钟",
  description: "显示当前时间与日期",
  icon: Clock,
  order: 10,
  compact: ClockCompact,
  expanded: ClockExpanded,
  size: "md",
  enabledByDefault: true,
};
