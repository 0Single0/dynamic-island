import { useEffect, useMemo, useState } from "react";
import { Download, Plus, Trash2, X } from "lucide-react";

import * as api from "@/api/gateway";
import { Select } from "@/components/common/Select";
import { cn } from "@/lib/utils";
import { toast } from "@/utils/toast";
import { errorText, formatTokens } from "./shared";

/**
 * 自定义供应商：地址 + 名称 + Key + 模型。
 *
 * 「模型」不是只能手填 —— 点「获取模型」按通用约定拉一次上游清单
 * （候选地址 / 协议鉴权 / 各家方言解析都在 Rust 侧，见 channels/custom.rs）。
 * 拉不到（供应商没开这个端点）也不拦人：手填模型 ID 照样能保存，
 * 这是通用做法，也是我们对上游的诚实态度。
 */
export interface CustomProviderDraft {
  /** 已保存的渠道 id；为空表示新建。 */
  channelId?: string;
  displayName: string;
  baseUrl: string;
  apiKey: string;
  protocol: string;
  modelsUrl: string;
  /// 是否需要密钥：本地推理服务（Ollama 等）可以不填。
  requireApiKey: boolean;
  priority: number;
}

/** 一条待保存的模型（含面板要展示、矩阵要用的元数据）。 */
interface ModelDraft {
  /** 本地临时键，仅用于列表 key 与编辑定位。 */
  key: string;
  id: string;
  contextLength: string;
  maxOutput: string;
  vision: boolean;
  toolCalls: boolean;
  reasoning: boolean;
  /** 思考档位，空格/逗号分隔（如 `low high max`）。 */
  efforts: string;
  defaultEffort: string;
  /** 强制思考：模型总是带思考，关不掉。 */
  alwaysReasoning: boolean;
  /** 拉取回来默认全选；用户取消勾选的不会保存。 */
  selected: boolean;
}

const protocolOptions = [
  { value: "openai-chat", label: "OpenAI 兼容", description: "/chat/completions" },
  { value: "openai-responses", label: "OpenAI Responses", description: "/responses" },
  { value: "anthropic-messages", label: "Anthropic 兼容", description: "/messages" },
];

/**
 * 思考档位的**候选表**（不是"已选值的反推"）。
 *
 * 早先这里的下拉选项是从 `draft.efforts` 现算的 —— 一个都没选时菜单里没有任何条目，
 * 于是第一档永远加不进去（"自定义供应商看不到思考档位"就是这个）。
 * 候选必须来自固定表，再并上用户已填的自定义值。
 */
export const EFFORT_PRESETS = ["low", "medium", "high", "xhigh", "max"];

/** `"low high"` → `["low","high"]`：空白、逗号、中文逗号都当分隔符。 */
export function parseEfforts(value: string): string[] {
  return value
    .split(/[\s,，]+/)
    .map((entry) => entry.trim())
    .filter(Boolean);
}

/** 去重后用空格拼回草稿字段（后端也按空白切分）。 */
export function joinEfforts(list: string[]): string {
  return [...new Set(list.filter(Boolean))].join(" ");
}

/** 切换一个档位：有就移除，没有就追加。 */
export function toggleEffort(current: string[], level: string): string[] {
  return current.includes(level)
    ? current.filter((entry) => entry !== level)
    : [...current, level];
}

export const blankProvider: CustomProviderDraft = {
  displayName: "",
  baseUrl: "",
  apiKey: "",
  protocol: "openai-chat",
  modelsUrl: "",
  requireApiKey: true,
  priority: 100,
};

export function newModelDraft(id = ""): ModelDraft {
  return {
    key: crypto.randomUUID(),
    id,
    contextLength: "",
    maxOutput: "",
    vision: false,
    toolCalls: false,
    reasoning: false,
    efforts: "",
    defaultEffort: "",
    alwaysReasoning: false,
    selected: true,
  };
}

/** 已保存的渠道 options → 编辑用的草稿（面板打开时就地回填）。 */
export function providerFromChannel(options: unknown, priority: number): CustomProviderDraft {
  const source = (options ?? {}) as Record<string, unknown>;
  const text = (key: string) =>
    typeof source[key] === "string" ? (source[key] as string) : "";
  return {
    displayName: "",
    baseUrl: text("base_url"),
    apiKey: "",
    protocol: text("protocol") || "openai-chat",
    modelsUrl: text("models_url"),
    requireApiKey: source.require_api_key !== false,
    // 优先级来自渠道配置（不在 options 里）：不回填就会在编辑时被重置成 100。
    priority,
  };
}

/** 已保存的渠道 options → 模型草稿列表。 */
export function modelsFromChannel(options: unknown): ModelDraft[] {
  const source = (options ?? {}) as Record<string, unknown>;
  const items = Array.isArray(source.models) ? source.models : [];
  return items.map((item) => {
    const entry = (typeof item === "string" ? { id: item } : item) as Record<string, unknown>;
    const capabilities = (entry.capabilities ?? {}) as Record<string, unknown>;
    const efforts = Array.isArray(entry.reasoning_efforts)
      ? (entry.reasoning_efforts as unknown[]).filter((v): v is string => typeof v === "string")
      : [];
    const number = (value: unknown) => (typeof value === "number" ? String(value) : "");
    return {
      ...newModelDraft(typeof entry.id === "string" ? entry.id : ""),
      contextLength: number(entry.context_length),
      maxOutput: number(entry.max_output_tokens),
      vision: capabilities.vision === true,
      toolCalls: capabilities.tool_calls === true,
      reasoning: capabilities.reasoning === true,
      efforts: efforts.join(" "),
      defaultEffort:
        typeof entry.reasoning_default_effort === "string" ? entry.reasoning_default_effort : "",
      alwaysReasoning: entry.always_reasoning === true,
    };
  });
}

/** 面板草稿 → 后端要的模型数组（只发选中的，空字段省略）。 */
export function serializeModels(drafts: ModelDraft[]): api.CustomProviderModel[] {
  return drafts
    .filter((draft) => draft.selected && draft.id.trim() !== "")
    .map((draft) => {
      const number = (value: string) => {
        const parsed = Number.parseInt(value, 10);
        return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
      };
      const efforts = parseEfforts(draft.efforts);
      return {
        id: draft.id.trim(),
        context_length: number(draft.contextLength),
        max_output_tokens: number(draft.maxOutput),
        capabilities: {
          vision: draft.vision,
          tool_calls: draft.toolCalls,
          reasoning: draft.reasoning,
        },
        reasoning_efforts: efforts,
        // 默认档必须落在可选档位里：后端/面板都会按它显示"默认 xxx"，
        // 选了一个不存在的档位等于对外宣传一个用不了的默认值。
        reasoning_default_effort:
          draft.defaultEffort.trim() && efforts.includes(draft.defaultEffort.trim())
            ? draft.defaultEffort.trim()
            : null,
        always_reasoning: draft.alwaysReasoning,
      };
    });
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="grid gap-1.5">
      <span className="flex items-baseline gap-1.5 text-[11px] text-zinc-400">
        {label}
        {hint && <span className="text-[10px] text-zinc-600">{hint}</span>}
      </span>
      {children}
    </label>
  );
}

function TextInput({
  className,
  ...props
}: React.InputHTMLAttributes<HTMLInputElement>) {
  // cn（tailwind-merge）是必须的：调用方传 w-44 这类定宽时，
  // 基础样式里的 w-full 必须被合并掉 —— 字符串拼接的话两个类都在，
  // 谁生效取决于样式表顺序，结果就是"输入框占满整行"。
  return (
    <input
      {...props}
      className={cn(
        "h-7 w-full rounded-md border border-white/10 bg-black/30 px-2 text-[11px] text-zinc-200 outline-none transition placeholder:text-zinc-600 focus:border-sky-400/50",
        className,
      )}
    />
  );
}

/** 能力开关：图片 / 工具 / 思考（对应面板模型卡上的三个徽标）。 */
function CapabilityToggle({
  label,
  active,
  onChange,
}: {
  label: string;
  active: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <button
      type="button"
      onClick={(event) => {
        event.stopPropagation();
        onChange(!active);
      }}
      className={`rounded px-1.5 py-0.5 text-[10px] transition-colors ${
        active
          ? "bg-sky-400/20 text-sky-100"
          : "bg-white/6 text-zinc-500 hover:bg-white/12 hover:text-zinc-300"
      }`}
    >
      {label}
    </button>
  );
}

/** 单条模型：一行放全（ID + 摘要 + 操作），点「设置」在行内展开编辑区。 */
function ModelRow({
  draft,
  onChange,
  onRemove,
}: {
  draft: ModelDraft;
  onChange: (patch: Partial<ModelDraft>) => void;
  onRemove: () => void;
}) {
  const [open, setOpen] = useState(false);
  const context = Number.parseInt(draft.contextLength, 10);
  const output = Number.parseInt(draft.maxOutput, 10);
  const efforts = parseEfforts(draft.efforts);
  const badges = [
    draft.vision ? "图片" : null,
    draft.toolCalls ? "工具" : null,
    draft.reasoning ? "思考" : null,
  ].filter((badge): badge is string => badge !== null);

  return (
    <div
      className={`rounded-lg border ${
        draft.selected ? "border-white/10 bg-white/[0.03]" : "border-white/6 bg-white/[0.01]"
      }`}
    >
      {/* 一行：勾选 + ID + 摘要 + 设置/删除。摘要直接铺在行内，不再另起一行。 */}
      <div className="flex items-center gap-2 px-2 py-1.5">
        <input
          type="checkbox"
          checked={draft.selected}
          onChange={(event) => onChange({ selected: event.target.checked })}
          aria-label={`保存 ${draft.id || "该模型"}`}
          className="h-3 w-3 shrink-0 accent-sky-400"
        />
        {open ? (
          <TextInput
            autoFocus
            value={draft.id}
            onChange={(event) => onChange({ id: event.target.value })}
            placeholder="模型 ID，如 deepseek-v4.1-flash"
            className="h-6 w-44 shrink-0 font-mono"
          />
        ) : (
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              setOpen(true);
            }}
            className="w-44 shrink-0 truncate text-left font-mono text-[11px] text-zinc-100 hover:text-sky-200"
            title={draft.id || "点击编辑模型 ID"}
          >
            {draft.id || "(未填模型 ID)"}
          </button>
        )}
        {!open && (
          <span className="flex min-w-0 flex-1 flex-wrap items-center gap-1 text-[9px] text-zinc-500">
            <span>上下文 {context > 0 ? formatTokens(context) : "?"}</span>
            <span className="text-zinc-700">·</span>
            <span>输出 {output > 0 ? formatTokens(output) : "?"}</span>
            {badges.map((badge) => (
              <span key={badge} className="rounded bg-white/8 px-1 text-zinc-300">
                {badge}
              </span>
            ))}
            {efforts.length > 0 && <span>档位 {efforts.join("/")}</span>}
            {draft.defaultEffort && (
              <span className="text-zinc-400">默认 {draft.defaultEffort}</span>
            )}
            {draft.alwaysReasoning && <span className="text-amber-300">强制思考</span>}
          </span>
        )}
        <span className="ml-auto flex shrink-0 items-center gap-0.5">
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              setOpen(!open);
            }}
            className="rounded px-1.5 py-0.5 text-[10px] text-zinc-500 hover:bg-white/10 hover:text-zinc-200"
          >
            {open ? "收起" : "设置"}
          </button>
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              onRemove();
            }}
            title="移除该模型"
            aria-label={`移除 ${draft.id || "该模型"}`}
            className="grid h-6 w-6 place-items-center rounded text-zinc-500 hover:bg-rose-400/10 hover:text-rose-300"
          >
            <Trash2 className="h-3 w-3" />
          </button>
        </span>
      </div>

      {open && (
        <div className="border-t border-white/8 px-2 py-2">
          <div className="grid grid-cols-2 gap-2">
            <Field label="上下文窗口" hint="token">
              <TextInput
                inputMode="numeric"
                value={draft.contextLength}
                onChange={(event) =>
                  onChange({ contextLength: event.target.value.replace(/\D/g, "") })
                }
                placeholder="1000000"
              />
            </Field>
            <Field label="最大输出" hint="token">
              <TextInput
                inputMode="numeric"
                value={draft.maxOutput}
                onChange={(event) => onChange({ maxOutput: event.target.value.replace(/\D/g, "") })}
                placeholder="128000"
              />
            </Field>
          </div>

          <div className="mt-2 flex items-center gap-1.5">
            <span className="text-[10px] text-zinc-500">能力</span>
            <CapabilityToggle
              label="图片"
              active={draft.vision}
              onChange={(next) => onChange({ vision: next })}
            />
            <CapabilityToggle
              label="工具"
              active={draft.toolCalls}
              onChange={(next) => onChange({ toolCalls: next })}
            />
            <CapabilityToggle
              label="思考"
              active={draft.reasoning}
              onChange={(next) => onChange({ reasoning: next })}
            />
          </div>

          <div className="mt-2">
            <Field label="思考档位" hint="点一下切换，可多选">
              {/* 候选取自固定表 + 已填的自定义值：早先是从"已选值"反推，
                  一个都没选时菜单是空的，第一档永远加不进去。 */}
              <span className="flex flex-wrap items-center gap-1">
                {[...new Set([...EFFORT_PRESETS, ...efforts])].map((level) => {
                  const active = efforts.includes(level);
                  return (
                    <button
                      key={level}
                      type="button"
                      onClick={(event) => {
                        event.stopPropagation();
                        const next = toggleEffort(efforts, level);
                        // 取消掉默认档时一并清空，避免留下一个不在列表里的默认值。
                        onChange({
                          efforts: joinEfforts(next),
                          defaultEffort:
                            draft.defaultEffort === level && !next.includes(level)
                              ? ""
                              : draft.defaultEffort,
                        });
                      }}
                      aria-pressed={active}
                      className={`rounded px-1.5 py-0.5 font-mono text-[10px] transition-colors ${
                        active
                          ? "bg-sky-400/20 text-sky-100"
                          : "bg-white/6 text-zinc-500 hover:bg-white/12 hover:text-zinc-300"
                      }`}
                    >
                      {level}
                    </button>
                  );
                })}
                <input
                  // 只显示"预置表之外"的自定义值：预置档位已经由上面的 chip 表达，
                  // 再把同一个值镜像进输入框（low/high 同时出现在两处）会让人以为要重复填。
                  value={efforts.filter((level) => !EFFORT_PRESETS.includes(level)).join(" ")}
                  onChange={(event) => {
                    const custom = parseEfforts(event.target.value);
                    const preset = efforts.filter((level) => EFFORT_PRESETS.includes(level));
                    onChange({ efforts: joinEfforts([...preset, ...custom]) });
                  }}
                  placeholder="自定义，空格分隔"
                  aria-label="自定义思考档位"
                  className="h-5 w-32 rounded border border-white/10 bg-black/30 px-1.5 font-mono text-[10px] text-zinc-200 outline-none placeholder:text-zinc-600 focus:border-sky-400/50"
                />
              </span>
            </Field>
          </div>

          <div className="mt-2 grid grid-cols-2 gap-2">
            <Field label="默认档位" hint={efforts.length === 0 ? "先勾一个档位" : undefined}>
              <Select
                value={draft.defaultEffort}
                options={
                  efforts.length === 0
                    ? [{ value: "", label: "先选思考档位" }]
                    : [
                        { value: "", label: "不指定" },
                        ...efforts.map((level) => ({ value: level, label: level })),
                      ]
                }
                onChange={(value) => onChange({ defaultEffort: value })}
                placeholder="不指定"
                className="h-7 text-[11px]"
                aria-label="默认档位"
              />
            </Field>
            <Field label="强制思考">
              <label className="flex h-7 items-center gap-1.5 text-[10px] text-zinc-400">
                <input
                  type="checkbox"
                  checked={draft.alwaysReasoning}
                  onChange={(event) => onChange({ alwaysReasoning: event.target.checked })}
                  className="h-3 w-3 accent-sky-400"
                />
                该模型总是带思考，关不掉
              </label>
            </Field>
          </div>
        </div>
      )}
    </div>
  );
}

/**
 * 自定义供应商弹窗。渠道与密钥一次写入 —— 分两步会出现"渠道建好但没有账号"的中间态。
 */
export function CustomProviderDialog({
  editing,
  onClose,
  onSaved,
}: {
  /** 传入即编辑已有供应商（回填地址、优先级与模型）。 */
  editing?: { channelId: string; displayName: string; options: unknown; priority: number };
  onClose: () => void;
  onSaved: () => void;
}) {
  const [draft, setDraft] = useState<CustomProviderDraft>(() => {
    if (!editing) return blankProvider;
    return {
      ...providerFromChannel(editing.options, editing.priority),
      channelId: editing.channelId,
      displayName: editing.displayName,
    };
  });
  const [models, setModels] = useState<ModelDraft[]>(() =>
    editing ? modelsFromChannel(editing.options) : [],
  );
  const [fetching, setFetching] = useState(false);
  const [saving, setSaving] = useState(false);
  const [hint, setHint] = useState<string | null>(null);

  const patch = (next: Partial<CustomProviderDraft>) =>
    setDraft((current) => ({ ...current, ...next }));

  const selectedCount = useMemo(
    () => models.filter((model) => model.selected && model.id.trim() !== "").length,
    [models],
  );

  /** 拉取模型：按 id 合并，已有条目保留用户改过的元数据。 */
  const fetchModels = async () => {
    if (!draft.baseUrl.trim()) {
      setHint("请先填写供应商地址");
      return;
    }
    setFetching(true);
    setHint(null);
    try {
      const result = await api.fetchModels({
        baseUrl: draft.baseUrl.trim(),
        apiKey: draft.apiKey.trim() || undefined,
        modelsUrl: draft.modelsUrl.trim() || undefined,
        protocol: draft.protocol,
      });
      setModels((current) => {
        const existing = new Map(current.map((model) => [model.id, model]));
        const next = result.models.map((model) => {
          const previous = existing.get(model.id);
          if (previous) {
            // 已有条目：勾选状态与用户填过的**非空**字段保留，
            // 空字段用这次拉到的元数据回填（否则第一次拉取没带上元数据时，
            // 之后再拉也永远是 "上下文 ?"）。
            return {
              ...previous,
              selected: true,
              contextLength: previous.contextLength || (model.context_length ? String(model.context_length) : ""),
              maxOutput: previous.maxOutput || (model.max_output_tokens ? String(model.max_output_tokens) : ""),
              vision: previous.vision || model.capabilities.vision,
              toolCalls: previous.toolCalls || model.capabilities.tool_calls,
              reasoning: previous.reasoning || model.capabilities.reasoning,
            };
          }
          return {
            ...newModelDraft(model.id),
            contextLength: model.context_length ? String(model.context_length) : "",
            maxOutput: model.max_output_tokens ? String(model.max_output_tokens) : "",
            vision: model.capabilities.vision,
            toolCalls: model.capabilities.tool_calls,
            reasoning: model.capabilities.reasoning,
          };
        });
        // 保留用户手填、但这次没拉到的模型（上游清单常常不全）。
        for (const model of current) {
          if (!next.some((entry) => entry.id === model.id)) next.push(model);
        }
        return next;
      });
      toast.success(`获取到 ${result.count} 个模型`);
    } catch (reason) {
      setHint(
        `${errorText(reason, "获取模型失败")}。可以不获取，直接在下面手填模型 ID。`,
      );
    } finally {
      setFetching(false);
    }
  };

  const save = async () => {
    if (!draft.displayName.trim()) {
      setHint("请填写供应商名称");
      return;
    }
    if (!draft.baseUrl.trim()) {
      setHint("请填写供应商地址");
      return;
    }
    if (draft.requireApiKey && !draft.apiKey.trim() && !editing) {
      setHint("请填写 API Key（本地服务可关闭「需要密钥」）");
      return;
    }
    setSaving(true);
    setHint(null);
    try {
      await api.saveCustomProvider({
        channelId: draft.channelId,
        displayName: draft.displayName.trim(),
        baseUrl: draft.baseUrl.trim(),
        apiKey: draft.apiKey.trim() || undefined,
        protocol: draft.protocol,
        modelsUrl: draft.modelsUrl.trim() || undefined,
        models: serializeModels(models),
        requireApiKey: draft.requireApiKey,
        priority: draft.priority,
      });
      toast.success(editing ? "供应商已更新" : "供应商已添加");
      onSaved();
      onClose();
    } catch (reason) {
      setHint(errorText(reason, "保存失败"));
    } finally {
      setSaving(false);
    }
  };

  // Esc 关闭：与「添加账号」弹窗的交互一致。
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

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
          <span className="text-[13px] font-medium text-zinc-100">
            {editing ? "编辑供应商" : "自定义供应商"}
          </span>
          <span className="text-[10px] text-zinc-500">
            任意 OpenAI / Anthropic 兼容端点，接入后与内置渠道同等调度
          </span>
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭"
            className="ml-auto grid h-6 w-6 place-items-center rounded-md text-zinc-400 hover:bg-white/12 hover:text-zinc-100"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </header>

        <div className="mt-3 min-h-0 flex-1 overflow-y-auto pr-1">
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="名称">
              <TextInput
                autoFocus
                value={draft.displayName}
                onChange={(event) => patch({ displayName: event.target.value })}
                placeholder="例如：智谱 GLM"
              />
            </Field>
            <Field label="API 格式">
              <Select
                value={draft.protocol}
                options={protocolOptions}
                onChange={(value) => patch({ protocol: value })}
                className="h-7 text-[11px]"
                aria-label="API 格式"
              />
            </Field>
          </div>

          <div className="mt-3">
            <Field label="地址" hint="以 /v1 结尾，或直接粘完整端点">
              <TextInput
                value={draft.baseUrl}
                onChange={(event) => patch({ baseUrl: event.target.value })}
                placeholder="https://api.example.com/v1"
                className="font-mono"
              />
            </Field>
          </div>

          <div className="mt-3 grid gap-3 sm:grid-cols-2">
            <Field label="API Key">
              <TextInput
                type="password"
                value={draft.apiKey}
                onChange={(event) => patch({ apiKey: event.target.value })}
                placeholder={editing ? "留空则保持不变" : "sk-..."}
                className="font-mono"
              />
            </Field>
            <Field label="模型清单地址" hint="留空自动推导">
              <TextInput
                value={draft.modelsUrl}
                onChange={(event) => patch({ modelsUrl: event.target.value })}
                placeholder="可选"
                className="font-mono"
              />
            </Field>
          </div>

          <div className="mt-2 flex flex-wrap items-center gap-3">
            <label className="flex items-center gap-1.5 text-[10px] text-zinc-400">
              <input
                type="checkbox"
                checked={draft.requireApiKey}
                onChange={(event) => patch({ requireApiKey: event.target.checked })}
                className="h-3 w-3 accent-sky-400"
              />
              需要密钥（本地服务可关闭）
            </label>
            <label className="flex items-center gap-1.5 text-[10px] text-zinc-400">
              优先级
              <TextInput
                inputMode="numeric"
                value={String(draft.priority)}
                onChange={(event) =>
                  patch({ priority: Number.parseInt(event.target.value.replace(/\D/g, ""), 10) || 0 })
                }
                className="h-6 w-16 text-center"
              />
              <span className="text-[10px] text-zinc-600">越大越先用</span>
            </label>
          </div>

          <div className="mt-4 flex items-center gap-2">
            <span className="text-[11px] font-medium text-zinc-200">模型</span>
            <span className="text-[10px] text-zinc-500">
              已选 {selectedCount} 个 · 点「设置」改上下文/能力/思考档位
            </span>
            <button
              type="button"
              onClick={() => void fetchModels()}
              disabled={fetching}
              className="ml-auto flex items-center gap-1 rounded-md bg-sky-400/15 px-2.5 py-1 text-[11px] text-sky-200 hover:bg-sky-400/25 disabled:opacity-40"
            >
              <Download className="h-3.5 w-3.5" />
              {fetching ? "获取中…" : "获取模型"}
            </button>
            <button
              type="button"
              onClick={() => setModels((current) => [...current, newModelDraft()])}
              className="flex items-center gap-1 rounded-md bg-white/10 px-2.5 py-1 text-[11px] text-zinc-200 hover:bg-white/16"
            >
              <Plus className="h-3.5 w-3.5" />
              手动添加
            </button>
          </div>

          {/* 模型区限高独立滚动：模型多时弹窗高度稳定，页脚（取消/保存）不被推出视口。 */}
          <div className="gw-scroll mt-2 flex max-h-72 min-h-0 flex-col gap-1 overflow-y-auto pr-1">
            {models.length === 0 ? (
              <div className="rounded-lg border border-dashed border-white/12 px-3 py-5 text-center text-[11px] text-zinc-500">
                点「获取模型」按地址拉取，或「手动添加」填模型 ID。
              </div>
            ) : (
              models.map((model) => (
                <ModelRow
                  key={model.key}
                  draft={model}
                  onChange={(next) =>
                    setModels((current) =>
                      current.map((entry) =>
                        entry.key === model.key ? { ...entry, ...next } : entry,
                      ),
                    )
                  }
                  onRemove={() =>
                    setModels((current) => current.filter((entry) => entry.key !== model.key))
                  }
                />
              ))
            )}
          </div>

          {hint && (
            <p className="mt-2 rounded-lg border border-amber-400/25 bg-amber-400/10 px-2.5 py-1.5 text-[10px] leading-relaxed text-amber-100">
              {hint}
            </p>
          )}
        </div>

        <footer className="mt-4 flex shrink-0 items-center gap-2">
          <span className="text-[10px] text-zinc-500">
            密钥写入系统凭据库；模型清单决定这个供应商对外提供哪些模型。
          </span>
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
