//! Gmail sign-in on Android with the phone's own Google accounts, through
//! `GoogleAuthPlugin` (Kotlin, in `gen/android`). The plugin shows Android's
//! account chooser and Google's consent sheet, and returns access tokens to
//! Rust only. Desktop uses the browser flow in `mail-auth` instead.

use std::sync::OnceLock;

use serde::Deserialize;
use tauri::Wry;
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};

use crate::error::{AppError, AppResult};

static PLUGIN: OnceLock<PluginHandle<Wry>> = OnceLock::new();

/// Google issues access tokens for about an hour; count a little less.
pub const TOKEN_LIFETIME_SECS: u64 = 50 * 60;

/// Register the Kotlin plugin. Android only.
pub fn init() -> TauriPlugin<Wry> {
    Builder::new("google-auth")
        .setup(|_app, api| {
            let handle = api.register_android_plugin("ke.co.mocky.mail", "GoogleAuthPlugin")?;
            let _ = PLUGIN.set(handle);
            Ok(())
        })
        .build()
}

/// An account and a fresh access token for it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Granted {
    /// The Google account chosen on the phone.
    pub email: String,
    /// Bearer token for IMAP and SMTP (XOAUTH2).
    pub access_token: String,
}

fn handle() -> AppResult<&'static PluginHandle<Wry>> {
    PLUGIN
        .get()
        .ok_or_else(|| AppError::internal("Google sign-in isn't available on this device."))
}

/// Map the plugin's reject codes to the app's error kinds.
fn plugin_error(error: tauri::plugin::mobile::PluginInvokeError) -> AppError {
    use tauri::plugin::mobile::PluginInvokeError as E;
    let (code, message) = match error {
        E::InvokeRejected(rejected) => (
            rejected.code.unwrap_or_default(),
            rejected.message.unwrap_or_default(),
        ),
        other => (String::new(), other.to_string()),
    };
    match code.as_str() {
        "cancelled" | "denied" | "needs_consent" => AppError::new("auth", message),
        // The app's Google registration is wrong: nothing the user can fix.
        "not_set_up" => AppError::new("config", message),
        _ => AppError::new("connect", format!("Google sign-in failed: {message}")),
    }
}

/// Show the account chooser, then Google's consent sheet if needed.
///
/// # Errors
/// `auth` when the user cancels or declines; `connect` for Play services
/// or network failures.
pub async fn sign_in() -> AppResult<Granted> {
    handle()?
        .run_mobile_plugin_async::<Granted>("signIn", serde_json::json!({}))
        .await
        .map_err(plugin_error)
}

#[derive(serde::Serialize)]
struct TokenArgs<'a> {
    email: &'a str,
}

/// A fresh access token for an account that already granted access, with no
/// screen shown.
///
/// # Errors
/// `auth` when access was withdrawn and the user must sign in again.
pub async fn token(email: &str) -> AppResult<String> {
    Ok(handle()?
        .run_mobile_plugin_async::<Granted>("token", TokenArgs { email })
        .await
        .map_err(plugin_error)?
        .access_token)
}
