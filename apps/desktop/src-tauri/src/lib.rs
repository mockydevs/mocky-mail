//! Tauri shell for Mocky Mail: the windows, the tray, the Android plugins,
//! and [`host::TauriHost`], through which `mail-app` (the state, the
//! background workers and every command) reaches the platform. The
//! commands in `docs/ipc.md` are thin wrappers in [`commands`]. Mail rules
//! live in `mail-core`; network code in `mail-sync`; storage in
//! `mail-store`.

// Tauri hands commands owned `State`, `AppHandle` and deserialized
// arguments, so taking them by value is required, not wasteful.
#![allow(clippy::needless_pass_by_value)]

mod commands;
#[cfg(target_os = "android")]
mod files_android;
#[cfg(target_os = "android")]
mod google_android;
mod host;
#[cfg(desktop)]
mod tray;

use std::sync::Arc;

use mail_app::{Ctx, mailto, opened};
use mail_sync::secrets::KeyringStore;
#[cfg(desktop)]
use tauri::Emitter;
use tauri::webview::NewWindowResponse;
use tauri::{Manager, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;

use crate::host::TauriHost;

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

/// Every app window: link clicks in email open in the user's own apps,
/// nothing opens inside the app, and the window never leaves the app.
pub(crate) fn guarded<'a, R: tauri::Runtime, M: Manager<R>>(
    builder: WebviewWindowBuilder<'a, R, M>,
    app: &tauri::AppHandle<R>,
) -> WebviewWindowBuilder<'a, R, M> {
    let for_popups = app.clone();
    let for_navigation = app.clone();
    builder
        .on_new_window(move |url, _features| {
            open_outside(&for_popups, &url);
            NewWindowResponse::Deny
        })
        .on_navigation(move |url| {
            if is_app_url(url) {
                return true;
            }
            open_outside(&for_navigation, url);
            false
        })
}

fn create_window(app: &tauri::App) -> tauri::Result<()> {
    let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::default());
    // Phones have one full-screen window; sizes and titles are desktop ideas.
    #[cfg(desktop)]
    let builder = builder
        .title("Mocky Mail")
        .inner_size(1280.0, 820.0)
        .min_inner_size(900.0, 600.0)
        // Started at login: wait in the tray.
        .visible(!tray::start_hidden());
    guarded(builder, app.handle()).build()?;
    Ok(())
}

/// Say so in a window when the app can't start. Started from the Start
/// menu or a launcher there is no console, so the reason printed above
/// would otherwise go nowhere and the app would just never appear.
#[cfg(desktop)]
fn startup_failed(reason: &str) {
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Mocky Mail could not start")
        .set_description(format!(
            "{reason}\n\nIf this keeps happening, restart the computer, then reinstall Mocky Mail from mailer.co.ke. Your mail on the server is not affected."
        ))
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

#[cfg(not(desktop))]
fn startup_failed(_reason: &str) {}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // Android: certificate checks use the phone's trust store. Must run
    // before any connection.
    android_bridge::init_tls()?;
    create_window(app)?;
    let data_dir = app.path().app_data_dir()?;
    let cache_dir = app.path().app_cache_dir()?;
    let state = Arc::new(mail_app::open(
        &data_dir,
        &cache_dir,
        Arc::new(KeyringStore),
    )?);
    let ctx = Ctx::new(
        state,
        Arc::new(TauriHost::new(app.handle().clone())),
        tauri::async_runtime::handle().inner().clone(),
    );
    app.manage(ctx.clone());
    #[cfg(desktop)]
    tray::build(app);

    mail_app::start(&ctx)?;
    // Android: files shared to the app or opened with it.
    #[cfg(target_os = "android")]
    files_android::watch_shared(app.handle().clone());
    Ok(())
}

/// Build and run the app.
///
/// Tauri reports a failed start (for example, no display available) by
/// returning an error; we print it and exit non-zero instead of panicking.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::too_many_lines, reason = "mostly the list of commands")]
pub fn run() {
    mailto::remember_launch_link();
    opened::remember_launch_files();
    let builder = tauri::Builder::default();
    // Must be first: a second copy would sync and send in parallel.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
        // Opening it again brings the window back, even from the tray.
        tray::show(app);
        // A mailto: link opened while the app runs: start a message.
        if let Some(url) = mailto::from_args(args.iter().map(String::as_str)) {
            let _ = app.emit("mail://mailto", serde_json::json!({ "url": url }));
        }
        // A mail, calendar or contact file opened while the app runs.
        let cwd = std::path::PathBuf::from(cwd);
        let host = TauriHost::new(app.clone());
        for path in opened::from_args(args.iter().skip(1).map(String::as_str), &cwd) {
            opened::deliver(Some(&host), &path, None, None);
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
            commands::suggest_servers,
            commands::list_accounts,
            commands::add_account,
            commands::add_oauth_account,
            commands::update_account,
            commands::reconnect_account,
            commands::remove_account,
            commands::list_folders,
            commands::create_folder,
            commands::rename_folder,
            commands::delete_folder,
            commands::list_messages,
            commands::get_thread,
            commands::set_focus_rule,
            commands::preview_body,
            commands::take_mailto,
            commands::parse_mailto,
            commands::get_message,
            commands::set_flags,
            commands::move_messages,
            commands::suggest_recipients,
            commands::delete_forever,
            commands::copy_messages,
            commands::mark_junk,
            commands::list_categories,
            commands::save_category,
            commands::delete_category,
            commands::set_categories,
            commands::set_folder_favorite,
            commands::reorder_accounts,
            commands::open_message_window,
            commands::save_attachment,
            commands::pick_attachments,
            commands::preview_attachment,
            commands::open_attachment,
            commands::save_message,
            commands::message_source,
            commands::print_html,
            commands::sync_now,
            commands::open_external,
            commands::check_draft,
            commands::send_message,
            commands::save_draft,
            commands::attachments_for_draft,
            commands::list_outbox,
            commands::outbox_send_now,
            commands::outbox_cancel,
            commands::outbox_edit,
            commands::get_settings,
            commands::set_settings,
            commands::list_contacts,
            commands::get_contact,
            commands::save_contact,
            commands::delete_contact,
            commands::sync_contacts,
            commands::list_contact_groups,
            commands::save_contact_group,
            commands::delete_contact_group,
            commands::connect_calendar_and_contacts,
            commands::list_calendars,
            commands::list_events,
            commands::get_event,
            commands::save_event,
            commands::delete_event,
            commands::sync_calendars,
            commands::set_calendar_visible,
            commands::get_free_busy,
            commands::list_due_reminders,
            commands::dismiss_reminder,
            commands::snooze_reminder,
            commands::list_task_lists,
            commands::list_tasks,
            commands::save_task,
            commands::delete_task,
            commands::sync_tasks,
            commands::answer_invitation,
            commands::get_out_of_office,
            commands::set_out_of_office,
            commands::remove_cancelled_event,
            commands::apply_invitation_reply,
            commands::get_signature,
            commands::list_signatures,
            commands::save_signature,
            commands::delete_signature,
            commands::set_signature_defaults,
            commands::set_signature_image,
            commands::set_signature_image_width,
            commands::remove_signature_image,
            commands::check_for_update,
            commands::install_update,
            commands::parse_search,
            commands::list_rules,
            commands::save_rule,
            commands::delete_rule,
            commands::reorder_rules,
            commands::run_rules,
            commands::preview_rule,
            commands::list_image_senders,
            commands::trust_image_sender,
            commands::untrust_image_sender,
            commands::resolve_recipients,
            commands::contact_card_attachment,
            commands::message_as_attachment,
            commands::open_compose_window,
            commands::take_compose_draft,
            commands::answer_read_receipt,
            commands::list_keys,
            commands::import_key,
            commands::export_public_key,
            commands::remove_key,
            commands::set_key_trusted,
            commands::set_account_key,
            commands::get_account_keys,
            commands::set_preferred_kind,
            commands::check_encryption,
            commands::import_mail_files,
            commands::export_mail,
            commands::import_calendar_file,
            commands::export_calendar,
            commands::import_contacts_file,
            commands::export_contacts,
            commands::import_opened_file,
            commands::import_message_file,
            commands::read_message_file,
            commands::import_pst,
            commands::take_opened_files,
        ])
        .build(tauri::generate_context!())
        .map(|app| app.run(on_run_event));

    if let Err(error) = result {
        eprintln!("Mocky Mail could not start: {error}");
        startup_failed(&error.to_string());
        std::process::exit(1);
    }
}

/// Events of the running app: on macOS, files opened with it (Finder's
/// "Open With", a double-click) come this way rather than as arguments.
fn on_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    if let tauri::RunEvent::Opened { urls } = &event {
        let host = TauriHost::new(app.clone());
        for url in urls {
            if let Ok(path) = url.to_file_path() {
                opened::deliver(Some(&host), &path, None, None);
            }
        }
    }
    let _ = (app, event);
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
