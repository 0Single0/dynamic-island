# 浏览器 / CDP 会话 Spike

日期：2026-09-29  
关联计划：`PLAN-00-07`

CDP（Chrome DevTools Protocol，Chromium 开发者工具协议）是 Chromium/CEF 应用暴露的调试控制协议，不是 DLL。当前项目已有网易云 CDP WebSocket 实现；本 Spike 规定它扩展为通用浏览器连接时必须遵循的边界：

- 只从本机调试端点发现 Target，并校验 `type`、协议 URL、允许的应用标识和会话发起者，不能把任意远程 WebSocket 地址交给模型。
- 默认创建临时 Profile；不读取浏览器密码、Cookie 或现有用户 Profile。复用登录态必须单独设置并逐次确认。
- 每个会话记录 Target ID、创建时间、临时目录和 owning Run；结束、取消或崩溃恢复时销毁/清理。
- 页面内容一律作为 `UntrustedContent`；导航、下载、上传、登录、提交、购买等动作经过 URL 策略、文件策略和审批。
- 下载/上传必须由 Rust 负责路径检查和大小限制；不能让网页脚本直接访问任意本地路径。

当前项目已有的 `netease::cdp` 可作为传输层复用；通用 Target 校验和临时 Profile 生命周期放在后续浏览器插件任务实现。由于当前机器未启动可安全接管的临时 Chromium Profile，本 Spike 不执行真实导航或上传。
