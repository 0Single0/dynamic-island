import { useEffect, useMemo, useState } from "react";
import { Plus, X } from "lucide-react";

import * as api from "@/api/gateway";
import type { GatewayAccount, GatewayChannel } from "@/api/gateway";
import { toast } from "@/utils/toast";
import { Select } from "@/components/common/Select";
import { errorText, type ModelChip } from "./shared";

/**
 * 编辑账号：昵称 / 优先级 / 权重 / 并发上限 / 模型白名单 / 模型改名。
 *
 * 这六样都是**我们自己的调度画像**，与上游无关：
 * · 昵称、优先级、权重、并发上限决定"这个号怎么被选中、一次能跑几个"；
 * · 白名单决定"这个号不接哪些模型"（没开通、单独计费、想专号专用）；
 * · 改名解决同名不同姓 —— Trae 把 `deepseek-v4.1-flash` 叫
 *   `DeepSeek-V4-Flash-Official`，映射一条就能让客户端继续用统一名字。
 *
 * 凭据、额度归上游所有，由刷新链路维护，这里一概不碰。
 */
export function EditAccountDialog({
  account,
  channel,
  models,
  publicNames,
  onClose,
  onSaved,
}: {
  account: GatewayAccount;
  /** 所属渠道：用来显示"回落到渠道的 X"，也让用户知道自己在改哪一层。 */
  channel: GatewayChannel | undefined;
  /** 该渠道可用模型（白名单候选项、改名的"上游名"都从这里来）。 */
  models: ModelChip[];
  /** 全局对外模型名（跨渠道共用）：改名的"对外名"候选。 */
  publicNames: string[];
  onClose: () => void;
  onSaved: () => void;
}) {
  const [nickname, setNickname] = useState(account.nickname);
  /**
   * 覆盖值用字符串存：空串 = "不覆盖，回落默认"。
   * 用 number 的话，"没填"和"填了 0"没法区分，而 0 是合法优先级。
   */
  const [priority, setPriority] = useState(
    account.priority_override === null ? "" : String(account.priority_override),
  );
  const [weight, setWeight] = useState(
    account.weight_override === null ? "" : String(account.weight_override),
  );
  const [maxInflight, setMaxInflight] = useState(String(account.max_inflight));
  /** 白名单：空 = 不限制。 */
  const [allow, setAllow] = useState<string[]>(account.model_allow ?? []);
  /** 改名：对外名 → 该账号的上游名。 */
  const [rename, setRename] = useState<Array<{ from: string; to: string }>>(() =>
    Object.entries(account.model_rename ?? {}).map(([from, to]) => ({ from, to })),
  );
  const [renaming, setRenaming] = useState(false);
  const [saving, setSaving] = useState(false);
  const [hint, setHint] = useState<string | null>(null);

  // Esc 关闭：与「添加账号」「自定义供应商」两个弹窗一致。
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  /** 该渠道的**上游**模型名（改名右侧的候选）：出站时真正要发的就是这个。 */
  const upstreamNames = useMemo(() => {
    const names = new Set(models.map((model) => model.upstream));
    // 已配但目录里没有的（模型下线 / 目录没刷新）也要能看见、能改回。
    for (const entry of rename) if (entry.to) names.add(entry.to);
    return [...names].sort();
  }, [models, rename]);

  /**
   * 改名右侧的候选：**对外模型名**（客户端将来请求的名字）。
   *
   * 取全局清单 + 本渠道公开名 + 已配条目 —— 跨渠道共享的对外名（例如
   * `deepseek-v4.1-flash` 同时存在于 wb-cn 与 trae）必须能在这里选到，
   * 否则"让 Trae 的模型对外叫成公共名"这件事就做不成。
   */
  const publicChoices = useMemo(() => {
    const names = new Set(publicNames);
    // 本渠道的公开名也并进来（目录可能比全局清单更新）。
    for (const model of models) names.add(model.id);
    // 已配但两个来源都没有的，也要能看见、能保留。
    for (const name of allow) names.add(name);
    for (const entry of rename) if (entry.from) names.add(entry.from);
    return [...names].sort();
  }, [publicNames, models, allow, rename]);

  /** 字符串 → 覆盖值：空串是 null（回落），非法数字也当 null。 */
  const override = (value: string): number | null => {
    const parsed = Number.parseInt(value, 10);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
  };

  const save = async () => {
    const trimmed = nickname.trim();
    if (trimmed.length > 40) {
      setHint("昵称最多 40 个字符");
      return;
    }
    // 改名表里"只填了一边"的条目属于半成品，保存前必须说清楚，
    // 否则用户以为改了、实际没生效。
    const incomplete = rename.filter(
      (entry) => (entry.from.trim() && !entry.to.trim()) || (!entry.from.trim() && entry.to.trim()),
    );
    if (incomplete.length > 0) {
      const entry = incomplete[0];
      setHint(
        entry.to.trim()
          ? `「${entry.to.trim()}」还没填对外名`
          : `「${entry.from.trim()}」还没选本渠道的模型`,
      );
      return;
    }
    setSaving(true);
    setHint(null);
    try {
      const mapping: Record<string, string> = {};
      for (const entry of rename) {
        const from = entry.from.trim();
        const to = entry.to.trim();
        if (from && to) mapping[from] = to;
      }
      // 先存画像（昵称/优先级/权重/并发），再存模型清单：两个命令各管一摊，
      // 任一失败都会在 catch 里报出来，不会静默吞掉。
      const result = await api.setAccountProfile(account.channel_id, account.uid, {
        nickname: trimmed,
        priority: override(priority),
        weight: override(weight),
        maxInflight: override(maxInflight),
      });
      const modelsResult = await api.setAccountModels(account.channel_id, account.uid, allow, mapping);
      toast.success(
        `已保存：${result.nickname || account.uid.slice(0, 8)} · 优先级 ${result.priority} · 权重 ${result.weight} · 并发 ${result.max_inflight}`,
      );
      // 后端替我们补了路由（对外名本来不在本渠道目录里）—— 说一声，别让它静默发生。
      if (modelsResult.routes_added.length > 0) {
        toast.info(`已为该渠道补上路由：${modelsResult.routes_added.join("、")}`);
      }
      // 上游名不在目录里的条目**没有生效**，必须明说，否则用户以为改好了。
      if (modelsResult.skipped.length > 0) {
        toast.error(`部分改名未生效：${modelsResult.skipped.join("；")}`);
      }
      onSaved();
      onClose();
    } catch (reason) {
      setHint(errorText(reason, "保存失败"));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      className="absolute inset-0 z-50 flex items-center justify-center bg-black/65 p-6"
      onClick={(event) => {
        event.stopPropagation();
        onClose();
      }}
    >
      <div
        className="flex max-h-full w-full max-w-2xl flex-col rounded-2xl border border-white/12 bg-zinc-900 p-4 shadow-2xl"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="flex shrink-0 items-center gap-2">
          <span className="text-[13px] font-medium text-zinc-100">编辑账号</span>
          <span className="min-w-0 truncate font-mono text-[10px] text-zinc-500" title={account.uid}>
            {channel?.display_name || account.channel_id} · {account.uid.slice(0, 12)}
          </span>
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭"
            className="ml-auto grid h-6 w-6 shrink-0 place-items-center rounded-md text-zinc-400 hover:bg-white/12 hover:text-zinc-100"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </header>

        <div className="mt-3 min-h-0 flex-1 overflow-y-auto pr-1">
          <label className="grid gap-1.5">
            <span className="flex items-baseline gap-1.5 text-[11px] text-zinc-400">
              昵称
              <span className="text-[10px] text-zinc-600">留空则显示 uid 前 8 位</span>
            </span>
            <input
              autoFocus
              value={nickname}
              onChange={(event) => setNickname(event.target.value)}
              maxLength={40}
              placeholder={account.uid.slice(0, 8)}
              className="h-7 w-full rounded-md border border-white/10 bg-black/30 px-2 text-[11px] text-zinc-200 outline-none transition placeholder:text-zinc-600 focus:border-sky-400/50"
            />
          </label>

          <div className="mt-3 grid grid-cols-3 gap-2">
            <label className="grid gap-1.5">
              <span className="text-[11px] text-zinc-400">并发上限</span>
              <input
                inputMode="numeric"
                value={maxInflight}
                onChange={(event) => setMaxInflight(event.target.value.replace(/\D/g, ""))}
                placeholder="1"
                className="h-7 w-full rounded-md border border-white/10 bg-black/30 px-2 text-center font-mono text-[11px] text-zinc-200 outline-none transition placeholder:text-zinc-600 focus:border-sky-400/50"
              />
            </label>
            <label className="grid gap-1.5">
              <span className="text-[11px] text-zinc-400">优先级</span>
              <input
                inputMode="numeric"
                value={priority}
                onChange={(event) => setPriority(event.target.value.replace(/\D/g, ""))}
                placeholder={String(channel?.priority ?? 100)}
                className="h-7 w-full rounded-md border border-white/10 bg-black/30 px-2 text-center font-mono text-[11px] text-zinc-200 outline-none transition placeholder:text-zinc-600 focus:border-sky-400/50"
              />
            </label>
            <label className="grid gap-1.5">
              <span className="text-[11px] text-zinc-400">权重</span>
              <input
                inputMode="numeric"
                value={weight}
                onChange={(event) => setWeight(event.target.value.replace(/\D/g, ""))}
                placeholder={String(channel?.weight ?? 1)}
                className="h-7 w-full rounded-md border border-white/10 bg-black/30 px-2 text-center font-mono text-[11px] text-zinc-200 outline-none transition placeholder:text-zinc-600 focus:border-sky-400/50"
              />
            </label>
          </div>

          <div className="mt-4 rounded-lg border border-white/8 bg-white/[0.02] p-2">
            <div className="flex items-center gap-2">
              <span className="text-[11px] text-zinc-300">可用模型</span>
              <span className="text-[10px] text-zinc-600">
                {allow.length === 0
                  ? "默认全选（该渠道有什么都能跑）"
                  : `只接 ${allow.length} 个`}
              </span>
              {allow.length > 0 && (
                <button
                  type="button"
                  onClick={() => setAllow([])}
                  className="ml-auto rounded px-1.5 py-0.5 text-[10px] text-zinc-400 hover:bg-white/10 hover:text-zinc-200"
                >
                  全选
                </button>
              )}
            </div>
            {publicChoices.length === 0 ? (
              <p className="mt-1.5 text-[10px] text-zinc-600">
                该渠道还没有模型目录，展开账号行的「可用模型」刷新一次即可。
              </p>
            ) : (
              // 限高 + 独立滚动：模型多的渠道（20+）否则会把弹窗撑到满屏，
              // 下面的"模型改名"整块被推到视口外。
              <div className="mt-1.5 flex max-h-32 flex-wrap gap-1 overflow-y-auto pr-0.5">
                {publicChoices.map((name) => {
                  // 空白名单 = 全选，所以未显式勾选时也算"启用"。
                  const active = allow.length === 0 || allow.includes(name);
                  return (
                    <button
                      key={name}
                      type="button"
                      onClick={() => {
                        // 从"全选"点第一个 = 只留它；再点 = 取消它。
                        const next =
                          allow.length === 0
                            ? [name]
                            : allow.includes(name)
                              ? allow.filter((entry) => entry !== name)
                              : [...allow, name];
                        // 全取消等于恢复"全选"语义，避免出现"一个都不接"的空号。
                        setAllow(next.length === publicChoices.length ? [] : next);
                      }}
                      aria-pressed={active}
                      className={`max-w-full truncate rounded px-1.5 py-0.5 font-mono text-[10px] transition-colors ${
                        active
                          ? "bg-sky-400/20 text-sky-100"
                          : "bg-white/6 text-zinc-500 hover:bg-white/12 hover:text-zinc-300"
                      }`}
                    >
                      {name}
                    </button>
                  );
                })}
              </div>
            )}
          </div>

          <div className="mt-3 rounded-lg border border-white/8 bg-white/[0.02] p-2">
            <div className="flex items-center gap-2">
              <span className="text-[11px] text-zinc-300">模型改名</span>
              <span className="text-[10px] text-zinc-600">本渠道的模型 → 对外叫成什么</span>
              <button
                type="button"
                onClick={() => setRenaming(true)}
                className="ml-auto flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] text-zinc-300 hover:bg-white/10"
              >
                <Plus className="h-3 w-3" />
                添加
              </button>
            </div>
            {rename.length === 0 && !renaming ? (
              <p className="mt-1.5 text-[10px] leading-relaxed text-zinc-600">
                不需要就不用填。典型用法：Trae 把这个模型叫
                <span className="font-mono"> DeepSeek-V4-Flash-Official</span>，
                你可以让它对外叫 <span className="font-mono">deepseek-v4.1-flash</span> ——
                这样别的渠道同名模型就能和它互为备份。
              </p>
            ) : (
              <div className="mt-1.5 flex flex-col gap-1">
                {rename.map((entry, index) => (
                  <div key={index} className="flex items-center gap-1.5">
                    {/* 方向固定为「本渠道模型 → 对外名」：
                        · 左边**只能**是本渠道目录里的名字（它就是上游真正要收的值）；
                        · 右边是客户端将来请求的名字，可以从全局清单里挑，也能手填。 */}
                    <div className="min-w-0 flex-1">
                      <Select
                        value={entry.to}
                        options={upstreamNames.map((name) => ({ value: name, label: name }))}
                        onChange={(value) =>
                          setRename((current) =>
                            current.map((item, at) =>
                              at === index ? { ...item, to: value } : item,
                            ),
                          )
                        }
                        placeholder="选本渠道的模型"
                        menuTitle="该渠道的上游模型名"
                        className="h-6 px-1.5 py-0 font-mono text-[10px]"
                        aria-label="本渠道的模型"
                      />
                    </div>
                    <span className="shrink-0 text-[10px] text-zinc-600">→</span>
                    <div className="min-w-0 flex-1">
                      <Select
                        value={entry.from}
                        options={publicChoices.map((name) => ({ value: name, label: name }))}
                        onChange={(value) =>
                          setRename((current) =>
                            current.map((item, at) =>
                              at === index ? { ...item, from: value } : item,
                            ),
                          )
                        }
                        placeholder="对外叫成"
                        menuTitle="对外的模型名（客户端用它请求）"
                        className="h-6 px-1.5 py-0 font-mono text-[10px]"
                        aria-label="对外模型名"
                      />
                    </div>
                    <button
                      type="button"
                      onClick={() => setRename((current) => current.filter((_, at) => at !== index))}
                      aria-label="删除这条改名"
                      className="grid h-5 w-5 shrink-0 place-items-center rounded text-zinc-500 hover:bg-rose-400/10 hover:text-rose-300"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </div>
                ))}
                {renaming && (
                  <div className="flex items-center gap-1.5">
                    <div className="min-w-0 flex-1">
                      <Select
                        value=""
                        options={upstreamNames.map((name) => ({ value: name, label: name }))}
                        onChange={(value) => {
                          setRename((current) => [...current, { from: value, to: value }]);
                          setRenaming(false);
                        }}
                        placeholder="选本渠道的模型"
                        menuTitle="该渠道的上游模型名"
                        className="h-6 px-1.5 py-0 font-mono text-[10px]"
                        aria-label="新增：本渠道的模型"
                      />
                    </div>
                    <span className="shrink-0 text-[10px] text-zinc-600">→</span>
                    <span className="min-w-0 flex-1 text-[10px] text-zinc-600">
                      默认同名，选完再改对外名
                    </span>
                    <button
                      type="button"
                      onClick={() => setRenaming(false)}
                      aria-label="取消添加"
                      className="grid h-5 w-5 shrink-0 place-items-center rounded text-zinc-500 hover:bg-white/10 hover:text-zinc-200"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </div>
                )}
              </div>
            )}
          </div>

          <p className="mt-3 text-[10px] leading-relaxed text-zinc-500">
            并发/优先级/权重留空 = 跟随渠道（{channel?.display_name || account.channel_id}：
            优先级 {channel?.priority ?? 100} · 权重 {channel?.weight ?? 1}）。
            凭据与额度不受影响。
          </p>
        </div>

        {hint && (
          <p className="mt-2 shrink-0 rounded-lg border border-amber-400/25 bg-amber-400/10 px-2.5 py-1.5 text-[10px] leading-relaxed text-amber-100">
            {hint}
          </p>
        )}

        <footer className="mt-3 flex shrink-0 items-center gap-2">
          <span className="truncate font-mono text-[10px] text-zinc-600">uid {account.uid}</span>
          <button
            type="button"
            onClick={onClose}
            className="ml-auto rounded-md bg-white/8 px-3 py-1 text-[11px] text-zinc-300 hover:bg-white/14"
          >
            取消
          </button>
          <button
            type="button"
            onClick={() => void save()}
            disabled={saving}
            className="rounded-md bg-emerald-400/20 px-3 py-1 text-[11px] text-emerald-100 hover:bg-emerald-400/30 disabled:opacity-40"
          >
            {saving ? "保存中…" : "保存"}
          </button>
        </footer>
      </div>
    </div>
  );
}
