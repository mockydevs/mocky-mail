//! Reading and organising mail: folders, lists, opening messages, flags,
//! moves, deletes, attachments and links.

use std::sync::Arc;

use mail_core::folders::FolderRole;
use mail_core::message::{Mailbox, attachment_bytes, parse_body};
use mail_core::sanitize::{RemoteContent, embed_inline_images, sanitize_html};
use mail_store::{Folder, ListQuery, MessageListItem, MessagePage, NewFolder};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, AppResult};
use crate::state::{AppState, lock};
use crate::sync::{self, Op};
use crate::{events, settings};

/// Most messages one action may touch.
const MAX_BATCH: usize = 1000;

fn check_batch(ids: &[String]) -> AppResult<()> {
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

/// `list_messages`.
#[tauri::command]
pub fn list_messages(state: State<'_, Arc<AppState>>, query: ListQuery) -> AppResult<MessagePage> {
    Ok(state.store.list_messages(&query)?)
}

/// An attachment as listed in the reading pane.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentMeta {
    index: usize,
    file_name: String,
    mime_type: String,
    size: u64,
}

/// `MessageDetail` in `docs/ipc.md`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDetail {
    #[serde(flatten)]
    item: MessageListItem,
    cc: Vec<Mailbox>,
    reply_to: Vec<Mailbox>,
    html: Option<String>,
    text: Option<String>,
    blocked_remote_images: usize,
    attachments: Vec<AttachmentMeta>,
    message_id_header: Option<String>,
    references: Vec<String>,
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
        reply_to: stored.reply_to,
        html,
        text: body.text,
        blocked_remote_images: blocked,
        attachments: body
            .attachments
            .into_iter()
            .enumerate()
            .map(|(index, a)| AttachmentMeta {
                index,
                file_name: a.file_name,
                mime_type: a.mime_type,
                size: a.size,
            })
            .collect(),
        message_id_header: stored.message_id_header,
        references: stored.references,
    })
}

fn notify_changed(app: &AppHandle, folders: &[(String, String)]) {
    let mut by_account: std::collections::HashMap<&str, Vec<String>> =
        std::collections::HashMap::new();
    for (account, folder) in folders {
        by_account.entry(account).or_default().push(folder.clone());
    }
    for (account, folder_ids) in by_account {
        events::changed(app, account, &folder_ids);
    }
}

/// `set_flags`: changes show at once and reach the server when online.
#[tauri::command]
pub fn set_flags(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    seen: Option<bool>,
    flagged: Option<bool>,
) -> AppResult<()> {
    check_batch(&ids)?;
    let groups = sync::group_by_folder(&state, &ids)?;
    state.store.set_flags(&ids, seen, flagged, None)?;
    for ((account_id, folder_id), messages) in &groups {
        let uids: Vec<u32> = messages.iter().filter_map(|(_, uid)| *uid).collect();
        if uids.is_empty() {
            continue;
        }
        let folder = state.store.get_folder(folder_id)?;
        sync::enqueue(
            &state,
            account_id,
            &Op::SetFlags {
                folder: folder.path,
                uids,
                seen,
                flagged,
                answered: None,
            },
        )?;
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
fn resolve_target(
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
            if let Some(folder) = state
                .store
                .list_folders(account_id)?
                .into_iter()
                .find(|f| f.role == *role)
            {
                return Ok((folder, false));
            }
            let path = default_path(*role)
                .ok_or_else(|| AppError::invalid("That folder doesn't exist in this account."))?;
            let id = state.store.upsert_folder(
                account_id,
                &NewFolder {
                    path,
                    name: path,
                    role: *role,
                    delimiter: None,
                },
            )?;
            Ok((state.store.get_folder(&id)?, true))
        }
    }
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
    let mut touched = Vec::new();
    for ((account_id, folder_id), messages) in groups {
        let (target_folder, create_target) = resolve_target(&state, &account_id, &target)?;
        if target_folder.id == folder_id {
            continue;
        }
        let source = state.store.get_folder(&folder_id)?;
        let uids: Vec<u32> = messages.iter().filter_map(|(_, uid)| *uid).collect();
        if !uids.is_empty() {
            sync::enqueue(
                &state,
                &account_id,
                &Op::Move {
                    from: source.path,
                    uids,
                    to: target_folder.path.clone(),
                    create_target,
                },
            )?;
        }
        let message_ids: Vec<&String> = messages.iter().map(|(id, _)| id).collect();
        state.store.move_local(&message_ids, &target_folder.id)?;
        touched.push((account_id.clone(), folder_id));
        touched.push((account_id, target_folder.id));
    }
    notify_changed(&app, &touched);
    Ok(())
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
                    folder: folder.path,
                    uids,
                },
            )?;
        }
    }
    state.store.delete_messages(&ids)?;
    notify_changed(&app, &groups.into_keys().collect::<Vec<_>>());
    Ok(())
}

/// `save_attachment`: asks where to save, then writes the file.
#[tauri::command]
pub async fn save_attachment(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    message_id: String,
    index: usize,
) -> AppResult<Option<String>> {
    let raw = sync::body_now(&state, &message_id).await?;
    let (info, bytes) = attachment_bytes(&raw, index)?
        .ok_or_else(|| AppError::new("not_found", "That attachment no longer exists."))?;
    let dialog = app.dialog().file().set_file_name(&info.file_name);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    let Some(path) = picked else { return Ok(None) };
    let path = path
        .into_path()
        .map_err(|e| AppError::internal(e.to_string()))?;
    tokio::fs::write(&path, bytes).await?;
    Ok(Some(path.display().to_string()))
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

/// `pick_attachments`: the open-file dialog. Only files picked here may be
/// attached to a message.
#[tauri::command]
pub async fn pick_attachments(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> AppResult<Vec<DraftAttachment>> {
    let dialog = app.dialog().file();
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_files())
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .unwrap_or_default();
    let mut out = Vec::new();
    for file in picked {
        let path = file
            .into_path()
            .map_err(|e| AppError::internal(e.to_string()))?;
        let metadata = tokio::fs::metadata(&path).await?;
        if !metadata.is_file() {
            continue;
        }
        let file_name = mail_core::message::safe_file_name(
            &path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        lock(&state.picked_files).insert(path.clone());
        out.push(DraftAttachment {
            path: path.display().to_string(),
            file_name,
            size: metadata.len(),
        });
    }
    Ok(out)
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
    fn batches_are_bounded() {
        assert!(check_batch(&[]).is_err());
        assert!(check_batch(&vec![String::new(); MAX_BATCH + 1]).is_err());
        assert!(check_batch(&[String::new()]).is_ok());
    }
}
