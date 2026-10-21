import { useCallback, useEffect, useRef, useState } from "react";
import { ClipboardPaste, ExternalLink, FileUp, X } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

import * as api from "@/api/gateway";
import { errorText } from "./shared";
import type { GatewayChannel, GatewayLoginTicket } from "@/api/gateway";
import { toast } from "@/utils/toast";

/**
 * 添加账号弹窗：挑渠道 → 浏览器授权 → 自动入库。
 *
 * 两条登录路径并存，由渠道的 `login_needs_callback` 决定 —— 弹窗不认识任何具体渠道：
 * · **轮询**（workbuddy 这类）：上游有 `auth/token?state=` 可查，授权完自动入库；
 * · **粘贴回调**（Trae 这类）：授权回调被上游强制为 `127.0.0.1:<port>`，
 *   网关没有那个监听端口，只能把地址栏整条复制回来再兑换凭据。
 *
 * 成功后**不关闭**弹窗，方便连着加第二个渠道的账号；右下角是"完成"。
 */
export function AddAccountDialog({
  channels,
  onClose,
  onAdded,
}: {
  channels: GatewayChannel[];
  onClose: () => void;
  onAdded: () => void;
}) {
  const [pending, setPending] = useState<{ channelId: string; ticket: GatewayLoginTicket } | null>(
    null,
  );
  /** 当前在等粘贴回调的渠道（与 pending 互斥，避免两条路径同时显示）。 */
  const [callback, setCallback] = useState<{ channel: GatewayChannel; authUrl: string } | null>(
    null,
  );
  const [callbackText, setCallbackText] = useState("");
  /** 当前选中的渠道：选中是纯前端状态，点「开始」才真正发起登录。 */
  const [selected, setSelected] = useState<GatewayChannel | null>(null);
  /** "凭据从哪来"的展开态：默认收起，不占版面。 */
  const [showSteps, setShowSteps] = useState(false);
  /** 正在读取凭据文件。 */
  const [picking, setPicking] = useState(false);
  /** 已读取的文件名（只用于提示，内容已经填进粘贴框）。 */
  const [pickedName, setPickedName] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [hint, setHint] = useState<string | null>(null);
  /**
   * 「点了渠道但登录压根没开起来」的错误。
   *
   * 与 `hint` 分开：hint 只在授权面板里显示（需要 pending/callback），
   * 而这类错误发生时那两个状态都还是空的 —— 混用就会出现"点了没反应"。
   */
  const [startError, setStartError] = useState<string | null>(null);
  const [added, setAdded] = useState<string[]>([]);
  const pollTimer = useRef<number | null>(null);

  const stopPolling = useCallback(() => {
    if (pollTimer.current !== null) {
      window.clearInterval(pollTimer.current);
      pollTimer.current = null;
    }
  }, []);
  useEffect(() => stopPolling, [stopPolling]);

  /** 授权成功后统一的收尾：记录、提示、刷新号池。 */
  const recordSuccess = useCallback(
    (channelId: string, nickname?: string, uid?: string) => {
      const name = nickname || uid || "";
      setAdded((previous) => [...previous, `${name}（${channelId}）`]);
      setPending(null);
      setCallback(null);
      setCallbackText("");
      setHint(null);
      toast.success(`已加入账号：${name}`);
      onAdded();
    },
    [onAdded],
  );

  const start = async (channel: GatewayChannel) => {
    setHint(null);
    setStartError(null);
    try {
      const ticket = await api.loginStart(channel.id);
      if (channel.login_needs_callback) {
        // 不轮询：这个渠道根本不会往我们这儿回调。
        stopPolling();
        setPending(null);
        setCallback({ channel, authUrl: ticket.auth_url });
        setCallbackText("");
        // **不自动打开浏览器**：点一下渠道就弹出一个窗口太容易误触，
        // 而且失败时还会静默（用户只看到"点了没反应"）。改成下面显式的按钮。
        setHint(
          channel.login_has_auth_page
            ? "先在下面打开授权页完成授权，再把浏览器地址栏整条粘回来"
            : "这个渠道没有授权页：选一份凭据文件，或把内容粘进来",
        );
        return;
      }
      setCallback(null);
      setPending({ channelId: channel.id, ticket });
      setHint("在浏览器里完成登录授权，这里会自动完成入库与签到");
      stopPolling();
      pollTimer.current = window.setInterval(async () => {
        try {
          const result = await api.loginPoll(channel.id, ticket.state);
          if (result.status === "pending") return;
          stopPolling();
          if (result.status === "ok") {
            recordSuccess(channel.id, result.account?.nickname, result.account?.uid);
          } else {
            setHint(result.message ?? "授权失败");
          }
        } catch (reason) {
          stopPolling();
          setHint(errorText(reason, "轮询失败"));
        }
      }, 2500);
    } catch (reason) {
      // 走到这里说明**连登录都开不起来**（渠道没实现 login_start、或网络失败）。
      // 以前只写 hint，而 hint 只在 pending/callback 存在时渲染 → 点了没反应。
      setStartError(errorText(reason, "无法开始登录"));
    }
  };

  const submitCallback = async () => {
    if (!callback || !callbackText.trim()) return;
    setSubmitting(true);
    try {
      const result = await api.loginComplete(callback.channel.id, callbackText.trim());
      if (result.status === "ok") {
        recordSuccess(callback.channel.id, result.account?.nickname, result.account?.uid);
      } else {
        setHint(result.message ?? "授权失败");
      }
    } catch (reason) {
      setHint(errorText(reason, "无法完成登录"));
    } finally {
      setSubmitting(false);
    }
  };

  /**
   * 选一份凭据文件读进粘贴框。
   *
   * 用原生对话框而不是 `<input type="file">`：Tauri 里后者拿到的是浏览器沙箱的
   * 假路径，读不到真实文件；原生对话框直接给真实路径，交给后端读。
   */
  const pickCredentialFile = async () => {
    if (picking) return;
    setPicking(true);
    try {
      const selected = await openDialog({
        multiple: false,
        directory: false,
        filters: [
          { name: "凭据文件", extensions: ["csv", "json", "txt", "text"] },
          { name: "所有文件", extensions: ["*"] },
        ],
      });
      // 用户取消：静默返回，不当错误。
      if (typeof selected !== "string") return;
      const file = await api.readCredentialFile(selected);
      setCallbackText(file.text.trim());
      setPickedName(file.name);
      setHint(null);
      toast.success(`已读取 ${file.name}`);
    } catch (reason) {
      setHint(errorText(reason, "读取文件失败"));
    } finally {
      setPicking(false);
    }
  };

  const pasteFromClipboard = async () => {
    try {
      const text = await navigator.clipboard.readText();
      if (text) setCallbackText(text.trim());
    } catch {
      // 读剪贴板需要焦点/权限，失败就让用户手动粘 —— 不必弹错误。
    }
  };

  // 自定义供应商不走 OAuth（它的"账号"就是 API Key，用「自定义供应商」按钮添加），
  // 列在这里点了只会得到"该渠道不支持登录流程"，不如不列。
  const enabled = channels.filter((channel) => channel.enabled && channel.adapter !== "custom");
  const busy = pending !== null || submitting;

  return (
    <div
      className="absolute inset-0 z-50 flex items-center justify-center bg-black/65 p-8"
      onClick={(event) => {
        event.stopPropagation();
        stopPolling();
        onClose();
      }}
    >
      <div
        className="w-full max-w-lg rounded-2xl border border-white/12 bg-zinc-900 p-4 shadow-2xl"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="flex items-center gap-2">
          <span className="text-[13px] font-medium text-zinc-100">添加账号</span>
          <span className="text-[10px] text-zinc-500">选一个渠道发起授权</span>
          <button
            type="button"
            onClick={() => {
              stopPolling();
              onClose();
            }}
            className="ml-auto flex h-6 w-6 items-center justify-center rounded-md text-zinc-400 hover:bg-white/12 hover:text-zinc-100"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </header>

        {/* 先选渠道、再点「开始」：早先是点一下就发起授权，
            既容易误触，也会在选错渠道时白开一次浏览器。 */}
        <div className="mt-3 flex flex-wrap gap-1.5">
          {enabled.map((channel) => {
            const active =
              selected?.id === channel.id ||
              pending?.channelId === channel.id ||
              callback?.channel.id === channel.id;
            return (
              <button
                key={channel.id}
                type="button"
                disabled={busy}
                onClick={() => {
                  setSelected(channel);
                  setStartError(null);
                }}
                aria-pressed={active}
                className={`rounded-lg border px-3 py-1.5 text-[11px] transition-colors ${
                  active
                    ? "border-sky-400/40 bg-sky-400/15 text-sky-100"
                    : "border-white/12 bg-white/[0.04] text-zinc-200 hover:bg-white/10 disabled:opacity-40"
                }`}
              >
                {channel.display_name || channel.id}
              </button>
            );
          })}
          {enabled.length === 0 && (
            <p className="text-[11px] text-zinc-500">
              没有可授权的渠道。内置渠道到「设置」里确认已启用；自定义供应商请用「自定义供应商」按钮添加。
            </p>
          )}
        </div>

        {/* 选中的渠道才给「开始」按钮：显式动作，不会误触。 */}
        {selected && !pending && !callback && (
          <div className="mt-2 flex items-center gap-2">
            <button
              type="button"
              onClick={() => void start(selected)}
              className="rounded-md bg-emerald-400/20 px-2.5 py-1 text-[11px] text-emerald-100 hover:bg-emerald-400/30"
            >
              {selected.login_has_auth_page ? "开始授权" : "填写凭据"}
            </button>
            <span className="text-[10px] text-zinc-500">
              {selected.login_has_auth_page
                ? "会生成授权链接，你确认后再打开浏览器"
                : "这个渠道没有授权页，需要粘贴凭据"}
            </span>
          </div>
        )}

        {/* 登录开不起来时的错误：必须**无条件渲染**，否则点了渠道看起来像没反应。 */}
        {startError && (
          <div className="mt-3 rounded-lg border border-rose-400/25 bg-rose-400/8 px-2.5 py-2">
            <p className="text-[11px] text-rose-100">{startError}</p>
            <p className="mt-1 text-[10px] text-rose-200/70">
              该渠道可能还没有可用的登录方式；也可以用「自定义供应商」手工接入。
            </p>
          </div>
        )}

        {pending && (
          <div className="mt-3 rounded-lg border border-sky-400/25 bg-sky-400/8 px-2.5 py-2">
            <p className="text-[11px] text-sky-100">{hint}</p>
            <button
              type="button"
              onClick={() => {
                void navigator.clipboard?.writeText(pending.ticket.auth_url);
                toast.success("授权链接已复制");
              }}
              className="mt-1.5 block max-w-full truncate rounded-md bg-black/30 px-2 py-1 text-left font-mono text-[10px] text-sky-100/85"
            >
              {pending.ticket.auth_url}
            </button>
            <p className="mt-1 text-[10px] text-sky-200/70">
              正在等待授权（每 2.5 秒检查一次）…
            </p>
          </div>
        )}

        {callback && (
          <div className="mt-3 rounded-lg border border-amber-400/25 bg-amber-400/8 px-2.5 py-2">
            <p className="text-[11px] text-amber-100">{hint}</p>
            {/* 有授权页才给「打开」按钮。CodeArts 这类没有授权页的渠道，
                `authUrl` 只是一行填写说明，拿去 openUrl 必然失败。 */}
            {callback.channel.login_has_auth_page ? (
              <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                <button
                  type="button"
                  onClick={() => void openUrl(callback.authUrl).catch(() => {
                    setHint("没能打开浏览器，请手动复制下面的链接");
                  })}
                  className="flex items-center gap-1 rounded-md bg-amber-400/20 px-2.5 py-1 text-[11px] text-amber-50 hover:bg-amber-400/30"
                >
                  <ExternalLink className="h-3 w-3 shrink-0" />
                  打开授权页
                </button>
                <button
                  type="button"
                  onClick={() => {
                    void navigator.clipboard?.writeText(callback.authUrl);
                    toast.success("授权链接已复制");
                  }}
                  className="rounded-md bg-white/10 px-2.5 py-1 text-[11px] text-zinc-200 hover:bg-white/16"
                >
                  复制链接
                </button>
              </div>
            ) : (
              // 无授权页：把后端给的"填写说明"原样显示，不假装它是个链接。
              <div className="mt-1.5 rounded-md border border-white/8 bg-black/25 px-2 py-1">
                <p className="whitespace-pre-wrap text-[10px] leading-relaxed text-amber-100/85">
                  {callback.authUrl}
                </p>
                {/* 光说"要三项"不够：得说清去哪拿、怎么排。默认收起，不占版面。 */}
                <button
                  type="button"
                  onClick={() => setShowSteps((value) => !value)}
                  className="mt-1 text-[10px] text-amber-200/70 underline-offset-2 hover:text-amber-100 hover:underline"
                >
                  {showSteps ? "收起步骤" : "这三项从哪来？"}
                </button>
                {showSteps && (
                  <ol className="mt-1 list-decimal space-y-0.5 pl-4 text-[10px] leading-relaxed text-amber-100/75">
                    <li>在 CodeArts Agent 里登录华为云账号，让它把凭据写到本机。</li>
                    <li>
                      打开它的凭据文件（或在华为云控制台「我的凭证 → 临时访问密钥」新建一组），
                      里面能拿到 <span className="font-mono">token</span>、
                      <span className="font-mono">AK</span>、
                      <span className="font-mono">SK</span> 三项。
                    </li>
                    <li>整份 JSON 粘进来，或按「令牌、AK、SK」三行粘贴 —— 任一种都认。</li>
                  </ol>
                )}
              </div>
            )}
            {/* 只有"粘贴凭据"型渠道才需要选文件（授权型渠道粘的是回调 URL）。 */}
            {!callback.channel.login_has_auth_page && (
              <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                <button
                  type="button"
                  onClick={() => void pickCredentialFile()}
                  disabled={picking}
                  className="flex items-center gap-1 rounded-md bg-white/10 px-2.5 py-1 text-[11px] text-zinc-200 hover:bg-white/16 disabled:opacity-40"
                >
                  <FileUp className="h-3 w-3 shrink-0" />
                  {picking ? "读取中…" : "选择文件"}
                </button>
                <span className="text-[10px] text-zinc-500">
                  {pickedName
                    ? `已读取 ${pickedName}`
                    : "支持 .csv（华为云控制台导出）、.json、.txt"}
                </span>
              </div>
            )}
            <div className="mt-1.5 flex items-start gap-1.5">
              <textarea
                value={callbackText}
                onChange={(event) => setCallbackText(event.target.value)}
                rows={2}
                placeholder={
                  callback.channel.login_has_auth_page
                    ? "http://127.0.0.1:8765/authorize?userJwt=…&userInfo=…"
                    : "粘 JSON，或按「令牌、AK、SK」三行粘贴"
                }
                className="min-w-0 flex-1 resize-none rounded-md border border-white/12 bg-black/30 px-2 py-1 font-mono text-[10px] text-zinc-100 outline-none placeholder:text-zinc-600 focus:border-amber-400/40"
              />
              <button
                type="button"
                onClick={() => void pasteFromClipboard()}
                title="从剪贴板粘贴"
                className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-white/10 text-zinc-300 hover:bg-white/16"
              >
                <ClipboardPaste className="h-3 w-3" />
              </button>
            </div>
            <div className="mt-1.5 flex items-center gap-2">
              <button
                type="button"
                disabled={submitting || !callbackText.trim()}
                onClick={() => void submitCallback()}
                className="rounded-md bg-amber-400/20 px-2.5 py-1 text-[11px] text-amber-50 hover:bg-amber-400/30 disabled:opacity-40"
              >
                {submitting
                  ? "处理中…"
                  : callback.channel.login_has_auth_page
                    ? "完成添加"
                    : "导入凭据"}
              </button>
              <button
                type="button"
                onClick={() => {
                  setCallback(null);
                  setCallbackText("");
                  setHint(null);
                }}
                className="text-[10px] text-zinc-500 hover:text-zinc-300"
              >
                取消
              </button>
            </div>
          </div>
        )}

        {added.length > 0 && (
          <div className="mt-3 rounded-lg border border-emerald-400/25 bg-emerald-400/8 px-2.5 py-2">
            <p className="text-[11px] text-emerald-100">本次新增 {added.length} 个账号</p>
            {added.map((entry) => (
              <div key={entry} className="text-[10px] text-emerald-200/80">
                {entry}
              </div>
            ))}
          </div>
        )}

        <footer className="mt-4 flex items-center gap-2">
          <span className="text-[10px] text-zinc-500">
            可以连续添加多个渠道；关闭弹窗前请先等授权完成
          </span>
          <button
            type="button"
            onClick={() => {
              stopPolling();
              onClose();
            }}
            className="ml-auto rounded-md bg-white/10 px-3 py-1 text-[11px] text-zinc-100 hover:bg-white/16"
          >
            完成
          </button>
        </footer>
      </div>
    </div>
  );
}
