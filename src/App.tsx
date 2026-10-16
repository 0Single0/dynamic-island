import { Loader2 } from "lucide-react";
import { AppToaster } from "@/components/common/AppToaster";
import { LoginPage } from "@/pages/Login";
import { ProxyPoolPage } from "@/pages/ProxyPool";
import { usePanelSession } from "@/stores/panel/usePanelSession";

/**
 * Web 面板根组件。
 *
 * 单页应用：未登录显示登录页，登录后直接是号池面板（不再有岛体/设置页）。
 * 登录态由服务端 Cookie 决定，刷新页面不会掉登录。
 */
export default function App() {
  const { state, username, login, logout } = usePanelSession();

  return (
    <div className="h-screen w-screen overflow-hidden bg-zinc-950 text-zinc-100 antialiased">
      {state === "checking" ? (
        <div className="flex h-full w-full items-center justify-center gap-2 text-[12px] text-zinc-500">
          <Loader2 className="h-4 w-4 animate-spin" />
          正在检查登录状态…
        </div>
      ) : state === "anonymous" ? (
        <LoginPage onLogin={login} />
      ) : (
        <ProxyPoolPage username={username} onLogout={logout} />
      )}
      <AppToaster />
    </div>
  );
}
