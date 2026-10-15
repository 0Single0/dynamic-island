import { useEffect, useMemo } from "react";
import { Route, Routes, useLocation } from "react-router-dom";
import { AppToaster } from "@/components/common/AppToaster";
import { routes } from "@/routes";
import { currentWindowLabel } from "@/api/window";

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
