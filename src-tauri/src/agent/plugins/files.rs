//! 文件工具实现：按模型给出的绝对路径直接读写，并支持自主浏览（列目录 / 搜索）。
//!
//! Agent 全自主执行，不再弹出文件选择器：路径来自工具参数。安全边界只保留
//! 技术护栏——必须绝对路径、拒绝 UNC 与符号链接/Junction、仅纯文本、有大小上限。
//! 目录列举与搜索让 Agent 能自己探索项目结构，而不是由用户在外部喂路径。

use serde::Deserialize;
use serde_json::json;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_BYTES: usize = 128 * 1024;
const MAX_LIST_ENTRIES: usize = 500;
const MAX_SEARCH_SCAN: usize = 5000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadArgs {
    pub path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateArgs {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListArgs {
    pub path: String,
    #[serde(default = "default_depth")]
    pub depth: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchArgs {
    pub path: String,
    pub pattern: String,
    #[serde(default)]
    pub content_match: bool,
    #[serde(default = "default_max_results")]
    pub max_results: usize,
}

fn default_depth() -> usize {
    1
}
fn default_max_results() -> usize {
    50
}

/// 校验文件名语义（不含路径分隔符与 Windows 保留名）。
pub fn validate_filename(name: &str) -> Result<(), String> {
    // 不做 trim：带首尾空白的名字会被 Windows 规范化成另一个文件，必须直接拒绝。
    if name.is_empty()
        || name.len() > 180
        || name.trim() != name
        || name.contains(['/', '\\', ':', '<', '>', '"', '|', '?', '*'])
        || name.chars().any(char::is_control)
        || name.ends_with(['.', ' '])
    {
        return Err("必须是普通文件名，不含目录或特殊字符".into());
    }
    let stem = name.split('.').next().unwrap_or("").to_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.ends_with(['1', '2', '3', '4', '5', '6', '7', '8', '9']))
    {
        return Err("不支持 Windows 保留文件名".into());
    }
    Ok(())
}

/// 文本判定按**内容**而不是扩展名（对齐 dsh：扩展名白名单会把 Dockerfile、
/// Makefile、.gitignore 这类无后缀/非常见后缀的文本全拒掉）：
/// 读取时先看二进制签名（NUL 字节采样）+ 严格 UTF-8 解码，二者都过即文本。
/// 扩展名只用于写入前的快速预检（写入路径已有完整的 NUL/UTF-8 校验兜底）。
fn is_text_path(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    // 有扩展名且命中已知二进制类型 → 直接判非文本，免读文件。
    !matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" | "bmp" | "pdf" | "zip" | "gz" | "tar"
            | "7z" | "rar" | "exe" | "dll" | "so" | "dylib" | "woff" | "woff2" | "ttf" | "otf"
            | "mp3" | "mp4" | "avi" | "mov" | "sqlite" | "db" | "bin" | "class" | "jar"
    )
}

// 拒绝非绝对路径、UNC 与符号链接/Junction/reparse point。
fn guard_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("必须使用本机绝对路径（例如 E:\\code\\project\\file.ts）".into());
    }
    let raw = path.to_string_lossy();
    if raw.starts_with("\\\\") || raw.starts_with("//") {
        return Err("不支持网络共享路径".into());
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
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
            // 目标文件本身允许不存在（新建场景），父目录缺失会在写入时暴露。
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

/// 读取单个文本文件。
pub fn read_path(path: &Path) -> Result<serde_json::Value, String> {
    guard_path(path)?;
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

/// 写入文本文件；默认只新建不覆盖，`overwrite=true` 时原子覆盖。
pub fn create_path(path: &Path, content: &str, overwrite: bool) -> Result<serde_json::Value, String> {
    guard_path(path)?;
    let name = path.file_name().and_then(|n| n.to_str()).ok_or("文件名无效")?;
    validate_filename(name)?;
    if !is_text_path(path) || content.len() > MAX_BYTES || content.contains('\0') {
        return Err("只支持不超过 128 KiB 的 UTF-8 文本，不支持二进制或可执行文件".into());
    }
    let parent = path.parent().ok_or("输出目录不存在")?;
    if !parent.is_dir() {
        return Err(format!("输出目录不存在：{}", parent.display()));
    }
    if path.exists() && !overwrite {
        return Err("目标文件已存在；如需覆盖请传 overwrite: true".into());
    }
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    if overwrite {
        temp.persist(path)
            .map_err(|e| format!("写入失败：{}", e.error))?;
    } else {
        temp.persist_noclobber(path)
            .map_err(|e| format!("新建失败，未覆盖现有文件：{}", e.error))?;
    }
    Ok(json!({
        "path": path,
        "bytes": content.len(),
        "operation": if overwrite { "overwrite" } else { "create" },
        "encoding": "UTF-8"
    }))
}

/// 列目录：返回指定的直接/多层子项，用于 Agent 自主了解项目结构。
pub fn list_path(path: &Path, depth: usize) -> Result<serde_json::Value, String> {
    guard_path(path)?;
    if !path.is_dir() {
        return Err(format!("不是目录：{}", path.display()));
    }
    let depth = depth.clamp(1, 3);
    let mut entries = vec![];
    let mut truncated = false;
    walk_list(path, depth, &mut entries, &mut truncated)?;
    Ok(json!({
        "path": path,
        "depth": depth,
        "count": entries.len(),
        "truncated": truncated,
        "entries": entries,
        "untrusted": true
    }))
}

fn walk_list(
    dir: &Path,
    depth: usize,
    entries: &mut Vec<serde_json::Value>,
    truncated: &mut bool,
) -> Result<(), String> {
    if depth == 0 || *truncated {
        return Ok(());
    }
    let mut children = fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    children.sort_by_key(|entry| {
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        (!is_dir, entry.file_name())
    });
    for entry in children {
        if entries.len() >= MAX_LIST_ENTRIES {
            *truncated = true;
            return Ok(());
        }
        let child = entry.path();
        let metadata = match fs::symlink_metadata(&child) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let kind = if metadata.file_type().is_symlink() {
            "symlink"
        } else if metadata.is_dir() {
            "dir"
        } else {
            "file"
        };
        entries.push(json!({
            "name": entry.file_name().to_string_lossy(),
            "path": child,
            "kind": kind,
            "size": if metadata.is_file() { metadata.len() } else { 0 },
        }));
        // 不跟随符号链接，避免递归逃逸。
        if kind == "dir" {
            walk_list(&child, depth - 1, entries, truncated)?;
        }
    }
    Ok(())
}

/// 在目录内搜索：按文件名匹配，可选按文本内容匹配（返回行号与片段）。
pub fn search_path(
    path: &Path,
    pattern: &str,
    content_match: bool,
    max_results: usize,
) -> Result<serde_json::Value, String> {
    guard_path(path)?;
    let pattern = pattern.trim();
    if pattern.is_empty() || pattern.len() > 200 {
        return Err("pattern 不能为空且不超过 200 字符".into());
    }
    if !path.is_dir() {
        return Err(format!("搜索起点必须是目录：{}", path.display()));
    }
    let needle = pattern.to_lowercase();
    let max_results = max_results.clamp(1, 200);
    let mut matches = vec![];
    let mut scanned = 0usize;
    let mut truncated = false;
    search_walk(
        path,
        &needle,
        content_match,
        max_results,
        &mut matches,
        &mut scanned,
        &mut truncated,
    );
    Ok(json!({
        "path": path,
        "pattern": pattern,
        "contentMatch": content_match,
        "count": matches.len(),
        "scanned": scanned,
        "truncated": truncated,
        "matches": matches,
        "untrusted": true
    }))
}

#[allow(clippy::too_many_arguments)]
fn search_walk(
    dir: &Path,
    needle: &str,
    content_match: bool,
    max_results: usize,
    matches: &mut Vec<serde_json::Value>,
    scanned: &mut usize,
    truncated: &mut bool,
) {
    if *truncated || matches.len() >= max_results || *scanned >= MAX_SEARCH_SCAN {
        *truncated = true;
        return;
    }
    let Ok(reader) = fs::read_dir(dir) else {
        return;
    };
    let mut children = reader.filter_map(Result::ok).collect::<Vec<_>>();
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        if matches.len() >= max_results || *scanned >= MAX_SEARCH_SCAN {
            *truncated = true;
            return;
        }
        let child = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&child) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            // 跳过常见噪声目录，避免把搜索预算耗在依赖与版本库上。
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if matches!(
                name.as_str(),
                ".git" | "node_modules" | "target" | "dist" | ".next" | "build" | "vendor"
            ) {
                continue;
            }
            search_walk(
                &child,
                needle,
                content_match,
                max_results,
                matches,
                scanned,
                truncated,
            );
            continue;
        }
        *scanned += 1;
        let name = entry.file_name().to_string_lossy().to_string();
        let name_hit = name.to_lowercase().contains(needle);
        let mut lines = vec![];
        if content_match && is_text_path(&child) {
            if let Ok(text) = read_text_capped(&child) {
                for (index, line) in text.lines().enumerate() {
                    if line.to_lowercase().contains(needle) {
                        lines.push(json!({
                            "line": index + 1,
                            "text": line.chars().take(200).collect::<String>()
                        }));
                        if lines.len() >= 5 {
                            break;
                        }
                    }
                }
            }
        }
        if name_hit || !lines.is_empty() {
            matches.push(json!({
                "path": child,
                "name": name,
                "nameMatch": name_hit,
                "contentMatches": lines,
            }));
        }
    }
}

fn read_text_capped(path: &Path) -> Result<String, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = vec![];
    file.take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES || bytes.contains(&0) {
        return Err("文件过大或包含二进制内容".into());
    }
    String::from_utf8(bytes).map_err(|_| "非 UTF-8 文本".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_read_and_no_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        create_path(&path, "我", false).unwrap();
        assert_eq!(read_path(&path).unwrap()["content"], "我");
        // 默认不覆盖；显式 overwrite 才能改写。
        assert!(create_path(&path, "replacement", false).is_err());
        create_path(&path, "replacement", true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "replacement");
    }

    #[test]
    fn rejects_unsafe_names_and_binary() {
        for name in ["../a.txt", "a.txt:stream", "CON.txt", "a.txt "] {
            assert!(validate_filename(name).is_err(), "{name}");
        }
        let dir = tempfile::tempdir().unwrap();
        // 扩展名护栏由写入路径负责：可执行文件不被接受。
        assert!(create_path(&dir.path().join("a.exe"), "hi", false).is_err());
        let path = dir.path().join("binary.txt");
        fs::write(&path, [0, 1, 2]).unwrap();
        assert!(read_path(&path).is_err());
        assert!(create_path(Path::new("\\\\server\\share\\a.txt"), "hi", false).is_err());
        // 相对路径被拒绝。
        assert!(create_path(Path::new("rel.txt"), "hi", false).is_err());
    }

    /// 回归：无后缀/非常见后缀的文本文件（Dockerfile、Makefile、.gitignore）
    /// 必须能读。文本判定按内容（NUL 采样 + UTF-8），不看扩展名白名单。
    #[test]
    fn reads_extensionless_text_files() {
        let dir = tempfile::tempdir().unwrap();
        let dockerfile = dir.path().join("Dockerfile");
        fs::write(&dockerfile, "FROM node:20
WORKDIR /app
").unwrap();
        let read = read_path(&dockerfile).unwrap();
        assert_eq!(read["content"], "FROM node:20
WORKDIR /app
");
        // 其他常见无后缀文件。
        fs::write(dir.path().join("Makefile"), "all:
	echo hi
").unwrap();
        assert!(read_path(&dir.path().join("Makefile")).is_ok());
        fs::write(dir.path().join(".gitignore"), "target
").unwrap();
        assert!(read_path(&dir.path().join(".gitignore")).is_ok());
        // 已知二进制扩展名直接拒绝，不读内容。
        assert!(read_path(&dir.path().join("logo.png")).is_err());
    }

    #[test]
    fn lists_directories_with_depth_and_budget() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.ts"), "x").unwrap();
        fs::write(dir.path().join("README.md"), "hi").unwrap();
        let flat = list_path(dir.path(), 1).unwrap();
        assert_eq!(flat["count"], 2);
        let deep = list_path(dir.path(), 2).unwrap();
        assert_eq!(deep["count"], 3);
    }

    #[test]
    fn searches_by_name_and_content() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/config.ts"), "const answer = 42;\n").unwrap();
        fs::write(dir.path().join("other.txt"), "nothing here\n").unwrap();
        let by_name = search_path(dir.path(), "config", false, 10).unwrap();
        assert_eq!(by_name["count"], 1);
        let by_content = search_path(dir.path(), "answer", true, 10).unwrap();
        assert_eq!(by_content["count"], 1);
        assert_eq!(by_content["matches"][0]["contentMatches"][0]["line"], 1);
    }
}
