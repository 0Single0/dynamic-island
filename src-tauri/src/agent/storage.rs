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
             FROM sessions WHERE archived = 0 ORDER BY updated_at DESC",
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

    pub fn create_run(&self, run_id: &str, session_id: &str) -> Result<(), StorageError> {
        let now = now();
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError("storage lock poisoned".into()))?;
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
}
