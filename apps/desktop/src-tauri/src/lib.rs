//! Tauri shell for Mocky Mail: opens the database, starts one sync engine
//! per account and the outbox worker, and exposes the commands in
//! `docs/ipc.md`. Mail rules live in `mail-core`; network code in
//! `mail-sync`; storage in `mail-store`.

// Tauri hands commands owned `State`, `AppHandle` and deserialized
// arguments, so taking them by value is required, not wasteful.
#![allow(clippy::needless_pass_by_value)]

mod accounts;
mod attachments;
mod calendar;
mod contacts;
mod convert;
mod credentials;
mod error;
mod events;
#[cfg(target_os = "android")]
mod files_android;
mod folders;
#[cfg(target_os = "android")]
mod google_android;
mod invitations;
mod mail;
mod mailto;
mod message_tools;
mod out_of_office;
mod outbox;
mod reminders;
mod settings;
mod signatures;
mod state;
mod sync;
#[cfg(desktop)]
mod tray;
mod updates;

use std::sync::Arc;

use mail_sync::secrets::KeyringStore;
#[cfg(desktop)]
use tauri::Emitter;
use tauri::webview::NewWindowResponse;
use tauri::{Manager, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;

use crate::state::{AppState, now};

/// Whether a URL is the app itself (bundled pages, dev server, iframes).
fn is_app_url(url: &Url) -> bool {
    is_app_url_in(url, cfg!(debug_assertions))
}

/// `dev_server`: also allow the Vite dev server on this computer. Only
/// debug builds use it; a release build must not navigate to whatever
/// happens to be listening on a local port.
fn is_app_url_in(url: &Url, dev_server: bool) -> bool {
    match url.scheme() {
        "tauri" | "about" => true,
        "http" | "https" => match url.host_str() {
            // The bundled pages on Windows (WebView2).
            Some("tauri.localhost") => true,
            Some("localhost" | "127.0.0.1") => dev_server,
            _ => false,
        },
        _ => false,
    }
}

/// Send web and mail links to the user's own apps; drop anything else.
fn open_outside<R: tauri::Runtime>(app: &tauri::AppHandle<R>, url: &Url) {
    if matches!(url.scheme(), "http" | "https" | "mailto")
        && let Err(error) = app.opener().open_url(url.as_str(), None::<&str>)
    {
        eprintln!("couldn't open link: {error}");
    }
}

fn create_window(app: &tauri::App) -> tauri::Result<()> {
    let for_popups = app.handle().clone();
    let for_navigation = app.handle().clone();
    let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::default());
    // Phones have one full-screen window; sizes and titles are desktop ideas.
    #[cfg(desktop)]
    let builder = builder
        .title("Mocky Mail")
        .inner_size(1280.0, 820.0)
        .min_inner_size(900.0, 600.0)
        // Started at login: wait in the tray.
        .visible(!tray::start_hidden());
    builder
        // Link clicks in email arrive here; nothing opens inside the app.
        .on_new_window(move |url, _features| {
            open_outside(&for_popups, &url);
            NewWindowResponse::Deny
        })
        // The app window never leaves the app.
        .on_navigation(move |url| {
            if is_app_url(url) {
                return true;
            }
            open_outside(&for_navigation, url);
            false
        })
        .build()?;
    Ok(())
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // Android: certificate checks use the phone's trust store. Must run
    // before any connection.
    android_bridge::init_tls()?;
    create_window(app)?;
    let data_dir = app.path().app_data_dir()?;
    let cache_dir = app.path().app_cache_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    std::fs::create_dir_all(&cache_dir)?;
    attachments::clear_opened(&cache_dir);
    let store = mail_store::Store::open(data_dir.join("mail.db"))?;
    let recovered = store.recover_outbox_after_restart(now())?;
    if recovered > 0 {
        eprintln!(
            "{recovered} message(s) were mid-send when the app stopped; checking Sent before retrying"
        );
    }
    let state = Arc::new(AppState::new(store, Arc::new(KeyringStore), cache_dir));
    app.manage(Arc::clone(&state));
    #[cfg(desktop)]
    tray::build(app);

    let handle = app.handle().clone();
    for account in state.store.list_accounts()? {
        sync::start(&handle, &state, &account.id);
    }
    contacts::start(handle.clone(), Arc::clone(&state));
    calendar::start(handle.clone(), Arc::clone(&state));
    reminders::start(handle.clone(), Arc::clone(&state));
    tauri::async_runtime::spawn(outbox::run(handle, state));
    Ok(())
}

/// Build and run the app.
///
/// Tauri reports a failed start (for example, no display available) by
/// returning an error; we print it and exit non-zero instead of panicking.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    mailto::remember_launch_link();
    let builder = tauri::Builder::default();
    // Must be first: a second copy would sync and send in parallel.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
        // Opening it again brings the window back, even from the tray.
        tray::show(app);
        // A mailto: link opened while the app runs: start a message.
        if let Some(url) = mailto::from_args(args.iter().map(String::as_str)) {
            let _ = app.emit("mail://mailto", serde_json::json!({ "url": url }));
        }
    }));
    // Android: Gmail sign-in with the phone's own Google accounts.
    #[cfg(target_os = "android")]
    let builder = builder.plugin(google_android::init());
    // Android: pick, save and open files through the phone's own screens.
    #[cfg(target_os = "android")]
    let builder = builder.plugin(files_android::init());
    // Start at login, into the tray.
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![tray::HIDDEN_ARG]),
        ))
        .on_window_event(tray::on_window_event);
    let result = builder
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
            folders::create_folder,
            folders::rename_folder,
            folders::delete_folder,
            mail::list_messages,
            mail::get_thread,
            mail::set_focus_rule,
            mail::preview_body,
            mailto::take_mailto,
            mailto::parse_mailto,
            mail::get_message,
            mail::set_flags,
            mail::move_messages,
            mail::suggest_recipients,
            mail::delete_forever,
            attachments::save_attachment,
            attachments::pick_attachments,
            attachments::preview_attachment,
            attachments::open_attachment,
            message_tools::save_message,
            message_tools::message_source,
            message_tools::print_html,
            mail::sync_now,
            mail::open_external,
            outbox::check_draft,
            outbox::send_message,
            outbox::save_draft,
            outbox::attachments_for_draft,
            outbox::list_outbox,
            outbox::outbox_send_now,
            outbox::outbox_cancel,
            outbox::outbox_edit,
            settings::get_settings,
            settings::set_settings,
            contacts::list_contacts,
            contacts::get_contact,
            contacts::save_contact,
            contacts::delete_contact,
            contacts::sync_contacts,
            calendar::list_calendars,
            calendar::list_events,
            calendar::get_event,
            calendar::save_event,
            calendar::delete_event,
            calendar::sync_calendars,
            invitations::answer_invitation,
            out_of_office::get_out_of_office,
            out_of_office::set_out_of_office,
            invitations::remove_cancelled_event,
            invitations::apply_invitation_reply,
            signatures::get_signature,
            signatures::list_signatures,
            signatures::save_signature,
            signatures::delete_signature,
            signatures::set_signature_defaults,
            signatures::set_signature_image,
            signatures::set_signature_image_width,
            signatures::remove_signature_image,
            updates::check_for_update,
            updates::install_update,
        ])
        .run(tauri::generate_context!());

    if let Err(error) = result {
        eprintln!("Mocky Mail could not start: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s)
            .unwrap_or_else(|_| Url::parse("about:invalid").unwrap_or_else(|_| unreachable!()))
    }

    #[test]
    fn only_the_app_itself_may_load_in_the_window() {
        assert!(is_app_url(&url("tauri://localhost/index.html")));
        assert!(is_app_url(&url("http://tauri.localhost/")));
        assert!(is_app_url_in(&url("http://localhost:5173/"), true));
        assert!(is_app_url(&url("about:srcdoc")));
        for outside in [
            "https://evil.example/",
            "http://localhost.evil.example/",
            "file:///etc/passwd",
            "mailto:a@b.co",
        ] {
            assert!(!is_app_url(&url(outside)), "{outside}");
        }
    }

    #[test]
    fn a_release_build_does_not_open_local_web_servers() {
        for local in ["http://localhost:5173/", "http://127.0.0.1:8080/x"] {
            assert!(!is_app_url_in(&url(local), false), "{local}");
        }
        assert!(is_app_url_in(&url("tauri://localhost/index.html"), false));
        assert!(is_app_url_in(&url("http://tauri.localhost/"), false));
        assert!(is_app_url_in(&url("https://tauri.localhost/"), false));
    }
}
