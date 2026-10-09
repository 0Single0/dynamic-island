//! 内置插件自动注册：宿主统一挂载，Agent 按请求解析工具。
//!
//! 业务能力只在各自模块实现一次；插件模块只声明清单与能力绑定。

pub mod desktop_plugin;
pub mod files;
pub mod files_plugin;
pub mod host;
pub mod manifest;
pub mod markdown;
pub mod markdown_plugin;
pub mod netease_plugin;
pub mod registry;
pub mod runtime;
pub mod system;
pub mod web_search;
mod web_search_plugin;

use std::sync::Arc;

use tauri::State;

use crate::netease::SharedNetease;

pub use manifest::PluginStatus;
pub use runtime::{PluginError, PluginRuntime};

/// 启动时挂载全部内置插件；失败意味着实现缺陷而不是用户配置问题。
pub async fn mount_builtin_plugins(
    runtime: &PluginRuntime,
    netease: Option<SharedNetease>,
) -> Result<Vec<String>, PluginError> {
    let mut mounted = Vec::new();
    for (manifest, handlers) in [
        (system::manifest(), system::handlers()),
        (files_plugin::manifest(), files_plugin::handlers()),
        (web_search_plugin::manifest(), web_search_plugin::handlers()),
        (markdown_plugin::manifest(), markdown_plugin::handlers()),
        (desktop_plugin::manifest(), desktop_plugin::handlers()),
        (
            netease_plugin::manifest(),
            netease_plugin::handlers(netease.clone()),
        ),
    ] {
        let id = manifest.id.clone();
        // 宿主依赖（AppHandle、取消查询、UIA 目标）在调用时经 HandlerContext 注入。
        runtime.mount(manifest, handlers).await?;
        mounted.push(id);
    }
    Ok(mounted)
}

/// 只读插件目录：设置中心展示的“已加载能力”，不提供启停控件。
#[tauri::command]
pub async fn list_builtin_plugins(
    runtime: State<'_, Arc<PluginRuntime>>,
) -> Result<Vec<PluginStatus>, String> {
    Ok(runtime.statuses().await)
}

/// 单个插件的工具清单，供前端展示当前请求会携带的范围。
#[tauri::command]
pub async fn list_plugin_tools(
    runtime: State<'_, Arc<PluginRuntime>>,
    plugin_id: Option<String>,
) -> Result<Vec<super::tools::ToolDefinition>, String> {
    Ok(match plugin_id {
        Some(id) => runtime.tools_for(&[id]).await,
        None => runtime.all_tools().await,
    })
}
