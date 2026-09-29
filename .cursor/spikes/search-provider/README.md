# 网络搜索 Provider Spike

日期：2026-09-29  
关联计划：`PLAN-00-06`  
实现：`src-tauri/src/agent/search.rs`

## 结论

定义 `WebSearchProvider` 抽象和统一 `SearchResult` 结构。无论后续接入哪一个真实搜索供应商，结果都必须保留标题、URL、发布时间（可空）、摘要和来源标识；前端据此渲染可追溯来源卡片，不能把搜索摘要当作模型自身知识。

当前实现为完全离线的 `MockSearchProvider`，用于 Agent Loop 和 UI 联调，不包含凭据、不发网络请求。真实 Provider 由后续网络工具任务接入，并复用 SSRF URL 规则、超时、调用预算和脱敏日志。
