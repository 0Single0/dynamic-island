//! 设备指纹派生：同账号恒定、跨账号天然隔离，避免多账号被上游关联风控。
//!
//! 参考实现的 `derive_id(uid, salt)` 只按 uid 派生；这里把 `channel_id` 也拌进去，
//! 同一个 uid 在不同渠道（workbuddy 国内版 / 国际版 / 未来渠道）拿到不同指纹。

use sha2::{Digest, Sha256};

/// 凭据信封：master key 进系统凭据库，密文进 SQLite。
///
/// 为什么不是每个 token 都进凭据库：上游每次刷新都会轮换 token，
/// 逐条写系统凭据库既慢又污染凭据库；换成"一把主密钥 + 库内密文"更合适。
pub mod envelope {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Nonce,
    };
    use base64::Engine as _;

    use crate::secrets::MASTER_KEY_REF;

    const NONCE_LEN: usize = 12;

    /// 取（或首次生成）master key。首次生成后立刻写入凭据存储，
    /// 否则重启后旧密文全部解不开、账号要重新登录。
    pub fn master_key(store: &dyn crate::secrets::SecretStore) -> Result<[u8; 32], String> {
        if let Some(existing) = store.get(MASTER_KEY_REF).map_err(|error| error.to_string())? {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(existing.as_bytes())
                .map_err(|_| "master key 不是合法 base64".to_string())?;
            return bytes
                .try_into()
                .map_err(|_| "master key 长度不是 32 字节".to_string());
        }
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).map_err(|error| format!("取随机数失败：{error}"))?;
        store
            .put(
                MASTER_KEY_REF,
                &base64::engine::general_purpose::STANDARD.encode(key),
            )
            .map_err(|error| error.to_string())?;
        Ok(key)
    }

    /// 加密：返回 `base64(nonce || ciphertext)`，nonce 随密文一起存。
    pub fn seal(key: &[u8; 32], plaintext: &str) -> Result<String, String> {
        let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| "密钥长度不对".to_string())?;
        let mut nonce_bytes = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce_bytes).map_err(|error| format!("取随机数失败：{error}"))?;
        let nonce =
            Nonce::try_from(&nonce_bytes[..]).map_err(|_| "nonce 长度不对".to_string())?;
        let ciphertext = cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| "加密失败".to_string())?;
        let mut combined = nonce_bytes.to_vec();
        combined.extend_from_slice(&ciphertext);
        Ok(base64::engine::general_purpose::STANDARD.encode(combined))
    }

    pub fn open(key: &[u8; 32], sealed: &str) -> Result<String, String> {
        let combined = base64::engine::general_purpose::STANDARD
            .decode(sealed.as_bytes())
            .map_err(|_| "密文不是合法 base64".to_string())?;
        if combined.len() <= NONCE_LEN {
            return Err("密文长度不对".to_string());
        }
        let (nonce_bytes, ciphertext) = combined.split_at(NONCE_LEN);
        let nonce =
            Nonce::try_from(nonce_bytes).map_err(|_| "nonce 长度不对".to_string())?;
        let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| "密钥长度不对".to_string())?;
        let plaintext = cipher
            .decrypt(&nonce, ciphertext)
            .map_err(|_| "解密失败（密钥不匹配或密文被改动）".to_string())?;
        String::from_utf8(plaintext).map_err(|_| "明文不是 UTF-8".to_string())
    }
}

/// 稳定派生一个 36 位十六进制标识（长度对齐参考实现，便于直接替换）。
pub fn derive_id(channel_id: &str, uid: &str, salt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{salt}:{channel_id}:{uid}").as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex.truncate(36);
    hex
}

/// 稳定短哈希：用于 API Key 作用域、日志指纹这类"要能比较但不必还原"的场景。
pub fn short_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(32);
    for byte in digest.iter().take(8) {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// 每请求唯一的 `X-Request-ID`：稳定前缀 + 单调序号，跨请求可追溯又不暴露账号。
pub fn request_id(channel_id: &str, uid: &str, sequence: u64) -> String {
    format!(
        "{}-{:06}",
        derive_id(channel_id, uid, "req"),
        sequence % 1_000_000
    )
}

#[cfg(test)]
pub mod tests_support {
    //! 测试用的内存凭据库：避免单元测试往系统凭据管理器里写东西。

    use std::sync::{Arc, Mutex};

    use zeroize::Zeroizing;

    use crate::secrets::{SecretStore, SecretStoreError};

    #[derive(Default)]
    pub struct MemoryStore(Mutex<std::collections::HashMap<String, String>>);

    impl SecretStore for MemoryStore {
        fn put(&self, reference: &str, secret: &str) -> Result<(), SecretStoreError> {
            self.0
                .lock()
                .expect("锁")
                .insert(reference.to_string(), secret.to_string());
            Ok(())
        }

        fn get(&self, reference: &str) -> Result<Option<Zeroizing<String>>, SecretStoreError> {
            Ok(self
                .0
                .lock()
                .expect("锁")
                .get(reference)
                .cloned()
                .map(Zeroizing::new))
        }

        fn delete(&self, reference: &str) -> Result<(), SecretStoreError> {
            self.0.lock().expect("锁").remove(reference);
            Ok(())
        }
    }

    pub fn memory_store() -> Arc<dyn SecretStore> {
        Arc::new(MemoryStore::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_per_account_and_channel() {
        let a = derive_id("wb-cn", "uid-1", "machine");
        assert_eq!(a, derive_id("wb-cn", "uid-1", "machine"), "同账号必须恒定");
        assert_eq!(a.len(), 36);
        assert_ne!(a, derive_id("wb-cn", "uid-2", "machine"), "换账号必须不同");
        assert_ne!(a, derive_id("wb-intl", "uid-1", "machine"), "换渠道必须不同");
        assert_ne!(a, derive_id("wb-cn", "uid-1", "session"), "换盐必须不同");
    }

    #[test]
    fn request_id_keeps_a_stable_prefix() {
        let first = request_id("wb-cn", "uid-1", 1);
        let second = request_id("wb-cn", "uid-1", 2);
        assert_eq!(first.len(), second.len());
        assert!(first.ends_with("-000001"));
        assert!(second.ends_with("-000002"));
        assert_eq!(first[..36], second[..36]);
    }

    #[test]
    fn envelope_round_trips_and_detects_tampering() {
        let key = [7u8; 32];
        let sealed = envelope::seal(&key, "access-token-1").expect("加密");
        assert_ne!(sealed, "access-token-1");
        assert_eq!(
            envelope::open(&key, &sealed).expect("解密"),
            "access-token-1"
        );
        // 换密钥解不开。
        assert!(envelope::open(&[8u8; 32], &sealed).is_err());
        // 密文被改动要报错，不能静默返回垃圾。
        let mut tampered = sealed.clone();
        tampered.push('X');
        assert!(envelope::open(&key, &tampered).is_err());
        assert!(envelope::open(&key, "not-base64!!").is_err());
    }

    #[test]
    fn master_key_is_generated_once_and_reused() {
        use crate::secrets::{SecretStore, SecretStoreError};
        use std::sync::Mutex;
        use zeroize::Zeroizing;

        /// 内存实现；错误分支用 expect 表达"测试里不该失败"（SecretStoreError 的字段不可构造）。
        #[derive(Default)]
        struct MemoryStore(Mutex<std::collections::HashMap<String, String>>);
        impl SecretStore for MemoryStore {
            fn put(&self, reference: &str, secret: &str) -> Result<(), SecretStoreError> {
                self.0
                    .lock()
                    .expect("锁")
                    .insert(reference.to_string(), secret.to_string());
                Ok(())
            }
            fn get(&self, reference: &str) -> Result<Option<Zeroizing<String>>, SecretStoreError> {
                Ok(self
                    .0
                    .lock()
                    .expect("锁")
                    .get(reference)
                    .cloned()
                    .map(Zeroizing::new))
            }
            fn delete(&self, reference: &str) -> Result<(), SecretStoreError> {
                self.0.lock().expect("锁").remove(reference);
                Ok(())
            }
        }

        let store = MemoryStore::default();
        let first = envelope::master_key(&store).expect("首次生成");
        let second = envelope::master_key(&store).expect("复用");
        assert_eq!(first, second, "第二次必须复用已存的密钥，否则旧密文全废");
    }
}
