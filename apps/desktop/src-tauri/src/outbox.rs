//! Sending: drafts become outbox entries, and one background worker sends
//! them with `mail-core`'s retry rules. Every state change is saved before
//! the next network step, so a crash can't lose or double-send a message.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mail_core::compose::parse_address_list;
use mail_core::folders::FolderRole;
use mail_core::limits::estimate_message_size;
use mail_core::message::{
    Mailbox, attachment_bytes, make_preview, parse_body, parse_summary, safe_file_name,
};
use mail_core::outbox::{OutboxItem, OutboxState, RetryPolicy, SendOutcome};
use mail_store::{Account, AuthKind, NewOutbox, OutboxEntry};
use mail_sync::imap::ImapConnection;
use mail_sync::outgoing::{OutgoingAttachment, OutgoingMessage, build, mime_for_file_name};
use mail_sync::smtp;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::convert::to_sync;
use crate::credentials;
use crate::error::{AppError, AppResult};
use crate::events;
use crate::mail::DraftAttachment;
use crate::state::{AppState, lock, now};
use crate::sync::{self, Op};

/// Size limit when the account doesn't say (most providers allow 25 MB).
const DEFAULT_LIMIT: u64 = 25 * 1024 * 1024;
/// Largest single attachment read from disk.
const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;
/// Longest the worker sleeps between checks.
const MAX_IDLE: Duration = Duration::from_secs(30);

/// A message as written in compose (`Draft` in `docs/ipc.md`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    account_id: String,
    to: String,
    cc: String,
    bcc: String,
    subject: String,
    body_text: String,
    in_reply_to: Option<String>,
    references: Vec<String>,
    attachments: Vec<DraftAttachment>,
    reply_to_message_id: Option<String>,
}

/// An outbox row for the interface (`OutboxEntry` in `docs/ipc.md`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxDto {
    id: String,
    account_id: String,
    to: Vec<Mailbox>,
    subject: String,
    preview: String,
    created_at: u64,
    attempts: u32,
    status: OutboxState,
    progress: Option<f64>,
}

/// Convert a stored entry for the interface.
pub fn dto(entry: &OutboxEntry) -> OutboxDto {
    OutboxDto {
        id: entry.id.clone(),
        account_id: entry.account_id.clone(),
        to: entry.to.clone(),
        subject: entry.subject.clone(),
        preview: entry.preview.clone(),
        created_at: entry.created_at,
        attempts: entry.item.attempts,
        status: entry.item.state.clone(),
        progress: None,
    }
}

fn limit_for(account: &Account) -> u64 {
    account.size_limit_bytes.unwrap_or(DEFAULT_LIMIT)
}

fn field(name: &str, value: &str) -> AppResult<Vec<Mailbox>> {
    parse_address_list(value).map_err(|e| AppError::invalid(format!("{name}: {e}")))
}

async fn check_picked(state: &AppState, attachment: &DraftAttachment) -> AppResult<(PathBuf, u64)> {
    let path = PathBuf::from(&attachment.path);
    if !lock(&state.picked_files).contains(&path) {
        return Err(AppError::invalid("Attach files with the Attach button."));
    }
    let size = tokio::fs::metadata(&path)
        .await
        .map_err(|_| {
            AppError::invalid(format!("{} can't be read any more.", attachment.file_name))
        })?
        .len();
    if size > MAX_ATTACHMENT_BYTES {
        return Err(AppError::new(
            "too_large",
            format!("{} is too large to attach.", attachment.file_name),
        ));
    }
    Ok((path, size))
}

fn size_error(estimated: u64, limit: u64) -> AppError {
    let mb = |b: u64| b.div_ceil(1024 * 1024);
    AppError::new(
        "too_large",
        format!(
            "This message is about {} MB once sent; this account accepts up to {} MB.",
            mb(estimated),
            mb(limit)
        ),
    )
}

/// Estimated size of a draft and the limit, checked.
async fn measure(state: &AppState, draft: &Draft) -> AppResult<(Account, u64, u64)> {
    let account = state.store.get_account(&draft.account_id)?;
    let mut sizes = Vec::new();
    for attachment in &draft.attachments {
        sizes.push(check_picked(state, attachment).await?.1);
    }
    let estimated = estimate_message_size(
        u64::try_from(draft.body_text.len()).unwrap_or(u64::MAX),
        &sizes,
    );
    let limit = limit_for(&account);
    if estimated > limit {
        return Err(size_error(estimated, limit));
    }
    Ok((account, estimated, limit))
}

async fn prepare(state: &AppState, draft: &Draft) -> AppResult<(Account, OutgoingMessage)> {
    let (account, _, _) = measure(state, draft).await?;
    let mut attachments = Vec::new();
    for attachment in &draft.attachments {
        let (path, _) = check_picked(state, attachment).await?;
        let file_name = safe_file_name(&attachment.file_name);
        attachments.push(OutgoingAttachment {
            mime_type: mime_for_file_name(&file_name).to_owned(),
            file_name,
            data: tokio::fs::read(&path).await?,
        });
    }
    let message = OutgoingMessage {
        from: Mailbox {
            name: Some(account.display_name.clone()),
            address: account.email.clone(),
        },
        to: field("To", &draft.to)?,
        cc: field("Cc", &draft.cc)?,
        bcc: field("Bcc", &draft.bcc)?,
        subject: draft.subject.clone(),
        body_text: draft.body_text.clone(),
        in_reply_to: draft.in_reply_to.clone(),
        references: draft.references.clone(),
        attachments,
    };
    Ok((account, message))
}

/// Size estimate for compose.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftCheck {
    estimated_bytes: u64,
    limit_bytes: u64,
}

/// `check_draft`: validates recipients and size before sending.
#[tauri::command]
pub async fn check_draft(state: State<'_, Arc<AppState>>, draft: Draft) -> AppResult<DraftCheck> {
    for (name, value) in [("To", &draft.to), ("Cc", &draft.cc), ("Bcc", &draft.bcc)] {
        field(name, value)?;
    }
    let (_, estimated_bytes, limit_bytes) = measure(&state, &draft).await?;
    Ok(DraftCheck {
        estimated_bytes,
        limit_bytes,
    })
}

fn folder_path(state: &AppState, account_id: &str, role: FolderRole) -> AppResult<Option<String>> {
    Ok(state
        .store
        .list_folders(account_id)?
        .into_iter()
        .find(|f| f.role == role)
        .map(|f| f.path))
}

/// `send_message`: queue it; the worker sends it.
#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    draft: Draft,
) -> AppResult<OutboxDto> {
    let (account, message) = prepare(&state, &draft).await?;
    let built = build(&message)?;
    let created_at = now();
    let id = state.store.insert_outbox(&NewOutbox {
        account_id: account.id.clone(),
        created_at,
        to: message.to.iter().chain(&message.cc).cloned().collect(),
        subject: message.subject.clone(),
        preview: make_preview(&message.body_text, mail_core::message::PREVIEW_CHARS),
        raw: built.raw,
        recipients: built.recipients,
        message_id_header: Some(built.message_id),
        reply_to_message_id: draft.reply_to_message_id.clone(),
        sent_folder_hint: folder_path(&state, &account.id, FolderRole::Sent)?,
        item: OutboxItem::new(created_at),
    })?;
    state.outbox_wake.notify_one();
    events::outbox_changed(&app, &state);
    Ok(dto(&state.store.get_outbox(&id)?.entry))
}

/// `save_draft`: stores it in the account's Drafts folder on the server.
#[tauri::command]
pub async fn save_draft(state: State<'_, Arc<AppState>>, draft: Draft) -> AppResult<()> {
    let (account, mut message) = prepare(&state, &draft).await?;
    if message.to.is_empty() && message.cc.is_empty() && message.bcc.is_empty() {
        // Drafts may have no recipients yet; address it to yourself for building only.
        message.bcc.push(message.from.clone());
    }
    let built = build(&message)?;
    let path = folder_path(&state, &account.id, FolderRole::Drafts)?
        .ok_or_else(|| AppError::invalid("This account has no Drafts folder."))?;
    let credentials = credentials::for_account(&state, &account).await?;
    let mut conn = ImapConnection::connect(&to_sync(&account.imap), &credentials).await?;
    conn.append(&path, &built.raw, "(\\Draft \\Seen)").await?;
    conn.logout().await;
    state.wake_sync(Some(&account.id));
    Ok(())
}

/// `list_outbox`.
#[tauri::command]
pub fn list_outbox(state: State<'_, Arc<AppState>>) -> AppResult<Vec<OutboxDto>> {
    Ok(state.store.list_outbox()?.iter().map(dto).collect())
}

fn update(app: &AppHandle, state: &AppState, id: &str, item: &OutboxItem) -> AppResult<()> {
    state.store.update_outbox_item(id, item)?;
    events::outbox_changed(app, state);
    Ok(())
}

fn transition(result: Result<(), mail_core::outbox::TransitionError>) -> AppResult<()> {
    result.map_err(|e| AppError::invalid(format!("That isn't possible right now: {e}.")))
}

/// `outbox_send_now`: skip the retry wait, or retry a failed message.
#[tauri::command]
pub fn outbox_send_now(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> AppResult<()> {
    let mut item = state.store.get_outbox(&id)?.entry.item;
    transition(item.send_now(now()))?;
    update(&app, &state, &id, &item)?;
    state.outbox_wake.notify_one();
    Ok(())
}

/// `outbox_cancel`.
#[tauri::command]
pub fn outbox_cancel(app: AppHandle, state: State<'_, Arc<AppState>>, id: String) -> AppResult<()> {
    let mut item = state.store.get_outbox(&id)?.entry.item;
    transition(item.cancel())?;
    state.store.delete_outbox(&id)?;
    events::outbox_changed(&app, &state);
    Ok(())
}

fn join(list: &[Mailbox]) -> String {
    list.iter()
        .map(|m| match &m.name {
            Some(name) => format!("\"{}\" <{}>", name.replace('"', ""), m.address),
            None => m.address.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `outbox_edit`: take a queued or failed message back into compose.
/// Attachments are restored as files in the app's cache folder.
#[tauri::command]
pub async fn outbox_edit(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> AppResult<Draft> {
    let message = state.store.get_outbox(&id)?;
    let mut item = message.entry.item.clone();
    transition(item.cancel())?;
    let summary = parse_summary(&message.raw)?;
    let body = parse_body(&message.raw)?;
    let dir = state.cache_dir.join("outbox-edit").join(&id);
    tokio::fs::create_dir_all(&dir).await?;
    let mut attachments = Vec::new();
    for index in 0..body.attachments.len() {
        if let Some((info, bytes)) = attachment_bytes(&message.raw, index)? {
            let path = dir.join(&info.file_name);
            tokio::fs::write(&path, &bytes).await?;
            lock(&state.picked_files).insert(path.clone());
            attachments.push(DraftAttachment {
                path: path.display().to_string(),
                file_name: info.file_name,
                size: info.size,
            });
        }
    }
    state.store.delete_outbox(&id)?;
    events::outbox_changed(&app, &state);
    Ok(Draft {
        account_id: message.entry.account_id,
        to: join(&summary.to),
        cc: join(&summary.cc),
        bcc: String::new(),
        subject: summary.subject,
        body_text: body.text.unwrap_or_default(),
        in_reply_to: summary.in_reply_to,
        references: summary.references,
        attachments,
        reply_to_message_id: message.entry.reply_to_message_id,
    })
}

/// The worker: runs for the life of the app.
pub async fn run(app: AppHandle, state: Arc<AppState>) {
    loop {
        let wait = match Box::pin(process(&app, &state)).await {
            Ok(wait) => wait,
            Err(error) => {
                eprintln!("outbox: {error}");
                MAX_IDLE
            }
        };
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            () = state.outbox_wake.notified() => {}
        }
    }
}

async fn process(app: &AppHandle, state: &AppState) -> AppResult<Duration> {
    let mut wait = MAX_IDLE;
    for entry in state.store.list_outbox()? {
        let t = now();
        match entry.item.state {
            OutboxState::Queued {
                maybe_sent: true, ..
            } => resolve_duplicate(app, state, &entry).await?,
            OutboxState::Queued {
                next_attempt_at, ..
            } if next_attempt_at > t => {
                wait = wait.min(Duration::from_secs(next_attempt_at.saturating_sub(t)));
            }
            OutboxState::Queued { .. } => Box::pin(send_one(app, state, &entry)).await?,
            OutboxState::Sent { .. } | OutboxState::Cancelled => {
                state.store.delete_outbox(&entry.id)?;
                events::outbox_changed(app, state);
            }
            OutboxState::Sending { .. } | OutboxState::Failed { .. } => {}
        }
    }
    Ok(wait.max(Duration::from_secs(1)))
}

/// After a crash mid-send: look in Sent before sending again.
async fn resolve_duplicate(
    app: &AppHandle,
    state: &AppState,
    entry: &OutboxEntry,
) -> AppResult<()> {
    let account = state.store.get_account(&entry.account_id)?;
    let sent = match entry.sent_folder_hint.clone() {
        Some(path) => Some(path),
        None => folder_path(state, &account.id, FolderRole::Sent)?,
    };
    let found = match (sent, entry.message_id_header.as_deref()) {
        (Some(path), Some(message_id)) => {
            let Ok(credentials) = credentials::for_account(state, &account).await else {
                return Ok(());
            };
            let Ok(mut conn) = ImapConnection::connect(&to_sync(&account.imap), &credentials).await
            else {
                return Ok(());
            };
            let found = conn.contains_message_id(&path, message_id).await;
            conn.logout().await;
            match found {
                Ok(found) => found,
                Err(_) => return Ok(()),
            }
        }
        _ => false,
    };
    let mut item = entry.item.clone();
    transition(item.resolve_duplicate_check(found, now()))?;
    update(app, state, &entry.id, &item)
}

async fn send_one(app: &AppHandle, state: &AppState, entry: &OutboxEntry) -> AppResult<()> {
    let account = state.store.get_account(&entry.account_id)?;
    let message = state.store.get_outbox(&entry.id)?;
    let mut item: OutboxItem = entry.item.clone();
    let estimated = u64::try_from(message.raw.len()).unwrap_or(u64::MAX);
    if estimated > limit_for(&account) {
        transition(item.fail_too_large(mail_core::limits::TooLarge {
            estimated,
            limit: limit_for(&account),
        }))?;
        return update(app, state, &entry.id, &item);
    }
    // Saved before the network call: if the app dies now, restart recovery
    // knows this message may have gone out.
    transition(item.begin_send(now()))?;
    update(app, state, &entry.id, &item)?;

    let outcome = match credentials::for_account(state, &account).await {
        Err(error) => SendOutcome::Transient(error.message),
        Ok(credentials) => {
            match smtp::send(
                &to_sync(&account.smtp),
                &credentials,
                &account.email,
                &message.recipients,
                &message.raw,
            )
            .await
            {
                Ok(()) => SendOutcome::Delivered,
                Err(mail_sync::SyncError::Send {
                    message,
                    permanent: true,
                }) => SendOutcome::Permanent(message),
                Err(error) => SendOutcome::Transient(error.to_string()),
            }
        }
    };
    let delivered = outcome == SendOutcome::Delivered;
    transition(item.finish(outcome, now(), &RetryPolicy::default()))?;
    update(app, state, &entry.id, &item)?;
    if delivered {
        after_delivery(state, &account, entry, &message.raw).await;
        state.store.delete_outbox(&entry.id)?;
        events::outbox_changed(app, state);
    }
    Ok(())
}

async fn after_delivery(state: &AppState, account: &Account, entry: &OutboxEntry, raw: &[u8]) {
    // Gmail and Microsoft 365 file sent mail themselves; others need a copy.
    if account.auth == AuthKind::Password {
        let sent = entry.sent_folder_hint.clone().or_else(|| {
            folder_path(state, &account.id, FolderRole::Sent)
                .ok()
                .flatten()
        });
        if let (Some(path), Ok(credentials)) =
            (sent, credentials::for_account(state, account).await)
        {
            match ImapConnection::connect(&to_sync(&account.imap), &credentials).await {
                Ok(mut conn) => {
                    if let Err(error) = conn.append(&path, raw, "(\\Seen)").await {
                        eprintln!("couldn't save a copy to Sent: {error}");
                    }
                    conn.logout().await;
                }
                Err(error) => eprintln!("couldn't save a copy to Sent: {error}"),
            }
        }
    }
    if let Some(original) = &entry.reply_to_message_id
        && let Ok(stored) = state.store.get_message(original)
    {
        let _ = state.store.set_flags(&[original], None, None, Some(true));
        if let (Some(uid), Ok(folder)) =
            (stored.uid, state.store.get_folder(&stored.item.folder_id))
        {
            let op = Op::SetFlags {
                folder: folder.path,
                uids: vec![uid],
                seen: None,
                flagged: None,
                answered: Some(true),
            };
            let _ = sync::enqueue(state, &account.id, &op);
        }
    }
    state.wake_sync(Some(&account.id));
}
