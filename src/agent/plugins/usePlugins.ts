import { useEffect, useState } from "react";
import { listBuiltinPlugins } from "@/agent/api";
import type { PluginStatus } from "./types";

/** 读取运行时已挂载的插件目录；只读，不提供任何启停语义。 */
export function useBuiltinPlugins() {
  const [plugins, setPlugins] = useState<PluginStatus[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    listBuiltinPlugins()
      .then((next) => {
        if (!active) return;
        setPlugins(Array.isArray(next) ? next : []);
        setError("");
      })
      .catch((cause) => {
        if (!active) return;
        setError(cause instanceof Error ? cause.message : "读取插件目录失败");
      })
      .finally(() => {
        if (active) setIsLoading(false);
      });
    return () => {
      active = false;
    };
  }, []);
  return { plugins, isLoading, error };
}
