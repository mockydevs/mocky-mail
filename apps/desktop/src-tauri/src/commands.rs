//! IPC commands the interface may call.
//!
//! Each command is a thin, synchronous wrapper around `mail-core`. Inputs
//! from the webview are size-checked here before any work is done, so a
//! compromised or buggy page cannot make the backend parse gigabytes.

// Tauri deserializes command arguments into owned values, so taking them by
// value is required, not wasteful.
#![allow(clippy::needless_pass_by_value)]

use mail_core::limits::{self, TooLarge};
use mail_core::message::{self, MessageSummary};
use mail_core::sanitize::{self, RemoteContent, SanitizedHtml};

/// Largest HTML body accepted for sanitizing (10 MiB).
pub const MAX_HTML_BYTES: usize = 10 * 1024 * 1024;
/// Most attachments accepted in one size check.
pub const MAX_ATTACHMENTS: usize = 500;

/// Clean an HTML email body for the sandboxed viewer.
///
/// # Errors
/// Refuses bodies larger than [`MAX_HTML_BYTES`].
#[tauri::command]
pub fn sanitize_email_html(html: String, allow_remote: bool) -> Result<SanitizedHtml, String> {
    if html.len() > MAX_HTML_BYTES {
        return Err(format!(
            "HTML body is {} bytes; the limit is {MAX_HTML_BYTES}",
            html.len()
        ));
    }
    let remote = if allow_remote {
        RemoteContent::Allow
    } else {
        RemoteContent::Block
    };
    Ok(sanitize::sanitize_html(&html, remote))
}

/// Read a raw message (for example an imported `.eml` file).
///
/// # Errors
/// Returns a readable message for empty, oversized or unreadable input.
#[tauri::command]
pub fn parse_message(raw: Vec<u8>) -> Result<MessageSummary, String> {
    message::parse_summary(&raw).map_err(|error| error.to_string())
}

/// Check a draft against the account's size limit before it is queued.
///
/// # Errors
/// Returns [`TooLarge`] when the draft will not fit, or a refusal when the
/// request itself is unreasonable.
#[tauri::command]
pub fn check_message_size(
    body_len: u64,
    attachment_sizes: Vec<u64>,
    limit: u64,
) -> Result<u64, SizeCheckError> {
    if attachment_sizes.len() > MAX_ATTACHMENTS {
        return Err(SizeCheckError::TooManyAttachments {
            count: attachment_sizes.len(),
            max: MAX_ATTACHMENTS,
        });
    }
    limits::check_message_size(body_len, &attachment_sizes, limit).map_err(SizeCheckError::TooLarge)
}

/// Why a size check failed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SizeCheckError {
    /// The draft is larger than the server accepts.
    TooLarge(TooLarge),
    /// More attachments than we accept in one request.
    TooManyAttachments {
        /// Attachments sent.
        count: usize,
        /// The maximum.
        max: usize,
    },
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_blocks_remote_by_default_flag() {
        let out =
            sanitize_email_html(r#"<img src="https://t.example/p.gif">"#.into(), false).unwrap();
        assert_eq!(out.blocked_remote_images, 1);
    }

    #[test]
    fn sanitize_refuses_huge_bodies() {
        let huge = "a".repeat(MAX_HTML_BYTES + 1);
        assert!(sanitize_email_html(huge, true).is_err());
    }

    #[test]
    fn parse_reports_errors_as_text() {
        assert_eq!(
            parse_message(Vec::new()),
            Err("message is empty".to_owned())
        );
    }

    #[test]
    fn size_check_limits_attachment_count() {
        let result = check_message_size(0, vec![1; MAX_ATTACHMENTS + 1], u64::MAX);
        assert!(matches!(
            result,
            Err(SizeCheckError::TooManyAttachments { .. })
        ));
    }

    #[test]
    fn size_check_passes_small_drafts() {
        assert!(check_message_size(100, vec![1_000], 25 * 1024 * 1024).is_ok());
    }
}
