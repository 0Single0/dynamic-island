import { useState, type FormEvent } from "react";
import { KeyRound, Loader2, User } from "lucide-react";
import { errorText } from "@/utils/error";

/**
 * 面板登录页：账号密码来自服务端环境变量（`PANEL_USERNAME` / `PANEL_PASSWORD`）。
 *
 * 登录成功后服务端下发 HttpOnly 会话 Cookie，前端拿不到也不需要拿。
 */
export function LoginPage({ onLogin }: { onLogin: (name: string, password: string) => Promise<void> }) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await onLogin(username.trim(), password);
    } catch (reason) {
      setError(errorText(reason, "登录失败"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex h-full w-full items-center justify-center p-6">
      <form
        onSubmit={submit}
        className="w-full max-w-sm rounded-2xl border border-white/12 bg-zinc-900/80 p-6 shadow-2xl backdrop-blur"
      >
        <h1 className="text-[15px] font-medium text-zinc-100">反代号池</h1>
        <p className="mt-1 text-[11px] text-zinc-500">登录以管理账号池与路由</p>

        <label className="mt-5 flex items-center gap-2 rounded-lg border border-white/12 bg-black/30 px-2.5 py-2 focus-within:border-sky-400/50">
          <User className="h-3.5 w-3.5 shrink-0 text-zinc-500" />
          <input
            value={username}
            onChange={(event) => setUsername(event.target.value)}
            placeholder="账号"
            autoComplete="username"
            autoFocus
            className="min-w-0 flex-1 bg-transparent text-[12px] text-zinc-100 outline-none placeholder:text-zinc-600"
          />
        </label>

        <label className="mt-2 flex items-center gap-2 rounded-lg border border-white/12 bg-black/30 px-2.5 py-2 focus-within:border-sky-400/50">
          <KeyRound className="h-3.5 w-3.5 shrink-0 text-zinc-500" />
          <input
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            placeholder="密码"
            autoComplete="current-password"
            className="min-w-0 flex-1 bg-transparent text-[12px] text-zinc-100 outline-none placeholder:text-zinc-600"
          />
        </label>

        {error && (
          <p className="mt-3 rounded-lg border border-rose-400/25 bg-rose-400/10 px-2.5 py-1.5 text-[11px] text-rose-100">
            {error}
          </p>
        )}

        <button
          type="submit"
          disabled={busy || username.trim() === "" || password === ""}
          className="mt-4 flex w-full items-center justify-center gap-1.5 rounded-lg bg-sky-400/20 py-2 text-[12px] text-sky-100 transition-colors hover:bg-sky-400/30 disabled:opacity-40"
        >
          {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
          {busy ? "登录中…" : "登录"}
        </button>
      </form>
    </div>
  );
}
