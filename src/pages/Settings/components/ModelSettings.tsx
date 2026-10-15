import { useEffect, useMemo, useState, type Dispatch, type InputHTMLAttributes, type ReactNode, type SetStateAction } from "react";
import { Pencil, Plus, Save, Trash2, X } from "lucide-react";
import { tauriInvoke, type ModelRecord, type ProviderRecord } from "@/agent/api";
import { Select } from "@/components/common/Select";

const protocolOptions = [
  { value: "chat-completions", label: "OpenAI Chat Completions", description: "/v1/chat/completions" },
  { value: "responses", label: "OpenAI Responses", description: "/v1/responses" },
  { value: "anthropic-messages", label: "Anthropic Messages", description: "/v1/messages" },
];

const blankProvider = { name: "", protocol: "chat-completions", baseUrl: "https://api.openai.com/v1", apiKey: "" };
const blankModel = { id: undefined as string | undefined, modelId: "", displayName: "", contextWindow: "128000" };

/** 尚未保存的供应商下暂存的模型；保存供应商时随之落库。 */
interface DraftModel {
  /** 本地临时键，仅用于列表 key 与编辑定位。 */
  key: string;
  modelId: string;
  displayName: string;
  contextWindow: string;
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return <label className="grid gap-1.5 text-xs text-zinc-400"><span>{label}</span>{children}</label>;
}

function TextInput({ className, ...props }: InputHTMLAttributes<HTMLInputElement>) {
  return <input {...props} className={`w-full rounded-lg border border-white/10 bg-white/[0.025] px-3 py-2 text-sm text-zinc-200 outline-none transition placeholder:text-zinc-600 focus:border-sky-300/45 ${className ?? ""}`} />;
}

function ModelModal({ form, setForm, onClose, onSave }: { form: typeof blankModel; setForm: Dispatch<SetStateAction<typeof blankModel>>; onClose: () => void; onSave: () => void }) {
  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/65 p-6 backdrop-blur-sm">
      <div className="w-full max-w-xl rounded-2xl border border-white/[0.12] bg-[#252629] p-5 shadow-2xl shadow-black/50">
        <div className="flex items-center justify-between">
          <h2 className="text-base font-semibold text-zinc-100">{form.id ? "编辑模型" : "添加模型"}</h2>
          <button type="button" onClick={onClose} className="rounded-lg p-1.5 text-zinc-500 hover:bg-white/[0.08] hover:text-zinc-200"><X className="h-4 w-4" /></button>
        </div>
        <div className="mt-5 grid gap-4">
          <Field label="模型 ID"><TextInput autoFocus value={form.modelId} onChange={(event) => setForm((current) => ({ ...current, modelId: event.target.value }))} placeholder="例如 deepseek-v4.1-flash" /></Field>
          <Field label="显示名称"><TextInput value={form.displayName} onChange={(event) => setForm((current) => ({ ...current, displayName: event.target.value }))} placeholder="留空则使用模型 ID" /></Field>
          <Field label="上下文窗口"><TextInput inputMode="numeric" value={form.contextWindow} onChange={(event) => setForm((current) => ({ ...current, contextWindow: event.target.value }))} placeholder="128000" /></Field>
          <div className="rounded-lg border border-white/[0.08] bg-white/[0.025] px-3 py-2.5 text-xs text-zinc-400">输入类型：文本　·　输出类型：文本</div>
        </div>
        <div className="mt-6 flex justify-end gap-2">
          <button type="button" onClick={onClose} className="rounded-lg border border-white/10 px-3 py-2 text-xs text-zinc-400 hover:bg-white/[0.06]">取消</button>
          <button type="button" onClick={onSave} className="rounded-lg bg-zinc-100 px-3 py-2 text-xs font-medium text-zinc-900 hover:bg-white"><Save className="mr-1.5 inline h-3.5 w-3.5" />保存</button>
        </div>
      </div>
    </div>
  );
}

/**
 * AI 与模型设置：管理自定义模型供应商及其模型。
 *
 * 这里保存的是全局配置，岛体那个窗口不会因为设置窗口打开而卸载，
 * 所以后端保存成功后会广播 `agent://config-changed`（见 agent/config.rs），
 * 由使用方（如 AI 对话组件的模型下拉）自行重读。
 */
export function ModelSettings() {
  const [providers, setProviders] = useState<ProviderRecord[]>([]);
  const [models, setModels] = useState<ModelRecord[]>([]);
  const [activeProviderId, setActiveProviderId] = useState("");
  const [providerForm, setProviderForm] = useState(blankProvider);
  const [modelForm, setModelForm] = useState(blankModel);
  const [draftModels, setDraftModels] = useState<DraftModel[]>([]);
  const [editingDraftKey, setEditingDraftKey] = useState<string | null>(null);
  const [notice, setNotice] = useState("");
  const [modelModalOpen, setModelModalOpen] = useState(false);

  const activeProvider = providers.find((provider) => provider.id === activeProviderId);
  const providerModels = useMemo(() => models.filter((model) => model.providerId === activeProviderId), [models, activeProviderId]);
  /** 新建供应商尚未拿到 id，其模型先暂存在 draftModels，保存供应商时一起落库。 */
  const isNewProvider = !activeProviderId;
  /** 内置「本机反代」：模型来自网关目录实时合成，不能编辑供应商、增删模型。 */
  const isBuiltinProvider = Boolean(activeProvider?.builtin);

  useEffect(() => {
    void Promise.all([
      tauriInvoke<ProviderRecord[]>("agent_list_provider_configs"),
      tauriInvoke<ModelRecord[]>("agent_list_model_configs"),
    ])
      .then(([nextProviders, nextModels]) => {
        setProviders(nextProviders);
        setModels(nextModels);
        setActiveProviderId(nextProviders[0]?.id ?? "");
      })
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    if (activeProvider) setProviderForm({ name: activeProvider.name, protocol: activeProvider.protocol, baseUrl: activeProvider.baseUrl, apiKey: "" });
  }, [activeProviderId, activeProvider]);

  const saveProvider = async () => {
    setNotice("");
    try {
      const provider = await tauriInvoke<ProviderRecord>("agent_save_provider_config", { input: { ...providerForm, id: activeProvider?.id, enabled: true } });
      setProviders((current) => [...current.filter((item) => item.id !== provider.id), provider]);
      setActiveProviderId(provider.id);
      setProviderForm((current) => ({ ...current, apiKey: "" }));
      // 新建供应商时暂存的模型现在有了 provider_id，逐个落库；失败不整体回滚，只提示哪条没写进去。
      if (draftModels.length) {
        const failed: string[] = [];
        for (const draft of draftModels) {
          try {
            const model = await tauriInvoke<ModelRecord>("agent_save_model_config", { input: { providerId: provider.id, modelId: draft.modelId, displayName: draft.displayName, contextWindow: Number(draft.contextWindow) || null, enabled: true } });
            setModels((current) => [...current.filter((item) => item.id !== model.id), model]);
          } catch {
            failed.push(draft.modelId);
          }
        }
        setDraftModels([]);
        setNotice(failed.length ? `供应商已保存，但模型 ${failed.join("、")} 写入失败。` : "供应商与模型已保存，API Key 仅写入系统凭据库。");
      } else {
        setNotice("供应商已保存，API Key 仅写入系统凭据库。");
      }
    } catch (error) {
      setNotice(error instanceof Error ? error.message : "保存供应商失败");
    }
  };

  const saveModel = async () => {
    if (!modelForm.modelId.trim()) return;
    // 供应商尚未保存：模型先暂存，等供应商落库后一起写入。
    if (isNewProvider) {
      const entry: DraftModel = { key: editingDraftKey ?? `draft-${crypto.randomUUID()}`, modelId: modelForm.modelId.trim(), displayName: modelForm.displayName.trim(), contextWindow: modelForm.contextWindow };
      setDraftModels((current) => (editingDraftKey ? current.map((item) => (item.key === editingDraftKey ? entry : item)) : [...current, entry]));
      setModelForm(blankModel);
      setEditingDraftKey(null);
      setModelModalOpen(false);
      return;
    }
    try {
      const model = await tauriInvoke<ModelRecord>("agent_save_model_config", { input: { ...modelForm, providerId: activeProviderId, contextWindow: Number(modelForm.contextWindow) || null, enabled: true } });
      setModels((current) => [...current.filter((item) => item.id !== model.id), model]);
      setModelForm(blankModel);
      setModelModalOpen(false);
    } catch (error) {
      setNotice(error instanceof Error ? error.message : "保存模型失败");
    }
  };

  const editModel = (model: ModelRecord) => {
    setEditingDraftKey(null);
    setModelForm({ id: model.id, modelId: model.modelId, displayName: model.displayName, contextWindow: model.contextWindow?.toString() ?? "" });
    setModelModalOpen(true);
  };

  const editDraftModel = (draft: DraftModel) => {
    setEditingDraftKey(draft.key);
    setModelForm({ id: undefined, modelId: draft.modelId, displayName: draft.displayName, contextWindow: draft.contextWindow });
    setModelModalOpen(true);
  };

  const deleteDraftModel = (draft: DraftModel) => {
    setDraftModels((current) => current.filter((item) => item.key !== draft.key));
  };

  const deleteModel = async (model: ModelRecord) => {
    try {
      await tauriInvoke<void>("agent_delete_model_config", { modelId: model.id });
      setModels((current) => current.filter((item) => item.id !== model.id));
    } catch (error) {
      setNotice(error instanceof Error ? error.message : "删除模型失败");
    }
  };

  const addProvider = () => {
    setActiveProviderId("");
    setProviderForm(blankProvider);
    setDraftModels([]);
    setEditingDraftKey(null);
    setNotice("");
  };

  /** 切换供应商：暂存模型只属于未保存的新供应商，切走即丢弃，避免错写到别家。 */
  const selectProvider = (providerId: string) => {
    setActiveProviderId(providerId);
    setDraftModels([]);
    setEditingDraftKey(null);
    setNotice("");
  };

  const openAddModel = () => {
    setEditingDraftKey(null);
    setModelForm(blankModel);
    setModelModalOpen(true);
  };

  const closeModelModal = () => {
    setModelModalOpen(false);
    setEditingDraftKey(null);
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="mb-5 shrink-0">
        <div className="text-[11px] uppercase tracking-[0.18em] text-zinc-600">Settings / AI 与模型</div>
        <h1 className="mt-1 text-xl font-semibold">模型设置</h1>
        <p className="mt-1 text-xs text-zinc-500">管理自定义模型供应商，配置后可在输入框的模型菜单中选择使用。</p>
      </header>

      <div className="grid min-h-0 flex-1 grid-cols-[200px_minmax(0,1fr)] overflow-hidden rounded-xl border border-white/[0.12] bg-[#1c1c1e]">
        <aside className="flex min-h-0 flex-col overflow-y-auto border-r border-white/10 p-2">
          <div className="px-2 py-1.5 text-xs font-medium text-zinc-500">供应商</div>
          {providers.map((provider) => (
            <button
              key={provider.id}
              type="button"
              onClick={() => selectProvider(provider.id)}
              className={`flex h-9 w-full items-center gap-2 rounded-lg px-2 text-left text-xs transition ${activeProviderId === provider.id ? "bg-white/[0.09] text-zinc-100" : "text-zinc-400 hover:bg-white/[0.05] hover:text-zinc-200"}`}
            >
              <span className={`grid h-5 w-5 shrink-0 place-items-center rounded text-[10px] ${provider.builtin ? "bg-sky-300/[0.14] text-sky-200" : "bg-white/[0.08] text-zinc-300"}`}>{provider.builtin ? "⇄" : "◇"}</span>
              <span className="min-w-0 flex-1 truncate">{provider.name}</span>
              <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${provider.hasApiKey ? "bg-emerald-400" : "bg-zinc-600"}`} />
            </button>
          ))}
          <button
            type="button"
            onClick={addProvider}
            className={`mt-1 flex h-9 w-full items-center gap-2 rounded-lg border border-white/[0.14] px-2 text-xs text-zinc-300 hover:bg-white/[0.06] ${!activeProviderId ? "bg-white/[0.06]" : ""}`}
          >
            <Plus className="h-3.5 w-3.5" />添加
          </button>
        </aside>

        <section className="min-h-0 overflow-y-auto p-5">
          <div className="mb-4">
            <h2 className="text-sm font-semibold">{isBuiltinProvider ? "本机反代" : activeProviderId ? "编辑模型供应商" : "添加模型供应商"}</h2>
            <p className="mt-1 text-xs text-zinc-500">{isBuiltinProvider ? "岛内反代号池网关：模型目录实时同步，无需手动配置。" : "配置一个完全自定义的 API 端点和初始模型。"}</p>
          </div>

          {isBuiltinProvider ? (
            <div className="max-w-2xl space-y-3">
              <div className="rounded-lg border border-sky-300/20 bg-sky-300/[0.06] px-3 py-2.5 text-xs leading-5 text-sky-100">
                这个供应商指向岛内网关，Base URL 与密钥自动取自网关配置。渠道与模型有变化时，下面的列表会自动更新。
              </div>
              <div className="rounded-lg border border-white/[0.08] bg-white/[0.025] px-3 py-2.5 text-xs text-zinc-400">
                <span className="text-zinc-500">Base URL：</span>{activeProvider?.baseUrl}
              </div>
            </div>
          ) : (
            <div className="grid max-w-2xl gap-3">
              <Field label="名称"><TextInput value={providerForm.name} onChange={(event) => setProviderForm((current) => ({ ...current, name: event.target.value }))} placeholder="例如：智谱 GLM" /></Field>
              <Field label="Base URL"><TextInput value={providerForm.baseUrl} onChange={(event) => setProviderForm((current) => ({ ...current, baseUrl: event.target.value }))} placeholder="https://api.example.com/v1" /></Field>
              <Field label="API Key"><TextInput type="password" value={providerForm.apiKey} onChange={(event) => setProviderForm((current) => ({ ...current, apiKey: event.target.value }))} placeholder={activeProvider?.hasApiKey ? "已配置，留空则保持不变" : "输入 API Key"} /></Field>
              <Field label="API 格式"><Select value={providerForm.protocol} options={protocolOptions} onChange={(value) => setProviderForm((current) => ({ ...current, protocol: value }))} /></Field>
            </div>
          )}

          <div className="mt-6 flex max-w-2xl items-center justify-between">
            <div>
              <h3 className="text-sm font-medium text-zinc-300">模型列表</h3>
              <p className="mt-0.5 text-xs text-zinc-500">{isBuiltinProvider ? "来自网关当前可用的对外模型。" : isNewProvider ? "可先添加模型，保存供应商时一起写入。" : "添加模型后即可在聊天页底部选择。"}</p>
            </div>
            {!isBuiltinProvider && (
              <button
                type="button"
                onClick={openAddModel}
                className="rounded-lg bg-white/[0.09] px-2.5 py-1.5 text-xs text-zinc-200 hover:bg-white/[0.14]"
              >
                <Plus className="mr-1 inline h-3.5 w-3.5" />添加模型
              </button>
            )}
          </div>

          <div className="mt-3 grid max-w-2xl gap-2">
            {isBuiltinProvider ? (
              providerModels.length ? providerModels.map((model) => (
                <div key={model.id} className="flex h-10 items-center gap-2 rounded-lg border border-white/[0.08] bg-black/20 px-3">
                  <div className="min-w-0 flex-1 truncate text-xs text-zinc-200">{model.displayName || model.modelId}</div>
                  {model.supportsImages && <span className="shrink-0 rounded bg-white/[0.06] px-1.5 py-0.5 text-[9px] text-zinc-500">视觉</span>}
                  <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-400" title="网关可用" />
                </div>
              )) : (
                <div className="rounded-lg border border-dashed border-white/[0.12] px-3 py-5 text-center text-xs text-zinc-500">网关当前没有可用模型，先在反代面板配置渠道与账号。</div>
              )
            ) : isNewProvider ? (
              draftModels.length ? draftModels.map((draft) => (
                <div key={draft.key} className="flex h-10 items-center gap-2 rounded-lg border border-dashed border-white/[0.14] bg-black/20 px-3">
                  <div className="min-w-0 flex-1 truncate text-xs text-zinc-200">{draft.displayName || draft.modelId}</div>
                  <span className="shrink-0 rounded bg-white/[0.06] px-1.5 py-0.5 text-[9px] text-zinc-500">待保存</span>
                  <button type="button" onClick={() => editDraftModel(draft)} title="编辑模型" aria-label={`编辑 ${draft.displayName || draft.modelId}`} className="grid h-7 w-7 place-items-center rounded-md text-zinc-500 hover:bg-white/[0.08] hover:text-zinc-100"><Pencil className="h-3.5 w-3.5" /></button>
                  <button type="button" onClick={() => deleteDraftModel(draft)} title="删除模型" aria-label={`删除 ${draft.displayName || draft.modelId}`} className="grid h-7 w-7 place-items-center rounded-md text-zinc-500 hover:bg-red-400/10 hover:text-red-300"><Trash2 className="h-3.5 w-3.5" /></button>
                </div>
              )) : (
                <div className="rounded-lg border border-dashed border-white/[0.12] px-3 py-5 text-center text-xs text-zinc-500">可先添加模型，保存供应商后即可在聊天页使用。</div>
              )
            ) : providerModels.length ? providerModels.map((model) => (
              <div key={model.id} className="flex h-10 items-center gap-2 rounded-lg border border-white/[0.08] bg-black/20 px-3">
                <div className="min-w-0 flex-1 truncate text-xs text-zinc-200">{model.displayName || model.modelId}</div>
                <button type="button" onClick={() => editModel(model)} title="编辑模型" aria-label={`编辑 ${model.displayName || model.modelId}`} className="grid h-7 w-7 place-items-center rounded-md text-zinc-500 hover:bg-white/[0.08] hover:text-zinc-100"><Pencil className="h-3.5 w-3.5" /></button>
                <button type="button" onClick={() => void deleteModel(model)} title="删除模型" aria-label={`删除 ${model.displayName || model.modelId}`} className="grid h-7 w-7 place-items-center rounded-md text-zinc-500 hover:bg-red-400/10 hover:text-red-300"><Trash2 className="h-3.5 w-3.5" /></button>
                <span className={`h-1.5 w-1.5 rounded-full ${model.enabled ? "bg-emerald-400" : "bg-zinc-600"}`} title={model.enabled ? "可用" : "已停用"} />
              </div>
            )) : (
              <div className="rounded-lg border border-dashed border-white/[0.12] px-3 py-5 text-center text-xs text-zinc-500">当前没有配置模型，添加模型后可在聊天页使用。</div>
            )}
          </div>

          {notice && <div className="mt-3 max-w-2xl rounded-lg border border-sky-300/15 bg-sky-300/[0.08] px-3 py-2 text-xs text-sky-100">{notice}</div>}

          {!isBuiltinProvider && (
            <div className="mt-5 flex max-w-2xl justify-end">
              <button type="button" onClick={() => void saveProvider()} className="rounded-lg bg-zinc-100 px-3 py-1.5 text-xs font-medium text-zinc-900 hover:bg-white"><Save className="mr-1.5 inline h-3.5 w-3.5" />保存供应商</button>
            </div>
          )}
        </section>
      </div>

      {modelModalOpen && <ModelModal form={modelForm} setForm={setModelForm} onClose={closeModelModal} onSave={() => void saveModel()} />}
    </div>
  );
}
