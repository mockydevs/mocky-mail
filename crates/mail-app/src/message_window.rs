//! A message in a window of its own (computers). The window loads the app
//! itself with `?message=<id>`, which the interface shows as that one
//! message; it has the main window's permissions (`capabilities`, windows
//! `message-*`) and the same link rules.

use std::fmt::Write as _;

use crate::Ctx;
use crate::error::{AppError, AppResult};
use crate::host::WindowSpec;
use crate::state::AppState;

/// The window's label: `message-` and the id, with anything a label can't
/// hold made `_`.
fn label_for(id: &str) -> String {
    let safe: String = id
        .chars()
        .take(100)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("message-{safe}")
}

/// The id as a URL query value (percent-encoded UTF-8).
fn query_value(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// The window title: the subject.
fn window_title(subject: &str) -> String {
    let subject: String = subject.trim().chars().take(200).collect();
    if subject.is_empty() {
        "(no subject)".to_owned()
    } else {
        subject
    }
}

/// `open_message_window`: the message in its own window, or the one
/// already showing it brought to the front. Phones reject it (`invalid`).
// Async so the window is made off the main thread: on Windows a
// synchronous command that creates a webview can deadlock.
#[allow(clippy::unused_async)]
pub async fn open_message_window(ctx: &Ctx, id: String) -> AppResult<()> {
    let state = &ctx.state;
    open(ctx, state, &id)
}

fn open(ctx: &Ctx, state: &AppState, id: &str) -> AppResult<()> {
    if !ctx.host.desktop() {
        return Err(AppError::invalid(
            "On a phone, messages open full screen in the app.",
        ));
    }
    let subject = state.store.get_message(id)?.item.subject;
    ctx.host
        .open_window(&WindowSpec {
            label: label_for(id),
            url: format!("index.html?message={}", query_value(id)),
            title: window_title(&subject),
            size: (900.0, 700.0),
            min_size: (480.0, 360.0),
        })
        .map_err(|e| AppError::internal(format!("Couldn't open the message window: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_urls_are_safe() {
        assert_eq!(
            label_for("0123456789abcdef0123456789abcdef"),
            "message-0123456789abcdef0123456789abcdef"
        );
        assert_eq!(label_for("a/b c?é"), "message-a_b_c__");
        assert_eq!(query_value("abc-123"), "abc-123");
        assert_eq!(query_value("a b&c=é"), "a%20b%26c%3D%C3%A9");
        assert_eq!(window_title("  Quote  "), "Quote");
        assert_eq!(window_title(""), "(no subject)");
    }
}
