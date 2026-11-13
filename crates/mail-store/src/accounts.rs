//! Mail accounts and their server settings. Passwords and tokens are never
//! stored here; they live in the OS keyring.

use mail_core::outbox::UnixSeconds;
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::Store;
use crate::error::{Result, StoreError};
use crate::ids::{new_id, to_i64, to_u64};

/// How the account signs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    /// Username and password (or app password).
    Password,
    /// Google OAuth.
    Google,
    /// Microsoft OAuth.
    Microsoft,
}

impl AuthKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Google => "google",
            Self::Microsoft => "microsoft",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "password" => Ok(Self::Password),
            "google" => Ok(Self::Google),
            "microsoft" => Ok(Self::Microsoft),
            _ => Err(StoreError::Corrupt("account auth")),
        }
    }
}

/// How a connection to a server is secured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Security {
    /// TLS from the first byte.
    Tls,
    /// Plain connection upgraded with STARTTLS.
    Starttls,
}

impl Security {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::Starttls => "starttls",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "tls" => Ok(Self::Tls),
            "starttls" => Ok(Self::Starttls),
            _ => Err(StoreError::Corrupt("server security")),
        }
    }
}

/// One IMAP or SMTP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Host name.
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// Connection security.
    pub security: Security,
}

/// Everything needed to save a new account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAccount {
    /// Email address.
    pub email: String,
    /// Name shown on sent mail.
    pub display_name: String,
    /// How the account signs in.
    pub auth: AuthKind,
    /// Hex colour from the app's palette.
    pub color: String,
    /// Incoming server.
    pub imap: ServerConfig,
    /// Outgoing server.
    pub smtp: ServerConfig,
    /// Largest message the SMTP server accepts, if it said.
    pub size_limit_bytes: Option<u64>,
    /// When the account was added.
    pub created_at: UnixSeconds,
}

/// A saved account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// Opaque ID.
    pub id: String,
    /// Email address.
    pub email: String,
    /// Name shown on sent mail.
    pub display_name: String,
    /// How the account signs in.
    pub auth: AuthKind,
    /// Hex colour from the app's palette.
    pub color: String,
    /// Incoming server.
    pub imap: ServerConfig,
    /// Outgoing server.
    pub smtp: ServerConfig,
    /// Largest message the SMTP server accepts, if known.
    pub size_limit_bytes: Option<u64>,
    /// When the account was added.
    pub created_at: UnixSeconds,
}

/// Column list shared by the account queries (a literal, so the SQL stays static).
macro_rules! account_columns {
    () => {
        "id, email, display_name, auth, color, imap_host, imap_port, imap_security, \
         smtp_host, smtp_port, smtp_security, size_limit_bytes, created_at"
    };
}

struct AccountRow {
    id: String,
    email: String,
    display_name: String,
    auth: String,
    color: String,
    imap: (String, u16, String),
    smtp: (String, u16, String),
    size_limit_bytes: Option<i64>,
    created_at: i64,
}

impl AccountRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            email: row.get(1)?,
            display_name: row.get(2)?,
            auth: row.get(3)?,
            color: row.get(4)?,
            imap: (row.get(5)?, row.get(6)?, row.get(7)?),
            smtp: (row.get(8)?, row.get(9)?, row.get(10)?),
            size_limit_bytes: row.get(11)?,
            created_at: row.get(12)?,
        })
    }

    fn into_account(self) -> Result<Account> {
        Ok(Account {
            id: self.id,
            email: self.email,
            display_name: self.display_name,
            auth: AuthKind::parse(&self.auth)?,
            color: self.color,
            imap: ServerConfig {
                host: self.imap.0,
                port: self.imap.1,
                security: Security::parse(&self.imap.2)?,
            },
            smtp: ServerConfig {
                host: self.smtp.0,
                port: self.smtp.1,
                security: Security::parse(&self.smtp.2)?,
            },
            size_limit_bytes: self
                .size_limit_bytes
                .map(|v| to_u64(v, "size limit"))
                .transpose()?,
            created_at: to_u64(self.created_at, "account created_at")?,
        })
    }
}

impl Store {
    /// Save a new account and return it with its ID.
    ///
    /// # Errors
    /// Fails on a database error or a value too large to store.
    pub fn insert_account(&self, new: &NewAccount) -> Result<Account> {
        let id = new_id()?;
        let size_limit = new
            .size_limit_bytes
            .map(|v| to_i64(v, "size limit"))
            .transpose()?;
        let created_at = to_i64(new.created_at, "created_at")?;
        self.conn().execute(
            "INSERT INTO accounts (id, email, display_name, auth, color, imap_host, imap_port,
                 imap_security, smtp_host, smtp_port, smtp_security, size_limit_bytes, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id,
                new.email,
                new.display_name,
                new.auth.as_str(),
                new.color,
                new.imap.host,
                new.imap.port,
                new.imap.security.as_str(),
                new.smtp.host,
                new.smtp.port,
                new.smtp.security.as_str(),
                size_limit,
                created_at,
            ],
        )?;
        self.get_account(&id)
    }

    /// All accounts, oldest first.
    ///
    /// # Errors
    /// Fails on a database error or an unreadable row.
    pub fn list_accounts(&self) -> Result<Vec<Account>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(concat!(
            "SELECT ",
            account_columns!(),
            " FROM accounts ORDER BY created_at, rowid"
        ))?;
        let rows = stmt.query_map([], AccountRow::read)?;
        rows.map(|row| row?.into_account()).collect()
    }

    /// One account by ID.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such account.
    pub fn get_account(&self, id: &str) -> Result<Account> {
        self.conn()
            .query_row(
                concat!(
                    "SELECT ",
                    account_columns!(),
                    " FROM accounts WHERE id = ?1"
                ),
                params![id],
                AccountRow::read,
            )
            .optional()?
            .ok_or(StoreError::NotFound("account"))?
            .into_account()
    }

    /// Change the name shown on sent mail.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such account.
    pub fn update_account_display_name(&self, id: &str, display_name: &str) -> Result<()> {
        let changed = self.conn().execute(
            "UPDATE accounts SET display_name = ?2 WHERE id = ?1",
            params![id, display_name],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound("account"));
        }
        Ok(())
    }

    /// Remove an account and everything stored for it: folders, messages,
    /// outbox items and pending offline actions.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such account.
    pub fn delete_account(&self, id: &str) -> Result<()> {
        let changed = self
            .conn()
            .execute("DELETE FROM accounts WHERE id = ?1", params![id])?;
        if changed == 0 {
            return Err(StoreError::NotFound("account"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{add_account, add_folder, add_message, new_account, summary};
    use mail_core::folders::FolderRole;

    #[test]
    fn insert_and_get_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let mut new = new_account("a@example.com");
        new.auth = AuthKind::Google;
        new.size_limit_bytes = Some(25_000_000);
        new.smtp.security = Security::Starttls;
        let saved = store.insert_account(&new).unwrap();
        assert_eq!(saved.id.len(), 32);
        assert_eq!(saved.email, "a@example.com");
        assert_eq!(saved.auth, AuthKind::Google);
        assert_eq!(saved.size_limit_bytes, Some(25_000_000));
        assert_eq!(saved.smtp.security, Security::Starttls);
        assert_eq!(saved.imap, new.imap);
        assert_eq!(saved.created_at, new.created_at);
        assert_eq!(store.get_account(&saved.id).unwrap(), saved);
    }

    #[test]
    fn get_unknown_account_is_not_found() {
        let store = Store::open_in_memory().unwrap();
        assert!(matches!(
            store.get_account("nope"),
            Err(StoreError::NotFound("account"))
        ));
    }

    #[test]
    fn rejects_size_limit_that_does_not_fit() {
        let store = Store::open_in_memory().unwrap();
        let mut new = new_account("a@example.com");
        new.size_limit_bytes = Some(u64::MAX);
        assert!(matches!(
            store.insert_account(&new),
            Err(StoreError::Invalid(_))
        ));
    }

    #[test]
    fn lists_accounts_oldest_first() {
        let store = Store::open_in_memory().unwrap();
        let mut second = new_account("b@example.com");
        second.created_at = 200;
        let mut first = new_account("a@example.com");
        first.created_at = 100;
        store.insert_account(&second).unwrap();
        store.insert_account(&first).unwrap();
        let emails: Vec<String> = store
            .list_accounts()
            .unwrap()
            .into_iter()
            .map(|a| a.email)
            .collect();
        assert_eq!(emails, ["a@example.com", "b@example.com"]);
    }

    #[test]
    fn updates_display_name() {
        let store = Store::open_in_memory().unwrap();
        let id = add_account(&store, "a@example.com");
        store.update_account_display_name(&id, "Ann B").unwrap();
        assert_eq!(store.get_account(&id).unwrap().display_name, "Ann B");
        assert!(matches!(
            store.update_account_display_name("nope", "x"),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn account_serializes_like_the_ipc_contract() {
        let store = Store::open_in_memory().unwrap();
        let id = add_account(&store, "a@example.com");
        let json = serde_json::to_value(store.get_account(&id).unwrap()).unwrap();
        assert_eq!(json["displayName"], "Test User");
        assert_eq!(json["auth"], "password");
        assert_eq!(json["imap"]["security"], "tls");
        assert!(json["sizeLimitBytes"].is_null());
    }

    #[test]
    fn delete_cascades_to_everything_the_account_owns() {
        let store = Store::open_in_memory().unwrap();
        let gone = add_account(&store, "gone@example.com");
        let kept = add_account(&store, "kept@example.com");
        for account in [&gone, &kept] {
            let folder = add_folder(&store, account, "INBOX", FolderRole::Inbox);
            add_message(&store, &folder, 1, &summary("hello cascade", 10));
            store
                .enqueue_op(account, crate::OpKind::Delete, &serde_json::json!({}), 1)
                .unwrap();
            store
                .insert_outbox(&crate::test_support::new_outbox(account))
                .unwrap();
        }
        store.delete_account(&gone).unwrap();

        assert!(matches!(
            store.get_account(&gone),
            Err(StoreError::NotFound(_))
        ));
        assert!(store.list_folders(&gone).unwrap().is_empty());
        assert!(store.list_ops(&gone).unwrap().is_empty());
        assert_eq!(store.list_outbox().unwrap().len(), 1);
        let conn = store.conn();
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(count("SELECT count(*) FROM messages"), 1);
        assert_eq!(count("SELECT count(*) FROM folders"), 1);
        // The search index lost the deleted account's message too.
        assert_eq!(count("SELECT count(*) FROM messages_fts"), 1);
        drop(conn);
        assert_eq!(store.list_folders(&kept).unwrap().len(), 1);
        assert!(matches!(
            store.delete_account(&gone),
            Err(StoreError::NotFound(_))
        ));
    }
}
