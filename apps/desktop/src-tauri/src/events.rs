//! Events pushed to the interface (`docs/ipc.md`, Events).

use mail_store::Account;
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

/// New unread mail arrived in an inbox.
pub fn new_mail(app: &AppHandle, state: &AppState, account: &Account, count: u32) {
    emit(
        app,
        "mail://new-mail",
        json!({ "accountId": account.id, "count": count }),
    );
    if settings::load(state).is_ok_and(|s| s.notifications) {
        let body = if count == 1 {
            "1 new message".to_owned()
        } else {
            format!("{count} new messages")
        };
        if let Err(error) = app
            .notification()
            .builder()
            .title(&account.display_name)
            .body(body)
            .show()
        {
            eprintln!("couldn't show notification: {error}");
        }
    }
}
