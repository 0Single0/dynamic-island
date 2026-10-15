//! 通用应用启动：按绝对路径或开始菜单名称启动本机程序。
//!
//! Agent 全自主执行，启动前不再弹确认。这里只做技术护栏：
//! 绝对路径、拒绝 UNC/符号链接、拒绝自身进程与本应用可执行文件。
//! 终端与浏览器仍在 `computer` 的 UIA 边界内被拒绝，但作为独立进程启动是允许的。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// 单次启动结果：用于轨迹里记录"到底启了什么、走哪条路径"。
#[derive(Debug, Clone)]
pub struct LaunchOutcome {
    pub target: String,
    pub via: &'static str,
}

fn lower_ext(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_lowercase()
}

fn guard(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("必须使用本机绝对路径".into());
    }
    let raw = path.to_string_lossy();
    if raw.starts_with("\\\\") || raw.starts_with("//") {
        return Err("不支持网络共享路径".into());
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err("不支持符号链接或 Junction/reparse point".into());
                    }
                }
                if metadata.file_type().is_symlink() {
                    return Err("不支持符号链接".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

/// 递归收集开始菜单里的快捷方式（上限 2000 条，深度上限 5 层）。
fn collect_links(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 5 || out.len() >= 2000 {
        return;
    }
    let Ok(reader) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in reader.filter_map(Result::ok) {
        if out.len() >= 2000 {
            return;
        }
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            collect_links(&path, out, depth + 1);
        } else if matches!(lower_ext(&path).as_str(), "lnk" | "url") {
            out.push(path);
        }
    }
}

/// 在开始菜单中按友好名查找快捷方式：先精确匹配文件名，再退化为包含匹配。
pub fn resolve_by_name(name: &str) -> Result<PathBuf, String> {
    let needle = name.trim().to_lowercase();
    if needle.is_empty() {
        return Err("name 不能为空".into());
    }
    let mut roots = Vec::new();
    if let Ok(program_data) = std::env::var("ProgramData") {
        roots.push(
            PathBuf::from(program_data).join("Microsoft\\Windows\\Start Menu\\Programs"),
        );
    }
    if let Ok(app_data) = std::env::var("APPDATA") {
        roots.push(PathBuf::from(app_data).join("Microsoft\\Windows\\Start Menu\\Programs"));
    }
    let mut candidates = Vec::new();
    for root in roots {
        collect_links(&root, &mut candidates, 0);
    }
    let mut partial = None;
    for path in &candidates {
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_lowercase();
        if stem == needle {
            return Ok(path.clone());
        }
        if partial.is_none() && stem.contains(&needle) {
            partial = Some(path.clone());
        }
    }
    partial.ok_or_else(|| format!("开始菜单中没有找到匹配「{name}」的快捷方式，请改用绝对路径 path"))
}

/// 启动目标：`.exe`/`.com` 直接创建进程；`.lnk`/`.url`/`.bat`/`.cmd` 及文档走 shell 关联。
pub fn launch_path(path: &Path, args: &[String]) -> Result<LaunchOutcome, String> {
    guard(path)?;
    if !path.exists() {
        return Err(format!("目标不存在：{}", path.display()));
    }
    let ext = lower_ext(path);
    let target = path.to_string_lossy().to_string();
    match ext.as_str() {
        "exe" | "com" => {
            let mut command = Command::new(path);
            command
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                // CREATE_NO_WINDOW：GUI 程序本就没有控制台，这里避免控制台闪烁。
                command.creation_flags(0x0800_0000);
            }
            command
                .spawn()
                .map_err(|error| format!("启动失败：{error}"))?;
            Ok(LaunchOutcome { target, via: "direct" })
        }
        _ => {
            // shell 关联：start 会按注册表处理 .lnk/.url/文档，并转发后续参数。
            let mut command = Command::new("cmd");
            command
                .arg("/C")
                .arg("start")
                .arg("")
                .arg(path)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x0800_0000);
            }
            command
                .spawn()
                .map_err(|error| format!("启动失败：{error}"))?;
            Ok(LaunchOutcome {
                target,
                via: "shell",
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_and_unc_paths_are_rejected() {
        assert!(guard(Path::new("notepad.exe")).is_err());
        assert!(guard(Path::new("\\\\server\\share\\a.exe")).is_err());
    }

    #[test]
    fn missing_target_is_reported() {
        let missing = std::env::temp_dir().join("dynamic-island-does-not-exist-12345.exe");
        assert!(launch_path(&missing, &[]).is_err());
    }

    #[test]
    fn empty_name_is_rejected() {
        assert!(resolve_by_name("   ").is_err());
    }
}
