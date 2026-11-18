//! Per-account email signatures, stored in the settings table.

use mail_core::signature::{Signature, allowed_width, clean_signature, signature_image};
use tauri::{AppHandle, State};

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
    let cleaned = clean_signature(&html).map_err(|e| AppError::invalid(e.to_string()))?;
    // Editing the words keeps the picture.
    let image = load(&state, &account_id)?.and_then(|s| s.image);
    let signature = match (cleaned, image) {
        (Some(mut s), image) => {
            s.image = image;
            Some(s)
        }
        (None, Some(image)) => Some(picture_only(&state, &account_id, image)?),
        (None, None) => None,
    };
    save(&state, &account_id, signature.as_ref())?;
    Ok(signature)
}

fn save(state: &AppState, account_id: &str, signature: Option<&Signature>) -> AppResult<()> {
    let json = serde_json::to_string(&signature).map_err(|e| AppError::internal(e.to_string()))?;
    state.store.set_setting(&key(account_id), &json)?;
    Ok(())
}

/// A signature that is just a picture. Its text, used for plain-text
/// readers and as the picture's description, is the account's name.
fn picture_only(
    state: &AppState,
    account_id: &str,
    image: mail_core::signature::SignatureImage,
) -> AppResult<Signature> {
    let account = state.store.get_account(account_id)?;
    let name = if account.display_name.trim().is_empty() {
        account.email
    } else {
        account.display_name
    };
    let mut text = String::new();
    for c in name.chars().filter(|c| !c.is_control()) {
        text.push(c);
    }
    Ok(Signature {
        html: String::new(),
        text,
        image: Some(image),
    })
}

/// `set_signature_image`: choose a picture (PNG, JPG or GIF, up to 1 MB) to
/// show under the signature. `None` if the user cancelled.
#[tauri::command]
pub async fn set_signature_image(
    app: AppHandle,
    state: State<'_, std::sync::Arc<AppState>>,
    account_id: String,
) -> AppResult<Option<Signature>> {
    state.store.get_account(&account_id)?;
    let Some(path) = crate::attachments::pick_image(&app).await? else {
        return Ok(None);
    };
    let bytes = tokio::fs::read(&path).await?;
    let image = signature_image(&bytes).map_err(|e| AppError::invalid(e.to_string()))?;
    let signature = match load(&state, &account_id)? {
        Some(mut s) if !s.html.is_empty() => {
            s.image = Some(image);
            s
        }
        _ => picture_only(&state, &account_id, image)?,
    };
    save(&state, &account_id, Some(&signature))?;
    Ok(Some(signature))
}

/// `set_signature_image_width`: how wide the picture shows, in pixels.
#[tauri::command]
pub fn set_signature_image_width(
    state: State<'_, std::sync::Arc<AppState>>,
    account_id: String,
    width: u32,
) -> AppResult<Option<Signature>> {
    let Some(mut signature) = load(&state, &account_id)? else {
        return Ok(None);
    };
    if let Some(image) = signature.image.as_mut() {
        image.width = allowed_width(width, image.natural_width);
    }
    save(&state, &account_id, Some(&signature))?;
    Ok(Some(signature))
}

/// `remove_signature_image`: drop the picture, keep the words.
#[tauri::command]
pub fn remove_signature_image(
    state: State<'_, std::sync::Arc<AppState>>,
    account_id: String,
) -> AppResult<Option<Signature>> {
    let signature = load(&state, &account_id)?.and_then(|mut s| {
        s.image = None;
        (!s.html.is_empty()).then_some(s)
    });
    save(&state, &account_id, signature.as_ref())?;
    Ok(signature)
}

/// The signature picture as an inline part for a message, if there is one.
#[must_use]
pub fn inline_image(signature: &Signature) -> Option<mail_sync::outgoing::InlineImage> {
    let image = signature.image.as_ref()?;
    let data = image.bytes()?;
    Some(mail_sync::outgoing::InlineImage {
        content_id: mail_core::signature::SIGNATURE_IMAGE_CID.to_owned(),
        mime_type: image.mime_type.clone(),
        data,
    })
}
