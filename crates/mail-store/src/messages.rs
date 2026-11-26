//! Message headers, flags, cached raw bodies and the message list query.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use mail_core::folders::FolderRole;
use mail_core::importance::Importance;
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

/// Most messages [`Store::get_thread`] reads for one conversation.
pub const MAX_THREAD_MESSAGES: u32 = 200;

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
    /// Only messages with attachments.
    Attachments,
    /// Only messages sent to the account's own address (To or Cc).
    #[serde(rename = "toMe")]
    ToMe,
    /// Only pinned messages.
    Pinned,
}

impl ListFilter {
    const fn code(self) -> i64 {
        match self {
            Self::All => 0,
            Self::Unread => 1,
            Self::Flagged => 2,
            Self::Attachments => 3,
            Self::ToMe => 4,
            Self::Pinned => 5,
        }
    }
}

/// How the message list is ordered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListSort {
    /// By date.
    #[default]
    Date,
    /// By sender name (or address).
    From,
    /// By subject.
    Subject,
    /// By size.
    Size,
}

/// The `ORDER BY` for a page: pinned messages first (`pinned` is the
/// column or expression saying so) unless searching, then the chosen sort,
/// newest first among equals. Built only from the fixed pieces below.
fn order_by(query: &ListQuery, pinned: &str, searching: bool) -> String {
    let dir = if query.descending.unwrap_or(true) {
        "DESC"
    } else {
        "ASC"
    };
    let newest = "m.date IS NULL, m.date DESC, m.seq DESC";
    let key = match query.sort.unwrap_or_default() {
        // Undated mail last either way.
        ListSort::Date => format!("m.date IS NULL, m.date {dir}, m.seq {dir}"),
        ListSort::From => format!(
            "lower(coalesce(nullif(trim(m.from_name), ''), m.from_address, '')) {dir}, {newest}"
        ),
        ListSort::Subject => format!("lower(m.subject) {dir}, {newest}"),
        ListSort::Size => format!("m.size {dir}, {newest}"),
    };
    if searching || query.ignore_pinned {
        key
    } else {
        format!("{pinned} DESC, {key}")
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
    /// Focused Inbox: only this half of inbox folders (other folders are
    /// unaffected). `None` shows everything.
    #[serde(default)]
    pub focus: Option<mail_core::focus::Focus>,
    /// Order; `None` is by date.
    #[serde(default)]
    pub sort: Option<ListSort>,
    /// Largest first (newest, Z to A, biggest); `None` is `true`.
    #[serde(default)]
    pub descending: Option<bool>,
    /// Leave pinned messages in their place (for the app's own lookups,
    /// such as the newest unread message; not part of the interface).
    #[serde(skip)]
    pub ignore_pinned: bool,
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
    /// Kept at the top of its folder (this device only).
    pub pinned: bool,
    /// From the headers, or as set here.
    pub importance: Importance,
    /// Category names, in the order they were added.
    pub categories: Vec<String>,
    /// Follow-up date of a flagged message, Unix seconds.
    pub flag_due: Option<i64>,
    /// Size of the whole message in bytes.
    pub size: u64,
    /// Messages in this conversation shown by the list (1 when not grouped).
    pub thread_count: u32,
    /// Their ids, newest first, when grouped; empty otherwise. Actions on a
    /// conversation row apply to all of them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub thread_ids: Vec<String>,
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
         m.preview, m.date, m.seen, m.flagged, m.answered, m.has_attachments, m.pinned, \
         m.importance, m.categories, m.flag_due, m.size"
    };
}
// 18 columns: queries that select more read them from index 18 on.

/// Selection shared by the page and the count. `?1` account, `?2` folder,
/// `?3` role, `?4` filter code, `?5` FTS query; a NULL selector matches all.
macro_rules! list_where {
    () => {
        " FROM messages m JOIN folders f ON f.id = m.folder_id
          LEFT JOIN focus_rules r1 ON r1.sender = lower(m.from_address)
          LEFT JOIN focus_rules r2
                 ON r2.sender = '@' || substr(lower(m.from_address), instr(m.from_address, '@') + 1)
          WHERE (?1 IS NULL OR m.account_id = ?1)
            AND (?2 IS NULL OR m.folder_id = ?2)
            AND (?3 IS NULL OR f.role = ?3)
            AND (?4 = 0 OR (?4 = 1 AND m.seen = 0) OR (?4 = 2 AND m.flagged = 1)
                 OR (?4 = 3 AND m.has_attachments = 1) OR (?4 = 5 AND m.pinned = 1)
                 OR (?4 = 4 AND EXISTS (
                     SELECT 1 FROM accounts a
                     WHERE a.id = m.account_id AND lower(a.email) IN (
                         SELECT lower(json_extract(t.value, '$.address')) FROM json_each(m.to_json) t
                         UNION ALL
                         SELECT lower(json_extract(c.value, '$.address')) FROM json_each(m.cc_json) c))))
            AND (?5 IS NULL OR m.seq IN
                 (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?5))
            AND (?8 IS NULL OR f.role != 'inbox' OR coalesce(r1.focus, r2.focus,
                 CASE
                   WHEN lower(coalesce(m.from_address, '')) IN (
                     SELECT lower(json_extract(t.value, '$.address'))
                     FROM messages s JOIN folders sf ON sf.id = s.folder_id, json_each(s.to_json) t
                     WHERE sf.role = 'sent') THEN 'focused'
                   WHEN m.bulk = 1
                     OR lower(coalesce(m.from_address, '')) GLOB 'noreply*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'no-reply*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'no_reply*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'donotreply*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'do-not-reply*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'do_not_reply*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'notification*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'newsletter*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'mailer-daemon*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'postmaster*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'bounce*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'marketing*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'news@*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'alerts@*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'alert@*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'updates@*'
                     OR lower(coalesce(m.from_address, '')) GLOB 'digest@*' THEN 'other'
                   ELSE 'focused'
                 END) = ?8)"
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
    pinned: bool,
    importance: String,
    categories: String,
    flag_due: Option<i64>,
    size: i64,
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
            pinned: row.get(13)?,
            importance: row.get(14)?,
            categories: row.get(15)?,
            flag_due: row.get(16)?,
            size: row.get(17)?,
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
            pinned: self.pinned,
            importance: Importance::parse(&self.importance)
                .ok_or(StoreError::Corrupt("importance"))?,
            categories: serde_json::from_str(&self.categories)?,
            flag_due: self.flag_due,
            size: to_u64(self.size, "message size")?,
            thread_count: 1,
            thread_ids: Vec::new(),
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
            uid: row.get(18)?,
            message_id_header: row.get(19)?,
            cc_json: row.get(20)?,
            reply_to_json: row.get(21)?,
            references_json: row.get(22)?,
            in_reply_to: row.get(23)?,
            size: row.get(24)?,
            attachments_json: row.get(25)?,
            has_raw: row.get(26)?,
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
             size = ?19, bulk = ?20,
             importance = CASE WHEN importance_local = 1 THEN importance ELSE ?21 END,
             flag_due = CASE WHEN ?17 = 0 THEN NULL ELSE flag_due END
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
            s.bulk,
            s.importance.as_str(),
        ],
    )?;
    link_thread(tx, id, s)
}

/// Finish a message's conversation key after its headers are saved. The
/// schema's trigger has just set it from the headers alone: the first
/// References id, else In-Reply-To, else the message's own id. Some
/// clients send a reply with In-Reply-To and no References, and its
/// parent's id is not the conversation's key when the parent is itself a
/// reply, so the key is taken from the stored parent instead. The parent
/// may also arrive after its replies (Sent is read after the Inbox), so
/// replies of that kind already keyed by this message's id are moved to
/// its conversation.
fn link_thread(tx: &Transaction<'_>, id: &str, s: &MessageSummary) -> Result<()> {
    let parent = s
        .in_reply_to
        .as_deref()
        .map(bare_message_id)
        .filter(|parent| !parent.is_empty());
    if let Some(parent) = parent.filter(|_| s.references.is_empty()) {
        tx.execute(
            "UPDATE messages SET thread_key = coalesce(
                 (SELECT p.thread_key FROM messages p
                   WHERE p.account_id = messages.account_id
                     AND p.message_id_header IN (?2, ?3)
                     AND p.id <> messages.id AND p.thread_key IS NOT NULL
                   ORDER BY p.seq LIMIT 1),
                 thread_key)
             WHERE id = ?1",
            params![id, parent, format!("<{parent}>")],
        )?;
    }
    let own = s
        .message_id
        .as_deref()
        .map(bare_message_id)
        .filter(|own| !own.is_empty());
    if let Some(own) = own {
        let (account_id, key): (String, Option<String>) = tx.query_row(
            "SELECT account_id, thread_key FROM messages WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if let Some(key) = key {
            tx.execute(
                "UPDATE messages SET thread_key = ?2
                 WHERE account_id = ?1 AND thread_key = lower(trim(?3, '<> '))
                   AND thread_key <> ?2 AND json_array_length(references_json) = 0",
                params![account_id, key, own],
            )?;
        }
    }
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
        let focus = query.focus.map(mail_core::focus::Focus::as_str);
        // ?6 and ?7 (page) are unused by the count but must be bound: ?8 is the focus.
        let selectors = params![
            query.account_id,
            query.folder_id,
            role,
            query.filter.code(),
            search,
            0,
            0,
            focus
        ];
        let conn = self.conn();
        let total: i64 = conn
            .prepare_cached(concat!("SELECT count(*)", list_where!()))?
            .query_row(selectors, |row| row.get(0))?;
        let order = order_by(query, "m.pinned", search.is_some());
        let mut stmt = conn.prepare_cached(&format!(
            "{} ORDER BY {order} LIMIT ?6 OFFSET ?7",
            concat!("SELECT ", item_columns!(), list_where!())
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
                    offset,
                    focus
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

    /// Like [`Store::list_messages`], one row per conversation: its newest
    /// matching message, unread if any of them is, with how many matched
    /// and their ids. `total` counts conversations.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn list_threads(&self, query: &ListQuery) -> Result<MessagePage> {
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
        let focus = query.focus.map(mail_core::focus::Focus::as_str);
        let conn = self.conn();
        let total: i64 = conn
            .prepare_cached(concat!(
                "SELECT count(DISTINCT m.account_id || char(0) || coalesce(m.thread_key, m.id))",
                list_where!()
            ))?
            .query_row(
                params![
                    query.account_id,
                    query.folder_id,
                    role,
                    query.filter.code(),
                    search,
                    0,
                    0,
                    focus
                ],
                |row| row.get(0),
            )?;
        let order = order_by(query, "r.any_pinned", search.is_some());
        let mut stmt = conn.prepare_cached(&format!(
            "{} ORDER BY {order} LIMIT ?6 OFFSET ?7",
            concat!(
                "WITH hits AS (SELECT m.seq AS seq, m.id AS id, m.account_id AS account_id,
                    coalesce(m.thread_key, m.id) AS tk, m.date AS date, m.seen AS seen,
                    m.pinned AS pinned",
                list_where!(),
                "), ranked AS (SELECT seq,
                    row_number() OVER (PARTITION BY account_id, tk
                        ORDER BY date IS NULL, date DESC, seq DESC) AS rn,
                    count(*) OVER whole AS n,
                    max(seen = 0) OVER whole AS any_unread,
                    group_concat(id, ',') OVER whole AS ids,
                    max(pinned) OVER whole AS any_pinned
                FROM hits
                WINDOW whole AS (PARTITION BY account_id, tk))
             SELECT ",
                item_columns!(),
                ", r.n, r.any_unread, r.ids, r.any_pinned FROM ranked r JOIN messages m ON m.seq = r.seq
             WHERE r.rn = 1"
            )
        ))?;
        let rows = stmt.query_map(
            params![
                query.account_id,
                query.folder_id,
                role,
                query.filter.code(),
                search,
                limit,
                offset,
                focus
            ],
            |row| {
                Ok((
                    ItemRow::read(row)?,
                    row.get::<_, i64>(18)?,
                    row.get::<_, bool>(19)?,
                    row.get::<_, String>(20)?,
                    row.get::<_, bool>(21)?,
                ))
            },
        )?;
        let mut items = Vec::new();
        for row in rows {
            let (item, count, any_unread, ids, any_pinned) = row?;
            let mut item = item.into_item()?;
            item.thread_count = u32::try_from(count).unwrap_or(1).max(1);
            item.unread = any_unread;
            item.pinned = any_pinned;
            let mut ids: Vec<String> = ids.split(',').map(str::to_owned).collect();
            // The shown message first.
            ids.retain(|id| id != &item.id);
            ids.insert(0, item.id.clone());
            item.thread_ids = ids;
            items.push(item);
        }
        Ok(MessagePage {
            items,
            total: to_u64(total, "conversation count")?,
        })
    }

    /// Every message in a message's conversation in its account, oldest
    /// first: received and sent, in any folder except Trash and Spam
    /// (unless the message itself is there). A conversation longer than
    /// [`MAX_THREAD_MESSAGES`] gives its newest messages.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown id.
    pub fn get_thread(&self, id: &str) -> Result<Vec<MessageListItem>> {
        let conn = self.conn();
        let (account_id, key, role): (String, String, String) = conn
            .query_row(
                "SELECT m.account_id, coalesce(m.thread_key, m.id), f.role
                 FROM messages m JOIN folders f ON f.id = m.folder_id WHERE m.id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(StoreError::NotFound("message"))?;
        let hide_bin = role != "trash" && role != "junk";
        // The newest of a very long conversation: those are the ones being
        // read and answered.
        let mut stmt = conn.prepare_cached(concat!(
            "SELECT ",
            item_columns!(),
            ", m.seq, m.message_id_header
             FROM messages m JOIN folders f ON f.id = m.folder_id
             WHERE m.account_id = ?1 AND coalesce(m.thread_key, m.id) = ?2
               AND (?3 = 0 OR f.role NOT IN ('trash', 'junk'))
             ORDER BY m.date IS NULL, m.date DESC, m.seq DESC LIMIT ?4"
        ))?;
        let mut rows = stmt
            .query_map(
                params![account_id, key, hide_bin, MAX_THREAD_MESSAGES],
                |row| {
                    Ok((
                        ItemRow::read(row)?,
                        row.get::<_, i64>(18)?,
                        row.get::<_, Option<String>>(19)?,
                    ))
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        // Oldest first, undated last.
        rows.sort_by_key(|(item, seq, _)| (item.date.is_none(), item.date, *seq));
        let mut seen_ids: Vec<String> = Vec::new();
        let mut items: Vec<MessageListItem> = Vec::new();
        for (row, _, message_id) in rows {
            let item = row.into_item()?;
            // The same message can sit in two folders (Gmail labels): show
            // it once. Its Message-ID says it is the same one; two
            // different messages can share a sender, subject and second,
            // so that only decides for a message without an id.
            let message_id = message_id
                .as_deref()
                .map(bare_message_id)
                .filter(|id| !id.is_empty());
            let repeated = match message_id {
                Some(id) => {
                    let known = seen_ids.iter().any(|seen| seen == id);
                    if !known {
                        seen_ids.push(id.to_owned());
                    }
                    known
                }
                None => items.iter().any(|i| {
                    i.date == item.date && i.subject == item.subject && i.from == item.from
                }),
            };
            if !repeated {
                items.push(item);
            }
        }
        Ok(items)
    }

    /// Change flags on messages; `None` leaves a flag as it is. Clearing
    /// the flag also clears its follow-up date. Returns how many messages
    /// were found.
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
                     answered = coalesce(?4, answered),
                     flag_due = CASE WHEN ?3 = 0 THEN NULL ELSE flag_due END
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

    /// Change what is kept only on this device: pinned, importance (which
    /// then no longer follows the headers) and the follow-up date
    /// (`Some(None)` clears it). `None` leaves each as it is. Returns how
    /// many messages were found.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn set_marks<S: AsRef<str>>(
        &self,
        ids: &[S],
        pinned: Option<bool>,
        importance: Option<Importance>,
        flag_due: Option<Option<i64>>,
    ) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut changed = 0usize;
        {
            let mut stmt = tx.prepare(
                "UPDATE messages SET pinned = coalesce(?2, pinned),
                     importance = coalesce(?3, importance),
                     importance_local = CASE WHEN ?3 IS NULL THEN importance_local ELSE 1 END,
                     flag_due = CASE WHEN ?4 THEN ?5 ELSE flag_due END
                 WHERE id = ?1",
            )?;
            for id in ids {
                changed = changed.saturating_add(stmt.execute(params![
                    id.as_ref(),
                    pinned,
                    importance.map(Importance::as_str),
                    flag_due.is_some(),
                    flag_due.flatten()
                ])?);
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Record the importance a message's headers give, unless it was set
    /// here (for mail synced before importance was read).
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn set_header_importance(&self, id: &str, importance: Importance) -> Result<()> {
        self.conn().execute(
            "UPDATE messages SET importance = ?2 WHERE id = ?1 AND importance_local = 0",
            params![id, importance.as_str()],
        )?;
        Ok(())
    }

    /// Flagged messages whose follow-up date is after `after` and no later
    /// than `upto`: `(id, subject)`, earliest first.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn flag_due_between(&self, after: i64, upto: i64) -> Result<Vec<(String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, subject FROM messages
             WHERE flag_due > ?1 AND flag_due <= ?2 AND flagged = 1
             ORDER BY flag_due, seq",
        )?;
        let rows = stmt.query_map(params![after, upto], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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

/// How many recipient suggestions to offer at most.
pub const MAX_SUGGESTIONS: usize = 8;

impl Store {
    /// People to suggest while typing a recipient: everyone the user has
    /// written to (Sent, weighted higher) or heard from, matching `query` at
    /// the start of the address, the name, or a word in the name. The user's
    /// own addresses and automated senders (no-reply) are left out. Best
    /// matches first: most messages, then most recent.
    ///
    /// # Errors
    /// Database errors.
    pub fn suggest_recipients(&self, query: &str, limit: usize) -> Result<Vec<Mailbox>> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let word = format!(" {needle}");
        let conn = self.conn();
        // Every name each address goes by. Matching is done below, not
        // here: SQLite's lower() and LIKE only fold ASCII letters, so
        // "él" would never find "Élodie".
        let mut stmt = conn.prepare(
            "WITH people(name, address, weight, date) AS (
                 SELECT m.from_name, m.from_address, 1, m.date
                   FROM messages m JOIN folders f ON f.id = m.folder_id
                  WHERE m.from_address IS NOT NULL AND f.role NOT IN ('sent', 'drafts', 'junk')
                 UNION ALL
                 SELECT json_extract(j.value, '$.name'), json_extract(j.value, '$.address'), 3, m.date
                   FROM messages m JOIN folders f ON f.id = m.folder_id, json_each(m.to_json) j
                  WHERE f.role = 'sent'
                 UNION ALL
                 SELECT json_extract(j.value, '$.name'), json_extract(j.value, '$.address'), 3, m.date
                   FROM messages m JOIN folders f ON f.id = m.folder_id, json_each(m.cc_json) j
                  WHERE f.role = 'sent'
             )
             SELECT name, address, SUM(weight), MAX(date)
               FROM people
              WHERE address LIKE '%@%'
                AND lower(address) NOT IN (SELECT lower(email) FROM accounts)
                AND lower(address) NOT LIKE '%noreply%'
                AND lower(address) NOT LIKE '%no-reply%'
                AND lower(address) NOT LIKE '%donotreply%'
                AND lower(address) NOT LIKE 'mailer-daemon@%'
              GROUP BY address, name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?;
        // One entry per address, from the rows that match: its greatest
        // name and spelling, its messages counted, its latest date.
        let mut people: HashMap<String, Suggestion> = HashMap::new();
        for row in rows {
            let (name, address, score, last) = row?;
            let lower_address = address.to_lowercase();
            let lower_name = name.as_deref().map(str::to_lowercase).unwrap_or_default();
            if !(lower_address.starts_with(&needle)
                || lower_name.starts_with(&needle)
                || lower_name.contains(&word))
            {
                continue;
            }
            match people.entry(lower_address) {
                Entry::Vacant(slot) => {
                    slot.insert(Suggestion {
                        name,
                        address,
                        score,
                        last,
                    });
                }
                Entry::Occupied(mut slot) => {
                    let person = slot.get_mut();
                    person.name = person.name.take().max(name);
                    if address < person.address {
                        person.address = address;
                    }
                    person.score = person.score.saturating_add(score);
                    person.last = person.last.max(last);
                }
            }
        }
        let mut people: Vec<Suggestion> = people.into_values().collect();
        // Best first; the address settles ties so the order is steady.
        people.sort_by(|a, b| {
            (b.score, b.last)
                .cmp(&(a.score, a.last))
                .then_with(|| a.address.cmp(&b.address))
        });
        people.truncate(limit.min(MAX_SUGGESTIONS));
        Ok(people
            .into_iter()
            .map(|person| {
                // A "name" that is just the address again adds nothing.
                let name = person
                    .name
                    .map(|n| n.trim().trim_matches(['"', '\'']).to_owned())
                    .filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case(&person.address));
                Mailbox {
                    name,
                    address: person.address,
                }
            })
            .collect())
    }
}

/// Someone to suggest, while their messages are being added up.
struct Suggestion {
    name: Option<String>,
    address: String,
    /// Messages to and from them, those written to counted three times.
    score: i64,
    /// Date of the latest one.
    last: Option<i64>,
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
    fn suggests_people_written_to_before_people_heard_from() {
        let f = fixture();
        let sent = add_folder(&f.store, &f.account, "Sent", FolderRole::Sent);
        // Heard from Alice twice; wrote to Bob once (counts more).
        f.store
            .upsert_message(&f.inbox, 1, &summary("one", 10), MessageFlags::default(), 1)
            .unwrap();
        f.store
            .upsert_message(&f.inbox, 2, &summary("two", 20), MessageFlags::default(), 1)
            .unwrap();
        let mut to_bob = summary("hi bob", 30);
        to_bob.from = Some(Mailbox {
            name: None,
            address: "a@example.com".into(),
        });
        f.store
            .upsert_message(&sent, 1, &to_bob, MessageFlags::default(), 1)
            .unwrap();
        let mut robot = summary("receipt", 40);
        robot.from = Some(Mailbox {
            name: Some("Shop".into()),
            address: "no-reply@shop.example".into(),
        });
        f.store
            .upsert_message(&f.inbox, 3, &robot, MessageFlags::default(), 1)
            .unwrap();

        let by_address = |q: &str| -> Vec<String> {
            f.store
                .suggest_recipients(q, 8)
                .unwrap()
                .into_iter()
                .map(|m| m.address)
                .collect()
        };
        assert_eq!(by_address("b"), ["bob@example.org"]);
        // A word inside the name matches too ("Sender" in "Alice Sender").
        assert_eq!(by_address("send"), ["alice@example.com"]);
        assert_eq!(
            f.store.suggest_recipients("Alice", 8).unwrap()[0]
                .name
                .as_deref(),
            Some("Alice Sender")
        );
        // Not the user's own address, no-reply senders, or wildcards.
        assert!(by_address("a@example").is_empty());
        assert!(by_address("no-reply").is_empty());
        assert!(by_address("%").is_empty());
        assert!(by_address("  ").is_empty());
    }

    #[test]
    fn suggestions_match_accented_names_in_any_case() {
        let f = fixture();
        for (uid, name, address) in [
            (1, "Élodie Durand", "elodie@atelier.example"),
            (2, "ÉLODIE DURAND", "Elodie@atelier.example"),
            (3, "Jürgen Öztürk", "j.o@werk.example"),
        ] {
            let mut m = summary("hello", i64::from(uid));
            m.from = Some(Mailbox {
                name: Some(name.into()),
                address: address.into(),
            });
            add_message(&f.store, &f.inbox, uid, &m);
        }
        let found = |q: &str| -> Vec<String> {
            f.store
                .suggest_recipients(q, 8)
                .unwrap()
                .into_iter()
                .map(|m| m.address)
                .collect()
        };
        // One entry for both spellings of the address.
        assert_eq!(found("él"), ["Elodie@atelier.example"]);
        assert_eq!(found("ÉLO"), ["Elodie@atelier.example"]);
        assert_eq!(found("özt"), ["j.o@werk.example"]);
        assert_eq!(found("JÜR"), ["j.o@werk.example"]);
        assert!(found("ö").len() == 1 && found("zt").is_empty());
        // The limit still holds, with the most written-to first.
        assert_eq!(f.store.suggest_recipients("e", 1).unwrap().len(), 1);
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
        assert_eq!(query.sort, None);
        assert_eq!(json["pinned"], false);
        assert_eq!(json["importance"], "normal");
        assert_eq!(json["categories"], serde_json::json!([]));
        assert!(json["flagDue"].is_null());
        assert_eq!(json["size"], 100);
        let query: ListQuery = serde_json::from_value(serde_json::json!({
            "accountId": null, "folderId": null, "role": null, "filter": "toMe",
            "search": null, "offset": 0, "limit": 50, "sort": "size", "descending": false
        }))
        .unwrap();
        assert_eq!(query.filter, ListFilter::ToMe);
        assert_eq!(query.sort, Some(ListSort::Size));
        assert_eq!(query.descending, Some(false));
        for (name, filter) in [
            ("attachments", ListFilter::Attachments),
            ("pinned", ListFilter::Pinned),
        ] {
            assert_eq!(
                serde_json::from_value::<ListFilter>(serde_json::json!(name)).unwrap(),
                filter
            );
        }
    }

    #[test]
    fn attachments_to_me_and_pinned_filters() {
        let f = fixture();
        let mut to_me = summary("to me", 3);
        to_me.cc.push(Mailbox {
            name: None,
            address: "A@Example.com".into(),
        });
        add_message(&f.store, &f.inbox, 1, &to_me);
        add_message(
            &f.store,
            &f.inbox,
            2,
            &with_attachment(summary("attached", 2)),
        );
        let pinned = add_message(&f.store, &f.inbox, 3, &summary("pinned", 1));
        f.store
            .set_marks(&[&pinned], Some(true), None, None)
            .unwrap();
        let only = |filter| {
            let page = f
                .store
                .list_messages(&ListQuery { filter, ..query() })
                .unwrap();
            page.items
                .iter()
                .map(|i| i.subject.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(only(ListFilter::ToMe), ["to me"]);
        assert_eq!(only(ListFilter::Attachments), ["attached"]);
        assert_eq!(only(ListFilter::Pinned), ["pinned"]);
        assert_eq!(only(ListFilter::All), ["pinned", "to me", "attached"]);
    }

    #[test]
    fn sorts_by_sender_subject_and_size_with_pinned_first() {
        let f = fixture();
        let mut zed = summary("apples", 10);
        zed.from = Some(Mailbox {
            name: Some("Zed".into()),
            address: "zed@x.co".into(),
        });
        let mut amy = summary("Bananas", 20);
        amy.from = Some(Mailbox {
            name: None,
            address: "amy@x.co".into(),
        });
        let mut mo = summary("cherries", 30);
        mo.from = Some(Mailbox {
            name: Some("mo".into()),
            address: "b@x.co".into(),
        });
        let zed = f
            .store
            .upsert_message(&f.inbox, 1, &zed, MessageFlags::default(), 500)
            .unwrap();
        f.store
            .upsert_message(&f.inbox, 2, &amy, MessageFlags::default(), 900)
            .unwrap();
        f.store
            .upsert_message(&f.inbox, 3, &mo, MessageFlags::default(), 100)
            .unwrap();
        let sorted = |sort, descending| {
            let page = f
                .store
                .list_messages(&ListQuery {
                    sort: Some(sort),
                    descending,
                    ..query()
                })
                .unwrap();
            page.items
                .iter()
                .map(|i| i.subject.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            sorted(ListSort::Date, None),
            ["cherries", "Bananas", "apples"]
        );
        assert_eq!(
            sorted(ListSort::Date, Some(false)),
            ["apples", "Bananas", "cherries"]
        );
        assert_eq!(
            sorted(ListSort::From, Some(false)),
            ["Bananas", "cherries", "apples"]
        );
        assert_eq!(
            sorted(ListSort::Subject, Some(true)),
            ["cherries", "Bananas", "apples"]
        );
        assert_eq!(
            sorted(ListSort::Size, None),
            ["Bananas", "apples", "cherries"]
        );
        // Pinned first, whatever the order, except in search results.
        f.store.set_marks(&[&zed], Some(true), None, None).unwrap();
        assert_eq!(
            sorted(ListSort::Size, None),
            ["apples", "Bananas", "cherries"]
        );
        assert_eq!(
            sorted(ListSort::Date, None),
            ["apples", "cherries", "Bananas"]
        );
        let threads = f.store.list_threads(&query()).unwrap();
        assert_eq!(subjects(&threads), ["apples", "cherries", "Bananas"]);
        assert!(threads.items[0].pinned);
        let found = f
            .store
            .list_messages(&ListQuery {
                search: Some("preview".into()),
                ..query()
            })
            .unwrap();
        assert_eq!(subjects(&found), ["cherries", "Bananas", "apples"]);
        // The app's own lookups can leave pins out.
        let plain = f
            .store
            .list_messages(&ListQuery {
                ignore_pinned: true,
                ..query()
            })
            .unwrap();
        assert_eq!(subjects(&plain), ["cherries", "Bananas", "apples"]);
    }

    #[test]
    fn marks_are_local_and_importance_follows_headers_until_set() {
        let f = fixture();
        let mut urgent = summary("urgent", 1);
        urgent.importance = Importance::High;
        let id = add_message(&f.store, &f.inbox, 1, &urgent);
        let item = f.store.get_message(&id).unwrap().item;
        assert_eq!(item.importance, Importance::High);
        assert!(!item.pinned);

        // Synced again: still from the headers.
        let mut calm = urgent.clone();
        calm.importance = Importance::Normal;
        add_message(&f.store, &f.inbox, 1, &calm);
        assert_eq!(
            f.store.get_message(&id).unwrap().item.importance,
            Importance::Normal
        );
        // Set here: the headers no longer change it.
        f.store
            .set_marks(&[&id], Some(true), Some(Importance::Low), Some(Some(5000)))
            .unwrap();
        let flagged = MessageFlags {
            flagged: true,
            ..MessageFlags::default()
        };
        f.store
            .upsert_message(&f.inbox, 1, &urgent, flagged, 100)
            .unwrap();
        f.store
            .set_header_importance(&id, Importance::High)
            .unwrap();
        let item = f.store.get_message(&id).unwrap().item;
        assert_eq!(item.importance, Importance::Low);
        assert!(item.pinned);
        assert_eq!(item.flag_due, Some(5000));
        // Leaving each out changes nothing; Some(None) clears the date.
        f.store.set_marks(&[&id], None, None, None).unwrap();
        assert_eq!(f.store.get_message(&id).unwrap().item.flag_due, Some(5000));
        f.store.set_marks(&[&id], None, None, Some(None)).unwrap();
        assert_eq!(f.store.get_message(&id).unwrap().item.flag_due, None);
    }

    #[test]
    fn follow_up_dates_end_with_the_flag() {
        let f = fixture();
        let id = add_message(&f.store, &f.inbox, 1, &summary("follow up", 1));
        let other = add_message(&f.store, &f.inbox, 2, &summary("other", 2));
        f.store
            .set_flags(&[&id, &other], None, Some(true), None)
            .unwrap();
        f.store
            .set_marks(&[&id, &other], None, None, Some(Some(1000)))
            .unwrap();
        assert_eq!(
            f.store.flag_due_between(999, 1000).unwrap(),
            [
                (id.clone(), "follow up".to_owned()),
                (other.clone(), "other".to_owned())
            ]
        );
        assert!(f.store.flag_due_between(1000, 2000).unwrap().is_empty());
        // Unflagged here, or on another device.
        f.store.set_flags(&[&id], None, Some(false), None).unwrap();
        assert_eq!(f.store.get_message(&id).unwrap().item.flag_due, None);
        f.store
            .set_flags_by_uid(
                &f.inbox,
                2,
                MessageFlags {
                    seen: false,
                    flagged: false,
                    answered: false,
                },
            )
            .unwrap();
        assert_eq!(f.store.get_message(&other).unwrap().item.flag_due, None);
        assert!(f.store.flag_due_between(0, 2000).unwrap().is_empty());
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

    /// A reply in a conversation started by `root`.
    fn reply(
        subject: &str,
        date: i64,
        root: &str,
        parent: &str,
    ) -> mail_core::message::MessageSummary {
        let mut m = summary(subject, date);
        m.in_reply_to = Some(parent.to_owned());
        m.references = vec![root.to_owned(), parent.to_owned()];
        m
    }

    #[test]
    fn conversations_show_once_with_their_newest_message() {
        let f = fixture();
        let sent = add_folder(&f.store, &f.account, "Sent", FolderRole::Sent);
        let trash = add_folder(&f.store, &f.account, "Trash", FolderRole::Trash);
        // A quote, my reply (in Sent), and their answer; plus an unrelated message.
        let quote = summary("Quote", 100);
        let root = quote.message_id.clone().unwrap_or_default();
        add_message(&f.store, &f.inbox, 1, &quote);
        add_message(&f.store, &sent, 1, &reply("Re: Quote", 200, &root, &root));
        let answer = add_message(
            &f.store,
            &f.inbox,
            2,
            &reply("Re: Quote", 300, &root, "mine@x"),
        );
        add_message(&f.store, &f.inbox, 3, &summary("Lunch", 250));
        add_message(&f.store, &trash, 1, &reply("Re: Quote", 50, &root, &root));

        let inbox = ListQuery {
            folder_id: Some(f.inbox.clone()),
            ..query()
        };
        let page = f.store.list_threads(&inbox).unwrap();
        assert_eq!(subjects(&page), ["Re: Quote", "Lunch"]);
        assert_eq!(page.total, 2);
        let thread = &page.items[0];
        assert_eq!(thread.id, answer);
        // Two of the three are in the Inbox; the row acts on those two.
        assert_eq!(thread.thread_count, 2);
        assert_eq!(thread.thread_ids.len(), 2);
        assert_eq!(thread.thread_ids[0], answer);
        // Unread because one of them is.
        assert!(thread.unread);
        // Ungrouped lists are unchanged.
        assert_eq!(f.store.list_messages(&inbox).unwrap().total, 3);

        // Opening it shows the whole conversation in order, Sent included,
        // Trash left out.
        let all = f.store.get_thread(&answer).unwrap();
        assert_eq!(
            all.iter().map(|m| m.date).collect::<Vec<_>>(),
            [Some(100), Some(200), Some(300)]
        );
    }

    /// A reply that names only what it answers, as some clients write them.
    fn bare_reply(subject: &str, date: i64, parent: &str) -> mail_core::message::MessageSummary {
        let mut m = summary(subject, date);
        m.in_reply_to = Some(format!("<{parent}>"));
        m
    }

    fn thread_dates(store: &Store, id: &str) -> Vec<Option<i64>> {
        store
            .get_thread(id)
            .unwrap()
            .iter()
            .map(|m| m.date)
            .collect()
    }

    #[test]
    fn a_reply_without_references_joins_its_parents_conversation() {
        let f = fixture();
        let quote = summary("Quote", 100);
        let root = quote.message_id.clone().unwrap();
        add_message(&f.store, &f.inbox, 1, &quote);
        let second = reply("Re: Quote", 200, &root, &root);
        let second_id = second.message_id.clone().unwrap();
        add_message(&f.store, &f.inbox, 2, &second);
        // In-Reply-To the second message, no References.
        let third = bare_reply("RE: Quote", 300, &second_id);
        let third_id = third.message_id.clone().unwrap();
        let third_row = add_message(&f.store, &f.inbox, 3, &third);
        // And an answer of the same kind to that one.
        let fourth = add_message(
            &f.store,
            &f.inbox,
            4,
            &bare_reply("RE: Quote", 400, &third_id),
        );
        let all = [Some(100), Some(200), Some(300), Some(400)];
        assert_eq!(thread_dates(&f.store, &fourth), all);
        assert_eq!(f.store.list_threads(&query()).unwrap().total, 1);
        // Reading the same message again (new flags) keeps it there.
        f.store
            .upsert_message(
                &f.inbox,
                3,
                &third,
                MessageFlags {
                    seen: true,
                    ..MessageFlags::default()
                },
                100,
            )
            .unwrap();
        assert_eq!(thread_dates(&f.store, &third_row), all);
    }

    #[test]
    fn replies_stored_before_their_parent_join_it_when_it_arrives() {
        let f = fixture();
        let sent = add_folder(&f.store, &f.account, "Sent", FolderRole::Sent);
        let quote = summary("Quote", 100);
        let root = quote.message_id.clone().unwrap();
        let mine = reply("Re: Quote", 200, &root, &root);
        let mine_id = mine.message_id.clone().unwrap();
        let theirs = bare_reply("RE: Quote", 300, &mine_id);
        let theirs_id = theirs.message_id.clone().unwrap();
        // The Inbox is read first: the newest messages, then older ones.
        let last = add_message(
            &f.store,
            &f.inbox,
            3,
            &bare_reply("RE: Quote", 400, &theirs_id),
        );
        add_message(&f.store, &f.inbox, 2, &theirs);
        add_message(&f.store, &f.inbox, 1, &quote);
        assert_eq!(thread_dates(&f.store, &last), [Some(300), Some(400)]);
        // My reply, from Sent, ties them to the conversation.
        add_message(&f.store, &sent, 1, &mine);
        assert_eq!(
            thread_dates(&f.store, &last),
            [Some(100), Some(200), Some(300), Some(400)]
        );
        // Another account's mail is never pulled in.
        let other = add_account(&f.store, "b@example.com");
        let other_inbox = add_folder(&f.store, &other, "INBOX", FolderRole::Inbox);
        let stray = add_message(
            &f.store,
            &other_inbox,
            1,
            &bare_reply("RE: Quote", 500, &mine_id),
        );
        assert_eq!(thread_dates(&f.store, &stray), [Some(500)]);
    }

    #[test]
    fn long_conversations_show_their_newest_messages_once_each() {
        let f = fixture();
        let labels = add_folder(&f.store, &f.account, "Label", FolderRole::Other);
        let quote = summary("Quote", 1);
        let root = quote.message_id.clone().unwrap();
        add_message(&f.store, &f.inbox, 1, &quote);
        for n in 2..=250u32 {
            // Two different messages from one sender in each second.
            let date = i64::from(n / 2);
            let mut m = reply("Re: Quote", date, &root, &root);
            m.message_id = Some(format!("reply-{n}@example.com"));
            add_message(&f.store, &f.inbox, n, &m);
        }
        // The newest one also sits in a second folder.
        let mut newest = reply("Re: Quote", 125, &root, &root);
        newest.message_id = Some("<reply-250@example.com>".into());
        let copy = add_message(&f.store, &labels, 1, &newest);
        let thread = f.store.get_thread(&copy).unwrap();
        // 200 read: the 199 newest messages, one of them twice.
        assert_eq!(thread.len(), 199);
        assert_eq!(thread.last().unwrap().date, Some(125));
        assert_eq!(thread.first().unwrap().date, Some(26));
        assert!(thread.windows(2).all(|w| w[0].date <= w[1].date));
        assert_eq!(thread.iter().filter(|m| m.date == Some(100)).count(), 2);
    }

    #[test]
    fn messages_without_reply_headers_stand_alone() {
        let f = fixture();
        let mut a = summary("Same subject", 1);
        a.message_id = None;
        let mut b = summary("Same subject", 2);
        b.message_id = None;
        add_message(&f.store, &f.inbox, 1, &a);
        let only = add_message(&f.store, &f.inbox, 2, &b);
        let page = f.store.list_threads(&query()).unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(f.store.get_thread(&only).unwrap().len(), 1);
    }

    #[test]
    fn focused_inbox_sorts_people_from_machines() {
        use mail_core::focus::Focus;
        let f = fixture();
        let sent = add_folder(&f.store, &f.account, "Sent", FolderRole::Sent);
        let archive = add_folder(&f.store, &f.account, "Archive", FolderRole::Archive);
        let from = |subject: &str, date: i64, address: &str, bulk: bool| {
            let mut m = summary(subject, date);
            m.from = Some(mail_core::message::Mailbox {
                name: None,
                address: address.into(),
            });
            m.bulk = bulk;
            m
        };
        add_message(
            &f.store,
            &f.inbox,
            1,
            &from("Quote", 1, "peter@client.example", false),
        );
        add_message(
            &f.store,
            &f.inbox,
            2,
            &from("Weekly news", 2, "editor@paper.example", true),
        );
        add_message(
            &f.store,
            &f.inbox,
            3,
            &from("Your code", 3, "noreply@bank.example", false),
        );
        add_message(
            &f.store,
            &f.inbox,
            4,
            &from("Agenda", 4, "events@club.example", true),
        );
        add_message(
            &f.store,
            &archive,
            1,
            &from("Old news", 5, "editor@paper.example", true),
        );
        // I write to the club: their list mail is Focused for me.
        let mut mine = summary("Re: Agenda", 6);
        mine.to = vec![mail_core::message::Mailbox {
            name: None,
            address: "Events@Club.example".into(),
        }];
        add_message(&f.store, &sent, 1, &mine);

        let view = |focus| ListQuery {
            role: Some(FolderRole::Inbox),
            focus: Some(focus),
            ..query()
        };
        assert_eq!(
            subjects(&f.store.list_messages(&view(Focus::Focused)).unwrap()),
            ["Agenda", "Quote"]
        );
        assert_eq!(
            subjects(&f.store.list_messages(&view(Focus::Other)).unwrap()),
            ["Your code", "Weekly news"]
        );
        assert_eq!(f.store.list_threads(&view(Focus::Other)).unwrap().total, 2);
        // Other folders ignore the split.
        let archived = ListQuery {
            folder_id: Some(archive.clone()),
            focus: Some(Focus::Focused),
            ..query()
        };
        assert_eq!(
            subjects(&f.store.list_messages(&archived).unwrap()),
            ["Old news"]
        );

        // Rules by address and by domain win.
        f.store
            .set_focus_rule("@paper.example", Some(Focus::Focused))
            .unwrap();
        f.store
            .set_focus_rule("peter@client.example", Some(Focus::Other))
            .unwrap();
        assert_eq!(
            subjects(&f.store.list_messages(&view(Focus::Focused)).unwrap()),
            ["Agenda", "Weekly news"]
        );
        assert_eq!(f.store.focus_rules().unwrap().len(), 2);
        f.store.set_focus_rule("@paper.example", None).unwrap();
        assert_eq!(f.store.focus_rules().unwrap().len(), 1);
    }
}
