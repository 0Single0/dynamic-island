import { useEffect, useState } from "react";
import { coverDataUrl } from "@/api/netease";

/**
 * 封面组件：网易云页面的封面地址是 orpheus://cache/?<https-url>，
 * 岛窗口加载不了该协议，交给 Rust 下载后转 base64 数据 URL 展示。
 * 无 Tauri 环境（纯浏览器调试）退回原始地址。
 */

const cache = new Map<string, string>();

async function resolveCover(url: string): Promise<string | null> {
  const hit = cache.get(url);
  if (hit) return hit;
  try {
    const dataUrl = await coverDataUrl(url);
    cache.set(url, dataUrl);
    return dataUrl;
  } catch {
    return null;
  }
}

export function CoverImage({
  url,
  alt = "",
  className,
}: {
  url?: string | null;
  alt?: string;
  className?: string;
}) {
  const [src, setSrc] = useState<string | null>(() =>
    url && !url.startsWith("orpheus://") ? url : null,
  );
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
    if (!url) {
      setSrc(null);
      return;
    }
    if (!url.startsWith("orpheus://")) {
      setSrc(url);
      return;
    }
    let alive = true;
    void resolveCover(url).then((dataUrl) => {
      if (!alive) return;
      if (dataUrl) setSrc(dataUrl);
      else setFailed(true);
    });
    return () => {
      alive = false;
    };
  }, [url]);

  if (!src || failed) return null;
  return <img src={src} alt={alt} className={className} draggable={false} />;
}
