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

/// Version 2: count how often the server has refused each queued change, so
/// a refusal is retried a few times before the change is given up.
const V2: &str = r"
ALTER TABLE pending_ops ADD COLUMN refusals INTEGER NOT NULL DEFAULT 0;
";

/// Version 3: the address book. `card` is the contact as JSON; `vcard` the
/// server's copy, so fields the app doesn't show survive an edit.
const V3: &str = r"
CREATE TABLE contacts (
    id         TEXT PRIMARY KEY NOT NULL,
    account_id TEXT REFERENCES accounts(id) ON DELETE CASCADE,
    href       TEXT,
    etag       TEXT,
    vcard      TEXT,
    card       TEXT NOT NULL,
    state      TEXT NOT NULL CHECK (state IN ('synced', 'changed', 'deleted')),
    sort_name  TEXT NOT NULL,
    search     TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX contacts_href ON contacts (account_id, href) WHERE href IS NOT NULL;
CREATE INDEX contacts_sort ON contacts (sort_name COLLATE NOCASE);
";

/// Version 4: conversations. A message's thread is the first id in its
/// References (the conversation's first message), else what it replies
/// to, else its own Message-ID: the headers every mail client writes.
const V4: &str = r"
ALTER TABLE messages ADD COLUMN thread_key TEXT;
UPDATE messages SET thread_key = lower(trim(coalesce(
    nullif(json_extract(references_json, '$[0]'), ''),
    nullif(in_reply_to, ''),
    nullif(message_id_header, ''),
    id), '<> '));
CREATE INDEX messages_thread ON messages (account_id, thread_key);
CREATE TRIGGER messages_thread_key_insert AFTER INSERT ON messages BEGIN
    UPDATE messages SET thread_key = lower(trim(coalesce(
        nullif(json_extract(new.references_json, '$[0]'), ''),
        nullif(new.in_reply_to, ''),
        nullif(new.message_id_header, ''),
        new.id), '<> '))
    WHERE seq = new.seq;
END;
-- Rows are inserted first and their headers filled in after.
CREATE TRIGGER messages_thread_key_update
AFTER UPDATE OF references_json, in_reply_to, message_id_header ON messages BEGIN
    UPDATE messages SET thread_key = lower(trim(coalesce(
        nullif(json_extract(new.references_json, '$[0]'), ''),
        nullif(new.in_reply_to, ''),
        nullif(new.message_id_header, ''),
        new.id), '<> '))
    WHERE seq = new.seq;
END;
";

/// Version 5: Focused Inbox. `bulk` marks mailing-list and automated
/// mail (filled as messages are synced); `focus_rules` holds the user's
/// "always Focused / always Other" choices by address or `@domain`.
const V5: &str = r"
ALTER TABLE messages ADD COLUMN bulk INTEGER NOT NULL DEFAULT 0;
CREATE TABLE focus_rules (
    sender TEXT PRIMARY KEY NOT NULL,
    focus  TEXT NOT NULL CHECK (focus IN ('focused', 'other'))
);
";

/// Version 6: calendars (`CalDAV`). Events are kept as the server's
/// iCalendar text; `starts_at`/`ends_at` (Unix seconds, `NULL` for a
/// series without an end) find a month's events without reading them all.
const V6: &str = r"
CREATE TABLE calendars (
    id         TEXT PRIMARY KEY NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    url        TEXT NOT NULL,
    name       TEXT NOT NULL,
    color      TEXT,
    UNIQUE (account_id, url)
);
CREATE TABLE calendar_events (
    id          TEXT PRIMARY KEY NOT NULL,
    calendar_id TEXT NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    href        TEXT NOT NULL,
    etag        TEXT,
    ics         TEXT NOT NULL,
    starts_at   INTEGER,
    ends_at     INTEGER,
    UNIQUE (calendar_id, href)
);
CREATE INDEX calendar_events_time ON calendar_events (starts_at, ends_at);
";

/// Version 7: mail handling (0.5.0).
///
/// - `accounts.sort_order`: the user's order in the sidebar; NULL until
///   they reorder, and for accounts added after (those go last, oldest
///   first). `sync_months`: months of mail kept in full; NULL for all.
/// - `folders.favorite`: listed under Favourites.
/// - `messages.pinned`, `flag_due` (Unix seconds) and `categories` (JSON
///   array of names, in the order they were added) are the user's;
///   `importance` comes from the headers unless `importance_local` says it
///   was set here.
/// - `categories`: name (unique ignoring case) and colour.
/// - `pending_ops` is rebuilt so its kinds include `copy`.
const V7: &str = r"
ALTER TABLE accounts ADD COLUMN sort_order INTEGER;
ALTER TABLE accounts ADD COLUMN sync_months INTEGER;
ALTER TABLE folders ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0;
ALTER TABLE messages ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
ALTER TABLE messages ADD COLUMN importance TEXT NOT NULL DEFAULT 'normal'
    CHECK (importance IN ('low', 'normal', 'high'));
ALTER TABLE messages ADD COLUMN importance_local INTEGER NOT NULL DEFAULT 0;
ALTER TABLE messages ADD COLUMN categories TEXT NOT NULL DEFAULT '[]';
ALTER TABLE messages ADD COLUMN flag_due INTEGER;
CREATE INDEX messages_flag_due ON messages (flag_due) WHERE flag_due IS NOT NULL;

CREATE TABLE categories (
    name  TEXT PRIMARY KEY NOT NULL COLLATE NOCASE,
    color TEXT NOT NULL
);

CREATE TABLE pending_ops_v7 (
    seq        INTEGER PRIMARY KEY,
    id         TEXT NOT NULL UNIQUE,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('set_flags', 'move', 'delete', 'copy')),
    payload    TEXT NOT NULL,
    refusals   INTEGER NOT NULL DEFAULT 0
);
INSERT INTO pending_ops_v7 (seq, id, account_id, created_at, kind, payload, refusals)
    SELECT seq, id, account_id, created_at, kind, payload, refusals FROM pending_ops;
DROP TABLE pending_ops;
ALTER TABLE pending_ops_v7 RENAME TO pending_ops;
CREATE INDEX pending_ops_account ON pending_ops (account_id, created_at, seq);
";

/// Version 8: search and rules (0.6.0).
///
/// - The search index gains a `cc` column (for `cc:`), so it is rebuilt
///   from the messages and its triggers recreated.
/// - `rules`: the user's mail rules. `spec` is the rule as JSON
///   (`mail_core::rules::RuleSpec`); `account_id` is NULL for every
///   account; `server_status` and `server_note` say whether the mail
///   server runs it too.
/// - `messages.rules_done`: rules have run on this new message (so a copy
///   a rule made, arriving later, doesn't set them off again).
/// - `image_senders`: addresses (or domains, `whole_domain`) whose remote
///   pictures always load.
const V8: &str = r"
DROP TRIGGER messages_fts_insert;
DROP TRIGGER messages_fts_delete;
DROP TRIGGER messages_fts_update;
DROP TABLE messages_fts;
CREATE VIRTUAL TABLE messages_fts USING fts5 (
    subject, sender, recipients, preview, body, cc,
    content = '',
    contentless_delete = 1,
    tokenize = 'unicode61 remove_diacritics 2'
);
INSERT INTO messages_fts (rowid, subject, sender, recipients, preview, body, cc)
SELECT
    m.seq,
    m.subject,
    coalesce(m.from_name, '') || ' ' || coalesce(m.from_address, ''),
    (SELECT coalesce(group_concat(
        coalesce(json_extract(value, '$.name'), '') || ' ' ||
        coalesce(json_extract(value, '$.address'), ''), ' '), '')
     FROM json_each(m.to_json)),
    m.preview,
    coalesce(m.body_text, ''),
    (SELECT coalesce(group_concat(
        coalesce(json_extract(value, '$.name'), '') || ' ' ||
        coalesce(json_extract(value, '$.address'), ''), ' '), '')
     FROM json_each(m.cc_json))
FROM messages m;

CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts (rowid, subject, sender, recipients, preview, body, cc)
    VALUES (
        new.seq,
        new.subject,
        coalesce(new.from_name, '') || ' ' || coalesce(new.from_address, ''),
        (SELECT coalesce(group_concat(
            coalesce(json_extract(value, '$.name'), '') || ' ' ||
            coalesce(json_extract(value, '$.address'), ''), ' '), '')
         FROM json_each(new.to_json)),
        new.preview,
        coalesce(new.body_text, ''),
        (SELECT coalesce(group_concat(
            coalesce(json_extract(value, '$.name'), '') || ' ' ||
            coalesce(json_extract(value, '$.address'), ''), ' '), '')
         FROM json_each(new.cc_json))
    );
END;

CREATE TRIGGER messages_fts_delete AFTER DELETE ON messages BEGIN
    DELETE FROM messages_fts WHERE rowid = old.seq;
END;

CREATE TRIGGER messages_fts_update
AFTER UPDATE OF subject, from_name, from_address, to_json, cc_json, preview, body_text ON messages
WHEN old.subject IS NOT new.subject
  OR old.from_name IS NOT new.from_name
  OR old.from_address IS NOT new.from_address
  OR old.to_json IS NOT new.to_json
  OR old.cc_json IS NOT new.cc_json
  OR old.preview IS NOT new.preview
  OR old.body_text IS NOT new.body_text
BEGIN
    DELETE FROM messages_fts WHERE rowid = old.seq;
    INSERT INTO messages_fts (rowid, subject, sender, recipients, preview, body, cc)
    VALUES (
        new.seq,
        new.subject,
        coalesce(new.from_name, '') || ' ' || coalesce(new.from_address, ''),
        (SELECT coalesce(group_concat(
            coalesce(json_extract(value, '$.name'), '') || ' ' ||
            coalesce(json_extract(value, '$.address'), ''), ' '), '')
         FROM json_each(new.to_json)),
        new.preview,
        coalesce(new.body_text, ''),
        (SELECT coalesce(group_concat(
            coalesce(json_extract(value, '$.name'), '') || ' ' ||
            coalesce(json_extract(value, '$.address'), ''), ' '), '')
         FROM json_each(new.cc_json))
    );
END;

CREATE TABLE rules (
    id            TEXT PRIMARY KEY NOT NULL,
    account_id    TEXT REFERENCES accounts(id) ON DELETE CASCADE,
    sort_order    INTEGER NOT NULL,
    spec          TEXT NOT NULL,
    server_status TEXT NOT NULL DEFAULT 'local'
        CHECK (server_status IN ('local', 'server', 'unsupported')),
    server_note   TEXT
);
CREATE INDEX rules_order ON rules (sort_order, id);

ALTER TABLE messages ADD COLUMN rules_done INTEGER NOT NULL DEFAULT 0;

CREATE TABLE image_senders (
    sender       TEXT PRIMARY KEY NOT NULL,
    whole_domain INTEGER NOT NULL DEFAULT 0
);
";

/// Version 9: compose (0.7.0).
///
/// - `read_receipts`: read receipts asked for by received messages that
///   were answered here (sent, or declined when `sent` is 0), so each is
///   asked about and sent once. Keyed by the account and the message's
///   Message-ID (`id:` and the local id when it has none), so moving the
///   message to another folder doesn't ask again.
const V9: &str = r"
CREATE TABLE read_receipts (
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    message_key TEXT NOT NULL,
    sent        INTEGER NOT NULL,
    answered_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, message_key)
);
";

/// All migrations in order. Index `n` upgrades version `n` to `n + 1`.
pub(crate) const MIGRATIONS: &[&str] = &[V1, V2, V3, V4, V5, V6, V7, V8, V9];

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
    fn a_version_one_database_keeps_its_queued_changes() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(V1).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute_batch(
            "INSERT INTO accounts VALUES ('a', 'a@x.co', 'A', 'password', '#000',
                 'imap.x.co', 993, 'tls', 'smtp.x.co', 465, 'tls', NULL, 0);
             INSERT INTO pending_ops (id, account_id, created_at, kind, payload)
                 VALUES ('op', 'a', 1, 'move', '{}');",
        )
        .unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), latest_version());
        let refusals: i64 = conn
            .query_row(
                "SELECT refusals FROM pending_ops WHERE id = 'op'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(refusals, 0);
    }

    #[test]
    fn a_version_six_database_keeps_its_queued_changes_and_takes_copies() {
        let mut conn = Connection::open_in_memory().unwrap();
        for sql in MIGRATIONS.iter().take(6) {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 6).unwrap();
        conn.execute_batch(
            "INSERT INTO accounts VALUES ('a', 'a@x.co', 'A', 'password', '#000',
                 'imap.x.co', 993, 'tls', 'smtp.x.co', 465, 'tls', NULL, 0);
             INSERT INTO pending_ops (id, account_id, created_at, kind, payload, refusals)
                 VALUES ('op', 'a', 1, 'move', '{}', 2);",
        )
        .unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), latest_version());
        let (kind, refusals): (String, i64) = conn
            .query_row(
                "SELECT kind, refusals FROM pending_ops WHERE id = 'op'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((kind.as_str(), refusals), ("move", 2));
        conn.execute(
            "INSERT INTO pending_ops (id, account_id, created_at, kind, payload)
             VALUES ('copy', 'a', 2, 'copy', '{}')",
            [],
        )
        .unwrap();
        let order: Option<i64> = conn
            .query_row("SELECT sort_order FROM accounts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(order, None);
        // Removing the account still removes its queued changes.
        conn.execute_batch("PRAGMA foreign_keys = ON; DELETE FROM accounts;")
            .unwrap();
        let left: i64 = conn
            .query_row("SELECT count(*) FROM pending_ops", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn a_version_seven_database_keeps_its_search_and_gains_cc() {
        let mut conn = Connection::open_in_memory().unwrap();
        for sql in MIGRATIONS.iter().take(7) {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 7).unwrap();
        conn.execute_batch(
            r#"INSERT INTO accounts (id, email, display_name, auth, color, imap_host, imap_port,
                 imap_security, smtp_host, smtp_port, smtp_security, size_limit_bytes, created_at)
                 VALUES ('a', 'a@x.co', 'A', 'password', '#000',
                 'imap.x.co', 993, 'tls', 'smtp.x.co', 465, 'tls', NULL, 0);
             INSERT INTO folders (id, account_id, path, name, role) VALUES ('f', 'a', 'INBOX', 'Inbox', 'inbox');
             INSERT INTO messages (id, account_id, folder_id, subject, cc_json, body_text)
                 VALUES ('m', 'a', 'f', 'Quarterly numbers',
                 '[{"name":"Carol Kamau","address":"carol@client.example"}]', 'see attached');"#,
        )
        .unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), latest_version());
        let hits = |q: &str| -> i64 {
            conn.query_row(
                "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH ?1",
                [q],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(hits("\"quarter\"*"), 1);
        assert_eq!(hits("body : \"attached\""), 1);
        assert_eq!(hits("cc : \"kamau\""), 1);
        assert_eq!(hits("sender : \"kamau\""), 0);
        // The recreated triggers keep the index up to date.
        conn.execute(
            "UPDATE messages SET cc_json = '[{\"name\":null,\"address\":\"dan@x.co\"}]' WHERE id = 'm'",
            [],
        )
        .unwrap();
        assert_eq!(hits("cc : \"kamau\""), 0);
        assert_eq!(hits("cc : \"dan\""), 1);
        conn.execute("DELETE FROM messages", []).unwrap();
        assert_eq!(hits("\"quarter\"*"), 0);
        // The new tables and column.
        conn.execute_batch(
            "INSERT INTO rules (id, account_id, sort_order, spec) VALUES ('r', 'a', 0, '{}');
             INSERT INTO image_senders (sender, whole_domain) VALUES ('x.co', 1);",
        )
        .unwrap();
        let status: String = conn
            .query_row("SELECT server_status FROM rules", [], |row| row.get(0))
            .unwrap();
        assert_eq!(status, "local");
        conn.execute_batch("PRAGMA foreign_keys = ON; DELETE FROM accounts;")
            .unwrap();
        let rules: i64 = conn
            .query_row("SELECT count(*) FROM rules", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rules, 0);
    }

    #[test]
    fn a_version_eight_database_gains_read_receipts() {
        let mut conn = Connection::open_in_memory().unwrap();
        for sql in MIGRATIONS.iter().take(8) {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 8).unwrap();
        conn.execute_batch(
            r"INSERT INTO accounts (id, email, display_name, auth, color, imap_host, imap_port,
                 imap_security, smtp_host, smtp_port, smtp_security, size_limit_bytes, created_at)
                 VALUES ('a', 'a@x.co', 'A', 'password', '#000',
                 'imap.x.co', 993, 'tls', 'smtp.x.co', 465, 'tls', NULL, 0);",
        )
        .unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), latest_version());
        conn.execute_batch(
            "INSERT INTO read_receipts (account_id, message_key, sent, answered_at) VALUES ('a', 'q@x', 1, 5);",
        )
        .unwrap();
        // Once per message.
        assert!(
            conn.execute_batch(
                "INSERT INTO read_receipts (account_id, message_key, sent, answered_at) VALUES ('a', 'q@x', 0, 6);",
            )
            .is_err()
        );
        conn.execute_batch("PRAGMA foreign_keys = ON; DELETE FROM accounts;")
            .unwrap();
        let left: i64 = conn
            .query_row("SELECT count(*) FROM read_receipts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0);
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
