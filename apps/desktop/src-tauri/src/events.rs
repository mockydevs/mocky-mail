//! Events pushed to the interface (`docs/ipc.md`, Events).

use mail_core::folders::FolderRole;
use mail_store::{Account, ListFilter, ListQuery, MessageListItem};
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

use crate::state::{AppState, now};
use crate::{outbox, settings};

fn emit(app: &AppHandle, name: &str, payload: serde_json::Value) {
    if let Err(error) = app.emit(name, payload) {
        eprintln!("couldn't emit {name}: {error}");
    }
}

/// Folders of an account changed; the interface refetches.
pub fn changed(app: &AppHandle, account_id: &str, folder_ids: &[String]) {
    emit(
        app,
        "mail://changed",
        json!({ "accountId": account_id, "folderIds": folder_ids }),
    );
}

/// Accounts were added, renamed or removed.
pub fn accounts_changed(app: &AppHandle) {
    emit(app, "mail://accounts-changed", json!({}));
}

/// Sync state of one account.
pub fn sync_status(app: &AppHandle, account_id: &str, state: &str, message: Option<&str>) {
    let last = (state == "idle").then(now);
    emit(
        app,
        "mail://sync-status",
        json!({ "accountId": account_id, "state": state, "lastSyncedAt": last, "message": message }),
    );
}

/// The outbox changed; sends the full list.
pub fn outbox_changed(app: &AppHandle, state: &AppState) {
    match state.store.list_outbox() {
        Ok(entries) => {
            let entries: Vec<_> = entries.iter().map(outbox::dto).collect();
            emit(app, "mail://outbox-changed", json!({ "entries": entries }));
        }
        Err(error) => eprintln!("couldn't list outbox: {error}"),
    }
}

/// What a new-mail notification says: the sender and subject when one
/// message arrived, otherwise the account and how many.
fn notification_text(
    account: &Account,
    count: u32,
    newest: Option<&MessageListItem>,
) -> (String, String) {
    let account_name = if account.display_name.trim().is_empty() {
        account.email.clone()
    } else {
        account.display_name.clone()
    };
    if count == 1
        && let Some(message) = newest
    {
        let sender = message.from.as_ref().map_or_else(
            || account_name.clone(),
            |from| {
                from.name
                    .clone()
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| from.address.clone())
            },
        );
        let subject = if message.subject.trim().is_empty() {
            "(no subject)".to_owned()
        } else {
            message.subject.clone()
        };
        return (sender, subject);
    }
    let body = if count == 1 {
        "1 new message".to_owned()
    } else {
        format!("{count} new messages")
    };
    (account_name, body)
}

/// Linux notification servers read the body as simple HTML (links, images,
/// bold), and the subject comes from whoever sent the email, so `<`, `>` and
/// `&` are escaped there. Other systems show the text as written.
fn notification_body(text: &str) -> String {
    if cfg!(all(unix, not(target_os = "macos"))) {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    } else {
        text.to_owned()
    }
}

/// The newest unread message in the account's inbox, for the notification.
fn newest_unread(state: &AppState, account: &Account) -> Option<MessageListItem> {
    let query = ListQuery {
        account_id: Some(account.id.clone()),
        role: Some(FolderRole::Inbox),
        filter: ListFilter::Unread,
        limit: 1,
        ..ListQuery::default()
    };
    match state.store.list_messages(&query) {
        Ok(page) => page.items.into_iter().next(),
        Err(error) => {
            eprintln!("couldn't read new mail for the notification: {error}");
            None
        }
    }
}

/// New unread mail arrived in an inbox.
pub fn new_mail(app: &AppHandle, state: &AppState, account: &Account, count: u32) {
    emit(
        app,
        "mail://new-mail",
        json!({ "accountId": account.id, "count": count }),
    );
    if settings::load(state).is_ok_and(|s| s.notifications) {
        let newest = if count == 1 {
            newest_unread(state, account)
        } else {
            None
        };
        let (title, body) = notification_text(account, count, newest.as_ref());
        if let Err(error) = app
            .notification()
            .builder()
            .title(title)
            .body(notification_body(&body))
            .show()
        {
            eprintln!("couldn't show notification: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use mail_core::message::Mailbox;
    use mail_store::{AuthKind, Security, ServerConfig};

    use super::*;

    fn account(display_name: &str) -> Account {
        let server = ServerConfig {
            host: "mail.example".into(),
            port: 993,
            security: Security::Tls,
        };
        Account {
            id: "a".into(),
            email: "me@example.com".into(),
            display_name: display_name.into(),
            auth: AuthKind::Password,
            color: "#2450c8".into(),
            imap: server.clone(),
            smtp: server,
            size_limit_bytes: None,
            created_at: 0,
        }
    }

    fn message(from: Option<Mailbox>, subject: &str) -> MessageListItem {
        MessageListItem {
            id: "m".into(),
            account_id: "a".into(),
            folder_id: "a:inbox".into(),
            from,
            to: Vec::new(),
            subject: subject.into(),
            preview: String::new(),
            date: None,
            unread: true,
            flagged: false,
            answered: false,
            has_attachments: false,
        }
    }

    #[test]
    fn one_message_shows_sender_and_subject() {
        let from = Mailbox {
            name: Some("Adobe".into()),
            address: "message@adobe.com".into(),
        };
        let newest = message(Some(from), "Your payment has been declined");
        assert_eq!(
            notification_text(&account("Graphics"), 1, Some(&newest)),
            ("Adobe".into(), "Your payment has been declined".into())
        );
    }

    #[test]
    fn falls_back_to_address_and_no_subject() {
        let from = Mailbox {
            name: Some("  ".into()),
            address: "billing@example.com".into(),
        };
        let newest = message(Some(from), " ");
        assert_eq!(
            notification_text(&account("Graphics"), 1, Some(&newest)),
            ("billing@example.com".into(), "(no subject)".into())
        );
    }

    #[test]
    fn several_messages_show_the_account_and_count() {
        assert_eq!(
            notification_text(&account(""), 3, None),
            ("me@example.com".into(), "3 new messages".into())
        );
        assert_eq!(
            notification_text(&account("Work"), 1, None),
            ("Work".into(), "1 new message".into())
        );
    }

    #[test]
    fn a_subject_cannot_add_markup_to_a_linux_notification() {
        let body = notification_body(
            r#"Invoice <a href="https://x.example">pay</a> & <img src="https://t.example/p">"#,
        );
        if cfg!(all(unix, not(target_os = "macos"))) {
            assert!(!body.contains('<') && !body.contains('>'), "{body}");
            assert!(body.starts_with("Invoice &lt;a href="), "{body}");
            assert!(body.contains("&amp; &lt;img"), "{body}");
        } else {
            assert!(body.contains("<a href="));
        }
    }
}
