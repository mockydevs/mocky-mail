//! Message headers, flags, cached raw bodies and the message list query.

use mail_core::folders::FolderRole;
use mail_core::message::{AttachmentInfo, Mailbox, MessageSummary};
use rusqlite::{OptionalExtension, Row, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::Store;
use crate::error::{Result, StoreError};
use crate::folders::role_str;
use crate::ids::{new_id, to_i64, to_u32, to_u64};
use crate::search::{SearchTerms, fts_query};

/// Largest page [`Store::list_messages`] returns; larger limits are capped.
pub const MAX_PAGE_SIZE: u32 = 200;

/// IMAP flags the app tracks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MessageFlags {
    /// `\Seen`.
    pub seen: bool,
    /// `\Flagged`.
    pub flagged: bool,
    /// `\Answered`.
    pub answered: bool,
}

/// Which messages of the selection to show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListFilter {
    /// Everything.
    #[default]
    All,
    /// Only messages not yet seen.
    Unread,
    /// Only flagged messages.
    Flagged,
}

impl ListFilter {
    const fn code(self) -> i64 {
        match self {
            Self::All => 0,
            Self::Unread => 1,
            Self::Flagged => 2,
        }
    }
}

/// A page request for the message list (the interface's `ListQuery`).
/// Every selector is optional and they combine with AND.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    /// One account, or `None` for all accounts (unified views).
    pub account_id: Option<String>,
    /// One folder.
    pub folder_id: Option<String>,
    /// Folders with this role, across the selected accounts.
    pub role: Option<FolderRole>,
    /// Unread or flagged only.
    pub filter: ListFilter,
    /// Full-text search over subject, sender, recipients, preview and body.
    pub search: Option<String>,
    /// Messages to skip.
    pub offset: u32,
    /// Page size, capped at [`MAX_PAGE_SIZE`].
    pub limit: u32,
}

/// One row of the message list.
// The flags mirror the interface's `MessageListItem` field for field.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageListItem {
    /// Opaque ID.
    pub id: String,
    /// Owning account.
    pub account_id: String,
    /// Folder it is in.
    pub folder_id: String,
    /// Sender.
    pub from: Option<Mailbox>,
    /// Recipients in To.
    pub to: Vec<Mailbox>,
    /// Subject.
    pub subject: String,
    /// Start of the body.
    pub preview: String,
    /// Date header as Unix seconds.
    pub date: Option<i64>,
    /// Not yet seen.
    pub unread: bool,
    /// Flagged.
    pub flagged: bool,
    /// Replied to.
    pub answered: bool,
    /// Has attachments.
    pub has_attachments: bool,
}

/// A page of the message list and how many messages match in total.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessagePage {
    /// The requested page, newest first.
    pub items: Vec<MessageListItem>,
    /// All matching messages, ignoring offset and limit.
    pub total: u64,
}

/// Everything stored about one message except its raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMessage {
    /// The list fields.
    pub item: MessageListItem,
    /// Server UID in its folder, or `None` while a local move awaits the
    /// server.
    pub uid: Option<u32>,
    /// The Message-ID header.
    pub message_id_header: Option<String>,
    /// Recipients in Cc.
    pub cc: Vec<Mailbox>,
    /// Reply-To addresses.
    pub reply_to: Vec<Mailbox>,
    /// Thread Message-IDs, oldest first.
    pub references: Vec<String>,
    /// The Message-ID this replies to.
    pub in_reply_to: Option<String>,
    /// Attachments known from the headers.
    pub attachments: Vec<AttachmentInfo>,
    /// Size on the server in bytes.
    pub size: u64,
    /// Whether the full message is cached locally.
    pub has_raw: bool,
}

macro_rules! item_columns {
    () => {
        "m.id, m.account_id, m.folder_id, m.from_name, m.from_address, m.to_json, m.subject, \
         m.preview, m.date, m.seen, m.flagged, m.answered, m.has_attachments"
    };
}

/// Selection shared by the page and the count. `?1` account, `?2` folder,
/// `?3` role, `?4` filter code, `?5` FTS query; a NULL selector matches all.
macro_rules! list_where {
    () => {
        " FROM messages m JOIN folders f ON f.id = m.folder_id
          WHERE (?1 IS NULL OR m.account_id = ?1)
            AND (?2 IS NULL OR m.folder_id = ?2)
            AND (?3 IS NULL OR f.role = ?3)
            AND (?4 = 0 OR (?4 = 1 AND m.seen = 0) OR (?4 = 2 AND m.flagged = 1))
            AND (?5 IS NULL OR m.seq IN
                 (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?5))"
    };
}

// One field per selected column.
#[allow(clippy::struct_excessive_bools)]
struct ItemRow {
    id: String,
    account_id: String,
    folder_id: String,
    from_name: Option<String>,
    from_address: Option<String>,
    to_json: String,
    subject: String,
    preview: String,
    date: Option<i64>,
    seen: bool,
    flagged: bool,
    answered: bool,
    has_attachments: bool,
}

impl ItemRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            account_id: row.get(1)?,
            folder_id: row.get(2)?,
            from_name: row.get(3)?,
            from_address: row.get(4)?,
            to_json: row.get(5)?,
            subject: row.get(6)?,
            preview: row.get(7)?,
            date: row.get(8)?,
            seen: row.get(9)?,
            flagged: row.get(10)?,
            answered: row.get(11)?,
            has_attachments: row.get(12)?,
        })
    }

    fn into_item(self) -> Result<MessageListItem> {
        Ok(MessageListItem {
            id: self.id,
            account_id: self.account_id,
            folder_id: self.folder_id,
            from: self.from_address.map(|address| Mailbox {
                name: self.from_name,
                address,
            }),
            to: serde_json::from_str(&self.to_json)?,
            subject: self.subject,
            preview: self.preview,
            date: self.date,
            unread: !self.seen,
            flagged: self.flagged,
            answered: self.answered,
            has_attachments: self.has_attachments,
        })
    }
}

struct DetailRow {
    item: ItemRow,
    uid: Option<i64>,
    message_id_header: Option<String>,
    cc_json: String,
    reply_to_json: String,
    references_json: String,
    in_reply_to: Option<String>,
    size: i64,
    attachments_json: String,
    has_raw: bool,
}

impl DetailRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            item: ItemRow::read(row)?,
            uid: row.get(13)?,
            message_id_header: row.get(14)?,
            cc_json: row.get(15)?,
            reply_to_json: row.get(16)?,
            references_json: row.get(17)?,
            in_reply_to: row.get(18)?,
            size: row.get(19)?,
            attachments_json: row.get(20)?,
            has_raw: row.get(21)?,
        })
    }

    fn into_message(self) -> Result<StoredMessage> {
        Ok(StoredMessage {
            item: self.item.into_item()?,
            uid: self.uid.map(|v| to_u32(v, "uid")).transpose()?,
            message_id_header: self.message_id_header,
            cc: serde_json::from_str(&self.cc_json)?,
            reply_to: serde_json::from_str(&self.reply_to_json)?,
            references: serde_json::from_str(&self.references_json)?,
            in_reply_to: self.in_reply_to,
            attachments: serde_json::from_str(&self.attachments_json)?,
            size: to_u64(self.size, "message size")?,
            has_raw: self.has_raw,
        })
    }
}

/// Header columns encoded for storage.
struct EncodedSummary<'a> {
    summary: &'a MessageSummary,
    to: String,
    cc: String,
    reply_to: String,
    references: String,
    attachments: String,
}

impl<'a> EncodedSummary<'a> {
    fn new(summary: &'a MessageSummary) -> Result<Self> {
        Ok(Self {
            summary,
            to: serde_json::to_string(&summary.to)?,
            cc: serde_json::to_string(&summary.cc)?,
            reply_to: serde_json::to_string(&summary.reply_to)?,
            references: serde_json::to_string(&summary.references)?,
            attachments: serde_json::to_string(&summary.attachments)?,
        })
    }
}

/// Overwrite a row's server-side fields (never `raw` or `body_text`).
fn update_headers(
    tx: &Transaction<'_>,
    id: &str,
    uid: u32,
    enc: &EncodedSummary<'_>,
    flags: MessageFlags,
    size: i64,
) -> Result<()> {
    let s = enc.summary;
    tx.execute(
        "UPDATE messages SET uid = ?2, message_id_header = ?3, subject = ?4, from_name = ?5,
             from_address = ?6, to_json = ?7, cc_json = ?8, reply_to_json = ?9,
             references_json = ?10, attachments_json = ?11, in_reply_to = ?12, date = ?13,
             preview = ?14, has_attachments = ?15, seen = ?16, flagged = ?17, answered = ?18,
             size = ?19
         WHERE id = ?1",
        params![
            id,
            uid,
            s.message_id,
            s.subject,
            s.from.as_ref().and_then(|m| m.name.as_deref()),
            s.from.as_ref().map(|m| m.address.as_str()),
            enc.to,
            enc.cc,
            enc.reply_to,
            enc.references,
            enc.attachments,
            s.in_reply_to,
            s.date,
            s.preview,
            !s.attachments.is_empty(),
            flags.seen,
            flags.flagged,
            flags.answered,
            size,
        ],
    )?;
    Ok(())
}

/// Strip one pair of surrounding angle brackets from a Message-ID.
fn bare_message_id(header: &str) -> &str {
    let trimmed = header.trim();
    trimmed
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(trimmed)
}

impl Store {
    /// Save a message's headers and flags by folder and UID, returning its
    /// ID. An existing row keeps its ID and its cached raw body. A row that
    /// was moved here locally (no UID yet) with the same Message-ID is
    /// adopted instead of duplicated.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if the folder does not exist;
    /// [`StoreError::Invalid`] if `size` does not fit.
    pub fn upsert_message(
        &self,
        folder_id: &str,
        uid: u32,
        summary: &MessageSummary,
        flags: MessageFlags,
        size: u64,
    ) -> Result<String> {
        let size = to_i64(size, "message size")?;
        let enc = EncodedSummary::new(summary)?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let account_id: String = tx
            .query_row(
                "SELECT account_id FROM folders WHERE id = ?1",
                params![folder_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound("folder"))?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM messages WHERE folder_id = ?1 AND uid = ?2",
                params![folder_id, uid],
                |row| row.get(0),
            )
            .optional()?;
        let existing = match (existing, summary.message_id.as_deref()) {
            (Some(id), _) => Some(id),
            (None, Some(message_id)) => tx
                .query_row(
                    "SELECT id FROM messages
                     WHERE folder_id = ?1 AND uid IS NULL AND message_id_header = ?2
                     ORDER BY seq LIMIT 1",
                    params![folder_id, message_id],
                    |row| row.get(0),
                )
                .optional()?,
            (None, None) => None,
        };
        let id = if let Some(id) = existing {
            id
        } else {
            let id = new_id()?;
            tx.execute(
                "INSERT INTO messages (id, account_id, folder_id) VALUES (?1, ?2, ?3)",
                params![id, account_id, folder_id],
            )?;
            id
        };
        update_headers(&tx, &id, uid, &enc, flags, size)?;
        tx.commit()?;
        Ok(id)
    }

    /// Cache the full RFC 822 message and its plain text (for search).
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such message.
    pub fn set_raw(&self, id: &str, raw: &[u8], body_text: Option<&str>) -> Result<()> {
        let changed = self.conn().execute(
            "UPDATE messages SET raw = ?2, body_text = ?3 WHERE id = ?1",
            params![id, raw, body_text],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound("message"));
        }
        Ok(())
    }

    /// The cached RFC 822 message, or `None` if it has not been fetched.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such message.
    pub fn get_raw(&self, id: &str) -> Result<Option<Vec<u8>>> {
        self.conn()
            .query_row(
                "SELECT raw FROM messages WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound("message"))
    }

    /// One message by ID.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such message.
    pub fn get_message(&self, id: &str) -> Result<StoredMessage> {
        self.conn()
            .query_row(
                concat!(
                    "SELECT ",
                    item_columns!(),
                    ", m.uid, m.message_id_header, m.cc_json, m.reply_to_json,
                     m.references_json, m.in_reply_to, m.size, m.attachments_json,
                     m.raw IS NOT NULL
                     FROM messages m WHERE m.id = ?1"
                ),
                params![id],
                DetailRow::read,
            )
            .optional()?
            .ok_or(StoreError::NotFound("message"))?
            .into_message()
    }

    /// A page of the message list, newest first (undated last), plus the
    /// total number of matches.
    ///
    /// # Errors
    /// Fails on a database error or an unreadable row. Search text can
    /// never cause an error: it is escaped before reaching FTS5.
    pub fn list_messages(&self, query: &ListQuery) -> Result<MessagePage> {
        let search = match fts_query(query.search.as_deref()) {
            SearchTerms::None => None,
            SearchTerms::Nothing => {
                return Ok(MessagePage {
                    items: Vec::new(),
                    total: 0,
                });
            }
            SearchTerms::Match(expr) => Some(expr),
        };
        let role = query.role.map(role_str);
        let limit = i64::from(query.limit.min(MAX_PAGE_SIZE));
        let offset = i64::from(query.offset);
        let selectors = params![
            query.account_id,
            query.folder_id,
            role,
            query.filter.code(),
            search
        ];
        let conn = self.conn();
        let total: i64 = conn
            .prepare_cached(concat!("SELECT count(*)", list_where!()))?
            .query_row(selectors, |row| row.get(0))?;
        let mut stmt = conn.prepare_cached(concat!(
            "SELECT ",
            item_columns!(),
            list_where!(),
            " ORDER BY m.date IS NULL, m.date DESC, m.seq DESC LIMIT ?6 OFFSET ?7"
        ))?;
        let items = stmt
            .query_map(
                params![
                    query.account_id,
                    query.folder_id,
                    role,
                    query.filter.code(),
                    search,
                    limit,
                    offset
                ],
                ItemRow::read,
            )?
            .map(|row| row?.into_item())
            .collect::<Result<Vec<_>>>()?;
        Ok(MessagePage {
            items,
            total: to_u64(total, "message count")?,
        })
    }

    /// Change flags on messages; `None` leaves a flag as it is. Returns how
    /// many messages were found.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn set_flags<S: AsRef<str>>(
        &self,
        ids: &[S],
        seen: Option<bool>,
        flagged: Option<bool>,
        answered: Option<bool>,
    ) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut changed = 0usize;
        {
            let mut stmt = tx.prepare(
                "UPDATE messages SET seen = coalesce(?2, seen), flagged = coalesce(?3, flagged),
                     answered = coalesce(?4, answered)
                 WHERE id = ?1",
            )?;
            for id in ids {
                changed = changed.saturating_add(stmt.execute(params![
                    id.as_ref(),
                    seen,
                    flagged,
                    answered
                ])?);
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Move messages to another folder of the same account locally. Their
    /// UIDs are cleared until the server assigns new ones (see
    /// [`Store::upsert_message`]), so record the old folder and UID in a
    /// pending operation before calling this. Messages from other accounts
    /// are skipped. Returns how many moved.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if the target folder does not exist.
    pub fn move_local<S: AsRef<str>>(&self, ids: &[S], folder_id: &str) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let account_id: String = tx
            .query_row(
                "SELECT account_id FROM folders WHERE id = ?1",
                params![folder_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound("folder"))?;
        let mut moved = 0usize;
        {
            let mut stmt = tx.prepare(
                "UPDATE messages SET folder_id = ?2, uid = NULL
                 WHERE id = ?1 AND account_id = ?3 AND folder_id <> ?2",
            )?;
            for id in ids {
                moved = moved.saturating_add(stmt.execute(params![
                    id.as_ref(),
                    folder_id,
                    account_id
                ])?);
            }
        }
        tx.commit()?;
        Ok(moved)
    }

    /// Delete messages by ID. Returns how many were deleted.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn delete_messages<S: AsRef<str>>(&self, ids: &[S]) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut deleted = 0usize;
        {
            let mut stmt = tx.prepare("DELETE FROM messages WHERE id = ?1")?;
            for id in ids {
                deleted = deleted.saturating_add(stmt.execute(params![id.as_ref()])?);
            }
        }
        tx.commit()?;
        Ok(deleted)
    }

    /// Delete a folder's messages by UID (expunged on the server). Returns
    /// how many were deleted.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn delete_by_uids(&self, folder_id: &str, uids: &[u32]) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut deleted = 0usize;
        {
            let mut stmt = tx.prepare("DELETE FROM messages WHERE folder_id = ?1 AND uid = ?2")?;
            for uid in uids {
                deleted = deleted.saturating_add(stmt.execute(params![folder_id, uid])?);
            }
        }
        tx.commit()?;
        Ok(deleted)
    }

    /// The UIDs stored for a folder, ascending. Messages awaiting a UID
    /// after a local move are not included.
    ///
    /// # Errors
    /// Fails on a database error or an out-of-range stored UID.
    pub fn uids_in_folder(&self, folder_id: &str) -> Result<Vec<u32>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT uid FROM messages WHERE folder_id = ?1 AND uid IS NOT NULL ORDER BY uid",
        )?;
        stmt.query_map(params![folder_id], |row| row.get::<_, i64>(0))?
            .map(|uid| to_u32(uid?, "uid"))
            .collect()
    }

    /// The ID of a message in a folder with this Message-ID header (with or
    /// without angle brackets). Used to check Sent before resending after a
    /// crash.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn find_by_message_id_header(
        &self,
        account_id: &str,
        folder_id: &str,
        header: &str,
    ) -> Result<Option<String>> {
        let bare = bare_message_id(header);
        if bare.is_empty() {
            return Ok(None);
        }
        let bracketed = format!("<{bare}>");
        Ok(self
            .conn()
            .query_row(
                "SELECT id FROM messages
                 WHERE account_id = ?1 AND folder_id = ?2 AND message_id_header IN (?3, ?4)
                 ORDER BY seq LIMIT 1",
                params![account_id, folder_id, bare, bracketed],
                |row| row.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use super::*;
    use crate::test_support::{add_account, add_folder, add_message, summary, with_attachment};
    use proptest::prelude::*;
    use std::sync::Arc;

    struct Fixture {
        store: Store,
        account: String,
        inbox: String,
    }

    fn fixture() -> Fixture {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let inbox = add_folder(&store, &account, "INBOX", FolderRole::Inbox);
        Fixture {
            store,
            account,
            inbox,
        }
    }

    fn query() -> ListQuery {
        ListQuery {
            limit: 50,
            ..ListQuery::default()
        }
    }

    fn subjects(page: &MessagePage) -> Vec<&str> {
        page.items.iter().map(|i| i.subject.as_str()).collect()
    }

    #[test]
    fn upsert_inserts_then_updates_in_place() {
        let f = fixture();
        let s = with_attachment(summary("Hello", 10));
        let id = f
            .store
            .upsert_message(&f.inbox, 7, &s, MessageFlags::default(), 2048)
            .unwrap();
        let stored = f.store.get_message(&id).unwrap();
        assert_eq!(stored.uid, Some(7));
        assert_eq!(stored.size, 2048);
        assert_eq!(stored.item.subject, "Hello");
        assert_eq!(stored.item.account_id, f.account);
        assert_eq!(stored.item.folder_id, f.inbox);
        assert_eq!(stored.item.from, s.from);
        assert_eq!(stored.item.to, s.to);
        assert_eq!(stored.cc, s.cc);
        assert_eq!(stored.attachments, s.attachments);
        assert!(stored.item.has_attachments);
        assert!(stored.item.unread);
        assert!(!stored.has_raw);
        assert_eq!(stored.message_id_header, s.message_id);

        let mut changed = summary("Hello again", 10);
        changed.references = vec!["<r1@x>".into()];
        changed.in_reply_to = Some("<r1@x>".into());
        changed.from = None;
        let again = f
            .store
            .upsert_message(
                &f.inbox,
                7,
                &changed,
                MessageFlags {
                    seen: true,
                    flagged: true,
                    answered: true,
                },
                4096,
            )
            .unwrap();
        assert_eq!(again, id);
        let stored = f.store.get_message(&id).unwrap();
        assert_eq!(stored.item.subject, "Hello again");
        assert_eq!(stored.item.from, None);
        assert_eq!(stored.references, ["<r1@x>"]);
        assert_eq!(stored.in_reply_to.as_deref(), Some("<r1@x>"));
        assert!(!stored.item.unread && stored.item.flagged && stored.item.answered);
        assert!(!stored.item.has_attachments);
        assert_eq!(stored.size, 4096);
        assert_eq!(f.store.uids_in_folder(&f.inbox).unwrap(), [7]);
    }

    #[test]
    fn upsert_into_unknown_folder_is_not_found() {
        let f = fixture();
        assert!(matches!(
            f.store
                .upsert_message("nope", 1, &summary("x", 1), MessageFlags::default(), 0),
            Err(StoreError::NotFound("folder"))
        ));
        assert!(matches!(
            f.store.upsert_message(
                &f.inbox,
                1,
                &summary("x", 1),
                MessageFlags::default(),
                u64::MAX
            ),
            Err(StoreError::Invalid(_))
        ));
    }

    #[test]
    fn raw_cache_survives_header_resync() {
        let f = fixture();
        let id = add_message(&f.store, &f.inbox, 1, &summary("Quarterly", 1));
        assert_eq!(f.store.get_raw(&id).unwrap(), None);
        f.store
            .set_raw(&id, b"raw bytes", Some("budget spreadsheet attached"))
            .unwrap();
        add_message(&f.store, &f.inbox, 1, &summary("Quarterly (edited)", 1));
        assert_eq!(
            f.store.get_raw(&id).unwrap().as_deref(),
            Some(&b"raw bytes"[..])
        );
        assert!(f.store.get_message(&id).unwrap().has_raw);
        // The body text is still searchable after the resync.
        let page = f
            .store
            .list_messages(&ListQuery {
                search: Some("spreadsheet".into()),
                ..query()
            })
            .unwrap();
        assert_eq!(subjects(&page), ["Quarterly (edited)"]);
    }

    #[test]
    fn raw_for_unknown_message_is_not_found() {
        let f = fixture();
        assert!(matches!(
            f.store.get_raw("nope"),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            f.store.set_raw("nope", b"x", None),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            f.store.get_message("nope"),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn lists_newest_first_with_undated_last() {
        let f = fixture();
        add_message(&f.store, &f.inbox, 1, &summary("old", 100));
        let mut undated = summary("undated", 0);
        undated.date = None;
        add_message(&f.store, &f.inbox, 2, &undated);
        add_message(&f.store, &f.inbox, 3, &summary("new", 300));
        add_message(&f.store, &f.inbox, 4, &summary("mid", 200));
        let page = f.store.list_messages(&query()).unwrap();
        assert_eq!(subjects(&page), ["new", "mid", "old", "undated"]);
        assert_eq!(page.total, 4);
    }

    #[test]
    fn pagination_keeps_the_total() {
        let f = fixture();
        for uid in 1..=25u32 {
            add_message(
                &f.store,
                &f.inbox,
                uid,
                &summary(&format!("m{uid}"), i64::from(uid)),
            );
        }
        let first = f
            .store
            .list_messages(&ListQuery {
                limit: 10,
                ..query()
            })
            .unwrap();
        assert_eq!(first.items.len(), 10);
        assert_eq!(first.total, 25);
        assert_eq!(first.items.first().unwrap().subject, "m25");
        let last = f
            .store
            .list_messages(&ListQuery {
                offset: 20,
                limit: 10,
                ..query()
            })
            .unwrap();
        assert_eq!(subjects(&last), ["m5", "m4", "m3", "m2", "m1"]);
        assert_eq!(last.total, 25);
        let past_end = f
            .store
            .list_messages(&ListQuery {
                offset: 100,
                limit: 10,
                ..query()
            })
            .unwrap();
        assert!(past_end.items.is_empty());
        assert_eq!(past_end.total, 25);
    }

    #[test]
    fn limit_is_capped() {
        let f = fixture();
        for uid in 1..=(MAX_PAGE_SIZE + 5) {
            add_message(&f.store, &f.inbox, uid, &summary("m", i64::from(uid)));
        }
        let page = f
            .store
            .list_messages(&ListQuery {
                limit: 10_000,
                ..query()
            })
            .unwrap();
        assert_eq!(page.items.len(), MAX_PAGE_SIZE as usize);
        assert_eq!(page.total, u64::from(MAX_PAGE_SIZE + 5));
    }

    #[test]
    fn unread_and_flagged_filters() {
        let f = fixture();
        let read = add_message(&f.store, &f.inbox, 1, &summary("read", 1));
        let flagged = add_message(&f.store, &f.inbox, 2, &summary("flagged", 2));
        add_message(&f.store, &f.inbox, 3, &summary("unread", 3));
        f.store.set_flags(&[&read], Some(true), None, None).unwrap();
        f.store
            .set_flags(&[&flagged], Some(true), Some(true), None)
            .unwrap();
        let unread = f
            .store
            .list_messages(&ListQuery {
                filter: ListFilter::Unread,
                ..query()
            })
            .unwrap();
        assert_eq!(subjects(&unread), ["unread"]);
        assert_eq!(unread.total, 1);
        let only_flagged = f
            .store
            .list_messages(&ListQuery {
                filter: ListFilter::Flagged,
                ..query()
            })
            .unwrap();
        assert_eq!(subjects(&only_flagged), ["flagged"]);
    }

    #[test]
    fn selects_by_folder_account_and_unified_role() {
        let f = fixture();
        let other = add_account(&f.store, "b@example.com");
        let other_inbox = add_folder(&f.store, &other, "INBOX", FolderRole::Inbox);
        let archive = add_folder(&f.store, &f.account, "Archive", FolderRole::Archive);
        add_message(&f.store, &f.inbox, 1, &summary("a inbox", 1));
        add_message(&f.store, &archive, 1, &summary("a archive", 2));
        add_message(&f.store, &other_inbox, 1, &summary("b inbox", 3));

        let unified = f
            .store
            .list_messages(&ListQuery {
                role: Some(FolderRole::Inbox),
                ..query()
            })
            .unwrap();
        assert_eq!(subjects(&unified), ["b inbox", "a inbox"]);
        assert_eq!(unified.total, 2);

        let one_account_role = f
            .store
            .list_messages(&ListQuery {
                account_id: Some(other.clone()),
                role: Some(FolderRole::Inbox),
                ..query()
            })
            .unwrap();
        assert_eq!(subjects(&one_account_role), ["b inbox"]);

        let folder = f
            .store
            .list_messages(&ListQuery {
                folder_id: Some(archive),
                ..query()
            })
            .unwrap();
        assert_eq!(subjects(&folder), ["a archive"]);

        let account = f
            .store
            .list_messages(&ListQuery {
                account_id: Some(f.account.clone()),
                ..query()
            })
            .unwrap();
        assert_eq!(account.total, 2);

        let everything = f.store.list_messages(&query()).unwrap();
        assert_eq!(everything.total, 3);
    }

    #[test]
    fn full_text_search_over_subject_people_and_body() {
        let f = fixture();
        let mut from_jane = summary("Lunch", 1);
        from_jane.from = Some(Mailbox {
            name: Some("Jane Wanjiku".into()),
            address: "jane@mocky.co.ke".into(),
        });
        add_message(&f.store, &f.inbox, 1, &from_jane);
        let mut to_peter = summary("Report", 2);
        to_peter.to = vec![Mailbox {
            name: Some("Peter Otieno".into()),
            address: "peter@client.example".into(),
        }];
        add_message(&f.store, &f.inbox, 2, &to_peter);
        let body = add_message(&f.store, &f.inbox, 3, &summary("Notes", 3));
        f.store
            .set_raw(&body, b"x", Some("The M-Pesa reconciliation is attached"))
            .unwrap();
        add_message(&f.store, &f.inbox, 4, &summary("Café menu", 4));

        let search = |text: &str| {
            let page = f
                .store
                .list_messages(&ListQuery {
                    search: Some(text.into()),
                    ..query()
                })
                .unwrap();
            subjects(&page)
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(search("lunch"), ["Lunch"]);
        assert_eq!(search("wanjiku"), ["Lunch"]);
        assert_eq!(search("mocky.co.ke"), ["Lunch"]);
        assert_eq!(search("otieno"), ["Report"]);
        assert_eq!(search("reconcil"), ["Notes"]);
        assert_eq!(search("cafe"), ["Café menu"]);
        assert_eq!(search("report peter"), ["Report"]);
        assert!(search("report jane").is_empty());
        // Searching combines with the other selectors and the total.
        let page = f
            .store
            .list_messages(&ListQuery {
                search: Some("preview".into()),
                limit: 2,
                ..query()
            })
            .unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.total, 4);
        // A changed subject is reindexed.
        add_message(&f.store, &f.inbox, 1, &summary("Dinner", 1));
        assert!(search("lunch").is_empty());
        assert_eq!(search("dinner"), ["Dinner"]);
    }

    #[test]
    fn hostile_search_input_never_errors() {
        let f = fixture();
        add_message(&f.store, &f.inbox, 1, &summary("a OR b", 1));
        for input in [
            "\"",
            "\"\"",
            "NEAR(",
            "NEAR(a b)",
            "*",
            "a*",
            "a OR",
            "OR",
            "AND NOT",
            "(",
            ")",
            "subject:a",
            "^a",
            "a + b",
            "-a",
            "'; DROP TABLE messages; --",
            "日本語",
            "🙂",
            "\u{0}",
            "\u{0}\u{b9}",
            "a\u{0}b",
            "a\"b",
            "{subject}: x",
            "",
        ] {
            let result = f.store.list_messages(&ListQuery {
                search: Some(input.into()),
                ..query()
            });
            assert!(result.is_ok(), "search {input:?} failed: {result:?}");
        }
        let page = f
            .store
            .list_messages(&ListQuery {
                search: Some("a OR".into()),
                ..query()
            })
            .unwrap();
        assert_eq!(subjects(&page), ["a OR b"]);
        let nothing = f
            .store
            .list_messages(&ListQuery {
                search: Some("***".into()),
                ..query()
            })
            .unwrap();
        assert_eq!(nothing.total, 0);
        let blank = f
            .store
            .list_messages(&ListQuery {
                search: Some("   ".into()),
                ..query()
            })
            .unwrap();
        assert_eq!(blank.total, 1);
    }

    #[test]
    fn set_flags_leaves_unspecified_flags_alone() {
        let f = fixture();
        let a = add_message(&f.store, &f.inbox, 1, &summary("a", 1));
        let b = add_message(&f.store, &f.inbox, 2, &summary("b", 2));
        assert_eq!(
            f.store
                .set_flags(
                    &[&a, &b, &"missing".to_owned()],
                    Some(true),
                    Some(true),
                    None
                )
                .unwrap(),
            2
        );
        f.store
            .set_flags(&[&a], None, Some(false), Some(true))
            .unwrap();
        let a = f.store.get_message(&a).unwrap().item;
        assert!(!a.unread && !a.flagged && a.answered);
        let b = f.store.get_message(&b).unwrap().item;
        assert!(!b.unread && b.flagged && !b.answered);
    }

    #[test]
    fn move_local_clears_uid_and_adopts_the_server_copy() {
        let f = fixture();
        let archive = add_folder(&f.store, &f.account, "Archive", FolderRole::Archive);
        let other = add_account(&f.store, "b@example.com");
        let other_folder = add_folder(&f.store, &other, "INBOX", FolderRole::Inbox);
        let s = summary("move me", 1);
        let id = add_message(&f.store, &f.inbox, 5, &s);
        f.store.set_raw(&id, b"cached", None).unwrap();

        assert_eq!(f.store.move_local(&[&id], &other_folder).unwrap(), 0);
        assert_eq!(f.store.move_local(&[&id], &archive).unwrap(), 1);
        assert_eq!(f.store.move_local(&[&id], &archive).unwrap(), 0);
        let moved = f.store.get_message(&id).unwrap();
        assert_eq!(moved.item.folder_id, archive);
        assert_eq!(moved.uid, None);
        assert!(f.store.uids_in_folder(&f.inbox).unwrap().is_empty());
        assert!(f.store.uids_in_folder(&archive).unwrap().is_empty());

        // The next sync of Archive sees the message under its new UID.
        let synced = add_message(&f.store, &archive, 77, &s);
        assert_eq!(synced, id);
        assert_eq!(f.store.uids_in_folder(&archive).unwrap(), [77]);
        assert_eq!(
            f.store.get_raw(&id).unwrap().as_deref(),
            Some(&b"cached"[..])
        );

        assert!(matches!(
            f.store.move_local(&[&id], "nope"),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn deletes_by_id_and_by_uid() {
        let f = fixture();
        let other = add_folder(&f.store, &f.account, "Other", FolderRole::Other);
        let a = add_message(&f.store, &f.inbox, 1, &summary("a", 1));
        add_message(&f.store, &f.inbox, 2, &summary("b", 2));
        add_message(&f.store, &f.inbox, 3, &summary("c", 3));
        add_message(&f.store, &other, 2, &summary("d", 4));

        assert_eq!(
            f.store
                .delete_messages(&[&a, &"missing".to_owned()])
                .unwrap(),
            1
        );
        assert_eq!(f.store.delete_by_uids(&f.inbox, &[2, 99]).unwrap(), 1);
        assert_eq!(f.store.uids_in_folder(&f.inbox).unwrap(), [3]);
        assert_eq!(f.store.uids_in_folder(&other).unwrap(), [2]);
        let page = f
            .store
            .list_messages(&ListQuery {
                search: Some("preview".into()),
                ..query()
            })
            .unwrap();
        assert_eq!(page.total, 2);
    }

    #[test]
    fn uids_are_sorted() {
        let f = fixture();
        for uid in [30, 4, u32::MAX, 1] {
            add_message(&f.store, &f.inbox, uid, &summary("m", 1));
        }
        assert_eq!(
            f.store.uids_in_folder(&f.inbox).unwrap(),
            [1, 4, 30, u32::MAX]
        );
    }

    #[test]
    fn finds_sent_copy_by_message_id() {
        let f = fixture();
        let sent = add_folder(&f.store, &f.account, "Sent", FolderRole::Sent);
        let mut s = summary("sent", 1);
        s.message_id = Some("abc@mocky.co.ke".into());
        let id = add_message(&f.store, &sent, 1, &s);
        for header in [
            "abc@mocky.co.ke",
            "<abc@mocky.co.ke>",
            " <abc@mocky.co.ke> ",
        ] {
            assert_eq!(
                f.store
                    .find_by_message_id_header(&f.account, &sent, header)
                    .unwrap()
                    .as_deref(),
                Some(id.as_str())
            );
        }
        assert_eq!(
            f.store
                .find_by_message_id_header(&f.account, &f.inbox, "abc@mocky.co.ke")
                .unwrap(),
            None
        );
        assert_eq!(
            f.store
                .find_by_message_id_header(&f.account, &sent, "<>")
                .unwrap(),
            None
        );
    }

    #[test]
    fn list_item_serializes_like_the_ipc_contract() {
        let f = fixture();
        let id = add_message(&f.store, &f.inbox, 1, &summary("s", 1));
        let item = f.store.get_message(&id).unwrap().item;
        let json = serde_json::to_value(&item).unwrap();
        for key in [
            "id",
            "accountId",
            "folderId",
            "from",
            "to",
            "subject",
            "preview",
            "date",
            "unread",
            "flagged",
            "answered",
            "hasAttachments",
        ] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
        let query: ListQuery = serde_json::from_value(serde_json::json!({
            "accountId": null, "folderId": null, "role": "inbox", "filter": "unread",
            "search": null, "offset": 0, "limit": 50
        }))
        .unwrap();
        assert_eq!(query.role, Some(FolderRole::Inbox));
        assert_eq!(query.filter, ListFilter::Unread);
    }

    #[test]
    fn concurrent_use_from_two_threads() {
        let f = fixture();
        let store = Arc::new(f.store);
        let writer = {
            let store = Arc::clone(&store);
            let inbox = f.inbox.clone();
            std::thread::spawn(move || {
                for uid in 1..=200u32 {
                    store
                        .upsert_message(
                            &inbox,
                            uid,
                            &summary("w", i64::from(uid)),
                            MessageFlags::default(),
                            1,
                        )
                        .unwrap();
                }
            })
        };
        let reader = {
            let store = Arc::clone(&store);
            std::thread::spawn(move || {
                let mut last = 0;
                for _ in 0..200 {
                    let total = store.list_messages(&query()).unwrap().total;
                    assert!(total >= last);
                    last = total;
                }
            })
        };
        writer.join().unwrap();
        reader.join().unwrap();
        assert_eq!(store.list_messages(&query()).unwrap().total, 200);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn any_search_text_is_safe(input in "\\PC{0,40}", raw in ".{0,40}") {
            let f = fixture();
            add_message(&f.store, &f.inbox, 1, &summary("hello world", 1));
            for text in [input, raw] {
                let result = f.store.list_messages(&ListQuery {
                    search: Some(text.clone()),
                    ..query()
                });
                prop_assert!(result.is_ok(), "search {:?} failed: {:?}", text, result);
            }
        }

        #[test]
        fn pages_partition_the_list(count in 0u32..40, limit in 1u32..15) {
            let f = fixture();
            for uid in 1..=count {
                add_message(&f.store, &f.inbox, uid, &summary("m", i64::from(uid % 7)));
            }
            let mut seen = Vec::new();
            let mut offset = 0;
            loop {
                let page = f.store.list_messages(&ListQuery { offset, limit, ..query() }).unwrap();
                prop_assert_eq!(page.total, u64::from(count));
                if page.items.is_empty() {
                    break;
                }
                seen.extend(page.items.into_iter().map(|i| i.id));
                offset += limit;
            }
            let unique: std::collections::HashSet<_> = seen.iter().collect();
            prop_assert_eq!(seen.len(), count as usize);
            prop_assert_eq!(unique.len(), seen.len());
        }
    }
}
