use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PluginRisk {
    L0,
    L1,
    L2,
    L3,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub builtin: bool,
    pub enabled_by_default: bool,
    pub risk: PluginRisk,
    pub tools: Vec<String>,
    pub slash_actions: Vec<String>,
}

pub fn builtin_registry() -> Vec<PluginManifest> {
    vec![
        manifest(
            "system",
            "系统",
            "时间、窗口和应用启动",
            PluginRisk::L0,
            true,
            &["system.get_time", "system.list_windows"],
            &["/system.time"],
        ),
        manifest(
            "files",
            "文件",
            "读取、编辑、备份和撤销文件",
            PluginRisk::L2,
            true,
            &["files.search", "files.read", "files.write", "files.diff"],
            &["/file.open", "/file.search"],
        ),
        manifest(
            "web-search",
            "网页搜索",
            "搜索网络并保留来源",
            PluginRisk::L1,
            true,
            &["web_search.search", "web_search.open_result"],
            &["/web.search"],
        ),
        manifest(
            "desktop",
            "桌面操作",
            "受控的 UIA 和键鼠操作",
            PluginRisk::L3,
            false,
            &[
                "computer.inspect_window",
                "computer.invoke",
                "computer.type_text",
            ],
            &["/computer.inspect"],
        ),
        manifest(
            "netease",
            "网易云音乐",
            "读取歌曲、歌词和播放状态",
            PluginRisk::L1,
            true,
            &["netease.current", "netease.control", "netease.seek"],
            &["/music.current"],
        ),
        manifest(
            "markdown",
            "Markdown 转 PDF",
            "把 Markdown 文件转换为 PDF",
            PluginRisk::L1,
            true,
            &["markdown.read", "markdown.to_pdf"],
            &["/markdown.pdf"],
        ),
    ]
}

fn manifest(
    id: &str,
    name: &str,
    description: &str,
    risk: PluginRisk,
    enabled: bool,
    tools: &[&str],
    slash_actions: &[&str],
) -> PluginManifest {
    PluginManifest {
        id: id.into(),
        name: name.into(),
        version: "0.1.0".into(),
        description: description.into(),
        builtin: true,
        enabled_by_default: enabled,
        risk,
        tools: tools.iter().map(|item| (*item).into()).collect(),
        slash_actions: slash_actions.iter().map(|item| (*item).into()).collect(),
    }
}

#[tauri::command]
pub fn list_builtin_plugins() -> Vec<PluginManifest> {
    builtin_registry()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_safe_defaults_and_disabled_desktop() {
        let plugins = builtin_registry();
        assert!(plugins.iter().any(|plugin| plugin.id == "netease"));
        assert!(plugins.iter().any(|plugin| plugin.id == "markdown"));
        assert!(
            !plugins
                .iter()
                .find(|plugin| plugin.id == "desktop")
                .unwrap()
                .enabled_by_default
        );
    }
}
