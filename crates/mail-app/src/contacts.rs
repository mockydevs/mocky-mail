//! The address book: commands for the Contacts screen, contact groups,
//! and syncing each account's contacts: password accounts over `CardDAV`
//! (for example `SOGo` on the account's mail server), connected Gmail
//! accounts over Google's `CardDAV` (groups through the People API),
//! connected Microsoft accounts over Microsoft Graph.
//!
//! Changes are saved here first and sent on the next sync, so contacts work
//! offline. Accounts whose server has no address book, and Gmail or
//! Microsoft accounts not connected, keep contacts on this device.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mail_core::vcard::{self, ContactCard, GroupCard};
use mail_store::{Account, Contact, ContactGroup, ContactState, MAX_CONTACTS};
use mail_sync::dav::{DavClient, DavLogin};
use mail_sync::rest::RestClient;
use mail_sync::{Credentials, SyncError, google, graph};
use reqwest::Url;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;

use crate::Ctx;
use crate::connections::{self, Service};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, now};

/// How often contacts are synced in the background.
const SYNC_EVERY: Duration = Duration::from_mins(15);

fn book_key(account_id: &str) -> String {
    format!("carddav:{account_id}")
}

fn delta_key(account_id: &str) -> String {
    format!("graph-contacts-delta:{account_id}")
}

/// Where an account's contacts live, for the Contacts screen.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddressBookStatus {
    account_id: String,
    /// `synced`, `unavailable` (no address book on the server, or not
    /// connected), or `error`.
    state: &'static str,
    /// Plain words for the user.
    message: Option<String>,
    /// A Gmail or Microsoft account that can be connected.
    connectable: bool,
}

/// `null` as `Some(None)`, a value as `Some(Some(value))`; a missing field
/// stays `None` (with `#[serde(default)]`).
#[allow(clippy::option_option, reason = "missing, null or a value")]
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
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
    /// Missing: as it was.
    #[serde(default)]
    favorite: Option<bool>,
    /// Missing: as they were.
    #[serde(default)]
    categories: Option<Vec<String>>,
    /// Missing: as it was; `null`: removed.
    #[allow(clippy::option_option, reason = "missing, null or a value")]
    #[serde(default, deserialize_with = "present")]
    photo: Option<Option<String>>,
}

/// A group as the editor sends it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInput {
    id: Option<String>,
    account_id: Option<String>,
    name: String,
    member_ids: Vec<String>,
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

fn card_from(input: &ContactInput, old: Option<&ContactCard>) -> AppResult<ContactCard> {
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
    let categories = match &input.categories {
        Some(names) => {
            let mut out: Vec<String> = Vec::new();
            for name in names {
                let name = tidy(name).replace('\n', " ");
                if !name.is_empty() && !out.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                    out.push(name);
                }
            }
            out.truncate(MAX_ITEMS);
            out
        }
        None => old.map(|o| o.categories.clone()).unwrap_or_default(),
    };
    let photo = match &input.photo {
        None => old.and_then(|o| o.photo.clone()),
        Some(None) => None,
        Some(Some(url)) => Some(vcard::checked_photo(url).map_err(AppError::invalid)?),
    };
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
        favorite: input
            .favorite
            .unwrap_or_else(|| old.is_some_and(|o| o.favorite)),
        categories,
        photo,
    };
    if card.name.is_empty() && card.emails.is_empty() && card.phones.is_empty() {
        return Err(AppError::invalid(
            "Add a name, an email address or a phone number.",
        ));
    }
    Ok(card)
}

/// `list_contacts`: saved contacts matching `query`, favourites first, then
/// by name.
pub fn list_contacts(ctx: &Ctx, query: String) -> AppResult<Vec<Contact>> {
    let state = &ctx.state;
    Ok(state.store.list_contacts(&query, MAX_CONTACTS)?)
}

/// `get_contact`.
pub fn get_contact(ctx: &Ctx, id: String) -> AppResult<Contact> {
    let state = &ctx.state;
    Ok(state.store.get_contact(&id)?)
}

/// `save_contact`: create or update; synced on the next contacts sync.
pub fn save_contact(ctx: &Ctx, contact: ContactInput) -> AppResult<Contact> {
    let state = &ctx.state;
    let old = match contact.id.as_deref() {
        Some(id) => Some(state.store.get_contact(id)?.card),
        None => None,
    };
    let card = card_from(&contact, old.as_ref())?;
    let saved = state.store.save_contact(
        contact.id.as_deref(),
        contact.account_id.as_deref(),
        card,
        now(),
    )?;
    soon(ctx, state);
    Ok(saved)
}

/// `delete_contact`: removed here at once, from the server on the next sync.
pub fn delete_contact(ctx: &Ctx, id: String) -> AppResult<()> {
    let state = &ctx.state;
    state.store.delete_contact(&id)?;
    soon(ctx, state);
    Ok(())
}

/// `list_contact_groups`: every group, by name.
pub fn list_contact_groups(ctx: &Ctx) -> AppResult<Vec<ContactGroup>> {
    let state = &ctx.state;
    Ok(state.store.list_contact_groups()?)
}

/// `save_contact_group`: create or change a group. Groups of password and
/// Gmail accounts sync with their address book; Microsoft has no groups
/// the app can use, so those are kept on this device.
pub fn save_contact_group(ctx: &Ctx, group: GroupInput) -> AppResult<Vec<ContactGroup>> {
    let state = &ctx.state;
    if let Some(account_id) = group.account_id.as_deref() {
        let account = state.store.get_account(account_id)?;
        if account.auth == mail_store::AuthKind::Microsoft {
            return Err(AppError::invalid(
                "Microsoft accounts have no contact groups Mocky Mail can use. Keep the group on this device instead.",
            ));
        }
    }
    let saved = state.store.save_contact_group(
        group.id.as_deref(),
        group.account_id.as_deref(),
        &group.name,
        &group.member_ids,
        now(),
    );
    match saved {
        Ok(_) => {}
        Err(mail_store::StoreError::Invalid(why)) => {
            return Err(AppError::invalid(match why {
                "a group needs a name" => "Give the group a name.",
                "too many people in one group" => "A group can hold up to 1000 people.",
                "too many groups" => "There are too many groups. Delete some first.",
                "a group can't move between address books" => {
                    "A group can't move to another address book."
                }
                _ => "A group of an account's address book can only hold that account's contacts.",
            }));
        }
        Err(other) => return Err(other.into()),
    }
    soon(ctx, state);
    Ok(state.store.list_contact_groups()?)
}

/// `delete_contact_group`: the group goes (its people stay).
pub fn delete_contact_group(ctx: &Ctx, id: String) -> AppResult<Vec<ContactGroup>> {
    let state = &ctx.state;
    state.store.delete_contact_group(&id)?;
    soon(ctx, state);
    Ok(state.store.list_contact_groups()?)
}

/// `sync_contacts`: sync every account's address book now.
pub async fn sync_contacts(ctx: &Ctx) -> AppResult<Vec<AddressBookStatus>> {
    let state = &ctx.state;
    Ok(sync_all(ctx, state).await)
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
pub(crate) fn soon(ctx: &Ctx, state: &Arc<AppState>) {
    let (task_ctx, state) = (ctx.clone(), Arc::clone(state));
    ctx.spawn(async move {
        sync_all(&task_ctx, &state).await;
    });
}

/// Background sync: once at start-up, then every 15 minutes.
pub fn start(ctx: Ctx, state: Arc<AppState>) {
    ctx.clone().spawn(async move {
        loop {
            sync_all(&ctx, &state).await;
            tokio::time::sleep(SYNC_EVERY).await;
        }
    });
}

pub(crate) async fn sync_all(ctx: &Ctx, state: &Arc<AppState>) -> Vec<AddressBookStatus> {
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
        let (status, message) = match sync_account(state, &account).await {
            Ok(None) => ("synced", None),
            Ok(Some(reason)) => ("unavailable", Some(reason)),
            Err(error) => {
                eprintln!("contacts: {}: {error}", account.email);
                ("error", Some(error.message))
            }
        };
        out.push(AddressBookStatus {
            account_id: account.id.clone(),
            state: status,
            message,
            connectable: connections::connectable(state, &account),
        });
    }
    if let Err(error) = ctx.emit("mail://contacts-changed", json!({ "books": out })) {
        eprintln!("couldn't emit contacts-changed: {error}");
    }
    out
}

/// Delete a card on the server. One the server refuses to delete is
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
            eprintln!("contacts: the server kept a deleted card: {reason}");
            Ok(())
        }
        other => other,
    }
}

/// Sync one account. `Ok(Some(reason))` when it has no address book here.
async fn sync_account(state: &AppState, account: &Account) -> AppResult<Option<String>> {
    match connections::service(state, account)? {
        Service::NotConnected => Ok(Some(
            "Contacts for this account are kept on this device. Connect its calendar and contacts to sync them.".to_owned(),
        )),
        Service::Microsoft => sync_microsoft(state, account).await.map(|()| None),
        Service::Google => {
            let token = connections::google_token(state, account).await?;
            let client = DavClient::bearer(token)?;
            let book = Url::parse(&google::carddav_book_url(&account.email))
                .map_err(|e| AppError::internal(e.to_string()))?;
            sync_dav_book(state, account, &client, &book, false).await?;
            let api = connections::google_client(state, account).await?;
            sync_google_groups(state, account, &api).await?;
            Ok(None)
        }
        Service::Dav => {
            let Credentials::Password { username, password } =
                crate::credentials::for_account(state, account).await?
            else {
                return Ok(Some(
                    "This account has no password to sync contacts with.".to_owned(),
                ));
            };
            let client = DavClient::new(DavLogin { username, password })?;
            let Some(book) = address_book(state, &client, account).await? else {
                return Ok(Some(
                    "The mail server doesn't offer an address book.".to_owned(),
                ));
            };
            sync_dav_book(state, account, &client, &book, true).await?;
            Ok(None)
        }
    }
}

/// Send what changed here to a `CardDAV` address book, then read what
/// changed there. With `groups`, contact groups are kept in the book as
/// group cards too (Google keeps them apart, in the People API).
#[allow(clippy::too_many_lines, reason = "send, then read, as the steps go")]
async fn sync_dav_book(
    state: &AppState,
    account: &Account,
    client: &DavClient,
    book: &Url,
    groups: bool,
) -> AppResult<()> {
    let keep_local = !groups;
    // Send what changed here first.
    for pending in state.store.contacts_to_push(&account.id)? {
        let id = pending.contact.id.clone();
        match pending.contact.state {
            ContactState::Deleted => {
                if let Some(href) = &pending.href {
                    push_delete(client, book, href, pending.etag.as_deref()).await?;
                }
                state.store.forget_contact(&id)?;
            }
            ContactState::Changed => {
                let text = vcard::write(&pending.contact.card, pending.vcard.as_deref());
                let uid = &pending.contact.card.uid;
                let put = client
                    .put_card(
                        book,
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
                            .put_card(book, pending.href.as_deref(), uid, &text, None)
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
    if groups {
        push_dav_groups(state, account, client, book).await?;
    }

    // Then read what changed on the server.
    let listed = match client.list_cards(book).await {
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
    let known_groups = state.store.group_etags(&account.id)?;
    let wanted: Vec<String> = listed
        .iter()
        .filter(
            |(href, etag)| match (known.get(href), known_groups.get(href)) {
                (Some((known_etag, _)), _) | (None, Some(known_etag)) => {
                    etag.is_none() || known_etag != etag
                }
                (None, None) => true,
            },
        )
        .map(|(href, _)| href.clone())
        .collect();
    let mut group_cards: Vec<(String, Option<String>, String, GroupCard)> = Vec::new();
    for remote in client.fetch_cards(book, &wanted).await? {
        if let Some(group) = vcard::parse_group(&remote.vcard) {
            group_cards.push((remote.href, remote.etag, remote.vcard, group));
        } else if let Some(card) = vcard::parse(&remote.vcard) {
            let (href, etag) = (&remote.href, remote.etag.as_deref());
            if keep_local {
                state.store.apply_server_contact_keeping_photo(
                    &account.id,
                    href,
                    etag,
                    &remote.vcard,
                    &card,
                    now(),
                )?;
            } else {
                state.store.apply_server_contact(
                    &account.id,
                    href,
                    etag,
                    &remote.vcard,
                    &card,
                    now(),
                )?;
            }
        }
    }
    // Groups after their members, so they can be found.
    for (href, etag, text, group) in group_cards {
        let members = state
            .store
            .contact_ids_by_uid(Some(&account.id), &group.members)?;
        state.store.apply_server_group(
            &account.id,
            &href,
            etag.as_deref(),
            Some(&text),
            &group.uid,
            &group.name,
            &members,
            now(),
        )?;
    }
    let on_server: Vec<String> = listed.into_iter().map(|(href, _)| href).collect();
    state
        .store
        .forget_contacts_not_on_server(&account.id, &on_server)?;
    if groups {
        state
            .store
            .forget_groups_not_on_server(&account.id, &on_server)?;
    }
    Ok(())
}

/// Send an account's group changes to its `CardDAV` book as group cards.
async fn push_dav_groups(
    state: &AppState,
    account: &Account,
    client: &DavClient,
    book: &Url,
) -> AppResult<()> {
    for pending in state.store.groups_to_push(&account.id)? {
        let id = pending.group.id.clone();
        match pending.state {
            ContactState::Deleted => {
                if let Some(href) = &pending.href {
                    push_delete(client, book, href, pending.etag.as_deref()).await?;
                }
                state.store.forget_contact_group(&id)?;
            }
            ContactState::Changed => {
                let members = state
                    .store
                    .contacts_by_id(&pending.group.member_ids)?
                    .into_iter()
                    .map(|c| c.card.uid)
                    .collect();
                let card = GroupCard {
                    uid: pending.uid.clone(),
                    name: pending.group.name.clone(),
                    members,
                };
                let text = vcard::write_group(&card, pending.vcard.as_deref());
                let put = client
                    .put_card(
                        book,
                        pending.href.as_deref(),
                        &card.uid,
                        &text,
                        pending.etag.as_deref(),
                    )
                    .await;
                let (href, etag) = match put {
                    Err(SyncError::Refused(_)) if pending.href.is_some() => {
                        client
                            .put_card(book, pending.href.as_deref(), &card.uid, &text, None)
                            .await?
                    }
                    // A server that won't keep group cards: the group stays
                    // here, waiting, rather than failing every sync.
                    Err(SyncError::Refused(reason)) => {
                        eprintln!("contacts: the server didn't keep a group: {reason}");
                        continue;
                    }
                    other => other?,
                };
                state.store.group_pushed(
                    &id,
                    &href,
                    etag.as_deref(),
                    Some(&text),
                    &pending.group,
                )?;
            }
            ContactState::Synced => {}
        }
    }
    Ok(())
}

/// The People API's people (`people/…`) for an account's contacts, by
/// matching addresses: local contact id to resource name, and back.
fn match_people(
    state: &AppState,
    account: &Account,
    emails: &HashMap<String, Vec<String>>,
) -> AppResult<(HashMap<String, String>, HashMap<String, String>)> {
    let contacts: Vec<Contact> = state
        .store
        .list_contacts("", MAX_CONTACTS)?
        .into_iter()
        .filter(|c| c.account_id.as_deref() == Some(account.id.as_str()))
        .collect();
    let mut to_person = HashMap::new();
    let mut to_contact = HashMap::new();
    for (person, addresses) in emails {
        if let Some(contact) = contacts.iter().find(|c| {
            c.card
                .emails
                .iter()
                .any(|e| addresses.iter().any(|a| a.eq_ignore_ascii_case(e)))
        }) {
            to_person.insert(contact.id.clone(), person.clone());
            to_contact.insert(person.clone(), contact.id.clone());
        }
    }
    Ok((to_person, to_contact))
}

/// Sync a Gmail account's contact groups with the People API. People are
/// matched by email address, so contacts without one can't be in a
/// Google group.
async fn sync_google_groups(
    state: &AppState,
    account: &Account,
    api: &RestClient,
) -> AppResult<()> {
    let (groups, emails, members) = google::list_groups(api).await?;
    let (to_person, to_contact) = match_people(state, account, &emails)?;
    for pending in state.store.groups_to_push(&account.id)? {
        let id = pending.group.id.clone();
        match pending.state {
            ContactState::Deleted => {
                if let Some(href) = &pending.href {
                    google::delete_group(api, href).await?;
                }
                state.store.forget_contact_group(&id)?;
            }
            ContactState::Changed => {
                let remote = match pending
                    .href
                    .as_deref()
                    .and_then(|h| groups.iter().find(|g| g.resource_name == h))
                {
                    Some(g) if g.name == pending.group.name => g.clone(),
                    Some(g) => google::rename_group(api, g, &pending.group.name).await?,
                    None => google::create_group(api, &pending.group.name).await?,
                };
                let now_in: Vec<String> = members
                    .get(&remote.resource_name)
                    .cloned()
                    .unwrap_or_default();
                let wanted: Vec<String> = pending
                    .group
                    .member_ids
                    .iter()
                    .filter_map(|m| to_person.get(m).cloned())
                    .collect();
                let add: Vec<String> = wanted
                    .iter()
                    .filter(|p| !now_in.contains(p))
                    .cloned()
                    .collect();
                let dropped: Vec<String> = now_in
                    .iter()
                    .filter(|p| !wanted.contains(p))
                    .cloned()
                    .collect();
                google::change_members(api, &remote.resource_name, &add, &dropped).await?;
                state.store.group_pushed(
                    &id,
                    &remote.resource_name,
                    remote.etag.as_deref(),
                    None,
                    &pending.group,
                )?;
            }
            ContactState::Synced => {}
        }
    }
    // What the server has (read again after the changes above, so new
    // groups are seen with their members).
    let (groups, emails, members) = google::list_groups(api).await?;
    let (_, refreshed) = match_people(state, account, &emails)?;
    let to_contact = if refreshed.is_empty() {
        to_contact
    } else {
        refreshed
    };
    for group in &groups {
        let ids: Vec<String> = members
            .get(&group.resource_name)
            .into_iter()
            .flatten()
            .filter_map(|p| to_contact.get(p).cloned())
            .collect();
        state.store.apply_server_group(
            &account.id,
            &group.resource_name,
            group.etag.as_deref(),
            None,
            &group.resource_name,
            &group.name,
            &ids,
            now(),
        )?;
    }
    let on_server: Vec<String> = groups.iter().map(|g| g.resource_name.clone()).collect();
    state
        .store
        .forget_groups_not_on_server(&account.id, &on_server)?;
    Ok(())
}

/// Sync a Microsoft account's contacts with Graph: changes here first,
/// then what changed there (a delta link remembers where it got to).
async fn sync_microsoft(state: &AppState, account: &Account) -> AppResult<()> {
    let client = connections::graph_client(state, account).await?;
    for pending in state.store.contacts_to_push(&account.id)? {
        let id = pending.contact.id.clone();
        match pending.contact.state {
            ContactState::Deleted => {
                if let Some(href) = &pending.href {
                    graph::delete_contact(&client, href).await?;
                }
                state.store.forget_contact(&id)?;
            }
            ContactState::Changed => {
                // The text the device compares against, as for CardDAV.
                let text = vcard::write(&pending.contact.card, pending.vcard.as_deref());
                let (href, etag) = match &pending.href {
                    Some(href) => {
                        let etag =
                            graph::update_contact(&client, href, &pending.contact.card).await?;
                        (href.clone(), etag)
                    }
                    None => graph::create_contact(&client, &pending.contact.card).await?,
                };
                state
                    .store
                    .contact_pushed(&id, &href, etag.as_deref(), &text)?;
            }
            ContactState::Synced => {}
        }
    }
    let saved = state
        .store
        .get_setting(&delta_key(&account.id))?
        .filter(|l| !l.is_empty());
    let (changed, removed, link, full) =
        match graph::contacts_delta(&client, saved.as_deref()).await {
            Ok((c, r, l)) => (c, r, l, saved.is_none()),
            // The link expired: read everything again.
            Err(SyncError::Refused(_)) if saved.is_some() => {
                let (c, r, l) = graph::contacts_delta(&client, None).await?;
                (c, r, l, true)
            }
            Err(other) => return Err(other.into()),
        };
    let mut listed = Vec::new();
    for item in &changed {
        if let Some((href, etag, card)) = graph::contact_from_json(item) {
            let text = vcard::write(&card, None);
            state.store.apply_server_contact_keeping_photo(
                &account.id,
                &href,
                etag.as_deref(),
                &text,
                &card,
                now(),
            )?;
            listed.push(href);
        }
    }
    for href in removed {
        state.store.forget_server_contact(&account.id, &href)?;
    }
    if full {
        state
            .store
            .forget_contacts_not_on_server(&account.id, &listed)?;
    }
    if let Some(link) = link {
        state.store.set_setting(&delta_key(&account.id), &link)?;
    }
    Ok(())
}

/// The account's address book URL: remembered, or found and remembered.
/// `None` when the server has none.
async fn address_book(
    state: &AppState,
    client: &DavClient,
    account: &Account,
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

/// The people in the group with this name, as recipients. `None` when no
/// group has that name.
pub fn group_recipients(
    state: &AppState,
    name: &str,
) -> AppResult<Option<Vec<mail_core::message::Mailbox>>> {
    Ok(state.store.group_members_named(name)?.map(|members| {
        members
            .into_iter()
            .filter_map(|c| {
                let address = c.card.emails.first()?.clone();
                Some(mail_core::message::Mailbox {
                    name: Some(c.card.name).filter(|n| !n.is_empty()),
                    address,
                })
            })
            .collect()
    }))
}

/// Saved contacts first (favourites leading), the people in groups whose
/// name matches, then people from mail, for recipient suggestions.
pub fn suggestions(
    state: &AppState,
    query: &str,
    limit: usize,
) -> AppResult<Vec<mail_core::message::Mailbox>> {
    let mut out: Vec<mail_core::message::Mailbox> = Vec::new();
    if query.trim().is_empty() {
        return Ok(state.store.suggest_recipients(query, limit)?);
    }
    let mut add = |mailbox: mail_core::message::Mailbox| {
        if !out
            .iter()
            .any(|m| m.address.eq_ignore_ascii_case(&mailbox.address))
        {
            out.push(mailbox);
        }
    };
    for group in state.store.groups_matching(query, 5)? {
        for contact in state.store.contacts_by_id(&group.member_ids)? {
            if let Some(email) = contact.card.emails.first() {
                add(mail_core::message::Mailbox {
                    name: Some(contact.card.name.clone()).filter(|n| !n.is_empty()),
                    address: email.clone(),
                });
            }
        }
    }
    for contact in state.store.list_contacts(query, limit)? {
        for email in &contact.card.emails {
            add(mail_core::message::Mailbox {
                name: Some(contact.card.name.clone()).filter(|n| !n.is_empty()),
                address: email.clone(),
            });
        }
    }
    for person in state.store.suggest_recipients(query, limit)? {
        add(person);
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
            favorite: None,
            categories: None,
            photo: None,
        }
    }

    #[test]
    fn typed_contacts_are_tidied_and_checked() {
        let card =
            card_from(&input("  Peter Mwangi ", "peter@client.example"), None).unwrap_or_default();
        assert_eq!(card.name, "Peter Mwangi");
        assert_eq!(card.phones, ["+254 700 000 001"]);
        assert_eq!(card.organization, "MockyDigital");
        assert_eq!(card.notes, "Line one\nLine two");
        assert!(card_from(&input("Peter", "not an address"), None).is_err());
        assert!(
            card_from(
                &ContactInput {
                    phones: vec![],
                    ..input("", "")
                },
                None
            )
            .is_err()
        );
    }

    #[test]
    fn missing_fields_keep_what_the_contact_had() {
        let old = ContactCard {
            favorite: true,
            categories: vec!["Clients".into()],
            photo: Some("data:image/png;base64,AAAA".into()),
            ..ContactCard::default()
        };
        let kept =
            card_from(&input("Peter", "peter@client.example"), Some(&old)).unwrap_or_default();
        assert!(kept.favorite);
        assert_eq!(kept.categories, ["Clients"]);
        assert_eq!(kept.photo, old.photo);
        let removed = card_from(
            &ContactInput {
                photo: Some(None),
                favorite: Some(false),
                categories: Some(vec![" Red ".into(), "red".into()]),
                ..input("Peter", "peter@client.example")
            },
            Some(&old),
        )
        .unwrap_or_default();
        assert_eq!(removed.photo, None);
        assert!(!removed.favorite);
        assert_eq!(removed.categories, ["Red"]);
        let bad = ContactInput {
            photo: Some(Some("https://x.example/a.png".into())),
            ..input("Peter", "peter@client.example")
        };
        assert!(card_from(&bad, None).is_err());
        // `photo: null` is told apart from no `photo` at all.
        let json: ContactInput = serde_json::from_str(
            r#"{"accountId":null,"name":"A","emails":[],"phones":[],"organization":"","title":"","notes":"","photo":null}"#,
        )
        .unwrap_or_else(|_| input("x", "x@y.co"));
        assert_eq!(json.photo, Some(None));
        let json: ContactInput = serde_json::from_str(
            r#"{"accountId":null,"name":"A","emails":[],"phones":[],"organization":"","title":"","notes":""}"#,
        )
        .unwrap_or_else(|_| input("x", "x@y.co"));
        assert_eq!(json.photo, None);
    }
}
