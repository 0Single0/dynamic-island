//! Scoped UI Automation backend. No COM pointers or permanent HWNDs reach the model.
mod guard;

use crate::agent::{native_dialog, tools::ToolExecutionResult};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use uiautomation::{
    patterns::{UIInvokePattern, UITextPattern, UIValuePattern},
    types::Handle,
    UIAutomation, UIElement, UITreeWalker,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

const TTL: Duration = Duration::from_secs(120);
const MAX_NODES: usize = 80;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct WindowTarget {
    hwnd: isize,
    pid: u32,
    title: String,
    runtime: Vec<i32>,
    expires: Instant,
}
#[derive(Clone)]
struct ElementTarget {
    window: WindowTarget,
    runtime: Vec<i32>,
    name: String,
    control: String,
    path: Vec<usize>,
}
#[derive(Default)]
pub struct Targets {
    windows: HashMap<String, WindowTarget>,
    elements: HashMap<String, ElementTarget>,
}
impl Targets {
    fn prune(&mut self) {
        let now = Instant::now();
        self.windows.retain(|_, w| w.expires > now);
        self.elements.retain(|_, e| e.window.expires > now);
        if self.windows.len() + self.elements.len() > 512 {
            self.windows.clear();
            self.elements.clear();
        }
    }
    fn window(&self, id: &str) -> Result<WindowTarget, String> {
        self.windows
            .get(id)
            .filter(|w| w.expires > Instant::now())
            .cloned()
            .ok_or_else(|| "窗口 ID 已过期或不属于本次运行，请重新枚举".into())
    }
    fn element(&self, id: &str) -> Result<ElementTarget, String> {
        self.elements
            .get(id)
            .filter(|e| e.window.expires > Instant::now())
            .cloned()
            .ok_or_else(|| "元素 ID 已过期或不属于本次运行，请重新检查窗口".into())
    }
}
fn id(prefix: &str) -> String {
    format!("{prefix}-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectArgs {
    pub window_id: String,
    #[serde(default = "depth_default")]
    pub depth: usize,
}
fn depth_default() -> usize {
    3
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ElementArgs {
    pub element_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValueArgs {
    pub element_id: String,
    pub value: String,
}

pub enum Call {
    List,
    Inspect(InspectArgs),
    Text(ElementArgs),
    Invoke(ElementArgs),
    SetValue(ValueArgs),
}

/// 工具名到调用的映射键；插件清单只声明能力，参数解析仍在这里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    List,
    Inspect,
    Text,
    Invoke,
    SetValue,
}

/// 解析并校验模型参数；未知字段、越界深度等都在这里拒绝。
pub fn parse_call(kind: CallKind, arguments: Value) -> Result<Call, String> {
    let invalid = |message: &str| message.to_owned();
    let decode = |value: Value| -> Result<Value, String> {
        if !value.is_object() {
            return Err(invalid("参数必须是 JSON 对象"));
        }
        Ok(value)
    };
    match kind {
        CallKind::List => {
            let args = decode(arguments)?;
            if args.as_object().is_some_and(|object| object.is_empty()) {
                Ok(Call::List)
            } else {
                Err(invalid("参数为空、过长或超出工具限制"))
            }
        }
        CallKind::Inspect => {
            let args: InspectArgs =
                serde_json::from_value(decode(arguments)?).map_err(|_| invalid("参数类型或字段不符合工具定义"))?;
            if args.window_id.is_empty() || args.window_id.len() > 100 || !(1..=5).contains(&args.depth) {
                return Err(invalid("参数为空、过长或超出工具限制"));
            }
            Ok(Call::Inspect(args))
        }
        CallKind::Text | CallKind::Invoke => {
            let args: ElementArgs =
                serde_json::from_value(decode(arguments)?).map_err(|_| invalid("参数类型或字段不符合工具定义"))?;
            if args.element_id.is_empty() || args.element_id.len() > 100 {
                return Err(invalid("参数为空、过长或超出工具限制"));
            }
            Ok(if kind == CallKind::Invoke {
                Call::Invoke(args)
            } else {
                Call::Text(args)
            })
        }
        CallKind::SetValue => {
            let args: ValueArgs =
                serde_json::from_value(decode(arguments)?).map_err(|_| invalid("参数类型或字段不符合工具定义"))?;
            if args.element_id.is_empty()
                || args.element_id.len() > 100
                || args.value.len() > 8000
                || args.value.contains('\0')
            {
                return Err(invalid("参数为空、过长或超出工具限制"));
            }
            Ok(Call::SetValue(args))
        }
    }
}

struct ComApartment;
struct StopOnDrop(Arc<AtomicBool>);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
fn automation() -> Result<(ComApartment, UIAutomation), String> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
    }
    let apartment = ComApartment;
    let automation = UIAutomation::new_direct().map_err(|e| e.to_string())?;
    Ok((apartment, automation))
}
fn window_element(ui: &UIAutomation, target: &WindowTarget) -> Result<UIElement, String> {
    if target.expires <= Instant::now() {
        return Err("目标已过期".into());
    }
    guard::allowed(target.hwnd, target.pid, &target.title)?;
    let element = ui
        .element_from_handle(Handle::from(target.hwnd))
        .map_err(|e| e.to_string())?;
    if element.get_process_id().map_err(|e| e.to_string())? != target.pid
        || element.get_runtime_id().map_err(|e| e.to_string())? != target.runtime
        || element.get_name().map_err(|e| e.to_string())? != target.title
    {
        return Err("目标窗口身份或标题已改变，请重新检查".into());
    }
    Ok(element)
}

fn child_at(walker: &UITreeWalker, parent: &UIElement, index: usize) -> Result<UIElement, String> {
    if index >= MAX_NODES {
        return Err("控件位置超过限制".into());
    }
    let mut element = walker.get_first_child(parent).map_err(|e| e.to_string())?;
    for _ in 0..index {
        element = walker
            .get_next_sibling(&element)
            .map_err(|e| e.to_string())?;
    }
    Ok(element)
}
fn checked_element(ui: &UIAutomation, target: &ElementTarget) -> Result<UIElement, String> {
    let mut element = window_element(ui, &target.window)?;
    let walker = ui.create_tree_walker().map_err(|e| e.to_string())?;
    for index in &target.path {
        element = child_at(&walker, &element, *index)?;
    }
    if element.get_runtime_id().map_err(|e| e.to_string())? != target.runtime
        || element.get_process_id().map_err(|e| e.to_string())? != target.window.pid
        || element.get_name().map_err(|e| e.to_string())? != target.name
        || format!(
            "{:?}",
            element.get_control_type().map_err(|e| e.to_string())?
        ) != target.control
    {
        return Err("控件身份发生变化，请重新检查窗口".into());
    }
    if element.is_password().unwrap_or(true)
        || !element.is_enabled().unwrap_or(false)
        || element.is_offscreen().unwrap_or(true)
    {
        return Err("不允许读写密码框、禁用或不可见控件".into());
    }
    Ok(element)
}
fn check_stop(stop: &AtomicBool) -> Result<(), String> {
    if stop.load(Ordering::SeqCst) {
        Err("操作已停止".into())
    } else {
        Ok(())
    }
}

fn list(ui: &UIAutomation, targets: &mut Targets, stop: &AtomicBool) -> Result<Value, String> {
    let walker = ui.create_tree_walker().map_err(|e| e.to_string())?;
    let root = ui.get_root_element().map_err(|e| e.to_string())?;
    let mut next = walker.get_first_child(&root).ok();
    let mut result = vec![];
    let start = Instant::now();
    for _ in 0..120 {
        check_stop(stop)?;
        if start.elapsed() > Duration::from_secs(5) || result.len() >= 40 {
            break;
        }
        let Some(element) = next.take() else {
            break;
        };
        next = walker.get_next_sibling(&element).ok();
        let hwnd: isize = element
            .get_native_window_handle()
            .map_err(|e| e.to_string())?
            .into();
        let pid = element.get_process_id().unwrap_or(0);
        let title = element.get_name().unwrap_or_default();
        if hwnd == 0 || title.is_empty() || title.chars().count() > 200 {
            continue;
        }
        let Ok(process) = guard::allowed(hwnd, pid, &title) else {
            continue;
        };
        let runtime = element.get_runtime_id().map_err(|e| e.to_string())?;
        let window_id = id("window");
        targets.windows.insert(
            window_id.clone(),
            WindowTarget {
                hwnd,
                pid,
                title: title.clone(),
                runtime,
                expires: Instant::now() + TTL,
            },
        );
        result.push(json!({"windowId":window_id,"title":title,"process":process}));
    }
    Ok(
        json!({"windows":result,"expiresInSeconds":120,"protectedWindowsOmitted":true,"untrusted":true}),
    )
}

fn inspect(
    ui: &UIAutomation,
    targets: &mut Targets,
    args: InspectArgs,
    stop: &AtomicBool,
) -> Result<Value, String> {
    let window = targets.window(&args.window_id)?;
    let root = window_element(ui, &window)?;
    let walker = ui.create_tree_walker().map_err(|e| e.to_string())?;
    let mut queue = std::collections::VecDeque::from([(root, vec![])]);
    let mut elements = vec![];
    let start = Instant::now();
    while let Some((element, path)) = queue.pop_front() {
        check_stop(stop)?;
        if elements.len() >= MAX_NODES || start.elapsed() > Duration::from_secs(5) {
            break;
        }
        if element.is_password().unwrap_or(true) {
            continue;
        }
        let name = element.get_name().unwrap_or_default();
        if name.chars().count() > 500 {
            continue;
        }
        let control = format!(
            "{:?}",
            element.get_control_type().map_err(|e| e.to_string())?
        );
        let element_id = id("element");
        let target = ElementTarget {
            window: window.clone(),
            runtime: element.get_runtime_id().map_err(|e| e.to_string())?,
            name: name.clone(),
            control: control.clone(),
            path: path.clone(),
        };
        elements.push(json!({"elementId":element_id,"name":name,"controlType":control,"depth":path.len(),"invoke":element.get_pattern::<UIInvokePattern>().is_ok(),"value":element.get_pattern::<UIValuePattern>().is_ok(),"text":element.get_pattern::<UITextPattern>().is_ok()}));
        targets.elements.insert(element_id, target);
        if path.len() >= args.depth {
            continue;
        }
        let mut child = walker.get_first_child(&element).ok();
        for index in 0..MAX_NODES {
            check_stop(stop)?;
            if queue.len() + elements.len() >= MAX_NODES {
                break;
            }
            let Some(current) = child.take() else {
                break;
            };
            child = walker.get_next_sibling(&current).ok();
            let mut child_path = path.clone();
            child_path.push(index);
            queue.push_back((current, child_path));
        }
    }
    Ok(
        json!({"windowId":args.window_id,"elements":elements,"limited":!queue.is_empty(),"untrusted":true}),
    )
}

fn perform(targets: &mut Targets, call: Call, stop: &AtomicBool) -> Result<Value, String> {
    let (_apartment, ui) = automation()?;
    targets.prune();
    check_stop(stop)?;
    match call {
        Call::List => list(&ui, targets, stop),
        Call::Inspect(args) => inspect(&ui, targets, args, stop),
        Call::Text(args) => {
            let target = targets.element(&args.element_id)?;
            let element = checked_element(&ui, &target)?;
            let text = if let Ok(pattern) = element.get_pattern::<UITextPattern>() {
                pattern
                    .get_document_range()
                    .and_then(|r| r.get_text(8000))
                    .map_err(|e| e.to_string())?
            } else {
                element
                    .get_pattern::<UIValuePattern>()
                    .and_then(|p| p.get_value())
                    .map_err(|_| "目标不支持读取文本")?
            };
            Ok(
                json!({"elementId":args.element_id,"text":text.chars().take(8000).collect::<String>(),"untrusted":true}),
            )
        }
        Call::Invoke(args) => {
            let target = targets.element(&args.element_id)?;
            let element = checked_element(&ui, &target)?;
            let pattern = element
                .get_pattern::<UIInvokePattern>()
                .map_err(|_| "控件不支持 Invoke，未尝试坐标点击")?;
            check_stop(stop)?;
            pattern.invoke().map_err(|e| e.to_string())?;
            Ok(json!({"elementId":args.element_id,"invoked":true}))
        }
        Call::SetValue(args) => {
            let target = targets.element(&args.element_id)?;
            let element = checked_element(&ui, &target)?;
            let pattern = element
                .get_pattern::<UIValuePattern>()
                .map_err(|_| "控件不支持 Value，未尝试键盘输入")?;
            if pattern.is_readonly().unwrap_or(true) {
                return Err("目标只读".into());
            }
            check_stop(stop)?;
            pattern.set_value(&args.value).map_err(|e| e.to_string())?;
            Ok(json!({"elementId":args.element_id,"valueSet":true}))
        }
    }
}

pub async fn execute(
    _app: &tauri::AppHandle,
    targets: Arc<Mutex<Targets>>,
    call: Call,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> ToolExecutionResult {
    // 执行前不再请求用户确认：目标合法性由 `guard` 的窗口/进程校验保证，
    // 这里只确认目标仍属于本次运行，失效时返回 STALE_TARGET。
    {
        let locked = match targets.try_lock() {
            Ok(v) => v,
            Err(_) => {
                return ToolExecutionResult::failure(
                    "DESKTOP_BUSY",
                    "上一次界面查询尚未退出，不能启动新动作",
                    false,
                )
            }
        };
        let target = match &call {
            Call::Inspect(args) => locked.window(&args.window_id).map(|_| ()),
            Call::Text(args) => locked.element(&args.element_id).map(|_| ()),
            Call::Invoke(args) => locked.element(&args.element_id).map(|_| ()),
            Call::SetValue(args) => locked.element(&args.element_id).map(|_| ()),
            Call::List => Ok(()),
        };
        if let Err(e) = target {
            return ToolExecutionResult::failure("STALE_TARGET", e, false);
        }
    }
    if cancelled() {
        return native_dialog::cancelled_result();
    }
    let stop = Arc::new(AtomicBool::new(false));
    let _stop_on_drop = StopOnDrop(stop.clone());
    let worker_stop = stop.clone();
    let mut worker = tokio::task::spawn_blocking(move || {
        let mut targets = targets.lock().map_err(|_| "电脑注册表不可用".to_owned())?;
        perform(&mut targets, call, &worker_stop)
    });
    let deadline = tokio::time::sleep(Duration::from_secs(15));
    tokio::pin!(deadline);
    let mut poll = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            result = &mut worker => return match result {
                Ok(Ok(data)) => ToolExecutionResult::success("电脑工具已执行",data),
                Ok(Err(error)) => ToolExecutionResult::failure("DESKTOP_OPERATION_FAILED",error,false),
                Err(_) => ToolExecutionResult::failure("DESKTOP_WORKER_FAILED","界面自动化线程失败",false),
            },
            _ = &mut deadline => { stop.store(true,Ordering::SeqCst); return ToolExecutionResult::failure("DESKTOP_TIMEOUT","目标应用未及时响应；已停止后续动作，正在执行的系统调用不能强制撤回",false); },
            _ = poll.tick() => if cancelled() { stop.store(true,Ordering::SeqCst); return native_dialog::cancelled_result(); },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn targets_are_run_local_and_expire() {
        let mut targets = Targets::default();
        assert!(targets.window("42").is_err());
        targets.windows.insert(
            "old".into(),
            WindowTarget {
                hwnd: 1,
                pid: 1,
                title: "fixture".into(),
                runtime: vec![1],
                expires: Instant::now() - Duration::from_secs(1),
            },
        );
        assert!(targets.window("old").is_err());
        targets.prune();
        assert!(targets.windows.is_empty());
        assert_ne!(id("window"), id("window"));
    }

    #[test]
    fn call_parsing_rejects_extra_fields_and_out_of_range_values() {
        assert!(matches!(
            parse_call(CallKind::List, json!({})),
            Ok(Call::List)
        ));
        assert!(parse_call(CallKind::List, json!({"command":"calc.exe"})).is_err());
        assert!(parse_call(
            CallKind::Inspect,
            json!({"windowId":"w","depth":99})
        )
        .is_err());
        assert!(parse_call(CallKind::Inspect, json!({"windowId":""})).is_err());
        assert!(parse_call(CallKind::Text, json!({"elementId":"e","script":"eval"})).is_err());
        assert!(matches!(
            parse_call(CallKind::Text, json!({"elementId":"e"})),
            Ok(Call::Text(_))
        ));
        assert!(matches!(
            parse_call(CallKind::Invoke, json!({"elementId":"e"})),
            Ok(Call::Invoke(_))
        ));
        assert!(parse_call(
            CallKind::SetValue,
            json!({"elementId":"e","value":"a\u{0}b"})
        )
        .is_err());
        assert!(parse_call(
            CallKind::SetValue,
            json!({"elementId":"e","value":"ok"})
        )
        .is_ok());
    }
}
