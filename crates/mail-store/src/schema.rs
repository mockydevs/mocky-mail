//! Schema migrations, tracked with `PRAGMA user_version`.
//!
//! Each entry in [`MIGRATIONS`] moves the schema up one version. Pending
//! migrations run together in one transaction, so a crash mid-upgrade
//! leaves the file at the old version. Never edit a released migration; add
//! a new one.

use rusqlite::Connection;

use crate::error::{Result, StoreError};

/// Version 1: the initial schema.
const V1: &str = r"
CREATE TABLE accounts (
    id               TEXT PRIMARY KEY NOT NULL,
    email            TEXT NOT NULL,
    display_name     TEXT NOT NULL,
    auth             TEXT NOT NULL CHECK (auth IN ('password', 'google', 'microsoft')),
    color            TEXT NOT NULL,
    imap_host        TEXT NOT NULL,
    imap_port        INTEGER NOT NULL,
    imap_security    TEXT NOT NULL CHECK (imap_security IN ('tls', 'starttls')),
    smtp_host        TEXT NOT NULL,
    smtp_port        INTEGER NOT NULL,
    smtp_security    TEXT NOT NULL CHECK (smtp_security IN ('tls', 'starttls')),
    size_limit_bytes INTEGER,
    created_at       INTEGER NOT NULL
);

CREATE TABLE folders (
    id          TEXT PRIMARY KEY NOT NULL,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    path        TEXT NOT NULL,
    name        TEXT NOT NULL,
    role        TEXT NOT NULL,
    delimiter   TEXT,
    uidvalidity INTEGER,
    highest_uid INTEGER,
    UNIQUE (account_id, path)
);

-- seq is an explicit INTEGER PRIMARY KEY so it survives VACUUM; the search
-- index refers to messages by it.
CREATE TABLE messages (
    seq               INTEGER PRIMARY KEY,
    id                TEXT NOT NULL UNIQUE,
    account_id        TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id         TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    uid               INTEGER,
    message_id_header TEXT,
    subject           TEXT NOT NULL DEFAULT '',
    from_name         TEXT,
    from_address      TEXT,
    to_json           TEXT NOT NULL DEFAULT '[]',
    cc_json           TEXT NOT NULL DEFAULT '[]',
    reply_to_json     TEXT NOT NULL DEFAULT '[]',
    references_json   TEXT NOT NULL DEFAULT '[]',
    attachments_json  TEXT NOT NULL DEFAULT '[]',
    in_reply_to       TEXT,
    date              INTEGER,
    preview           TEXT NOT NULL DEFAULT '',
    has_attachments   INTEGER NOT NULL DEFAULT 0,
    seen              INTEGER NOT NULL DEFAULT 0,
    flagged           INTEGER NOT NULL DEFAULT 0,
    answered          INTEGER NOT NULL DEFAULT 0,
    size              INTEGER NOT NULL DEFAULT 0,
    raw               BLOB,
    body_text         TEXT,
    UNIQUE (folder_id, uid)
);
CREATE INDEX messages_folder_date ON messages (folder_id, date DESC);
CREATE INDEX messages_account_date ON messages (account_id, date DESC);
CREATE INDEX messages_date ON messages (date DESC);
CREATE INDEX messages_message_id ON messages (account_id, message_id_header);

CREATE VIRTUAL TABLE messages_fts USING fts5 (
    subject, sender, recipients, preview, body,
    content = '',
    contentless_delete = 1,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts (rowid, subject, sender, recipients, preview, body)
    VALUES (
        new.seq,
        new.subject,
        coalesce(new.from_name, '') || ' ' || coalesce(new.from_address, ''),
        (SELECT coalesce(group_concat(
            coalesce(json_extract(value, '$.name'), '') || ' ' ||
            coalesce(json_extract(value, '$.address'), ''), ' '), '')
         FROM json_each(new.to_json)),
        new.preview,
        coalesce(new.body_text, '')
    );
END;

CREATE TRIGGER messages_fts_delete AFTER DELETE ON messages BEGIN
    DELETE FROM messages_fts WHERE rowid = old.seq;
END;

CREATE TRIGGER messages_fts_update
AFTER UPDATE OF subject, from_name, from_address, to_json, preview, body_text ON messages
WHEN old.subject IS NOT new.subject
  OR old.from_name IS NOT new.from_name
  OR old.from_address IS NOT new.from_address
  OR old.to_json IS NOT new.to_json
  OR old.preview IS NOT new.preview
  OR old.body_text IS NOT new.body_text
BEGIN
    DELETE FROM messages_fts WHERE rowid = old.seq;
    INSERT INTO messages_fts (rowid, subject, sender, recipients, preview, body)
    VALUES (
        new.seq,
        new.subject,
        coalesce(new.from_name, '') || ' ' || coalesce(new.from_address, ''),
        (SELECT coalesce(group_concat(
            coalesce(json_extract(value, '$.name'), '') || ' ' ||
            coalesce(json_extract(value, '$.address'), ''), ' '), '')
         FROM json_each(new.to_json)),
        new.preview,
        coalesce(new.body_text, '')
    );
END;

CREATE TABLE pending_ops (
    seq        INTEGER PRIMARY KEY,
    id         TEXT NOT NULL UNIQUE,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('set_flags', 'move', 'delete')),
    payload    TEXT NOT NULL
);
CREATE INDEX pending_ops_account ON pending_ops (account_id, created_at, seq);

CREATE TABLE outbox (
    seq                 INTEGER PRIMARY KEY,
    id                  TEXT NOT NULL UNIQUE,
    account_id          TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at          INTEGER NOT NULL,
    to_json             TEXT NOT NULL,
    subject             TEXT NOT NULL,
    preview             TEXT NOT NULL,
    raw                 BLOB NOT NULL,
    recipients_json     TEXT NOT NULL DEFAULT '[]',
    message_id_header   TEXT,
    reply_to_message_id TEXT,
    sent_folder_hint    TEXT,
    item                TEXT NOT NULL
);
CREATE INDEX outbox_account ON outbox (account_id);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
";

/// All migrations in order. Index `n` upgrades version `n` to `n + 1`.
pub(crate) const MIGRATIONS: &[&str] = &[V1];

/// Highest schema version this build understands.
pub(crate) fn latest_version() -> i64 {
    i64::try_from(MIGRATIONS.len()).unwrap_or(i64::MAX)
}

/// Bring the schema up to date. Safe to call on every open.
pub(crate) fn migrate(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    let current: i64 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let latest = latest_version();
    if current > latest {
        return Err(StoreError::SchemaTooNew {
            found: current,
            supported: latest,
        });
    }
    let start = usize::try_from(current).map_err(|_| StoreError::Corrupt("user_version"))?;
    for sql in MIGRATIONS.iter().skip(start) {
        tx.execute_batch(sql)?;
    }
    if current != latest {
        tx.pragma_update(None, "user_version", latest)?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    fn table_count(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type IN ('table', 'trigger', 'index')",
            [],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn migrates_an_empty_database() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(version(&conn), 0);
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), latest_version());
        assert!(table_count(&conn) > 0);
    }

    #[test]
    fn migrating_twice_is_a_no_op() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let objects = table_count(&conn);
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), latest_version());
        assert_eq!(table_count(&conn), objects);
    }

    #[test]
    fn refuses_a_newer_schema() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", latest_version() + 1)
            .unwrap();
        assert!(matches!(
            migrate(&mut conn),
            Err(StoreError::SchemaTooNew { .. })
        ));
    }

    #[test]
    fn a_failed_migration_rolls_back() {
        let mut conn = Connection::open_in_memory().unwrap();
        // Occupy a name the first migration creates, so it fails part way.
        conn.execute_batch("CREATE TABLE settings (x)").unwrap();
        assert!(migrate(&mut conn).is_err());
        assert_eq!(version(&conn), 0);
        let accounts: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name = 'accounts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(accounts, 0);
    }

    #[test]
    fn fts5_and_json_are_available() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE VIRTUAL TABLE t USING fts5(a)")
            .unwrap();
        let n: i64 = conn
            .query_row("SELECT json_array_length('[1,2]')", [], |row| row.get(0))
            .unwrap();
        assert_eq!(n, 2);
    }
}
