//! The per-account sync engine.
//!
//! Each account has one task that keeps an IMAP connection, replays changes
//! made while offline, downloads new mail, applies changes made on other
//! devices, prefetches recent bodies and then waits for new mail with IDLE.
//! Errors back off and retry; a wrong password waits for the user.

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::FutureExt as _;

use mail_core::folders::{FolderRole, decode_folder_name, detect_role_unclaimed, special_use_role};
use mail_core::message::{parse_body, parse_summary};
use mail_core::sync_window::imap_since;
use mail_store::{Account, Folder, MessageFlags, NewFolder, OpKind};
use mail_sync::imap::{
    FlagChange, IdleEnd, ImapConnection, MAX_BACKGROUND_BODY_BYTES, Tags, uid_set,
};
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
/// With a sync window (`Account::sync_months`), every body inside it is
/// downloaded instead of the counts above, this many per folder per pass so
/// new mail isn't held up behind a long first download.
const WINDOW_BODY_BATCH: u32 = 500;
/// How often every folder is checked, not just the inbox.
const FULL_SYNC_EVERY: Duration = Duration::from_mins(10);
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
    /// Change flags on the messages with this Message-ID: ones moved
    /// locally whose new UID isn't known yet.
    SetFlagsByMessageId {
        /// Folder path on the server.
        folder: String,
        /// Message-ID without angle brackets.
        message_id: String,
        /// New read state.
        seen: Option<bool>,
        /// New flagged state.
        flagged: Option<bool>,
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
    /// Move the messages with this Message-ID: one moved locally whose new
    /// UID isn't known yet (moved again before the next sync, or Undo).
    MoveByMessageId {
        /// Source folder path.
        from: String,
        /// Message-ID without angle brackets.
        message_id: String,
        /// Target folder path.
        to: String,
        /// Create the target first.
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
    /// Copy messages to another folder; the originals stay. The copies
    /// arrive as new messages when the target is synced.
    Copy {
        /// Source folder path.
        from: String,
        /// Message UIDs in the source.
        uids: Vec<u32>,
        /// Target folder path.
        to: String,
        /// Create the target first (it was made locally, e.g. Archive).
        #[serde(default)]
        create_target: bool,
    },
    /// Copy the messages with this Message-ID (moved here locally, new UID
    /// not known yet).
    CopyByMessageId {
        /// Source folder path.
        from: String,
        /// Message-ID without angle brackets.
        message_id: String,
        /// Target folder path.
        to: String,
        /// Create the target first.
        #[serde(default)]
        create_target: bool,
    },
    /// Add and remove IMAP keywords (`$Junk`, `$NotJunk`, category
    /// keywords). Skipped in folders that don't keep keywords.
    SetKeywords {
        /// Folder path.
        folder: String,
        /// Message UIDs.
        uids: Vec<u32>,
        /// Keywords to add.
        add: Vec<String>,
        /// Keywords to remove.
        remove: Vec<String>,
    },
    /// [`Op::SetKeywords`] on the messages with this Message-ID.
    SetKeywordsByMessageId {
        /// Folder path.
        folder: String,
        /// Message-ID without angle brackets.
        message_id: String,
        /// Keywords to add.
        add: Vec<String>,
        /// Keywords to remove.
        remove: Vec<String>,
    },
    /// Add and remove Gmail labels: the categories of a Gmail account, by
    /// name (encoded when sent).
    SetLabels {
        /// Folder path.
        folder: String,
        /// Message UIDs.
        uids: Vec<u32>,
        /// Labels to add.
        add: Vec<String>,
        /// Labels to remove.
        remove: Vec<String>,
    },
    /// [`Op::SetLabels`] on the messages with this Message-ID.
    SetLabelsByMessageId {
        /// Folder path.
        folder: String,
        /// Message-ID without angle brackets.
        message_id: String,
        /// Labels to add.
        add: Vec<String>,
        /// Labels to remove.
        remove: Vec<String>,
    },
}

/// What the sync status says while Work Offline is on.
pub const WORKING_OFFLINE: &str = "Working offline. Turn off Work Offline to sync and send.";

/// Queue a server change for an account and wake its engine.
///
/// # Errors
/// Database errors.
pub fn enqueue(state: &AppState, account_id: &str, op: &Op) -> AppResult<()> {
    let kind = match op {
        Op::SetFlags { .. }
        | Op::SetFlagsByMessageId { .. }
        | Op::SetKeywords { .. }
        | Op::SetKeywordsByMessageId { .. }
        | Op::SetLabels { .. }
        | Op::SetLabelsByMessageId { .. } => OpKind::SetFlags,
        Op::Move { .. } | Op::MoveByMessageId { .. } => OpKind::Move,
        Op::Delete { .. } | Op::DeleteByMessageId { .. } => OpKind::Delete,
        Op::Copy { .. } | Op::CopyByMessageId { .. } => OpKind::Copy,
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
    // Whether a rejected sign-in token has already been swapped for a new
    // one since the last session that worked.
    let mut fresh_token_tried = false;
    loop {
        // Work Offline: no connection at all until it is switched off
        // (which wakes every engine). Changes made meanwhile wait in the
        // queue as they do without a network.
        if crate::settings::working_offline(&state) {
            events::sync_status(&app, &account_id, "offline", Some(WORKING_OFFLINE));
            wake.notified().await;
            continue;
        }
        let account = match state.store.get_account(&account_id) {
            Ok(account) => account,
            // The account was removed: this engine is done.
            Err(mail_store::StoreError::NotFound(_)) => return,
            // A database hiccup must not end this account's mail for good.
            Err(error) => {
                eprintln!("mail sync couldn't read its account; trying again: {error}");
                tokio::time::sleep(backoff).await;
                backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
                continue;
            }
        };
        events::sync_status(&app, &account_id, "syncing", None);
        // A bug must not stop this account's mail for good: a session that
        // panics is reported like any other error and retried.
        let attempt = AssertUnwindSafe(session(&app, &state, &account, &wake)).catch_unwind();
        let error = match attempt.await {
            Ok(Ok(())) => {
                backoff = FIRST_BACKOFF;
                fresh_token_tried = false;
                continue;
            }
            Ok(Err(error)) => error,
            Err(_) => {
                eprintln!("mail sync stopped unexpectedly; restarting it");
                AppError::internal("Mail sync stopped unexpectedly. Restarting it.")
            }
        };
        // A cached sign-in token the server no longer accepts is thrown
        // away and a new one fetched at once, instead of offering the same
        // token again after the long wait below.
        if error.kind == "auth"
            && !fresh_token_tried
            && lock(&state.access_tokens).remove(&account_id).is_some()
        {
            fresh_token_tried = true;
            continue;
        }
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

/// What started a sync pass within one connected session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trigger {
    /// Just logged in.
    Connected,
    /// IDLE or the poll timer ended on its own (new mail, or time to look).
    Waited,
    /// The engine was woken: changes queued in the app, or "check now".
    Woken,
}

/// Which folders a pass syncs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Re-list the folders and sync every one.
    All,
    /// The inbox, plus folders the replayed changes touched.
    Inbox,
    /// The inbox, Sent and Drafts (sending and saving drafts wake the
    /// engine), plus folders the replayed changes touched.
    Changes,
}

/// A wake with changes to send syncs only what they touched. A wake with
/// none is "check now" (or new sign-in details): everything is checked, as
/// a fresh login would, but on the same connection.
const fn scope_of(trigger: Trigger, replayed_any: bool) -> Scope {
    match trigger {
        Trigger::Waited => Scope::Inbox,
        Trigger::Woken if replayed_any => Scope::Changes,
        Trigger::Connected | Trigger::Woken => Scope::All,
    }
}

/// Whether a pass with this scope syncs a folder with this role; `touched`
/// says whether a change replayed in this pass was in it.
const fn folder_due(scope: Scope, role: FolderRole, touched: bool) -> bool {
    // Virtual views of other folders (Gmail's All Mail, Starred).
    if matches!(role, FolderRole::All | FolderRole::Flagged) {
        return false;
    }
    match scope {
        Scope::All => true,
        Scope::Inbox => touched || matches!(role, FolderRole::Inbox),
        Scope::Changes => {
            touched
                || matches!(
                    role,
                    FolderRole::Inbox | FolderRole::Sent | FolderRole::Drafts
                )
        }
    }
}

/// Folders a queued change reads or writes on the server.
fn op_folders(op: &Op) -> [Option<&str>; 2] {
    match op {
        Op::SetFlags { folder, .. }
        | Op::SetFlagsByMessageId { folder, .. }
        | Op::Delete { folder, .. }
        | Op::DeleteByMessageId { folder, .. }
        | Op::SetKeywords { folder, .. }
        | Op::SetKeywordsByMessageId { folder, .. }
        | Op::SetLabels { folder, .. }
        | Op::SetLabelsByMessageId { folder, .. } => [Some(folder), None],
        Op::Move { from, to, .. }
        | Op::MoveByMessageId { from, to, .. }
        | Op::Copy { from, to, .. }
        | Op::CopyByMessageId { from, to, .. } => [Some(from), Some(to)],
    }
}

/// How a wait between passes ended.
enum Waited {
    /// Time for the next pass on the same connection.
    Pass(Box<ImapConnection>, Trigger),
    /// Log in again: the periodic full sync is due, or the connection
    /// broke while being taken back for a wake.
    Reconnect,
}

/// One connected session. Returns `Ok` to reconnect (periodic full sync, or
/// a connection found broken when woken).
///
/// A wake (changes queued in the app) during IDLE ends IDLE with DONE and
/// keeps the same logged-in connection: the changes are sent and only the
/// folders they touched are synced, then IDLE resumes. Logging in again for
/// every change would trip servers' login limits.
async fn session(
    app: &AppHandle,
    state: &AppState,
    account: &Account,
    wake: &Notify,
) -> AppResult<()> {
    let credentials = credentials::for_account(state, account).await?;
    let mut conn = ImapConnection::connect(&to_sync(&account.imap), &credentials).await?;
    let started = Instant::now();
    let mut trigger = Trigger::Connected;
    loop {
        // Work Offline was switched on (it wakes the engine): hang up.
        if crate::settings::working_offline(state) {
            conn.logout().await;
            return Ok(());
        }
        // Anything that goes wrong here is reported after the pass, not
        // instead of it: new mail must still arrive. Only a lost connection
        // ends the pass early.
        let mut problem: Option<AppError> = None;
        let mut touched = HashSet::new();
        if let Err(error) = replay_ops(state, account, &mut conn, &mut touched).await {
            stop_if_disconnected(error, &mut problem)?;
        }
        let scope = scope_of(trigger, !touched.is_empty());
        if scope == Scope::All
            && let Err(error) = refresh_folders(state, account, &mut conn).await
        {
            stop_if_disconnected(error, &mut problem)?;
        }
        let folders = state.store.list_folders(&account.id)?;
        let mut new_unseen: u32 = 0;
        // Inbox first, and the interface hears about each folder as soon as
        // it is done: a first sync of a big mailbox takes minutes, and the
        // inbox shouldn't stay empty while Sent and Trash download.
        let mut due: Vec<_> = folders
            .iter()
            .filter(|f| folder_due(scope, f.role, touched.contains(&f.path)))
            .collect();
        due.sort_by_key(|f| f.role != FolderRole::Inbox);
        for folder in due {
            let report = match sync_folder(state, account, &mut conn, folder).await {
                Ok(report) => report,
                Err(error) => {
                    // One folder the server or the database chokes on must
                    // not hold up the others; the inbox goes first anyway.
                    stop_if_disconnected(error, &mut problem)?;
                    continue;
                }
            };
            if report.changed {
                events::changed(app, &account.id, std::slice::from_ref(&folder.id));
            }
            new_unseen = new_unseen.saturating_add(report.new_unseen);
        }
        if scope == Scope::All {
            // Once per inbox: mark old mail that is newsletters or
            // notifications, for the Focused Inbox. Never holds up mail.
            for inbox in folders.iter().filter(|f| f.role == FolderRole::Inbox) {
                if let Err(error) = mark_bulk_once(state, &mut conn, inbox).await {
                    stop_if_disconnected(error, &mut problem)?;
                }
            }
        }
        if new_unseen > 0 {
            events::new_mail(app, state, account, new_unseen);
        }
        if let Some(error) = problem {
            // Shown as the account's status; the session backs off and the
            // next pass tries everything again.
            return Err(error);
        }
        events::sync_status(app, &account.id, "idle", None);

        let inbox = folders
            .iter()
            .find(|f| f.role == FolderRole::Inbox)
            .map(|f| f.path.clone());
        match wait_for_change(state, conn, inbox.as_deref(), started, wake).await? {
            Waited::Pass(same, next) => {
                conn = *same;
                trigger = next;
                if trigger == Trigger::Woken {
                    events::sync_status(app, &account.id, "syncing", None);
                }
            }
            Waited::Reconnect => return Ok(()),
        }
    }
}

/// Wait for new mail (IDLE on the inbox, or the poll timer on servers
/// without IDLE), a wake, or the periodic full sync, whichever comes first.
async fn wait_for_change(
    state: &AppState,
    mut conn: ImapConnection,
    inbox: Option<&str>,
    started: Instant,
    wake: &Notify,
) -> AppResult<Waited> {
    // Settings > Check for new mail every (servers without IDLE).
    let poll = Duration::from_secs(
        u64::from(crate::settings::load(state).map_or(2, |s| s.check_every_mins))
            .saturating_mul(60),
    );
    // A wake that came during the pass left its permit in the Notify: the
    // changes behind it are sent now instead of after an IDLE round trip.
    // Taken first, so a full sync that is due anyway also answers it.
    let woken = wake.notified().now_or_never().is_some();
    let remaining = FULL_SYNC_EVERY.max(poll).saturating_sub(started.elapsed());
    if remaining.is_zero() {
        return Ok(Waited::Reconnect);
    }
    if woken {
        return Ok(Waited::Pass(Box::new(conn), Trigger::Woken));
    }
    match inbox {
        Some(path) if conn.supports_idle() => {
            let why = std::sync::OnceLock::new();
            let stop = async {
                let reason = tokio::select! {
                    () = wake.notified() => Trigger::Woken,
                    // The periodic full sync is due.
                    () = tokio::time::sleep(remaining) => Trigger::Connected,
                };
                let _ = why.set(reason);
            };
            match conn.idle_until(path, stop).await {
                Ok((conn, IdleEnd::Stopped)) if why.get() == Some(&Trigger::Woken) => {
                    Ok(Waited::Pass(Box::new(conn), Trigger::Woken))
                }
                Ok((_, IdleEnd::Stopped)) => Ok(Waited::Reconnect),
                Ok((conn, IdleEnd::Changed | IdleEnd::Quiet)) => {
                    Ok(Waited::Pass(Box::new(conn), Trigger::Waited))
                }
                // Ending IDLE for a wake or the full sync found the
                // connection broken: log in again at once, as before.
                Err(error) if why.get().is_some() => {
                    eprintln!("mail sync: reconnecting after IDLE ended badly: {error}");
                    Ok(Waited::Reconnect)
                }
                Err(error) => Err(error.into()),
            }
        }
        _ => {
            tokio::select! {
                () = tokio::time::sleep(poll.min(remaining)) => {
                    Ok(Waited::Pass(Box::new(conn), Trigger::Waited))
                }
                () = wake.notified() => {
                    // The connection sat quiet: make sure it still works
                    // before sending changes on it, or log in again.
                    match conn.noop().await {
                        Ok(()) => Ok(Waited::Pass(Box::new(conn), Trigger::Woken)),
                        Err(error) => {
                            eprintln!("mail sync: reconnecting, connection went quiet: {error}");
                            Ok(Waited::Reconnect)
                        }
                    }
                }
            }
        }
    }
}

/// A lost connection (or a refused login) ends the pass; anything else is
/// remembered, the first one reported after the pass.
fn stop_if_disconnected(error: AppError, problem: &mut Option<AppError>) -> AppResult<()> {
    if ends_session(error.kind) {
        return Err(error);
    }
    eprintln!("mail sync: {}", error.message);
    problem.get_or_insert(error);
    Ok(())
}

/// Errors after which the connection can't be used for anything else.
fn ends_session(kind: &str) -> bool {
    matches!(kind, "connect" | "tls" | "auth" | "config" | "keyring")
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
        Op::SetFlagsByMessageId {
            folder,
            message_id,
            seen,
            flagged,
        } => {
            let uids = conn.uids_with_message_id(folder, message_id).await?;
            if uids.is_empty() {
                // Moved on or deleted since: nothing left to change.
                return Ok(());
            }
            let changes = [seen.map(FlagChange::Seen), flagged.map(FlagChange::Flagged)];
            for change in changes.into_iter().flatten() {
                conn.store_flags(folder, &uids, change).await?;
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
        Op::MoveByMessageId {
            from,
            message_id,
            to,
            create_target,
        } => {
            let uids = conn.uids_with_message_id(from, message_id).await?;
            if uids.is_empty() {
                // Already moved (or gone): nothing left to do.
                return Ok(());
            }
            if *create_target {
                let _ = conn.create_folder(to).await;
            }
            conn.move_messages(from, &uids, to).await
        }
        Op::Delete { folder, uids } => conn.delete_forever(folder, uids).await,
        Op::DeleteByMessageId { folder, message_id } => {
            let uids = conn.uids_with_message_id(folder, message_id).await?;
            conn.delete_forever(folder, &uids).await
        }
        Op::Copy { .. }
        | Op::CopyByMessageId { .. }
        | Op::SetKeywords { .. }
        | Op::SetKeywordsByMessageId { .. }
        | Op::SetLabels { .. }
        | Op::SetLabelsByMessageId { .. } => apply_copy_or_tags(conn, op).await,
    }
}

/// Copies, keywords and labels (the 0.5.0 changes).
async fn apply_copy_or_tags(
    conn: &mut ImapConnection,
    op: &Op,
) -> Result<(), mail_sync::SyncError> {
    match op {
        Op::Copy {
            from,
            uids,
            to,
            create_target,
        } => {
            if *create_target {
                // Fails harmlessly when the folder already exists.
                let _ = conn.create_folder(to).await;
            }
            conn.copy_messages(from, uids, to).await
        }
        Op::CopyByMessageId {
            from,
            message_id,
            to,
            create_target,
        } => {
            let uids = conn.uids_with_message_id(from, message_id).await?;
            if uids.is_empty() {
                return Ok(());
            }
            if *create_target {
                let _ = conn.create_folder(to).await;
            }
            conn.copy_messages(from, &uids, to).await
        }
        Op::SetKeywords {
            folder,
            uids,
            add,
            remove,
        } => store_keywords(conn, folder, uids, add, remove).await,
        Op::SetKeywordsByMessageId {
            folder,
            message_id,
            add,
            remove,
        } => {
            let uids = conn.uids_with_message_id(folder, message_id).await?;
            store_keywords(conn, folder, &uids, add, remove).await
        }
        Op::SetLabels {
            folder,
            uids,
            add,
            remove,
        } => {
            conn.store_labels(folder, uids, &labels(add), &labels(remove))
                .await
        }
        Op::SetLabelsByMessageId {
            folder,
            message_id,
            add,
            remove,
        } => {
            let uids = conn.uids_with_message_id(folder, message_id).await?;
            conn.store_labels(folder, &uids, &labels(add), &labels(remove))
                .await
        }
        // Handled by `apply`.
        Op::SetFlags { .. }
        | Op::SetFlagsByMessageId { .. }
        | Op::Move { .. }
        | Op::MoveByMessageId { .. }
        | Op::Delete { .. }
        | Op::DeleteByMessageId { .. } => Ok(()),
    }
}

/// Gmail labels for category names.
fn labels(names: &[String]) -> Vec<String> {
    names
        .iter()
        .map(|name| mail_core::categories::label_for(name))
        .collect()
}

/// Keywords are kept only where the folder allows them; elsewhere the
/// change stays on this device, which is not an error.
async fn store_keywords(
    conn: &mut ImapConnection,
    folder: &str,
    uids: &[u32],
    add: &[String],
    remove: &[String],
) -> Result<(), mail_sync::SyncError> {
    if !conn.store_keywords(folder, uids, add, remove).await? {
        eprintln!("mail sync: this folder doesn't keep keywords; categories stay on this device");
    }
    Ok(())
}

/// Send the changes queued in the app, oldest first. The folders they touch
/// are added to `touched`, so the pass after can check the result.
async fn replay_ops(
    state: &AppState,
    account: &Account,
    conn: &mut ImapConnection,
    touched: &mut HashSet<String>,
) -> AppResult<()> {
    for pending in state.store.list_ops(&account.id)? {
        let result = match serde_json::from_value::<Op>(pending.payload.clone()) {
            Ok(op) => {
                touched.extend(op_folders(&op).into_iter().flatten().map(str::to_owned));
                apply(conn, &op).await
            }
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

/// Headers fetched per round when marking old mail as bulk.
const BULK_BATCH: usize = 200;

/// Record which already-synced messages in a folder are bulk mail, once.
async fn mark_bulk_once(
    state: &AppState,
    conn: &mut ImapConnection,
    folder: &Folder,
) -> AppResult<()> {
    let key = format!("bulk_checked:{}", folder.id);
    if state.store.get_setting(&key)?.is_some() {
        return Ok(());
    }
    let uids = state.store.uids_in_folder(&folder.id)?;
    for batch in uids.chunks(BULK_BATCH) {
        let set = batch
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let marks = conn.fetch_bulk_marks(&folder.path, &set).await?;
        state.store.set_bulk_by_uid(&folder.id, &marks)?;
    }
    state.store.set_setting(&key, "1")?;
    Ok(())
}

pub(crate) async fn refresh_folders(
    state: &AppState,
    account: &Account,
    conn: &mut ImapConnection,
) -> AppResult<()> {
    let mut paths = Vec::new();
    let listed = conn.list_folders().await?;
    // Roles the server names with special-use flags. A folder that merely
    // has the same name (Clients/Sent) must not claim one of these.
    let claimed: Vec<FolderRole> = listed
        .iter()
        .filter_map(|f| {
            let attributes: Vec<&str> = f.attributes.iter().map(String::as_str).collect();
            special_use_role(&attributes)
        })
        .collect();
    for folder in listed.iter().filter(|f| f.selectable) {
        let attributes: Vec<&str> = folder.attributes.iter().map(String::as_str).collect();
        // Gmail's "Important" is a virtual view of other folders.
        if attributes
            .iter()
            .any(|a| a.eq_ignore_ascii_case("\\Important"))
        {
            continue;
        }
        let role = detect_role_unclaimed(
            &folder.path,
            &attributes,
            folder.delimiter.as_deref(),
            &claimed,
        );
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

/// The categories a message has on the server: its Gmail labels, or its
/// category keywords where the folder keeps keywords. `None` when the
/// server can't hold them, so the ones set here are left alone.
fn server_categories(tags: &Tags, keywords_kept: bool) -> Option<Vec<String>> {
    if let Some(labels) = &tags.labels {
        return Some(
            labels
                .iter()
                .filter_map(|label| mail_core::categories::name_from_label(label))
                .collect(),
        );
    }
    keywords_kept.then(|| {
        tags.keywords
            .iter()
            .filter_map(|keyword| mail_core::categories::name_from_keyword(keyword))
            .collect()
    })
}

/// Record a message's categories as the server has them. Returns whether
/// anything changed.
fn apply_categories(
    state: &AppState,
    folder: &Folder,
    uid: u32,
    tags: &Tags,
    keywords_kept: bool,
) -> AppResult<bool> {
    let Some(names) = server_categories(tags, keywords_kept) else {
        return Ok(false);
    };
    state.store.ensure_categories(&names)?;
    Ok(state.store.set_categories_by_uid(&folder.id, uid, &names)?)
}

/// With a sync window (the account's `sync_months`), the UIDs of the
/// folder's messages inside it (IMAP `SEARCH SINCE`); `None` without one.
async fn window_uids(
    conn: &mut ImapConnection,
    account: &Account,
    folder: &Folder,
    empty: bool,
) -> AppResult<Option<HashSet<u32>>> {
    let since = account
        .sync_months
        .and_then(|months| imap_since(i64::try_from(now()).unwrap_or(0), months));
    Ok(match since {
        Some(since) if !empty => Some(conn.uids_since(&folder.path, &since).await?),
        Some(_) => Some(HashSet::new()),
        None => None,
    })
}

async fn sync_folder(
    state: &AppState,
    account: &Account,
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
    let window = window_uids(conn, account, folder, server.is_empty()).await?;

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
    let fresh = uids_to_fetch(&server, &local_before, highest, settled, window.as_ref());
    let mut new_unseen: u32 = 0;
    // Only UIDs actually stored move the high-water mark, so a message the
    // server didn't return this time is asked for again next time.
    let mut stored_max = highest;
    for chunk in fresh.chunks(FETCH_CHUNK) {
        let Some(set) = uid_set(chunk) else { continue };
        for message in conn.fetch_headers(&folder.path, &set).await? {
            stored_max = stored_max.max(message.uid);
            state.store.upsert_message(
                &folder.id,
                message.uid,
                &message.summary,
                flags(message.flags),
                u64::from(message.size),
            )?;
            apply_categories(
                state,
                folder,
                message.uid,
                &message.tags,
                status.accepts_keywords,
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
    if stored_max > highest {
        state.store.set_highest_uid(&folder.id, stored_max)?;
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
        for (uid, server_flags, tags) in conn.fetch_flags_and_tags(&folder.path, &set).await? {
            changed |= state
                .store
                .set_flags_by_uid(&folder.id, uid, flags(server_flags))?;
            changed |= apply_categories(state, folder, uid, &tags, status.accepts_keywords)?;
        }
    }

    changed |= prefetch_bodies(state, conn, folder, window.as_ref()).await?;
    Ok(Report {
        changed,
        new_unseen,
    })
}

/// UIDs to download, ascending: those above `highest` (new since the last
/// sync) and, when `fill_gaps`, any not in `local`, both limited to the
/// newest `INITIAL_MESSAGES` on the server and, with a sync window, the
/// messages inside it.
fn uids_to_fetch(
    server: &HashSet<u32>,
    local: &HashSet<u32>,
    highest: u32,
    fill_gaps: bool,
    window: Option<&HashSet<u32>>,
) -> Vec<u32> {
    let mut newest: Vec<u32> = server.iter().copied().collect();
    newest.sort_unstable();
    let skip = newest.len().saturating_sub(INITIAL_MESSAGES);
    newest
        .into_iter()
        .enumerate()
        .filter(|(index, uid)| *index >= skip || window.is_some_and(|w| w.contains(uid)))
        .map(|(_, uid)| uid)
        .filter(|uid| *uid > highest || (fill_gaps && !local.contains(uid)))
        .collect()
}

/// Download small bodies so they open offline: every message inside the
/// account's sync window (a batch per pass), or without one the most
/// recent few. Returns whether any were downloaded.
async fn prefetch_bodies(
    state: &AppState,
    conn: &mut ImapConnection,
    folder: &Folder,
    window: Option<&HashSet<u32>>,
) -> AppResult<bool> {
    let mut fetched = false;
    let prefetch = if folder.role == FolderRole::Inbox {
        INBOX_BODY_PREFETCH
    } else {
        BODY_PREFETCH
    };
    let missing = match window {
        Some(window) => {
            let uids: Vec<u32> = window.iter().copied().collect();
            state.store.messages_missing_raw_among(
                &folder.id,
                &uids,
                WINDOW_BODY_BATCH,
                u64::from(MAX_BACKGROUND_BODY_BYTES),
            )?
        }
        None => state.store.messages_missing_raw(
            &folder.id,
            prefetch,
            u64::from(MAX_BACKGROUND_BODY_BYTES),
        )?,
    };
    for (id, uid) in missing {
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
    let summary = parse_summary(raw).ok();
    let preview = summary
        .as_ref()
        .map(|s| s.preview.clone())
        .unwrap_or_default();
    let text = parse_body(raw)
        .ok()
        .and_then(|b| b.text)
        .unwrap_or_else(|| preview.clone());
    state.store.set_body(id, raw, Some(&text), &preview)?;
    // Mail synced before importance was read gets it now.
    if let Some(summary) = summary {
        state.store.set_header_importance(id, summary.importance)?;
    }
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
    if crate::settings::working_offline(state) {
        return Err(AppError::new(
            "connect",
            "You're working offline, and this message isn't on this device yet. Turn off Work Offline to download it.",
        ));
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
    fn only_a_lost_connection_ends_a_sync_pass() {
        let mut problem = None;
        // A folder the database or server chokes on is remembered, not fatal.
        assert!(stop_if_disconnected(AppError::internal("bad folder"), &mut problem).is_ok());
        assert!(stop_if_disconnected(AppError::new("storage", "disk"), &mut problem).is_ok());
        // The first problem is the one reported after the pass.
        assert_eq!(problem.map(|e| e.message), Some("bad folder".to_owned()));
        for kind in ["connect", "tls", "auth", "config", "keyring"] {
            let mut none = None;
            assert!(
                stop_if_disconnected(AppError::new(kind, "x"), &mut none).is_err(),
                "{kind}"
            );
        }
    }

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
        assert_eq!(uids_to_fetch(&server, &local, 8, false, None), vec![9, 10]);
        // Settled: the message missing at UID 4 comes back too.
        assert_eq!(
            uids_to_fetch(&server, &local, 8, true, None),
            vec![4, 9, 10]
        );
        // First sync of a huge folder: only the newest INITIAL_MESSAGES.
        let big: HashSet<u32> = (1..=5000).collect();
        let first = uids_to_fetch(&big, &HashSet::new(), 0, true, None);
        assert_eq!(first.len(), INITIAL_MESSAGES);
        assert_eq!(first.first(), Some(&4001));
        // Older mail that was never downloaded is not treated as a gap.
        let local: HashSet<u32> = (4001..=5000).collect();
        assert!(uids_to_fetch(&big, &local, 5000, true, None).is_empty());
    }

    #[test]
    fn a_sync_window_reaches_past_the_newest_thousand() {
        let big: HashSet<u32> = (1..=5000).collect();
        // A year of mail: UIDs 2001 on.
        let window: HashSet<u32> = (2001..=5000).collect();
        let first = uids_to_fetch(&big, &HashSet::new(), 0, true, Some(&window));
        assert_eq!(first.len(), 3000);
        assert_eq!(first.first(), Some(&2001));
        // A short window still gets the newest INITIAL_MESSAGES headers.
        let week: HashSet<u32> = (4990..=5000).collect();
        let first = uids_to_fetch(&big, &HashSet::new(), 0, true, Some(&week));
        assert_eq!(first.len(), INITIAL_MESSAGES);
        // Widened later: the older ones inside it come in as gaps.
        let local: HashSet<u32> = (4001..=5000).collect();
        let widened = uids_to_fetch(&big, &local, 5000, true, Some(&window));
        assert_eq!(widened.len(), 2000);
        assert!(uids_to_fetch(&big, &local, 5000, false, Some(&window)).is_empty());
    }

    #[test]
    fn categories_come_from_labels_or_kept_keywords() {
        let gmail = Tags {
            keywords: vec!["$Cat_Ignored".into()],
            labels: Some(vec![
                "\\Inbox".into(),
                "\\Important".into(),
                "Clients".into(),
                "Entw&APw-rfe".into(),
            ]),
        };
        assert_eq!(
            server_categories(&gmail, false),
            Some(vec!["Clients".to_owned(), "Entwürfe".to_owned()])
        );
        let imap = Tags {
            keywords: vec!["$Junk".into(), "$Cat_Red_20team".into(), "$label1".into()],
            labels: None,
        };
        assert_eq!(
            server_categories(&imap, true),
            Some(vec!["Red team".to_owned()])
        );
        // A folder that can't keep keywords: the ones set here stay.
        assert_eq!(server_categories(&imap, false), None);
    }

    #[test]
    fn new_changes_round_trip_and_touch_their_folders() {
        let copy = Op::Copy {
            from: "INBOX".into(),
            uids: vec![4],
            to: "Clients".into(),
            create_target: false,
        };
        assert_eq!(op_folders(&copy), [Some("INBOX"), Some("Clients")]);
        let keywords = Op::SetKeywordsByMessageId {
            folder: "Junk".into(),
            message_id: "a@b.c".into(),
            add: vec!["$Junk".into()],
            remove: vec!["$NotJunk".into()],
        };
        assert_eq!(op_folders(&keywords), [Some("Junk"), None]);
        for op in [copy, keywords] {
            let value = serde_json::to_value(&op).unwrap_or_default();
            assert_eq!(serde_json::from_value::<Op>(value).ok(), Some(op));
        }
        assert_eq!(
            labels(&["Entwürfe".to_owned()]),
            vec!["Entw&APw-rfe".to_owned()]
        );
    }

    #[test]
    fn a_wake_with_changes_syncs_only_what_they_touched() {
        assert_eq!(scope_of(Trigger::Connected, false), Scope::All);
        assert_eq!(scope_of(Trigger::Connected, true), Scope::All);
        assert_eq!(scope_of(Trigger::Waited, false), Scope::Inbox);
        assert_eq!(scope_of(Trigger::Woken, true), Scope::Changes);
        // "Check now": nothing queued, so everything is checked.
        assert_eq!(scope_of(Trigger::Woken, false), Scope::All);

        // Marking mail read in Archive checks Archive, not every folder.
        assert!(folder_due(Scope::Changes, FolderRole::Archive, true));
        assert!(!folder_due(Scope::Changes, FolderRole::Archive, false));
        assert!(!folder_due(Scope::Changes, FolderRole::Trash, false));
        for role in [FolderRole::Inbox, FolderRole::Sent, FolderRole::Drafts] {
            assert!(folder_due(Scope::Changes, role, false), "{role:?}");
        }
        assert!(folder_due(Scope::Inbox, FolderRole::Inbox, false));
        assert!(!folder_due(Scope::Inbox, FolderRole::Sent, false));
        assert!(folder_due(Scope::Inbox, FolderRole::Sent, true));
        assert!(folder_due(Scope::All, FolderRole::Trash, false));
        // Virtual views are never synced on their own.
        assert!(!folder_due(Scope::All, FolderRole::All, true));
        assert!(!folder_due(Scope::Changes, FolderRole::Flagged, true));
    }

    #[test]
    fn a_move_touches_both_folders() {
        let moved = Op::MoveByMessageId {
            from: "INBOX".into(),
            message_id: "a@b.c".into(),
            to: "Archive".into(),
            create_target: false,
        };
        assert_eq!(op_folders(&moved), [Some("INBOX"), Some("Archive")]);
        let flagged = Op::SetFlags {
            folder: "Work".into(),
            uids: vec![3],
            seen: Some(true),
            flagged: None,
            answered: None,
        };
        assert_eq!(op_folders(&flagged), [Some("Work"), None]);
    }

    #[test]
    fn folder_display_names() {
        assert_eq!(leaf_name("INBOX.Entw&APw-rfe", Some(".")), "Entwürfe");
        assert_eq!(leaf_name("[Gmail]/Sent Mail", Some("/")), "Sent Mail");
        assert_eq!(leaf_name("Clients", None), "Clients");
    }
}
