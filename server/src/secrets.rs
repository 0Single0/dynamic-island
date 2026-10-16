//! 凭据存储：服务器版把 master key 从「操作系统凭据库」换成「环境变量 / 文件」。
//!
//! 桌面端用 `keyring` 写 Windows 凭据管理器；容器里没有等价设施
//! （Linux 的 Secret Service 需要 DBus 会话，精简镜像里不存在），所以这里改成：
//!
//! 1. 环境变量 `GATEWAY_MASTER_KEY`（base64 的 32 字节）优先 —— 跨重启、跨重建稳定，
//!    是生产推荐用法（换机器/换镜像只要这个值不变，旧密文照样解得开）。
//! 2. 没有环境变量时，首次生成随机主密钥并落到 `<data>/secrets/gateway__master-key`
//!    （0600），之后复用。
//!
//! 无论走哪条，SQLite 里存的都只是 `base64(nonce || ciphertext)`，
//! 明文凭据不进库、不进日志、不进接口返回。

use std::{
    fmt,
    path::{Path, PathBuf},
};

use zeroize::Zeroizing;

/// 主密钥在存储里的条目名（与 `gateway::security::envelope::MASTER_KEY_REF` 一致）。
pub const MASTER_KEY_REF: &str = "gateway/master-key";

#[derive(Debug)]
pub struct SecretStoreError(String);

impl SecretStoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for SecretStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SecretStoreError {}

pub trait SecretStore: Send + Sync {
    fn put(&self, secret_ref: &str, secret: &str) -> Result<(), SecretStoreError>;
    fn get(&self, secret_ref: &str) -> Result<Option<Zeroizing<String>>, SecretStoreError>;
    /// 删除一条密钥。目前只有测试用到 —— 账号凭据随 `accounts` 行一起删，
    /// 主密钥删掉等于丢弃全部凭据，所以生产路径刻意不暴露这个操作。
    #[allow(dead_code)]
    fn delete(&self, secret_ref: &str) -> Result<(), SecretStoreError>;
}

/// 文件 / 环境变量支撑的凭据库（服务器版默认实现）。
pub struct FileSecretStore {
    dir: PathBuf,
    /// `GATEWAY_MASTER_KEY`：设了就压过文件，且不允许被覆盖写。
    env_master: Option<String>,
}

impl FileSecretStore {
    /// `dir` 是密钥文件目录（通常 `<data>/secrets`）。
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let env_master = std::env::var("GATEWAY_MASTER_KEY")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        Self {
            dir: dir.into(),
            env_master,
        }
    }

    /// 条目名 → 文件名：把路径分隔符换掉，避免越出目录。
    fn path_for(&self, reference: &str) -> PathBuf {
        let safe: String = reference
            .chars()
            .map(|ch| match ch {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => ch,
                _ => '_',
            })
            .collect();
        self.dir.join(safe)
    }
}

impl SecretStore for FileSecretStore {
    fn put(&self, secret_ref: &str, secret: &str) -> Result<(), SecretStoreError> {
        // 环境变量提供的主密钥是只读的：写回文件反而会造成"文件与 env 不一致"的困惑。
        if secret_ref == MASTER_KEY_REF && self.env_master.is_some() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)
            .map_err(|error| SecretStoreError::new(format!("创建密钥目录失败：{error}")))?;
        let path = self.path_for(secret_ref);
        std::fs::write(&path, secret)
            .map_err(|error| SecretStoreError::new(format!("写入密钥失败：{error}")))?;
        restrict_permissions(&path);
        Ok(())
    }

    fn get(&self, secret_ref: &str) -> Result<Option<Zeroizing<String>>, SecretStoreError> {
        if secret_ref == MASTER_KEY_REF {
            if let Some(value) = &self.env_master {
                return Ok(Some(Zeroizing::new(value.clone())));
            }
        }
        let path = self.path_for(secret_ref);
        match std::fs::read_to_string(&path) {
            Ok(value) => Ok(Some(Zeroizing::new(value))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(SecretStoreError::new(format!("读取密钥失败：{error}"))),
        }
    }

    fn delete(&self, secret_ref: &str) -> Result<(), SecretStoreError> {
        let path = self.path_for(secret_ref);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(SecretStoreError::new(format!("删除密钥失败：{error}"))),
        }
    }
}

/// 只在 Unix 上收权限（Windows 走 ACL，另有默认策略）。
fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_round_trips_and_deletes() {
        let dir = tempfile::tempdir().expect("临时目录");
        let store = FileSecretStore::new(dir.path());
        assert!(store.get("gateway/master-key").expect("读取").is_none());
        store.put("gateway/master-key", "abc").expect("写入");
        assert_eq!(
            store
                .get("gateway/master-key")
                .expect("读取")
                .as_deref()
                .map(String::as_str),
            Some("abc")
        );
        store.delete("gateway/master-key").expect("删除");
        assert!(store.get("gateway/master-key").expect("读取").is_none());
    }

    #[test]
    fn reference_with_separators_never_escapes_the_directory() {
        let dir = tempfile::tempdir().expect("临时目录");
        let store = FileSecretStore::new(dir.path());
        let path = store.path_for("../../etc/passwd");
        assert_eq!(path.parent(), Some(dir.path()), "必须留在密钥目录内");
    }
}
