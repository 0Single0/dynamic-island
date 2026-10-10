import { useEffect, useMemo } from "react";
import { Route, Routes, useLocation } from "react-router-dom";
import { AppToaster } from "@/components/common/AppToaster";
import { routes } from "@/routes";
import { currentWindowLabel, setWindowInteractive } from "@/api/window";

/** 工具窗口的标签约定是 `tool-<id>`（见 src-tauri 的 open_tool_window）。 */
const TOOL_WINDOW_PREFIX = "tool-";

function RoutedApp() {
  const location = useLocation();
  // 浏览器预览时拿不到标签，返回空串（不影响任何逻辑）。
  const label = useMemo(() => currentWindowLabel(), []);
  const toolId = label.startsWith(TOOL_WINDOW_PREFIX)
    ? label.slice(TOOL_WINDOW_PREFIX.length)
    : null;

  useEffect(() => {
    // 自愈：工具窗口必须落在工具页。开发/生产下 URL 处理偶有差异，
    // 一旦 hash 没带上，窗口里跑的就是岛体首页 —— 表现为"点了没反应"。
    if (toolId && !location.pathname.startsWith("/tool/")) {
      window.location.hash = `#/tool/${toolId}`;
    }
  }, [toolId, location.pathname]);

  useEffect(() => {
    // 点击穿透是**岛体那个窗口**的策略，且 Rust 侧是全局开关：
    // 工具窗口绝不能去改它，否则岛体的穿透行为会被别的窗口随手关掉。
    if (toolId) return;
    // 设置中心是完整窗口页面，必须关闭灵动岛区域外的点击穿透；
    // 回到首页后恢复透明窗口的局部命中策略。
    void setWindowInteractive(location.pathname !== "/");
  }, [location.pathname, toolId]);

  return (
    <>
      <Routes>
        {routes.map((route) => (
          <Route
            key={route.path}
            path={route.path}
            element={<route.Component />}
          />
        ))}
      </Routes>
      <AppToaster />
    </>
  );
}

export default function App() {
  return (
    <div className="h-screen w-screen overflow-hidden bg-transparent text-zinc-100 antialiased">
      <RoutedApp />
    </div>
  );
}
