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
    /// Every envelope recipient, Bcc included. Bcc never appears in `raw`,
    /// so this list is the only record of who to deliver to.
    pub recipients: Vec<String>,
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
    /// Envelope recipients, Bcc included.
    pub recipients: Vec<String>,
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

    /// Never fails: one damaged row must not hide the rest of the Outbox or
    /// stop the app. Unreadable fields get safe stand-ins, and an unreadable
    /// delivery state becomes [`OutboxItem::damaged`], which never sends by
    /// itself.
    fn into_entry(self) -> OutboxEntry {
        let item = read_item(&self.item);
        OutboxEntry {
            id: self.id,
            account_id: self.account_id,
            created_at: to_u64(self.created_at, "outbox created_at").unwrap_or(0),
            to: serde_json::from_str(&self.to_json).unwrap_or_default(),
            subject: self.subject,
            preview: self.preview,
            message_id_header: self.message_id_header,
            reply_to_message_id: self.reply_to_message_id,
            sent_folder_hint: self.sent_folder_hint,
            item,
        }
    }
}

impl Store {
    /// Add a message to the outbox. Returns its ID.
    ///
    /// # Errors
    /// Fails on a database error, for example an unknown account.
    pub fn insert_outbox(&self, new: &NewOutbox) -> Result<String> {
        let id = new_id()?;
        let created_at = to_i64(new.created_at, "created_at")?;
        let to_json = serde_json::to_string(&new.to)?;
        let item = serde_json::to_string(&new.item)?;
        let recipients = serde_json::to_string(&new.recipients)?;
        durably(&self.conn(), |conn| {
            conn.execute(
                "INSERT INTO outbox (id, account_id, created_at, to_json, subject, preview, raw,
                 message_id_header, reply_to_message_id, sent_folder_hint, item, recipients_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id,
                    new.account_id,
                    created_at,
                    to_json,
                    new.subject,
                    new.preview,
                    new.raw,
                    new.message_id_header,
                    new.reply_to_message_id,
                    new.sent_folder_hint,
                    item,
                    recipients,
                ],
            )
        })?;
        Ok(id)
    }

    /// Every outbox entry, oldest first, without the raw bytes.
    ///
    /// # Errors
    /// Fails on a database error. A damaged row is listed with stand-ins
    /// (see `EntryRow::into_entry`), never an error.
    pub fn list_outbox(&self) -> Result<Vec<OutboxEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(concat!(
            "SELECT ",
            entry_columns!(),
            " FROM outbox ORDER BY created_at, seq"
        ))?;
        stmt.query_map([], EntryRow::read)?
            .map(|row| Ok(row?.into_entry()))
            .collect()
    }

    /// One outbox entry with its raw bytes.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such entry.
    pub fn get_outbox(&self, id: &str) -> Result<OutboxMessage> {
        let (row, raw, recipients_json) = self
            .conn()
            .query_row(
                concat!(
                    "SELECT ",
                    entry_columns!(),
                    ", raw, recipients_json FROM outbox WHERE id = ?1"
                ),
                params![id],
                |row| {
                    Ok((
                        EntryRow::read(row)?,
                        row.get::<_, Vec<u8>>(10)?,
                        row.get::<_, String>(11)?,
                    ))
                },
            )
            .optional()?
            .ok_or(StoreError::NotFound("outbox item"))?;
        Ok(OutboxMessage {
            entry: row.into_entry(),
            raw,
            recipients: serde_json::from_str(&recipients_json)?,
        })
    }

    /// Save a new delivery state.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such entry.
    pub fn update_outbox_item(&self, id: &str, item: &OutboxItem) -> Result<()> {
        let json = serde_json::to_string(item)?;
        let changed = durably(&self.conn(), |conn| {
            conn.execute(
                "UPDATE outbox SET item = ?2 WHERE id = ?1",
                params![id, json],
            )
        })?;
        if changed == 0 {
            return Err(StoreError::NotFound("outbox item"));
        }
        Ok(())
    }

    /// Replace an outbox item only if it is still `expected`, in one step.
    ///
    /// Returns `false`, changing nothing, when the entry is gone or someone
    /// else changed it first. The worker and the user (cancel, edit, send
    /// now) both change items; this stops one from overwriting the other,
    /// for example a cancel landing just as sending starts.
    ///
    /// # Errors
    /// On database or JSON errors.
    pub fn replace_outbox_item(
        &self,
        id: &str,
        expected: &OutboxItem,
        new: &OutboxItem,
    ) -> Result<bool> {
        let conn = self.conn();
        if !outbox_item_is(&conn, id, expected)? {
            return Ok(false);
        }
        let json = serde_json::to_string(new)?;
        durably(&conn, |conn| {
            conn.execute(
                "UPDATE outbox SET item = ?2 WHERE id = ?1",
                params![id, json],
            )
        })?;
        Ok(true)
    }

    /// Delete an outbox entry only if its item is still `expected`.
    /// Returns `false`, deleting nothing, when it is gone or has changed
    /// (for example, sending has started).
    ///
    /// # Errors
    /// On database or JSON errors.
    pub fn delete_outbox_if(&self, id: &str, expected: &OutboxItem) -> Result<bool> {
        let conn = self.conn();
        if !outbox_item_is(&conn, id, expected)? {
            return Ok(false);
        }
        durably(&conn, |conn| {
            conn.execute("DELETE FROM outbox WHERE id = ?1", params![id])
        })?;
        Ok(true)
    }

    /// Remove an entry. Returns whether it existed.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn delete_outbox(&self, id: &str) -> Result<bool> {
        let deleted = durably(&self.conn(), |conn| {
            conn.execute("DELETE FROM outbox WHERE id = ?1", params![id])
        })?;
        Ok(deleted > 0)
    }

    /// Call once at startup: every entry that was mid-send when the app
    /// stopped goes back to the queue, marked "maybe sent" so it is checked
    /// against Sent before any resend. An entry whose state can't be read is
    /// rewritten as [`OutboxItem::damaged`], so it shows in the Outbox for
    /// the user to decide on and can't stop the app from starting. All
    /// entries change in one transaction. Returns how many changed.
    ///
    /// # Errors
    /// Fails on a database error; nothing changes.
    pub fn recover_outbox_after_restart(&self, now: UnixSeconds) -> Result<usize> {
        let mut conn = self.conn();
        conn.pragma_update(None, "synchronous", "FULL")?;
        let result = recover_all(&mut conn, now);
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        result
    }
}

fn recover_all(conn: &mut rusqlite::Connection, now: UnixSeconds) -> Result<usize> {
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
            let (mut item, damaged) = match serde_json::from_str::<OutboxItem>(&json) {
                Ok(item) => (item, false),
                Err(_) => (OutboxItem::damaged(), true),
            };
            if item.recover_after_restart(now) || damaged {
                update.execute(params![id, serde_json::to_string(&item)?])?;
                recovered = recovered.saturating_add(1);
            }
        }
    }
    tx.commit()?;
    Ok(recovered)
}

/// A stored delivery state, or [`OutboxItem::damaged`] if it can't be read.
fn read_item(json: &str) -> OutboxItem {
    serde_json::from_str(json).unwrap_or_else(|_| OutboxItem::damaged())
}

/// Run one Outbox write with a full sync to disk.
///
/// The database normally runs with `synchronous=NORMAL`: fast, and safe
/// against the app crashing, but a power cut can lose the last few commits.
/// For the Outbox that could mean a queued message vanishing, a cancel
/// being forgotten, or a send that was claimed being forgotten and sent
/// twice. These writes are rare, so they pay for an fsync. The caller holds
/// the connection lock, so nothing else runs with the stricter setting.
fn durably<T>(
    conn: &rusqlite::Connection,
    write: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T>,
) -> Result<T> {
    conn.pragma_update(None, "synchronous", "FULL")?;
    let result = write(conn);
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(result?)
}

/// Whether the stored item for `id` exists and equals `expected`. The caller
/// holds the connection lock, so nothing can change it before the write.
fn outbox_item_is(conn: &rusqlite::Connection, id: &str, expected: &OutboxItem) -> Result<bool> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT item FROM outbox WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    match stored {
        None => Ok(false),
        Some(json) => Ok(read_item(&json) == *expected),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{add_account, new_outbox};
    use mail_core::outbox::OutboxState;

    #[test]
    fn envelope_recipients_including_bcc_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let account = crate::test_support::add_account(&store, "a@x.co");
        let id = store
            .insert_outbox(&crate::test_support::new_outbox(&account))
            .unwrap();
        let message = store.get_outbox(&id).unwrap();
        assert_eq!(
            message.recipients,
            vec!["to@x.co".to_owned(), "hidden@x.co".to_owned()]
        );
    }

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
    fn a_damaged_entry_does_not_stop_startup_or_hide_the_others() {
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
                "UPDATE outbox SET item = 'not json', to_json = '{' WHERE id = ?1",
                params![broken],
            )
            .unwrap();

        // Listing works before recovery too: the bad row reads as damaged.
        let listed = store.list_outbox().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(
            listed.iter().find(|e| e.id == broken).unwrap().item,
            OutboxItem::damaged()
        );

        // Startup recovery succeeds, fixes the mid-send one and saves the
        // damaged one as "unconfirmed" so the user can retry or delete it.
        assert_eq!(store.recover_outbox_after_restart(500).unwrap(), 2);
        assert!(matches!(
            store.get_outbox(&sending).unwrap().entry.item.state,
            OutboxState::Queued {
                maybe_sent: true,
                ..
            }
        ));
        let damaged = store.get_outbox(&broken).unwrap().entry;
        assert_eq!(damaged.item, OutboxItem::damaged());
        assert!(damaged.to.is_empty());
        assert!(!damaged.item.is_due(u64::MAX));
        // The saved copy is now readable, so cancel/retry can compare it.
        let mut retry = damaged.item.clone();
        retry.send_now(600).unwrap();
        assert!(
            store
                .replace_outbox_item(&broken, &damaged.item, &retry)
                .unwrap()
        );
        assert_eq!(store.recover_outbox_after_restart(700).unwrap(), 0);
    }

    #[test]
    fn outbox_writes_leave_normal_sync_mode_afterwards() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let id = store.insert_outbox(&new_outbox(&account)).unwrap();
        store.delete_outbox(&id).unwrap();
        store.recover_outbox_after_restart(1).unwrap();
        // 1 = NORMAL. Only the Outbox write itself runs with FULL.
        let mode: i64 = store
            .conn()
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, 1);
    }

    #[test]
    fn replace_only_when_unchanged() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let id = store.insert_outbox(&new_outbox(&account)).unwrap();
        let queued = store.get_outbox(&id).unwrap().entry.item;
        let mut sending = queued.clone();
        sending.begin_send(100).unwrap();
        assert!(store.replace_outbox_item(&id, &queued, &sending).unwrap());
        // A second writer still holding the old Queued copy loses.
        let mut stale = queued.clone();
        stale.send_now(200).unwrap();
        assert!(!store.replace_outbox_item(&id, &queued, &stale).unwrap());
        assert_eq!(store.get_outbox(&id).unwrap().entry.item, sending);
        assert!(
            !store
                .replace_outbox_item("nope", &queued, &sending)
                .unwrap()
        );
    }

    #[test]
    fn cancel_loses_to_a_send_that_already_started() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let id = store.insert_outbox(&new_outbox(&account)).unwrap();
        // The user's cancel read the item while it was still queued...
        let seen_by_cancel = store.get_outbox(&id).unwrap().entry.item;
        // ...then the worker started sending it.
        let mut sending = seen_by_cancel.clone();
        sending.begin_send(100).unwrap();
        assert!(
            store
                .replace_outbox_item(&id, &seen_by_cancel, &sending)
                .unwrap()
        );
        assert!(!store.delete_outbox_if(&id, &seen_by_cancel).unwrap());
        assert!(store.get_outbox(&id).is_ok());
        // A cancel that sees the current state goes through.
        let mut failed = sending.clone();
        failed
            .finish(
                mail_core::outbox::SendOutcome::Permanent("no".into()),
                110,
                &mail_core::outbox::RetryPolicy::default(),
            )
            .unwrap();
        assert!(store.replace_outbox_item(&id, &sending, &failed).unwrap());
        assert!(store.delete_outbox_if(&id, &failed).unwrap());
        assert!(!store.delete_outbox_if(&id, &failed).unwrap());
    }
}
