//! Reading and organising mail: folders, lists, opening messages, flags,
//! moves, deletes, attachments and links.

use std::sync::Arc;

use mail_core::folders::FolderRole;
use mail_core::importance::Importance;
use mail_core::message::{Mailbox, parse_body};
use mail_core::preview::PreviewKind;
use mail_core::sanitize::{RemoteContent, embed_inline_images, sanitize_html};
use mail_store::{Folder, ListQuery, MessageListItem, MessagePage, NewFolder};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::sync::{self, Op};
use crate::{events, settings};

/// Most messages one action may touch.
const MAX_BATCH: usize = 1000;

pub(crate) fn check_batch(ids: &[String]) -> AppResult<()> {
    if ids.is_empty() || ids.len() > MAX_BATCH {
        return Err(AppError::invalid(format!(
            "Select between 1 and {MAX_BATCH} messages."
        )));
    }
    Ok(())
}

/// `list_folders`.
#[tauri::command]
pub fn list_folders(state: State<'_, Arc<AppState>>, account_id: &str) -> AppResult<Vec<Folder>> {
    Ok(state.store.list_folders(account_id)?)
}

/// `list_messages`. With `threads`, one row per conversation.
#[tauri::command]
pub fn list_messages(
    state: State<'_, Arc<AppState>>,
    query: ListQuery,
    threads: Option<bool>,
) -> AppResult<MessagePage> {
    if threads.unwrap_or(false) {
        Ok(state.store.list_threads(&query)?)
    } else {
        Ok(state.store.list_messages(&query)?)
    }
}

/// `set_focus_rule`: always put mail from this sender (or, with
/// `wholeDomain`, anyone at its domain) in Focused or Other; `None` removes
/// the rule.
#[tauri::command]
pub fn set_focus_rule(
    state: State<'_, Arc<AppState>>,
    sender: String,
    focus: Option<mail_core::focus::Focus>,
    whole_domain: bool,
) -> AppResult<()> {
    let key = mail_core::focus::rule_key(&sender, whole_domain)
        .ok_or_else(|| AppError::invalid("That isn't an email address."))?;
    Ok(state.store.set_focus_rule(&key, focus)?)
}

/// `preview_body`: the HTML recipients will see for what is typed in
/// compose (formatting marks rendered, typed HTML escaped).
#[tauri::command]
pub fn preview_body(body: String) -> AppResult<String> {
    if body.len() > 2 * 1024 * 1024 {
        return Err(AppError::invalid("This message is too long to preview."));
    }
    Ok(mail_core::formatting::html_document(&body))
}

/// `get_thread`: every message in a message's conversation, oldest first.
#[tauri::command]
pub fn get_thread(state: State<'_, Arc<AppState>>, id: String) -> AppResult<Vec<MessageListItem>> {
    Ok(state.store.get_thread(&id)?)
}

/// An attachment as listed in the reading pane.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentMeta {
    index: usize,
    file_name: String,
    mime_type: String,
    size: u64,
    /// How the in-app preview shows it, or `None` (open or save instead).
    preview: Option<PreviewKind>,
}

/// `MessageDetail` in `docs/ipc.md`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDetail {
    #[serde(flatten)]
    item: MessageListItem,
    cc: Vec<Mailbox>,
    /// Bcc recipients, for the user's own drafts and sent mail.
    bcc: Vec<Mailbox>,
    reply_to: Vec<Mailbox>,
    html: Option<String>,
    text: Option<String>,
    blocked_remote_images: usize,
    attachments: Vec<AttachmentMeta>,
    message_id_header: Option<String>,
    in_reply_to: Option<String>,
    references: Vec<String>,
    /// A meeting invitation (or a cancellation or answer) in the message.
    invitation: Option<crate::invitations::InvitationView>,
}

/// `get_message`: the body is downloaded if needed, cleaned and returned.
/// Opening a message never marks it read; the interface calls `set_flags`.
#[tauri::command]
pub async fn get_message(
    state: State<'_, Arc<AppState>>,
    id: String,
    allow_remote: bool,
) -> AppResult<MessageDetail> {
    let raw = sync::body_now(&state, &id).await?;
    let stored = state.store.get_message(&id)?;
    let body = parse_body(&raw)?;
    let remote = if allow_remote || settings::load(&state)?.remote_images == "allow" {
        RemoteContent::Allow
    } else {
        RemoteContent::Block
    };
    let invitation = match body.calendar.as_deref() {
        Some(calendar) => {
            let account = state.store.get_account(&stored.item.account_id)?;
            crate::invitations::view(&state, &account, calendar)?
        }
        None => None,
    };
    let (html, blocked) = match body.html.as_deref() {
        Some(html) => {
            let clean = sanitize_html(html, remote);
            (
                Some(embed_inline_images(&clean.html, &body.inline_images)),
                clean.blocked_remote_images,
            )
        }
        None => (None, 0),
    };
    Ok(MessageDetail {
        item: stored.item,
        cc: stored.cc,
        bcc: crate::outbox::bcc_in(&raw),
        reply_to: stored.reply_to,
        html,
        text: body.text,
        blocked_remote_images: blocked,
        attachments: body
            .attachments
            .into_iter()
            .enumerate()
            // The invitation card stands for the calendar part.
            .filter(|(_, a)| {
                invitation.is_none()
                    || !matches!(a.mime_type.as_str(), "text/calendar" | "application/ics")
            })
            .map(|(index, a)| AttachmentMeta {
                index,
                preview: crate::attachments::preview_for(&a),
                file_name: a.file_name,
                mime_type: a.mime_type,
                size: a.size,
            })
            .collect(),
        message_id_header: stored.message_id_header,
        in_reply_to: stored.in_reply_to,
        references: stored.references,
        invitation,
    })
}

pub(crate) fn notify_changed(app: &AppHandle, folders: &[(String, String)]) {
    let mut by_account: std::collections::HashMap<&str, Vec<String>> =
        std::collections::HashMap::new();
    for (account, folder) in folders {
        by_account.entry(account).or_default().push(folder.clone());
    }
    for (account, folder_ids) in by_account {
        events::changed(app, account, &folder_ids);
    }
}

/// Queue one server change for messages of one folder: by UID, and by
/// Message-ID for those moved here a moment ago, whose new UID isn't known
/// yet (or the change would never leave this device and the next sync
/// would put the old state back).
pub(crate) fn enqueue_for(
    state: &AppState,
    account_id: &str,
    messages: &[(String, Option<u32>)],
    by_uids: impl FnOnce(Vec<u32>) -> Op,
    by_message_id: impl Fn(String) -> Op,
) -> AppResult<()> {
    let uids: Vec<u32> = messages.iter().filter_map(|(_, uid)| *uid).collect();
    if !uids.is_empty() {
        sync::enqueue(state, account_id, &by_uids(uids))?;
    }
    for (id, _) in messages.iter().filter(|(_, uid)| uid.is_none()) {
        if let Some(message_id) = state.store.get_message(id)?.message_id_header {
            let bare = message_id.trim_matches(['<', '>']).to_owned();
            sync::enqueue(state, account_id, &by_message_id(bare))?;
        }
    }
    Ok(())
}

/// `set_flags` arguments (`ids` and the interface's `FlagChange`).
#[allow(
    clippy::option_option,
    reason = "a flagDue of null differs from one left out"
)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlagArgs {
    ids: Vec<String>,
    #[serde(default)]
    seen: Option<bool>,
    #[serde(default)]
    flagged: Option<bool>,
    #[serde(default)]
    pinned: Option<bool>,
    #[serde(default)]
    importance: Option<Importance>,
    /// Left out: unchanged; null: no date; seconds: a date (and flagged).
    #[serde(default, deserialize_with = "crate::convert::present")]
    flag_due: Option<Option<i64>>,
}

impl FlagArgs {
    /// The flagged state to set: a follow-up date flags the message.
    fn flagged(&self) -> Option<bool> {
        match self.flag_due {
            Some(Some(_)) => Some(true),
            _ => self.flagged,
        }
    }
}

/// `set_flags`: read, flagged, pinned, importance and follow-up date.
/// Changes show at once; read and flagged reach the server when online,
/// the rest stay on this device. The arguments are read whole, since
/// Tauri can't tell a `flagDue` of null from one left out.
#[tauri::command]
pub fn set_flags(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    request: tauri::ipc::Request<'_>,
) -> AppResult<()> {
    let tauri::ipc::InvokeBody::Json(body) = request.body() else {
        return Err(AppError::invalid("set_flags expects JSON arguments."));
    };
    let args =
        FlagArgs::deserialize(body).map_err(|e| AppError::invalid(format!("set_flags: {e}")))?;
    check_batch(&args.ids)?;
    if args.flag_due.flatten().is_some_and(|due| due < 0) {
        return Err(AppError::invalid("That follow-up date isn't valid."));
    }
    let ids = &args.ids;
    let (seen, flagged) = (args.seen, args.flagged());
    let groups = sync::group_by_folder(&state, ids)?;
    state.store.set_flags(ids, seen, flagged, None)?;
    state
        .store
        .set_marks(ids, args.pinned, args.importance, args.flag_due)?;
    if seen.is_some() || flagged.is_some() {
        for ((account_id, folder_id), messages) in &groups {
            let folder = state.store.get_folder(folder_id)?.path;
            enqueue_for(
                &state,
                account_id,
                messages,
                |uids| Op::SetFlags {
                    folder: folder.clone(),
                    uids,
                    seen,
                    flagged,
                    answered: None,
                },
                |message_id| Op::SetFlagsByMessageId {
                    folder: folder.clone(),
                    message_id,
                    seen,
                    flagged,
                },
            )?;
        }
    }
    notify_changed(&app, &groups.into_keys().collect::<Vec<_>>());
    Ok(())
}

/// Where to move messages.
#[derive(Debug, Deserialize)]
#[serde(untagged, rename_all = "camelCase")]
pub enum MoveTarget {
    /// A role, resolved per account (Archive, Trash, Inbox...).
    Role {
        /// The role.
        role: FolderRole,
    },
    /// A specific folder.
    Folder {
        /// Folder id.
        #[serde(rename = "folderId")]
        folder_id: String,
    },
}

fn default_path(role: FolderRole) -> Option<&'static str> {
    match role {
        FolderRole::Archive => Some("Archive"),
        FolderRole::Trash => Some("Trash"),
        FolderRole::Junk => Some("Junk"),
        _ => None,
    }
}

/// Find (or create locally) the target folder in an account.
pub(crate) fn resolve_target(
    state: &AppState,
    account_id: &str,
    target: &MoveTarget,
) -> AppResult<(Folder, bool)> {
    match target {
        MoveTarget::Folder { folder_id } => {
            let folder = state.store.get_folder(folder_id)?;
            if folder.account_id != account_id {
                return Err(AppError::invalid(
                    "Messages can only be moved within the same account.",
                ));
            }
            Ok((folder, false))
        }
        MoveTarget::Role { role } => {
            let folders = state.store.list_folders(account_id)?;
            if let Some(folder) = folders.iter().find(|f| f.role == *role) {
                return Ok((folder.clone(), false));
            }
            let name = default_path(*role)
                .ok_or_else(|| AppError::invalid("That folder doesn't exist in this account."))?;
            // Servers that keep every folder under Inbox (`INBOX.Archive`)
            // refuse a folder created beside it, so the message would bounce
            // back to where it came from.
            let delimiter = crate::folders::delimiter_of(&folders);
            let prefix = crate::folders::top_level_prefix(&folders, delimiter.as_deref());
            let path = format!("{prefix}{name}");
            let id = state.store.upsert_folder(
                account_id,
                &NewFolder {
                    path: &path,
                    name,
                    role: *role,
                    delimiter: delimiter.as_deref(),
                },
            )?;
            Ok((state.store.get_folder(&id)?, true))
        }
    }
}

/// `suggest_recipients`: people to offer while typing an address.
///
/// # Errors
/// Database errors.
#[tauri::command]
pub fn suggest_recipients(
    state: State<'_, Arc<AppState>>,
    query: String,
) -> AppResult<Vec<Mailbox>> {
    // Saved contacts first, then people from mail.
    crate::contacts::suggestions(&state, &query, mail_store::MAX_SUGGESTIONS)
}

/// `move_messages`: archive, trash, junk or any folder.
#[tauri::command]
pub fn move_messages(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    target: MoveTarget,
) -> AppResult<()> {
    check_batch(&ids)?;
    let groups = sync::group_by_folder(&state, &ids)?;
    let touched = move_groups(&state, groups, &target)?;
    notify_changed(&app, &touched);
    Ok(())
}

/// Move messages, grouped by folder, locally and (queued) on the server.
/// Returns the (account, folder) pairs that changed.
pub(crate) fn move_groups(
    state: &AppState,
    groups: sync::FolderGroups,
    target: &MoveTarget,
) -> AppResult<Vec<(String, String)>> {
    let mut touched = Vec::new();
    for ((account_id, folder_id), messages) in groups {
        let (target_folder, create_target) = resolve_target(state, &account_id, target)?;
        if target_folder.id == folder_id {
            continue;
        }
        let source = state.store.get_folder(&folder_id)?;
        // Moved locally before the server's new UID arrived (moved twice in
        // a row, or Undo): found on the server by Message-ID instead.
        enqueue_for(
            state,
            &account_id,
            &messages,
            |uids| Op::Move {
                from: source.path.clone(),
                uids,
                to: target_folder.path.clone(),
                create_target,
            },
            |message_id| Op::MoveByMessageId {
                from: source.path.clone(),
                message_id,
                to: target_folder.path.clone(),
                create_target,
            },
        )?;
        let message_ids: Vec<&String> = messages.iter().map(|(id, _)| id).collect();
        state.store.move_local(&message_ids, &target_folder.id)?;
        touched.push((account_id.clone(), folder_id));
        touched.push((account_id, target_folder.id));
    }
    Ok(touched)
}

/// `delete_forever`: only from Trash or Junk, so a slip can't destroy mail.
#[tauri::command]
pub fn delete_forever(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
) -> AppResult<()> {
    check_batch(&ids)?;
    let groups = sync::group_by_folder(&state, &ids)?;
    for (_, folder_id) in groups.keys() {
        let folder = state.store.get_folder(folder_id)?;
        if !matches!(folder.role, FolderRole::Trash | FolderRole::Junk) {
            return Err(AppError::invalid(
                "Only messages in Trash or Spam can be deleted forever. Move them to Trash first.",
            ));
        }
    }
    for ((account_id, folder_id), messages) in &groups {
        let folder = state.store.get_folder(folder_id)?;
        let uids: Vec<u32> = messages.iter().filter_map(|(_, uid)| *uid).collect();
        if !uids.is_empty() {
            sync::enqueue(
                &state,
                account_id,
                &Op::Delete {
                    folder: folder.path.clone(),
                    uids,
                },
            )?;
        }
        // Moved to Trash a moment ago (no UID yet): delete by Message-ID,
        // or the queued move puts it back in Trash on the server.
        for (id, _) in messages.iter().filter(|(_, uid)| uid.is_none()) {
            if let Some(message_id) = state.store.get_message(id)?.message_id_header {
                sync::enqueue(
                    &state,
                    account_id,
                    &Op::DeleteByMessageId {
                        folder: folder.path.clone(),
                        message_id: message_id.trim_matches(['<', '>']).to_owned(),
                    },
                )?;
            }
        }
    }
    state.store.delete_messages(&ids)?;
    notify_changed(&app, &groups.into_keys().collect::<Vec<_>>());
    Ok(())
}

/// A file chosen to attach.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftAttachment {
    /// Absolute path.
    pub path: String,
    /// Name shown and sent.
    pub file_name: String,
    /// Size in bytes.
    pub size: u64,
}

/// `sync_now`.
#[tauri::command]
pub fn sync_now(state: State<'_, Arc<AppState>>, account_id: Option<String>) {
    state.wake_sync(account_id.as_deref());
    state.outbox_wake.notify_one();
}

/// Whether a link may be opened outside the app.
fn is_openable(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:"))
        && !url.chars().any(char::is_control)
}

/// `open_external`: web and mail links open in the user's own apps.
#[tauri::command]
pub fn open_external(app: AppHandle, url: String) -> AppResult<()> {
    if !is_openable(&url) {
        return Err(AppError::invalid("Only web and email links can be opened."));
    }
    app.opener()
        .open_url(url.trim(), None::<&str>)
        .map_err(|e| AppError::internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_safe_links_open() {
        assert!(is_openable("https://mocky.co.ke"));
        assert!(is_openable("mailto:info@mocky.co.ke"));
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "smb://host/share",
            "https://x\n.co",
            "/relative",
        ] {
            assert!(!is_openable(bad), "{bad}");
        }
    }

    #[test]
    fn move_targets_parse_both_shapes() {
        let role: MoveTarget =
            serde_json::from_str(r#"{"role":"archive"}"#).unwrap_or(MoveTarget::Folder {
                folder_id: String::new(),
            });
        assert!(matches!(
            role,
            MoveTarget::Role {
                role: FolderRole::Archive
            }
        ));
        let folder: MoveTarget =
            serde_json::from_str(r#"{"folderId":"f1"}"#).unwrap_or(MoveTarget::Role {
                role: FolderRole::Other,
            });
        assert!(matches!(folder, MoveTarget::Folder { ref folder_id } if folder_id == "f1"));
    }

    #[test]
    fn a_null_follow_up_date_is_told_from_one_left_out() {
        let parse = |json: serde_json::Value| FlagArgs::deserialize(&json).ok();
        let left_out = parse(serde_json::json!({ "ids": ["a"], "pinned": true }));
        assert!(left_out.as_ref().is_some_and(|a| a.flag_due.is_none()
            && a.pinned == Some(true)
            && a.flagged().is_none()));
        let cleared = parse(serde_json::json!({ "ids": ["a"], "flagDue": null }));
        assert!(cleared.is_some_and(|a| a.flag_due == Some(None) && a.flagged().is_none()));
        // A date flags the message too.
        let dated = parse(
            serde_json::json!({ "ids": ["a"], "flagDue": 1_800_000_000, "importance": "high" }),
        );
        assert!(
            dated.is_some_and(|a| a.flag_due == Some(Some(1_800_000_000))
                && a.flagged() == Some(true)
                && a.importance == Some(Importance::High))
        );
        assert!(parse(serde_json::json!({ "ids": ["a"], "importance": "urgent" })).is_none());
    }

    #[test]
    fn batches_are_bounded() {
        assert!(check_batch(&[]).is_err());
        assert!(check_batch(&vec![String::new(); MAX_BATCH + 1]).is_err());
        assert!(check_batch(&[String::new()]).is_ok());
    }
}
