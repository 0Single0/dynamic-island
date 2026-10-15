/**
 * 灵动岛的展开态与当前面板。
 *
 * 这些状态原本放在 Island 组件内部；存到组件之外，是为了让岛体在短暂
 * 卸载后（例如热重载、路由抖动）仍回到离开时的面板，而不是重置成启动器。
 * 设置中心已经是独立窗口，不再卸载岛体，所以没有"关闭设置时重置面板"这回事。
 *
 * 工具的 id 用 `ToolId`（见 widgets/tools.ts）：标题、尺寸、渲染组件都由注册表提供。
 */
import type { ToolId } from "@/widgets/tools";

export type ActivePanel =
  | { kind: "launcher" }
  | { kind: "widget"; id: string }
  | { kind: "tool"; id: ToolId };

let expanded = false;
let panel: ActivePanel = { kind: "launcher" };

export function readIslandPanel() {
  return { expanded, panel };
}

export function rememberIslandPanel(nextExpanded: boolean, nextPanel: ActivePanel) {
  expanded = nextExpanded;
  panel = nextPanel;
}
