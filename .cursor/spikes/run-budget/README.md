# Run 预算、费用、重试、备用模型和并发锁 Spike

日期：2026-09-29  
关联计划：`PLAN-00-08`

可执行契约已落在 `agent::limits::BudgetGuard`：步数、总 token、工具/搜索/Computer Use 次数、运行时长和可选估算费用都由后端检查。重试按错误类别分类，认证、参数错误和取消不重试；限流/超时/服务端暂时错误有限退避；上下文超限先压缩；备用模型切换必须在后续 Agent Loop 中重新验证能力和剩余预算。

并发锁的下一步实现应以 `(session_id, resource_type, resource_id)` 为键；同一会话只允许一个 active Run，文件、窗口和浏览器 Target 按资源互斥，只读且无副作用的工具才可声明并行。
