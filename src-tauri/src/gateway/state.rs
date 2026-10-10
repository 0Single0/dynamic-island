//! 网关共享状态：配置、渠道注册表、账号池、并发槽、服务状态与统计。
//!
//! 面板（Tauri command）与 HTTP 服务共用同一份状态，避免两套真相。

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
};

use serde::Serialize;
use tokio::sync::Semaphore;

use crate::agent::secrets::{OsCredentialStore, SecretStore};
use crate::gateway::{
    channels::{self, AccountSnapshot, ChannelAdapter},
    config::Config,
    core::{
        affinity::AffinityStore,
        matrix::{ChannelCatalog, Matrix},
        pool::{AccountPool, PoolAccount},
    },
    errors::GatewayError,
    storage::{AccountRecord, Storage},
    timeutil::now_ms,
};

/// 进程内计数，重启清零；面板读它做"今天怎么样"的粗看。
#[derive(Debug, Default)]
pub struct Stats {
    pub requests: std::sync::atomic::AtomicU64,
    pub failures: std::sync::atomic::AtomicU64,
    pub retries: std::sync::atomic::AtomicU64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ServerStatus {
    pub running: bool,
    /// 实际监听地址（端口被占用时可能与配置不同）。
    pub addr: Option<String>,
    pub error: Option<String>,
}

pub type SharedGateway = Arc<GatewayState>;

pub struct GatewayState {
    pub started_at_ms: u64,
    pub client: reqwest::Client,
    pub pool: AccountPool,
    /// 账号 / 凭据 / 用量 / 亲和的落库层。
    pub storage: Arc<Storage>,
    /// 数据目录（配置写回用）。
    pub dir: std::path::PathBuf,
    /// 全局并发闸门（每账号并发在 `PoolAccount::max_inflight`）。
    pub slots: Arc<Semaphore>,
    pub stats: Stats,
    config: RwLock<Config>,
    adapters: RwLock<HashMap<String, Arc<dyn ChannelAdapter>>>,
    /// 会话亲和：保上游前缀缓存命中。
    pub affinity: AffinityStore,
    /// 模型 × 渠道矩阵：懒加载 + TTL，见 `ensure_matrix`。
    matrix: RwLock<Option<Arc<Matrix>>>,
    matrix_lock: tokio::sync::Mutex<()>,
    matrix_built_at: AtomicU64,
    /// 最近的调度决策（面板的"调度"视图与排障用）。
    decisions: RwLock<VecDeque<serde_json::Value>>,
    /// 任务执行进度：key = `channel/uid`。长任务在后台跑，前端轮询这里看进展。
    task_runs: RwLock<HashMap<String, serde_json::Value>>,
    status: RwLock<ServerStatus>,
    warnings: RwLock<Vec<String>>,
}

impl GatewayState {
    pub fn initialize(config: Config, dir: std::path::PathBuf) -> Result<SharedGateway, GatewayError> {
        Self::initialize_with_secrets(config, dir, &OsCredentialStore)
    }

    /// 与 `initialize` 相同，但凭据库可注入：测试用内存实现，避免写系统凭据管理器。
    pub fn initialize_with_secrets(
        config: Config,
        dir: std::path::PathBuf,
        secrets: &dyn SecretStore,
    ) -> Result<SharedGateway, GatewayError> {
        eprintln!("[gateway] 数据目录 {}", dir.display());
        let storage = Storage::open(&dir, secrets)?;
        // 首次运行把配置文件里声明的账号灌进库；之后库是账号的唯一事实来源。
        if let Err(error) = storage.seed_from_config(&config) {
            eprintln!("[gateway] 初始账号灌入失败：{}", error.message);
        }
        let records = storage.list_accounts().unwrap_or_default();

        let mut warnings = Vec::new();
        let adapters = channels::build_registry(&config, &mut warnings);
        let accounts = build_accounts(&config, &adapters, &records, &mut warnings);
        let pool = AccountPool::new(accounts, config.cooldowns.clone());
        // 亲和绑定落盘后再恢复：进程重启不该让所有会话换账号（那等于把缓存全丢掉）。
        let affinity = AffinityStore::new(&config.affinity);
        match storage.load_affinity() {
            Ok(rows) if !rows.is_empty() => {
                let count = rows.len();
                affinity.restore(rows);
                eprintln!("[gateway] 已恢复 {count} 条会话亲和绑定");
            }
            Ok(_) => {}
            Err(error) => warnings.push(format!("亲和绑定恢复失败：{}", error.message)),
        }
        let slots = Arc::new(Semaphore::new(config.concurrency.global.max(1) as usize));
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            // 只设连接超时：流式响应是长连接，任何总超时都会把长回答掐断。
            .pool_max_idle_per_host(8)
            .user_agent("dynamic-island-gateway/0.1")
            .build()
            .map_err(|error| GatewayError::internal(format!("HTTP 客户端构建失败：{error}")))?;

        Ok(Arc::new(Self {
            started_at_ms: now_ms(),
            client,
            pool,
            storage,
            dir,
            slots,
            stats: Stats::default(),
            config: RwLock::new(config),
            adapters: RwLock::new(adapters),
            affinity,
            matrix: RwLock::new(None),
            matrix_lock: tokio::sync::Mutex::new(()),
            matrix_built_at: AtomicU64::new(0),
            decisions: RwLock::new(VecDeque::with_capacity(DECISION_HISTORY)),
            task_runs: RwLock::new(HashMap::new()),
            status: RwLock::new(ServerStatus::default()),
            warnings: RwLock::new(warnings),
        }))
    }

    pub fn config(&self) -> Config {
        self.config
            .read()
            .map(|config| config.clone())
            .unwrap_or_default()
    }

    pub fn channel(&self, id: &str) -> Option<Arc<dyn ChannelAdapter>> {
        self.adapters.read().ok()?.get(id).cloned()
    }

    /// 渠道列表按 id 排序：HashMap 的迭代顺序不确定，而 `/v1/models` 的顺序要对用户稳定。
    pub fn channels(&self) -> Vec<Arc<dyn ChannelAdapter>> {
        let mut list: Vec<Arc<dyn ChannelAdapter>> = self
            .adapters
            .read()
            .map(|map| map.values().cloned().collect())
            .unwrap_or_default();
        list.sort_by(|a, b| a.id().cmp(b.id()));
        list
    }

    pub fn first_account_for(&self, channel_id: &str) -> Option<AccountSnapshot> {
        self.pool
            .by_channel(channel_id)
            .first()
            .map(|account| account.snapshot())
    }

    pub fn warnings(&self) -> Vec<String> {
        self.warnings.read().map(|list| list.clone()).unwrap_or_default()
    }

    pub fn set_server_status(&self, status: ServerStatus) {
        if let Ok(mut slot) = self.status.write() {
            *slot = status;
        }
    }

    pub fn server_status(&self) -> ServerStatus {
        self.status
            .read()
            .map(|status| status.clone())
            .unwrap_or_default()
    }

    pub fn uptime_ms(&self) -> u64 {
        now_ms().saturating_sub(self.started_at_ms)
    }

    pub fn global_available(&self) -> usize {
        self.slots.available_permits()
    }

    /// 取当前矩阵；过期或不存在时**按需**重建（TTL + 双检锁，并发下只打一轮上游）。
    pub async fn ensure_matrix(&self, force: bool) -> Arc<Matrix> {
        let ttl_ms = self.config().catalog_ttl_seconds.saturating_mul(1000);
        if !force {
            if let Some(matrix) = self.matrix.read().ok().and_then(|slot| slot.clone()) {
                if now_ms().saturating_sub(self.matrix_built_at.load(Ordering::Relaxed)) < ttl_ms {
                    return matrix;
                }
            }
        }
        let _guard = self.matrix_lock.lock().await;
        if !force {
            if let Some(matrix) = self.matrix.read().ok().and_then(|slot| slot.clone()) {
                if now_ms().saturating_sub(self.matrix_built_at.load(Ordering::Relaxed)) < ttl_ms {
                    return matrix;
                }
            }
        }

        let config = self.config();
        let mut catalogs = Vec::new();
        for channel in self.channels() {
            let channel_config = config.channel(channel.id());
            // 没有账号也要试着拉目录：假渠道不需要凭据，真实渠道拿不到凭据时会回落到内置快照。
            let account = self
                .first_account_for(channel.id())
                .unwrap_or_else(|| AccountSnapshot {
                    channel_id: channel.id().to_string(),
                    ..AccountSnapshot::default()
                });
            let (models, live) = match channel.list_models(&self.client, &account).await {
                Ok(Some(models)) => (models, true),
                // 拉取失败：拿不到任何清单（假渠道没实现目录时也走这里）。
                _ => (Vec::new(), false),
            };
            catalogs.push(ChannelCatalog {
                channel_id: channel.id().to_string(),
                priority: channel_config.map(|entry| entry.priority).unwrap_or(100),
                weight: channel_config.map(|entry| entry.weight).unwrap_or(1),
                models,
                live,
            });
        }
        let matrix = Arc::new(Matrix::build(&catalogs, &config.aliases, &config.routes));
        let (live_count, builtin_count) = matrix.sources();
        eprintln!(
            "[gateway] 模型矩阵已刷新：{} 条路由（实时 {live_count} / 快照 {builtin_count}）",
            matrix.route_count()
        );
        if let Ok(mut slot) = self.matrix.write() {
            *slot = Some(Arc::clone(&matrix));
        }
        self.matrix_built_at.store(now_ms(), Ordering::Relaxed);
        matrix
    }

    pub fn matrix_snapshot(&self) -> Option<Arc<Matrix>> {
        self.matrix.read().ok().and_then(|slot| slot.clone())
    }

    /// 记一条调度决策（内存环形缓冲，供面板"调度"视图）。
    pub fn push_decision(&self, entry: serde_json::Value) {
        if let Ok(mut decisions) = self.decisions.write() {
            if decisions.len() >= DECISION_HISTORY {
                decisions.pop_front();
            }
            decisions.push_back(entry);
        }
    }

    /// 开始一次任务执行；已在执行中返回 false（调用方据此跳过该账号）。
    pub fn begin_task_run(&self, channel_id: &str, uid: &str) -> bool {
        let Ok(mut runs) = self.task_runs.write() else {
            return false;
        };
        let key = format!("{channel_id}/{uid}");
        if runs
            .get(&key)
            .map(|entry| entry["running"].as_bool().unwrap_or(false))
            .unwrap_or(false)
        {
            return false;
        }
        runs.insert(
            key,
            serde_json::json!({
                "running": true,
                "started_at": now_ms(),
                "finished_at": serde_json::Value::Null,
                "results": [],
                "done": 0,
                "earned": 0,
            }),
        );
        true
    }

    /// 推一条任务结果（后台执行线程调用）。
    pub fn push_task_result(
        &self,
        channel_id: &str,
        uid: &str,
        result: crate::gateway::channels::TaskRunResult,
    ) {
        let Ok(mut runs) = self.task_runs.write() else {
            return;
        };
        let Some(entry) = runs.get_mut(&format!("{channel_id}/{uid}")) else {
            return;
        };
        if let Some(list) = entry["results"].as_array_mut() {
            list.push(serde_json::to_value(result).unwrap_or(serde_json::Value::Null));
        }
    }

    pub fn finish_task_run(&self, channel_id: &str, uid: &str, done: u32, earned: u32) {
        let Ok(mut runs) = self.task_runs.write() else {
            return;
        };
        if let Some(entry) = runs.get_mut(&format!("{channel_id}/{uid}")) {
            entry["running"] = serde_json::json!(false);
            entry["finished_at"] = serde_json::json!(now_ms());
            entry["done"] = serde_json::json!(done);
            entry["earned"] = serde_json::json!(earned);
        }
    }

    pub fn fail_task_run(&self, channel_id: &str, uid: &str, message: &str) {
        let Ok(mut runs) = self.task_runs.write() else {
            return;
        };
        if let Some(entry) = runs.get_mut(&format!("{channel_id}/{uid}")) {
            entry["running"] = serde_json::json!(false);
            entry["finished_at"] = serde_json::json!(now_ms());
            entry["error"] = serde_json::json!(message);
        }
    }

    /// 读任务执行进度（前端轮询）。
    pub fn task_progress(&self, channel_id: &str, uid: &str) -> serde_json::Value {
        self.task_runs
            .read()
            .ok()
            .and_then(|runs| runs.get(&format!("{channel_id}/{uid}")).cloned())
            .unwrap_or_else(|| {
                serde_json::json!({
                    "running": false,
                    "results": [],
                    "done": 0,
                    "earned": 0,
                })
            })
    }

    /// 最近的决策，最新在前。
    pub fn decisions(&self) -> Vec<serde_json::Value> {
        self.decisions
            .read()
            .map(|decisions| decisions.iter().rev().cloned().collect())
            .unwrap_or_default()
    }

    /// 替换配置并重建立生效的派生结构（渠道适配器、账号池、亲和策略）。
    ///
    /// 配置写回磁盘，矩阵置空等下次请求按需重建。
    pub fn replace_config(&self, config: Config) -> Result<(), GatewayError> {
        config.save(&self.dir)?;

        let mut warnings = Vec::new();
        let adapters = channels::build_registry(&config, &mut warnings);
        let records = self.storage.list_accounts().unwrap_or_default();
        let accounts = build_accounts(&config, &adapters, &records, &mut warnings);
        self.pool.replace(accounts);
        if let Ok(mut slot) = self.adapters.write() {
            *slot = adapters;
        }
        if let Ok(mut slot) = self.warnings.write() {
            *slot = warnings;
        }

        // 亲和策略可能变了（模式/TTL/容量）：原地生效，已有绑定保留。
        self.affinity.reconfigure(&config.affinity);

        if let Ok(mut slot) = self.config.write() {
            *slot = config;
        }
        if let Ok(mut slot) = self.matrix.write() {
            *slot = None;
        }
        self.matrix_built_at.store(0, Ordering::Relaxed);
        Ok(())
    }

    /// 改配置的便捷入口：读当前配置 → 交给 `mutate` 改 → 生效。
    pub fn edit_config(
        &self,
        mutate: impl FnOnce(&mut Config),
    ) -> Result<Config, GatewayError> {
        let mut config = self.config();
        mutate(&mut config);
        self.replace_config(config.clone())?;
        Ok(config)
    }

    /// 账号表变化后重建账号池（登录成功、启停、删除都走它）。
    pub fn reload_accounts(&self) -> Result<usize, GatewayError> {
        let records = self.storage.list_accounts()?;
        let config = self.config();
        let adapters = self
            .adapters
            .read()
            .map(|adapters| adapters.clone())
            .unwrap_or_default();
        let mut warnings = Vec::new();
        let accounts = build_accounts(&config, &adapters, &records, &mut warnings);
        let count = accounts.len();
        self.pool.replace(accounts);
        if !warnings.is_empty() {
            if let Ok(mut slot) = self.warnings.write() {
                *slot = warnings;
            }
        }
        Ok(count)
    }

    /// 按 key 找账号（面板操作与登录落库后使用）。
    pub fn account(&self, channel_id: &str, uid: &str) -> Option<Arc<PoolAccount>> {
        self.pool.get(&crate::gateway::core::pool::AccountKey::new(channel_id, uid))
    }
}

/// 内存里保留多少条决策（面板一次看不到更多）。
const DECISION_HISTORY: usize = 200;

/// 按落库记录组装账号：渠道不存在（配置写错或适配器未实现）的账号跳过并记警告，
/// 而不是让整个网关起不来。
fn build_accounts(
    config: &Config,
    adapters: &HashMap<String, Arc<dyn ChannelAdapter>>,
    records: &[AccountRecord],
    warnings: &mut Vec<String>,
) -> Vec<Arc<PoolAccount>> {
    let per_account = config.concurrency.per_account.max(1);
    records
        .iter()
        .filter_map(|record| {
            let Some(adapter) = adapters.get(&record.channel_id) else {
                warnings.push(format!(
                    "账号 {}/{} 所属渠道没有可用适配器，已跳过",
                    record.channel_id, record.uid
                ));
                return None;
            };
            let channel = config.channel(&record.channel_id);
            let mut pooled = PoolAccount::new(record.channel_id.clone(), record.uid.clone());
            pooled.nickname = if record.nickname.is_empty() {
                record.uid.clone()
            } else {
                record.nickname.clone()
            };
            pooled.enabled = record.enabled;
            pooled.requires_credential = adapter.requires_credential();
            pooled.priority = record
                .priority_override
                .or_else(|| channel.map(|channel| channel.priority))
                .unwrap_or(100);
            pooled.weight = record
                .weight_override
                .or_else(|| channel.map(|channel| channel.weight))
                .unwrap_or(1);
            pooled.max_inflight = record.max_inflight.unwrap_or(per_account);
            pooled.proxy_slot = record.proxy_slot.clone();
            if let Some(credential) = &record.credential {
                pooled.set_credentials(credential.clone());
            }
            Some(Arc::new(pooled))
        })
        .collect()
}
