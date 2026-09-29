import { useEffect, useRef, useState } from "react";
import { CheckCircle2, ExternalLink, Loader2, UploadCloud } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog, save } from "@tauri-apps/plugin-dialog";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { cn } from "@/lib/utils";
import { MarkdownIcon } from "./MarkdownIcon";

type Status = { kind: "idle" | "error" | "success"; message?: string };

marked.setOptions({ gfm: true, breaks: false });

function nameFromPath(path: string) {
  return path.split(/[\\/]/).pop() || path;
}

function isMarkdownName(name: string) {
  return /\.(md|markdown)$/i.test(name);
}

export function MarkdownPdfExpanded() {
  const inputRef = useRef<HTMLInputElement>(null);
  const [fileName, setFileName] = useState("");
  const [characterCount, setCharacterCount] = useState(0);
  const [pendingHtml, setPendingHtml] = useState("");
  const [pendingTitle, setPendingTitle] = useState("");
  const [dragging, setDragging] = useState(false);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const [outputPath, setOutputPath] = useState("");

  const renderMarkdown = async (text: string, markdownPath?: string) => {
    const parsed = String(await marked.parse(text));
    if (!markdownPath) {
      return DOMPurify.sanitize(parsed, {
        USE_PROFILES: { html: true },
        ADD_ATTR: ["target", "rel"],
      });
    }

    const document = new DOMParser().parseFromString(parsed, "text/html");
    const images = Array.from(document.querySelectorAll<HTMLImageElement>("img[src]"));
    await Promise.all(
      images.map(async (image) => {
        const source = image.getAttribute("src");
        if (!source || /^(data|https?|blob):/i.test(source)) return;
        let decodedSource = source;
        try {
          decodedSource = decodeURIComponent(source);
        } catch {
          // 保留原始路径，交给 Rust 侧继续尝试读取。
        }
        try {
          const dataUrl = await invoke<string>("read_markdown_image_data", {
            imagePath: decodedSource,
            markdownPath,
          });
          image.setAttribute("src", dataUrl);
        } catch {
          image.setAttribute("alt", `${image.alt || "图片"}（未找到）`);
        }
      }),
    );

    return DOMPurify.sanitize(document.body.innerHTML, {
      USE_PROFILES: { html: true },
      ADD_ATTR: ["target", "rel"],
    });
  };

  const generate = async (html = pendingHtml, title = pendingTitle) => {
    if (!html || !title || busy) return;
    const defaultName = title.replace(/\.(md|markdown)$/i, "") + ".pdf";
    let selectedPath: string | null;
    try {
      selectedPath = await save({
        title: "保存 PDF",
        defaultPath: defaultName,
        filters: [{ name: "PDF 文件", extensions: ["pdf"] }],
      });
    } catch {
      setStatus({ kind: "error", message: "无法打开保存对话框" });
      return;
    }
    if (!selectedPath) {
      setStatus({ kind: "idle", message: "已读取文件，点击保存按钮即可生成 PDF" });
      return;
    }

    setBusy(true);
    setStatus({ kind: "idle", message: "正在生成 PDF…" });
    setOutputPath("");
    try {
      await invoke("markdown_to_pdf", {
        html,
        outputPath: selectedPath,
        title: title.replace(/\.(md|markdown)$/i, ""),
      });
      setOutputPath(selectedPath);
      setStatus({ kind: "success", message: "PDF 已生成" });
    } catch (error) {
      setStatus({
        kind: "error",
        message: typeof error === "string" ? error : "PDF 生成失败，请稍后重试",
      });
    } finally {
      setBusy(false);
    }
  };

  const loadText = async (name: string, text: string, markdownPath?: string) => {
    if (!isMarkdownName(name)) {
      setStatus({ kind: "error", message: "请选择 .md 或 .markdown 文件" });
      return;
    }
    try {
      const html = await renderMarkdown(text, markdownPath);
      setFileName(name);
      setCharacterCount(text.length);
      setPendingHtml(html);
      setPendingTitle(name);
      setOutputPath("");
      setStatus({ kind: "idle", message: "文件已读取，正在打开保存对话框…" });
      await generate(html, name);
    } catch {
      setStatus({ kind: "error", message: "读取 Markdown 文件失败" });
    }
  };

  const loadBrowserFile = async (file?: File) => {
    if (!file) return;
    try {
      await loadText(file.name, await file.text());
    } catch {
      setStatus({ kind: "error", message: "读取 Markdown 文件失败" });
    }
  };

  const loadNativePath = async (path: string) => {
    const name = nameFromPath(path);
    if (!isMarkdownName(name)) {
      setStatus({ kind: "error", message: "请选择 .md 或 .markdown 文件" });
      return;
    }
    try {
      const text = await invoke<string>("read_markdown_file", { path });
      await loadText(name, text, path);
    } catch (error) {
      setStatus({
        kind: "error",
        message: typeof error === "string" ? error : "无法读取拖入的 Markdown 文件",
      });
    }
  };

  const chooseMarkdownFile = async () => {
    try {
      const selected = await openDialog({
        multiple: false,
        directory: false,
        filters: [{ name: "Markdown 文件", extensions: ["md", "markdown"] }],
      });
      if (typeof selected === "string") await loadNativePath(selected);
    } catch {
      // 浏览器开发环境或原生对话框不可用时，保留 input 作为后备入口。
      inputRef.current?.click();
    }
  };

  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];
    void listen<{ paths: string[] }>("tauri://drag-enter", () => setDragging(true)).then((fn) => {
      unlisteners.push(fn);
    });
    void listen<{ paths: string[] }>("tauri://drag-leave", () => setDragging(false)).then((fn) => {
      unlisteners.push(fn);
    });
    void listen<{ paths: string[] }>("tauri://drag-drop", (event) => {
      setDragging(false);
      const path = event.payload.paths?.[0];
      if (path) void loadNativePath(path);
    }).then((fn) => {
      unlisteners.push(fn);
    });
    return () => unlisteners.forEach((unlisten) => unlisten());
  }, []);

  return (
    <div className="flex h-full min-h-0 w-full flex-col gap-3">
      <div className="text-[11px] text-zinc-500">拖入或选择 Markdown 文件后直接转换并保存</div>

      <input
        ref={inputRef}
        type="file"
        accept=".md,.markdown,text/markdown,text/plain"
        className="hidden"
        onChange={(event) => void loadBrowserFile(event.target.files?.[0])}
      />

      <button
        type="button"
        onClick={() => void chooseMarkdownFile()}
        onDragOver={(event) => event.preventDefault()}
        className={cn(
          "flex min-h-40 flex-1 flex-col items-center justify-center rounded-2xl border border-dashed px-4 text-center transition-colors",
          dragging
            ? "border-violet-300/70 bg-violet-400/10"
            : "border-white/15 bg-white/[0.035] hover:border-white/30 hover:bg-white/[0.06]",
        )}
      >
        <UploadCloud className="h-9 w-9 text-violet-300" strokeWidth={1.6} />
        <span className="mt-3 text-sm font-medium text-zinc-200">拖拽 Markdown 到这里</span>
        <span className="mt-1 text-[11px] text-zinc-500">或点击选择 .md 文件</span>
      </button>

      <div className="flex min-h-11 items-center gap-2 rounded-xl bg-white/[0.045] px-3 py-2">
        <MarkdownIcon className={cn("h-6 w-6 shrink-0", fileName ? "text-violet-300" : "text-zinc-600")} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-xs text-zinc-200">{fileName || "尚未选择 Markdown 文件"}</div>
          {fileName && <div className="text-[10px] text-zinc-500">{characterCount.toLocaleString()} 个字符</div>}
        </div>
        {pendingHtml && !busy && !outputPath && (
          <button
            type="button"
            onClick={() => void generate()}
            className="shrink-0 rounded-lg bg-violet-400 px-2.5 py-1.5 text-[11px] font-semibold text-black hover:bg-violet-300"
          >
            保存 PDF
          </button>
        )}
        {busy && <Loader2 className="h-4 w-4 shrink-0 animate-spin text-violet-300" />}
      </div>

      {status.message && (
        <div className={cn("flex items-center gap-2 text-[11px]", status.kind === "success" ? "text-emerald-300" : status.kind === "error" ? "text-rose-300" : "text-zinc-400")}>
          {status.kind === "success" && <CheckCircle2 className="h-3.5 w-3.5" />}
          <span className="truncate">{status.message}</span>
          {status.kind === "success" && outputPath && (
            <>
              <button type="button" className="ml-auto inline-flex items-center gap-1 hover:text-emerald-200" onClick={() => void openPath(outputPath)}>
                打开 PDF <ExternalLink className="h-3 w-3" />
              </button>
              <button type="button" className="hover:text-emerald-200" onClick={() => void revealItemInDir(outputPath)}>
                所在目录
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}
