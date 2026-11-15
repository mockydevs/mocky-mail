//! Per-account email signatures, stored in the settings table.

use mail_core::signature::{Signature, clean_signature};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

fn key(account_id: &str) -> String {
    format!("signature:{account_id}")
}

/// The account's signature, if it has one.
///
/// # Errors
/// Database errors.
pub fn load(state: &AppState, account_id: &str) -> AppResult<Option<Signature>> {
    let Some(json) = state.store.get_setting(&key(account_id))? else {
        return Ok(None);
    };
    // An unreadable stored value means no signature rather than a broken compose.
    Ok(serde_json::from_str::<Option<Signature>>(&json).unwrap_or(None))
}

/// Forget an account's signature (the account is being removed).
///
/// # Errors
/// Database errors.
pub fn remove(state: &AppState, account_id: &str) -> AppResult<()> {
    state.store.set_setting(&key(account_id), "null")?;
    Ok(())
}

/// `get_signature` command.
#[tauri::command]
pub fn get_signature(
    state: State<'_, std::sync::Arc<AppState>>,
    account_id: String,
) -> AppResult<Option<Signature>> {
    state.store.get_account(&account_id)?;
    load(&state, &account_id)
}

/// `set_signature`: clean and store a signature; an empty one clears it.
/// Returns what was stored, so the interface shows the cleaned version.
#[tauri::command]
pub fn set_signature(
    state: State<'_, std::sync::Arc<AppState>>,
    account_id: String,
    html: String,
) -> AppResult<Option<Signature>> {
    state.store.get_account(&account_id)?;
    // Pasted signatures with embedded images can be huge; refuse before cleaning.
    if html.len() > 4 * mail_core::signature::MAX_SIGNATURE_BYTES {
        return Err(AppError::invalid(
            "This signature is too large. Use a linked logo instead of a pasted image.",
        ));
    }
    let signature = clean_signature(&html).map_err(|e| AppError::invalid(e.to_string()))?;
    let json = serde_json::to_string(&signature).map_err(|e| AppError::internal(e.to_string()))?;
    state.store.set_setting(&key(&account_id), &json)?;
    Ok(signature)
}
