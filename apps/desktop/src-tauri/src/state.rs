//! Shared state for commands and background tasks.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mail_store::Store;
use mail_sync::secrets::SecretStore;
use tokio::sync::Notify;

/// A running sync engine for one account.
pub struct EngineHandle {
    /// Wake the engine early (new local changes, "sync now").
    pub wake: Arc<Notify>,
    /// The engine task.
    pub task: tauri::async_runtime::JoinHandle<()>,
}

/// Everything the app shares.
pub struct AppState {
    /// Local database.
    pub store: Arc<Store>,
    /// Passwords and refresh tokens.
    pub secrets: Arc<dyn SecretStore>,
    /// Access tokens in memory only: account id → (token, expires at).
    pub access_tokens: Mutex<HashMap<String, (String, u64)>>,
    /// One sync engine per account.
    pub engines: Mutex<HashMap<String, EngineHandle>>,
    /// Wakes the outbox worker.
    pub outbox_wake: Arc<Notify>,
    /// Files the user picked in the open dialog this session. Only these
    /// may be attached, so a compromised page can't mail out arbitrary files.
    pub picked_files: Mutex<HashSet<PathBuf>>,
    /// Where temporary files (re-opened outbox attachments) go.
    pub cache_dir: PathBuf,
}

/// Lock a mutex, recovering from a panic elsewhere instead of failing.
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl AppState {
    /// New state around an opened store.
    pub fn new(store: Store, secrets: Arc<dyn SecretStore>, cache_dir: PathBuf) -> Self {
        Self {
            store: Arc::new(store),
            secrets,
            access_tokens: Mutex::new(HashMap::new()),
            engines: Mutex::new(HashMap::new()),
            outbox_wake: Arc::new(Notify::new()),
            picked_files: Mutex::new(HashSet::new()),
            cache_dir,
        }
    }

    /// Wake one account's engine, or all when `None`.
    pub fn wake_sync(&self, account_id: Option<&str>) {
        for (id, engine) in lock(&self.engines).iter() {
            if account_id.is_none_or(|wanted| wanted == id) {
                engine.wake.notify_one();
            }
        }
    }
}

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
