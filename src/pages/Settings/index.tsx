import { useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { listen } from "@tauri-apps/api/event";
import { Bot, Database, Keyboard, LockKeyhole, MonitorCog, Network, Palette, Plug, ShieldCheck, SlidersHorizontal } from "lucide-react";
import { SearchSettings } from "./components/SearchSettings";
import { PluginCatalog } from "./components/PluginCatalog";
import { ModelSettings } from "./components/ModelSettings";
import { SETTINGS_SECTION_EVENT, SETTINGS_WINDOW_LABEL, currentWindowLabel } from "@/api/window";
import { WindowControls, useOpaqueWindowBackground } from "@/components/common/WindowChrome";

const sections = [
  { id: "general", label: "通用", icon: SlidersHorizontal }, { id: "ai-models", label: "AI 与模型", icon: Bot },
  { id: "plugins", label: "插件与连接器", icon: Plug }, { id: "permissions", label: "工具与权限", icon: ShieldCheck },
  { id: "shortcuts", label: "快捷键", icon: Keyboard }, { id: "context", label: "上下文与历史", icon: Database },
  { id: "network", label: "网络与代理", icon: Network }, { id: "privacy", label: "数据与隐私", icon: LockKeyhole },
  { id: "appearance", label: "外观", icon: Palette }, { id: "about", label: "关于", icon: MonitorCog },
];

/**
 * 设置中心（`#/settings`）。
 *
 * 这是**独立窗口**页面，不是岛体窗口里的一条路由：
 * 岛体窗口被钉死成 812×512 的透明无边框矩形（置顶、不可缩放、不在任务栏），
 * 多栏表单挤进去只能靠压缩字号和行高，且打开设置会卸载整个岛体。
 * 独立窗口有正常尺寸、进任务栏、可被别的窗口盖住，岛体也不受影响。
 */
export default function SettingsPage() {
  const [searchParams, setSearchParams] = useSearchParams();
  // 分类以 URL 为准：切分类即一次站内导航，返回键才能逐个退回上一个分类。
  const requestedSection = searchParams.get("section") ?? "";
  const active = sections.some((section) => section.id === requestedSection) ? requestedSection : "general";
  const activeSection = sections.find((section) => section.id === active) ?? sections[0];
  const [pushedSection, setPushedSection] = useState<string | null>(null);

  useOpaqueWindowBackground();

  // 设置窗口是单例：从岛体再点“打开模型设置”时不会重建窗口，
  // 只会发这个事件，由这里把分类切过去。
  useEffect(() => {
    if (currentWindowLabel() !== SETTINGS_WINDOW_LABEL) return;
    let stop: (() => void) | undefined;
    let disposed = false;
    void listen<string>(SETTINGS_SECTION_EVENT, ({ payload }) => {
      if (sections.some((section) => section.id === payload)) setPushedSection(payload);
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    });
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);

  useEffect(() => {
    if (!pushedSection) return;
    setSearchParams((current) => {
      const next = new URLSearchParams(current);
      next.set("section", pushedSection);
      return next;
    });
    setPushedSection(null);
  }, [pushedSection, setSearchParams]);

  // 分类以 URL 为准（深链进入、刷新都保持当前位置），每次切换压入一条历史记录；重复点当前分类不再压栈。
  const openSection = (id: string) => {
    if (id === active) return;
    const next = new URLSearchParams(searchParams);
    next.set("section", id);
    setSearchParams(next);
  };

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-[#131315] text-zinc-100">
      {/* 无边框窗口：这一条既是标题栏也是拖拽手柄。 */}
      <div
        data-tauri-drag-region
        className="flex h-10 shrink-0 cursor-grab items-center gap-2 border-b border-white/8 px-3 active:cursor-grabbing"
      >
        <span className="grid h-5 w-5 place-items-center rounded bg-sky-300/15 text-sky-200"><Bot className="h-3 w-3" /></span>
        <span className="text-[12px] font-medium text-zinc-200">设置中心</span>
        <span className="text-[11px] text-zinc-600">·</span>
        <span className="text-[11px] text-zinc-500">{activeSection.label}</span>
        <span className="ml-auto">
          <WindowControls />
        </span>
      </div>

      <div className="flex min-h-0 flex-1">
        <aside className="flex w-52 shrink-0 flex-col overflow-y-auto border-r border-white/8 bg-black/20 p-2.5">
          <nav className="space-y-0.5">
            {sections.map((section) => {
              const Icon = section.icon;
              return (
                <button
                  key={section.id}
                  type="button"
                  onClick={() => openSection(section.id)}
                  className={`flex w-full items-center gap-2 rounded-lg px-2 py-2 text-left text-xs transition ${active === section.id ? "bg-white/[0.09] text-white" : "text-zinc-500 hover:bg-white/[0.045] hover:text-zinc-200"}`}
                >
                  <Icon className="h-3.5 w-3.5" />
                  {section.label}
                </button>
              );
            })}
          </nav>
        </aside>

        <main className="flex min-w-0 flex-1 flex-col overflow-hidden p-7">
          {active === "ai-models" ? (
            <ModelSettings />
          ) : (
            <div className="mx-auto min-h-0 w-full max-w-4xl flex-1 overflow-y-auto">
              <header className="mb-6">
                <div className="text-[11px] uppercase tracking-[0.18em] text-zinc-600">Settings</div>
                <h1 className="mt-1 text-xl font-semibold">{activeSection.label}</h1>
                <p className="mt-1 text-xs text-zinc-500">全局设置中心，后续 Agent、插件和数据策略都从这里管理。</p>
              </header>
              {active === "network" ? (
                <SearchSettings />
              ) : active === "plugins" ? (
                <PluginCatalog />
              ) : (
                <section className="rounded-xl border border-white/[0.08] bg-white/[0.035] p-5">
                  <h2 className="text-sm font-medium">{activeSection.label}</h2>
                  <p className="mt-2 text-xs leading-5 text-zinc-500">这一分类的详细配置将在对应阶段接入。当前页面已经预留完整设置中心结构。</p>
                </section>
              )}
            </div>
          )}
        </main>
      </div>
    </div>
  );
}
