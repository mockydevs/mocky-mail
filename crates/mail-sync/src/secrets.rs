//! Passwords and tokens, kept in the operating system's keyring (GNOME
//! Keyring or `KWallet` via Secret Service, Windows Credential Manager, macOS
//! Keychain, or on Android an encrypted store whose key lives in the Android
//! Keystore). Nothing secret is ever written to the app's own files in the
//! clear.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

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

/// The operating system keyring. Secrets too long for one entry are split
/// over several (see [`MAX_PART_CHARS`]).
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

/// The longest piece of a secret kept in one keyring entry. Windows
/// Credential Manager holds at most 2,560 bytes of UTF-16 (1,280
/// characters), and a Microsoft sign-in token can be longer: stored whole,
/// adding or renewing such an account would fail on Windows.
pub const MAX_PART_CHARS: usize = 1_000;

/// The most pieces one secret may take (about 32,000 characters).
const MAX_PARTS: usize = 32;

/// Marks an entry that only says how many pieces a secret was split into.
/// No password or token starts with this control character.
const SPLIT_MARKER: &str = "\u{1}parts:";

fn part_key(key: &str, index: usize) -> String {
    format!("{key}#part{index}")
}

/// Remove the pieces of an earlier, longer secret from `first` on.
fn delete_parts(store: &dyn SecretStore, key: &str, first: usize) -> Result<(), SyncError> {
    for index in first..MAX_PARTS {
        let part = part_key(key, index);
        if store.get(&part)?.is_none() {
            break;
        }
        store.delete(&part)?;
    }
    Ok(())
}

/// Store a secret, split over several entries when it is too long for one.
fn set_split(store: &dyn SecretStore, key: &str, secret: &str) -> Result<(), SyncError> {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= MAX_PART_CHARS {
        store.set(key, secret)?;
        return delete_parts(store, key, 0);
    }
    let parts: Vec<String> = chars
        .chunks(MAX_PART_CHARS)
        .map(|chunk| chunk.iter().collect())
        .collect();
    if parts.len() > MAX_PARTS {
        return Err(SyncError::Keyring(
            "the sign-in is too long to keep in the system password store".to_owned(),
        ));
    }
    // The pieces first: the entry naming them is written last, so a reader
    // never finds it pointing at pieces that aren't there yet.
    for (index, part) in parts.iter().enumerate() {
        store.set(&part_key(key, index), part)?;
    }
    store.set(key, &format!("{SPLIT_MARKER}{}", parts.len()))?;
    delete_parts(store, key, parts.len())
}

/// Read a secret, joining its pieces if it was split.
fn get_split(store: &dyn SecretStore, key: &str) -> Result<Option<String>, SyncError> {
    let Some(stored) = store.get(key)? else {
        return Ok(None);
    };
    let Some(count) = stored.strip_prefix(SPLIT_MARKER) else {
        return Ok(Some(stored));
    };
    let count: usize = count
        .parse()
        .ok()
        .filter(|n| (1..=MAX_PARTS).contains(n))
        .ok_or_else(|| SyncError::Keyring("a saved sign-in is damaged".to_owned()))?;
    let mut secret = String::new();
    for index in 0..count {
        let part = store
            .get(&part_key(key, index))?
            .ok_or_else(|| SyncError::Keyring("part of a saved sign-in is missing".to_owned()))?;
        secret.push_str(&part);
    }
    Ok(Some(secret))
}

/// Remove a secret and any pieces it was split into.
fn delete_split(store: &dyn SecretStore, key: &str) -> Result<(), SyncError> {
    store.delete(key)?;
    delete_parts(store, key, 0)
}

/// One keyring entry per key, as the operating system stores it.
struct RawKeyring;

fn keyring_error(error: &keyring_core::Error) -> SyncError {
    SyncError::Keyring(error.to_string())
}

/// Pick this platform's store once. Errors are kept as text so every later
/// call reports the same reason.
static STORE_READY: LazyLock<Result<(), String>> = LazyLock::new(|| {
    #[cfg(target_os = "android")]
    {
        let store = android_native_keyring_store::Store::new().map_err(|e| e.to_string())?;
        keyring_core::set_default_store(store);
        Ok(())
    }
    #[cfg(not(target_os = "android"))]
    {
        // Sets the desktop default store as a side effect.
        keyring::Entry::store_status()
            .as_ref()
            .map(|&()| ())
            .map_err(ToString::to_string)
    }
});

fn entry(key: &str) -> Result<keyring_core::Entry, SyncError> {
    STORE_READY
        .as_ref()
        .map_err(|e| SyncError::Keyring(e.clone()))?;
    keyring_core::Entry::new(SERVICE, key).map_err(|e| keyring_error(&e))
}

impl SecretStore for KeyringStore {
    fn set(&self, key: &str, secret: &str) -> Result<(), SyncError> {
        set_split(&RawKeyring, key, secret)
    }

    fn get(&self, key: &str) -> Result<Option<String>, SyncError> {
        get_split(&RawKeyring, key)
    }

    fn delete(&self, key: &str) -> Result<(), SyncError> {
        delete_split(&RawKeyring, key)
    }
}

impl SecretStore for RawKeyring {
    fn set(&self, key: &str, secret: &str) -> Result<(), SyncError> {
        entry(key)?
            .set_password(secret)
            .map_err(|e| keyring_error(&e))
    }

    fn get(&self, key: &str) -> Result<Option<String>, SyncError> {
        match entry(key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_error(&e)),
        }
    }

    fn delete(&self, key: &str) -> Result<(), SyncError> {
        match entry(key)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
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

    #[test]
    fn long_secrets_are_split_and_joined_again() {
        let store = MemoryStore::default();
        // Longer than Windows allows in one entry, with characters beyond ASCII.
        let token: String = "é-0123456789".repeat(300);
        set_split(&store, "oauth:a", &token).unwrap();
        assert_eq!(
            get_split(&store, "oauth:a").unwrap().as_deref(),
            Some(token.as_str())
        );
        for index in 0..4 {
            let part = store.get(&part_key("oauth:a", index)).unwrap().unwrap();
            assert!(part.chars().count() <= MAX_PART_CHARS);
        }

        // A shorter secret replaces it and leaves no pieces behind.
        set_split(&store, "oauth:a", "short").unwrap();
        assert_eq!(
            get_split(&store, "oauth:a").unwrap().as_deref(),
            Some("short")
        );
        assert_eq!(store.get(&part_key("oauth:a", 0)).unwrap(), None);

        set_split(&store, "oauth:a", &token).unwrap();
        delete_split(&store, "oauth:a").unwrap();
        assert_eq!(get_split(&store, "oauth:a").unwrap(), None);
        assert_eq!(store.get(&part_key("oauth:a", 1)).unwrap(), None);
    }

    #[test]
    fn a_missing_piece_is_an_error_not_a_wrong_password() {
        let store = MemoryStore::default();
        set_split(&store, "k", &"x".repeat(2_500)).unwrap();
        store.delete(&part_key("k", 1)).unwrap();
        assert!(get_split(&store, "k").is_err());
        // Ordinary secrets are untouched.
        set_split(&store, "p", "hunter2").unwrap();
        assert_eq!(store.get("p").unwrap().as_deref(), Some("hunter2"));
    }
}
