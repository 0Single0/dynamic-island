import type { ReactNode } from "react";
import { FileText, FolderOpen, Image as ImageIcon, Minimize2, Monitor, Search } from "lucide-react";
import { MarkdownIcon } from "@/widgets/MarkdownIcon";

export type AgentIconId =
  | "system"
  | "files"
  | "web-search"
  | "desktop"
  | "netease"
  | "markdown"
  | "attach-file"
  | "attach-image"
  | "search"
  | "summarize";

interface AgentIconProps {
  id: AgentIconId;
  className?: string;
}

const knownIconIds: string[] = [
  "system",
  "files",
  "web-search",
  "desktop",
  "netease",
  "markdown",
  "attach-file",
  "attach-image",
  "search",
  "summarize",
];

/** 插件清单里的 icon 是字符串，这里收敛回受支持的图标集合，未知值退回默认图标。 */
export function toAgentIconId(id: string): AgentIconId {
  return knownIconIds.includes(id) ? (id as AgentIconId) : "system";
}

/**
 * Shared icon mapping for the Agent picker and settings.
 * Keep this as a component mapping instead of storing rendered nodes in plugin data,
 * so the registry remains serializable and can still be used by the Rust side later.
 */
export function AgentIcon({ id, className = "h-4 w-4" }: AgentIconProps) {
  const iconClassName = `shrink-0 ${className}`;

  switch (id) {
    case "system":
      return <Monitor className={iconClassName} aria-hidden="true" />;
    case "files":
    case "attach-file":
      return <FolderOpen className={iconClassName} aria-hidden="true" />;
    case "web-search":
    case "search":
      return <Search className={iconClassName} aria-hidden="true" />;
    case "desktop":
      return <Monitor className={iconClassName} aria-hidden="true" />;
    case "netease":
      return <NeteaseMusicIcon className={className} />;
    case "markdown":
      return <MarkdownIcon className={iconClassName} />;
    case "attach-image":
      return <ImageIcon className={iconClassName} aria-hidden="true" />;
    case "summarize":
      return <Minimize2 className={iconClassName} aria-hidden="true" />;
    default:
      return <FileText className={iconClassName} aria-hidden="true" />;
  }
}

/** Local NetEase Cloud Music brand mark based on the published SVG path. */
export function NeteaseMusicIcon({ className = "h-4 w-4" }: { className?: string }): ReactNode {
  return (
    <span className={`grid shrink-0 place-items-center rounded-[5px] bg-[#e94b63] text-white ${className}`} aria-hidden="true">
      <svg viewBox="0 0 24 24" className="h-[78%] w-[78%]" fill="currentColor">
        <path d="M13.046 9.388a3.919 3.919 0 0 0-.66.19c-.809.312-1.447.991-1.666 1.775a2.269 2.269 0 0 0-.074.81c.048.546.333 1.05.764 1.35a1.483 1.483 0 0 0 2.01-.286c.406-.531.355-1.183.24-1.636-.098-.387-.22-.816-.345-1.249a64.76 64.76 0 0 1-.269-.954zm-.82 10.07c-3.984 0-7.224-3.24-7.224-7.223 0-.98.226-3.02 1.884-4.822A7.188 7.188 0 0 1 9.502 5.6a.792.792 0 1 1 .587 1.472 5.619 5.619 0 0 0-2.79 2.462 5.538 5.538 0 0 0-.707 2.7 5.645 5.645 0 0 0 5.638 5.638c1.844 0 3.627-.953 4.542-2.428 1.042-1.68.772-3.931-.627-5.238a3.299 3.299 0 0 0-1.437-.777c.172.589.334 1.18.494 1.772.284 1.12.1 2.181-.519 2.989-.39.51-.956.888-1.592 1.064a3.038 3.038 0 0 1-2.58-.44 3.45 3.45 0 0 1-1.44-2.514c-.04-.467.002-.93.128-1.376.35-1.256 1.356-2.339 2.622-2.826a5.5 5.5 0 0 1 .823-.246l-.134-.505c-.37-1.371.25-2.579 1.547-3.007.329-.109.68-.145 1.025-.105.792.09 1.476.592 1.709 1.023.258.507-.096 1.153-.706 1.153a.788.788 0 0 1-.54-.213c-.088-.08-.163-.174-.259-.247a.825.825 0 0 0-.632-.166.807.807 0 0 0-.634.551c-.056.191-.031.406.02.595.07.256.159.597.217.82 1.11.098 2.162.54 2.97 1.296 1.974 1.844 2.35 4.886.892 7.233-1.197 1.93-3.509 3.177-5.889 3.177zM0 12c0 6.627 5.373 12 12 12s12-5.373 12-12S18.627 0 12 0 0 5.373 0 12Z" />
      </svg>
    </span>
  );
}
