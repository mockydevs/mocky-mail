//! Email signatures: several named ones per account (for example "Full" and
//! "Short"), each with optional words and a picture, plus which one new
//! messages and which one replies and forwards start with. Stored as JSON
//! in the settings table.
//!
//! The single signature older versions stored is moved into the list the
//! first time it is read.

use std::time::{SystemTime, UNIX_EPOCH};

use mail_core::signature::{
    Signature, SignatureImage, allowed_width, clean_signature, html_with_signature, signature_image,
};
use serde::{Deserialize, Serialize};

use crate::Ctx;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Most signatures one account may have.
const MAX_SIGNATURES: usize = 20;
/// Longest signature name, characters.
const MAX_NAME_CHARS: usize = 60;

fn key(account_id: &str) -> String {
    format!("signatures:{account_id}")
}

/// Where older versions kept the account's one signature.
fn old_key(account_id: &str) -> String {
    format!("signature:{account_id}")
}

/// One of an account's signatures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedSignature {
    /// Stable id.
    pub id: String,
    /// Shown in Settings and in the compose picker.
    pub name: String,
    /// Words (cleaned HTML and text) and picture.
    #[serde(flatten)]
    pub signature: Signature,
}

/// An account's signatures and defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureSet {
    /// In the order made.
    pub items: Vec<NamedSignature>,
    /// Added to new messages.
    pub default_new: Option<String>,
    /// Added to replies and forwards.
    pub default_reply: Option<String>,
}

impl SignatureSet {
    fn get(&self, id: Option<&str>) -> Option<&NamedSignature> {
        let id = id?;
        self.items.iter().find(|s| s.id == id)
    }

    fn get_mut(&mut self, id: &str) -> AppResult<&mut NamedSignature> {
        self.items
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| AppError::new("not_found", "That signature no longer exists."))
    }
}

fn empty() -> Signature {
    Signature {
        html: String::new(),
        text: String::new(),
        image: None,
    }
}

fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("s{nanos:x}")
}

/// The account's signatures, moving an older single signature into the list.
///
/// # Errors
/// Database errors.
pub fn load_set(state: &AppState, account_id: &str) -> AppResult<SignatureSet> {
    if let Some(json) = state.store.get_setting(&key(account_id))? {
        // An unreadable stored value means no signatures rather than a broken compose.
        return Ok(serde_json::from_str(&json).unwrap_or_default());
    }
    let old = state
        .store
        .get_setting(&old_key(account_id))?
        .and_then(|json| {
            serde_json::from_str::<Option<Signature>>(&json)
                .ok()
                .flatten()
        });
    let set = match old {
        Some(signature) => SignatureSet {
            items: vec![NamedSignature {
                id: "main".to_owned(),
                name: "Signature".to_owned(),
                signature,
            }],
            default_new: Some("main".to_owned()),
            default_reply: Some("main".to_owned()),
        },
        None => SignatureSet::default(),
    };
    Ok(set)
}

fn save_set(state: &AppState, account_id: &str, set: &SignatureSet) -> AppResult<()> {
    let json = serde_json::to_string(set).map_err(|e| AppError::internal(e.to_string()))?;
    state.store.set_setting(&key(account_id), &json)?;
    Ok(())
}

/// The signature a message of this kind starts with, if any.
///
/// # Errors
/// Database errors.
pub fn load_default(
    state: &AppState,
    account_id: &str,
    reply: bool,
) -> AppResult<Option<Signature>> {
    let set = load_set(state, account_id)?;
    let id = if reply {
        &set.default_reply
    } else {
        &set.default_new
    };
    Ok(set.get(id.as_deref()).map(|s| s.signature.clone()))
}

/// The account signature whose block is still in `body` as compose put it,
/// so the message can be sent with its full HTML and picture.
///
/// # Errors
/// Database errors.
pub fn in_body(state: &AppState, account_id: &str, body: &str) -> AppResult<Option<Signature>> {
    Ok(in_body_named(state, account_id, body)?.map(|s| s.signature))
}

/// [`in_body`], with the signature's id and name.
///
/// # Errors
/// Database errors.
pub fn in_body_named(
    state: &AppState,
    account_id: &str,
    body: &str,
) -> AppResult<Option<NamedSignature>> {
    Ok(load_set(state, account_id)?
        .items
        .into_iter()
        .filter(|s| !s.signature.text.is_empty())
        // A longer signature first, so "Don\nMocky" wins over "Don".
        .max_by_key(|s| {
            (
                html_with_signature(body, &s.signature).is_some(),
                s.signature.text.len(),
            )
        })
        .filter(|s| html_with_signature(body, &s.signature).is_some()))
}

/// One of the account's signatures by id.
///
/// # Errors
/// Database errors.
pub fn by_id(state: &AppState, account_id: &str, id: &str) -> AppResult<Option<NamedSignature>> {
    Ok(load_set(state, account_id)?
        .items
        .into_iter()
        .find(|s| s.id == id))
}

/// Forget an account's signatures (the account is being removed).
///
/// # Errors
/// Database errors.
pub fn remove(state: &AppState, account_id: &str) -> AppResult<()> {
    state.store.set_setting(&key(account_id), "{}")?;
    state.store.set_setting(&old_key(account_id), "null")?;
    Ok(())
}

/// A signature that is just a picture uses the account's name as its text
/// (for plain-text readers, and as the picture's description).
fn picture_text(state: &AppState, account_id: &str) -> AppResult<String> {
    let account = state.store.get_account(account_id)?;
    let name = if account.display_name.trim().is_empty() {
        account.email
    } else {
        account.display_name
    };
    Ok(name.chars().filter(|c| !c.is_control()).collect())
}

/// `get_signature`: the signature a new message (or, with `reply`, a reply
/// or forward) starts with.
pub fn get_signature(
    ctx: &Ctx,
    account_id: String,
    reply: Option<bool>,
) -> AppResult<Option<Signature>> {
    let state = &ctx.state;
    state.store.get_account(&account_id)?;
    load_default(state, &account_id, reply.unwrap_or(false))
}

/// `list_signatures`: all of the account's signatures and its defaults.
pub fn list_signatures(ctx: &Ctx, account_id: String) -> AppResult<SignatureSet> {
    let state = &ctx.state;
    state.store.get_account(&account_id)?;
    load_set(state, &account_id)
}

/// `save_signature`: create (`id` null) or change a signature's name and
/// words. The words are cleaned like incoming mail; the picture is kept.
/// The first signature becomes the default for new messages and replies.
pub fn save_signature(
    ctx: &Ctx,
    account_id: String,
    id: Option<String>,
    name: String,
    html: String,
) -> AppResult<SignatureSet> {
    let state = &ctx.state;
    state.store.get_account(&account_id)?;
    // Pasted signatures with embedded images can be huge; refuse before cleaning.
    if html.len() > 4 * mail_core::signature::MAX_SIGNATURE_BYTES {
        return Err(AppError::invalid(
            "This signature is too large. Add the logo with Add a picture instead of pasting it.",
        ));
    }
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name: String = name.chars().take(MAX_NAME_CHARS).collect();
    let cleaned = clean_signature(&html).map_err(|e| AppError::invalid(e.to_string()))?;
    let mut set = load_set(state, &account_id)?;
    let id = if let Some(id) = id {
        id
    } else {
        if set.items.len() >= MAX_SIGNATURES {
            return Err(AppError::invalid(format!(
                "An account can have up to {MAX_SIGNATURES} signatures."
            )));
        }
        let id = new_id();
        let number = set.items.len().saturating_add(1);
        set.items.push(NamedSignature {
            id: id.clone(),
            name: format!("Signature {number}"),
            signature: empty(),
        });
        if set.items.len() == 1 {
            set.default_new = Some(id.clone());
            set.default_reply = Some(id.clone());
        }
        id
    };
    let picture_name = picture_text(state, &account_id)?;
    let item = set.get_mut(&id)?;
    if !name.is_empty() {
        item.name = name;
    }
    let image = item.signature.image.take();
    item.signature = match cleaned {
        Some(mut words) => {
            words.image = image;
            words
        }
        None => Signature {
            html: String::new(),
            text: if image.is_some() {
                picture_name
            } else {
                String::new()
            },
            image,
        },
    };
    save_set(state, &account_id, &set)?;
    Ok(set)
}

/// `delete_signature`: remove one; defaults that pointed at it become none.
pub fn delete_signature(ctx: &Ctx, account_id: String, id: String) -> AppResult<SignatureSet> {
    let state = &ctx.state;
    let mut set = load_set(state, &account_id)?;
    set.items.retain(|s| s.id != id);
    for default in [&mut set.default_new, &mut set.default_reply] {
        if default.as_deref() == Some(id.as_str()) {
            *default = None;
        }
    }
    save_set(state, &account_id, &set)?;
    Ok(set)
}

/// `set_signature_defaults`: which signature new messages, and which
/// replies and forwards, start with (null: none).
pub fn set_signature_defaults(
    ctx: &Ctx,
    account_id: String,
    default_new: Option<String>,
    default_reply: Option<String>,
) -> AppResult<SignatureSet> {
    let state = &ctx.state;
    let mut set = load_set(state, &account_id)?;
    let known = |id: &Option<String>| {
        id.as_ref()
            .is_none_or(|id| set.items.iter().any(|s| &s.id == id))
    };
    if !known(&default_new) || !known(&default_reply) {
        return Err(AppError::new(
            "not_found",
            "That signature no longer exists.",
        ));
    }
    set.default_new = default_new;
    set.default_reply = default_reply;
    save_set(state, &account_id, &set)?;
    Ok(set)
}

fn set_image(item: &mut NamedSignature, image: Option<SignatureImage>, picture_name: String) {
    let words = !item.signature.html.is_empty();
    item.signature.image = image;
    if !words {
        // Picture-only: the account's name stands in for words; none left: empty.
        item.signature.text = if item.signature.image.is_some() {
            picture_name
        } else {
            String::new()
        };
    }
}

/// `set_signature_image`: choose a picture (PNG, JPG or GIF, up to 1 MB)
/// for a signature. `None` if the user cancelled.
pub async fn set_signature_image(
    ctx: &Ctx,
    account_id: String,
    id: String,
) -> AppResult<Option<SignatureSet>> {
    let state = &ctx.state;
    load_set(state, &account_id)?.get_mut(&id)?;
    let Some(path) = crate::attachments::pick_image(ctx).await? else {
        return Ok(None);
    };
    let bytes = tokio::fs::read(&path).await?;
    let image = signature_image(&bytes).map_err(|e| AppError::invalid(e.to_string()))?;
    let picture_name = picture_text(state, &account_id)?;
    let mut set = load_set(state, &account_id)?;
    set_image(set.get_mut(&id)?, Some(image), picture_name);
    save_set(state, &account_id, &set)?;
    Ok(Some(set))
}

/// `set_signature_image_width`: how wide a signature's picture shows, px.
pub fn set_signature_image_width(
    ctx: &Ctx,
    account_id: String,
    id: String,
    width: u32,
) -> AppResult<SignatureSet> {
    let state = &ctx.state;
    let mut set = load_set(state, &account_id)?;
    if let Some(image) = set.get_mut(&id)?.signature.image.as_mut() {
        image.width = allowed_width(width, image.natural_width);
    }
    save_set(state, &account_id, &set)?;
    Ok(set)
}

/// `remove_signature_image`: drop a signature's picture, keep its words.
pub fn remove_signature_image(
    ctx: &Ctx,
    account_id: String,
    id: String,
) -> AppResult<SignatureSet> {
    let state = &ctx.state;
    let mut set = load_set(state, &account_id)?;
    set_image(set.get_mut(&id)?, None, String::new());
    save_set(state, &account_id, &set)?;
    Ok(set)
}

/// A signature's picture as an inline part for a message, if it has one.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn named(id: &str, text: &str) -> NamedSignature {
        NamedSignature {
            id: id.into(),
            name: id.into(),
            signature: Signature {
                html: format!("<b>{text}</b>"),
                text: text.into(),
                image: None,
            },
        }
    }

    #[test]
    fn older_single_signatures_read_as_a_list() {
        let json = r#"{"items":[{"id":"main","name":"Signature","html":"<b>Don</b>","text":"Don"}],"defaultNew":"main","defaultReply":null}"#;
        let set: SignatureSet = serde_json::from_str(json).unwrap_or_default();
        assert_eq!(set.items.len(), 1);
        assert_eq!(
            set.get(Some("main")).map(|s| s.signature.text.as_str()),
            Some("Don")
        );
        assert_eq!(set.default_reply, None);
    }

    #[test]
    fn picture_only_signatures_use_the_name_and_drop_it_with_the_picture() {
        let mut item = named("a", "");
        item.signature.html = String::new();
        let image = mail_core::signature::SignatureImage {
            mime_type: "image/png".into(),
            data: String::new(),
            width: 10,
            natural_width: 10,
            natural_height: 10,
        };
        set_image(&mut item, Some(image), "Don O.".into());
        assert_eq!(item.signature.text, "Don O.");
        set_image(&mut item, None, String::new());
        assert_eq!(item.signature.text, "");
        // With words, the words stay.
        let mut worded = named("b", "Don");
        set_image(&mut worded, None, String::new());
        assert_eq!(worded.signature.text, "Don");
    }
}
