//! Exchange Server accounts (Exchange Web Services): the mail sync, the
//! changes made here replayed on the server, bodies, sending and drafts,
//! folders, calendars, contacts, automatic replies and free/busy.
//!
//! Mail is kept exactly as IMAP mail is: each message gets a UID here (one
//! more than its folder's highest) beside its Exchange id, its header block
//! is read as an IMAP message's would be, and its body is the whole MIME
//! message. So search, rules, security, attachments and the rest work
//! unchanged. Changes made here are queued as for IMAP, translated to
//! Exchange ids when they are queued ([`enqueue`]).
//!
//! The server is asked for changes every few minutes (Settings › Check for
//! new mail every), and straight away when something changes here.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use mail_core::folders::FolderRole;
use mail_core::message::{MessageSummary, parse_summary};
use mail_store::{
    Account, Calendar, CalendarProvider, Folder, FoundCalendar, MessageFlags, OpKind,
};
use mail_sync::SyncError;
use mail_sync::ews::{self, EwsClient, EwsLogin, ItemChange, MessageChange, WellKnown};
use mail_sync::imap::MAX_BACKGROUND_BODY_BYTES;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::Ctx;
use crate::credentials::password_key;
use crate::error::{AppError, AppResult};
use crate::events;
use crate::state::{AppState, lock, now};
use crate::sync::{self, Op, Scope, Trigger};

/// Items asked for per `SyncFolderItems` page.
const PAGE: u32 = 512;
/// Pages read per folder per pass, so one huge folder doesn't hold up the
/// rest (the next pass carries on).
const MAX_PAGES: usize = 20;
/// Most messages kept from a folder's first sync (newest first); with a
/// sync window, every one inside it up to [`MAX_WINDOW_MESSAGES`].
const INITIAL_MESSAGES: u32 = 1000;
const MAX_WINDOW_MESSAGES: u32 = 20_000;
/// Calendar items and contacts asked for per page.
const OTHER_PAGE: u32 = 200;

/// A change made here, for the server: what it applies to and what it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EwsOp {
    /// One change on some messages.
    Ews {
        /// The messages.
        target: Target,
        /// The change.
        change: Change,
    },
}

/// Which messages a queued change applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum Target {
    /// By their Exchange ids.
    Items {
        /// Exchange ids.
        ids: Vec<String>,
        /// The folder they are in (path), to sync after.
        folder: String,
    },
    /// The messages with this Message-ID in a folder (moved here before
    /// their new Exchange id was known).
    MessageId {
        /// Folder path.
        folder: String,
        /// Message-ID without angle brackets.
        message_id: String,
    },
}

impl Target {
    fn folder(&self) -> &str {
        match self {
            Self::Items { folder, .. } | Self::MessageId { folder, .. } => folder,
        }
    }
}

/// What a queued change does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    /// Read or unread.
    Read {
        /// Read now.
        read: bool,
    },
    /// Flagged or not.
    Flagged {
        /// Flagged now.
        flagged: bool,
    },
    /// Categories added and removed.
    Categories {
        /// Added.
        add: Vec<String>,
        /// Removed.
        remove: Vec<String>,
    },
    /// Moved to a folder (path).
    Move {
        /// Target folder path.
        to: String,
        /// Made here first: create it on the server.
        create_target: bool,
    },
    /// Copied to a folder (path).
    Copy {
        /// Target folder path.
        to: String,
        /// Made here first: create it on the server.
        create_target: bool,
    },
    /// Deleted for good.
    Delete,
}

/// Clients kept per account, so the NTLM sign-in of one connection serves
/// many requests: (EWS address, sign-in name, password) and the client.
type ClientCache = HashMap<String, ((String, String, String), EwsClient)>;
static CLIENTS: Mutex<Option<ClientCache>> = Mutex::new(None);

/// Forget an account's client (its password changed, or it was removed).
pub fn forget_client(account_id: &str) {
    if let Some(cache) = lock(&CLIENTS).as_mut() {
        cache.remove(account_id);
    }
}

/// The Windows sign-in name an account uses: the one given, or its address.
fn username(account: &Account, settings: &mail_store::EwsSettings) -> String {
    settings
        .username
        .clone()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| account.email.clone())
}

/// A signed-in client for an Exchange account.
///
/// # Errors
/// `auth` when the password is missing; `config` without an EWS address.
pub fn client(state: &AppState, account: &Account) -> AppResult<EwsClient> {
    let settings = state.store.ews_settings(&account.id)?.ok_or_else(|| {
        AppError::new(
            "config",
            "This Exchange account has no server address. Remove it and add it again.",
        )
    })?;
    let password = state
        .secrets
        .get(&password_key(&account.id))?
        .ok_or_else(|| {
            AppError::new(
                "auth",
                "The password for this account is missing. Update it in Settings.",
            )
        })?;
    let key = (settings.url.clone(), username(account, &settings), password);
    let mut cache = lock(&CLIENTS);
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some((known, client)) = cache.get(&account.id)
        && *known == key
    {
        return Ok(client.clone());
    }
    let client = EwsClient::new(
        &settings.url,
        EwsLogin {
            username: key.1.clone(),
            password: key.2.clone(),
        },
    )?;
    cache.insert(account.id.clone(), (key, client.clone()));
    Ok(client)
}

/// Plain words for a refused Exchange sign-in.
#[must_use]
pub fn sign_in_error(error: SyncError) -> AppError {
    match error {
        SyncError::Auth => AppError::new(
            "auth",
            "Exchange didn't accept the sign-in. Check the user name (often DOMAIN\\name or your address) and the password.",
        ),
        other => other.into(),
    }
}

// ---------------------------------------------------------------- changes

fn folder_id_of(state: &AppState, account_id: &str, path: &str) -> AppResult<Option<String>> {
    Ok(state
        .store
        .folder_by_path(account_id, path)?
        .map(|(id, _)| id))
}

fn items(
    state: &AppState,
    account_id: &str,
    folder: &str,
    uids: &[u32],
) -> AppResult<Option<Target>> {
    let Some(folder_id) = folder_id_of(state, account_id, folder)? else {
        return Ok(None);
    };
    let ids = state.store.remote_ids_for_uids(&folder_id, uids)?;
    Ok((!ids.is_empty()).then(|| Target::Items {
        ids,
        folder: folder.to_owned(),
    }))
}

fn by_message_id(folder: &str, message_id: &str) -> Target {
    Target::MessageId {
        folder: folder.to_owned(),
        message_id: message_id.to_owned(),
    }
}

/// Category names from IMAP keywords (`$Junk` and the like are left out:
/// Exchange keeps junk by folder).
fn categories(keywords: &[String]) -> Vec<String> {
    keywords
        .iter()
        .filter_map(|k| mail_core::categories::name_from_keyword(k))
        .collect()
}

/// An IMAP change as Exchange changes: the folder's UIDs become Exchange
/// ids now, before the messages move here. Read and flagged are separate
/// changes; Gmail labels don't apply.
///
/// # Errors
/// Database errors.
#[allow(clippy::too_many_lines, reason = "one arm per kind of change")]
pub fn translate(state: &AppState, account_id: &str, op: &Op) -> AppResult<Vec<EwsOp>> {
    let mut out = Vec::new();
    let mut push = |target: Option<Target>, change: Change| {
        if let Some(target) = target {
            out.push(EwsOp::Ews { target, change });
        }
    };
    match op {
        Op::SetFlags {
            folder,
            uids,
            seen,
            flagged,
            ..
        } => {
            let target = items(state, account_id, folder, uids)?;
            if let Some(read) = seen {
                push(target.clone(), Change::Read { read: *read });
            }
            if let Some(flagged) = flagged {
                push(target, Change::Flagged { flagged: *flagged });
            }
        }
        Op::SetFlagsByMessageId {
            folder,
            message_id,
            seen,
            flagged,
        } => {
            let target = Some(by_message_id(folder, message_id));
            if let Some(read) = seen {
                push(target.clone(), Change::Read { read: *read });
            }
            if let Some(flagged) = flagged {
                push(target, Change::Flagged { flagged: *flagged });
            }
        }
        Op::Move {
            from,
            uids,
            to,
            create_target,
        } => push(
            items(state, account_id, from, uids)?,
            Change::Move {
                to: to.clone(),
                create_target: *create_target,
            },
        ),
        Op::MoveByMessageId {
            from,
            message_id,
            to,
            create_target,
        } => push(
            Some(by_message_id(from, message_id)),
            Change::Move {
                to: to.clone(),
                create_target: *create_target,
            },
        ),
        Op::Copy {
            from,
            uids,
            to,
            create_target,
        } => push(
            items(state, account_id, from, uids)?,
            Change::Copy {
                to: to.clone(),
                create_target: *create_target,
            },
        ),
        Op::CopyByMessageId {
            from,
            message_id,
            to,
            create_target,
        } => push(
            Some(by_message_id(from, message_id)),
            Change::Copy {
                to: to.clone(),
                create_target: *create_target,
            },
        ),
        Op::Delete { folder, uids } => {
            push(items(state, account_id, folder, uids)?, Change::Delete);
        }
        Op::DeleteByMessageId { folder, message_id } => {
            push(Some(by_message_id(folder, message_id)), Change::Delete);
        }
        Op::SetKeywords {
            folder,
            uids,
            add,
            remove,
        } => {
            let (add, remove) = (categories(add), categories(remove));
            if !add.is_empty() || !remove.is_empty() {
                push(
                    items(state, account_id, folder, uids)?,
                    Change::Categories { add, remove },
                );
            }
        }
        Op::SetKeywordsByMessageId {
            folder,
            message_id,
            add,
            remove,
        } => {
            let (add, remove) = (categories(add), categories(remove));
            if !add.is_empty() || !remove.is_empty() {
                push(
                    Some(by_message_id(folder, message_id)),
                    Change::Categories { add, remove },
                );
            }
        }
        Op::SetLabels { .. } | Op::SetLabelsByMessageId { .. } => {}
    }
    Ok(out)
}

/// Queue a change for an Exchange account and wake its engine.
///
/// # Errors
/// Database errors.
pub fn enqueue(state: &AppState, account_id: &str, op: &Op) -> AppResult<()> {
    for ews_op in translate(state, account_id, op)? {
        let EwsOp::Ews { change, .. } = &ews_op;
        let kind = match change {
            Change::Read { .. } | Change::Flagged { .. } | Change::Categories { .. } => {
                OpKind::SetFlags
            }
            Change::Move { .. } => OpKind::Move,
            Change::Copy { .. } => OpKind::Copy,
            Change::Delete => OpKind::Delete,
        };
        let payload =
            serde_json::to_value(&ews_op).map_err(|e| AppError::internal(e.to_string()))?;
        state.store.enqueue_op(account_id, kind, &payload, now())?;
    }
    state.wake_sync(Some(account_id));
    Ok(())
}

/// The Exchange id of the folder at `path`, made on the server first when
/// `create` (a folder made here, such as Archive).
async fn folder_remote(
    state: &AppState,
    account: &Account,
    client: &EwsClient,
    path: &str,
    create: bool,
) -> AppResult<Option<String>> {
    match state.store.folder_by_path(&account.id, path)? {
        Some((_, Some(remote))) => Ok(Some(remote)),
        Some((local, None)) if create => {
            let root = client.well_known().await?.root.ok_or_else(|| {
                AppError::internal("Exchange didn't say where the mailbox starts")
            })?;
            let name = path.rsplit('/').next().unwrap_or(path);
            let made = client.create_folder(&root, name).await?;
            state.store.set_folder_remote_id(&local, &made)?;
            Ok(Some(made))
        }
        _ => Ok(None),
    }
}

async fn resolve(
    state: &AppState,
    account: &Account,
    client: &EwsClient,
    target: &Target,
) -> AppResult<Vec<String>> {
    match target {
        Target::Items { ids, .. } => Ok(ids.clone()),
        Target::MessageId { folder, message_id } => {
            let Some(remote) = folder_remote(state, account, client, folder, false).await? else {
                return Ok(Vec::new());
            };
            Ok(client.find_by_message_id(&remote, message_id).await?)
        }
    }
}

/// Apply one queued change. Messages gone meanwhile count as done.
async fn apply(
    state: &AppState,
    account: &Account,
    client: &EwsClient,
    op: &EwsOp,
) -> AppResult<()> {
    let EwsOp::Ews { target, change } = op;
    let ids = resolve(state, account, client, target).await?;
    if ids.is_empty() {
        return Ok(());
    }
    match change {
        Change::Read { read } => {
            client
                .update_messages(&ids, &MessageChange::Read(*read))
                .await?;
        }
        Change::Flagged { flagged } => {
            client
                .update_messages(&ids, &MessageChange::Flagged(*flagged))
                .await?;
        }
        Change::Categories { add, remove } => {
            for entry in client.get_headers(&ids).await? {
                let mut names = entry.state.categories.clone().unwrap_or_default();
                names.retain(|n| !remove.iter().any(|r| r.eq_ignore_ascii_case(n)));
                for name in add {
                    if !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                        names.push(name.clone());
                    }
                }
                client
                    .update_messages(
                        std::slice::from_ref(&entry.state.id),
                        &MessageChange::Categories(&names),
                    )
                    .await?;
            }
        }
        Change::Move { to, create_target } | Change::Copy { to, create_target } => {
            let Some(remote) = folder_remote(state, account, client, to, *create_target).await?
            else {
                return Err(AppError::new(
                    "not_found",
                    format!("The folder {to} isn't on the server."),
                ));
            };
            let copy = matches!(change, Change::Copy { .. });
            client.move_or_copy(&ids, &remote, copy).await?;
        }
        Change::Delete => client.delete_items(&ids, true).await?,
    }
    Ok(())
}

/// The folders a queued change reads or writes.
fn op_folders(op: &EwsOp) -> Vec<String> {
    let EwsOp::Ews { target, change } = op;
    let mut out = vec![target.folder().to_owned()];
    if let Change::Move { to, .. } | Change::Copy { to, .. } = change {
        out.push(to.clone());
    }
    out
}

async fn replay(
    state: &AppState,
    account: &Account,
    client: &EwsClient,
    touched: &mut HashSet<String>,
) -> AppResult<()> {
    for pending in state.store.list_ops(&account.id)? {
        let result = match serde_json::from_value::<EwsOp>(pending.payload.clone()) {
            Ok(op) => {
                touched.extend(op_folders(&op));
                apply(state, account, client, &op).await
            }
            // Something this version doesn't know: dropped.
            Err(_) => Ok(()),
        };
        match result {
            Ok(()) => {}
            Err(error) if sync::ends_session(error.kind) || error.kind == "connect" => {
                return Err(error);
            }
            Err(error) if error.kind == "internal" && error.message.contains("busy") => {
                return Err(error);
            }
            Err(error) => {
                let refusals = state.store.record_op_refusal(&pending.id)?;
                if !sync::give_up_after(refusals) {
                    return Err(AppError::internal(format!(
                        "Exchange refused a change ({}). Trying again shortly.",
                        error.message
                    )));
                }
                eprintln!(
                    "giving up on a change Exchange refused {refusals} times: {}",
                    error.message
                );
            }
        }
        state.store.delete_op(&pending.id)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- folders

/// The well-known folder an id is, as a role.
fn role_of(id: &str, known: &WellKnown) -> Option<FolderRole> {
    let is = |slot: &Option<String>| slot.as_deref() == Some(id);
    if is(&known.inbox) {
        Some(FolderRole::Inbox)
    } else if is(&known.sent) {
        Some(FolderRole::Sent)
    } else if is(&known.drafts) {
        Some(FolderRole::Drafts)
    } else if is(&known.trash) {
        Some(FolderRole::Trash)
    } else if is(&known.junk) {
        Some(FolderRole::Junk)
    } else if is(&known.archive) {
        Some(FolderRole::Archive)
    } else {
        None
    }
}

/// A folder's path: its names from the top of the mailbox, joined with
/// `/` (a `/` in a name becomes `∕`, so the levels stay apart).
fn path_of(id: &str, by_id: &HashMap<String, ews::Folder>, root: &str) -> Option<String> {
    let mut names = Vec::new();
    let mut at = id.to_owned();
    for _ in 0..32 {
        if at == root {
            names.reverse();
            return Some(names.join("/"));
        }
        let folder = by_id.get(&at)?;
        if !folder.is_mail() {
            return None;
        }
        names.push(folder.name.replace('/', "\u{2215}"));
        at = folder.parent_id.clone()?;
    }
    None
}

/// The mail folders of the mailbox, as (Exchange id, path, name, role).
#[must_use]
pub fn mail_folders(
    folders: &[ews::Folder],
    known: &WellKnown,
) -> Vec<(String, String, String, FolderRole)> {
    let Some(root) = known.root.as_deref() else {
        return Vec::new();
    };
    let by_id: HashMap<String, ews::Folder> =
        folders.iter().map(|f| (f.id.clone(), f.clone())).collect();
    let mut out = Vec::new();
    for folder in folders {
        if !folder.is_mail() || known.outbox.as_deref() == Some(folder.id.as_str()) {
            continue;
        }
        let Some(path) = path_of(&folder.id, &by_id, root) else {
            continue;
        };
        let top_level = folder.parent_id.as_deref() == Some(root);
        let role = role_of(&folder.id, known).unwrap_or(
            if top_level && known.archive.is_none() && folder.name.eq_ignore_ascii_case("archive") {
                FolderRole::Archive
            } else {
                FolderRole::Other
            },
        );
        let name = if role == FolderRole::Inbox {
            "Inbox".to_owned()
        } else {
            folder.name.clone()
        };
        out.push((folder.id.clone(), path, name, role));
    }
    out
}

/// Read the mailbox's folders and keep them here.
async fn refresh_folders(state: &AppState, account: &Account, client: &EwsClient) -> AppResult<()> {
    let known = client.well_known().await?;
    let hierarchy = client.sync_hierarchy(None).await?;
    let mut keep = Vec::new();
    for (remote, path, name, role) in mail_folders(&hierarchy.changed, &known) {
        state
            .store
            .upsert_ews_folder(&account.id, &remote, &path, &name, role)?;
        keep.push(remote);
    }
    if !keep.is_empty() {
        state.store.delete_ews_folders_not_in(&account.id, &keep)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- mail sync

struct Report {
    changed: bool,
    new_unseen: u32,
    new_mail: Vec<(String, bool)>,
}

/// A summary from a list entry's header block; a bare one when it can't be
/// read (the body brings the rest).
fn summary_of(entry: &ews::Headers) -> MessageSummary {
    let block = ews::convert::header_block(entry);
    parse_summary(block.as_bytes()).unwrap_or_else(|_| MessageSummary {
        message_id: entry
            .message_id
            .as_deref()
            .map(|m| m.trim_matches(['<', '>']).to_owned()),
        subject: entry.subject.clone(),
        from: None,
        reply_to: Vec::new(),
        to: Vec::new(),
        cc: Vec::new(),
        date: entry.date.as_deref().and_then(ews::convert::parse_time),
        preview: String::new(),
        has_html: false,
        attachments: Vec::new(),
        in_reply_to: None,
        references: Vec::new(),
        bulk: false,
        importance: mail_core::importance::Importance::Normal,
        signed: false,
        encrypted: false,
    })
}

/// The newest items of a folder a first sync keeps (within the sync window
/// when there is one).
async fn wanted_first(
    client: &EwsClient,
    remote: &str,
    account: &Account,
) -> AppResult<HashSet<String>> {
    let since = account.sync_months.map(|months| {
        i64::try_from(now())
            .unwrap_or(0)
            .saturating_sub(i64::from(months).saturating_mul(31 * 86_400))
    });
    let most = if since.is_some() {
        MAX_WINDOW_MESSAGES
    } else {
        INITIAL_MESSAGES
    };
    let mut out = HashSet::new();
    let mut offset = 0u32;
    while offset < most {
        let page = client
            .find_newest(
                remote,
                offset,
                INITIAL_MESSAGES.min(most.saturating_sub(offset)),
                since,
            )
            .await?;
        let got = u32::try_from(page.items.len()).unwrap_or(u32::MAX);
        out.extend(page.items.into_iter().map(|i| i.id));
        if page.last || got == 0 {
            break;
        }
        offset = offset.saturating_add(got);
        if since.is_none() {
            break;
        }
    }
    Ok(out)
}

/// Store new messages: their list entries, a UID each, their state.
async fn add_messages(
    state: &AppState,
    client: &EwsClient,
    folder: &Folder,
    ids: &[String],
    report: (&mut Report, bool),
) -> AppResult<()> {
    let (report, announce) = report;
    if ids.is_empty() {
        return Ok(());
    }
    let mut highest = state.store.get_folder(&folder.id)?.highest_uid.unwrap_or(0);
    for entry in client.get_headers(ids).await? {
        let summary = summary_of(&entry);
        highest = highest.saturating_add(1);
        let flags = MessageFlags {
            seen: entry.state.read.unwrap_or(true),
            flagged: entry.state.flagged.unwrap_or(false),
            answered: false,
        };
        let id = state.store.upsert_message(
            &folder.id,
            highest,
            &summary,
            flags,
            entry.size.unwrap_or(0),
        )?;
        state.store.set_message_remote_id(&id, &entry.state.id)?;
        if let Some(names) = &entry.state.categories {
            state.store.ensure_categories(names)?;
            state
                .store
                .set_categories_by_uid(&folder.id, highest, names)?;
        }
        if announce && folder.role == FolderRole::Inbox {
            if !flags.seen {
                report.new_unseen = report.new_unseen.saturating_add(1);
            }
            report.new_mail.push((id, !flags.seen));
        }
        report.changed = true;
    }
    state.store.set_highest_uid(&folder.id, highest)?;
    Ok(())
}

/// Bring a folder up to date: new messages, changed and deleted ones, and
/// some bodies for reading offline.
async fn sync_folder(
    state: &AppState,
    account: &Account,
    client: &EwsClient,
    folder: &Folder,
) -> AppResult<Report> {
    let mut report = Report {
        changed: false,
        new_unseen: 0,
        new_mail: Vec::new(),
    };
    let Some(remote) = state.store.folder_remote_id(&folder.id)? else {
        return Ok(report);
    };
    let mut token = state.store.folder_sync_state(&folder.id)?;
    let mut first = token.is_none();
    let mut wanted = if first {
        Some(wanted_first(client, &remote, account).await?)
    } else {
        None
    };
    let mut reset_once = false;
    for _ in 0..MAX_PAGES {
        let page = match client
            .sync_items(&remote, token.as_deref(), PAGE, true)
            .await
        {
            Ok(page) => page,
            Err(SyncError::Refused(code))
                if code == ews::parse::INVALID_SYNC_STATE && !reset_once =>
            {
                // The server forgot where this folder's sync was: read it again.
                reset_once = true;
                state.store.reset_ews_folder(&folder.id)?;
                token = None;
                first = true;
                wanted = Some(wanted_first(client, &remote, account).await?);
                report.changed = true;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let known = state.store.ews_items(&folder.id)?;
        let mut created = Vec::new();
        for change in page.changes {
            let new = matches!(change, ItemChange::Created(_));
            match change {
                ItemChange::Created(item) | ItemChange::Updated(item) => {
                    if let Some(local) = known.get(&item.id) {
                        let stored = state.store.get_message(&local.id)?;
                        let flags = MessageFlags {
                            seen: item.read.unwrap_or(!stored.item.unread),
                            flagged: item.flagged.unwrap_or(stored.item.flagged),
                            answered: stored.item.answered,
                        };
                        report.changed |=
                            state.store.set_flags_by_uid(&folder.id, local.uid, flags)?;
                        if let Some(names) = &item.categories {
                            state.store.ensure_categories(names)?;
                            report.changed |= state
                                .store
                                .set_categories_by_uid(&folder.id, local.uid, names)?;
                        }
                    } else if new && wanted.as_ref().is_none_or(|w| w.contains(&item.id)) {
                        // New mail. (A change to old mail never kept here
                        // doesn't bring it in.)
                        created.push(item.id);
                    }
                }
                ItemChange::ReadChanged { id, read } => {
                    if let Some(local) = known.get(&id) {
                        let stored = state.store.get_message(&local.id)?;
                        let flags = MessageFlags {
                            seen: read,
                            flagged: stored.item.flagged,
                            answered: stored.item.answered,
                        };
                        report.changed |=
                            state.store.set_flags_by_uid(&folder.id, local.uid, flags)?;
                    }
                }
                ItemChange::Deleted(id) => {
                    if let Some(local) = known.get(&id) {
                        report.changed |= state.store.delete_by_uids(&folder.id, &[local.uid])? > 0;
                    }
                }
            }
        }
        add_messages(state, client, folder, &created, (&mut report, !first)).await?;
        token = Some(page.sync_state);
        // A first sync is saved once it is whole; later ones page by page.
        if !first || page.done {
            state
                .store
                .set_folder_sync_state(&folder.id, token.as_deref())?;
        }
        if page.done {
            break;
        }
    }
    if state.store.list_ops(&account.id)?.is_empty()
        && state.store.delete_unplaced_moves(&folder.id)? > 0
    {
        report.changed = true;
    }
    report.changed |= prefetch(state, client, folder).await?;
    Ok(report)
}

/// Download small bodies of the newest messages so they open offline.
async fn prefetch(state: &AppState, client: &EwsClient, folder: &Folder) -> AppResult<bool> {
    let count = if folder.role == FolderRole::Inbox {
        sync::INBOX_BODY_PREFETCH
    } else {
        sync::BODY_PREFETCH
    };
    let missing = state.store.messages_missing_raw(
        &folder.id,
        count,
        u64::from(MAX_BACKGROUND_BODY_BYTES),
    )?;
    if missing.is_empty() {
        return Ok(false);
    }
    let by_remote: HashMap<String, String> = state
        .store
        .ews_items(&folder.id)?
        .into_iter()
        .filter(|(_, item)| missing.iter().any(|(id, _)| *id == item.id))
        .map(|(remote, item)| (remote, item.id))
        .collect();
    let remotes: Vec<String> = by_remote.keys().cloned().collect();
    let mut fetched = false;
    for item in client.get_mime(&remotes).await? {
        if let Some(id) = by_remote.get(&item.id) {
            sync::cache_body(state, id, &item.raw)?;
            fetched = true;
        }
    }
    Ok(fetched)
}

/// One connected stretch of an Exchange account's sync: changes sent, the
/// folders read, then a wait for the next check. `Ok` to start again (the
/// periodic full sync).
pub async fn session(
    ctx: &Ctx,
    state: &AppState,
    account: &Account,
    wake: &Notify,
) -> AppResult<()> {
    let client = client(state, account)?;
    // Signing in first: a wrong password is reported as such.
    client.test().await.map_err(sign_in_error)?;
    let started = Instant::now();
    let mut trigger = Trigger::Connected;
    loop {
        if crate::settings::working_offline(state) {
            return Ok(());
        }
        let mut problem: Option<AppError> = None;
        let mut touched = HashSet::new();
        if let Err(error) = replay(state, account, &client, &mut touched).await {
            sync::stop_if_disconnected(error, &mut problem)?;
        }
        let scope = sync::scope_of(trigger, !touched.is_empty());
        if scope == Scope::All
            && let Err(error) = refresh_folders(state, account, &client).await
        {
            sync::stop_if_disconnected(error, &mut problem)?;
        }
        let folders = state.store.list_folders(&account.id)?;
        let mut due: Vec<&Folder> = folders
            .iter()
            .filter(|f| sync::folder_due(scope, f.role, touched.contains(&f.path)))
            .collect();
        due.sort_by_key(|f| f.role != FolderRole::Inbox);
        let mut new_unseen: u32 = 0;
        for folder in due {
            let report = match sync_folder(state, account, &client, folder).await {
                Ok(report) => report,
                Err(error) => {
                    sync::stop_if_disconnected(error, &mut problem)?;
                    continue;
                }
            };
            if report.changed {
                events::changed(ctx, &account.id, std::slice::from_ref(&folder.id));
            }
            let handled = if report.new_mail.is_empty() {
                0
            } else {
                crate::rules::on_new_mail(ctx, state, account, &report.new_mail).unwrap_or_else(
                    |error| {
                        eprintln!("mail rules didn't run: {}", error.message);
                        0
                    },
                )
            };
            new_unseen = new_unseen.saturating_add(report.new_unseen.saturating_sub(handled));
        }
        if new_unseen > 0 {
            events::new_mail(ctx, state, account, new_unseen);
        }
        if let Some(error) = problem {
            return Err(error);
        }
        events::sync_status(ctx, &account.id, "idle", None);
        // Exchange is asked again every few minutes, or now for a wake.
        let poll = Duration::from_secs(
            u64::from(crate::settings::load(state).map_or(2, |s| s.check_every_mins))
                .saturating_mul(60),
        );
        let remaining = sync::FULL_SYNC_EVERY
            .max(poll)
            .saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Ok(());
        }
        trigger = tokio::select! {
            () = tokio::time::sleep(poll.min(remaining)) => Trigger::Waited,
            () = wake.notified() => Trigger::Woken,
        };
        if trigger == Trigger::Woken {
            events::sync_status(ctx, &account.id, "syncing", None);
        }
    }
}

/// A message's whole MIME content, downloaded now.
///
/// # Errors
/// `connect` when offline; `not_found` when it was deleted on the server.
pub async fn fetch_body(state: &AppState, account: &Account, id: &str) -> AppResult<Vec<u8>> {
    let remote = state.store.message_remote_id(id)?.ok_or_else(|| {
        AppError::new(
            "connect",
            "This message is still being moved on the server. Try again in a moment.",
        )
    })?;
    let client = client(state, account)?;
    let raw = client
        .get_message(&remote)
        .await?
        .ok_or_else(|| AppError::new("not_found", "This message was deleted on the server."))?;
    sync::cache_body(state, id, &raw)?;
    Ok(raw)
}

/// Download the bodies of a folder's messages not on this device yet (for
/// exporting it).
///
/// # Errors
/// Network and database errors.
pub async fn fetch_missing_bodies(
    state: &AppState,
    account: &Account,
    folder: &Folder,
) -> AppResult<()> {
    let client = client(state, account)?;
    let rows = state.store.export_rows(&folder.id)?;
    let missing: HashSet<String> = rows
        .into_iter()
        .filter(|r| !r.has_raw)
        .map(|r| r.id)
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let by_remote: HashMap<String, String> = state
        .store
        .ews_items(&folder.id)?
        .into_iter()
        .filter(|(_, item)| missing.contains(&item.id))
        .map(|(remote, item)| (remote, item.id))
        .collect();
    let remotes: Vec<String> = by_remote.keys().cloned().collect();
    for item in client.get_mime(&remotes).await? {
        if let Some(id) = by_remote.get(&item.id) {
            sync::cache_body(state, id, &item.raw)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- sending

/// Send a queued message through Exchange (a copy goes to Sent Items). The
/// blind copies are the envelope's recipients the message doesn't name.
///
/// # Errors
/// As [`EwsClient::send_mime`].
pub async fn send(
    state: &AppState,
    account: &Account,
    raw: &[u8],
    envelope: &[String],
) -> Result<(), SyncError> {
    let client = client(state, account).map_err(|e| SyncError::Config(e.message))?;
    let bcc = parse_summary(raw)
        .map(|shown| crate::outbox::hidden_recipients(envelope, &shown.to, &shown.cc))
        .unwrap_or_default();
    client.send_mime(raw, &bcc).await
}

/// Whether a message with this Message-ID is in a folder (Sent, after the
/// app stopped mid-send).
///
/// # Errors
/// Network errors and refusals.
pub async fn folder_has_message(
    state: &AppState,
    account: &Account,
    path: &str,
    message_id: &str,
) -> AppResult<bool> {
    let client = client(state, account)?;
    let Some(remote) = folder_remote(state, account, &client, path, false).await? else {
        return Ok(false);
    };
    Ok(!client
        .find_by_message_id(&remote, message_id)
        .await?
        .is_empty())
}

/// Save a draft (whole MIME) in the account's Drafts folder.
///
/// # Errors
/// Network errors and refusals.
pub async fn save_draft(
    state: &AppState,
    account: &Account,
    path: &str,
    raw: &[u8],
) -> AppResult<()> {
    let client = client(state, account)?;
    let remote = match folder_remote(state, account, &client, path, false).await? {
        Some(remote) => remote,
        None => client
            .well_known()
            .await?
            .drafts
            .ok_or_else(|| AppError::invalid("This account has no Drafts folder."))?,
    };
    client.save_mime(raw, &remote, true).await?;
    Ok(())
}

/// Save an imported message (whole MIME) in the folder at `path`.
///
/// # Errors
/// Network errors and refusals.
pub async fn import_message(
    state: &AppState,
    account: &Account,
    path: &str,
    raw: &[u8],
) -> AppResult<()> {
    let client = client(state, account)?;
    let remote = folder_remote(state, account, &client, path, true)
        .await?
        .ok_or_else(|| {
            AppError::new(
                "not_found",
                format!("The folder {path} isn't on the server."),
            )
        })?;
    client.save_mime(raw, &remote, false).await?;
    Ok(())
}

/// Make the folders of `path` that aren't on the server yet (for imports).
///
/// # Errors
/// Network errors and refusals.
pub async fn ensure_folder(state: &AppState, account: &Account, path: &str) -> AppResult<()> {
    let client = client(state, account)?;
    let mut parent: Option<String> = None;
    let mut so_far = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        if !so_far.is_empty() {
            so_far.push('/');
        }
        so_far.push_str(part);
        if let Some((_, Some(remote))) = state.store.folder_by_path(&account.id, &so_far)? {
            parent = Some(remote);
            continue;
        }
        let under = match parent.clone() {
            Some(p) => p,
            None => client.well_known().await?.root.ok_or_else(|| {
                AppError::internal("Exchange didn't say where the mailbox starts")
            })?,
        };
        let made = client.create_folder(&under, part).await?;
        state
            .store
            .upsert_ews_folder(&account.id, &made, &so_far, part, FolderRole::Other)?;
        parent = Some(made);
    }
    Ok(())
}

// ---------------------------------------------------------------- folders

/// After a folder change: read the folder list again and tell the
/// interface.
async fn folders_changed(ctx: &Ctx, account: &Account, client: &EwsClient) -> AppResult<()> {
    refresh_folders(&ctx.state, account, client).await?;
    events::changed(ctx, &account.id, &[]);
    Ok(())
}

/// `create_folder` for an Exchange account.
///
/// # Errors
/// Network errors and refusals.
pub async fn create_folder(
    ctx: &Ctx,
    account: &Account,
    parent: Option<&Folder>,
    name: &str,
) -> AppResult<()> {
    let state = &ctx.state;
    let name = mail_core::folders::clean_folder_name(name, Some("/")).map_err(AppError::invalid)?;
    let client = client(state, account)?;
    let under =
        match parent {
            Some(folder) => state
                .store
                .folder_remote_id(&folder.id)?
                .ok_or_else(|| AppError::invalid("That folder isn't on the server yet."))?,
            None => client.well_known().await?.root.ok_or_else(|| {
                AppError::internal("Exchange didn't say where the mailbox starts")
            })?,
        };
    client
        .create_folder(&under, &name)
        .await
        .map_err(|error| match error {
            SyncError::Refused(why)
                if why.contains("already exists") || why.contains("ErrorFolderExists") =>
            {
                AppError::invalid("There's already a folder with that name here.")
            }
            other => other.into(),
        })?;
    folders_changed(ctx, account, &client).await
}

/// `rename_folder` for an Exchange account.
///
/// # Errors
/// Network errors and refusals.
pub async fn rename_folder(
    ctx: &Ctx,
    account: &Account,
    folder: &Folder,
    name: &str,
) -> AppResult<()> {
    let state = &ctx.state;
    let name = mail_core::folders::clean_folder_name(name, Some("/")).map_err(AppError::invalid)?;
    let client = client(state, account)?;
    let remote = state
        .store
        .folder_remote_id(&folder.id)?
        .ok_or_else(|| AppError::invalid("That folder isn't on the server yet."))?;
    client.rename_folder(&remote, &name).await?;
    folders_changed(ctx, account, &client).await
}

/// `delete_folder` for an Exchange account (to Deleted Items, with its mail).
///
/// # Errors
/// Network errors and refusals.
pub async fn delete_folder(ctx: &Ctx, account: &Account, folder: &Folder) -> AppResult<()> {
    let state = &ctx.state;
    let client = client(state, account)?;
    let remote = state
        .store
        .folder_remote_id(&folder.id)?
        .ok_or_else(|| AppError::invalid("That folder isn't on the server yet."))?;
    client.delete_folder(&remote).await?;
    folders_changed(ctx, account, &client).await
}

// ---------------------------------------------------------------- calendar

/// The `url` an Exchange calendar is kept under here.
fn calendar_url(remote: &str) -> String {
    format!("ews:{remote}")
}

/// Whether a calendar is an Exchange one.
#[must_use]
pub fn is_ews_calendar(calendar: &Calendar) -> bool {
    calendar.url.starts_with("ews:")
}

fn setting_key(kind: &str, id: &str) -> String {
    format!("ews_{kind}_state:{id}")
}

/// Read an account's calendars and what changed in them.
///
/// # Errors
/// Network and database errors.
pub async fn sync_calendars(state: &AppState, account: &Account) -> AppResult<()> {
    let client = client(state, account)?;
    let known = client.well_known().await?;
    let hierarchy = client.sync_hierarchy(None).await?;
    let mut found: Vec<FoundCalendar> = hierarchy
        .changed
        .iter()
        .filter(|f| f.is_calendar())
        .map(|f| FoundCalendar {
            url: calendar_url(&f.id),
            name: if f.name.is_empty() {
                "Calendar".to_owned()
            } else {
                f.name.clone()
            },
            color: None,
            writable: true,
            provider: CalendarProvider::Caldav,
            remote_id: Some(f.id.clone()),
        })
        .collect();
    // The main calendar first.
    found.sort_by_key(|c| c.remote_id != known.calendar);
    let calendars = state.store.set_calendars(&account.id, &found)?;
    for calendar in &calendars {
        sync_calendar(state, &client, calendar).await?;
    }
    Ok(())
}

async fn sync_calendar(state: &AppState, client: &EwsClient, calendar: &Calendar) -> AppResult<()> {
    let Some(remote) = calendar.remote_id.as_deref() else {
        return Ok(());
    };
    let key = setting_key("calendar", &calendar.id);
    let mut token = state.store.get_setting(&key)?.filter(|t| !t.is_empty());
    let full = token.is_none();
    let mut listed: Vec<String> = Vec::new();
    for _ in 0..MAX_PAGES {
        let page = match client
            .sync_items(remote, token.as_deref(), OTHER_PAGE, false)
            .await
        {
            Ok(page) => page,
            Err(SyncError::Refused(code))
                if code == ews::parse::INVALID_SYNC_STATE && token.is_some() =>
            {
                state.store.set_setting(&key, "")?;
                token = None;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let mut wanted = Vec::new();
        for change in page.changes {
            match change {
                ItemChange::Created(item) | ItemChange::Updated(item) => wanted.push(item.id),
                ItemChange::Deleted(id) => {
                    if let Some(stored) = state.store.calendar_event_at(&calendar.id, &id)? {
                        state.store.delete_calendar_event(&stored.id)?;
                    }
                }
                ItemChange::ReadChanged { .. } => {}
            }
        }
        for item in client.get_mime(&wanted).await? {
            if let Some(ics) = ews::convert::calendar_text(&item.raw) {
                state.store.put_calendar_event(
                    &calendar.id,
                    &item.id,
                    item.change_key.as_deref(),
                    &ics,
                )?;
                listed.push(item.id);
            }
        }
        token = Some(page.sync_state);
        if !full || page.done {
            state
                .store
                .set_setting(&key, token.as_deref().unwrap_or(""))?;
        }
        if page.done {
            if full {
                state
                    .store
                    .forget_calendar_events_not_on_server(&calendar.id, &listed)?;
            }
            break;
        }
    }
    Ok(())
}

/// Save an event in an Exchange calendar (Exchange sends the invitations),
/// then read it back.
///
/// # Errors
/// `invalid` for one date of a series, which isn't supported for Exchange
/// yet; network errors and refusals.
pub async fn save_event(
    state: &AppState,
    account: &Account,
    calendar: &Calendar,
    existing: Option<&mail_store::StoredEvent>,
    input: &mail_core::ical::EventInput,
    rule: Option<Option<&mail_core::ical::RepeatRule>>,
) -> AppResult<mail_store::StoredEvent> {
    let client = client(state, account)?;
    let remote = calendar
        .remote_id
        .clone()
        .ok_or_else(|| AppError::internal("Exchange calendar without an id"))?;
    if input.scope == mail_core::ical::Scope::This && input.occurrence.is_some() {
        return Err(AppError::invalid(
            "Changing one date of a repeating Exchange event isn't available yet. Change the whole series, or change it in Outlook.",
        ));
    }
    let (fields, recurrence) = ews::convert::event_fields(input, rule)?;
    let notify = input.notify_attendees;
    let href = match existing {
        Some(stored) if stored.calendar_id != calendar.id => {
            return Err(AppError::invalid(
                "Events can't move between an Exchange calendar and another one. Create it again in the other calendar.",
            ));
        }
        Some(stored) => {
            client
                .update_event(&stored.href, &fields, recurrence.as_deref(), notify)
                .await?;
            stored.href.clone()
        }
        None => client.create_event(&remote, &fields, notify).await?,
    };
    sync_calendar(state, &client, calendar).await?;
    // A newly made one may not be in the changes yet: read it directly.
    if state
        .store
        .calendar_event_at(&calendar.id, &href)?
        .is_none()
        && let Some(item) = client
            .get_mime(std::slice::from_ref(&href))
            .await?
            .into_iter()
            .next()
        && let Some(ics) = ews::convert::calendar_text(&item.raw)
    {
        state
            .store
            .put_calendar_event(&calendar.id, &item.id, item.change_key.as_deref(), &ics)?;
    }
    state.store.calendar_event_at(&calendar.id, &href)?.ok_or_else(|| {
        AppError::invalid("The event was saved, but couldn't be read back yet. It will show after the next sync.")
    })
}

/// Delete an event from an Exchange calendar (cancellations sent when
/// `notify`).
///
/// # Errors
/// `invalid` for one date of a series; network errors and refusals.
pub async fn delete_event(
    state: &AppState,
    account: &Account,
    stored: &mail_store::StoredEvent,
    occurrence: Option<i64>,
    notify: bool,
) -> AppResult<()> {
    if occurrence.is_some() {
        return Err(AppError::invalid(
            "Deleting one date of a repeating Exchange event isn't available yet. Delete the whole series, or do it in Outlook.",
        ));
    }
    let client = client(state, account)?;
    client.delete_event(&stored.href, notify).await?;
    state.store.delete_calendar_event(&stored.id)?;
    Ok(())
}

/// When people are busy, from Exchange (`None` for those it can't tell).
///
/// # Errors
/// Network errors and refusals.
pub async fn free_busy(
    state: &AppState,
    account: &Account,
    emails: &[String],
    from: i64,
    to: i64,
) -> AppResult<Vec<(String, Option<Vec<mail_core::ical::BusyPeriod>>)>> {
    let client = client(state, account)?;
    let answers = client.availability(emails, from, to).await?;
    Ok(emails
        .iter()
        .enumerate()
        .map(|(n, email)| (email.clone(), answers.get(n).cloned().flatten()))
        .collect())
}

// ---------------------------------------------------------------- contacts

/// Send contacts changed here to Exchange, then read what changed there.
///
/// # Errors
/// Network and database errors.
pub async fn sync_contacts(state: &AppState, account: &Account) -> AppResult<()> {
    let client = client(state, account)?;
    let folder =
        client.well_known().await?.contacts.ok_or_else(|| {
            AppError::internal("Exchange has no Contacts folder for this mailbox")
        })?;
    for pending in state.store.contacts_to_push(&account.id)? {
        let id = pending.contact.id.clone();
        match pending.contact.state {
            mail_store::ContactState::Deleted => {
                if let Some(href) = &pending.href {
                    client
                        .delete_items(std::slice::from_ref(href), false)
                        .await?;
                }
                state.store.forget_contact(&id)?;
            }
            mail_store::ContactState::Changed => {
                let fields = ews::convert::contact_fields(&pending.contact.card);
                let text = mail_core::vcard::write(&pending.contact.card, pending.vcard.as_deref());
                let (href, etag) = match &pending.href {
                    Some(href) => (href.clone(), client.update_contact(href, &fields).await?),
                    None => client.create_contact(&folder, &fields).await?,
                };
                state
                    .store
                    .contact_pushed(&id, &href, etag.as_deref(), &text)?;
            }
            mail_store::ContactState::Synced => {}
        }
    }
    let key = setting_key("contacts", &account.id);
    let mut token = state.store.get_setting(&key)?.filter(|t| !t.is_empty());
    let full = token.is_none();
    let mut listed = Vec::new();
    for _ in 0..MAX_PAGES {
        let page = match client
            .sync_items(&folder, token.as_deref(), OTHER_PAGE, false)
            .await
        {
            Ok(page) => page,
            Err(SyncError::Refused(code))
                if code == ews::parse::INVALID_SYNC_STATE && token.is_some() =>
            {
                state.store.set_setting(&key, "")?;
                token = None;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let mut wanted = Vec::new();
        for change in page.changes {
            match change {
                ItemChange::Created(item) | ItemChange::Updated(item) => wanted.push(item.id),
                ItemChange::Deleted(id) => state.store.forget_server_contact(&account.id, &id)?,
                ItemChange::ReadChanged { .. } => {}
            }
        }
        for contact in client.get_contacts(&wanted).await? {
            let card = ews::convert::card(&contact);
            if card.name.is_empty() && card.emails.is_empty() && card.phones.is_empty() {
                continue;
            }
            let text = mail_core::vcard::write(&card, None);
            state.store.apply_server_contact_keeping_photo(
                &account.id,
                &contact.id,
                contact.change_key.as_deref(),
                &text,
                &card,
                now(),
            )?;
            listed.push(contact.id);
        }
        token = Some(page.sync_state);
        if !full || page.done {
            state
                .store
                .set_setting(&key, token.as_deref().unwrap_or(""))?;
        }
        if page.done {
            if full {
                state
                    .store
                    .forget_contacts_not_on_server(&account.id, &listed)?;
            }
            break;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- out of office

/// An Exchange account's automatic replies, changed first when `change`.
///
/// # Errors
/// Network errors and refusals.
pub async fn out_of_office(
    state: &AppState,
    account: &Account,
    change: Option<&mail_core::vacation::Vacation>,
) -> AppResult<mail_core::vacation::Vacation> {
    let client = client(state, account)?;
    let offset = crate::connections::local_offset();
    if let Some(v) = change {
        let now = i64::try_from(now()).unwrap_or(0);
        client
            .set_oof(&account.email, &ews::convert::oof(v, offset, now))
            .await?;
    }
    let oof = client.get_oof(&account.email).await?;
    let mut settings = ews::convert::vacation(&oof, offset);
    // Exchange has no subject or "days before replying again".
    if let Some(v) = change {
        settings.subject.clone_from(&v.subject);
        settings.days = v.days;
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn folder(id: &str, parent: &str, name: &str, class: Option<&str>) -> ews::Folder {
        ews::Folder {
            id: id.into(),
            parent_id: Some(parent.into()),
            name: name.into(),
            class: class.map(str::to_owned),
            total: None,
            unread: None,
        }
    }

    #[test]
    fn mail_folders_get_paths_and_roles() {
        let known = WellKnown {
            inbox: Some("I".into()),
            sent: Some("S".into()),
            trash: Some("T".into()),
            outbox: Some("O".into()),
            root: Some("R".into()),
            ..WellKnown::default()
        };
        let folders = vec![
            folder("I", "R", "Posteingang", Some("IPF.Note")),
            folder("C", "I", "Clients/Partners", Some("IPF.Note")),
            folder("S", "R", "Sent Items", Some("IPF.Note")),
            folder("T", "R", "Deleted Items", None),
            folder("O", "R", "Outbox", Some("IPF.Note")),
            folder("A", "R", "Archive", Some("IPF.Note")),
            folder("K", "R", "Calendar", Some("IPF.Appointment")),
            folder("X", "K", "Under the calendar", Some("IPF.Note")),
            folder("Z", "elsewhere", "Search root", Some("IPF.Note")),
        ];
        let found = mail_folders(&folders, &known);
        let get = |id: &str| found.iter().find(|f| f.0 == id).cloned();
        assert_eq!(
            get("I").map(|f| (f.1, f.2, f.3)),
            Some(("Posteingang".into(), "Inbox".into(), FolderRole::Inbox))
        );
        assert_eq!(
            get("C").map(|f| f.1),
            Some("Posteingang/Clients\u{2215}Partners".into())
        );
        assert_eq!(get("S").map(|f| f.3), Some(FolderRole::Sent));
        assert_eq!(get("T").map(|f| f.3), Some(FolderRole::Trash));
        assert_eq!(get("A").map(|f| f.3), Some(FolderRole::Archive));
        for left_out in ["O", "K", "X", "Z"] {
            assert!(get(left_out).is_none(), "{left_out}");
        }
    }

    fn context() -> (Ctx, Account) {
        let state = crate::AppState::new(
            mail_store::Store::open_in_memory().unwrap(),
            Arc::new(mail_sync::secrets::MemoryStore::default()),
            std::env::temp_dir(),
        );
        let ctx = Ctx::new(
            Arc::new(state),
            Arc::new(crate::host::NoHost),
            tokio::runtime::Handle::current(),
        );
        let server = mail_store::ServerConfig {
            host: "mail.corp.example".into(),
            port: 443,
            security: mail_store::Security::Tls,
        };
        let account = ctx
            .state
            .store
            .insert_exchange_account(
                &mail_store::NewAccount {
                    email: "don@corp.example".into(),
                    display_name: "Don".into(),
                    auth: mail_store::AuthKind::Exchange,
                    color: "#2450C8".into(),
                    imap: server.clone(),
                    smtp: server,
                    size_limit_bytes: None,
                    created_at: 0,
                },
                &mail_store::EwsSettings {
                    url: "https://mail.corp.example/EWS/Exchange.asmx".into(),
                    username: None,
                },
            )
            .unwrap();
        (ctx, account)
    }

    #[tokio::test]
    async fn changes_are_queued_with_exchange_ids() {
        let (ctx, account) = context();
        let state = &ctx.state;
        let inbox = state
            .store
            .upsert_ews_folder(&account.id, "RI", "Inbox", "Inbox", FolderRole::Inbox)
            .unwrap();
        let summary = parse_summary(b"Message-ID: <m1@x>\r\nSubject: Hi\r\n\r\n").unwrap();
        let id = state
            .store
            .upsert_message(&inbox, 1, &summary, MessageFlags::default(), 10)
            .unwrap();
        state.store.set_message_remote_id(&id, "E1").unwrap();
        let read = Op::SetFlags {
            folder: "Inbox".into(),
            uids: vec![1],
            seen: Some(true),
            flagged: Some(false),
            answered: Some(true),
        };
        let ops = translate(state, &account.id, &read).unwrap();
        assert_eq!(
            ops,
            vec![
                EwsOp::Ews {
                    target: Target::Items {
                        ids: vec!["E1".into()],
                        folder: "Inbox".into()
                    },
                    change: Change::Read { read: true }
                },
                EwsOp::Ews {
                    target: Target::Items {
                        ids: vec!["E1".into()],
                        folder: "Inbox".into()
                    },
                    change: Change::Flagged { flagged: false }
                },
            ]
        );
        // Through sync::enqueue, as every command queues changes.
        sync::enqueue(
            state,
            &account.id,
            &Op::Move {
                from: "Inbox".into(),
                uids: vec![1],
                to: "Archive".into(),
                create_target: true,
            },
        )
        .unwrap();
        let queued = state.store.list_ops(&account.id).unwrap();
        assert_eq!(queued.len(), 1);
        let op: EwsOp = serde_json::from_value(queued[0].payload.clone()).unwrap();
        assert_eq!(
            op_folders(&op),
            vec!["Inbox".to_owned(), "Archive".to_owned()]
        );
        // Keywords that are categories become categories; others are left.
        let tags = Op::SetKeywordsByMessageId {
            folder: "Inbox".into(),
            message_id: "m1@x".into(),
            add: vec!["$Cat_Red_20team".into(), "$Junk".into()],
            remove: vec![],
        };
        assert_eq!(
            translate(state, &account.id, &tags).unwrap(),
            vec![EwsOp::Ews {
                target: Target::MessageId {
                    folder: "Inbox".into(),
                    message_id: "m1@x".into()
                },
                change: Change::Categories {
                    add: vec!["Red team".into()],
                    remove: vec![]
                }
            }]
        );
        let junk_only = Op::SetKeywords {
            folder: "Inbox".into(),
            uids: vec![1],
            add: vec!["$Junk".into()],
            remove: vec!["$NotJunk".into()],
        };
        assert!(
            translate(state, &account.id, &junk_only)
                .unwrap()
                .is_empty()
        );
        // UIDs it doesn't know (moved meanwhile) queue nothing.
        let unknown = Op::Delete {
            folder: "Inbox".into(),
            uids: vec![99],
        };
        assert!(translate(state, &account.id, &unknown).unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_client_is_kept_until_the_password_changes() {
        let (ctx, account) = context();
        let state = &ctx.state;
        assert_eq!(
            client(state, &account).map_err(|e| e.kind).err(),
            Some("auth")
        );
        state
            .secrets
            .set(&password_key(&account.id), "one")
            .unwrap();
        let first = client(state, &account).unwrap();
        assert_eq!(first.url(), "https://mail.corp.example/EWS/Exchange.asmx");
        state
            .secrets
            .set(&password_key(&account.id), "two")
            .unwrap();
        assert!(client(state, &account).is_ok());
        forget_client(&account.id);
    }
}
