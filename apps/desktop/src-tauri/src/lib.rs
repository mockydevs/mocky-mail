//! Tauri shell for Mocky Mail: opens the database, starts one sync engine
//! per account and the outbox worker, and exposes the commands in
//! `docs/ipc.md`. Mail rules live in `mail-core`; network code in
//! `mail-sync`; storage in `mail-store`.

// Tauri hands commands owned `State`, `AppHandle` and deserialized
// arguments, so taking them by value is required, not wasteful.
#![allow(clippy::needless_pass_by_value)]

mod accounts;
mod convert;
mod credentials;
mod error;
mod events;
mod mail;
mod outbox;
mod settings;
mod state;
mod sync;

use std::sync::Arc;

use mail_sync::secrets::KeyringStore;
use tauri::Manager;

use crate::state::{AppState, now};

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = app.path().app_data_dir()?;
    let cache_dir = app.path().app_cache_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    std::fs::create_dir_all(&cache_dir)?;
    let store = mail_store::Store::open(data_dir.join("mail.db"))?;
    let recovered = store.recover_outbox_after_restart(now())?;
    if recovered > 0 {
        eprintln!(
            "{recovered} message(s) were mid-send when the app stopped; checking Sent before retrying"
        );
    }
    let state = Arc::new(AppState::new(store, Arc::new(KeyringStore), cache_dir));
    app.manage(Arc::clone(&state));

    let handle = app.handle().clone();
    for account in state.store.list_accounts()? {
        sync::start(&handle, &state, &account.id);
    }
    tauri::async_runtime::spawn(outbox::run(handle, state));
    Ok(())
}

/// Build and run the app.
///
/// Tauri reports a failed start (for example, no display available) by
/// returning an error; we print it and exit non-zero instead of panicking.
pub fn run() {
    let result = tauri::Builder::default()
        // Must be first: a second copy would sync and send in parallel.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(setup)
        .invoke_handler(tauri::generate_handler![
            accounts::suggest_servers,
            accounts::list_accounts,
            accounts::add_account,
            accounts::add_oauth_account,
            accounts::update_account,
            accounts::remove_account,
            mail::list_folders,
            mail::list_messages,
            mail::get_message,
            mail::set_flags,
            mail::move_messages,
            mail::delete_forever,
            mail::save_attachment,
            mail::pick_attachments,
            mail::sync_now,
            mail::open_external,
            outbox::check_draft,
            outbox::send_message,
            outbox::save_draft,
            outbox::list_outbox,
            outbox::outbox_send_now,
            outbox::outbox_cancel,
            outbox::outbox_edit,
            settings::get_settings,
            settings::set_settings,
        ])
        .run(tauri::generate_context!());

    if let Err(error) = result {
        eprintln!("Mocky Mail could not start: {error}");
        std::process::exit(1);
    }
}
