use super::super::{native_dialog, tools::ToolExecutionResult};
use serde::Deserialize;
use serde_json::json;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

const MAX_BYTES: usize = 128 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadArgs {
    #[serde(default)]
    pub suggested_path: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateArgs {
    pub filename: String,
    pub content: String,
    #[serde(default)]
    pub suggested_directory: Option<String>,
}

pub fn validate_create(args: &CreateArgs) -> Result<(), String> {
    let name = args.filename.as_str();
    // 不做 trim：带首尾空白的名字会被 Windows 规范化成另一个文件，必须直接拒绝。
    if name.is_empty()
        || name.len() > 180
        || name.trim() != name
        || name.contains(['/', '\\', ':', '<', '>', '"', '|', '?', '*'])
        || name.chars().any(char::is_control)
        || name.ends_with(['.', ' '])
    {
        return Err("filename 必须是普通文件名，不含目录或特殊字符".into());
    }
    let stem = name.split('.').next().unwrap_or("").to_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.ends_with(['1', '2', '3', '4', '5', '6', '7', '8', '9']))
    {
        return Err("不支持 Windows 保留文件名".into());
    }
    if !is_text_path(Path::new(name))
        || args.content.len() > MAX_BYTES
        || args.content.contains('\0')
    {
        return Err("只支持不超过 128 KiB 的 UTF-8 文本，不支持二进制或可执行文件".into());
    }
    Ok(())
}

fn is_text_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase()
            .as_str(),
        "txt"
            | "md"
            | "markdown"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
            | "rs"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "css"
            | "html"
            | "xml"
            | "csv"
            | "log"
            | "py"
    )
}

// Hold directory handles without delete sharing while using an explicitly selected path.
fn guard_selected(path: &Path) -> Result<Vec<fs::File>, String> {
    if !path.is_absolute()
        || path.to_string_lossy().starts_with("\\\\")
        || path.to_string_lossy().starts_with("//")
    {
        return Err("请选择本机绝对路径，不支持网络共享路径".into());
    }
    let mut current = PathBuf::new();
    let mut guards = vec![];
    for component in path.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err("不支持符号链接或 Junction/reparse point".into());
                    }
                    if metadata.is_dir() {
                        guards.push(
                            fs::OpenOptions::new()
                                .read(true)
                                .share_mode(3)
                                .custom_flags(0x02000000)
                                .open(&current)
                                .map_err(|e| e.to_string())?,
                        );
                    }
                }
                if metadata.file_type().is_symlink() {
                    return Err("不支持符号链接".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && current == path => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(guards)
}

pub fn read_selected(path: &Path) -> Result<serde_json::Value, String> {
    let _guards = guard_selected(path)?;
    if !is_text_path(path) {
        return Err("暂只支持常见文本文件".into());
    }
    #[cfg(windows)]
    use std::os::windows::fs::OpenOptionsExt;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("不是普通文件".into());
    }
    let mut bytes = vec![];
    file.take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES || bytes.contains(&0) {
        return Err("文件过大或包含二进制内容".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "暂只支持 UTF-8 编码，请先转换编码")?;
    let text = text.trim_start_matches('\u{feff}');
    Ok(
        json!({"path":path,"content":text.chars().take(12000).collect::<String>(),"truncated":text.chars().count()>12000,"untrusted":true,"encoding":"UTF-8"}),
    )
}

pub fn create_selected(path: &Path, content: &str) -> Result<serde_json::Value, String> {
    let _guards = guard_selected(path)?;
    validate_create(&CreateArgs {
        filename: path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("文件名无效")?
            .into(),
        content: content.into(),
        suggested_directory: None,
    })?;
    if path.exists() {
        return Err("目标文件已存在；此工具只新建、不覆盖，请选择新文件名".into());
    }
    let parent = path.parent().ok_or("输出目录不存在")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(content.as_bytes())
        .map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist_noclobber(path)
        .map_err(|e| format!("新建失败，未覆盖现有文件：{}", e.error))?;
    Ok(json!({"path":path,"bytes":content.len(),"operation":"create","encoding":"UTF-8"}))
}

pub async fn read(
    app: &AppHandle,
    args: ReadArgs,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> ToolExecutionResult {
    if cancelled() {
        return native_dialog::cancelled_result();
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_title("选择允许 Agent 读取的文本文件");
    if let Some(path) = args.suggested_path.as_ref().map(Path::new) {
        if let Some(parent) = path.parent().filter(|p| p.is_absolute()) {
            dialog = dialog.set_directory(parent);
        }
        if let Some(name) = path.file_name().and_then(|p| p.to_str()) {
            dialog = dialog.set_file_name(name);
        }
    }
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    dialog.pick_file(move |path| {
        let _ = send.send(path);
    });
    let path = match native_dialog::wait(receive, cancelled).await {
        Ok(Some(path)) => match path.into_path() {
            Ok(path) => path,
            Err(_) => {
                return ToolExecutionResult::failure("INVALID_PATH", "无法解析选择的路径", false)
            }
        },
        Ok(None) => {
            return ToolExecutionResult::failure("USER_DENIED", "用户取消选取，未读取文件", false)
        }
        Err(error) => return error,
    };
    match read_selected(&path) {
        Ok(data) => ToolExecutionResult::success("已读取用户选中的文本文件", data),
        Err(error) => ToolExecutionResult::failure("FILE_READ_FAILED", error, false),
    }
}

pub async fn create(
    app: &AppHandle,
    args: CreateArgs,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> ToolExecutionResult {
    if let Err(error) = validate_create(&args) {
        return ToolExecutionResult::failure("INVALID_TOOL_ARGS", error, false);
    }
    if let Err(error) = native_dialog::confirm(app, format!("新建文本文件：{}\n\n内容预览：\n{}\n\n确认后请选择保存位置。只创建新文件，不覆盖已有文件。", args.filename, args.content.chars().take(1200).collect::<String>()), cancelled).await { return error; }
    let (send, receive) = tokio::sync::oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_title("保存 Agent 生成的文本文件（不覆盖）")
        .set_file_name(&args.filename);
    if let Some(path) = args
        .suggested_directory
        .as_ref()
        .map(Path::new)
        .filter(|p| p.is_absolute())
    {
        dialog = dialog.set_directory(path);
    }
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    dialog.save_file(move |path| {
        let _ = send.send(path);
    });
    let path = match native_dialog::wait(receive, cancelled).await {
        Ok(Some(path)) => match path.into_path() {
            Ok(path) => path,
            Err(_) => {
                return ToolExecutionResult::failure("INVALID_PATH", "无法解析保存路径", false)
            }
        },
        Ok(None) => {
            return ToolExecutionResult::failure("USER_DENIED", "用户取消保存，未写入文件", false)
        }
        Err(error) => return error,
    };
    if cancelled() {
        return native_dialog::cancelled_result();
    }
    match create_selected(&path, &args.content) {
        Ok(data) => ToolExecutionResult::success("已创建文本文件", data),
        Err(error) => ToolExecutionResult::failure("FILE_CREATE_FAILED", error, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn create_read_and_no_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        create_selected(&path, "我").unwrap();
        assert_eq!(read_selected(&path).unwrap()["content"], "我");
        assert!(create_selected(&path, "replacement").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "我");
    }
    #[test]
    fn rejects_unsafe_names_and_binary() {
        for name in ["../a.txt", "a.txt:stream", "CON.txt", "a.exe", "a.txt "] {
            assert!(
                validate_create(&CreateArgs {
                    filename: name.into(),
                    content: "hi".into(),
                    suggested_directory: None
                })
                .is_err(),
                "{name}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("binary.txt");
        fs::write(&path, [0, 1, 2]).unwrap();
        assert!(read_selected(&path).is_err());
        assert!(create_selected(Path::new("\\\\server\\share\\a.txt"), "hi").is_err());
    }
}
