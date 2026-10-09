import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import { selectPosition } from "./selectPosition";

export interface SelectOption { value: string; label: string; description?: string; disabled?: boolean }
export interface SelectProps {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  menuClassName?: string;
  variant?: "default" | "text";
  menuTitle?: string;
  "aria-label"?: string;
}

export function Select({ value, options, onChange, placeholder = "请选择", disabled = false, className, menuClassName, variant = "default", menuTitle, "aria-label": ariaLabel }: SelectProps) {
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<ReturnType<typeof selectPosition> | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuId = useId();
  const selected = options.find(option => option.value === value);

  useLayoutEffect(() => {
    if (!open || disabled) return;
    const place = () => {
      const trigger = triggerRef.current;
      if (!trigger) return;
      const rect = trigger.getBoundingClientRect();
      const bounds = { left: 8, top: 8, right: window.innerWidth - 8, bottom: window.innerHeight - 8 };
      // Stay inside the island's clickable surface, including transparent Tauri windows.
      for (let parent = trigger.parentElement; parent; parent = parent.parentElement) {
        const style = getComputedStyle(parent);
        if (/(hidden|clip|auto|scroll)/.test(`${style.overflowX} ${style.overflowY}`)) {
          const box = parent.getBoundingClientRect();
          bounds.left = Math.max(bounds.left, box.left + 4); bounds.right = Math.min(bounds.right, box.right - 4);
          bounds.top = Math.max(bounds.top, box.top + 4); bounds.bottom = Math.min(bounds.bottom, box.bottom - 4);
        }
      }
      const height = Math.min(264, menuRef.current ? Math.ceil(menuRef.current.scrollHeight + 2) : ((menuTitle ? 34 : 0) + Math.max(1, options.length) * (options.some(o => o.description) ? 52 : 34) + 14));
      setPosition(selectPosition(rect, bounds, height, Math.max(rect.width, variant === "text" ? 240 : rect.width)));
    };
    place();
    const observer = new ResizeObserver(place);
    if (rootRef.current) observer.observe(rootRef.current);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => { observer.disconnect(); window.removeEventListener("resize", place); window.removeEventListener("scroll", place, true); };
  }, [open, disabled, options, menuTitle, variant]);

  useEffect(() => {
    if (!open) return;
    const close = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node) && !menuRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, [open]);

  const focusOption = (direction: number) => {
    const buttons = [...(menuRef.current?.querySelectorAll<HTMLButtonElement>("[role=option]:not(:disabled)") ?? [])];
    if (!buttons.length) return;
    const index = buttons.findIndex(button => button === document.activeElement);
    buttons[index < 0 ? (direction > 0 ? 0 : buttons.length - 1) : (index + direction + buttons.length) % buttons.length]?.focus();
  };

  return <div ref={rootRef} className="relative min-w-0" onBlur={event => {
    if (!rootRef.current?.contains(event.relatedTarget) && !menuRef.current?.contains(event.relatedTarget)) setOpen(false);
  }}>
    <button ref={triggerRef} type="button" aria-label={ariaLabel} aria-haspopup="listbox" aria-controls={menuId} aria-expanded={open && !disabled} disabled={disabled}
      onClick={() => setOpen(current => !current)}
      onKeyDown={event => {
        if (event.key === "Escape") setOpen(false);
        if (["ArrowDown", "ArrowUp"].includes(event.key)) { event.preventDefault(); setOpen(true); requestAnimationFrame(() => focusOption(event.key === "ArrowDown" ? 1 : -1)); }
      }}
      className={cn("flex w-full items-center justify-between gap-2 rounded-lg px-3 py-2 text-left text-xs text-zinc-200 outline-none transition disabled:cursor-not-allowed disabled:opacity-50 focus-visible:ring-1 focus-visible:ring-white/30", variant === "text" ? "border-0 bg-transparent px-1.5 hover:text-white" : "border border-white/10 bg-white/[0.04] hover:border-white/[0.18] hover:bg-white/[0.07]", className)}>
      <span className={cn("min-w-0 truncate", !selected && "text-zinc-500")}>{selected?.label ?? placeholder}</span>
      <ChevronDown className={cn("h-3 w-3 shrink-0 text-zinc-500 transition-transform", open && "rotate-180")} />
    </button>
    {open && !disabled && position && createPortal(<div ref={menuRef} id={menuId} role="listbox" aria-label={menuTitle || ariaLabel} data-placement={position.upwards ? "top" : "bottom"}
      style={{ left: position.left, top: position.top, width: position.width, maxHeight: position.maxHeight }}
      className={cn("fixed z-[100] overflow-y-auto rounded-lg border border-white/[0.12] bg-[#282828] p-1.5 shadow-2xl shadow-black/40 [scrollbar-color:rgba(255,255,255,.2)_transparent] [scrollbar-width:thin]", menuClassName)}
      onKeyDown={event => {
        if (event.key === "Escape") { event.preventDefault(); setOpen(false); triggerRef.current?.focus(); }
        if (["ArrowDown", "ArrowUp"].includes(event.key)) { event.preventDefault(); focusOption(event.key === "ArrowDown" ? 1 : -1); }
      }}>
      {menuTitle && <div className="px-2.5 pb-2 pt-1 text-[11px] text-zinc-400">{menuTitle}</div>}
      {options.length === 0 ? <div className="px-2.5 py-2 text-xs text-zinc-500">暂无选项</div> : options.map(option => <button key={option.value} type="button" role="option" aria-selected={option.value === value} disabled={option.disabled}
        onClick={() => { onChange(option.value); setOpen(false); triggerRef.current?.focus(); }}
        className="flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-xs text-zinc-200 outline-none transition hover:bg-white/[0.07] focus-visible:bg-white/[0.07] disabled:cursor-not-allowed disabled:opacity-40">
        <span className="min-w-0 flex-1"><span className="block truncate">{option.label}</span>{option.description && <span className="mt-0.5 block truncate text-[10px] text-zinc-500">{option.description}</span>}</span>
        <span className="grid h-3.5 w-3.5 shrink-0 place-items-center">{option.value === value && <Check className="h-3.5 w-3.5 text-zinc-300" />}</span>
      </button>)}
    </div>, document.body)}
  </div>;
}
