//! The per-account sync engine.
//!
//! Each account has one task that keeps an IMAP connection, replays changes
//! made while offline, downloads new mail, applies changes made on other
//! devices, prefetches recent bodies and then waits for new mail with IDLE.
//! Errors back off and retry; a wrong password waits for the user.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mail_core::folders::{FolderRole, decode_folder_name, detect_role};
use mail_core::message::{parse_body, parse_summary};
use mail_store::{Account, Folder, MessageFlags, NewFolder, OpKind};
use mail_sync::imap::{FlagChange, ImapConnection, MAX_BACKGROUND_BODY_BYTES, uid_set};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tokio::sync::Notify;

use crate::convert::to_sync;
use crate::credentials;
use crate::error::{AppError, AppResult};
use crate::events;
use crate::state::{AppState, EngineHandle, lock, now};

/// Most messages downloaded per folder on first sync (newest first).
const INITIAL_MESSAGES: usize = 1000;
/// UIDs per header fetch.
const FETCH_CHUNK: usize = 100;
/// UIDs per flag fetch.
const FLAG_CHUNK: usize = 500;
/// Bodies prefetched per folder per pass, for offline reading and search.
const BODY_PREFETCH: u32 = 40;
/// The inbox is read most, so more of it is kept offline.
const INBOX_BODY_PREFETCH: u32 = 200;
/// How often every folder is checked, not just the inbox.
const FULL_SYNC_EVERY: Duration = Duration::from_mins(10);
/// Check interval when the server has no IDLE.
const POLL_INTERVAL: Duration = Duration::from_mins(2);
/// First retry delay after a network error; doubles up to `MAX_BACKOFF`.
const FIRST_BACKOFF: Duration = Duration::from_secs(15);
/// Longest retry delay.
const MAX_BACKOFF: Duration = Duration::from_mins(5);
/// Wait after a login failure, unless the user changes the password.
const AUTH_RETRY: Duration = Duration::from_mins(30);

/// A change made in the app, to be applied on the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Change flags.
    SetFlags {
        /// Folder path on the server.
        folder: String,
        /// Message UIDs.
        uids: Vec<u32>,
        /// New read state.
        seen: Option<bool>,
        /// New flagged state.
        flagged: Option<bool>,
        /// New answered state.
        answered: Option<bool>,
    },
    /// Move messages.
    Move {
        /// Source folder path.
        from: String,
        /// Message UIDs in the source.
        uids: Vec<u32>,
        /// Target folder path.
        to: String,
        /// Create the target first (it was made locally, e.g. Archive).
        create_target: bool,
    },
    /// Delete permanently the messages with this Message-ID (an old draft
    /// copy whose UID may not be known yet).
    DeleteByMessageId {
        /// Folder path.
        folder: String,
        /// Message-ID without angle brackets.
        message_id: String,
    },
    /// Delete permanently.
    Delete {
        /// Folder path.
        folder: String,
        /// Message UIDs.
        uids: Vec<u32>,
    },
}

/// Queue a server change for an account and wake its engine.
///
/// # Errors
/// Database errors.
pub fn enqueue(state: &AppState, account_id: &str, op: &Op) -> AppResult<()> {
    let kind = match op {
        Op::SetFlags { .. } => OpKind::SetFlags,
        Op::Move { .. } => OpKind::Move,
        Op::Delete { .. } | Op::DeleteByMessageId { .. } => OpKind::Delete,
    };
    let payload = serde_json::to_value(op).map_err(|e| AppError::internal(e.to_string()))?;
    state.store.enqueue_op(account_id, kind, &payload, now())?;
    state.wake_sync(Some(account_id));
    Ok(())
}

/// Start (or restart) the engine for an account.
pub fn start(app: &AppHandle, state: &Arc<AppState>, account_id: &str) {
    let wake = Arc::new(Notify::new());
    let task = tauri::async_runtime::spawn(run(
        app.clone(),
        Arc::clone(state),
        account_id.to_owned(),
        Arc::clone(&wake),
    ));
    if let Some(old) =
        lock(&state.engines).insert(account_id.to_owned(), EngineHandle { wake, task })
    {
        old.task.abort();
    }
}

async fn run(app: AppHandle, state: Arc<AppState>, account_id: String, wake: Arc<Notify>) {
    let mut backoff = FIRST_BACKOFF;
    loop {
        let Ok(account) = state.store.get_account(&account_id) else {
            return;
        };
        events::sync_status(&app, &account_id, "syncing", None);
        let error = match session(&app, &state, &account, &wake).await {
            Ok(()) => {
                backoff = FIRST_BACKOFF;
                continue;
            }
            Err(error) => error,
        };
        let label = if error.kind == "connect" {
            "offline"
        } else {
            "error"
        };
        events::sync_status(&app, &account_id, label, Some(&error.message));
        let wait = if matches!(error.kind, "auth" | "config") {
            AUTH_RETRY
        } else {
            backoff
        };
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            () = wake.notified() => {}
        }
        backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
    }
}

/// One connected session. Returns `Ok` to reconnect (woken, or periodic).
async fn session(
    app: &AppHandle,
    state: &AppState,
    account: &Account,
    wake: &Notify,
) -> AppResult<()> {
    let credentials = credentials::for_account(state, account).await?;
    let mut conn = ImapConnection::connect(&to_sync(&account.imap), &credentials).await?;
    let started = Instant::now();
    let mut full_done = false;
    loop {
        replay_ops(state, account, &mut conn).await?;
        if !full_done {
            refresh_folders(state, account, &mut conn).await?;
        }
        let folders = state.store.list_folders(&account.id)?;
        let mut touched = Vec::new();
        let mut new_unseen: u32 = 0;
        for folder in folders
            .iter()
            .filter(|f| should_sync(f) && (!full_done || f.role == FolderRole::Inbox))
        {
            let report = sync_folder(state, &mut conn, folder).await?;
            if report.changed {
                touched.push(folder.id.clone());
            }
            new_unseen = new_unseen.saturating_add(report.new_unseen);
        }
        full_done = true;
        if !touched.is_empty() {
            events::changed(app, &account.id, &touched);
        }
        if new_unseen > 0 {
            events::new_mail(app, state, account, new_unseen);
        }
        events::sync_status(app, &account.id, "idle", None);

        let remaining = FULL_SYNC_EVERY.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Ok(());
        }
        let inbox = folders
            .iter()
            .find(|f| f.role == FolderRole::Inbox)
            .map(|f| f.path.clone());
        match inbox {
            Some(path) if conn.supports_idle() => {
                tokio::select! {
                    result = conn.idle(&path) => conn = result?.0,
                    () = wake.notified() => return Ok(()),
                    () = tokio::time::sleep(remaining) => return Ok(()),
                }
            }
            _ => {
                tokio::select! {
                    () = tokio::time::sleep(POLL_INTERVAL.min(remaining)) => {}
                    () = wake.notified() => return Ok(()),
                }
            }
        }
    }
}

const fn should_sync(folder: &Folder) -> bool {
    !matches!(folder.role, FolderRole::All | FolderRole::Flagged)
}

async fn apply(conn: &mut ImapConnection, op: &Op) -> Result<(), mail_sync::SyncError> {
    match op {
        Op::SetFlags {
            folder,
            uids,
            seen,
            flagged,
            answered,
        } => {
            let changes = [
                seen.map(FlagChange::Seen),
                flagged.map(FlagChange::Flagged),
                answered.map(FlagChange::Answered),
            ];
            for change in changes.into_iter().flatten() {
                conn.store_flags(folder, uids, change).await?;
            }
            Ok(())
        }
        Op::Move {
            from,
            uids,
            to,
            create_target,
        } => {
            if *create_target {
                // Fails harmlessly when the folder already exists.
                let _ = conn.create_folder(to).await;
            }
            conn.move_messages(from, uids, to).await
        }
        Op::Delete { folder, uids } => conn.delete_forever(folder, uids).await,
        Op::DeleteByMessageId { folder, message_id } => {
            let uids = conn.uids_with_message_id(folder, message_id).await?;
            conn.delete_forever(folder, &uids).await
        }
    }
}

async fn replay_ops(
    state: &AppState,
    account: &Account,
    conn: &mut ImapConnection,
) -> AppResult<()> {
    for pending in state.store.list_ops(&account.id)? {
        let result = match serde_json::from_value::<Op>(pending.payload.clone()) {
            Ok(op) => apply(conn, &op).await,
            Err(_) => Ok(()),
        };
        match result {
            Ok(()) => {}
            Err(error) if error.is_transient() => return Err(error.into()),
            Err(error) => {
                let refusals = state.store.record_op_refusal(&pending.id)?;
                if !give_up_after(refusals) {
                    // Stop here, keeping the order of changes, and let the
                    // session back off and try again: some servers refuse
                    // briefly ("mailbox in use") without saying it's temporary.
                    return Err(AppError::internal(format!(
                        "The mail server refused a change ({error}). Trying again shortly."
                    )));
                }
                eprintln!("giving up on a change the server refused {refusals} times: {error}");
            }
        }
        state.store.delete_op(&pending.id)?;
    }
    Ok(())
}

/// How many times the server may refuse one queued change before it is
/// given up. Giving up is safe: the next sync shows the server's real state.
const MAX_OP_REFUSALS: u32 = 3;

const fn give_up_after(refusals: u32) -> bool {
    refusals >= MAX_OP_REFUSALS
}

fn leaf_name(path: &str, delimiter: Option<&str>) -> String {
    let leaf = delimiter
        .filter(|d| !d.is_empty())
        .and_then(|d| path.rsplit(d).next())
        .unwrap_or(path);
    decode_folder_name(leaf)
}

async fn refresh_folders(
    state: &AppState,
    account: &Account,
    conn: &mut ImapConnection,
) -> AppResult<()> {
    let mut paths = Vec::new();
    for folder in conn.list_folders().await?.iter().filter(|f| f.selectable) {
        let attributes: Vec<&str> = folder.attributes.iter().map(String::as_str).collect();
        // Gmail's "Important" is a virtual view of other folders.
        if attributes
            .iter()
            .any(|a| a.eq_ignore_ascii_case("\\Important"))
        {
            continue;
        }
        let role = detect_role(&folder.path, &attributes, folder.delimiter.as_deref());
        let name = if role == FolderRole::Inbox {
            "Inbox".to_owned()
        } else {
            leaf_name(&folder.path, folder.delimiter.as_deref())
        };
        state.store.upsert_folder(
            &account.id,
            &NewFolder {
                path: &folder.path,
                name: &name,
                role,
                delimiter: folder.delimiter.as_deref(),
            },
        )?;
        paths.push(folder.path.clone());
    }
    state.store.delete_folders_not_in(&account.id, &paths)?;
    Ok(())
}

struct Report {
    changed: bool,
    new_unseen: u32,
}

const fn flags(f: mail_sync::imap::Flags) -> MessageFlags {
    MessageFlags {
        seen: f.seen,
        flagged: f.flagged,
        answered: f.answered,
    }
}

/// Open a folder, or `None` when the server refuses it (removed meanwhile,
/// no permission): one such folder must not stop the rest of the account.
async fn select_or_skip(
    conn: &mut ImapConnection,
    folder: &Folder,
) -> AppResult<Option<mail_sync::imap::FolderStatus>> {
    match conn.select(&folder.path).await {
        Ok(status) => Ok(Some(status)),
        Err(mail_sync::SyncError::Refused(reason)) => {
            eprintln!("skipping a folder the server won't open: {reason}");
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

async fn sync_folder(
    state: &AppState,
    conn: &mut ImapConnection,
    folder: &Folder,
) -> AppResult<Report> {
    let Some(status) = select_or_skip(conn, folder).await? else {
        return Ok(Report {
            changed: false,
            new_unseen: 0,
        });
    };
    let mut changed = state
        .store
        .set_uidvalidity(&folder.id, status.uid_validity)?;
    let highest = state.store.get_folder(&folder.id)?.highest_uid.unwrap_or(0);
    let initial = highest == 0;
    let server: HashSet<u32> = if status.exists == 0 {
        HashSet::new()
    } else {
        conn.all_uids(&folder.path).await?
    };

    // With no local changes waiting, anything recent on the server that is
    // missing here is a gap (a move the server refused) and is fetched again.
    let settled = state.store.list_ops(&folder.account_id)?.is_empty();
    let local_before: HashSet<u32> = if settled {
        state
            .store
            .uids_in_folder(&folder.id)?
            .into_iter()
            .collect()
    } else {
        HashSet::new()
    };
    let fresh = uids_to_fetch(&server, &local_before, highest, settled);
    let mut new_unseen: u32 = 0;
    for chunk in fresh.chunks(FETCH_CHUNK) {
        let Some(set) = uid_set(chunk) else { continue };
        for message in conn.fetch_headers(&folder.path, &set).await? {
            state.store.upsert_message(
                &folder.id,
                message.uid,
                &message.summary,
                flags(message.flags),
                u64::from(message.size),
            )?;
            if !initial
                && message.uid > highest
                && !message.flags.seen
                && folder.role == FolderRole::Inbox
            {
                new_unseen = new_unseen.saturating_add(1);
            }
            changed = true;
        }
    }
    if let Some(&max) = fresh.last().filter(|&&max| max > highest) {
        state.store.set_highest_uid(&folder.id, max)?;
    }
    if settled && state.store.delete_unplaced_moves(&folder.id)? > 0 {
        changed = true;
    }

    let local = state.store.uids_in_folder(&folder.id)?;
    let gone: Vec<u32> = local
        .iter()
        .copied()
        .filter(|uid| !server.contains(uid))
        .collect();
    if !gone.is_empty() {
        state.store.delete_by_uids(&folder.id, &gone)?;
        changed = true;
    }
    let known: Vec<u32> = local
        .iter()
        .copied()
        .filter(|uid| server.contains(uid) && *uid <= highest)
        .collect();
    for chunk in known.chunks(FLAG_CHUNK) {
        let Some(set) = uid_set(chunk) else { continue };
        for (uid, server_flags) in conn.fetch_flags(&folder.path, &set).await? {
            changed |= state
                .store
                .set_flags_by_uid(&folder.id, uid, flags(server_flags))?;
        }
    }

    changed |= prefetch_bodies(state, conn, folder).await?;
    Ok(Report {
        changed,
        new_unseen,
    })
}

/// UIDs to download, ascending: those above `highest` (new since the last
/// sync) and, when `fill_gaps`, any not in `local`, both limited to the
/// newest `INITIAL_MESSAGES` on the server.
fn uids_to_fetch(
    server: &HashSet<u32>,
    local: &HashSet<u32>,
    highest: u32,
    fill_gaps: bool,
) -> Vec<u32> {
    let mut newest: Vec<u32> = server.iter().copied().collect();
    newest.sort_unstable();
    let skip = newest.len().saturating_sub(INITIAL_MESSAGES);
    newest
        .into_iter()
        .skip(skip)
        .filter(|uid| *uid > highest || (fill_gaps && !local.contains(uid)))
        .collect()
}

/// Download recent small bodies so they open offline. Returns whether any did.
async fn prefetch_bodies(
    state: &AppState,
    conn: &mut ImapConnection,
    folder: &Folder,
) -> AppResult<bool> {
    let mut fetched = false;
    let prefetch = if folder.role == FolderRole::Inbox {
        INBOX_BODY_PREFETCH
    } else {
        BODY_PREFETCH
    };
    for (id, uid) in state.store.messages_missing_raw(
        &folder.id,
        prefetch,
        u64::from(MAX_BACKGROUND_BODY_BYTES),
    )? {
        if let Some(raw) = conn
            .fetch_raw(&folder.path, uid, u64::from(MAX_BACKGROUND_BODY_BYTES))
            .await?
        {
            cache_body(state, &id, &raw)?;
            fetched = true;
        }
    }
    Ok(fetched)
}

/// Store a downloaded body with its searchable text and list preview.
///
/// # Errors
/// Database errors.
pub fn cache_body(state: &AppState, id: &str, raw: &[u8]) -> AppResult<()> {
    let preview = parse_summary(raw).map(|s| s.preview).unwrap_or_default();
    let text = parse_body(raw)
        .ok()
        .and_then(|b| b.text)
        .unwrap_or_else(|| preview.clone());
    state.store.set_body(id, raw, Some(&text), &preview)?;
    Ok(())
}

/// The full message, downloading it now if it isn't cached.
///
/// # Errors
/// `connect` when offline, `not_found` when it was deleted on the server.
pub async fn body_now(state: &AppState, id: &str) -> AppResult<Vec<u8>> {
    if let Some(raw) = state.store.get_raw(id)? {
        return Ok(raw);
    }
    let message = state.store.get_message(id)?;
    let uid = message.uid.ok_or_else(|| {
        AppError::new(
            "connect",
            "This message is still being moved on the server. Try again in a moment.",
        )
    })?;
    let folder = state.store.get_folder(&message.item.folder_id)?;
    let account = state.store.get_account(&message.item.account_id)?;
    let credentials = credentials::for_account(state, &account).await?;
    let mut conn = ImapConnection::connect(&to_sync(&account.imap), &credentials).await?;
    let raw = conn
        .fetch_raw(&folder.path, uid, message.size)
        .await?
        .ok_or_else(|| AppError::new("not_found", "This message was deleted on the server."))?;
    conn.logout().await;
    cache_body(state, id, &raw)?;
    Ok(raw)
}

/// Messages grouped by (account id, folder id), each with its server UID.
pub type FolderGroups = HashMap<(String, String), Vec<(String, Option<u32>)>>;

/// Group message ids by account and folder, with their server UIDs.
///
/// # Errors
/// Unknown ids.
pub fn group_by_folder(state: &AppState, ids: &[String]) -> AppResult<FolderGroups> {
    let mut groups = FolderGroups::new();
    for id in ids {
        let message = state.store.get_message(id)?;
        groups
            .entry((
                message.item.account_id.clone(),
                message.item.folder_id.clone(),
            ))
            .or_default()
            .push((id.clone(), message.uid));
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_round_trip_as_json() {
        let op = Op::Move {
            from: "INBOX".into(),
            uids: vec![1, 2],
            to: "Archive".into(),
            create_target: true,
        };
        let value = serde_json::to_value(&op).unwrap_or_default();
        assert_eq!(value["op"], "move");
        assert_eq!(serde_json::from_value::<Op>(value).ok(), Some(op));
    }

    #[test]
    fn a_refused_change_gets_more_than_one_chance() {
        assert!(!give_up_after(1));
        assert!(!give_up_after(MAX_OP_REFUSALS - 1));
        assert!(give_up_after(MAX_OP_REFUSALS));
    }

    #[test]
    fn fetches_new_mail_and_refills_recent_gaps() {
        let server: HashSet<u32> = (1..=10).collect();
        let local: HashSet<u32> = [1, 2, 3, 5, 6, 7, 8].into_iter().collect();
        // Changes still waiting: only what is new.
        assert_eq!(uids_to_fetch(&server, &local, 8, false), vec![9, 10]);
        // Settled: the message missing at UID 4 comes back too.
        assert_eq!(uids_to_fetch(&server, &local, 8, true), vec![4, 9, 10]);
        // First sync of a huge folder: only the newest INITIAL_MESSAGES.
        let big: HashSet<u32> = (1..=5000).collect();
        let first = uids_to_fetch(&big, &HashSet::new(), 0, true);
        assert_eq!(first.len(), INITIAL_MESSAGES);
        assert_eq!(first.first(), Some(&4001));
        // Older mail that was never downloaded is not treated as a gap.
        let local: HashSet<u32> = (4001..=5000).collect();
        assert!(uids_to_fetch(&big, &local, 5000, true).is_empty());
    }

    #[test]
    fn folder_display_names() {
        assert_eq!(leaf_name("INBOX.Entw&APw-rfe", Some(".")), "Entwürfe");
        assert_eq!(leaf_name("[Gmail]/Sent Mail", Some("/")), "Sent Mail");
        assert_eq!(leaf_name("Clients", None), "Clients");
    }
}
