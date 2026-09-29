use std::fmt;

use keyring::{Entry, Error as KeyringError};
use zeroize::Zeroizing;

const SERVICE_NAME: &str = "com.dynamic-island.agent";

#[derive(Debug)]
pub struct SecretStoreError(String);

impl fmt::Display for SecretStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SecretStoreError {}

pub trait SecretStore: Send + Sync {
    fn put(&self, secret_ref: &str, secret: &str) -> Result<(), SecretStoreError>;
    fn get(&self, secret_ref: &str) -> Result<Option<Zeroizing<String>>, SecretStoreError>;
    fn delete(&self, secret_ref: &str) -> Result<(), SecretStoreError>;
}

/// Uses the operating system's native credential vault through the keyring crate.
/// On Windows this resolves to Windows Credential Manager; secrets never enter
/// provider configuration, SQLite, logs, or frontend invoke results.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsCredentialStore;

impl OsCredentialStore {
    fn entry(secret_ref: &str) -> Result<Entry, SecretStoreError> {
        if secret_ref.is_empty() || secret_ref.len() > 200 {
            return Err(SecretStoreError("invalid secret reference".into()));
        }

        Entry::new(SERVICE_NAME, secret_ref)
            .map_err(|_| SecretStoreError("credential entry could not be created".into()))
    }
}

impl SecretStore for OsCredentialStore {
    fn put(&self, secret_ref: &str, secret: &str) -> Result<(), SecretStoreError> {
        Self::entry(secret_ref)?
            .set_password(secret)
            .map_err(|_| SecretStoreError("credential could not be saved".into()))
    }

    fn get(&self, secret_ref: &str) -> Result<Option<Zeroizing<String>>, SecretStoreError> {
        match Self::entry(secret_ref)?.get_password() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(_) => Err(SecretStoreError("credential could not be read".into())),
        }
    }

    fn delete(&self, secret_ref: &str) -> Result<(), SecretStoreError> {
        match Self::entry(secret_ref)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(_) => Err(SecretStoreError("credential could not be deleted".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use super::*;

    #[derive(Default)]
    struct MemorySecretStore(Mutex<HashMap<String, String>>);

    impl SecretStore for MemorySecretStore {
        fn put(&self, secret_ref: &str, secret: &str) -> Result<(), SecretStoreError> {
            self.0
                .lock()
                .map_err(|_| SecretStoreError("test store unavailable".into()))?
                .insert(secret_ref.to_owned(), secret.to_owned());
            Ok(())
        }

        fn get(&self, secret_ref: &str) -> Result<Option<Zeroizing<String>>, SecretStoreError> {
            Ok(self
                .0
                .lock()
                .map_err(|_| SecretStoreError("test store unavailable".into()))?
                .get(secret_ref)
                .cloned()
                .map(Zeroizing::new))
        }

        fn delete(&self, secret_ref: &str) -> Result<(), SecretStoreError> {
            self.0
                .lock()
                .map_err(|_| SecretStoreError("test store unavailable".into()))?
                .remove(secret_ref);
            Ok(())
        }
    }

    #[test]
    fn secret_lifecycle_uses_opaque_references() {
        let store = MemorySecretStore::default();
        assert!(store.get("provider/test/api-key").unwrap().is_none());
        store
            .put("provider/test/api-key", "test-only-not-a-real-key")
            .unwrap();
        assert_eq!(
            store
                .get("provider/test/api-key")
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("test-only-not-a-real-key")
        );
        store.delete("provider/test/api-key").unwrap();
        assert!(store.get("provider/test/api-key").unwrap().is_none());
    }

    #[test]
    fn secret_references_are_bounded_and_never_derived_from_secret_values() {
        assert!(OsCredentialStore::entry("").is_err());
        assert!(OsCredentialStore::entry(&"x".repeat(201)).is_err());
        assert!(OsCredentialStore::entry("provider/42/api-key").is_ok());
    }
}
