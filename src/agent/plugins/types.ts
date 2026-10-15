export type PluginRisk = "L0" | "L1" | "L2" | "L3";
export type PluginLifecycle =
  | "discovered"
  | "validated"
  | "mounted"
  | "active"
  | "draining"
  | "unmounted"
  | "failed";

/** 已加载能力（只读）：available=false 表示预留能力，已注册但未实现。 */
export interface CapabilityStatus {
  id: string;
  description: string;
  riskLevel: PluginRisk;
  available: boolean;
}

/** 暴露给模型的工具；name 是模型看到的 `<namespace>__<tool>`。 */
export interface ToolStatus {
  name: string;
  toolName: string;
  description: string;
  capabilityId: string;
  riskLevel: PluginRisk;
  legacyNames: string[];
  available: boolean;
}

/**
 * 运行时插件目录项，来自 Rust 侧 `list_builtin_plugins`。
 * 前端只读展示；插件在启动时自动加载，没有启停、排序或权限开关。
 */
export interface PluginStatus {
  id: string;
  namespace: string;
  name: string;
  version: string;
  description: string;
  icon: string;
  defaultScope: boolean;
  refAliases: string[];
  risk: PluginRisk;
  lifecycle: PluginLifecycle;
  generation: number;
  tools: ToolStatus[];
  capabilities: CapabilityStatus[];
  resources: { id: string; description: string }[];
  events: { id: string; description: string }[];
}
