//! The address book: commands for the Contacts screen, and syncing each
//! password account's contacts with its server over `CardDAV` (for example
//! `SOGo` on the account's mail server).
//!
//! Changes are saved here first and sent on the next sync, so contacts work
//! offline. Accounts whose server has no address book, and Gmail or
//! Microsoft accounts (which would need the provider's contacts API), keep
//! contacts on this device.

use std::sync::Arc;
use std::time::Duration;

use mail_core::vcard::{self, ContactCard};
use mail_store::{AuthKind, Contact, ContactState, MAX_CONTACTS};
use mail_sync::dav::{DavClient, DavLogin};
use mail_sync::{Credentials, SyncError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, State, Url};

use crate::error::{AppError, AppResult};
use crate::state::{AppState, now};

/// How often contacts are synced in the background.
const SYNC_EVERY: Duration = Duration::from_mins(15);

fn book_key(account_id: &str) -> String {
    format!("carddav:{account_id}")
}

/// Where an account's contacts live, for the Contacts screen.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddressBookStatus {
    account_id: String,
    /// `synced`, `unavailable` (no address book on the server or not
    /// supported yet), or `error`.
    state: &'static str,
    /// Plain words for the user.
    message: Option<String>,
}

/// A contact as typed in the editor.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactInput {
    /// Missing for a new contact.
    id: Option<String>,
    /// The account whose address book holds it; missing: this device only.
    account_id: Option<String>,
    name: String,
    emails: Vec<String>,
    phones: Vec<String>,
    organization: String,
    title: String,
    notes: String,
}

const MAX_FIELD: usize = 2000;
const MAX_ITEMS: usize = 20;

fn tidy(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(MAX_FIELD)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn card_from(input: &ContactInput) -> AppResult<ContactCard> {
    let emails: Vec<String> = input
        .emails
        .iter()
        .map(|e| tidy(e).replace('\n', ""))
        .filter(|e| !e.is_empty())
        .take(MAX_ITEMS)
        .collect();
    for email in &emails {
        mail_core::compose::parse_address_list(email)
            .ok()
            .filter(|list| list.len() == 1)
            .ok_or_else(|| AppError::invalid(format!("{email} isn't an email address.")))?;
    }
    let card = ContactCard {
        uid: String::new(),
        name: tidy(&input.name).replace('\n', " "),
        emails,
        phones: input
            .phones
            .iter()
            .map(|p| tidy(p).replace('\n', ""))
            .filter(|p| !p.is_empty())
            .take(MAX_ITEMS)
            .collect(),
        organization: tidy(&input.organization).replace('\n', " "),
        title: tidy(&input.title).replace('\n', " "),
        notes: tidy(&input.notes),
    };
    if card.name.is_empty() && card.emails.is_empty() && card.phones.is_empty() {
        return Err(AppError::invalid(
            "Add a name, an email address or a phone number.",
        ));
    }
    Ok(card)
}

/// `list_contacts`: saved contacts matching `query`, by name.
#[tauri::command]
pub fn list_contacts(state: State<'_, Arc<AppState>>, query: String) -> AppResult<Vec<Contact>> {
    Ok(state.store.list_contacts(&query, MAX_CONTACTS)?)
}

/// `get_contact`.
#[tauri::command]
pub fn get_contact(state: State<'_, Arc<AppState>>, id: String) -> AppResult<Contact> {
    Ok(state.store.get_contact(&id)?)
}

/// `save_contact`: create or update; synced on the next contacts sync.
#[tauri::command]
pub fn save_contact(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    contact: ContactInput,
) -> AppResult<Contact> {
    let card = card_from(&contact)?;
    let saved = state.store.save_contact(
        contact.id.as_deref(),
        contact.account_id.as_deref(),
        card,
        now(),
    )?;
    soon(&app, &state);
    Ok(saved)
}

/// `delete_contact`: removed here at once, from the server on the next sync.
#[tauri::command]
pub fn delete_contact(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> AppResult<()> {
    state.store.delete_contact(&id)?;
    soon(&app, &state);
    Ok(())
}

/// `sync_contacts`: sync every account's address book now.
#[tauri::command]
pub async fn sync_contacts(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> AppResult<Vec<AddressBookStatus>> {
    Ok(sync_all(&app, &state).await)
}

/// Whether an account's contacts have an address book on its server (as
/// found by the last sync).
pub(crate) fn has_address_book(state: &AppState, account_id: &str) -> bool {
    state
        .store
        .get_setting(&book_key(account_id))
        .ok()
        .flatten()
        .is_some_and(|saved| Url::parse(&saved).is_ok())
}

/// Sync shortly, after a change made here.
pub(crate) fn soon(app: &AppHandle, state: &Arc<AppState>) {
    let (app, state) = (app.clone(), Arc::clone(state));
    tauri::async_runtime::spawn(async move {
        sync_all(&app, &state).await;
    });
}

/// Background sync: once at start-up, then every 15 minutes.
pub fn start(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        loop {
            sync_all(&app, &state).await;
            tokio::time::sleep(SYNC_EVERY).await;
        }
    });
}

async fn sync_all(app: &AppHandle, state: &Arc<AppState>) -> Vec<AddressBookStatus> {
    // One sync at a time; a second caller waits and then syncs again.
    let _guard = state.contacts_sync.lock().await;
    let accounts = match state.store.list_accounts() {
        Ok(accounts) => accounts,
        Err(error) => {
            eprintln!("contacts: couldn't list accounts: {error}");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for account in accounts {
        let status = match sync_account(state, &account).await {
            Ok(None) => AddressBookStatus {
                account_id: account.id.clone(),
                state: "synced",
                message: None,
            },
            Ok(Some(reason)) => AddressBookStatus {
                account_id: account.id.clone(),
                state: "unavailable",
                message: Some(reason),
            },
            Err(error) => {
                eprintln!("contacts: {}: {error}", account.email);
                AddressBookStatus {
                    account_id: account.id.clone(),
                    state: "error",
                    message: Some(error.message),
                }
            }
        };
        out.push(status);
    }
    if let Err(error) = app.emit("mail://contacts-changed", json!({ "books": out })) {
        eprintln!("couldn't emit contacts-changed: {error}");
    }
    out
}

/// Delete a contact on the server. One the server refuses to delete is
/// left there rather than failing the whole sync.
async fn push_delete(
    client: &DavClient,
    book: &Url,
    href: &str,
    etag: Option<&str>,
) -> Result<(), SyncError> {
    let mut deleted = client.delete_card(book, href, etag).await;
    // Changed elsewhere since: the delete, being newer, wins.
    if matches!(deleted, Err(SyncError::Refused(_))) {
        deleted = client.delete_card(book, href, None).await;
    }
    match deleted {
        // The server won't delete it at all (a read-only book). Asking
        // again on every sync would block everything after it; the next
        // download brings the contact back instead.
        Err(SyncError::Refused(reason)) => {
            eprintln!("contacts: the server kept a deleted contact: {reason}");
            Ok(())
        }
        other => other,
    }
}

/// Sync one account. `Ok(Some(reason))` when it has no address book here.
async fn sync_account(
    state: &AppState,
    account: &mail_store::Account,
) -> AppResult<Option<String>> {
    if account.auth != AuthKind::Password {
        return Ok(Some(
            "Contacts for Gmail and Microsoft accounts are kept on this device for now.".to_owned(),
        ));
    }
    let Credentials::Password { username, password } =
        crate::credentials::for_account(state, account).await?
    else {
        return Ok(Some(
            "This account has no password to sync contacts with.".to_owned(),
        ));
    };
    let client = DavClient::new(DavLogin { username, password })?;
    let book = match address_book(state, &client, account).await {
        Ok(Some(book)) => book,
        Ok(None) => {
            return Ok(Some(
                "The mail server doesn't offer an address book.".to_owned(),
            ));
        }
        Err(error) => return Err(error),
    };

    // Send what changed here first.
    for pending in state.store.contacts_to_push(&account.id)? {
        let id = pending.contact.id.clone();
        match pending.contact.state {
            ContactState::Deleted => {
                if let Some(href) = &pending.href {
                    push_delete(&client, &book, href, pending.etag.as_deref()).await?;
                }
                state.store.forget_contact(&id)?;
            }
            ContactState::Changed => {
                let text = vcard::write(&pending.contact.card, pending.vcard.as_deref());
                let uid = &pending.contact.card.uid;
                let put = client
                    .put_card(
                        &book,
                        pending.href.as_deref(),
                        uid,
                        &text,
                        pending.etag.as_deref(),
                    )
                    .await;
                let (href, etag) = match put {
                    // Changed elsewhere since: the newest edit (this one) wins.
                    Err(SyncError::Refused(_)) if pending.href.is_some() => {
                        client
                            .put_card(&book, pending.href.as_deref(), uid, &text, None)
                            .await?
                    }
                    other => other?,
                };
                state
                    .store
                    .contact_pushed(&id, &href, etag.as_deref(), &text)?;
            }
            ContactState::Synced => {}
        }
    }

    // Then read what changed on the server.
    let listed = match client.list_cards(&book).await {
        Ok(listed) => listed,
        Err(error) => {
            // The remembered address book may have moved: look again next time.
            if matches!(
                error,
                SyncError::Protocol(_) | SyncError::Refused(_) | SyncError::Config(_)
            ) {
                state.store.set_setting(&book_key(&account.id), "")?;
            }
            return Err(error.into());
        }
    };
    let known = state.store.contact_etags(&account.id)?;
    let wanted: Vec<String> = listed
        .iter()
        .filter(|(href, etag)| match known.get(href) {
            Some((known_etag, _)) => etag.is_none() || known_etag != etag,
            None => true,
        })
        .map(|(href, _)| href.clone())
        .collect();
    for remote in client.fetch_cards(&book, &wanted).await? {
        if let Some(card) = vcard::parse(&remote.vcard) {
            state.store.apply_server_contact(
                &account.id,
                &remote.href,
                remote.etag.as_deref(),
                &remote.vcard,
                &card,
                now(),
            )?;
        }
    }
    let on_server: Vec<String> = listed.into_iter().map(|(href, _)| href).collect();
    state
        .store
        .forget_contacts_not_on_server(&account.id, &on_server)?;
    Ok(None)
}

/// The account's address book URL: remembered, or found and remembered.
/// `None` when the server has none.
async fn address_book(
    state: &AppState,
    client: &DavClient,
    account: &mail_store::Account,
) -> AppResult<Option<Url>> {
    if let Some(url) = state
        .store
        .get_setting(&book_key(&account.id))?
        .and_then(|saved| Url::parse(&saved).ok())
    {
        return Ok(Some(url));
    }
    let server = Url::parse(&format!("https://{}/", account.imap.host))
        .map_err(|e| AppError::internal(e.to_string()))?;
    let books = match client.discover_address_books(&server).await {
        Ok(books) => books,
        // No CardDAV service at all (404, a web page, no principal).
        Err(SyncError::Config(_) | SyncError::Protocol(_) | SyncError::Refused(_)) => {
            return Ok(None);
        }
        Err(other) => return Err(other.into()),
    };
    // SOGo calls the main one "personal"; otherwise take the first.
    let book = books
        .iter()
        .find(|b| b.url.path().trim_end_matches('/').ends_with("/personal"))
        .or_else(|| books.first())
        .map(|b| b.url.clone());
    if let Some(book) = &book {
        state
            .store
            .set_setting(&book_key(&account.id), book.as_str())?;
    }
    Ok(book)
}

/// Saved contacts first, then people from mail, for recipient suggestions.
pub fn suggestions(
    state: &AppState,
    query: &str,
    limit: usize,
) -> AppResult<Vec<mail_core::message::Mailbox>> {
    let mut out: Vec<mail_core::message::Mailbox> = Vec::new();
    if query.trim().is_empty() {
        return Ok(state.store.suggest_recipients(query, limit)?);
    }
    for contact in state.store.list_contacts(query, limit)? {
        for email in &contact.card.emails {
            if !out.iter().any(|m| m.address.eq_ignore_ascii_case(email)) {
                out.push(mail_core::message::Mailbox {
                    name: Some(contact.card.name.clone()).filter(|n| !n.is_empty()),
                    address: email.clone(),
                });
            }
        }
    }
    for person in state.store.suggest_recipients(query, limit)? {
        if !out
            .iter()
            .any(|m| m.address.eq_ignore_ascii_case(&person.address))
        {
            out.push(person);
        }
    }
    out.truncate(limit);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, email: &str) -> ContactInput {
        ContactInput {
            id: None,
            account_id: None,
            name: name.into(),
            emails: vec![email.into()],
            phones: vec![" +254 700 000 001 ".into(), String::new()],
            organization: "Mocky\u{0}Digital".into(),
            title: String::new(),
            notes: "Line one\nLine two".into(),
        }
    }

    #[test]
    fn typed_contacts_are_tidied_and_checked() {
        let card = card_from(&input("  Peter Mwangi ", "peter@client.example")).unwrap_or_default();
        assert_eq!(card.name, "Peter Mwangi");
        assert_eq!(card.phones, ["+254 700 000 001"]);
        assert_eq!(card.organization, "MockyDigital");
        assert_eq!(card.notes, "Line one\nLine two");
        assert!(card_from(&input("Peter", "not an address")).is_err());
        assert!(
            card_from(&ContactInput {
                phones: vec![],
                ..input("", "")
            })
            .is_err()
        );
    }
}
