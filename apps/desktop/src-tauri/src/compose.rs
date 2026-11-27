//! Compose helpers: Check Names, contact cards and messages as
//! attachments, and compose windows of their own (computers).
//!
//! A compose window loads the app with `?compose=<key>`; the draft it
//! continues waits here under that key until the window takes it. Compose
//! windows (`compose-*`) have the main window's permissions and link rules,
//! like message windows.

use std::fmt::Write as _;
use std::sync::Arc;

use mail_core::compose::RecipientCheck;
use mail_store::ContactState;
use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::mail::DraftAttachment;
use crate::outbox::Draft;
use crate::state::{AppState, lock};
use crate::sync;

/// Longest recipient field Check Names reads.
const MAX_FIELD_BYTES: usize = 64 * 1024;
/// Most drafts waiting for their compose window.
#[cfg_attr(mobile, allow(dead_code))]
const MAX_WAITING_DRAFTS: usize = 32;

/// `resolve_recipients`: Check Names for one recipient field.
#[tauri::command]
pub fn resolve_recipients(
    state: State<'_, Arc<AppState>>,
    text: String,
) -> AppResult<RecipientCheck> {
    if text.len() > MAX_FIELD_BYTES {
        return Err(AppError::invalid("There are too many recipients to check."));
    }
    let name_for = |address: &str| -> Option<String> {
        let contacts = state
            .store
            .list_contacts(address, mail_store::MAX_SUGGESTIONS)
            .unwrap_or_default();
        contacts
            .into_iter()
            .find(|c| {
                c.card
                    .emails
                    .iter()
                    .any(|e| e.trim().eq_ignore_ascii_case(address))
            })
            .map(|c| c.card.name)
            .filter(|n| !n.trim().is_empty())
            .or_else(|| {
                crate::contacts::suggestions(&state, address, mail_store::MAX_SUGGESTIONS)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|m| m.address.eq_ignore_ascii_case(address))
                    .and_then(|m| m.name)
            })
    };
    let search = |query: &str| {
        crate::contacts::suggestions(&state, query, mail_store::MAX_SUGGESTIONS).unwrap_or_default()
    };
    let group = |name: &str| {
        crate::contacts::group_recipients(&state, name)
            .ok()
            .flatten()
    };
    Ok(mail_core::compose::resolve_recipients_with_groups(
        &text, group, name_for, search,
    ))
}

/// The `.vcf` file name for a contact.
fn vcf_name(name: &str, email: Option<&str>) -> String {
    let base = if name.trim().is_empty() {
        email.unwrap_or_default()
    } else {
        name
    };
    let base: String = base.trim().chars().take(80).collect();
    let spaced = base.replace([':', '*', '?', '"', '<', '>', '|', '/', '\\'], " ");
    let base = mail_core::message::safe_file_name(
        &spaced.split_whitespace().collect::<Vec<_>>().join(" "),
    );
    let base = if base == "attachment" || base.trim().is_empty() {
        "contact".to_owned()
    } else {
        base
    };
    format!("{base}.vcf")
}

/// `contact_card_attachment`: a contact as a vCard 3.0 `.vcf` file to
/// attach.
#[tauri::command]
pub async fn contact_card_attachment(
    state: State<'_, Arc<AppState>>,
    contact_id: String,
) -> AppResult<DraftAttachment> {
    let contact = state.store.get_contact(&contact_id)?;
    if contact.state == ContactState::Deleted {
        return Err(AppError::new("not_found", "That contact no longer exists."));
    }
    let card = mail_core::vcard::write(&contact.card, None);
    let file_name = vcf_name(
        &contact.card.name,
        contact.card.emails.first().map(String::as_str),
    );
    crate::outbox::cache_for_draft(
        &state,
        &format!("contact-{contact_id}"),
        &file_name,
        card.as_bytes(),
    )
    .await
}

/// `message_as_attachment`: a message, whole, as an `.eml` file to attach
/// (sent as `message/rfc822`).
#[tauri::command]
pub async fn message_as_attachment(
    state: State<'_, Arc<AppState>>,
    message_id: String,
) -> AppResult<DraftAttachment> {
    let raw = sync::body_now(&state, &message_id).await?;
    if u64::try_from(raw.len()).unwrap_or(u64::MAX) > crate::attachments::MAX_ATTACHMENT_BYTES {
        return Err(AppError::new(
            "too_large",
            "This message is too large to attach.",
        ));
    }
    let subject = state.store.get_message(&message_id)?.item.subject;
    let file_name = crate::message_tools::eml_name(&subject);
    crate::outbox::cache_for_draft(&state, &format!("eml-{message_id}"), &file_name, &raw).await
}

/// A key nobody can guess, for a waiting draft.
#[cfg_attr(mobile, allow(dead_code))]
fn new_key() -> AppResult<String> {
    use ring::rand::SecureRandom as _;
    let mut bytes = [0u8; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| AppError::internal("No randomness available."))?;
    let mut key = String::with_capacity(32);
    for byte in bytes {
        let _ = write!(key, "{byte:02x}");
    }
    Ok(key)
}

/// The compose window's title: the subject, or "New message".
#[cfg_attr(mobile, allow(dead_code))]
fn window_title(subject: &str) -> String {
    let subject: String = subject.trim().chars().take(200).collect();
    if subject.is_empty() {
        "New message".to_owned()
    } else {
        subject
    }
}

/// Keep a draft for its window; the oldest waiting one goes if too many
/// are waiting (a window that never opened).
#[cfg_attr(mobile, allow(dead_code))]
fn keep(state: &AppState, key: &str, draft: Draft) {
    let mut waiting = lock(&state.compose_drafts);
    while waiting.len() >= MAX_WAITING_DRAFTS {
        waiting.remove(0);
    }
    waiting.push((key.to_owned(), draft));
}

fn take(state: &AppState, key: &str) -> Option<Draft> {
    let mut waiting = lock(&state.compose_drafts);
    let at = waiting.iter().position(|(k, _)| k == key)?;
    Some(waiting.remove(at).1)
}

/// `open_compose_window`: continue writing this draft in a window of its
/// own. Phones reject it (`invalid`).
// Async so the window is made off the main thread: on Windows a
// synchronous command that creates a webview can deadlock.
#[allow(clippy::unused_async)]
#[tauri::command]
pub async fn open_compose_window(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    draft: Draft,
) -> AppResult<()> {
    open(&app, &state, draft)
}

#[cfg(desktop)]
fn open(app: &AppHandle, state: &AppState, draft: Draft) -> AppResult<()> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    let key = new_key()?;
    let title = window_title(draft.subject());
    keep(state, &key, draft);
    let url = WebviewUrl::App(format!("index.html?compose={key}").into());
    let built = crate::guarded(
        WebviewWindowBuilder::new(app, format!("compose-{key}"), url),
        app,
    )
    .title(title)
    .inner_size(900.0, 760.0)
    .min_inner_size(520.0, 420.0)
    .build();
    if let Err(error) = built {
        take(state, &key);
        return Err(AppError::internal(format!(
            "Couldn't open the compose window: {error}"
        )));
    }
    Ok(())
}

#[cfg(mobile)]
fn open(_app: &AppHandle, _state: &AppState, _draft: Draft) -> AppResult<()> {
    Err(AppError::invalid(
        "On a phone, messages are written full screen in the app.",
    ))
}

/// `take_compose_draft`: the draft a compose window was opened with, once;
/// `None` after that or for an unknown key.
#[tauri::command]
pub fn take_compose_draft(state: State<'_, Arc<AppState>>, key: String) -> Option<Draft> {
    take(&state, &key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_files_are_named_after_the_contact() {
        assert_eq!(vcf_name("Peter Mwangi", None), "Peter Mwangi.vcf");
        assert_eq!(
            vcf_name("  ", Some("peter@client.example")),
            "peter@client.example.vcf"
        );
        assert_eq!(vcf_name("a/b:c", None), "a b c.vcf");
        assert_eq!(vcf_name("", None), "contact.vcf");
    }

    #[test]
    fn window_keys_and_titles() {
        let a = new_key().unwrap_or_default();
        let b = new_key().unwrap_or_default();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
        assert_eq!(window_title(""), "New message");
        assert_eq!(window_title(" Quote "), "Quote");
    }
}
