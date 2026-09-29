# Agent 预算与安全边界 Spike

日期：2026-09-29  
关联计划：`PLAN-00-08`、`PLAN-00-09`  
实现：`src-tauri/src/agent/limits.rs`、`src-tauri/src/agent/security.rs`

## 已落地的最小契约

- 每个 Run 以 `BudgetGuard` 记录步骤、总 token、工具调用、搜索调用、Computer Use 动作、估算费用和起始时间。
- 预算检查在进入下一轮、记录模型用量和记录工具调用后执行；任何计数超限都返回结构化终止原因。
- 重试分类明确区分：认证/取消/参数错误不重试；超时、限流和服务端暂时错误有限退避重试；上下文超限先压缩再重试；其他可恢复 Provider 错误才允许备用模型。
- 外部网页、文件和剪贴板内容包装为 `UntrustedContent`，只作为数据传递，不能改变系统策略或权限。
- 远程供应商和搜索 URL 默认只允许 HTTPS；HTTP 只允许 localhost；明确拒绝 URL 用户名密码以及直接的环回、私有、链路本地 IP。

## 仍需后续实现

- DNS 解析后的地址校验和重绑定防护应放进真实 HTTP client；当前函数只验证 URL 字面量。
- 真实 Agent Loop 需要把 `BudgetGuard` 与取消 token、运行事件和持久化 Run 记录绑定。
- SSRF（Server-Side Request Forgery，服务器端请求伪造）防护还要覆盖重定向、下载、浏览器导航和上传目标，不能只依赖单次 URL 校验。

## 验收

`cargo test --lib` 覆盖预算、重试分类、HTTPS/localhost 策略和外部内容标记；测试完全离线。
