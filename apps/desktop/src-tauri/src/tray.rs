//! Desktop only: the tray icon, and keeping the app running in the tray
//! when its window is closed, so mail keeps syncing and new-mail and
//! reminder notifications keep coming. Also start at login, opening
//! straight into the tray.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};

use crate::state::AppState;

/// The tray icon is showing, so a closed window can come back from it.
static READY: AtomicBool = AtomicBool::new(false);
/// Launched at login: start in the tray.
pub const HIDDEN_ARG: &str = "--hidden";
/// The "still running in the tray" note was shown once.
const TOLD_KEY: &str = "tray:told";

/// Started at login, so the window waits in the tray.
#[must_use]
pub fn start_hidden() -> bool {
    std::env::args().any(|a| a == HIDDEN_ARG)
}

/// Bring the window back.
pub fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Add the tray icon. A desktop without a tray (some Linux desktops) just
/// has none; closing the window then quits as usual.
pub fn build(app: &tauri::App) {
    match try_build(app) {
        Ok(()) => READY.store(true, Ordering::Relaxed),
        Err(error) => eprintln!("no tray icon: {error}"),
    }
}

fn try_build(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Mocky Mail", true, None::<&str>)?;
    let compose = MenuItem::with_id(app, "compose", "New message", true, None::<&str>)?;
    let check = MenuItem::with_id(app, "check", "Check for mail", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Mocky Mail", true, None::<&str>)?;
    let gap = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &compose, &check, &gap, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Mocky Mail")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show(app),
            "compose" => {
                show(app);
                let _ = app.emit("mail://compose", serde_json::json!({}));
            }
            "check" => {
                if let Some(state) = app.try_state::<Arc<AppState>>() {
                    state.wake_sync(None);
                    state.outbox_wake.notify_one();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Closing the window hides it to the tray (when the tray is there and the
/// setting is on); the first time, a note says the app is still running.
pub fn on_window_event(window: &tauri::Window, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    let app = window.app_handle();
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };
    let keep = READY.load(Ordering::Relaxed)
        && crate::settings::load(&state).is_ok_and(|s| s.close_to_tray);
    if !keep {
        return;
    }
    api.prevent_close();
    let _ = window.hide();
    if state.store.get_setting(TOLD_KEY).ok().flatten().is_none() {
        let _ = state.store.set_setting(TOLD_KEY, "1");
        crate::events::notify(
            app,
            "Mocky Mail is still running",
            "It keeps checking for mail. Open it from the tray icon; Quit is there too.",
        );
    }
}
