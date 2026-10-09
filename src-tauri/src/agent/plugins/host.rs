//! 宿主执行环境：业务处理器通过它访问 Tauri 应用句柄、取消状态和本机服务。
//!
//! Agent Core 不直接依赖这些字段；插件构造成 `CapabilityHandler` 时只接收 `PluginEnv`。

use std::sync::Arc;

use tauri::AppHandle;

pub use super::registry::{CapabilityHandler, HandlerContext, PluginToolCall};
use crate::agent::tools::ToolExecutionResult;

/// 与一次运行无关的宿主依赖；应用启动时构建一次，长期复用。
#[derive(Clone)]
pub struct PluginEnv {
    /// 生产环境由 Tauri 注入；纯逻辑测试可以为空。
    pub app: Option<AppHandle>,
    pub control: Arc<dyn RunControl>,
    /// Windows UI Automation 短期目标注册表。
    pub targets: Arc<std::sync::Mutex<crate::computer::Targets>>,
}

impl Default for PluginEnv {
    fn default() -> Self {
        Self {
            app: None,
            control: Arc::new(NeverControl),
            targets: Arc::new(std::sync::Mutex::new(crate::computer::Targets::default())),
        }
    }
}

impl PluginEnv {
    pub fn require_app(&self) -> Result<AppHandle, ToolExecutionResult> {
        self.app.clone().ok_or_else(|| {
            ToolExecutionResult::failure(
                "HOST_UNAVAILABLE",
                "此执行环境不支持需要窗口交互的本机工具",
                false,
            )
        })
    }
}

/// 一次运行的标识与取消查询；由 Agent 循环或手动调用构造。
#[derive(Clone)]
pub struct RunScope {
    pub session_id: String,
    pub run_id: String,
    pub cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl RunScope {
    pub fn new(
        session_id: impl Into<String>,
        run_id: impl Into<String>,
        cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            run_id: run_id.into(),
            cancelled,
        }
    }

    /// 无会话关联的本地调用（手动工具按钮、诊断）。
    pub fn manual(env: &PluginEnv) -> Self {
        Self {
            session_id: String::new(),
            run_id: "manual".into(),
            cancelled: {
                let control = Arc::clone(&env.control);
                Arc::new(move || control.cancelled("manual"))
            },
        }
    }
}

/// 运行取消查询；生产环境绑定 AgentState。
pub trait RunControl: Send + Sync {
    fn cancelled(&self, run_id: &str) -> bool;
}

impl PluginEnv {
    /// 某一运行 id 的取消查询闭包。
    pub fn cancellation(&self, run_id: &str) -> Arc<dyn Fn() -> bool + Send + Sync> {
        let control = Arc::clone(&self.control);
        let run_id = run_id.to_owned();
        Arc::new(move || control.cancelled(&run_id))
    }
}

/// 从处理器上下文取得 AppHandle，缺失时返回结构化错误。
pub fn app(ctx: &HandlerContext) -> Result<AppHandle, ToolExecutionResult> {
    ctx.env.require_app()
}

pub fn cancelled_result() -> ToolExecutionResult {
    crate::agent::native_dialog::cancelled_result()
}

/// 已实现业务逻辑的能力处理器集合，按 capability ID 索引。
pub type HandlerMap = std::collections::BTreeMap<String, Arc<dyn CapabilityHandler>>;

/// 把普通异步函数适配成 capability handler，避免手写 trait 实现样板。
pub fn bind<F, Fut>(handlers: &mut HandlerMap, capability_id: &str, call: F)
where
    F: Fn(PluginToolCall, HandlerContext) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ToolExecutionResult> + Send + 'static,
{
    struct Adapter<F> {
        call: F,
    }
    #[async_trait::async_trait]
    impl<F, Fut> CapabilityHandler for Adapter<F>
    where
        F: Fn(PluginToolCall, HandlerContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ToolExecutionResult> + Send + 'static,
    {
        async fn call(&self, call: &PluginToolCall, ctx: &HandlerContext) -> ToolExecutionResult {
            (self.call)(call.clone(), ctx.clone()).await
        }
    }
    handlers.insert(capability_id.into(), Arc::new(Adapter { call }));
}

/// 测试与本地探针使用：永不取消。
pub struct NeverControl;
impl RunControl for NeverControl {
    fn cancelled(&self, _run_id: &str) -> bool {
        false
    }
}
