import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "src"),
    },
  },

  build: {
    // 产物直接给 Rust 静态托管（容器里 COPY 到 /app/web）。
    outDir: "dist",
    emptyOutDir: true,
  },

  // 面板与后端同源：开发时把 /api 转给本地起的 gateway-server。
  server: {
    port: 5273,
    proxy: {
      "/api": {
        target: process.env.PANEL_DEV_API ?? "http://127.0.0.1:8791",
        changeOrigin: true,
      },
    },
  },
});
