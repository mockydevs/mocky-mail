//! Local `SQLite` database for Mocky Mail.
//!
//! The store keeps accounts, folders, message headers and cached message
//! bodies, a full-text search index, the outbox and offline actions waiting
//! to be replayed on the server. It is the app's source of truth for what
//! is shown; the sync engine fills it from IMAP.
//!
//! One [`Store`] wraps one connection behind a mutex, so it is `Send` and
//! `Sync` and can be shared with `Arc`. Every query is parameterized; no
//! SQL is ever built from user text.

mod accounts;
mod error;
mod folders;
mod ids;
mod messages;
mod ops;
mod outbox;
mod schema;
mod search;
mod settings;
#[cfg(test)]
mod test_support;

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use rusqlite::Connection;

pub use accounts::{Account, AuthKind, NewAccount, Security, ServerConfig};
pub use error::{Result, StoreError};
pub use folders::{Folder, NewFolder};
pub use messages::{
    ListFilter, ListQuery, MAX_PAGE_SIZE, MessageFlags, MessageListItem, MessagePage, StoredMessage,
};
pub use ops::{OpKind, PendingOp};
pub use outbox::{NewOutbox, OutboxEntry, OutboxMessage};

/// How long a writer waits for another connection's lock before failing.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The local mail database.
#[derive(Debug)]
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (or create) the database file at `path` and bring its schema up
    /// to date.
    ///
    /// # Errors
    /// Fails if the file cannot be opened, is not a database, or was written
    /// by a newer version of the app.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    /// A fresh database in memory, for tests and previews.
    ///
    /// # Errors
    /// Fails only if `SQLite` cannot allocate the database.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        schema::migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Exclusive access to the connection. A panic in another thread while
    /// it held the lock does not poison the store: any open transaction was
    /// rolled back when it was dropped.
    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn store_is_send_and_sync() {
        assert_send_sync::<Store>();
    }

    #[test]
    fn opens_in_memory_with_foreign_keys_on() {
        let store = Store::open_in_memory().unwrap();
        let on: bool = store
            .conn()
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert!(on);
    }

    #[test]
    fn opens_a_file_in_wal_mode_and_reopens_it() {
        let dir = std::env::temp_dir().join(format!("mail-store-test-{}", ids::new_id().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mail.db");
        {
            let store = Store::open(&path).unwrap();
            let mode: String = store
                .conn()
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
            let timeout: i64 = store
                .conn()
                .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
                .unwrap();
            assert_eq!(timeout, 5000);
            store.set_setting("theme", "dark").unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.get_setting("theme").unwrap().as_deref(), Some("dark"));
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_panic_while_locked_does_not_break_the_store() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let clone = Arc::clone(&store);
        let result = std::thread::spawn(move || {
            let _guard = clone.conn();
            panic!("boom");
        })
        .join();
        assert!(result.is_err());
        store.set_setting("k", "v").unwrap();
        assert_eq!(store.get_setting("k").unwrap().as_deref(), Some("v"));
    }
}
