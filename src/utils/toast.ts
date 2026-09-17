import { toast as sonnerToast } from "sonner";

/**
 * toast 统一包装。业务代码只允许从这里调用，
 * 不直接 import "sonner"，便于后续替换实现或统一样式。
 */
export const toast = {
  success: (message: string) => sonnerToast.success(message),
  error: (message: string) => sonnerToast.error(message),
  info: (message: string) => sonnerToast.info(message),
  loading: (message: string) => sonnerToast.loading(message),
};
