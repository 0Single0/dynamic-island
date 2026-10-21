//! 存储层：SQLite（`<app_data>/gateway/state.db`）。
//!
//! - 账号与凭据在这里落库（凭据走 [`crate::gateway::security::envelope`] 加密）；
//! - 用量走"单写线程 + 批量事务"，不阻塞请求路径；
//! - 亲和表结构与调度器共用在 P1 落地，这里先建表。
//!
//! 除用量外的读写都是本地 WAL 上的毫秒级操作，直接同步调用；
//! 只有用量是每请求一次的热路径，才单独走通道。

use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::agent::secrets::SecretStore;
use crate::gateway::{
    channels::Credential,
    errors::GatewayError,
    protocol::cir::Usage,
    security::envelope,
    timeutil::{civil_to_epoch_ms, now_ms},
};

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS accounts (
    channel_id        TEXT    NOT NULL,
    uid               TEXT    NOT NULL,
    nickname          TEXT    NOT NULL DEFAULT '',
    enabled           INTEGER NOT NULL DEFAULT 1,
    priority_override INTEGER,
    weight_override   INTEGER,
    max_inflight      INTEGER,
    proxy_slot        TEXT,
    credential        TEXT,
    added_at          INTEGER NOT NULL,
    source            TEXT    NOT NULL DEFAULT 'manual',
    -- JSON 数组：这个账号只服务哪些**对外模型名**（空/NULL = 不限制）。
    model_allow       TEXT,
    -- JSON 对象：对外名到该账号上游名的映射（账号级改名）。
    model_rename      TEXT,
    PRIMARY KEY (channel_id, uid)
);
CREATE TABLE IF NOT EXISTS usage (
    ts                INTEGER NOT NULL,
    model             TEXT    NOT NULL,
    channel_id        TEXT    NOT NULL,
    account_uid       TEXT    NOT NULL,
    status            TEXT    NOT NULL,
    stream            INTEGER NOT NULL,
    attempts          INTEGER NOT NULL,
    prompt_tokens     INTEGER NOT NULL,
    completion_tokens INTEGER NOT NULL,
    reasoning_tokens  INTEGER NOT NULL,
    cached_tokens     INTEGER NOT NULL,
    total_tokens      INTEGER NOT NULL,
    credit            REAL,
    ttft_ms           INTEGER,
    total_ms          INTEGER NOT NULL,
    effort            TEXT,
    estimated         INTEGER NOT NULL DEFAULT 0,
    upstream_model    TEXT,
    error_detail      TEXT
);
CREATE INDEX IF NOT EXISTS usage_ts ON usage(ts);
CREATE INDEX IF NOT EXISTS usage_channel ON usage(channel_id, ts);
CREATE TABLE IF NOT EXISTS affinity (
    key            TEXT    PRIMARY KEY,
    channel_id     TEXT    NOT NULL,
    uid            TEXT    NOT NULL,
    upstream_model TEXT    NOT NULL,
    expires_at     INTEGER NOT NULL,
    hits           INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS affinity_expires ON affinity(expires_at);
";

/// 模型白名单 / 改名的落库编码。
///
/// 存 JSON 而不是逗号分隔：模型名里可能出现逗号或冒号（中转站命名很随意），
/// 自造的定界符迟早会被一个奇怪的模型名撑破。
fn encode_string_list(values: &[String]) -> Option<String> {
    (!values.is_empty()).then(|| serde_json::to_string(values).unwrap_or_default())
}

fn encode_string_map(values: &std::collections::BTreeMap<String, String>) -> Option<String> {
    (!values.is_empty()).then(|| serde_json::to_string(values).unwrap_or_default())
}

/// 解码失败一律当"没有配置"：一条读不出来的老数据不该让整个账号列表打不开。
fn decode_string_list(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|text| serde_json::from_str::<Vec<String>>(text).ok())
        .unwrap_or_default()
}

fn decode_string_map(raw: Option<&str>) -> std::collections::BTreeMap<String, String> {
    raw.and_then(|text| serde_json::from_str::<std::collections::BTreeMap<String, String>>(text).ok())
        .unwrap_or_default()
}

/// 账号落库记录（含解密后的凭据）。
#[derive(Debug, Clone, Serialize)]
pub struct AccountRecord {
    pub channel_id: String,
    pub uid: String,
    pub nickname: String,
    pub enabled: bool,
    pub priority_override: Option<u32>,
    pub weight_override: Option<u32>,
    pub max_inflight: Option<u32>,
    pub proxy_slot: Option<String>,
    pub added_at: u64,
    pub source: String,
    pub credential: Option<Credential>,
    /// 这个账号只服务哪些**对外模型名**。空 = 不限制（渠道有什么就用什么）。
    pub model_allow: Vec<String>,
    /// 账号级模型改名：`对外名 -> 该账号的上游名`。
    ///
    /// 与渠道级 `config.routes[].upstream_model` 的区别是**作用域**：同一渠道的两个账号
    /// 可以把同一个对外模型映射到不同的上游名（Trae 的 `DeepSeek-V4-Flash-Official`
    /// 与别处的 `deepseek-v4.1-flash` 就是一例）。
    pub model_rename: std::collections::BTreeMap<String, String>,
}

/// 一条请求的用量记录。
#[derive(Debug, Clone, Serialize)]
pub struct UsageRecord {
    pub ts: u64,
    pub model: String,
    pub channel_id: String,
    pub account_uid: String,
    /// completed / client_aborted / failed
    pub status: String,
    pub stream: bool,
    pub attempts: usize,
    pub usage: Usage,
    /// 上游没给 usage、由本地估算时为真。
    pub estimated: bool,
    pub ttft_ms: Option<u64>,
    pub total_ms: u64,
    pub effort: Option<String>,
    /// 出站实际用的上游模型名（对外名可能不同）。
    pub upstream_model: Option<String>,
    /// 失败时上游返回的原文（面板要能点开看"上游到底报了什么"）。
    pub error_detail: Option<String>,
}

pub struct Storage {
    path: PathBuf,
    master_key: [u8; 32],
    conn: Mutex<Connection>,
    usage_tx: mpsc::Sender<UsageRecord>,
}

impl Storage {
    /// `hours = 0` 表示"由调用方决定起点"（"今日"），返回 None；否则返回 now - hours。
    fn since_ms(hours: u64) -> Option<i64> {
        if hours == 0 {
            None
        } else {
            Some(now_ms().saturating_sub(hours * 3600 * 1000) as i64)
        }
    }

    /// 本地今天 00:00 的纪元毫秒（"今日"筛选的起点）。
    fn local_today_start_ms() -> i64 {
        let (year, month, day, _) = crate::gateway::timeutil::local_date_hour();
        civil_to_epoch_ms(
            year as i64,
            month as i64,
            day as i64,
            0,
            0,
            0,
            Self::local_utc_offset_secs(),
        ) as i64
    }

    /// 本机 UTC 偏移（秒）：用"本地小时"与"UTC 小时"的差反推，夹回 ±12h。
    fn local_utc_offset_secs() -> i64 {
        let now = now_ms() as i64;
        let (_, _, _, hour) = crate::gateway::timeutil::local_date_hour();
        let utc_hour = ((now / 3_600_000) % 24) as i64;
        let mut offset = (hour as i64 - utc_hour) * 3600;
        if offset > 12 * 3600 {
            offset -= 24 * 3600;
        }
        if offset < -12 * 3600 {
            offset += 24 * 3600;
        }
        offset
    }

    /// 打开（或新建）库并启动用量写入线程。
    pub fn open(dir: &Path, secrets: &dyn SecretStore) -> Result<Arc<Self>, GatewayError> {
        let path = dir.join("state.db");
        let conn = Connection::open(&path)
            .map_err(|error| GatewayError::internal(format!("打开数据库失败：{error}")))?;
        conn.execute_batch(SCHEMA)
            .map_err(|error| GatewayError::internal(format!("建表失败：{error}")))?;
        // 轻量迁移：老库补列（列已存在时 ALTER 会报错，忽略即可）。
        for statement in [
            "ALTER TABLE usage ADD COLUMN upstream_model TEXT",
            "ALTER TABLE usage ADD COLUMN error_detail TEXT",
            // 账号级模型限制 / 改名（老库没有这两列）。
            "ALTER TABLE accounts ADD COLUMN model_allow TEXT",
            "ALTER TABLE accounts ADD COLUMN model_rename TEXT",
        ] {
            let _ = conn.execute(statement, []);
        }
        let master_key = envelope::master_key(secrets).map_err(GatewayError::internal)?;

        // 用量写入线程独占一条连接，避免和账号读写抢同一把锁。
        let (usage_tx, usage_rx) = mpsc::channel::<UsageRecord>();
        let writer_path = path.clone();
        std::thread::Builder::new()
            .name("gateway-usage-writer".into())
            .spawn(move || usage_writer_loop(writer_path, usage_rx))
            .map_err(|error| GatewayError::internal(format!("启动用量写入线程失败：{error}")))?;

        Ok(Arc::new(Self {
            path,
            master_key,
            conn: Mutex::new(conn),
            usage_tx,
        }))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn with_conn<T>(&self, action: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, GatewayError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| GatewayError::internal("数据库连接锁被污染"))?;
        action(&conn).map_err(|error| GatewayError::internal(format!("数据库操作失败：{error}")))
    }

    pub fn list_accounts(&self) -> Result<Vec<AccountRecord>, GatewayError> {
        let rows = self.with_conn(|conn| {
            let mut statement = conn.prepare(
                "SELECT channel_id, uid, nickname, enabled, priority_override, weight_override,
                        max_inflight, proxy_slot, credential, added_at, source,
                        model_allow, model_rename
                 FROM accounts ORDER BY channel_id, added_at",
            )?;
            let mapped = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                ))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    channel_id,
                    uid,
                    nickname,
                    enabled,
                    priority_override,
                    weight_override,
                    max_inflight,
                    proxy_slot,
                    credential,
                    added_at,
                    source,
                    model_allow,
                    model_rename,
                )| AccountRecord {
                    channel_id,
                    uid,
                    nickname,
                    enabled: enabled != 0,
                    priority_override: priority_override.map(|value| value as u32),
                    weight_override: weight_override.map(|value| value as u32),
                    max_inflight: max_inflight.map(|value| value as u32),
                    proxy_slot,
                    added_at: added_at as u64,
                    source,
                    credential: credential.and_then(|sealed| self.open_credential(&sealed)),
                    model_allow: decode_string_list(model_allow.as_deref()),
                    model_rename: decode_string_map(model_rename.as_deref()),
                },
            )
            .collect())
    }

    pub fn account_count(&self) -> Result<usize, GatewayError> {
        let count: Option<i64> = self.with_conn(|conn| {
            conn.query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
                .optional()
        })?;
        Ok(count.unwrap_or(0) as usize)
    }

    pub fn upsert_account(&self, record: &AccountRecord) -> Result<(), GatewayError> {
        let sealed = match &record.credential {
            Some(credential) => Some(self.seal_credential(credential)?),
            None => None,
        };
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO accounts (channel_id, uid, nickname, enabled, priority_override,
                                       weight_override, max_inflight, proxy_slot, credential, added_at, source,
                                       model_allow, model_rename)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(channel_id, uid) DO UPDATE SET
                    nickname = excluded.nickname,
                    enabled = excluded.enabled,
                    priority_override = excluded.priority_override,
                    weight_override = excluded.weight_override,
                    max_inflight = excluded.max_inflight,
                    proxy_slot = excluded.proxy_slot,
                    model_allow = excluded.model_allow,
                    model_rename = excluded.model_rename,
                    credential = COALESCE(excluded.credential, accounts.credential)",
                params![
                    record.channel_id,
                    record.uid,
                    record.nickname,
                    record.enabled as i64,
                    record.priority_override.map(|value| value as i64),
                    record.weight_override.map(|value| value as i64),
                    record.max_inflight.map(|value| value as i64),
                    record.proxy_slot,
                    sealed,
                    record.added_at as i64,
                    record.source,
                    encode_string_list(&record.model_allow),
                    encode_string_map(&record.model_rename),
                ],
            )?;
            Ok(())
        })
    }

    /// 只更新凭据（刷新链路的热路径）。`None` 表示清空（账号需要重新登录）。
    pub fn set_credential(
        &self,
        channel_id: &str,
        uid: &str,
        credential: Option<&Credential>,
    ) -> Result<(), GatewayError> {
        let sealed = match credential {
            Some(credential) => Some(self.seal_credential(credential)?),
            None => None,
        };
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE accounts SET credential = ?3 WHERE channel_id = ?1 AND uid = ?2",
                params![channel_id, uid, sealed],
            )?;
            Ok(())
        })
    }

    /// 设置账号的并发上限（None = 回落到渠道/全局默认）。写入 accounts 表。
    pub fn set_account_max_inflight(
        &self,
        channel_id: &str,
        uid: &str,
        max_inflight: Option<u32>,
    ) -> Result<(), GatewayError> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE accounts SET max_inflight = ?3 WHERE channel_id = ?1 AND uid = ?2",
                params![channel_id, uid, max_inflight.map(|value| value as i64)],
            )?;
            Ok(())
        })
    }

    /// 改账号的**可变画像**：昵称 / 优先级 / 权重 / 并发上限。
    ///
    /// 刻意不走 `upsert_account`：那条语句要求把整条记录（含凭据）重新给全，
    /// 少给一个字段就会把凭据一起清掉。这里只 UPDATE 这几列，凭据与运行态一概不碰 ——
    /// 「编辑账号」不该有"顺手把登录态弄丢"的副作用。
    ///
    /// `priority / weight / max_inflight` 为 `None` = 清掉覆盖、回落到渠道/全局默认。
    #[allow(clippy::too_many_arguments)]
    pub fn set_account_profile(
        &self,
        channel_id: &str,
        uid: &str,
        nickname: Option<&str>,
        priority: Option<u32>,
        weight: Option<u32>,
        max_inflight: Option<u32>,
    ) -> Result<bool, GatewayError> {
        let changed = self.with_conn(|conn| {
            let mut sets: Vec<&str> = Vec::new();
            if nickname.is_some() {
                sets.push("nickname = ?3");
            }
            sets.push("priority_override = ?4");
            sets.push("weight_override = ?5");
            sets.push("max_inflight = ?6");
            let sql = format!(
                "UPDATE accounts SET {} WHERE channel_id = ?1 AND uid = ?2",
                sets.join(", ")
            );
            conn.execute(
                &sql,
                params![
                    channel_id,
                    uid,
                    nickname,
                    priority.map(|value| value as i64),
                    weight.map(|value| value as i64),
                    max_inflight.map(|value| value as i64),
                ],
            )
        })?;
        Ok(changed > 0)
    }

    /// 改账号的模型清单：白名单（空 = 不限）+ 账号级改名。
    ///
    /// 只碰 `model_allow` / `model_rename` 两列 —— 这两样是"调度画像"，改它们
    /// 不该影响凭据、额度或冷却。
    pub fn set_account_models(
        &self,
        channel_id: &str,
        uid: &str,
        allow: &[String],
        rename: &std::collections::BTreeMap<String, String>,
    ) -> Result<bool, GatewayError> {
        let changed = self.with_conn(|conn| {
            conn.execute(
                "UPDATE accounts SET model_allow = ?3, model_rename = ?4 WHERE channel_id = ?1 AND uid = ?2",
                params![
                    channel_id,
                    uid,
                    encode_string_list(allow),
                    encode_string_map(rename),
                ],
            )
        })?;
        Ok(changed > 0)
    }

    pub fn set_account_enabled(
        &self,
        channel_id: &str,
        uid: &str,
        enabled: bool,
    ) -> Result<(), GatewayError> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE accounts SET enabled = ?3 WHERE channel_id = ?1 AND uid = ?2",
                params![channel_id, uid, enabled as i64],
            )?;
            Ok(())
        })
    }

    pub fn delete_account(&self, channel_id: &str, uid: &str) -> Result<bool, GatewayError> {
        let changed = self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM accounts WHERE channel_id = ?1 AND uid = ?2",
                params![channel_id, uid],
            )
        })?;
        Ok(changed > 0)
    }

    /// 首次运行：把配置文件里声明的账号灌进库（之后库是唯一事实来源）。
    pub fn seed_from_config(&self, config: &crate::gateway::config::Config) -> Result<usize, GatewayError> {
        if self.account_count()? > 0 {
            return Ok(0);
        }
        let now = now_ms();
        for account in &config.accounts {
            self.upsert_account(&AccountRecord {
                channel_id: account.channel_id.clone(),
                uid: account.uid.clone(),
                nickname: account.nickname.clone(),
                enabled: account.enabled,
                priority_override: account.priority_override,
                weight_override: account.weight_override,
                max_inflight: account.max_inflight,
                proxy_slot: account.proxy_slot.clone(),
                added_at: now,
                source: "config".into(),
                credential: None,
                model_allow: Vec::new(),
                model_rename: std::collections::BTreeMap::new(),
            })?;
        }
        Ok(config.accounts.len())
    }

    /// 记一条用量（非阻塞，写入线程批量落库）。
    pub fn record_usage(&self, record: UsageRecord) {
        // 接收端消失说明写入线程已退出（进程收尾），此时丢弃即可，不影响请求。
        let _ = self.usage_tx.send(record);
    }

    /// 近 `hours` 小时的用量汇总：总量 + 按渠道 + 按模型 + 缓存命中率。
    pub fn usage_summary(&self, hours: u64) -> Result<serde_json::Value, GatewayError> {
        // hours = 0 是特殊语义："今日"（本地 00:00 起），面板的日期筛选要用。
        let since = match Self::since_ms(hours) {
            Some(value) => value,
            None => Self::local_today_start_ms(),
        };
        self.usage_summary_range(since, now_ms() as i64)
    }

    /// 指定毫秒区间的汇总（日期选择器用）。
    pub fn usage_summary_range(&self, from_ms: i64, to_ms: i64) -> Result<serde_json::Value, GatewayError> {
        let since = from_ms;
        let until = to_ms.max(from_ms + 1);
        let totals = self.with_conn(|conn| {
            conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0),
                        COALESCE(SUM(cached_tokens),0), COALESCE(SUM(total_tokens),0),
                        COALESCE(SUM(credit),0), COALESCE(AVG(ttft_ms),0)
                 FROM usage WHERE ts >= ?1 AND ts < ?2 AND model NOT LIKE '(未进入调度%)'",
                params![since, until],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, f64>(5)?,
                        row.get::<_, f64>(6)?,
                    ))
                },
            )
        })?;
        let by_channel = self.grouped_usage_range("channel_id", since, until)?;
        let by_model = self.grouped_usage_range("model", since, until)?;
        let (requests, prompt, completion, cached, total, credit, ttft) = totals;
        let hit_rate = if prompt > 0 {
            cached as f64 / prompt as f64
        } else {
            0.0
        };
        Ok(serde_json::json!({
            "window_hours": ((until - since) / 3_600_000).max(1),
            "requests": requests,
            "prompt_tokens": prompt,
            "completion_tokens": completion,
            "cached_tokens": cached,
            "total_tokens": total,
            "credit": credit,
            "avg_ttft_ms": ttft.round() as i64,
            "cache_hit_rate": (hit_rate * 1000.0).round() / 1000.0,
            "by_channel": by_channel,
            "by_model": by_model,
        }))
    }

    fn grouped_usage(&self, column: &str, since: i64) -> Result<Vec<serde_json::Value>, GatewayError> {
        self.grouped_usage_range(column, since, now_ms() as i64 + 1)
    }

    /// 分组统计（指定区间）：排除"未进入调度"行（它们没有真实的模型/渠道归属）。
    fn grouped_usage_range(
        &self,
        column: &str,
        since: i64,
        until: i64,
    ) -> Result<Vec<serde_json::Value>, GatewayError> {
        // column 只来自本函数内部的两个字面量，不存在注入面。
        let sql = format!(
            "SELECT {column}, COUNT(*), COALESCE(SUM(total_tokens),0), COALESCE(SUM(cached_tokens),0)
             FROM usage WHERE ts >= ?1 AND ts < ?2 AND {column} NOT LIKE '(未进入调度%'
             GROUP BY {column} ORDER BY 2 DESC LIMIT 50"
        );
        self.with_conn(|conn| {
            let mut statement = conn.prepare(&sql)?;
            let mapped = statement.query_map(params![since, until], |row| {
                Ok(serde_json::json!({
                    "key": row.get::<_, String>(0)?,
                    "requests": row.get::<_, i64>(1)?,
                    "total_tokens": row.get::<_, i64>(2)?,
                    "cached_tokens": row.get::<_, i64>(3)?,
                }))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// 落一条会话亲和绑定（只在绑定点变化时写：命中续期只改内存）。
    pub fn save_affinity(
        &self,
        key: &str,
        binding: &crate::gateway::core::affinity::Binding,
    ) -> Result<(), GatewayError> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO affinity (key, channel_id, uid, upstream_model, expires_at, hits)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(key) DO UPDATE SET
                    channel_id = excluded.channel_id,
                    uid = excluded.uid,
                    upstream_model = excluded.upstream_model,
                    expires_at = excluded.expires_at,
                    hits = excluded.hits",
                params![
                    key,
                    binding.channel_id,
                    binding.uid,
                    binding.upstream_model,
                    binding.expires_at_ms as i64,
                    binding.hits as i64,
                ],
            )?;
            Ok(())
        })
    }

    pub fn load_affinity(
        &self,
    ) -> Result<Vec<(String, crate::gateway::core::affinity::Binding)>, GatewayError> {
        let rows = self.with_conn(|conn| {
            let mut statement = conn.prepare(
                "SELECT key, channel_id, uid, upstream_model, expires_at, hits
                 FROM affinity WHERE expires_at > ?1",
            )?;
            let mapped = statement.query_map(params![now_ms() as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        Ok(rows
            .into_iter()
            .map(
                |(key, channel_id, uid, upstream_model, expires_at, hits)| {
                    (
                        key,
                        crate::gateway::core::affinity::Binding {
                            channel_id,
                            uid,
                            upstream_model,
                            expires_at_ms: expires_at as u64,
                            hits: hits as u32,
                        },
                    )
                },
            )
            .collect())
    }

    pub fn delete_affinity(&self, key: &str) -> Result<(), GatewayError> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM affinity WHERE key = ?1", params![key])?;
            Ok(())
        })
    }

    /// 清掉过期的亲和绑定（定时器调用）。
    pub fn prune_affinity(&self) -> Result<usize, GatewayError> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM affinity WHERE expires_at <= ?1", params![now_ms() as i64])
        })
    }

    /// 按小时分桶的时间序列（折线图用）。`model` 为 None 表示不筛选模型。
    pub fn usage_series(
        &self,
        hours: u64,
        model: Option<&str>,
        bucket_hours: u64,
    ) -> Result<Vec<serde_json::Value>, GatewayError> {
        // hours = 0 同 summary："今日"。
        let since = match Self::since_ms(hours) {
            Some(value) => value,
            None => Self::local_today_start_ms(),
        };
        self.usage_series_range(since, now_ms() as i64, model, bucket_hours)
    }

    /// 指定毫秒区间的趋势序列（日期选择器用）。
    pub fn usage_series_range(
        &self,
        from_ms: i64,
        to_ms: i64,
        model: Option<&str>,
        bucket_hours: u64,
    ) -> Result<Vec<serde_json::Value>, GatewayError> {
        let since = from_ms;
        let until = to_ms.max(from_ms + 1);
        let bucket_ms = bucket_hours.max(1) as i64 * 3600 * 1000;
        self.with_conn(|conn| {
            let mut statement = conn.prepare(
                "SELECT (ts / ?1) * ?1 AS bucket,
                        COUNT(*), COALESCE(SUM(total_tokens),0),
                        COALESCE(SUM(cached_tokens),0), COALESCE(SUM(completion_tokens),0),
                        COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(credit),0)
                 FROM usage
                 WHERE ts >= ?2 AND ts < ?4 AND model NOT LIKE '(未进入调度%)' AND (?3 IS NULL OR model = ?3)
                 GROUP BY bucket ORDER BY bucket",
            )?;
            let mapped = statement.query_map(params![bucket_ms, since, model, until], |row| {
                Ok(serde_json::json!({
                    "ts": row.get::<_, i64>(0)?,
                    "requests": row.get::<_, i64>(1)?,
                    "total_tokens": row.get::<_, i64>(2)?,
                    "cached_tokens": row.get::<_, i64>(3)?,
                    "completion_tokens": row.get::<_, i64>(4)?,
                    "prompt_tokens": row.get::<_, i64>(5)?,
                    "credit": row.get::<_, f64>(6)?,
                }))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// 最近的请求明细（日志表格用）：带上游模型、错误原文与全部用量字段。
    ///
    /// `limit = 0` 表示**全量**（日志页的分页在前端做，后端不再截断）。
    pub fn recent_usage(&self, limit: u32) -> Result<Vec<serde_json::Value>, GatewayError> {
        let sql = if limit > 0 {
            "SELECT ts, model, COALESCE(upstream_model, model), channel_id, account_uid,
                    status, stream, attempts, prompt_tokens, completion_tokens,
                    reasoning_tokens, cached_tokens, total_tokens, credit,
                    ttft_ms, total_ms, effort, estimated, error_detail
             FROM usage ORDER BY ts DESC LIMIT ?1"
        } else {
            "SELECT ts, model, COALESCE(upstream_model, model), channel_id, account_uid,
                    status, stream, attempts, prompt_tokens, completion_tokens,
                    reasoning_tokens, cached_tokens, total_tokens, credit,
                    ttft_ms, total_ms, effort, estimated, error_detail
             FROM usage ORDER BY ts DESC"
        };
        self.with_conn(|conn| {
            let mut statement = conn.prepare(sql)?;
            let map_row = |row: &rusqlite::Row| -> rusqlite::Result<serde_json::Value> {
                let total_ms: i64 = row.get(15)?;
                let completion: i64 = row.get(9)?;
                let decode_ms = total_ms - row.get::<_, Option<i64>>(14)?.unwrap_or(total_ms);
                Ok(serde_json::json!({
                    "ts": row.get::<_, i64>(0)?,
                    "model": row.get::<_, String>(1)?,
                    "upstream_model": row.get::<_, String>(2)?,
                    "channel_id": row.get::<_, String>(3)?,
                    "account_uid": row.get::<_, String>(4)?,
                    "status": row.get::<_, String>(5)?,
                    "stream": row.get::<_, i64>(6)? != 0,
                    "attempts": row.get::<_, i64>(7)?,
                    "prompt_tokens": row.get::<_, i64>(8)?,
                    "completion_tokens": completion,
                    "reasoning_tokens": row.get::<_, i64>(10)?,
                    "cached_tokens": row.get::<_, i64>(11)?,
                    "total_tokens": row.get::<_, i64>(12)?,
                    "credit": row.get::<_, Option<f64>>(13)?,
                    "ttft_ms": row.get::<_, Option<i64>>(14)?,
                    "total_ms": total_ms,
                    // tokens/s 按"首字之后"的生成时间算，才反映真实解码速度。
                    "tokens_per_second": if decode_ms > 0 {
                        (completion as f64 / (decode_ms as f64 / 1000.0) * 10.0).round() / 10.0
                    } else { 0.0 },
                    "effort": row.get::<_, Option<String>>(16)?,
                    "estimated": row.get::<_, i64>(17)? != 0,
                    "error_detail": row.get::<_, Option<String>>(18)?,
                }))
            };
            let mut rows: Vec<serde_json::Value> = Vec::new();
            if limit > 0 {
                let mapped = statement.query_map(params![limit as i64], map_row)?;
                for row in mapped {
                    rows.push(row?);
                }
            } else {
                let mapped = statement.query_map([], map_row)?;
                for row in mapped {
                    rows.push(row?);
                }
            }
            Ok(rows)
        })
    }

    fn seal_credential(&self, credential: &Credential) -> Result<String, GatewayError> {        let json = serde_json::to_string(credential)
            .map_err(|error| GatewayError::internal(format!("凭据序列化失败：{error}")))?;
        envelope::seal(&self.master_key, &json).map_err(GatewayError::internal)
    }

    fn open_credential(&self, sealed: &str) -> Option<Credential> {
        let json = envelope::open(&self.master_key, sealed).ok()?;
        serde_json::from_str(&json).ok()
    }
}

/// 用量写入线程：批量提交，避免每条请求一次事务。
fn usage_writer_loop(path: PathBuf, receiver: mpsc::Receiver<UsageRecord>) {
    let Ok(mut conn) = Connection::open(&path) else {
        eprintln!("[gateway] 用量写入线程无法打开数据库，用量记录将被丢弃");
        return;
    };
    let mut pending: Vec<UsageRecord> = Vec::new();
    loop {
        // 先等一条（阻塞），再尽量把队列里已有的都收走。
        let first = match receiver.recv() {
            Ok(record) => record,
            Err(_) => {
                flush_usage(&mut conn, &mut pending);
                return;
            }
        };
        pending.push(first);
        while pending.len() < 128 {
            match receiver.try_recv() {
                Ok(record) => pending.push(record),
                Err(_) => break,
            }
        }
        flush_usage(&mut conn, &mut pending);
    }
}

fn flush_usage(conn: &mut Connection, pending: &mut Vec<UsageRecord>) {
    if pending.is_empty() {
        return;
    }
    let result = (|| -> rusqlite::Result<()> {
        let transaction = conn.transaction()?;
        {
            let mut statement = transaction.prepare(
                "INSERT INTO usage (ts, model, channel_id, account_uid, status, stream, attempts,
                                    prompt_tokens, completion_tokens, reasoning_tokens, cached_tokens,
                                    total_tokens, credit, ttft_ms, total_ms, effort, estimated,
                                    upstream_model, error_detail)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                         ?18, ?19)",
            )?;
            for record in pending.iter() {
                statement.execute(params![
                    record.ts as i64,
                    record.model,
                    record.channel_id,
                    record.account_uid,
                    record.status,
                    record.stream as i64,
                    record.attempts as i64,
                    record.usage.prompt_tokens as i64,
                    record.usage.completion_tokens as i64,
                    record.usage.reasoning_tokens as i64,
                    record.usage.cached_tokens as i64,
                    record.usage.total_tokens as i64,
                    record.usage.credit,
                    record.ttft_ms.map(|value| value as i64),
                    record.total_ms as i64,
                    record.effort,
                    record.estimated as i64,
                    record.upstream_model,
                    record.error_detail,
                ])?;
            }
        }
        transaction.commit()
    })();
    if let Err(error) = result {
        eprintln!("[gateway] 用量落库失败（丢弃 {} 条）：{error}", pending.len());
    }
    pending.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::config::{AccountConfig, ChannelConfig, Config};

    #[test]
    fn accounts_round_trip_with_encrypted_credentials() {
        let dir = tempfile::tempdir().expect("临时目录");
        let secrets = crate::gateway::security::tests_support::memory_store();
        let storage = Storage::open(dir.path(), secrets.as_ref()).expect("打开存储");

        // 凭据必须以密文落库：直接读文件里不该出现明文 token。
        let credential = Credential {
            access_token: "super-secret-token".into(),
            refresh_token: Some("refresh-1".into()),
            expires_at_ms: Some(1_700_000_000_000),
            uid: Some("uid-1".into()),
            nickname: Some("昵称".into()),
            enterprise_id: None,
            domain: Some("copilot.tencent.com".into()),
            ..Default::default()
        };
        storage
            .upsert_account(&AccountRecord {
                channel_id: "wb-cn".into(),
                uid: "uid-1".into(),
                nickname: "测试".into(),
                enabled: true,
                priority_override: None,
                weight_override: None,
                max_inflight: Some(2),
                proxy_slot: Some("slot-a".into()),
                added_at: 1,
                source: "oauth".into(),
                credential: Some(credential.clone()),
                model_allow: Vec::new(),
                model_rename: std::collections::BTreeMap::new(),
            })
            .expect("写入账号");

        let raw = std::fs::read(dir.path().join("state.db")).expect("读库文件");
        assert!(
            !raw.windows(19).any(|window| window == b"super-secret-token"),
            "明文 token 不该出现在库里"
        );

        let accounts = storage.list_accounts().expect("读账号");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].max_inflight, Some(2));
        assert_eq!(accounts[0].credential.as_ref(), Some(&credential));
    }

    #[test]
    fn upsert_keeps_existing_credential_when_none_is_given() {
        let dir = tempfile::tempdir().expect("临时目录");
        let secrets = crate::gateway::security::tests_support::memory_store();
        let storage = Storage::open(dir.path(), secrets.as_ref()).expect("打开存储");
        let record = AccountRecord {
            channel_id: "wb-cn".into(),
            uid: "uid-1".into(),
            nickname: "旧".into(),
            enabled: true,
            priority_override: None,
            weight_override: None,
            max_inflight: None,
            proxy_slot: None,
            added_at: 1,
            source: "oauth".into(),
            credential: Some(Credential {
                access_token: "token-1".into(),
                ..Credential::default()
            }),
            model_allow: Vec::new(),
            model_rename: std::collections::BTreeMap::new(),
        };
        storage.upsert_account(&record).expect("首次写入");

        storage
            .upsert_account(&AccountRecord {
                nickname: "新昵称".into(),
                credential: None,
                model_allow: Vec::new(),
                model_rename: std::collections::BTreeMap::new(),
                ..record
            })
            .expect("二次写入");
        let accounts = storage.list_accounts().expect("读账号");
        assert_eq!(accounts[0].nickname, "新昵称");
        assert_eq!(
            accounts[0].credential.as_ref().map(|c| c.access_token.as_str()),
            Some("token-1"),
            "没给新凭据时不该把旧的清掉"
        );
    }

    #[test]
    fn seeding_from_config_only_happens_once() {
        let dir = tempfile::tempdir().expect("临时目录");
        let secrets = crate::gateway::security::tests_support::memory_store();
        let storage = Storage::open(dir.path(), secrets.as_ref()).expect("打开存储");

        let mut config = Config::default();
        config.channels.push(ChannelConfig {
            id: "fake".into(),
            adapter: "fake".into(),
            ..ChannelConfig::default()
        });
        config.accounts.push(AccountConfig {
            channel_id: "fake".into(),
            uid: "u1".into(),
            ..AccountConfig::default()
        });
        assert_eq!(storage.seed_from_config(&config).expect("灌入"), 1);
        assert_eq!(storage.seed_from_config(&config).expect("再灌"), 0);
        assert_eq!(storage.account_count().expect("计数"), 1);
    }

    #[test]
    fn usage_is_written_and_aggregated() {
        let dir = tempfile::tempdir().expect("临时目录");
        let secrets = crate::gateway::security::tests_support::memory_store();
        let storage = Storage::open(dir.path(), secrets.as_ref()).expect("打开存储");
        for index in 0..3 {
            storage.record_usage(UsageRecord {
                ts: now_ms(),
                model: "glm-5.3".into(),
                channel_id: "wb-cn".into(),
                account_uid: format!("u{index}"),
                status: "completed".into(),
                stream: true,
                attempts: 1,
                usage: Usage {
                    prompt_tokens: 100,
                    completion_tokens: 10,
                    cached_tokens: 60,
                    total_tokens: 110,
                    ..Usage::default()
                },
                estimated: false,
                ttft_ms: Some(500),
                total_ms: 1200,
                effort: Some("high".into()),
                upstream_model: Some("glm-5.3".into()),
                error_detail: None,
            });
        }
        // 写入是异步的：轮询等它落库（测试里最多等 2 秒）。
        let mut summary = storage.usage_summary(24).expect("汇总");
        for _ in 0..40 {
            if summary["requests"].as_i64() == Some(3) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            summary = storage.usage_summary(24).expect("汇总");
        }
        assert_eq!(summary["requests"], 3);
        assert_eq!(summary["prompt_tokens"], 300);
        assert_eq!(summary["cached_tokens"], 180);
        assert_eq!(summary["cache_hit_rate"], 0.6);
        assert_eq!(summary["by_channel"][0]["key"], "wb-cn");
        assert_eq!(summary["by_model"][0]["key"], "glm-5.3");
    }
}

