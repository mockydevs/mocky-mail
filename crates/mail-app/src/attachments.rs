//! Attachments of received mail: preview inside the app, open in another
//! app, save; and picking files to attach to a new message.
//!
//! Picking, saving and opening go through the [`Host`](crate::Host):
//! desktop uses normal file dialogs and paths; Android's file screens hand
//! back `content://` links instead, so there the host uses the system's own
//! pick, save and open screens, with copies in the cache.

use std::path::{Path, PathBuf};

use mail_core::attachment_policy;
use mail_core::message::{AttachmentInfo, attachment_bytes, safe_file_name};
use mail_core::preview::{self, PreviewKind};

use crate::Ctx;
use crate::error::{AppError, AppResult};
use crate::host::{FilePick, SaveChoice};
use crate::mail::DraftAttachment;
use crate::state::{AppState, lock};
use crate::sync;

/// Largest file that may be attached (`outbox` checks it again at send).
pub const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;

/// Cache folders holding copies made for opening or saving. Emptied when
/// the app starts; files picked to attach live elsewhere because queued
/// messages may still need them.
const OPENED_DIR: &str = "opened";
/// Cache folder holding attachments copied out for a draft.
pub const DRAFT_COPIES_DIR: &str = "attachments";

async fn load(
    state: &AppState,
    message_id: &str,
    index: usize,
) -> AppResult<(AttachmentInfo, Vec<u8>)> {
    let raw = sync::body_now(state, message_id).await?;
    // An encrypted message's attachments are inside what is decrypted.
    let shown = crate::security::shown_bytes(state, message_id, raw)?;
    attachment_bytes(&shown, index)?
        .ok_or_else(|| AppError::new("not_found", "That attachment no longer exists."))
}

/// Write a copy to `cache/opened/<slot>/<name>` and return its path.
async fn write_copy(
    state: &AppState,
    slot: &str,
    info: &AttachmentInfo,
    bytes: &[u8],
) -> AppResult<PathBuf> {
    let dir = state.cache_dir.join(OPENED_DIR).join(safe_file_name(slot));
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(&info.file_name);
    tokio::fs::write(&path, bytes).await?;
    Ok(path)
}

/// Empty the copies left from the last run. Called once at start-up.
///
/// That includes the attachments copied out for a draft (a forward, an
/// edited Outbox message): queued messages carry their own bytes, and the
/// list of files a draft may attach starts empty on every run, so nothing
/// can still use them. Left alone they pile up, including for accounts
/// that have since been removed.
pub fn clear_opened(cache_dir: &Path) {
    let _ = std::fs::remove_dir_all(cache_dir.join(OPENED_DIR));
    let _ = std::fs::remove_dir_all(cache_dir.join(DRAFT_COPIES_DIR));
}

/// `preview_attachment`: the attachment's bytes, for the in-app preview.
/// Refused (`invalid`) unless the file really is a photo, PDF or plain
/// text, whatever its name says. The interface receives the bytes as they
/// are (an `ArrayBuffer`), not as JSON.
pub async fn preview_attachment(ctx: &Ctx, message_id: String, index: usize) -> AppResult<Vec<u8>> {
    let state = &ctx.state;
    let (info, bytes) = load(state, &message_id, index).await?;
    let kind = preview::preview_kind(&info.file_name, &info.mime_type, info.size);
    match kind {
        Some(kind) if preview::content_is(kind, &bytes) => Ok(bytes),
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
    })?;
    if attachment_policy::opens_with_care(&info.file_name) {
        return Err(AppError::invalid(format!(
            "{} can run code or show a web page when opened, so Mocky Mail won't open it from the message. Save it, and open it yourself only if you trust the sender.",
            info.file_name
        )));
    }
    Ok(())
}

/// `open_attachment`: open the file in the app the device uses for its
/// type (PDF reader, Word, photo viewer).
pub async fn open_attachment(ctx: &Ctx, message_id: String, index: usize) -> AppResult<()> {
    let state = &ctx.state;
    let (info, bytes) = load(state, &message_id, index).await?;
    check_openable(&info, &bytes)?;
    let path = write_copy(state, &format!("{message_id}-{index}"), &info, &bytes).await?;
    open_path(ctx, &path, &info).await
}

async fn open_path(ctx: &Ctx, path: &Path, info: &AttachmentInfo) -> AppResult<()> {
    ctx.host
        .open_file(path, &info.file_name, &info.mime_type)
        .await
}

/// `save_attachment`: ask where to save it, then write it there. The place
/// it was saved, or `None` if the user cancelled.
pub async fn save_attachment(
    ctx: &Ctx,
    message_id: String,
    index: usize,
) -> AppResult<Option<String>> {
    let state = &ctx.state;
    let (info, bytes) = load(state, &message_id, index).await?;
    save(ctx, state, &format!("{message_id}-{index}"), &info, bytes).await
}

/// Save a file the user asked for: where the save dialog says, or (Android)
/// through a copy in the cache and the system's "Save to" screen. Where (or
/// under what name) it was saved, or `None` if cancelled.
pub(crate) async fn save(
    ctx: &Ctx,
    state: &AppState,
    slot: &str,
    info: &AttachmentInfo,
    bytes: Vec<u8>,
) -> AppResult<Option<String>> {
    match ctx.host.save_dialog(&info.file_name).await? {
        SaveChoice::Cancelled => Ok(None),
        SaveChoice::Path(path) => {
            tokio::fs::write(&path, bytes).await?;
            Ok(Some(path.display().to_string()))
        }
        SaveChoice::AfterWrite => {
            let path = write_copy(state, slot, info, &bytes).await?;
            ctx.host
                .save_from_cache(&path, &info.file_name, &info.mime_type)
                .await
        }
    }
}

/// `pick_attachments`: the open-file dialog. Only files picked here may be
/// attached to a message.
pub async fn pick_attachments(ctx: &Ctx) -> AppResult<Vec<DraftAttachment>> {
    let state = &ctx.state;
    let mut out = Vec::new();
    for path in pick_paths(ctx).await? {
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
        // Checked before reading: a disc image or a long video must not
        // be loaded into memory only to be refused afterwards.
        if metadata.len() > MAX_ATTACHMENT_BYTES {
            return Err(AppError::new(
                "too_large",
                format!("{file_name} is too large to attach. The limit is 100 MB."),
            ));
        }
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

/// The files picked to attach. Android's picker copies each one into the
/// app's cache.
async fn pick_paths(ctx: &Ctx) -> AppResult<Vec<PathBuf>> {
    ctx.host
        .pick_files(FilePick {
            filter: None,
            many: true,
            max_bytes: MAX_ATTACHMENT_BYTES,
            not_local: "That file can't be attached. Pick a file on this computer.",
        })
        .await
}

/// The preview offered for an attachment in the reading pane.
#[must_use]
pub fn preview_for(info: &AttachmentInfo) -> Option<PreviewKind> {
    preview::preview_kind(&info.file_name, &info.mime_type, info.size)
}

/// Choose one picture (signature images). `None` if cancelled.
pub async fn pick_image(ctx: &Ctx) -> AppResult<Option<PathBuf>> {
    Ok(ctx
        .host
        .pick_files(FilePick {
            filter: Some(("Pictures", &["png", "jpg", "jpeg", "gif"])),
            many: false,
            // A little over the 1 MB limit, so a too-big file gets the clear message.
            max_bytes: 2 * 1024 * 1024,
            not_local: "Pick a picture on this computer.",
        })
        .await?
        .into_iter()
        .next())
}
