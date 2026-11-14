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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            remote_images: "block".into(),
            notifications: true,
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
        }
        .normalized();
        assert_eq!(s.theme, "system");
        assert_eq!(s.remote_images, "block");
        assert!(!s.notifications);
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
}
