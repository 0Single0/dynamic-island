use std::{path::Path, sync::Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct StorageError(String);

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StorageError {}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self(error.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRecord {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineRecord {
    pub id: String,
    pub session_id: String,
    pub kind: String,
    pub text: Option<String>,
    pub title: Option<String>,
    pub detail: Option<String>,
    pub status: String,
    pub position: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRecord {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub secret_ref: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRecord {
    pub id: String,
    pub provider_id: String,
    pub model_id: String,
    pub display_name: String,
    pub context_window: Option<u64>,
    pub supports_tools: bool,
    pub supports_streaming: bool,
    pub supports_images: bool,
    pub enabled: bool,
}

pub struct AgentStore {
    connection: Mutex<Connection>,
}

use super::{
    events::{EventDraft, ExecutionEvent},
    model::ModelMessage,
};

impl AgentStore {
    pub fn open(app_data_dir: &Path) -> Result<Self, StorageError> {
        std::fs::create_dir_all(app_data_dir).map_err(|error| StorageError(error.to_string()))?;
        let path = app_data_dir.join("agent.sqlite3");
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS schema_migrations (
               version INTEGER PRIMARY KEY,
               applied_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sessions (
               id TEXT PRIMARY KEY,
               title TEXT NOT NULL,
               created_at TEXT NOT NULL,
               updated_at TEXT NOT NULL,
               archived INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS messages (
               id TEXT PRIMARY KEY,
               session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
               role TEXT NOT NULL,
               content TEXT NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS messages_session_created
               ON messages(session_id, created_at);
             CREATE TABLE IF NOT EXISTS agent_timeline (
               id TEXT PRIMARY KEY,
               session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
               kind TEXT NOT NULL,
               text TEXT,
               title TEXT,
               detail TEXT,
               status TEXT NOT NULL DEFAULT 'completed',
               position INTEGER NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS agent_timeline_session_position
               ON agent_timeline(session_id, position);
             CREATE TABLE IF NOT EXISTS providers (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               protocol TEXT NOT NULL,
               base_url TEXT NOT NULL,
               secret_ref TEXT,
               enabled INTEGER NOT NULL DEFAULT 1,
               created_at TEXT NOT NULL,
               updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS models (
               id TEXT PRIMARY KEY,
               provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
               model_id TEXT NOT NULL,
               display_name TEXT NOT NULL,
               context_window INTEGER,
               supports_tools INTEGER NOT NULL DEFAULT 1,
               supports_streaming INTEGER NOT NULL DEFAULT 1,
               supports_images INTEGER NOT NULL DEFAULT 0,
               enabled INTEGER NOT NULL DEFAULT 1
             );
             CREATE INDEX IF NOT EXISTS models_provider ON models(provider_id);
             CREATE TABLE IF NOT EXISTS runs (
               id TEXT PRIMARY KEY,
               session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
               status TEXT NOT NULL,
               stop_reason TEXT,
               created_at TEXT NOT NULL,
               updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS approvals (
               id TEXT PRIMARY KEY,
               run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
               tool_name TEXT NOT NULL,
               args_json TEXT NOT NULL,
               args_hash TEXT NOT NULL,
               risk TEXT NOT NULL,
               status TEXT NOT NULL,
               expires_at TEXT NOT NULL,
               created_at TEXT NOT NULL
             );",
        )?;
        connection.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS agent_events (
               id TEXT PRIMARY KEY, version INTEGER NOT NULL DEFAULT 1,
               session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
               run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
               step INTEGER, block_id TEXT, call_id TEXT, parent_call_id TEXT,
               seq INTEGER NOT NULL, event_type TEXT NOT NULL, payload TEXT NOT NULL,
               created_at TEXT NOT NULL, UNIQUE(session_id, seq)
             );
             CREATE INDEX IF NOT EXISTS agent_events_session_seq ON agent_events(session_id, seq);
             CREATE TABLE IF NOT EXISTS agent_context (
               seq INTEGER PRIMARY KEY AUTOINCREMENT,
               session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
               run_id TEXT, payload TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS agent_context_session ON agent_context(session_id, seq);
             CREATE TABLE IF NOT EXISTS agent_context_seeded (
               session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE
             );
             INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (2, strftime('%s','now'));
             COMMIT;",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn create_session(&self, title: &str) -> Result<SessionRecord, StorageError> {
        let now = now();
        let record = SessionRecord {
            id: new_id("session"),
            title: if title.trim().is_empty() {
                "新会话".into()
            } else {
                title.trim().into()
            },
            created_at: now.clone(),
            updated_at: now,
            archived: false,
        };
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        connection.execute(
            "INSERT INTO sessions (id, title, created_at, updated_at, archived) VALUES (?1, ?2, ?3, ?4, 0)",
            params![record.id, record.title, record.created_at, record.updated_at],
        )?;
        Ok(record)
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionRecord>, StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        let mut statement = connection.prepare(
            "SELECT id, title, created_at, updated_at, archived
             FROM sessions WHERE archived = 0 AND (
               EXISTS(SELECT 1 FROM messages WHERE session_id=sessions.id)
               OR EXISTS(SELECT 1 FROM agent_events WHERE session_id=sessions.id)
               OR EXISTS(SELECT 1 FROM agent_timeline WHERE session_id=sessions.id)
             ) ORDER BY updated_at DESC, rowid DESC",
        )?;
        let records = statement
            .query_map([], |row| {
                Ok(SessionRecord {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                    archived: row.get::<_, i64>(4)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }

    /// 删除会话本身；消息、轨迹、事件、上下文与运行记录由外键级联清理。
    /// 正在运行的会话不允许删除，否则运行中的写入会失去归属。
    pub fn delete_session(&self, id: &str) -> Result<bool, StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        let running: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE session_id = ?1 AND status = 'running')",
            params![id],
            |row| row.get(0),
        )?;
        if running {
            return Err(StorageError("此会话正在运行，无法删除".into()));
        }
        let removed = connection.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        Ok(removed > 0)
    }

    pub fn add_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
    ) -> Result<MessageRecord, StorageError> {
        let now = now();
        let record = MessageRecord {
            id: new_id("message"),
            session_id: session_id.into(),
            role: role.into(),
            content: content.into(),
            created_at: now.clone(),
        };
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        let exists: Option<String> = connection
            .query_row(
                "SELECT id FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(StorageError("session not found".into()));
        }
        connection.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![record.id, record.session_id, record.role, record.content, record.created_at],
        )?;
        connection.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        if role == "user" {
            let title: String = content
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(32)
                .collect();
            connection.execute(
                "UPDATE sessions SET title=?1 WHERE id=?2 AND title IN ('新对话','新会话') AND (SELECT COUNT(*) FROM messages WHERE session_id=?2 AND role='user')=1",
                params![title, session_id],
            )?;
        }
        Ok(record)
    }

    pub fn list_messages(&self, session_id: &str) -> Result<Vec<MessageRecord>, StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        let mut statement = connection.prepare(
            "SELECT id, session_id, role, content, created_at FROM messages
             WHERE session_id = ?1 ORDER BY created_at ASC",
        )?;
        let records = statement
            .query_map(params![session_id], |row| {
                Ok(MessageRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }

    pub fn add_timeline_item(
        &self,
        session_id: &str,
        id: &str,
        kind: &str,
        text: Option<&str>,
        title: Option<&str>,
        detail: Option<&str>,
        status: &str,
    ) -> Result<TimelineRecord, StorageError> {
        let now = now();
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        let position: i64 = connection.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM agent_timeline WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;
        connection.execute(
            "INSERT OR REPLACE INTO agent_timeline
             (id, session_id, kind, text, title, detail, status, position, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![id, session_id, kind, text, title, detail, status, position, now],
        )?;
        Ok(TimelineRecord {
            id: id.into(),
            session_id: session_id.into(),
            kind: kind.into(),
            text: text.map(str::to_owned),
            title: title.map(str::to_owned),
            detail: detail.map(str::to_owned),
            status: status.into(),
            position,
            created_at: now,
        })
    }

    pub fn update_timeline_status(
        &self,
        id: &str,
        status: &str,
        detail: Option<&str>,
    ) -> Result<(), StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        connection.execute(
            "UPDATE agent_timeline SET status = ?1, detail = COALESCE(?2, detail) WHERE id = ?3",
            params![status, detail, id],
        )?;
        Ok(())
    }

    pub fn append_timeline_text(&self, id: &str, delta: &str) -> Result<(), StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        connection.execute(
            "UPDATE agent_timeline SET text = COALESCE(text, '') || ?1 WHERE id = ?2",
            params![delta, id],
        )?;
        Ok(())
    }

    pub fn update_timeline_tool(
        &self,
        id: &str,
        title: &str,
        detail: &str,
        status: &str,
    ) -> Result<(), StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        connection.execute(
            "UPDATE agent_timeline SET title = ?1, detail = ?2, status = ?3 WHERE id = ?4",
            params![title, detail, status, id],
        )?;
        Ok(())
    }

    pub fn list_timeline(&self, session_id: &str) -> Result<Vec<TimelineRecord>, StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        let mut statement = connection.prepare(
            "SELECT id, session_id, kind, text, title, detail, status, position, created_at
             FROM agent_timeline WHERE session_id = ?1 ORDER BY position ASC",
        )?;
        let records = statement
            .query_map(params![session_id], |row| {
                Ok(TimelineRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    kind: row.get(2)?,
                    text: row.get(3)?,
                    title: row.get(4)?,
                    detail: row.get(5)?,
                    status: row.get(6)?,
                    position: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }

    pub fn upsert_provider(&self, provider: &ProviderRecord) -> Result<(), StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        connection.execute(
            "INSERT INTO providers (id, name, protocol, base_url, secret_ref, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, protocol=excluded.protocol,
             base_url=excluded.base_url, secret_ref=excluded.secret_ref, enabled=excluded.enabled,
             updated_at=excluded.updated_at",
            params![provider.id, provider.name, provider.protocol, provider.base_url, provider.secret_ref,
                provider.enabled as i64, provider.created_at, provider.updated_at],
        )?;
        Ok(())
    }

    pub fn list_providers(&self) -> Result<Vec<ProviderRecord>, StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        let mut statement = connection.prepare(
            "SELECT id, name, protocol, base_url, secret_ref, enabled, created_at, updated_at
             FROM providers ORDER BY created_at ASC",
        )?;
        let records = statement
            .query_map([], |row| {
                Ok(ProviderRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    protocol: row.get(2)?,
                    base_url: row.get(3)?,
                    secret_ref: row.get(4)?,
                    enabled: row.get::<_, i64>(5)? != 0,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }

    pub fn get_provider(&self, id: &str) -> Result<Option<ProviderRecord>, StorageError> {
        Ok(self
            .list_providers()?
            .into_iter()
            .find(|provider| provider.id == id))
    }

    pub fn upsert_model(&self, model: &ModelRecord) -> Result<(), StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        connection.execute(
            "INSERT INTO models (id, provider_id, model_id, display_name, context_window, supports_tools,
             supports_streaming, supports_images, enabled) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET provider_id=excluded.provider_id, model_id=excluded.model_id,
             display_name=excluded.display_name, context_window=excluded.context_window,
             supports_tools=excluded.supports_tools, supports_streaming=excluded.supports_streaming,
             supports_images=excluded.supports_images, enabled=excluded.enabled",
            params![model.id, model.provider_id, model.model_id, model.display_name, model.context_window,
                model.supports_tools as i64, model.supports_streaming as i64, model.supports_images as i64,
                model.enabled as i64],
        )?;
        Ok(())
    }

    pub fn list_models(&self) -> Result<Vec<ModelRecord>, StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        let mut statement = connection.prepare(
            "SELECT id, provider_id, model_id, display_name, context_window, supports_tools,
             supports_streaming, supports_images, enabled FROM models ORDER BY display_name ASC",
        )?;
        let records = statement
            .query_map([], |row| {
                Ok(ModelRecord {
                    id: row.get(0)?,
                    provider_id: row.get(1)?,
                    model_id: row.get(2)?,
                    display_name: row.get(3)?,
                    context_window: row.get(4)?,
                    supports_tools: row.get::<_, i64>(5)? != 0,
                    supports_streaming: row.get::<_, i64>(6)? != 0,
                    supports_images: row.get::<_, i64>(7)? != 0,
                    enabled: row.get::<_, i64>(8)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }

    pub fn get_model(&self, id: &str) -> Result<Option<ModelRecord>, StorageError> {
        Ok(self.list_models()?.into_iter().find(|model| model.id == id))
    }

    pub fn delete_model(&self, id: &str) -> Result<(), StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        connection.execute("DELETE FROM models WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn create_run(&self, run_id: &str, session_id: &str) -> Result<(), StorageError> {
        let now = now();
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        let running: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE session_id=?1 AND status='running')",
            params![session_id],
            |row| row.get(0),
        )?;
        if running {
            return Err(StorageError("此会话已有正在运行的请求".into()));
        }
        connection.execute(
            "INSERT INTO runs (id, session_id, status, created_at, updated_at) VALUES (?1, ?2, 'running', ?3, ?3)",
            params![run_id, session_id, now],
        )?;
        Ok(())
    }

    pub fn finish_run(
        &self,
        run_id: &str,
        status: &str,
        reason: Option<&str>,
    ) -> Result<(), StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        connection.execute(
            "UPDATE runs SET status = ?1, stop_reason = ?2, updated_at = ?3 WHERE id = ?4",
            params![status, reason, now(), run_id],
        )?;
        Ok(())
    }

    pub fn append_event(
        &self,
        session_id: &str,
        run_id: &str,
        draft: EventDraft,
    ) -> Result<ExecutionEvent, StorageError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq),0)+1 FROM agent_events WHERE session_id=?1",
            params![session_id],
            |row| row.get(0),
        )?;
        let event = ExecutionEvent {
            version: 1,
            id: new_id("event"),
            session_id: session_id.into(),
            run_id: run_id.into(),
            step: draft.step,
            block_id: draft.block_id,
            call_id: draft.call_id,
            parent_call_id: draft.parent_call_id,
            seq,
            event_type: draft.event_type,
            payload: draft.payload,
            created_at: now(),
        };
        tx.execute("INSERT INTO agent_events (id,version,session_id,run_id,step,block_id,call_id,parent_call_id,seq,event_type,payload,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![event.id,event.version,event.session_id,event.run_id,event.step,event.block_id,event.call_id,event.parent_call_id,event.seq,event.event_type,event.payload.to_string(),event.created_at])?;
        let terminal = match event.event_type.as_str() {
            "run_completed" => Some("completed"),
            "run_failed" => Some("failed"),
            "run_cancelled" => Some("cancelled"),
            "run_interrupted" => Some("interrupted"),
            _ => None,
        };
        if let Some(status) = terminal {
            tx.execute(
                "UPDATE runs SET status=?1,stop_reason=?2,updated_at=?3 WHERE id=?4",
                params![
                    status,
                    event.payload["error"]["category"].as_str(),
                    event.created_at,
                    run_id
                ],
            )?;
        }
        tx.commit()?;
        Ok(event)
    }

    pub fn list_events(
        &self,
        session_id: &str,
        after_seq: i64,
    ) -> Result<Vec<ExecutionEvent>, StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        let mut statement=connection.prepare("SELECT id,version,session_id,run_id,step,block_id,call_id,parent_call_id,seq,event_type,payload,created_at FROM agent_events WHERE session_id=?1 AND seq>?2 ORDER BY seq")?;
        let events = statement
            .query_map(params![session_id, after_seq], |row| {
                let payload: String = row.get(10)?;
                let payload = serde_json::from_str(&payload).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        10,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                Ok(ExecutionEvent {
                    id: row.get(0)?,
                    version: row.get(1)?,
                    session_id: row.get(2)?,
                    run_id: row.get(3)?,
                    step: row.get(4)?,
                    block_id: row.get(5)?,
                    call_id: row.get(6)?,
                    parent_call_id: row.get(7)?,
                    seq: row.get(8)?,
                    event_type: row.get(9)?,
                    payload,
                    created_at: row.get(11)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(events)
    }

    pub fn seed_context(&self, session_id: &str) -> Result<(), StorageError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        let tx = connection.transaction()?;
        let seeded: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_context_seeded WHERE session_id=?1)",
            params![session_id],
            |r| r.get(0),
        )?;
        if !seeded {
            let mut statement = tx.prepare(
                "SELECT role,content FROM messages WHERE session_id=?1 ORDER BY created_at,rowid",
            )?;
            let messages = statement
                .query_map(params![session_id], |row| {
                    Ok(ModelMessage::text(
                        &row.get::<_, String>(0)?,
                        &row.get::<_, String>(1)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(statement);
            for message in messages {
                tx.execute(
                    "INSERT INTO agent_context (session_id,payload) VALUES (?1,?2)",
                    params![
                        session_id,
                        serde_json::to_string(&message).map_err(|e| StorageError(e.to_string()))?
                    ],
                )?;
            }
            tx.execute(
                "INSERT INTO agent_context_seeded(session_id) VALUES (?1)",
                params![session_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn append_context(
        &self,
        session_id: &str,
        run_id: &str,
        messages: &[ModelMessage],
    ) -> Result<(), StorageError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        let tx = connection.transaction()?;
        for message in messages {
            tx.execute(
                "INSERT INTO agent_context(session_id,run_id,payload) VALUES (?1,?2,?3)",
                params![
                    session_id,
                    run_id,
                    serde_json::to_string(message).map_err(|e| StorageError(e.to_string()))?
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_context(&self, session_id: &str) -> Result<Vec<ModelMessage>, StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
        let mut statement = connection
            .prepare("SELECT payload FROM agent_context WHERE session_id=?1 ORDER BY seq")?;
        let rows = statement
            .query_map(params![session_id], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|row| serde_json::from_str(&row).map_err(|e| StorageError(e.to_string())))
            .collect()
    }

    pub fn recover_interrupted_runs(&self) -> Result<(), StorageError> {
        let runs = {
            let connection = self
                .connection
                .lock()
                .map_err(|_| StorageError("storage lock poisoned".into()))?;
            let mut statement =
                connection.prepare("SELECT id,session_id FROM runs WHERE status='running'")?;
            let rows = statement
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        for (run_id, session_id) in runs {
            self.append_event(
                &session_id,
                &run_id,
                EventDraft::new(
                    "run_interrupted",
                    serde_json::json!({"message":"应用关闭导致运行中断，已保留生成内容"}),
                ),
            )?;
            self.finish_run(&run_id, "interrupted", Some("app_restarted"))?;
        }
        Ok(())
    }
}

fn now() -> String {
    // ISO-like UTC timestamp without adding another time dependency.
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or_default();
    timestamp.to_string()
}

fn new_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    format!("{prefix}-{nanos}-{}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_session_and_messages_in_sqlite() {
        let path =
            std::env::temp_dir().join(format!("dynamic-island-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let store = AgentStore::open(&path).unwrap();
        let session = store.create_session("测试").unwrap();
        store.add_message(&session.id, "user", "你好").unwrap();
        store
            .add_message(&session.id, "assistant", "你好，我在")
            .unwrap();
        let messages = store.list_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn empty_sessions_stay_out_of_history_until_the_first_message() {
        let path = std::env::temp_dir().join(new_id("history-test"));
        let store = AgentStore::open(&path).unwrap();
        let empty = store.create_session("新对话").unwrap();
        assert!(store.list_sessions().unwrap().is_empty());
        store
            .add_message(&empty.id, "user", "帮我\n查看 播放状态")
            .unwrap();
        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "帮我 查看 播放状态");
        store.add_message(&empty.id, "user", "第二个问题").unwrap();
        assert_eq!(store.list_sessions().unwrap()[0].title, sessions[0].title);
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn deleting_a_session_cascades_to_messages_timeline_and_events() {
        use serde_json::json;

        let path = std::env::temp_dir().join(new_id("delete-session-test"));
        let store = AgentStore::open(&path).unwrap();
        let kept = store.create_session("保留").unwrap();
        let removed = store.create_session("删除").unwrap();
        store.add_message(&kept.id, "user", "留我").unwrap();
        store.add_message(&removed.id, "user", "删我").unwrap();
        store
            .add_timeline_item(
                &removed.id,
                "t",
                "assistant",
                Some("内容"),
                None,
                None,
                "completed",
            )
            .unwrap();
        store.create_run("run-1", &removed.id).unwrap();
        store
            .append_event(&removed.id, "run-1", EventDraft::new("usage", json!({})))
            .unwrap();
        store.finish_run("run-1", "completed", None).unwrap();

        assert!(store.delete_session(&removed.id).unwrap());
        assert!(!store.delete_session(&removed.id).unwrap());
        assert!(store.list_messages(&removed.id).unwrap().is_empty());
        assert!(store.list_timeline(&removed.id).unwrap().is_empty());
        assert!(store.list_events(&removed.id, 0).unwrap().is_empty());
        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, kept.id);
        drop(store);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn running_sessions_reject_deletion() {
        let path = std::env::temp_dir().join(new_id("delete-running-test"));
        let store = AgentStore::open(&path).unwrap();
        let session = store.create_session("运行中").unwrap();
        store.add_message(&session.id, "user", "开始").unwrap();
        store.create_run("run-live", &session.id).unwrap();

        assert!(store.delete_session(&session.id).is_err());
        assert_eq!(store.list_sessions().unwrap().len(), 1);
        store.finish_run("run-live", "cancelled", None).unwrap();
        assert!(store.delete_session(&session.id).unwrap());
        assert!(store.list_sessions().unwrap().is_empty());
        drop(store);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn reopens_interleaved_reasoning_text_tool_and_answer_in_event_order() {
        let path = std::env::temp_dir().join(format!(
            "dynamic-island-trace-{}-{}",
            std::process::id(),
            new_id("test")
        ));
        let session_id;
        {
            let store = AgentStore::open(&path).unwrap();
            session_id = store.create_session("轨迹").unwrap().id;
            store
                .add_timeline_item(
                    &session_id,
                    "u",
                    "user",
                    Some("请求"),
                    None,
                    None,
                    "completed",
                )
                .unwrap();
            store
                .add_timeline_item(
                    &session_id,
                    "r",
                    "reasoning",
                    Some(""),
                    None,
                    None,
                    "running",
                )
                .unwrap();
            store.append_timeline_text("r", "先查询").unwrap();
            store
                .add_timeline_item(
                    &session_id,
                    "t1",
                    "assistant",
                    Some("我来查询。"),
                    None,
                    None,
                    "completed",
                )
                .unwrap();
            store
                .add_timeline_item(
                    &session_id,
                    "call",
                    "tool",
                    None,
                    Some("读取播放状态"),
                    Some("执行中"),
                    "running",
                )
                .unwrap();
            store
                .update_timeline_tool("call", "读取播放状态", "成功", "completed")
                .unwrap();
            store
                .add_timeline_item(
                    &session_id,
                    "t2",
                    "assistant",
                    Some("当前歌曲是…"),
                    None,
                    None,
                    "completed",
                )
                .unwrap();
            store
                .update_timeline_status("r", "completed", None)
                .unwrap();
        }
        let store = AgentStore::open(&path).unwrap();
        let timeline = store.list_timeline(&session_id).unwrap();
        assert_eq!(
            timeline
                .iter()
                .map(|item| item.kind.as_str())
                .collect::<Vec<_>>(),
            ["user", "reasoning", "assistant", "tool", "assistant"]
        );
        assert_eq!(timeline[1].text.as_deref(), Some("先查询"));
        assert_eq!(timeline[3].status, "completed");
        drop(store);
        let _ = std::fs::remove_dir_all(path);
    }
}
