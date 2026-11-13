//! Passwords and tokens, kept in the operating system's keyring (GNOME
//! Keyring or `KWallet` via Secret Service, Windows Credential Manager, macOS
//! Keychain). Nothing secret is ever written to the app's own files.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::SyncError;

/// The service name every entry is stored under.
pub const SERVICE: &str = "Mocky Mail";

/// Somewhere to keep secrets, keyed by a string such as an account id.
pub trait SecretStore: Send + Sync {
    /// Store or replace a secret.
    ///
    /// # Errors
    /// When the store is unavailable or refuses the write.
    fn set(&self, key: &str, secret: &str) -> Result<(), SyncError>;
    /// Read a secret; `Ok(None)` when there is none.
    ///
    /// # Errors
    /// When the store is unavailable.
    fn get(&self, key: &str) -> Result<Option<String>, SyncError>;
    /// Remove a secret. Removing a missing secret is not an error.
    ///
    /// # Errors
    /// When the store is unavailable.
    fn delete(&self, key: &str) -> Result<(), SyncError>;
}

/// The operating system keyring.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

fn keyring_error(error: &keyring::Error) -> SyncError {
    SyncError::Keyring(error.to_string())
}

impl SecretStore for KeyringStore {
    fn set(&self, key: &str, secret: &str) -> Result<(), SyncError> {
        keyring::Entry::new(SERVICE, key)
            .and_then(|entry| entry.set_password(secret))
            .map_err(|e| keyring_error(&e))
    }

    fn get(&self, key: &str) -> Result<Option<String>, SyncError> {
        let entry = keyring::Entry::new(SERVICE, key).map_err(|e| keyring_error(&e))?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_error(&e)),
        }
    }

    fn delete(&self, key: &str) -> Result<(), SyncError> {
        let entry = keyring::Entry::new(SERVICE, key).map_err(|e| keyring_error(&e))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_error(&e)),
        }
    }
}

/// An in-memory store for tests and for running without a keyring.
#[derive(Debug, Default)]
pub struct MemoryStore {
    secrets: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, String>>, SyncError> {
        self.secrets
            .lock()
            .map_err(|_| SyncError::Keyring("secret store lock poisoned".to_owned()))
    }
}

impl SecretStore for MemoryStore {
    fn set(&self, key: &str, secret: &str) -> Result<(), SyncError> {
        self.lock()?.insert(key.to_owned(), secret.to_owned());
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<String>, SyncError> {
        Ok(self.lock()?.get(key).cloned())
    }

    fn delete(&self, key: &str) -> Result<(), SyncError> {
        self.lock()?.remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trip() {
        let store = MemoryStore::default();
        assert_eq!(store.get("a").unwrap(), None);
        store.set("a", "s3cret").unwrap();
        assert_eq!(store.get("a").unwrap().as_deref(), Some("s3cret"));
        store.delete("a").unwrap();
        store.delete("a").unwrap();
        assert_eq!(store.get("a").unwrap(), None);
    }
}
