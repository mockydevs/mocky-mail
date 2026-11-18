//! Whole-message actions in the reader: save as `.eml`, view source and
//! print.

use std::sync::Arc;

use mail_core::message::{AttachmentInfo, safe_file_name};
use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::sync;

/// Most of a message's source shown in "View source".
const MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;

/// The `.eml` file name for a subject.
fn eml_name(subject: &str) -> String {
    let base: String = subject.trim().chars().take(80).collect();
    // Characters files can't have become spaces, so "Re: a/b" stays readable.
    let spaced = base.replace([':', '*', '?', '"', '<', '>', '|', '/', '\\'], " ");
    let base = safe_file_name(&spaced.split_whitespace().collect::<Vec<_>>().join(" "));
    let base = if base == "attachment" || base.trim().is_empty() {
        "message".to_owned()
    } else {
        base
    };
    format!("{base}.eml")
}

/// `save_message`: save the whole message as an `.eml` file, which mail
/// apps and Outlook open. Where it was saved, or `None` if cancelled.
#[tauri::command]
pub async fn save_message(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    message_id: String,
) -> AppResult<Option<String>> {
    let raw = sync::body_now(&state, &message_id).await?;
    let subject = state.store.get_message(&message_id)?.item.subject;
    let info = AttachmentInfo {
        file_name: eml_name(&subject),
        mime_type: "message/rfc822".to_owned(),
        size: u64::try_from(raw.len()).unwrap_or(u64::MAX),
    };
    crate::attachments::save(&app, &state, &format!("{message_id}-eml"), &info, raw).await
}

/// `message_source`: the message as received (headers and body), as text.
/// Very large messages are cut, with a note.
#[tauri::command]
pub async fn message_source(
    state: State<'_, Arc<AppState>>,
    message_id: String,
) -> AppResult<String> {
    let raw = sync::body_now(&state, &message_id).await?;
    let shown = raw.get(..raw.len().min(MAX_SOURCE_BYTES)).unwrap_or(&raw);
    let mut text = String::from_utf8_lossy(shown).into_owned();
    if raw.len() > MAX_SOURCE_BYTES {
        text.push_str(
            "\n\n[… the rest of this message is not shown. Save it as .eml to see all of it.]",
        );
    }
    Ok(text)
}

/// `print_html`: Android only: print a page (the system print screen,
/// which can also save a PDF). Desktop prints from the page itself.
#[tauri::command]
pub async fn print_html(html: String, title: String) -> AppResult<()> {
    if html.len() > 16 * 1024 * 1024 {
        return Err(AppError::invalid("This message is too large to print."));
    }
    print(&html, &title).await
}

#[cfg(target_os = "android")]
async fn print(html: &str, title: &str) -> AppResult<()> {
    crate::files_android::print_html(html, title).await
}

#[cfg(not(target_os = "android"))]
#[allow(clippy::unused_async)] // async to match the Android version
async fn print(_html: &str, _title: &str) -> AppResult<()> {
    Err(AppError::invalid(
        "Printing is done from the window on this device.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eml_names_are_safe_and_readable() {
        assert_eq!(eml_name("Invoice INV-0142"), "Invoice INV-0142.eml");
        assert_eq!(eml_name("Re: a/b\\c"), "Re a b c.eml");
        assert_eq!(eml_name(""), "message.eml");
        assert_eq!(eml_name("   "), "message.eml");
        assert!(eml_name(&"x".repeat(500)).len() <= 84);
    }
}
