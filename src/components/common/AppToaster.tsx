import { Toaster } from "sonner";

/** 全局 toast 挂载点，深色玻璃风格。业务代码通过 utils/toast.ts 调用。 */
export function AppToaster() {
  return (
    <Toaster
      position="top-center"
      theme="dark"
      toastOptions={{
        style: {
          background: "rgba(0,0,0,0.6)",
          backdropFilter: "blur(12px) saturate(119%)",
          border: "1px solid rgba(255,255,255,0.15)",
          color: "#f4f4f5",
        },
      }}
    />
  );
}
