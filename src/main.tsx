import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "@/index.css";

/**
 * 面板入口。
 *
 * 用 BrowserRouter 而不是桌面端的 HashRouter：这是部署在服务器上的普通网页，
 * 路径就该是 `/`、`/accounts` 这种正常 URL（服务端已对未知路径回落 index.html）。
 */
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
