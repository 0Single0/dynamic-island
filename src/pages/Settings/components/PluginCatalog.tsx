import { useBuiltinPlugins } from "@/agent/plugins";
import { AgentIcon, toAgentIconId } from "@/components/agent/AgentIcons";

const lifecycleLabels: Record<string, string> = {
  discovered: "已发现",
  validated: "已校验",
  mounted: "已挂载",
  active: "运行中",
  draining: "卸载中",
  unmounted: "已卸载",
  failed: "加载失败",
};

/** 只读的“已加载能力”目录：插件在启动时自动装载，这里没有任何启停或排序开关。 */
export function PluginCatalog() {
  const { plugins, isLoading, error } = useBuiltinPlugins();
  return (
    <section className="space-y-3">
      <div className="rounded-xl border border-white/[0.08] bg-white/[0.035] px-4 py-3 text-xs leading-5 text-zinc-500">
        插件在应用启动时自动加载。使用 AI 时，通过输入框中的 <span className="text-sky-200">@</span> 选择本轮需要调用的插件；高风险操作仍会在执行前请求确认。
      </div>
      {error ? (
        <div className="rounded-xl border border-rose-300/20 bg-rose-300/[0.08] px-4 py-3 text-xs text-rose-100">{error}</div>
      ) : null}
      {isLoading && plugins.length === 0 ? (
        <div className="rounded-xl border border-dashed border-white/[0.12] px-4 py-7 text-center text-xs text-zinc-500">正在读取已加载插件…</div>
      ) : null}
      {plugins.map((plugin) => (
        <div key={plugin.id} className="rounded-xl border border-white/[0.08] bg-white/[0.035] p-4">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-white/[0.07] text-base text-sky-200">
              <AgentIcon id={toAgentIconId(plugin.icon)} className="h-5 w-5" />
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2 text-sm font-medium">
                {plugin.name}
                <span className="rounded-full bg-white/[0.06] px-1.5 py-0.5 text-[9px] text-zinc-500">{plugin.risk}</span>
                <span className="rounded-full bg-white/[0.06] px-1.5 py-0.5 text-[9px] text-zinc-500">
                  {plugin.defaultScope ? "默认范围" : "按需 @"}
                </span>
              </div>
              <div className="mt-1 text-xs text-zinc-500">{plugin.description}</div>
              <div className="mt-2 text-[10px] text-zinc-600">
                {plugin.tools.length} 个工具 · v{plugin.version} · {lifecycleLabels[plugin.lifecycle] ?? plugin.lifecycle}
              </div>
            </div>
          </div>
          <div className="mt-3 grid gap-1.5 border-t border-white/[0.06] pt-3">
            {plugin.capabilities.map((capability) => (
              <div key={capability.id} className="flex items-baseline gap-2 text-[11px]">
                <span className="shrink-0 text-zinc-300">{capability.id}</span>
                <span className="min-w-0 flex-1 truncate text-zinc-600">{capability.description}</span>
                <span className={capability.available ? "text-emerald-200/80" : "text-zinc-500"}>
                  {capability.available ? "已实现" : "预留"}
                </span>
              </div>
            ))}
          </div>
        </div>
      ))}
    </section>
  );
}
