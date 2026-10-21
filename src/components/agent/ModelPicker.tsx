import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown, ChevronRight, Settings2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { selectPosition } from "@/components/common/selectPosition";
import type { ModelRecord, ProviderRecord } from "@/agent/api";

export interface ModelPickerProps {
  providers: ProviderRecord[];
  models: ModelRecord[];
  value: string;
  onChange: (modelId: string) => void;
  /** 打开设置中心的「AI 与模型」分类。 */
  onManage: () => void;
  placeholder?: string;
  className?: string;
  "aria-label"?: string;
}

/**
 * 按供应商分组的模型选择器：左侧供应商、右侧该供应商的模型，底部提供「管理模型」入口。
 *
 * 现有 `Select` 只支持扁平列表，Supplier→Model 的两级结构在这里单独实现；
 * 弹层定位复用 `selectPosition`，保证不越出岛体可点击区域（透明的 Tauri 窗口会裁掉越界内容）。
 */
export function ModelPicker({ providers, models, value, onChange, onManage, placeholder = "未配置模型", className, "aria-label": ariaLabel }: ModelPickerProps) {
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<ReturnType<typeof selectPosition> | null>(null);
  const [hovered, setHovered] = useState("");
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuId = useId();

  const enabledModels = models.filter((model) => model.enabled);
  const selected = enabledModels.find((model) => model.id === value);
  const groups = providers
    .filter((provider) => provider.enabled)
    .map((provider) => ({ provider, models: enabledModels.filter((model) => model.providerId === provider.id) }))
    .filter((group) => group.models.length > 0);
  const hoveredGroup = groups.find((group) => group.provider.id === hovered) ?? groups[0];

  const providerName = (providerId: string) => providers.find((provider) => provider.id === providerId)?.name ?? providerId;
  const triggerLabel = selected ? `${providerName(selected.providerId)} / ${selected.displayName || selected.modelId}` : null;

  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const trigger = triggerRef.current;
      if (!trigger) return;
      const rect = trigger.getBoundingClientRect();
      const bounds = { left: 8, top: 8, right: window.innerWidth - 8, bottom: window.innerHeight - 8 };
      // 与 Select 一致：收敛到最近一个会裁剪的祖先，避免弹层被岛体窗口裁掉。
      for (let parent = trigger.parentElement; parent; parent = parent.parentElement) {
        const style = getComputedStyle(parent);
        if (/(hidden|clip|auto|scroll)/.test(`${style.overflowX} ${style.overflowY}`)) {
          const box = parent.getBoundingClientRect();
          bounds.left = Math.max(bounds.left, box.left + 4); bounds.right = Math.min(bounds.right, box.right - 4);
          bounds.top = Math.max(bounds.top, box.top + 4); bounds.bottom = Math.min(bounds.bottom, box.bottom - 4);
        }
      }
      // 内容高度取两个内层滚动列的 scrollHeight：外层容器被 maxHeight 裁剪后
      // 自身 scrollHeight 会返回裁剪后的高度，直接读会形成“越滚越矮”的反馈循环。
      const columns = menuRef.current ? Array.from(menuRef.current.children) as HTMLElement[] : [];
      const content = columns.length ? Math.max(...columns.map((column) => column.scrollHeight)) + 12 : 240;
      const height = Math.min(264, content);
      setPosition(selectPosition(rect, bounds, height, 360));
    };
    place();
    const observer = new ResizeObserver(place);
    if (rootRef.current) observer.observe(rootRef.current);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => { observer.disconnect(); window.removeEventListener("resize", place); window.removeEventListener("scroll", place, true); };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    // 打开时把高亮定位到当前选中模型所属的供应商，右侧直接显示它。
    setHovered(enabledModels.find((model) => model.id === value)?.providerId ?? groups[0]?.provider.id ?? "");
    const close = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node) && !menuRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
    // groups/value 只在打开瞬间取初值，此处无需跟随变化重算监听。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  return (
    <div ref={rootRef} className="relative min-w-0" onBlur={(event) => {
      if (!rootRef.current?.contains(event.relatedTarget) && !menuRef.current?.contains(event.relatedTarget)) setOpen(false);
    }}>
      <button
        ref={triggerRef}
        type="button"
        aria-label={ariaLabel ?? "选择模型"}
        aria-haspopup="dialog"
        aria-controls={menuId}
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
        onKeyDown={(event) => { if (event.key === "Escape") setOpen(false); }}
        className={cn("flex w-full items-center justify-between gap-2 rounded-lg px-3 py-2 text-left text-xs text-zinc-200 outline-none transition hover:text-white focus-visible:ring-1 focus-visible:ring-white/30", "border-0 bg-transparent px-1.5", className)}
      >
        <span className={cn("min-w-0 truncate", !triggerLabel && "text-zinc-500")}>{triggerLabel ?? placeholder}</span>
        <ChevronDown className={cn("h-3 w-3 shrink-0 text-zinc-500 transition-transform", open && "rotate-180")} />
      </button>
      {open && position && createPortal(
        <div
          ref={menuRef}
          id={menuId}
          role="dialog"
          aria-label="选择模型"
          data-placement={position.upwards ? "top" : "bottom"}
          style={{ left: position.left, top: position.top, width: position.width, maxHeight: position.maxHeight }}
          className="fixed z-[100] flex overflow-hidden rounded-lg border border-white/[0.12] bg-[#282828] shadow-2xl shadow-black/40"
          onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); setOpen(false); triggerRef.current?.focus(); } }}
        >
          <div className="flex w-[150px] shrink-0 flex-col overflow-y-auto p-1.5 [scrollbar-color:rgba(255,255,255,.2)_transparent] [scrollbar-width:thin]">
            {groups.length === 0 ? (
              <div className="px-2.5 py-2 text-xs text-zinc-500">暂无可用模型</div>
            ) : groups.map((group) => (
              <button
                key={group.provider.id}
                type="button"
                onMouseEnter={() => setHovered(group.provider.id)}
                onFocus={() => setHovered(group.provider.id)}
                onClick={() => setHovered(group.provider.id)}
                aria-current={hoveredGroup?.provider.id === group.provider.id}
                className={cn("flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-xs transition", hoveredGroup?.provider.id === group.provider.id ? "bg-white/[0.08] text-zinc-100" : "text-zinc-300 hover:bg-white/[0.05]")}
              >
                <span className="min-w-0 flex-1 truncate">{group.provider.name}</span>
                {selected?.providerId === group.provider.id && <Check className="h-3.5 w-3.5 shrink-0 text-zinc-300" />}
                <ChevronRight className="h-3 w-3 shrink-0 text-zinc-500" />
              </button>
            ))}
            <div className="mt-1 border-t border-white/[0.08] pt-1">
              <button
                type="button"
                onClick={() => { setOpen(false); onManage(); }}
                className="flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-xs text-zinc-400 transition hover:bg-white/[0.05] hover:text-zinc-100"
              >
                <Settings2 className="h-3.5 w-3.5 shrink-0" />管理模型
              </button>
            </div>
          </div>
          <div className="flex min-w-0 flex-1 flex-col overflow-y-auto border-l border-white/[0.08] p-1.5 [scrollbar-color:rgba(255,255,255,.2)_transparent] [scrollbar-width:thin]">
            {hoveredGroup ? hoveredGroup.models.map((model) => (
              <button
                key={model.id}
                type="button"
                onClick={() => { onChange(model.id); setOpen(false); triggerRef.current?.focus(); }}
                aria-selected={model.id === value}
                className="flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-xs text-zinc-200 transition hover:bg-white/[0.07]"
              >
                <span className="min-w-0 flex-1 truncate">{model.displayName || model.modelId}</span>
                {model.supportsImages && <span className="shrink-0 rounded bg-white/[0.1] px-1.5 py-0.5 text-[9px] text-zinc-400">视觉</span>}
                <span className="grid h-3.5 w-3.5 shrink-0 place-items-center">{model.id === value && <Check className="h-3.5 w-3.5 text-zinc-300" />}</span>
              </button>
            )) : (
              <div className="px-2.5 py-2 text-xs text-zinc-500">选择左侧供应商</div>
            )}
          </div>
        </div>,
        document.body,
      )}
    </div>
  );
}
