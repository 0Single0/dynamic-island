import axios, { AxiosError, type AxiosInstance } from "axios";
import type { ApiResponse } from "@/types/api";
import { toast } from "@/utils/toast";

/**
 * 全局唯一 Axios 实例。页面代码禁止直接 import axios 或使用 fetch。
 * 统一处理 ApiResponse<T> 信封：code === 0 返回 data，否则抛出带 msg 的错误。
 */
const instance: AxiosInstance = axios.create({
  timeout: 15_000,
  headers: { "Content-Type": "application/json" },
});

instance.interceptors.response.use(
  (response) => {
    const body = response.data as ApiResponse<unknown>;
    // 非 JSON 或无信封结构的响应原样放行，由调用方按需处理
    if (body === null || typeof body !== "object" || !("code" in body)) {
      return response;
    }
    if (body.code === 0) {
      // 将 data 挂在响应上，request 泛型助手负责解包
      return response;
    }
    return Promise.reject(new Error(body.msg || "请求失败"));
  },
  (error: AxiosError) => {
    const message =
      (error.response?.data as { msg?: string } | undefined)?.msg ??
      (error.code === "ECONNABORTED"
        ? "请求超时"
        : error.message || "网络异常");
    return Promise.reject(new Error(message));
  },
);

/** 泛型请求助手：自动解包 code === 0 的 data 并统一弹 toast */
export async function request<T>(
  config: Parameters<AxiosInstance["request"]>[0],
  options: { silent?: boolean } = {},
): Promise<T> {
  try {
    const response = await instance.request<ApiResponse<T>>(config);
    const body = response.data;
    if (body && typeof body === "object" && "code" in body) {
      return body.data;
    }
    return body as unknown as T;
  } catch (error) {
    const message =
      error instanceof Error ? error.message : "未知错误";
    if (!options.silent) toast.error(message);
    throw error;
  }
}

export { instance as axiosInstance };
