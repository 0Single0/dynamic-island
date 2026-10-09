import { useEditor, EditorContent } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Mention from "@tiptap/extension-mention";
import type { SuggestionKeyDownProps, SuggestionOptions, SuggestionProps } from "@tiptap/suggestion";
import { PluginKey } from "@tiptap/pm/state";
import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import type { Editor, JSONContent } from "@tiptap/core";
import type { ReactNode } from "react";
import { Plus, Send } from "lucide-react";
import { cn } from "@/lib/utils";
import { AgentIcon } from "./AgentIcons";

export interface AgentComposerItem {
  id: string;
  label: string;
  description: string;
  icon?: ReactNode;
  insertText?: string;
  group?: string;
  pickerKind?: "attachment" | "skill" | "plugin";
}

export interface AgentComposerProps {
  onSubmit: (document: JSONContent) => void;
  disabled?: boolean;
  document?: JSONContent;
  onDocumentChange?: (document: JSONContent) => void;
  attachments?: AgentComposerItem[];
  skills?: AgentComposerItem[];
  plugins?: AgentComposerItem[];
  placeholder?: string;
  footer?: ReactNode;
}

interface SuggestionListProps {
  items: AgentComposerItem[];
  command: (item: AgentComposerItem) => void;
  selectedIndex: number;
  onSelectedIndexChange: (index: number) => void;
}

interface PickerState {
  items: AgentComposerItem[];
  command: (item: AgentComposerItem) => void;
}

const defaultAttachments: AgentComposerItem[] = [
  { id: "attach-file", label: "文件和文件夹", description: "添加本地文件或文件夹", icon: <AgentIcon id="attach-file" />, group: "添加", pickerKind: "attachment" },
  { id: "attach-image", label: "图片和附件", description: "引用图片、文档或其他附件", icon: <AgentIcon id="attach-image" />, group: "添加", pickerKind: "attachment" },
];

const defaultSkills: AgentComposerItem[] = [
  { id: "search", label: "搜索网络", description: "搜索网页并保留来源", icon: <AgentIcon id="search" />, insertText: "/web.search " },
  { id: "summarize", label: "压缩上下文", description: "保留任务状态并减少上下文长度", icon: <AgentIcon id="summarize" />, insertText: "/context.compact " },
];

const defaultPlugins: AgentComposerItem[] = [
  { id: "files", label: "文件工具", description: "读取、编辑和生成文件", icon: <AgentIcon id="files" />, group: "插件", pickerKind: "plugin" },
  { id: "web-search", label: "网页搜索", description: "搜索并保留来源卡片", icon: <AgentIcon id="web-search" />, group: "插件", pickerKind: "plugin" },
  { id: "desktop", label: "电脑", description: "查看并操作受控的桌面窗口", icon: <AgentIcon id="desktop" />, group: "插件", pickerKind: "plugin" },
  { id: "netease", label: "网易云音乐", description: "读取当前歌曲并控制播放", icon: <AgentIcon id="netease" />, group: "插件", pickerKind: "plugin" },
  { id: "markdown", label: "Markdown", description: "将 Markdown 转换为 PDF", icon: <AgentIcon id="markdown" />, group: "插件", pickerKind: "plugin" },
];

function SuggestionList({ items, command, selectedIndex, onSelectedIndexChange }: SuggestionListProps) {
  return (
    <div className="rounded-xl border border-white/[0.13] bg-[#18191c]/98 p-1.5 shadow-2xl shadow-black/40 backdrop-blur-2xl" role="listbox">
      {items.length === 0 ? (
        <div className="px-3 py-3.5 text-[11px] text-white/45">没有匹配的能力</div>
      ) : (
        items.map((item, index) => (
          <Fragment key={item.id}>
            {(index === 0 || item.group !== items[index - 1]?.group) && item.group && (
              <div className={cn("px-2.5 pb-1 pt-2 text-[11px] leading-tight text-white/45", index > 0 && "pt-3")}>{item.group}</div>
            )}
            <button
              className={cn(
                "flex w-full items-center gap-2.5 rounded-lg border-0 bg-transparent px-[5px] py-1 text-left text-inherit transition-colors hover:bg-white/[0.09]",
                index === selectedIndex && "bg-white/[0.09]",
              )}
              onMouseDown={(event) => {
                event.preventDefault();
                command(item);
              }}
              onMouseEnter={() => onSelectedIndexChange(index)}
              role="option"
              type="button"
              aria-selected={index === selectedIndex}
            >
              <span className="grid h-[22px] w-[22px] shrink-0 place-items-center rounded-lg text-[14px] text-white/90">{item.icon ?? "•"}</span>
              <span className="flex min-w-0 items-baseline gap-[9px]">
                <span className="shrink-0 whitespace-nowrap text-xs">{item.label}</span>
                <span className="min-w-0 overflow-hidden text-ellipsis whitespace-nowrap text-[11px] text-white/47">{item.description}</span>
              </span>
            </button>
          </Fragment>
        ))
      )}
    </div>
  );
}

function filterItems(items: AgentComposerItem[], query: string) {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return items.slice(0, 6);
  return items
    .filter((item) => `${item.label} ${item.description}`.toLocaleLowerCase().includes(normalized))
    .slice(0, 6);
}

function createSuggestionRenderer(
  onChange: (state: PickerState | null) => void,
  onKeyDown: (props: SuggestionKeyDownProps) => boolean,
): NonNullable<SuggestionOptions<AgentComposerItem, AgentComposerItem>["render"]> {
  return () => ({
    onStart: (props: SuggestionProps<AgentComposerItem>) => {
      onChange({ items: props.items, command: (item) => props.command(item) });
    },
    onUpdate: (props: SuggestionProps<AgentComposerItem>) => {
      onChange({ items: props.items, command: (item) => props.command(item) });
    },
    onKeyDown,
    onExit: () => onChange(null),
  });
}

function createDirectoryMention(options: {
  char: string;
  pluginKey: string;
  items: AgentComposerItem[];
  insertAsMention: boolean;
  onChange: (state: PickerState | null) => void;
  onKeyDown: (props: SuggestionKeyDownProps) => boolean;
}) {
  const suggestion: Omit<SuggestionOptions<AgentComposerItem, AgentComposerItem>, "editor"> = {
    char: options.char,
    pluginKey: new PluginKey(options.pluginKey),
    items: ({ query }: { query: string }) => filterItems(options.items, query),
    command: ({ editor, range, props }: Parameters<NonNullable<SuggestionOptions<AgentComposerItem, AgentComposerItem>["command"]>>[0]) => {
      const content = options.insertAsMention && props.pickerKind !== "skill"
        ? { type: "mention", attrs: { id: props.id, label: props.label } }
        : props.insertText ?? `${options.char}${props.label} `;
      editor.chain().focus().deleteRange(range).insertContent(content).run();
    },
    render: createSuggestionRenderer(options.onChange, options.onKeyDown),
  };

  return suggestion;
}

export function Composer({
  onSubmit,
  disabled = false,
  document: draftDocument,
  onDocumentChange,
  attachments = defaultAttachments,
  skills = defaultSkills,
  plugins = defaultPlugins,
  placeholder = "描述你想完成的事情，输入 / 选择技能，输入 @ 选择插件",
  footer,
}: AgentComposerProps) {
  const [isEmpty, setIsEmpty] = useState(true);
  const [pickerMode, setPickerMode] = useState<"add" | "at" | "slash" | null>(null);
  const [pickerState, setPickerState] = useState<PickerState | null>(null);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const composerRef = useRef<HTMLDivElement | null>(null);
  const pickerRef = useRef<PickerState | null>(null);
  const selectedIndexRef = useRef(0);
  const submissionRef = useRef({ disabled, onSubmit, onDocumentChange });
  submissionRef.current = { disabled, onSubmit, onDocumentChange };
  const pickerItems = useMemo(() => [...attachments, ...plugins], [attachments, plugins]);
  const slashSkills = useMemo(() => skills.map((item) => ({ ...item, group: "技能" })), [skills]);

  const setSelectedPickerIndex = (index: number) => {
    selectedIndexRef.current = index;
    setSelectedIndex(index);
  };

  const handlePickerKeyDown = ({ event }: SuggestionKeyDownProps) => {
    const items = pickerRef.current?.items ?? [];
    if (event.key === "ArrowUp") {
      event.preventDefault();
      setSelectedPickerIndex((selectedIndexRef.current + items.length - 1) % Math.max(items.length, 1));
      return true;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setSelectedPickerIndex((selectedIndexRef.current + 1) % Math.max(items.length, 1));
      return true;
    }
    if ((event.key === "Enter" || event.key === "Tab") && items[selectedIndexRef.current]) {
      event.preventDefault();
      pickerRef.current?.command(items[selectedIndexRef.current]);
      return true;
    }
    if (event.key === "Escape") {
      setPickerMode(null);
      setPickerState(null);
      pickerRef.current = null;
      return true;
    }
    return false;
  };

  const updateSuggestionPicker = (mode: "at" | "slash", state: PickerState | null) => {
    pickerRef.current = state;
    setPickerMode(state ? mode : null);
    setPickerState(state);
    setSelectedPickerIndex(0);
  };

  const actionsExtension = useMemo(
    () => createDirectoryMention({
      char: "/",
      pluginKey: "agent-slash",
      items: slashSkills,
      insertAsMention: false,
      onChange: (state) => updateSuggestionPicker("slash", state),
      onKeyDown: handlePickerKeyDown,
    }),
    [slashSkills],
  );
  const pluginsExtension = useMemo(
    () => createDirectoryMention({
      char: "@",
      pluginKey: "agent-plugin",
      items: pickerItems,
      insertAsMention: true,
      onChange: (state) => updateSuggestionPicker("at", state),
      onKeyDown: handlePickerKeyDown,
    }),
    [pickerItems],
  );
  const mentionExtension = useMemo(
    () => Mention.configure({
      HTMLAttributes: { class: "inline-flex select-all whitespace-nowrap rounded-[7px] border border-blue-200/22 bg-blue-500/[0.16] px-[7px] py-px text-[#b8d6ff]" },
      deleteTriggerWithBackspace: true,
      suggestions: [actionsExtension, pluginsExtension] as never,
    }),
    [actionsExtension, pluginsExtension],
  );
  const editor = useEditor({
    extensions: [StarterKit, mentionExtension],
    content: draftDocument || "",
    editorProps: {
      attributes: {
        class: "min-h-[38px] max-h-24 cursor-text overflow-y-auto whitespace-pre-wrap break-words text-[13px] leading-[1.55] outline-none select-text [scrollbar-color:rgba(255,255,255,.2)_transparent] [scrollbar-width:thin] [&_p]:m-0 [&_p.is-editor-empty:first-child::before]:pointer-events-none [&_p.is-editor-empty:first-child::before]:float-left [&_p.is-editor-empty:first-child::before]:h-0 [&_p.is-editor-empty:first-child::before]:text-white/[0.38] [&_p.is-editor-empty:first-child::before]:content-[attr(data-placeholder)]",
        "data-placeholder": placeholder,
      },
      handleKeyDown: (_view, event) => {
        if (event.isComposing || event.keyCode === 229) return false;
        if (event.key === "Enter" && !event.shiftKey) {
          event.preventDefault();
          if (submissionRef.current.disabled) return true;
          const document = editor?.getJSON();
          if (editor?.getText().trim() && document) submissionRef.current.onSubmit(document);
          editor?.commands.clearContent();
          return true;
        }
        return false;
      },
    },
    onUpdate: ({ editor: nextEditor }) => {
      setIsEmpty(nextEditor.isEmpty);
      submissionRef.current.onDocumentChange?.(nextEditor.getJSON());
    },
  });

  useEffect(() => {
    if (!editor || !draftDocument) return;
    if (JSON.stringify(editor.getJSON()) !== JSON.stringify(draftDocument)) editor.commands.setContent(draftDocument, { emitUpdate: false });
    setIsEmpty(editor.isEmpty);
  }, [editor, draftDocument]);

  useEffect(() => {
    if (pickerMode !== "add") return;
    const close = (event: MouseEvent) => {
      if (!composerRef.current?.contains(event.target as Node)) {
        setPickerMode(null);
        setPickerState(null);
        pickerRef.current = null;
      }
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [pickerMode]);

  const insertPlugin = (item: AgentComposerItem) => {
    if (!editor) return;
    editor.chain().focus().insertContent({
      type: "mention",
      attrs: { id: item.id, label: item.label },
    }).insertContent(" ").run();
    setPickerMode(null);
    setPickerState(null);
    pickerRef.current = null;
  };

  const openAddPicker = () => {
    const nextOpen = pickerMode !== "add";
    const nextState = nextOpen ? { items: pickerItems, command: insertPlugin } : null;
    pickerRef.current = nextState;
    setPickerMode(nextOpen ? "add" : null);
    setPickerState(nextState);
    setSelectedPickerIndex(0);
  };

  return (
    <div
      ref={composerRef}
      data-agent-composer
      className="relative box-border min-h-[92px] w-full rounded-2xl border border-white/10 bg-white/[0.055] px-[14px] pb-[42px] pt-[13px] text-white/[0.92] transition focus-within:border-white/[0.22] focus-within:shadow-[0_0_0_3px_rgba(255,255,255,.035)]"
    >
      {pickerMode && (
        <div
          className="absolute inset-x-0 bottom-[calc(100%+8px)] z-20 max-h-[280px] w-full overflow-auto rounded-xl [scrollbar-color:rgba(255,255,255,.22)_transparent] [scrollbar-width:thin]"
          role="listbox"
          aria-label="选择插件"
        >
          <SuggestionList
            items={pickerState?.items ?? []}
            command={pickerState?.command ?? insertPlugin}
            selectedIndex={selectedIndex}
            onSelectedIndexChange={setSelectedPickerIndex}
          />
        </div>
      )}
      <EditorContent editor={editor} />
      <button
        type="button"
        className={cn(
          "absolute bottom-[9px] left-[11px] grid h-6 w-6 place-items-center rounded-full border-0 bg-white/[0.08] text-white/[0.85] transition hover:bg-white/[0.16]",
          pickerMode === "add" && "rotate-45 bg-white/[0.16]",
        )}
        aria-label="选择插件"
        aria-expanded={pickerMode === "add"}
        onMouseDown={(event) => event.preventDefault()}
        onClick={openAddPicker}
      >
        <Plus className="h-4 w-4" />
      </button>
      {footer && <div className="absolute bottom-2.5 right-12 flex min-w-0 max-w-[calc(100%-92px)] items-center justify-end">{footer}</div>}
      <button
        type="button"
        className="absolute bottom-2 right-2 grid h-7 w-7 place-items-center rounded-lg border-0 bg-sky-300/[0.16] text-[#d7f0ff] transition hover:bg-sky-300/[0.28] disabled:cursor-default disabled:opacity-35"
        aria-label="发送消息"
        disabled={isEmpty || disabled}
        onMouseDown={(event) => event.preventDefault()}
        onClick={() => {
          const document = editor?.getJSON();
          if (!submissionRef.current.disabled && !editor?.view.composing && editor?.getText().trim() && document) {
            submissionRef.current.onSubmit(document);
            editor.commands.clearContent();
          }
        }}
      >
        <Send className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}

export function getComposerText(editor: Editor | null) {
  return editor?.getText().trim() ?? "";
}

export function getComposerDocument(editor: Editor | null): JSONContent | null {
  return editor?.getJSON() ?? null;
}
