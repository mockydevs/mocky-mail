//! App settings, stored as one JSON value in the database.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::AppResult;
use crate::state::AppState;

const KEY: &str = "settings";

/// User preferences (`Settings` in `docs/ipc.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// "system", "light" or "dark".
    pub theme: String,
    /// "block" or "allow".
    pub remote_images: String,
    /// Desktop notifications for new mail.
    pub notifications: bool,
    /// A short chime with each new-mail notification.
    pub sound: bool,
    /// Seconds Send waits so it can be undone; 0 sends at once.
    pub undo_send_secs: u32,
    /// "open" (as soon as a message is opened), "delay" (after a few
    /// seconds open) or "manual" (only with Mark as read).
    pub mark_read: String,
    /// Group replies with the message they answer, as one conversation.
    pub threads: bool,
}

/// Undo-send choices offered in Settings.
pub const UNDO_SEND_CHOICES: [u32; 5] = [0, 5, 10, 20, 30];

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            remote_images: "block".into(),
            notifications: true,
            sound: true,
            undo_send_secs: 5,
            mark_read: "open".into(),
            threads: true,
        }
    }
}

impl Settings {
    fn normalized(mut self) -> Self {
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") {
            self.theme = "system".into();
        }
        if !matches!(self.remote_images.as_str(), "block" | "allow") {
            self.remote_images = "block".into();
        }
        if !UNDO_SEND_CHOICES.contains(&self.undo_send_secs) {
            self.undo_send_secs = 5;
        }
        if !matches!(self.mark_read.as_str(), "open" | "delay" | "manual") {
            self.mark_read = "open".into();
        }
        self
    }
}

/// Read settings, falling back to defaults.
///
/// # Errors
/// Database errors.
pub fn load(state: &AppState) -> AppResult<Settings> {
    Ok(state
        .store
        .get_setting(KEY)?
        .and_then(|json| serde_json::from_str::<Settings>(&json).ok())
        .unwrap_or_default()
        .normalized())
}

/// `get_settings` command.
#[tauri::command]
pub fn get_settings(state: State<'_, std::sync::Arc<AppState>>) -> AppResult<Settings> {
    load(&state)
}

/// `set_settings` command.
#[tauri::command]
pub fn set_settings(
    state: State<'_, std::sync::Arc<AppState>>,
    settings: Settings,
) -> AppResult<Settings> {
    let settings = settings.normalized();
    let json = serde_json::to_string(&settings)
        .map_err(|e| crate::error::AppError::internal(e.to_string()))?;
    state.store.set_setting(KEY, &json)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_values_fall_back() {
        let s = Settings {
            theme: "neon".into(),
            remote_images: "maybe".into(),
            notifications: false,
            sound: false,
            undo_send_secs: 7,
            mark_read: "sometimes".into(),
            threads: false,
        }
        .normalized();
        assert_eq!(s.theme, "system");
        assert_eq!(s.remote_images, "block");
        assert!(!s.notifications);
        assert_eq!(s.undo_send_secs, 5);
        assert_eq!(s.mark_read, "open");
    }

    #[test]
    fn missing_fields_use_defaults() {
        let s: Settings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap_or_default();
        assert_eq!(
            s,
            Settings {
                theme: "dark".into(),
                ..Settings::default()
            }
        );
    }

    #[test]
    fn settings_saved_before_the_sound_option_keep_the_chime_on() {
        let s: Settings = serde_json::from_str(
            r#"{"theme":"light","remoteImages":"block","notifications":true}"#,
        )
        .unwrap_or_default();
        assert!(s.sound);
    }
}
