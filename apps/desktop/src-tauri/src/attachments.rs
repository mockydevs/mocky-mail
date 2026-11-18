//! Attachments of received mail: preview inside the app, open in another
//! app, save; and picking files to attach to a new message.
//!
//! Desktop uses normal file dialogs and paths. Android's file screens hand
//! back `content://` links instead, so there these go through
//! `files_android` (the system's own pick, save and open screens).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mail_core::attachment_policy;
use mail_core::message::{AttachmentInfo, attachment_bytes, safe_file_name};
use mail_core::preview::{self, PreviewKind};
use tauri::ipc::Response;
use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::mail::DraftAttachment;
use crate::state::{AppState, lock};
use crate::sync;

/// Largest file that may be attached (`outbox` checks it again at send).
pub const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;

/// Cache folders holding copies made for opening or saving. Emptied when
/// the app starts; files picked to attach live elsewhere because queued
/// messages may still need them.
const OPENED_DIR: &str = "opened";

async fn load(
    state: &AppState,
    message_id: &str,
    index: usize,
) -> AppResult<(AttachmentInfo, Vec<u8>)> {
    let raw = sync::body_now(state, message_id).await?;
    attachment_bytes(&raw, index)?
        .ok_or_else(|| AppError::new("not_found", "That attachment no longer exists."))
}

/// Write a copy to `cache/opened/<message>-<index>/<name>` and return its path.
async fn write_copy(
    state: &AppState,
    message_id: &str,
    index: usize,
    info: &AttachmentInfo,
    bytes: &[u8],
) -> AppResult<PathBuf> {
    let dir = state
        .cache_dir
        .join(OPENED_DIR)
        .join(format!("{}-{index}", safe_file_name(message_id)));
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(&info.file_name);
    tokio::fs::write(&path, bytes).await?;
    Ok(path)
}

/// Empty the copies left from the last run. Called once at start-up.
pub fn clear_opened(cache_dir: &Path) {
    let _ = std::fs::remove_dir_all(cache_dir.join(OPENED_DIR));
}

/// `preview_attachment`: the attachment's bytes, for the in-app preview.
/// Refused (`invalid`) unless the file really is a photo, PDF or plain
/// text, whatever its name says.
#[tauri::command]
pub async fn preview_attachment(
    state: State<'_, Arc<AppState>>,
    message_id: String,
    index: usize,
) -> AppResult<Response> {
    let (info, bytes) = load(&state, &message_id, index).await?;
    let kind = preview::preview_kind(&info.file_name, &info.mime_type, info.size);
    match kind {
        Some(kind) if preview::content_is(kind, &bytes) => Ok(Response::new(bytes)),
        _ => Err(AppError::invalid(format!(
            "{} can't be shown here. Open it in another app or save it.",
            info.file_name
        ))),
    }
}

/// Programs and scripts are never opened from an email; they may be saved.
fn check_openable(info: &AttachmentInfo, bytes: &[u8]) -> AppResult<()> {
    attachment_policy::check_outgoing(&info.file_name, bytes).map_err(|_| {
        AppError::invalid(format!(
            "{} is a program or script, so Mocky Mail won't open it. Save it only if you trust the sender.",
            info.file_name
        ))
    })
}

/// `open_attachment`: open the file in the app the device uses for its
/// type (PDF reader, Word, photo viewer).
#[tauri::command]
pub async fn open_attachment(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    message_id: String,
    index: usize,
) -> AppResult<()> {
    let (info, bytes) = load(&state, &message_id, index).await?;
    check_openable(&info, &bytes)?;
    let path = write_copy(&state, &message_id, index, &info, &bytes).await?;
    open_path(&app, &path, &info).await
}

#[cfg(target_os = "android")]
async fn open_path(_app: &AppHandle, path: &Path, info: &AttachmentInfo) -> AppResult<()> {
    crate::files_android::open_file(path, &info.file_name, &info.mime_type).await
}

#[cfg(not(target_os = "android"))]
#[allow(clippy::unused_async)] // async to match the Android version
async fn open_path(app: &AppHandle, path: &Path, _info: &AttachmentInfo) -> AppResult<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(path.display().to_string(), None::<&str>)
        .map_err(|_| {
            AppError::invalid("No app on this computer can open this file. Save it instead.")
        })
}

/// `save_attachment`: ask where to save it, then write it there. The place
/// it was saved, or `None` if the user cancelled.
#[tauri::command]
pub async fn save_attachment(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    message_id: String,
    index: usize,
) -> AppResult<Option<String>> {
    let (info, bytes) = load(&state, &message_id, index).await?;
    save(&app, &state, &message_id, index, &info, bytes).await
}

#[cfg(target_os = "android")]
async fn save(
    _app: &AppHandle,
    state: &AppState,
    message_id: &str,
    index: usize,
    info: &AttachmentInfo,
    bytes: Vec<u8>,
) -> AppResult<Option<String>> {
    let path = write_copy(state, message_id, index, info, &bytes).await?;
    crate::files_android::save_file(&path, &info.file_name, &info.mime_type).await
}

#[cfg(not(target_os = "android"))]
async fn save(
    app: &AppHandle,
    _state: &AppState,
    _message_id: &str,
    _index: usize,
    info: &AttachmentInfo,
    bytes: Vec<u8>,
) -> AppResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app.dialog().file().set_file_name(&info.file_name);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    let Some(path) = picked else { return Ok(None) };
    let path = path.into_path().map_err(|_| {
        AppError::invalid("That place can't be saved to. Pick a folder on this computer.")
    })?;
    tokio::fs::write(&path, bytes).await?;
    Ok(Some(path.display().to_string()))
}

/// `pick_attachments`: the open-file dialog. Only files picked here may be
/// attached to a message.
#[tauri::command]
pub async fn pick_attachments(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> AppResult<Vec<DraftAttachment>> {
    let mut out = Vec::new();
    for path in pick_paths(&app).await? {
        let metadata = tokio::fs::metadata(&path).await?;
        if !metadata.is_file() {
            continue;
        }
        let file_name = safe_file_name(
            &path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        // Programs and scripts can't be sent; say so now, not at Send.
        let data = tokio::fs::read(&path).await?;
        attachment_policy::check_outgoing(&file_name, &data)
            .map_err(|refusal| AppError::invalid(format!("{file_name}: {refusal}")))?;
        lock(&state.picked_files).insert(path.clone());
        out.push(DraftAttachment {
            path: path.display().to_string(),
            file_name,
            size: metadata.len(),
        });
    }
    Ok(out)
}

/// Android: the phone's picker copies each file into the app's cache.
#[cfg(target_os = "android")]
async fn pick_paths(_app: &AppHandle) -> AppResult<Vec<PathBuf>> {
    Ok(crate::files_android::pick_files(MAX_ATTACHMENT_BYTES)
        .await?
        .into_iter()
        .map(|p| p.path)
        .collect())
}

#[cfg(not(target_os = "android"))]
async fn pick_paths(app: &AppHandle) -> AppResult<Vec<PathBuf>> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app.dialog().file();
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_files())
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .unwrap_or_default();
    picked
        .into_iter()
        .map(|file| {
            file.into_path().map_err(|_| {
                AppError::invalid("That file can't be attached. Pick a file on this computer.")
            })
        })
        .collect()
}

/// The preview offered for an attachment in the reading pane.
#[must_use]
pub fn preview_for(info: &AttachmentInfo) -> Option<PreviewKind> {
    preview::preview_kind(&info.file_name, &info.mime_type, info.size)
}

/// Choose one picture (signature images). `None` if cancelled.
pub async fn pick_image(app: &AppHandle) -> AppResult<Option<PathBuf>> {
    pick_image_path(app).await
}

#[cfg(target_os = "android")]
async fn pick_image_path(_app: &AppHandle) -> AppResult<Option<PathBuf>> {
    // A little over the 1 MB limit, so a too-big file gets the clear message.
    Ok(crate::files_android::pick_files(2 * 1024 * 1024)
        .await?
        .into_iter()
        .next()
        .map(|p| p.path))
}

#[cfg(not(target_os = "android"))]
async fn pick_image_path(app: &AppHandle) -> AppResult<Option<PathBuf>> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app
        .dialog()
        .file()
        .add_filter("Pictures", &["png", "jpg", "jpeg", "gif"]);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_file())
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    picked
        .map(|file| {
            file.into_path()
                .map_err(|_| AppError::invalid("Pick a picture on this computer."))
        })
        .transpose()
}
