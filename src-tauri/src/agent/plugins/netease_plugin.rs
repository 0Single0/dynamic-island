//! 网易云插件：适配层，把 CDP 业务模块暴露为稳定能力。
//!
//! CDP 连接、页面检测、播放状态读取、播放控制、进度跳转、歌词同步、断线重连
//! 全部保留在 `crate::netease`；此处不复制任何实现。

use serde_json::json;

use super::host::{bind, HandlerMap};
use super::manifest::{
    CapabilityBinding, CapabilitySpec, PluginManifest, ToolExposure, INTERACTIVE_TIMEOUT_MS,
};
use crate::agent::permissions::RiskLevel;
use crate::agent::tools::ToolExecutionResult;
use crate::netease::SharedNetease;

pub const CAPABILITY_STATE: &str = "music.player.current_state";
pub const CAPABILITY_CONTROL: &str = "music.player.control";
pub const CAPABILITY_SEEK: &str = "music.player.seek";
pub const CAPABILITY_CONNECT: &str = "music.player.start_or_connect";

pub fn manifest() -> PluginManifest {
    PluginManifest {
        id: "netease".into(),
        namespace: "netease".into(),
        name: "网易云音乐".into(),
        version: "0.1.0".into(),
        description: "读取歌曲、歌词和播放状态".into(),
        icon: "netease".into(),
        default_scope: false,
        ref_aliases: vec![
            "网易云音乐".into(),
            "netease-music".into(),
            "music".into(),
        ],
        capabilities: vec![
            CapabilitySpec::new(CAPABILITY_STATE, "读取当前歌曲、歌词与播放进度", RiskLevel::L0),
            CapabilitySpec::new(CAPABILITY_CONTROL, "控制播放、暂停、上一首或下一首", RiskLevel::L1),
            CapabilitySpec::new(CAPABILITY_SEEK, "跳转播放进度", RiskLevel::L1),
            CapabilitySpec::new(CAPABILITY_CONNECT, "启动客户端并连接同步", RiskLevel::L1),
        ],
        bindings: vec![
            CapabilityBinding::direct(CAPABILITY_STATE),
            CapabilityBinding::direct(CAPABILITY_CONTROL),
            CapabilityBinding::direct(CAPABILITY_SEEK),
            CapabilityBinding::direct(CAPABILITY_CONNECT),
        ],
        tools: vec![
            ToolExposure {
                name: "get_current_state".into(),
                description: "读取网易云当前歌曲、歌词和播放进度".into(),
                input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
                capability_id: CAPABILITY_STATE.into(),
                risk_level: RiskLevel::L0,
                side_effects: vec![],
                legacy_names: vec![
                    "netease.get_current_state".into(),
                    "netease_get_current_state".into(),
                ],
                timeout_ms: 20_000,
            },
            ToolExposure {
                name: "control".into(),
                description: "控制网易云播放、暂停、上一首或下一首".into(),
                input_schema: json!({"type":"object","required":["action"],"properties":{"action":{"type":"string","enum":["play","pause","next","previous"]}},"additionalProperties":false}),
                capability_id: CAPABILITY_CONTROL.into(),
                risk_level: RiskLevel::L1,
                side_effects: vec!["修改播放状态".into()],
                legacy_names: vec!["netease.control".into(), "netease_control".into()],
                timeout_ms: INTERACTIVE_TIMEOUT_MS,
            },
            ToolExposure {
                name: "seek".into(),
                description: "跳转网易云播放进度".into(),
                input_schema: json!({"type":"object","required":["positionMs"],"properties":{"positionMs":{"type":"integer","minimum":0}},"additionalProperties":false}),
                capability_id: CAPABILITY_SEEK.into(),
                risk_level: RiskLevel::L1,
                side_effects: vec!["修改播放进度".into()],
                legacy_names: vec!["netease.seek".into(), "netease_seek".into()],
                timeout_ms: INTERACTIVE_TIMEOUT_MS,
            },
            ToolExposure {
                name: "start_or_connect".into(),
                description: "启动网易云音乐并连接歌曲/歌词同步，属于网易云插件。执行前弹出确认；不会自动退出或重启已经运行的客户端。仅在用户要求启动或连接网易云时使用。".into(),
                input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
                capability_id: CAPABILITY_CONNECT.into(),
                risk_level: RiskLevel::L1,
                side_effects: vec!["启动桌面应用".into(), "开启经用户确认的本机音乐同步连接".into()],
                legacy_names: vec!["netease.start_or_connect".into(), "netease_start_or_connect".into()],
                timeout_ms: INTERACTIVE_TIMEOUT_MS,
            },
        ],
        resources: vec![],
        events: vec![],
    }
}

/// `shared` 为 None 时（纯逻辑测试、无 Tauri 环境）处理器统一返回结构化错误，
/// 但能力和工具名仍然完整注册，保证工具解析与轨迹行为可测。
pub fn handlers(shared: Option<SharedNetease>) -> HandlerMap {
    let mut handlers = HandlerMap::new();
    let unavailable = |shared: &Option<SharedNetease>| {
        shared.is_none().then(|| {
            ToolExecutionResult::failure(
                "HOST_UNAVAILABLE",
                "此执行环境不支持网易云工具",
                false,
            )
        })
    };

    let state_shared = shared.clone();
    bind(&mut handlers, CAPABILITY_STATE, move |_call, _ctx| {
        let shared = state_shared.clone();
        async move {
            if let Some(error) = unavailable(&shared) {
                return error;
            }
            let shared = shared.expect("checked above");
            ToolExecutionResult::success("已读取网易云播放状态", json!(shared.runtime.current()))
        }
    });

    let control_shared = shared.clone();
    bind(&mut handlers, CAPABILITY_CONTROL, move |call, ctx| {
        let shared = control_shared.clone();
        async move {
            if let Some(error) = unavailable(&shared) {
                return error;
            }
            let shared = shared.expect("checked above");
            let Some(action) = call
                .arguments
                .get("action")
                .and_then(serde_json::Value::as_str)
                .filter(|action| matches!(*action, "play" | "pause" | "next" | "previous"))
                .map(str::to_owned)
            else {
                return ToolExecutionResult::failure(
                    "INVALID_TOOL_ARGS",
                    "参数类型或字段不符合工具定义",
                    false,
                );
            };
            if ctx.is_cancelled() {
                return super::host::cancelled_result();
            }
            control(&shared, &action).await
        }
    });

    let seek_shared = shared.clone();
    bind(&mut handlers, CAPABILITY_SEEK, move |call, ctx| {
        let shared = seek_shared.clone();
        async move {
            if let Some(error) = unavailable(&shared) {
                return error;
            }
            let shared = shared.expect("checked above");
            let Some(position_ms) = call
                .arguments
                .get("positionMs")
                .and_then(serde_json::Value::as_u64)
            else {
                return ToolExecutionResult::failure(
                    "INVALID_TOOL_ARGS",
                    "参数类型或字段不符合工具定义",
                    false,
                );
            };
            if ctx.is_cancelled() {
                return super::host::cancelled_result();
            }
            seek(&shared, position_ms).await
        }
    });

    bind(&mut handlers, CAPABILITY_CONNECT, move |_call, ctx| {
        let shared = shared.clone();
        async move {
            if let Some(error) = unavailable(&shared) {
                return error;
            }
            let shared = shared.expect("checked above");
            if ctx.is_cancelled() {
                return super::host::cancelled_result();
            }
            use crate::netease::launcher::StartOutcome;
            match crate::netease::start_for_agent(shared).await {
                StartOutcome::Ready => ToolExecutionResult::success(
                    "客户端调试页面已就绪，已请求后台同步连接",
                    json!({"applicationId":"netease","status":"ready_for_sync"}),
                ),
                StartOutcome::NeedsRestart => ToolExecutionResult::failure(
                    "RESTART_REQUIRED",
                    "网易云客户端已运行，但未开启同步。没有结束进程，请用户在音乐面板确认重启后再同步。",
                    false,
                ),
                StartOutcome::NotFound => ToolExecutionResult::failure(
                    "APP_NOT_FOUND",
                    "未找到网易云安装路径，请配置安装路径后重试",
                    false,
                ),
                StartOutcome::Timeout => ToolExecutionResult::failure(
                    "LAUNCH_TIMEOUT",
                    "等待客户端同步页面超时，不能确认启动成功",
                    true,
                ),
            }
        }
    });
    handlers
}

/// 播放控制：CDP 会话由 netease 模块持有，Agent 层不接触协议细节。
async fn control(shared: &SharedNetease, action: &str) -> ToolExecutionResult {
    let mut guard = shared.session.lock().await;
    let Some(session) = guard.as_mut() else {
        return ToolExecutionResult::failure("NETEASE_NOT_CONNECTED", "网易云当前未连接", true);
    };
    if crate::netease::sync::control(&shared.runtime, session, action).await {
        ToolExecutionResult::success(
            format!("已执行网易云操作：{action}"),
            json!({"action": action}),
        )
    } else {
        ToolExecutionResult::failure("NETEASE_CONTROL_FAILED", "网易云操作未成功执行", true)
    }
}

async fn seek(shared: &SharedNetease, position_ms: u64) -> ToolExecutionResult {
    let mut guard = shared.session.lock().await;
    let Some(session) = guard.as_mut() else {
        return ToolExecutionResult::failure("NETEASE_NOT_CONNECTED", "网易云当前未连接", true);
    };
    if crate::netease::sync::seek(&shared.runtime, session, position_ms).await {
        ToolExecutionResult::success(
            format!("已跳转到 {} 秒", position_ms / 1000),
            json!({"positionMs": position_ms}),
        )
    } else {
        ToolExecutionResult::failure("NETEASE_SEEK_FAILED", "网易云进度跳转未成功执行", true)
    }
}
