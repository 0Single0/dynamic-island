/**
 * 灵动岛的展开态与当前面板。
 *
 * 这些状态原本放在 Island 组件内部，但去设置中心会卸载整个 Island，
 * 回首页时就只能重置成收起的启动器。存到组件之外后：
 * · 用「返回上一页」离开设置 → 回到离开时的面板（例如对话页）；
 * · 用「关闭设置」离开 → 先 forgetIslandPanel()，回首页是收起状态。
 */
export type ActivePanel =
  | { kind: "launcher" }
  | { kind: "widget"; id: string }
  | { kind: "tool"; id: "markdown-pdf" };

let expanded = false;
let panel: ActivePanel = { kind: "launcher" };

export function readIslandPanel() {
  return { expanded, panel };
}

export function rememberIslandPanel(nextExpanded: boolean, nextPanel: ActivePanel) {
  expanded = nextExpanded;
  panel = nextPanel;
}

/** 收起态：关闭设置时显式重置，避免回首页又弹开上次的面板。 */
export function forgetIslandPanel() {
  expanded = false;
  panel = { kind: "launcher" };
}
