//! The address book: contacts kept on this device, and contacts synced with
//! an account's server (`CardDAV`).
//!
//! A contact belongs to an account (and is synced to its address book) or
//! to no account (this device only). Changes are made here first and marked
//! `changed` or `deleted`; the sync pushes them, then marks them `synced`.
//! Until then a server copy never overwrites a local change.

use std::collections::HashMap;

use mail_core::vcard::ContactCard;
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::Store;
use crate::error::{Result, StoreError};
use crate::ids::new_id;

/// Seconds since 1970.
type UnixSeconds = u64;

/// Most contacts one list returns.
pub const MAX_CONTACTS: usize = 2000;

/// Where a contact is in syncing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactState {
    /// Same as the server (or kept on this device only).
    Synced,
    /// Edited or created here; not on the server yet.
    Changed,
    /// Deleted here; the server copy is still to be deleted.
    Deleted,
}

impl ContactState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Synced => "synced",
            Self::Changed => "changed",
            Self::Deleted => "deleted",
        }
    }

    fn parse(s: &str) -> Result<Self> {
        match s {
            "synced" => Ok(Self::Synced),
            "changed" => Ok(Self::Changed),
            "deleted" => Ok(Self::Deleted),
            _ => Err(StoreError::Corrupt("contact state")),
        }
    }
}

/// A contact as the app shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    /// Local id.
    pub id: String,
    /// The account whose address book holds it; `None`: this device only.
    pub account_id: Option<String>,
    /// The fields shown and edited.
    #[serde(flatten)]
    pub card: ContactCard,
    /// Whether a change is still to be sent.
    pub state: ContactState,
}

/// A contact's server side, for syncing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactSync {
    /// The contact.
    pub contact: Contact,
    /// Its path on the server, once it is there.
    pub href: Option<String>,
    /// The server's version tag for that copy.
    pub etag: Option<String>,
    /// The server's card text, so fields the app doesn't show are kept.
    pub vcard: Option<String>,
}

/// id, account, card JSON, state, href, etag, vCard.
type RawRow = (
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

const COLUMNS: &str = "id, account_id, card, state, href, etag, vcard";

fn from_row(row: &Row<'_>) -> rusqlite::Result<RawRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

fn to_sync(raw: RawRow) -> Result<ContactSync> {
    let (id, account_id, card, state, href, etag, vcard) = raw;
    Ok(ContactSync {
        contact: Contact {
            id,
            account_id,
            card: serde_json::from_str(&card)?,
            state: ContactState::parse(&state)?,
        },
        href,
        etag,
        vcard,
    })
}

/// Lower-case text a search matches against.
fn search_text(card: &ContactCard) -> String {
    let mut text = card.name.to_lowercase();
    for part in card
        .emails
        .iter()
        .chain(&card.phones)
        .chain([&card.organization, &card.title])
    {
        text.push(' ');
        text.push_str(&part.to_lowercase());
    }
    text
}

/// `%`, `_` and `\` escaped for a `LIKE … ESCAPE '\'` pattern.
fn like_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn sort_name(card: &ContactCard) -> String {
    if card.name.trim().is_empty() {
        card.emails.first().cloned().unwrap_or_default()
    } else {
        card.name.trim().to_owned()
    }
}

impl Store {
    /// Contacts matching `query` (name, email, phone, company), by name.
    /// Deleted ones are left out.
    ///
    /// # Errors
    /// Database errors.
    pub fn list_contacts(&self, query: &str, limit: usize) -> Result<Vec<Contact>> {
        let pattern = format!("%{}%", like_escape(&query.trim().to_lowercase()));
        let limit = i64::try_from(limit.clamp(1, MAX_CONTACTS)).unwrap_or(1);
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM contacts
             WHERE state != 'deleted' AND search LIKE ?1 ESCAPE '\\'
             ORDER BY coalesce(json_extract(card, '$.favorite'), 0) DESC,
                 sort_name COLLATE NOCASE, id LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![pattern, limit], from_row)?;
        rows.map(|row| Ok(to_sync(row?)?.contact)).collect()
    }

    /// One contact.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] when there is no such contact.
    pub fn get_contact(&self, id: &str) -> Result<Contact> {
        Ok(self.contact_sync(id)?.contact)
    }

    fn contact_sync(&self, id: &str) -> Result<ContactSync> {
        let raw = self
            .conn()
            .query_row(
                &format!("SELECT {COLUMNS} FROM contacts WHERE id = ?1 AND state != 'deleted'"),
                params![id],
                from_row,
            )
            .optional()?
            .ok_or(StoreError::NotFound("contact"))?;
        to_sync(raw)
    }

    /// Create (`id` `None`) or update a contact. Contacts of an account are
    /// marked to be sent to its server; device-only ones are simply saved.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown id or account.
    pub fn save_contact(
        &self,
        id: Option<&str>,
        account_id: Option<&str>,
        mut card: ContactCard,
        now: UnixSeconds,
    ) -> Result<Contact> {
        let now = crate::ids::to_i64(now, "time")?;
        let state = if account_id.is_some() {
            ContactState::Changed
        } else {
            ContactState::Synced
        };
        let id = if let Some(id) = id {
            let old = self.contact_sync(id)?;
            card.uid.clone_from(&old.contact.card.uid);
            if old.contact.account_id.as_deref() != account_id {
                return Err(StoreError::Invalid(
                    "a contact can't move between address books",
                ));
            }
            id.to_owned()
        } else {
            if let Some(account) = account_id {
                self.get_account(account)?;
            }
            let id = new_id()?;
            if card.uid.trim().is_empty() {
                card.uid = format!("{id}-mocky");
            }
            self.conn().execute(
                "INSERT INTO contacts (id, account_id, card, state, sort_name, search, updated_at)
                 VALUES (?1, ?2, '{}', 'synced', '', '', ?3)",
                params![id, account_id, now],
            )?;
            id
        };
        self.conn().execute(
            "UPDATE contacts SET card = ?2, state = ?3, sort_name = ?4, search = ?5, updated_at = ?6
             WHERE id = ?1",
            params![
                id,
                serde_json::to_string(&card)?,
                state.as_str(),
                sort_name(&card),
                search_text(&card),
                now
            ],
        )?;
        self.get_contact(&id)
    }

    /// Delete a contact. One on a server is kept, marked `deleted`, until the
    /// sync deletes the server copy. So is an account's contact still
    /// waiting to be sent: it may be on its way to the server right now,
    /// and [`Store::contact_pushed`] needs the row to learn where it went.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown id.
    pub fn delete_contact(&self, id: &str) -> Result<()> {
        let sync = self.contact_sync(id)?;
        let sending =
            sync.contact.account_id.is_some() && sync.contact.state == ContactState::Changed;
        if sync.href.is_some() || sending {
            self.conn().execute(
                "UPDATE contacts SET state = 'deleted' WHERE id = ?1",
                params![id],
            )?;
        } else {
            self.conn()
                .execute("DELETE FROM contacts WHERE id = ?1", params![id])?;
        }
        Ok(())
    }

    /// An account's contacts with changes to send (created, edited, deleted).
    ///
    /// # Errors
    /// Database errors.
    pub fn contacts_to_push(&self, account_id: &str) -> Result<Vec<ContactSync>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM contacts WHERE account_id = ?1 AND state != 'synced'"
        ))?;
        let rows = stmt.query_map(params![account_id], from_row)?;
        rows.map(|row| to_sync(row?)).collect()
    }

    /// The server copies known for an account: href to (etag, state).
    ///
    /// # Errors
    /// Database errors.
    pub fn contact_etags(
        &self,
        account_id: &str,
    ) -> Result<HashMap<String, (Option<String>, ContactState)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT href, etag, state FROM contacts WHERE account_id = ?1 AND href IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let (href, etag, state) = row?;
            out.insert(href, (etag, ContactState::parse(&state)?));
        }
        Ok(out)
    }

    /// Store a card read from the server. A contact with a change still to
    /// send is left alone; the push wins.
    ///
    /// # Errors
    /// Database errors.
    pub fn apply_server_contact(
        &self,
        account_id: &str,
        href: &str,
        etag: Option<&str>,
        vcard: &str,
        card: &ContactCard,
        now: UnixSeconds,
    ) -> Result<()> {
        self.apply_server_card(account_id, href, etag, vcard, card, now, false)
    }

    /// [`Store::apply_server_contact`] for a server that doesn't keep the
    /// picture and favourite set here (Google, Microsoft): a picture the
    /// device has is kept when the server's copy comes without one, and so
    /// is a favourite.
    ///
    /// # Errors
    /// Database errors.
    pub fn apply_server_contact_keeping_photo(
        &self,
        account_id: &str,
        href: &str,
        etag: Option<&str>,
        vcard: &str,
        card: &ContactCard,
        now: UnixSeconds,
    ) -> Result<()> {
        self.apply_server_card(account_id, href, etag, vcard, card, now, true)
    }

    #[allow(clippy::too_many_arguments, reason = "one private helper")]
    fn apply_server_card(
        &self,
        account_id: &str,
        href: &str,
        etag: Option<&str>,
        vcard: &str,
        card: &ContactCard,
        now: UnixSeconds,
        keep_photo: bool,
    ) -> Result<()> {
        let now = crate::ids::to_i64(now, "time")?;
        let existing: Option<(String, String, String)> = self
            .conn()
            .query_row(
                "SELECT id, state, card FROM contacts WHERE account_id = ?1 AND href = ?2",
                params![account_id, href],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let mut card = card.clone();
        if keep_photo
            && let Some((_, _, old)) = &existing
            && let Ok(old) = serde_json::from_str::<ContactCard>(old)
        {
            if card.photo.is_none() {
                card.photo = old.photo;
            }
            card.favorite = card.favorite || old.favorite;
        }
        let card = &card;
        let card_json = serde_json::to_string(card)?;
        match existing {
            Some((_, state, _)) if state != "synced" => {}
            Some((id, _, _)) => {
                self.conn().execute(
                    "UPDATE contacts SET card = ?2, etag = ?3, vcard = ?4, sort_name = ?5,
                        search = ?6, updated_at = ?7 WHERE id = ?1",
                    params![
                        id,
                        card_json,
                        etag,
                        vcard,
                        sort_name(card),
                        search_text(card),
                        now
                    ],
                )?;
            }
            None => {
                self.conn().execute(
                    "INSERT INTO contacts (id, account_id, href, etag, vcard, card, state, sort_name, search, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'synced', ?7, ?8, ?9)",
                    params![
                        new_id()?,
                        account_id,
                        href,
                        etag,
                        vcard,
                        card_json,
                        sort_name(card),
                        search_text(card),
                        now
                    ],
                )?;
            }
        }
        Ok(())
    }

    /// A change reached the server: remember where, and its new version.
    /// `vcard` is the text that was sent, which must be
    /// `mail_core::vcard::write(card, server copy)` for the contact as it
    /// was read from [`Store::contacts_to_push`].
    ///
    /// The contact is only marked `synced` if it is still what was sent.
    /// Sending takes a while, and one edited or deleted here meanwhile
    /// stays `changed` or `deleted` (now with its place on the server), so
    /// the next sync sends that too instead of losing it.
    ///
    /// # Errors
    /// Database errors.
    pub fn contact_pushed(
        &self,
        id: &str,
        href: &str,
        etag: Option<&str>,
        vcard: &str,
    ) -> Result<()> {
        // One lock for the check and the update.
        let conn = self.conn();
        let row: Option<(String, String, Option<String>)> = conn
            .query_row(
                "SELECT card, state, vcard FROM contacts WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((card, state, old_vcard)) = row else {
            return Ok(());
        };
        let unchanged = state == ContactState::Changed.as_str()
            && serde_json::from_str::<ContactCard>(&card)
                .is_ok_and(|card| mail_core::vcard::write(&card, old_vcard.as_deref()) == vcard);
        let state = if unchanged {
            ContactState::Synced.as_str()
        } else {
            state.as_str()
        };
        conn.execute(
            "UPDATE contacts SET state = ?5, href = ?2, etag = ?3, vcard = ?4 WHERE id = ?1",
            params![id, href, etag, vcard, state],
        )?;
        Ok(())
    }

    /// Remove a contact for good (its server copy is gone).
    ///
    /// # Errors
    /// Database errors.
    pub fn forget_contact(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM contacts WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Drop the synced contact at `href` (deleted on the server). One with
    /// a change still to send is kept.
    ///
    /// # Errors
    /// Database errors.
    pub fn forget_server_contact(&self, account_id: &str, href: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM contacts WHERE account_id = ?1 AND href = ?2 AND state = 'synced'",
            params![account_id, href],
        )?;
        Ok(())
    }

    /// The ids of the contacts with these `UID`s in one address book (an
    /// account's, or with `None` this device's), in the order asked.
    ///
    /// # Errors
    /// Database errors.
    pub fn contact_ids_by_uid(
        &self,
        account_id: Option<&str>,
        uids: &[String],
    ) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id FROM contacts WHERE account_id IS ?1 AND state != 'deleted'
             AND json_extract(card, '$.uid') = ?2",
        )?;
        let mut out = Vec::new();
        for uid in uids {
            let found: Option<String> = stmt
                .query_row(params![account_id, uid], |row| row.get(0))
                .optional()?;
            if let Some(id) = found
                && !out.contains(&id)
            {
                out.push(id);
            }
        }
        Ok(out)
    }

    /// Contacts by id, in the order asked; deleted and unknown ones are
    /// left out.
    ///
    /// # Errors
    /// Database errors.
    pub fn contacts_by_id(&self, ids: &[String]) -> Result<Vec<Contact>> {
        let mut out = Vec::new();
        for id in ids {
            match self.get_contact(id) {
                Ok(contact) => out.push(contact),
                Err(StoreError::NotFound(_)) => {}
                Err(other) => return Err(other),
            }
        }
        Ok(out)
    }

    /// Drop synced contacts whose server copy no longer exists (deleted in
    /// webmail or another app). Pending local changes are kept.
    ///
    /// # Errors
    /// Database errors.
    pub fn forget_contacts_not_on_server(
        &self,
        account_id: &str,
        on_server: &[String],
    ) -> Result<usize> {
        let known = self.contact_etags(account_id)?;
        let mut removed = 0usize;
        for (href, (_, state)) in known {
            if state == ContactState::Synced && !on_server.contains(&href) {
                removed = removed.saturating_add(self.conn().execute(
                    "DELETE FROM contacts WHERE account_id = ?1 AND href = ?2",
                    params![account_id, href],
                )?);
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::test_support;

    fn new_account(store: &Store) -> String {
        store
            .insert_account(&test_support::new_account("me@work.example"))
            .unwrap()
            .id
    }

    /// The text the sync sends for a pending contact.
    fn sent(pending: &ContactSync) -> String {
        mail_core::vcard::write(&pending.contact.card, pending.vcard.as_deref())
    }

    fn card(name: &str, email: &str) -> ContactCard {
        ContactCard {
            name: name.into(),
            emails: vec![email.into()],
            ..ContactCard::default()
        }
    }

    #[test]
    fn device_contacts_are_saved_searched_and_deleted() {
        let store = Store::open_in_memory().unwrap();
        let wanjiru = store
            .save_contact(None, None, card("Wanjiru Kamau", "wanjiru@shop.example"), 1)
            .unwrap();
        store
            .save_contact(None, None, card("Amina Hassan", "amina@studio.example"), 1)
            .unwrap();
        assert_eq!(wanjiru.state, ContactState::Synced);
        assert!(!wanjiru.card.uid.is_empty());
        let all = store.list_contacts("", 100).unwrap();
        assert_eq!(
            all.iter().map(|c| c.card.name.as_str()).collect::<Vec<_>>(),
            ["Amina Hassan", "Wanjiru Kamau"]
        );
        assert_eq!(store.list_contacts("SHOP.ex", 100).unwrap().len(), 1);
        // LIKE wildcards in a search are plain text.
        assert!(store.list_contacts("%", 100).unwrap().is_empty());
        let mut edited = wanjiru.card.clone();
        edited.phones.push("+254 700 000 003".into());
        store
            .save_contact(Some(&wanjiru.id), None, edited, 2)
            .unwrap();
        assert_eq!(store.list_contacts("000 003", 100).unwrap().len(), 1);
        store.delete_contact(&wanjiru.id).unwrap();
        assert!(matches!(
            store.get_contact(&wanjiru.id),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn account_contacts_wait_for_the_server() {
        let store = Store::open_in_memory().unwrap();
        let account = new_account(&store);
        let c = store
            .save_contact(
                None,
                Some(&account),
                card("Peter Mwangi", "peter@client.example"),
                1,
            )
            .unwrap();
        assert_eq!(c.state, ContactState::Changed);
        let push = store.contacts_to_push(&account).unwrap();
        assert_eq!(push.len(), 1);
        store
            .contact_pushed(&c.id, "/dav/peter.vcf", Some("\"e1\""), &sent(&push[0]))
            .unwrap();
        assert!(store.contacts_to_push(&account).unwrap().is_empty());
        // Deleting keeps it until the server copy is gone.
        store.delete_contact(&c.id).unwrap();
        assert!(store.list_contacts("", 10).unwrap().is_empty());
        let push = store.contacts_to_push(&account).unwrap();
        assert_eq!(
            push.first().map(|p| p.contact.state),
            Some(ContactState::Deleted)
        );
        store.forget_contact(&c.id).unwrap();
        assert!(store.contacts_to_push(&account).unwrap().is_empty());
    }

    #[test]
    fn a_change_made_while_sending_is_not_lost() {
        let store = Store::open_in_memory().unwrap();
        let account = new_account(&store);
        let c = store
            .save_contact(
                None,
                Some(&account),
                card("Peter", "peter@client.example"),
                1,
            )
            .unwrap();
        // The sync reads the contact and starts sending it...
        let push = store.contacts_to_push(&account).unwrap();
        let text = sent(&push[0]);
        // ...and it is edited before the server answers.
        let mut edited = c.card.clone();
        edited.title = "Site manager".into();
        store
            .save_contact(Some(&c.id), Some(&account), edited, 2)
            .unwrap();
        store
            .contact_pushed(&c.id, "/dav/peter.vcf", Some("\"e1\""), &text)
            .unwrap();
        // Still to send, now as an update of the server's copy.
        let push = store.contacts_to_push(&account).unwrap();
        assert_eq!(push.len(), 1);
        assert_eq!(push[0].contact.state, ContactState::Changed);
        assert_eq!(push[0].contact.card.title, "Site manager");
        assert_eq!(push[0].href.as_deref(), Some("/dav/peter.vcf"));
        assert_eq!(push[0].etag.as_deref(), Some("\"e1\""));
        assert_eq!(push[0].vcard.as_deref(), Some(text.as_str()));
        // The second send goes through untouched and settles it.
        let text = sent(&push[0]);
        assert!(text.contains("TITLE:Site manager"));
        store
            .contact_pushed(&c.id, "/dav/peter.vcf", Some("\"e2\""), &text)
            .unwrap();
        assert!(store.contacts_to_push(&account).unwrap().is_empty());
        assert_eq!(
            store.get_contact(&c.id).unwrap().state,
            ContactState::Synced
        );
    }

    #[test]
    fn a_contact_deleted_while_first_sending_is_deleted_on_the_server_too() {
        let store = Store::open_in_memory().unwrap();
        let account = new_account(&store);
        let c = store
            .save_contact(
                None,
                Some(&account),
                card("Peter", "peter@client.example"),
                1,
            )
            .unwrap();
        let push = store.contacts_to_push(&account).unwrap();
        let text = sent(&push[0]);
        store.delete_contact(&c.id).unwrap();
        assert!(store.list_contacts("", 10).unwrap().is_empty());
        store
            .contact_pushed(&c.id, "/dav/peter.vcf", Some("\"e1\""), &text)
            .unwrap();
        // It stays deleted, and the sync knows which server copy to remove.
        let push = store.contacts_to_push(&account).unwrap();
        assert_eq!(push.len(), 1);
        assert_eq!(push[0].contact.state, ContactState::Deleted);
        assert_eq!(push[0].href.as_deref(), Some("/dav/peter.vcf"));
        assert!(store.list_contacts("", 10).unwrap().is_empty());
        // A device-only contact is still simply removed.
        let local = store
            .save_contact(None, None, card("Kept", "k@x.example"), 1)
            .unwrap();
        store.delete_contact(&local.id).unwrap();
        assert!(store.contacts_to_push(&account).unwrap().len() == 1);
        // A contact that is gone by the time the server answers is no error.
        store.forget_contact(&c.id).unwrap();
        store
            .contact_pushed(&c.id, "/dav/x.vcf", None, &text)
            .unwrap();
    }

    #[test]
    fn server_copies_never_overwrite_a_local_change() {
        let store = Store::open_in_memory().unwrap();
        let account = new_account(&store);
        store
            .apply_server_contact(
                &account,
                "/dav/a.vcf",
                Some("\"1\""),
                "v1",
                &card("Brian", "brian@x.example"),
                1,
            )
            .unwrap();
        let brian = store.list_contacts("brian", 10).unwrap().remove(0);
        let mut mine = brian.card.clone();
        mine.name = "Brian Otieno".into();
        store
            .save_contact(Some(&brian.id), Some(&account), mine, 2)
            .unwrap();
        store
            .apply_server_contact(
                &account,
                "/dav/a.vcf",
                Some("\"2\""),
                "v2",
                &card("Brian O.", "brian@x.example"),
                3,
            )
            .unwrap();
        assert_eq!(
            store.get_contact(&brian.id).unwrap().card.name,
            "Brian Otieno"
        );
        // Gone from the server and unchanged here: forgotten. Changed: kept.
        store
            .apply_server_contact(
                &account,
                "/dav/b.vcf",
                Some("\"1\""),
                "v1",
                &card("Old", "old@x.example"),
                1,
            )
            .unwrap();
        assert_eq!(
            store.forget_contacts_not_on_server(&account, &[]).unwrap(),
            1
        );
        assert_eq!(store.list_contacts("", 10).unwrap().len(), 1);
    }

    #[test]
    fn contacts_go_when_their_account_is_removed() {
        let store = Store::open_in_memory().unwrap();
        let account = new_account(&store);
        store
            .save_contact(None, Some(&account), card("Peter", "p@x.example"), 1)
            .unwrap();
        store
            .save_contact(None, None, card("Kept", "k@x.example"), 1)
            .unwrap();
        store.delete_account(&account).unwrap();
        assert_eq!(store.list_contacts("", 10).unwrap().len(), 1);
    }

    #[test]
    fn favourites_come_first_and_api_servers_keep_local_pictures() {
        let store = Store::open_in_memory().unwrap();
        let account = new_account(&store);
        store
            .save_contact(None, None, card("Amina", "amina@studio.example"), 1)
            .unwrap();
        let zed = ContactCard {
            favorite: true,
            ..card("Zawadi", "zawadi@shop.example")
        };
        store.save_contact(None, None, zed, 1).unwrap();
        let listed = store.list_contacts("", 10).unwrap();
        assert_eq!(listed[0].card.name, "Zawadi");

        // A Microsoft contact with a picture and favourite set here.
        let theirs = card("Peter", "peter@client.example");
        store
            .apply_server_contact_keeping_photo(&account, "AAMk1", Some("1"), "v", &theirs, 1)
            .unwrap();
        let id = store.list_contacts("peter", 1).unwrap()[0].id.clone();
        let mut mine = store.get_contact(&id).unwrap().card;
        mine.photo = Some("data:image/png;base64,AAAA".into());
        mine.favorite = true;
        store
            .save_contact(Some(&id), Some(&account), mine, 2)
            .unwrap();
        let pending = store.contacts_to_push(&account).unwrap();
        store
            .contact_pushed(&id, "AAMk1", Some("2"), &sent(&pending[0]))
            .unwrap();
        // The next download has neither: both stay.
        store
            .apply_server_contact_keeping_photo(&account, "AAMk1", Some("3"), "v", &theirs, 3)
            .unwrap();
        let kept = store.get_contact(&id).unwrap().card;
        assert!(kept.photo.is_some());
        assert!(kept.favorite);
        // Deleted on the server.
        store.forget_server_contact(&account, "AAMk1").unwrap();
        assert!(store.get_contact(&id).is_err());
        // Contacts by UID, for groups.
        let uid = listed[0].card.uid.clone();
        assert_eq!(
            store.contact_ids_by_uid(None, &[uid]).unwrap(),
            [listed[0].id.clone()]
        );
    }
}
