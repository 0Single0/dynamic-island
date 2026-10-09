import { useEffect, useState } from "react";
import { Save } from "lucide-react";
import { tauriInvoke } from "@/agent/api";
import { Select } from "@/components/common/Select";

interface SearchConfig {
  provider: "unconfigured" | "bing-public" | "searxng";
  endpoint: string;
}
const options = [
  { value: "unconfigured", label: "未配置" },
  { value: "bing-public", label: "Bing 公开搜索" },
  { value: "searxng", label: "SearXNG" },
];

export function SearchSettings() {
  const [config, setConfig] = useState<SearchConfig>({ provider: "unconfigured", endpoint: "" });
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(false);
  useEffect(() => {
    void tauriInvoke<SearchConfig>("agent_get_search_config")
      .then((value) => { setConfig(value); setLoaded(true); })
      .catch((error) => setNotice(String(error)));
  }, []);
  const save = async () => {
    setBusy(true);
    setNotice("");
    try {
      setConfig(await tauriInvoke<SearchConfig>("agent_save_search_config", { input: config }));
      setNotice("搜索配置已保存");
    } catch (error) { setNotice(String(error)); }
    finally { setBusy(false); }
  };
  return <section className="max-w-xl space-y-5">
    <h2 className="text-sm font-medium text-zinc-200">网页搜索</h2>
    <label className="grid gap-2 text-xs text-zinc-400">
      <span>搜索后端</span>
      <Select value={config.provider} options={options} onChange={(value) => setConfig((current) => ({ ...current, provider: value as SearchConfig["provider"] }))} />
    </label>
    {config.provider === "searxng" && <label className="grid gap-2 text-xs text-zinc-400">
      <span>SearXNG 搜索地址</span>
      <input value={config.endpoint} onChange={(event) => setConfig((current) => ({ ...current, endpoint: event.target.value }))} placeholder="https://search.example.com/search" className="w-full rounded-lg border border-white/10 bg-white/[0.025] px-3 py-2 text-sm text-zinc-200 outline-none focus:border-sky-300/45" />
    </label>}
    {config.provider === "bing-public" && <p className="text-xs leading-5 text-zinc-500">无需密钥。搜索词将发送至 Bing；公开接口可能限流或不可用。</p>}
    {config.provider === "searxng" && <p className="text-xs leading-5 text-zinc-500">实例需开启 JSON 搜索输出。当前只允许公开 HTTPS 地址，不连接本机或内网实例。</p>}
    <div className="flex justify-end"><button type="button" disabled={!loaded || busy} onClick={() => void save()} className="inline-flex items-center gap-1.5 rounded-lg bg-zinc-100 px-3 py-2 text-xs font-medium text-zinc-900 hover:bg-white disabled:opacity-40"><Save className="h-3.5 w-3.5" />保存</button></div>
    {notice && <p role="status" className="text-xs text-zinc-400">{notice}</p>}
  </section>;
}
