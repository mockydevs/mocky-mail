//! App settings, stored as one JSON value in the database.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::AppResult;
use crate::state::AppState;

const KEY: &str = "settings";

/// User preferences (`Settings` in `docs/ipc.md`).
// Each bool is a separate on/off preference the interface shows as a checkbox.
#[allow(clippy::struct_excessive_bools)]
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
    /// Split the Inbox into Focused (people) and Other (newsletters,
    /// notifications). Off until the user turns it on.
    pub focused_inbox: bool,
    /// Minutes between checks for servers that can't announce new mail
    /// (no IDLE); all folders are checked at least every 10 minutes.
    pub check_every_mins: u32,
    /// Seconds a message must stay open to count as read ("delay").
    pub mark_read_delay_secs: u32,
    /// Notifications before calendar events.
    pub event_reminders: bool,
    /// Reminder for new events, minutes before; -1 for none.
    pub default_reminder_mins: i32,
}

/// Default-reminder choices offered in Settings, minutes (-1: none).
pub const REMINDER_CHOICES: [i32; 9] = [-1, 0, 5, 10, 15, 30, 60, 120, 1440];

/// Check-interval choices offered in Settings, minutes.
pub const CHECK_EVERY_CHOICES: [u32; 6] = [1, 2, 5, 10, 15, 30];
/// Mark-as-read delay choices offered in Settings, seconds.
pub const MARK_READ_DELAY_CHOICES: [u32; 4] = [1, 3, 5, 10];

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
            focused_inbox: false,
            check_every_mins: 2,
            mark_read_delay_secs: 3,
            event_reminders: true,
            default_reminder_mins: 15,
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
        if !CHECK_EVERY_CHOICES.contains(&self.check_every_mins) {
            self.check_every_mins = 2;
        }
        if !MARK_READ_DELAY_CHOICES.contains(&self.mark_read_delay_secs) {
            self.mark_read_delay_secs = 3;
        }
        if !REMINDER_CHOICES.contains(&self.default_reminder_mins) {
            self.default_reminder_mins = 15;
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
            focused_inbox: true,
            check_every_mins: 7,
            mark_read_delay_secs: 0,
            event_reminders: false,
            default_reminder_mins: 7,
        }
        .normalized();
        assert_eq!(s.theme, "system");
        assert_eq!(s.remote_images, "block");
        assert!(!s.notifications);
        assert_eq!(s.undo_send_secs, 5);
        assert_eq!(s.mark_read, "open");
        assert_eq!(s.check_every_mins, 2);
        assert_eq!(s.mark_read_delay_secs, 3);
        assert_eq!(s.default_reminder_mins, 15);
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
