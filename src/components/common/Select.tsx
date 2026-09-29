import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";

export interface SelectOption {
  value: string;
  label: string;
  description?: string;
  disabled?: boolean;
}

export interface SelectProps {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  menuClassName?: string;
  "aria-label"?: string;
}

/**
 * 项目内统一的暗色下拉菜单。
 * 不使用原生 select，避免 WebView2 在深色玻璃窗口中渲染出系统白色菜单。
 */
export function Select({
  value,
  options,
  onChange,
  placeholder = "请选择",
  disabled = false,
  className,
  menuClassName,
  "aria-label": ariaLabel,
}: SelectProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const selected = options.find((option) => option.value === value);

  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);

  return (
    <div ref={rootRef} className="relative min-w-0">
      <button
        type="button"
        aria-label={ariaLabel}
        aria-expanded={open}
        disabled={disabled}
        onClick={() => setOpen((current) => !current)}
        className={cn(
          "flex w-full items-center justify-between gap-2 rounded-lg border border-white/[0.1] bg-white/[0.04] px-3 py-2 text-left text-xs text-zinc-200 outline-none transition",
          "hover:border-white/[0.18] hover:bg-white/[0.07] focus:border-sky-300/40",
          "disabled:cursor-not-allowed disabled:opacity-50",
          className,
        )}
      >
        <span className={cn("min-w-0 truncate", !selected && "text-zinc-500")}>{selected?.label ?? placeholder}</span>
        <ChevronDown className={cn("h-3.5 w-3.5 shrink-0 text-zinc-500 transition-transform", open && "rotate-180")} />
      </button>
      {open && !disabled && (
        <div className={cn("absolute left-0 right-0 top-[calc(100%+6px)] z-50 max-h-64 overflow-y-auto rounded-xl border border-white/[0.12] bg-[#202125] p-1.5 shadow-2xl shadow-black/40 [scrollbar-color:rgba(255,255,255,.2)_transparent]", menuClassName)}>
          {options.length === 0 ? (
            <div className="px-2.5 py-2 text-xs text-zinc-500">暂无选项</div>
          ) : options.map((option) => (
            <button
              key={option.value}
              type="button"
              disabled={option.disabled}
              onClick={() => {
                if (option.disabled) return;
                onChange(option.value);
                setOpen(false);
              }}
              className={cn(
                "flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-xs transition",
                "hover:bg-white/[0.08] disabled:cursor-not-allowed disabled:opacity-40",
                option.value === value ? "bg-sky-300/10 text-sky-100" : "text-zinc-300",
              )}
            >
              <span className="min-w-0 flex-1">
                <span className="block truncate">{option.label}</span>
                {option.description && <span className="mt-0.5 block truncate text-[10px] text-zinc-500">{option.description}</span>}
              </span>
              {option.value === value && <Check className="h-3.5 w-3.5 shrink-0 text-sky-200" />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
