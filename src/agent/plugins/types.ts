export type PluginRisk = "L0" | "L1" | "L2" | "L3";
export type PluginIconId = "system" | "files" | "web-search" | "desktop" | "netease" | "markdown";

export interface PluginManifest {
  id: string;
  name: string;
  description: string;
  icon: PluginIconId;
  version: string;
  builtin: boolean;
  enabledByDefault: boolean;
  risk: PluginRisk;
  tools: string[];
  slashActions: string[];
}

export const builtinPluginRegistry: PluginManifest[] = [
  {
    id: "system",
    name: "系统",
    description: "时间、窗口和应用启动",
    icon: "system",
    version: "0.1.0",
    builtin: true,
    enabledByDefault: true,
    risk: "L0",
    tools: ["system.get_time", "system.list_windows"],
    slashActions: ["/system.time"],
  },
  {
    id: "files",
    name: "文件",
    description: "读取、编辑、备份和撤销文件",
    icon: "files",
    version: "0.1.0",
    builtin: true,
    enabledByDefault: true,
    risk: "L2",
    tools: ["files.search", "files.read", "files.write", "files.diff"],
    slashActions: ["/file.open", "/file.search"],
  },
  {
    id: "web-search",
    name: "网页搜索",
    description: "搜索网络并保留来源",
    icon: "web-search",
    version: "0.1.0",
    builtin: true,
    enabledByDefault: true,
    risk: "L1",
    tools: ["web_search.search", "web_search.open_result"],
    slashActions: ["/web.search"],
  },
  {
    id: "desktop",
    name: "电脑",
    description: "受控的 UIA 和键鼠操作",
    icon: "desktop",
    version: "0.1.0",
    builtin: true,
    enabledByDefault: false,
    risk: "L3",
    tools: ["computer.inspect_window", "computer.invoke", "computer.type_text"],
    slashActions: ["/computer.inspect"],
  },
  {
    id: "netease",
    name: "网易云音乐",
    description: "读取歌曲、歌词和播放状态",
    icon: "netease",
    version: "0.1.0",
    builtin: true,
    enabledByDefault: true,
    risk: "L1",
    tools: ["netease.current", "netease.control", "netease.seek"],
    slashActions: ["/music.current"],
  },
  {
    id: "markdown",
    name: "Markdown 转 PDF",
    description: "把 Markdown 文件转换为 PDF",
    icon: "markdown",
    version: "0.1.0",
    builtin: true,
    enabledByDefault: true,
    risk: "L1",
    tools: ["markdown.read", "markdown.to_pdf"],
    slashActions: ["/markdown.pdf"],
  },
];
