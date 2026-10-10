//! 网关配置：`<app_data>/gateway/config.json`。
//!
//! 渠道/账号/策略都在这里声明，面板改配置时写回同一份文件。
//! 首次启动生成默认配置与一份带示例的 `config.example.json`（不覆盖已有文件）。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::gateway::errors::GatewayError;

/// 监听端口默认 8790：8788 已被常见的 workbuddy 反代实现占用。
pub const DEFAULT_PORT: u16 = 8790;
/// 端口被占用时的回退扫描范围。
pub const PORT_SCAN_RANGE: u16 = 11;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 是否随岛启动 HTTP 服务（关掉后只有面板可用）。
    pub enabled: bool,
    pub port: u16,
    /// 下游 API Key；留空表示不校验（只绑回环，等价于"本机免鉴权"）。
    pub api_keys: Vec<String>,
    pub channels: Vec<ChannelConfig>,
    pub accounts: Vec<AccountConfig>,
    pub affinity: AffinityConfig,
    pub cooldowns: CooldownConfig,
    pub concurrency: ConcurrencyConfig,
    pub scoring: ScoringConfig,
    /// 对外模型别名：`{"ds-flash": "deepseek-v4.1-flash"}`。
    pub aliases: std::collections::HashMap<String, String>,
    /// 人工覆盖：改优先级/权重/上限、换上游名、临时禁用某条候选。
    pub routes: Vec<RouteOverride>,
    /// 渠道目录缓存 TTL（秒）。
    pub catalog_ttl_seconds: u64,
}

/// 开箱即用的渠道：两个 workbuddy（国内/国际），默认启用。
///
/// 没有渠道的网关是"面板打开也没得可点"——所以默认配置必须带上它们，
/// 用户只需要点登录、走一次授权就有账号了。
pub fn default_channels() -> Vec<ChannelConfig> {
    vec![
        ChannelConfig {
            id: "wb-cn".into(),
            adapter: "workbuddy".into(),
            display_name: "WorkBuddy 国内版".into(),
            enabled: true,
            priority: 100,
            weight: 5,
            options: serde_json::json!({ "realm": "cn", "outbound_identity": "workbuddy" }),
        },
        ChannelConfig {
            id: "wb-intl".into(),
            adapter: "workbuddy".into(),
            display_name: "WorkBuddy 国际版".into(),
            enabled: true,
            priority: 100,
            weight: 5,
            options: serde_json::json!({ "realm": "intl", "outbound_identity": "workbuddy" }),
        },
    ]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            port: DEFAULT_PORT,
            api_keys: Vec::new(),
            channels: default_channels(),
            accounts: Vec::new(),
            affinity: AffinityConfig::default(),
            cooldowns: CooldownConfig::default(),
            concurrency: ConcurrencyConfig::default(),
            scoring: ScoringConfig::default(),
            aliases: std::collections::HashMap::new(),
            routes: Vec::new(),
            catalog_ttl_seconds: 300,
        }
    }
}

/// 一条「对外模型 → 渠道」的人工覆盖。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RouteOverride {
    pub model: String,
    pub channel_id: String,
    /// 目录里有这条时用它改上游名；目录里没有时**必须**给，否则视为配置写错。
    pub upstream_model: Option<String>,
    pub priority: Option<u32>,
    pub weight: Option<u32>,
    pub enabled: Option<bool>,
}

/// 选号打分的权重：量纲统一在"权重 1.0 ≈ 一次完整缓存命中的偏好"附近。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ScoringConfig {
    /// 与上次使用的账号/渠道不同时的惩罚（换渠道必丢缓存）。
    pub switch_penalty: f64,
    /// 命中亲和绑定的加成。
    pub affinity_bonus: f64,
    /// 在飞负载的惩罚系数。
    pub load_weight: f64,
    /// 免费档的加成（配 0 关闭）。
    pub cost_bonus: f64,
}

impl Default for ScoringConfig {
    fn default() -> Self {
        Self {
            switch_penalty: 0.35,
            affinity_bonus: 0.5,
            load_weight: 0.3,
            cost_bonus: 0.1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelConfig {
    /// 渠道实例 id（账号联合主键的一半）：如 `wb-cn`、`qoder`。
    pub id: String,
    /// 适配器类型（注册中心按它建实例）：如 `workbuddy`、`fake`。
    pub adapter: String,
    pub display_name: String,
    pub enabled: bool,
    /// 成本/质量档：越大越先被选中（免费档 > 付费档）。
    pub priority: u32,
    /// 同档内的容量份额。
    pub weight: u32,
    /// 适配器私有参数（域名、出站身份、失败注入等）。
    pub options: serde_json::Value,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            adapter: String::new(),
            display_name: String::new(),
            enabled: true,
            priority: 100,
            weight: 1,
            options: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountConfig {
    pub channel_id: String,
    pub uid: String,
    pub nickname: String,
    pub enabled: bool,
    pub priority_override: Option<u32>,
    pub weight_override: Option<u32>,
    pub max_inflight: Option<u32>,
    pub proxy_slot: Option<String>,
}

impl Default for AccountConfig {
    fn default() -> Self {
        Self {
            channel_id: String::new(),
            uid: String::new(),
            nickname: String::new(),
            enabled: true,
            priority_override: None,
            weight_override: None,
            max_inflight: None,
            proxy_slot: None,
        }
    }
}

/// 会话亲和：绑定到账号级，保上游前缀缓存命中。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AffinityConfig {
    pub enabled: bool,
    /// prefer / strict / off
    pub mode: String,
    pub ttl_seconds: u64,
    pub max_entries: usize,
}

impl Default for AffinityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: "prefer".to_string(),
            ttl_seconds: 1800,
            max_entries: 20_000,
        }
    }
}

/// 冷却与退避参数，语义见计划 §6.4。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CooldownConfig {
    pub auth_secs: u64,
    pub soft_base_secs: u64,
    pub soft_cap_secs: u64,
    pub breaker_after: u32,
    pub breaker_base_secs: u64,
    pub breaker_cap_secs: u64,
}

impl Default for CooldownConfig {
    fn default() -> Self {
        Self {
            auth_secs: 60,
            soft_base_secs: 600,
            soft_cap_secs: 7_200,
            breaker_after: 3,
            breaker_base_secs: 1_800,
            breaker_cap_secs: 21_600,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConcurrencyConfig {
    pub global: u32,
    pub per_account: u32,
}

impl Default for ConcurrencyConfig {
    fn default() -> Self {
        Self {
            global: 32,
            per_account: 3,
        }
    }
}

impl Config {
    /// 读取配置；不存在时写一份默认配置 + 示例文件。
    pub fn load_or_create(dir: &Path) -> Result<Self, GatewayError> {
        let path = dir.join("config.json");
        if !path.exists() {
            let config = Config::default();
            config.save(dir)?;
            let _ = std::fs::write(dir.join("config.example.json"), EXAMPLE_JSON);
            return Ok(config);
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|error| GatewayError::internal(format!("读取配置失败：{error}")))?;
        let mut config: Self = serde_json::from_str(&text)
            .map_err(|error| GatewayError::internal(format!("配置不是合法 JSON：{error}")))?;
        // 老配置文件（或手工清空过 channels）自动补上默认渠道，否则面板无从下手。
        if config.channels.is_empty() {
            config.channels = default_channels();
            let _ = config.save(dir);
            eprintln!("[gateway] 配置里没有渠道，已写入默认渠道（workbuddy 国内/国际）");
        }
        // 清掉历史遗留的"假渠道"：那只是链路自测用的，不该出现在用户的渠道列表里。
        let before = config.channels.len();
        config.channels.retain(|channel| channel.adapter != "fake");
        if config.channels.len() != before {
            let _ = config.save(dir);
            eprintln!("[gateway] 已移除链路自测用的假渠道");
        }
        Ok(config)
    }

    pub fn save(&self, dir: &Path) -> Result<(), GatewayError> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| GatewayError::internal(format!("配置序列化失败：{error}")))?;
        std::fs::write(dir.join("config.json"), text)
            .map_err(|error| GatewayError::internal(format!("写入配置失败：{error}")))
    }

    pub fn channel(&self, id: &str) -> Option<&ChannelConfig> {
        self.channels.iter().find(|channel| channel.id == id)
    }
}

/// 首次启动落到磁盘的示例：一个假渠道 + 一份 workbuddy 模板，照抄改名即可用。
const EXAMPLE_JSON: &str = r#"{
  "//": "复制到 config.json 的 channels/accounts 里即可生效。fake 渠道不需要凭据，用来验证链路。",
  "channels": [
    { "id": "fake", "adapter": "fake", "display_name": "假渠道（链路自测）", "enabled": true, "priority": 100, "weight": 1,
      "options": { "model": "fake-model", "fail_first": 0, "fail_status": 429 } },
    { "id": "wb-cn", "adapter": "workbuddy", "display_name": "WorkBuddy 国内版", "enabled": false, "priority": 100, "weight": 5,
      "options": { "realm": "cn", "outbound_identity": "workbuddy" } },
    { "id": "wb-intl", "adapter": "workbuddy", "display_name": "WorkBuddy 国际版", "enabled": false, "priority": 100, "weight": 5,
      "options": { "realm": "intl", "outbound_identity": "workbuddy" } }
  ],
  "accounts": [
    { "channel_id": "fake", "uid": "fake-account-1", "nickname": "假账号", "enabled": true }
  ]
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_produces_default_config_and_example() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = Config::load_or_create(dir.path()).expect("默认配置");
        assert_eq!(config.port, DEFAULT_PORT);
        assert!(dir.path().join("config.json").exists());
        assert!(dir.path().join("config.example.json").exists());

        // 已存在时不再覆盖：改动必须能留存。
        let mut edited = Config::load_or_create(dir.path()).expect("再次读取");
        edited.port = 9001;
        edited.save(dir.path()).expect("写回");
        let reloaded = Config::load_or_create(dir.path()).expect("第三次读取");
        assert_eq!(reloaded.port, 9001);
    }

    #[test]
    fn partial_json_falls_back_to_defaults() {
        let config: Config = serde_json::from_str(r#"{"port": 8899}"#).expect("部分字段");
        assert_eq!(config.port, 8899);
        assert_eq!(config.concurrency.global, 32);
        assert_eq!(config.cooldowns.auth_secs, 60);
        assert!(config.enabled);
    }
}
