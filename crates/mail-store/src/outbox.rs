//! Messages waiting to be sent, with their delivery state.

use mail_core::message::Mailbox;
use mail_core::outbox::{OutboxItem, UnixSeconds};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Result, StoreError};
use crate::ids::{new_id, to_i64, to_u64};

/// A message to add to the outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOutbox {
    /// Sending account.
    pub account_id: String,
    /// When it was queued.
    pub created_at: UnixSeconds,
    /// Recipients, for the outbox list.
    pub to: Vec<Mailbox>,
    /// Subject, for the outbox list.
    pub subject: String,
    /// Start of the body, for the outbox list.
    pub preview: String,
    /// The exact RFC 822 bytes to send.
    pub raw: Vec<u8>,
    /// Its Message-ID, used to check Sent before resending.
    pub message_id_header: Option<String>,
    /// The message being replied to, marked answered after sending.
    pub reply_to_message_id: Option<String>,
    /// Sent folder to check or append to, if known.
    pub sent_folder_hint: Option<String>,
    /// Delivery state.
    pub item: OutboxItem,
}

/// An outbox entry without its raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEntry {
    /// Opaque ID.
    pub id: String,
    /// Sending account.
    pub account_id: String,
    /// When it was queued.
    pub created_at: UnixSeconds,
    /// Recipients.
    pub to: Vec<Mailbox>,
    /// Subject.
    pub subject: String,
    /// Start of the body.
    pub preview: String,
    /// Its Message-ID.
    pub message_id_header: Option<String>,
    /// The message being replied to.
    pub reply_to_message_id: Option<String>,
    /// Sent folder hint.
    pub sent_folder_hint: Option<String>,
    /// Delivery state.
    pub item: OutboxItem,
}

/// An outbox entry with the bytes to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxMessage {
    /// Everything but the bytes.
    pub entry: OutboxEntry,
    /// The RFC 822 message.
    pub raw: Vec<u8>,
}

macro_rules! entry_columns {
    () => {
        "id, account_id, created_at, to_json, subject, preview, message_id_header, \
         reply_to_message_id, sent_folder_hint, item"
    };
}

struct EntryRow {
    id: String,
    account_id: String,
    created_at: i64,
    to_json: String,
    subject: String,
    preview: String,
    message_id_header: Option<String>,
    reply_to_message_id: Option<String>,
    sent_folder_hint: Option<String>,
    item: String,
}

impl EntryRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            account_id: row.get(1)?,
            created_at: row.get(2)?,
            to_json: row.get(3)?,
            subject: row.get(4)?,
            preview: row.get(5)?,
            message_id_header: row.get(6)?,
            reply_to_message_id: row.get(7)?,
            sent_folder_hint: row.get(8)?,
            item: row.get(9)?,
        })
    }

    fn into_entry(self) -> Result<OutboxEntry> {
        Ok(OutboxEntry {
            id: self.id,
            account_id: self.account_id,
            created_at: to_u64(self.created_at, "outbox created_at")?,
            to: serde_json::from_str(&self.to_json)?,
            subject: self.subject,
            preview: self.preview,
            message_id_header: self.message_id_header,
            reply_to_message_id: self.reply_to_message_id,
            sent_folder_hint: self.sent_folder_hint,
            item: serde_json::from_str(&self.item)?,
        })
    }
}

impl Store {
    /// Add a message to the outbox. Returns its ID.
    ///
    /// # Errors
    /// Fails on a database error, for example an unknown account.
    pub fn insert_outbox(&self, new: &NewOutbox) -> Result<String> {
        let id = new_id()?;
        self.conn().execute(
            "INSERT INTO outbox (id, account_id, created_at, to_json, subject, preview, raw,
                 message_id_header, reply_to_message_id, sent_folder_hint, item)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                new.account_id,
                to_i64(new.created_at, "created_at")?,
                serde_json::to_string(&new.to)?,
                new.subject,
                new.preview,
                new.raw,
                new.message_id_header,
                new.reply_to_message_id,
                new.sent_folder_hint,
                serde_json::to_string(&new.item)?,
            ],
        )?;
        Ok(id)
    }

    /// Every outbox entry, oldest first, without the raw bytes.
    ///
    /// # Errors
    /// Fails on a database error or an unreadable row.
    pub fn list_outbox(&self) -> Result<Vec<OutboxEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(concat!(
            "SELECT ",
            entry_columns!(),
            " FROM outbox ORDER BY created_at, seq"
        ))?;
        stmt.query_map([], EntryRow::read)?
            .map(|row| row?.into_entry())
            .collect()
    }

    /// One outbox entry with its raw bytes.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such entry.
    pub fn get_outbox(&self, id: &str) -> Result<OutboxMessage> {
        let (row, raw) = self
            .conn()
            .query_row(
                concat!(
                    "SELECT ",
                    entry_columns!(),
                    ", raw FROM outbox WHERE id = ?1"
                ),
                params![id],
                |row| Ok((EntryRow::read(row)?, row.get::<_, Vec<u8>>(10)?)),
            )
            .optional()?
            .ok_or(StoreError::NotFound("outbox item"))?;
        Ok(OutboxMessage {
            entry: row.into_entry()?,
            raw,
        })
    }

    /// Save a new delivery state.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such entry.
    pub fn update_outbox_item(&self, id: &str, item: &OutboxItem) -> Result<()> {
        let changed = self.conn().execute(
            "UPDATE outbox SET item = ?2 WHERE id = ?1",
            params![id, serde_json::to_string(item)?],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound("outbox item"));
        }
        Ok(())
    }

    /// Remove an entry. Returns whether it existed.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn delete_outbox(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM outbox WHERE id = ?1", params![id])?
            > 0)
    }

    /// Call once at startup: every entry that was mid-send when the app
    /// stopped goes back to the queue, marked "maybe sent" so it is checked
    /// against Sent before any resend. All entries change in one
    /// transaction. Returns how many were recovered.
    ///
    /// # Errors
    /// Fails on a database error or an unreadable entry; nothing changes.
    pub fn recover_outbox_after_restart(&self, now: UnixSeconds) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let rows = {
            let mut stmt = tx.prepare("SELECT id, item FROM outbox")?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut recovered = 0usize;
        {
            let mut update = tx.prepare("UPDATE outbox SET item = ?2 WHERE id = ?1")?;
            for (id, json) in rows {
                let mut item: OutboxItem = serde_json::from_str(&json)?;
                if item.recover_after_restart(now) {
                    update.execute(params![id, serde_json::to_string(&item)?])?;
                    recovered = recovered.saturating_add(1);
                }
            }
        }
        tx.commit()?;
        Ok(recovered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{add_account, new_outbox};
    use mail_core::outbox::OutboxState;

    #[test]
    fn insert_get_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let new = new_outbox(&account);
        let id = store.insert_outbox(&new).unwrap();
        let got = store.get_outbox(&id).unwrap();
        assert_eq!(got.raw, new.raw);
        assert_eq!(got.entry.id, id);
        assert_eq!(got.entry.to, new.to);
        assert_eq!(got.entry.subject, new.subject);
        assert_eq!(got.entry.message_id_header, new.message_id_header);
        assert_eq!(got.entry.sent_folder_hint, new.sent_folder_hint);
        assert_eq!(got.entry.reply_to_message_id, new.reply_to_message_id);
        assert_eq!(got.entry.item, new.item);
        assert_eq!(got.entry.created_at, new.created_at);
    }

    #[test]
    fn list_is_oldest_first() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let mut late = new_outbox(&account);
        late.created_at = 50;
        let mut early = new_outbox(&account);
        early.created_at = 10;
        let late_id = store.insert_outbox(&late).unwrap();
        let early_id = store.insert_outbox(&early).unwrap();
        let ids: Vec<String> = store
            .list_outbox()
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, [early_id, late_id]);
    }

    #[test]
    fn update_and_delete() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let id = store.insert_outbox(&new_outbox(&account)).unwrap();
        let mut item = OutboxItem::new(100);
        item.begin_send(100).unwrap();
        store.update_outbox_item(&id, &item).unwrap();
        assert_eq!(store.get_outbox(&id).unwrap().entry.item, item);
        assert!(matches!(
            store.update_outbox_item("nope", &item),
            Err(StoreError::NotFound(_))
        ));
        assert!(store.delete_outbox(&id).unwrap());
        assert!(!store.delete_outbox(&id).unwrap());
        assert!(matches!(
            store.get_outbox(&id),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn insert_for_unknown_account_fails() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.insert_outbox(&new_outbox("nope")).is_err());
    }

    #[test]
    fn recovery_requeues_only_messages_mid_send() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let sending = store.insert_outbox(&new_outbox(&account)).unwrap();
        let queued = store.insert_outbox(&new_outbox(&account)).unwrap();
        let mut item = OutboxItem::new(100);
        item.begin_send(100).unwrap();
        store.update_outbox_item(&sending, &item).unwrap();

        assert_eq!(store.recover_outbox_after_restart(500).unwrap(), 1);
        let recovered = store.get_outbox(&sending).unwrap().entry.item;
        assert_eq!(
            recovered.state,
            OutboxState::Queued {
                next_attempt_at: 500,
                maybe_sent: true
            }
        );
        assert_eq!(recovered.attempts, 0);
        assert_eq!(
            store.get_outbox(&queued).unwrap().entry.item,
            OutboxItem::new(100)
        );
        // Running it again changes nothing.
        assert_eq!(store.recover_outbox_after_restart(600).unwrap(), 0);
    }

    #[test]
    fn recovery_is_all_or_nothing() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let sending = store.insert_outbox(&new_outbox(&account)).unwrap();
        let mut item = OutboxItem::new(100);
        item.begin_send(100).unwrap();
        store.update_outbox_item(&sending, &item).unwrap();
        let broken = store.insert_outbox(&new_outbox(&account)).unwrap();
        store
            .conn()
            .execute(
                "UPDATE outbox SET item = 'not json' WHERE id = ?1",
                params![broken],
            )
            .unwrap();
        assert!(matches!(
            store.recover_outbox_after_restart(500),
            Err(StoreError::Json(_))
        ));
        assert_eq!(store.get_outbox(&sending).unwrap().entry.item, item);
    }
}
