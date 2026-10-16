import { useCallback, useEffect, useState } from "react";
import * as api from "@/api/gateway";
import { onUnauthorized } from "@/utils/request";

export type SessionState = "checking" | "anonymous" | "authenticated";

/**
 * 面板登录态。
 *
 * 会话本身在服务端的签名 Cookie 里，这里只缓存"当前是不是登录了"，
 * 启动时探一次 `/api/auth/session`；任何请求撞上 401（会话过期）都会
 * 通过 `onUnauthorized` 把状态打回匿名，界面自然回到登录页。
 */
export function usePanelSession() {
  const [state, setState] = useState<SessionState>("checking");
  const [username, setUsername] = useState<string | null>(null);

  const probe = useCallback(async () => {
    try {
      const session = await api.authSession();
      setUsername(session.username);
      setState("authenticated");
    } catch {
      setUsername(null);
      setState("anonymous");
    }
  }, []);

  useEffect(() => {
    void probe();
  }, [probe]);

  useEffect(
    () =>
      onUnauthorized(() => {
        setUsername(null);
        setState("anonymous");
      }),
    [],
  );

  const login = useCallback(async (name: string, password: string) => {
    const session = await api.authLogin(name, password);
    setUsername(session.username);
    setState("authenticated");
  }, []);

  const logout = useCallback(async () => {
    try {
      await api.authLogout();
    } finally {
      setUsername(null);
      setState("anonymous");
    }
  }, []);

  return { state, username, login, logout };
}
