import { GlassCard } from "@/components/common/GlassCard";
import { widgetRegistry } from "@/widgets";

/**
 * 设置页：管理启用的小组件与应用行为（占位）。
 * 后续接入持久化配置后可勾选开关各 widget。
 */
export default function SettingsPage() {
  return (
    <div className="flex h-screen w-screen items-center justify-center bg-zinc-950/80 p-6">
      <GlassCard className="w-full max-w-md" padding="lg">
        <h1 className="mb-1 text-lg font-semibold text-white">设置</h1>
        <p className="mb-4 text-xs text-zinc-500">
          管理灵动岛上的小组件（配置持久化待接入）
        </p>
        <ul className="flex flex-col gap-2">
          {widgetRegistry.map((widget) => {
            const Icon = widget.icon;
            return (
              <li
                key={widget.id}
                className="flex items-center gap-3 rounded-lg bg-white/5 px-3 py-2"
              >
                <Icon className="h-4 w-4 text-zinc-300" />
                <div className="flex-1">
                  <div className="text-sm text-zinc-100">{widget.name}</div>
                  {widget.description && (
                    <div className="text-xs text-zinc-500">
                      {widget.description}
                    </div>
                  )}
                </div>
                <div className="text-xs text-zinc-600">
                  {widget.enabledByDefault ? "已启用" : "未启用"}
                </div>
              </li>
            );
          })}
        </ul>
      </GlassCard>
    </div>
  );
}
