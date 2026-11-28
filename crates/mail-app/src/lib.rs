//! Mocky Mail's application layer: the shared state, the background workers
//! (sync, outbox, reminders, calendars, contacts, tasks, server rules) and
//! the body of every command in `docs/ipc.md`, as plain functions taking a
//! [`Ctx`]. Nothing here depends on Tauri: whatever is platform-specific
//! (events, dialogs, notifications, windows) goes through the [`Host`]
//! trait, so the desktop and Android app, the command line, the MCP server
//! and the browser version all run the same logic.
//!
//! [`registry::dispatch`] runs any command by name with JSON arguments.
//!
//! Mail rules live in `mail-core`; network code in `mail-sync`; storage in
//! `mail-store`.

// The commands keep the argument types the interface sends (owned strings
// and values), so the Tauri wrappers pass them straight through.
#![allow(clippy::needless_pass_by_value)]
// Every command returns `AppError`, whose kinds `docs/ipc.md` lists.
#![allow(clippy::missing_errors_doc)]

pub mod accounts;
pub mod attachments;
pub mod calendar;
pub mod compose;
pub mod connections;
pub mod contacts;
pub mod convert;
pub mod credentials;
pub mod ctx;
pub mod error;
pub mod events;
pub mod ews_sync;
pub mod files;
pub mod folders;
pub mod host;
pub mod invitations;
pub mod mail;
pub mod mailto;
pub mod message_tools;
pub mod message_window;
pub mod opened;
pub mod organize;
pub mod out_of_office;
pub mod outbox;
pub mod pst;
pub mod read_receipts;
pub mod registry;
pub mod reminders;
pub mod rules;
pub mod security;
pub mod server_rules;
pub mod settings;
pub mod signatures;
pub mod state;
pub mod sync;
pub mod tasks;
pub mod updates;

use std::path::Path;
use std::sync::Arc;

use mail_sync::secrets::SecretStore;

pub use crate::ctx::Ctx;
pub use crate::error::{AppError, AppResult};
pub use crate::host::Host;
pub use crate::state::AppState;

/// Open the app's state: the database in `data_dir`, temporary files in
/// `cache_dir` (both made if missing; the copies left in the cache by the
/// last run are removed). Messages that were mid-send when the app stopped
/// are checked against Sent before they are retried.
///
/// # Errors
/// When a folder can't be made or the database can't be opened.
pub fn open(
    data_dir: &Path,
    cache_dir: &Path,
    secrets: Arc<dyn SecretStore>,
) -> Result<AppState, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(data_dir)?;
    std::fs::create_dir_all(cache_dir)?;
    attachments::clear_opened(cache_dir);
    let store = mail_store::Store::open(data_dir.join("mail.db"))?;
    let recovered = store.recover_outbox_after_restart(state::now())?;
    if recovered > 0 {
        eprintln!(
            "{recovered} message(s) were mid-send when the app stopped; checking Sent before retrying"
        );
    }
    Ok(AppState::new(store, secrets, cache_dir.to_path_buf()))
}

/// Start the background work: one sync engine per account, contacts,
/// calendars, reminders, rules kept on mail servers, and the outbox.
///
/// # Errors
/// When the accounts can't be read.
pub fn start(ctx: &Ctx) -> Result<(), mail_store::StoreError> {
    for account in ctx.state.store.list_accounts()? {
        sync::start(ctx, &ctx.state, &account.id);
    }
    contacts::start(ctx.clone(), Arc::clone(&ctx.state));
    calendar::start(ctx.clone(), Arc::clone(&ctx.state));
    reminders::start(ctx.clone(), Arc::clone(&ctx.state));
    server_rules::start(ctx);
    ctx.spawn(outbox::run(ctx.clone(), Arc::clone(&ctx.state)));
    Ok(())
}
