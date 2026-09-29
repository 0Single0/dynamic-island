import { useEffect } from "react";
import { Route, Routes, useLocation } from "react-router-dom";
import { AppToaster } from "@/components/common/AppToaster";
import { routes } from "@/routes";
import { setWindowInteractive } from "@/api/window";

function RoutedApp() {
  const location = useLocation();

  useEffect(() => {
    // 设置中心是完整窗口页面，必须关闭灵动岛区域外的点击穿透；
    // 回到首页后恢复透明窗口的局部命中策略。
    void setWindowInteractive(location.pathname !== "/");
  }, [location.pathname]);

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
