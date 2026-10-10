//! WorkBuddy 出站身份：域名、UA、请求头组。
//!
//! 三个身份（桌面端 / 官方 CLI / 官方 VSCode 插件）× 两个区域（国内版 / 国际版）。
//! 上游用 UA 与头组判断"你是什么客户端"，所以身份切换时**端点、X-Domain、UA 必须同步换**，
//! 否则会 401/403 或拿到不完整的模型目录。

use crate::gateway::{channels::AccountSnapshot, security};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Realm {
    /// 国内版：copilot.tencent.com + www.codebuddy.cn
    Cn,
    /// 国际版：www.workbuddy.ai
    Intl,
}

impl Realm {
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "intl" | "international" | "global" => Self::Intl,
            _ => Self::Cn,
        }
    }

}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outbound {
    /// 桌面端（默认）：UA `WorkBuddy/5.5.6 WorkBuddy/5.5.6 CLI/2.137.1`
    Desktop,
    /// 官方 CodeBuddy CLI。
    Cli,
    /// 官方 VSCode 插件（走 www.workbuddy.cn）。
    Vscode,
}

impl Outbound {
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "cli" | "codebuddy" => Self::Cli,
            "vscode" | "vsc" => Self::Vscode,
            _ => Self::Desktop,
        }
    }

}

#[derive(Debug, Clone)]
pub struct Hosts {
    /// 对话与配置端点。
    pub chat: String,
    /// 计费/签到端点。
    pub billing: String,
    /// Origin/Referer 用。
    pub origin: String,
    /// `X-Domain` 的值。
    pub domain: String,
}

impl Hosts {
    pub fn chat_url(&self, path: &str) -> String {
        format!("{}://{}{}", self.scheme(), self.chat, path)
    }

    pub fn billing_url(&self, path: &str) -> String {
        format!("{}://{}{}", self.scheme(), self.billing, path)
    }

    fn scheme(&self) -> &'static str {
        "https"
    }
}

pub fn hosts(realm: Realm, outbound: Outbound) -> Hosts {
    match (realm, outbound) {
        (Realm::Cn, Outbound::Vscode) => Hosts {
            chat: "www.workbuddy.cn".into(),
            billing: "www.codebuddy.cn".into(),
            origin: "https://www.codebuddy.cn".into(),
            domain: "www.workbuddy.cn".into(),
        },
        (Realm::Cn, _) => Hosts {
            chat: "copilot.tencent.com".into(),
            billing: "www.codebuddy.cn".into(),
            origin: "https://www.codebuddy.cn".into(),
            domain: "copilot.tencent.com".into(),
        },
        (Realm::Intl, Outbound::Vscode) => Hosts {
            chat: "www.workbuddy.cn".into(),
            billing: "www.workbuddy.ai".into(),
            origin: "https://www.workbuddy.ai".into(),
            domain: "www.workbuddy.ai".into(),
        },
        (Realm::Intl, _) => Hosts {
            chat: "www.workbuddy.ai".into(),
            billing: "www.workbuddy.ai".into(),
            origin: "https://www.workbuddy.ai".into(),
            domain: "www.workbuddy.ai".into(),
        },
    }
}

fn user_agent(realm: Realm, outbound: Outbound) -> String {
    match (realm, outbound) {
        (Realm::Cn, Outbound::Desktop) => {
            "WorkBuddy/5.5.6 WorkBuddy/5.5.6 CLI/2.137.1".into()
        }
        (Realm::Cn, Outbound::Cli) => "CLI/2.117.2 CodeBuddy/2.117.2".into(),
        (Realm::Cn, Outbound::Vscode) => "VSCode/1.96.0 WorkBuddy/5.5.6".into(),
        (Realm::Intl, Outbound::Desktop) => {
            "WorkBuddy/5.5.2 WorkBuddy AI/5.5.2 CLI/5.5.2".into()
        }
        (Realm::Intl, Outbound::Cli) => "CLI/2.117.2 CodeBuddy/2.117.2".into(),
        (Realm::Intl, Outbound::Vscode) => "VSCode/1.96.0 WorkBuddy/5.5.2".into(),
    }
}

/// 覆盖 `User-Agent`（而不是追加两个）：`/v3/config` 必须用桌面长 UA，
/// 否则上游直接 400（code 12403）。
pub fn with_user_agent(mut headers: Vec<(String, String)>, user_agent: &str) -> Vec<(String, String)> {
    headers.retain(|(name, _)| !name.eq_ignore_ascii_case("User-Agent"));
    headers.push(("User-Agent".into(), user_agent.to_string()));
    headers
}

/// 桌面身份的长 UA（目录接口专用）。
pub fn desktop_user_agent(realm: Realm) -> String {
    user_agent(realm, Outbound::Desktop)
}

/// 授权类接口的固定头（不带鉴权）：申请 state、轮询 token 都用这一套。
///
/// 注意：`User-Agent` 是计费/授权口径的短 UA，**不要**换成桌面长 UA，
/// 只有 `/v3/config` 才必须用桌面长 UA。
pub fn base_headers(hosts: &Hosts) -> Vec<(String, String)> {
    vec![
        ("Content-Type".into(), "application/json".into()),
        (
            "Accept".into(),
            "application/json, text/plain, */*".into(),
        ),
        ("X-Requested-With".into(), "XMLHttpRequest".into()),
        ("User-Agent".into(), billing_user_agent(&hosts.domain)),
        ("Origin".into(), hosts.origin.clone()),
        ("Referer".into(), format!("{}/", hosts.origin)),
    ]
}

fn billing_user_agent(domain: &str) -> String {
    if domain.contains("workbuddy.ai") {
        "WorkBuddy/5.5.2".into()
    } else {
        "WorkBuddy/5.5.6".into()
    }
}

/// 对话与配置请求的头组：鉴权 + 设备指纹 + 出站身份。
pub fn chat_headers(
    channel_id: &str,
    realm: Realm,
    outbound: Outbound,
    hosts: &Hosts,
    account: &AccountSnapshot,
    request_sequence: u64,
) -> Vec<(String, String)> {
    // 对话身份用长 UA：必须**替换**基头里的短 UA，否则会同时发两个 User-Agent。
    let mut headers = with_user_agent(base_headers(hosts), &user_agent(realm, outbound));
    let uid = account.uid.as_str();
    headers.extend([
        (
            "Authorization".into(),
            format!(
                "Bearer {}",
                account.access_token.as_deref().unwrap_or_default()
            ),
        ),
        ("X-User-Id".into(), uid.to_string()),
        ("X-Domain".into(), hosts.domain.clone()),
        ("X-CodeBuddy-Request".into(), "1".into()),
        ("Accept-Language".into(), "zh-CN".into()),
        // 指纹：由 (渠道, uid) 稳定派生，多账号之间天然隔离。
        (
            "X-Machine-ID".into(),
            security::derive_id(channel_id, uid, "machine"),
        ),
        (
            "X-Session-ID".into(),
            security::derive_id(channel_id, uid, "session"),
        ),
        (
            "X-Request-ID".into(),
            security::request_id(channel_id, uid, request_sequence),
        ),
    ]);

    match outbound {
        Outbound::Desktop => {
            headers.extend([
                ("X-Agent-Purpose".into(), "conversation".into()),
                ("X-IDE-Name".into(), "WorkBuddy".into()),
                ("X-IDE-Type".into(), "WorkBuddy".into()),
                ("X-IDE-Version".into(), "5.5.6".into()),
                ("X-Product".into(), "WorkBuddy".into()),
            ]);
        }
        Outbound::Cli => {
            headers.extend([
                ("X-Agent-Purpose".into(), "conversation".into()),
                ("X-Agent-Intent".into(), "craft".into()),
                ("X-IDE-Name".into(), "CodeBuddy".into()),
                ("X-IDE-Type".into(), "CLI".into()),
                ("X-Product".into(), "SaaS".into()),
            ]);
        }
        Outbound::Vscode => {
            headers.extend([
                ("X-IDE-Name".into(), "VSCode".into()),
                ("X-IDE-Type".into(), "VSCode".into()),
                ("X-Product".into(), "SaaS".into()),
                ("X-Env-ID".into(), "production".into()),
            ]);
        }
    }

    if let Some(enterprise) = account.enterprise_id.as_deref() {
        if !enterprise.is_empty() {
            headers.push(("X-Enterprise-Id".into(), enterprise.to_string()));
            headers.push(("X-Tenant-Id".into(), enterprise.to_string()));
        }
    }
    headers
}

/// 计费侧头组：比对话头更轻，无企业身份时显式声明 `X-No-Enterprise-Id: 1`。
pub fn billing_headers(
    hosts: &Hosts,
    account: &AccountSnapshot,
    client_platform: Option<&str>,
) -> Vec<(String, String)> {
    let mut headers = base_headers(hosts);
    headers.push((
        "Authorization".into(),
        format!(
            "Bearer {}",
            account.access_token.as_deref().unwrap_or_default()
        ),
    ));
    headers.push(("X-User-Id".into(), account.uid.clone()));
    headers.push(("X-Domain".into(), hosts.domain.clone()));
    headers.push(("X-CodeBuddy-Request".into(), "1".into()));
    headers.push(("X-Product".into(), "SaaS".into()));
    if let Some(platform) = client_platform {
        headers.push(("X-Client-Platform".into(), platform.to_string()));
    }
    headers.push((
        "X-Machine-ID".into(),
        security::derive_id(&account.channel_id, &account.uid, "machine"),
    ));
    headers.push((
        "X-Session-ID".into(),
        security::derive_id(&account.channel_id, &account.uid, "session"),
    ));
    match account.enterprise_id.as_deref() {
        Some(enterprise) if !enterprise.is_empty() => {
            headers.push(("X-Enterprise-Id".into(), enterprise.to_string()));
        }
        _ => headers.push(("X-No-Enterprise-Id".into(), "1".into())),
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> AccountSnapshot {
        AccountSnapshot {
            channel_id: "wb-cn".into(),
            uid: "uid-1".into(),
            access_token: Some("token".into()),
            refresh_token: Some("refresh".into()),
            enterprise_id: None,
        }
    }

    #[test]
    fn identity_switch_changes_endpoint_domain_and_user_agent() {
        let desktop = hosts(Realm::Cn, Outbound::Desktop);
        let vscode = hosts(Realm::Cn, Outbound::Vscode);
        assert_eq!(desktop.chat, "copilot.tencent.com");
        assert_eq!(vscode.chat, "www.workbuddy.cn");

        let headers = chat_headers("wb-cn", Realm::Cn, Outbound::Desktop, &desktop, &account(), 1);
        let find = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(find("X-Domain").as_deref(), Some("copilot.tencent.com"));
        assert_eq!(
            find("User-Agent").as_deref(),
            Some("WorkBuddy/5.5.6 WorkBuddy/5.5.6 CLI/2.137.1")
        );
        assert_eq!(find("X-IDE-Type").as_deref(), Some("WorkBuddy"));
        assert_eq!(find("Authorization").as_deref(), Some("Bearer token"));

        let cli = chat_headers("wb-cn", Realm::Cn, Outbound::Cli, &desktop, &account(), 1);
        assert!(cli
            .iter()
            .any(|(key, value)| key == "X-IDE-Type" && value == "CLI"));
    }

    #[test]
    fn fingerprints_are_stable_and_channel_scoped() {
        let hosts = hosts(Realm::Cn, Outbound::Desktop);
        let first = chat_headers("wb-cn", Realm::Cn, Outbound::Desktop, &hosts, &account(), 7);
        let second = chat_headers("wb-cn", Realm::Cn, Outbound::Desktop, &hosts, &account(), 8);
        let get = |headers: &Vec<(String, String)>, key: &str| {
            headers
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
                .unwrap()
        };
        assert_eq!(get(&first, "X-Machine-ID"), get(&second, "X-Machine-ID"));
        assert_ne!(get(&first, "X-Request-ID"), get(&second, "X-Request-ID"));

        let other_channel = chat_headers(
            "wb-intl",
            Realm::Cn,
            Outbound::Desktop,
            &hosts,
            &account(),
            7,
        );
        assert_ne!(
            get(&first, "X-Machine-ID"),
            get(&other_channel, "X-Machine-ID")
        );
    }

    #[test]
    fn billing_headers_declare_missing_enterprise_explicitly() {
        let headers = billing_headers(
            &hosts(Realm::Cn, Outbound::Desktop),
            &account(),
            Some("web"),
        );
        assert!(headers
            .iter()
            .any(|(key, value)| key == "X-No-Enterprise-Id" && value == "1"));
        assert!(headers
            .iter()
            .any(|(key, value)| key == "X-Client-Platform" && value == "web"));
    }

    #[test]
    fn enterprise_account_gets_tenant_headers_instead() {
        let mut account = account();
        account.enterprise_id = Some("ent-9".into());
        let headers = chat_headers(
            "wb-cn",
            Realm::Cn,
            Outbound::Desktop,
            &hosts(Realm::Cn, Outbound::Desktop),
            &account,
            1,
        );
        assert!(headers
            .iter()
            .any(|(key, value)| key == "X-Enterprise-Id" && value == "ent-9"));
        assert!(headers
            .iter()
            .any(|(key, value)| key == "X-Tenant-Id" && value == "ent-9"));
    }
}
