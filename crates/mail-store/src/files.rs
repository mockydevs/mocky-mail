//! What importing and exporting files needs from the database: a folder's
//! messages with their cached bodies and Message-IDs, a calendar's events,
//! an address book's contacts, and contacts brought in with their whole
//! vCard.

use std::collections::HashSet;

use mail_core::vcard::ContactCard;
use rusqlite::{OptionalExtension, params};

use crate::Store;
use crate::calendar::StoredEvent;
use crate::contacts::{Contact, ContactState, ContactSync};
use crate::error::{Result, StoreError};
use crate::ids::{new_id, to_u32};

/// The UIDs and (name, address) pairs of an address book.
pub type ContactKeys = (HashSet<String>, HashSet<(String, String)>);

/// A folder's message as an export reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct ExportRow {
    /// Local id.
    pub id: String,
    /// Server UID, when known.
    pub uid: Option<u32>,
    /// Read.
    pub seen: bool,
    /// Flagged.
    pub flagged: bool,
    /// Replied to.
    pub answered: bool,
    /// Whether the whole message is on this device.
    pub has_raw: bool,
}

/// Which address book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Book<'a> {
    /// This device's own contacts.
    Device,
    /// An account's.
    Account(&'a str),
    /// Every contact.
    All,
}

/// `Message-ID` without angle brackets or spaces, lower case: how
/// duplicates are compared.
#[must_use]
pub fn message_id_key(id: &str) -> String {
    id.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_ascii_lowercase()
}

impl Store {
    /// Every message of a folder known here, oldest UID first.
    ///
    /// # Errors
    /// Database errors.
    pub fn export_rows(&self, folder_id: &str) -> Result<Vec<ExportRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, uid, seen, flagged, answered, raw IS NOT NULL FROM messages
             WHERE folder_id = ?1 ORDER BY uid IS NULL, uid, seq",
        )?;
        let rows = stmt.query_map(params![folder_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, bool>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, bool>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })?;
        rows.map(|row| {
            let (id, uid, seen, flagged, answered, has_raw) = row?;
            Ok(ExportRow {
                id,
                uid: uid.map(|u| to_u32(u, "uid")).transpose()?,
                seen,
                flagged,
                answered,
                has_raw,
            })
        })
        .collect()
    }

    /// The cached message at a UID in a folder, if it is here.
    ///
    /// # Errors
    /// Database errors.
    pub fn raw_by_uid(&self, folder_id: &str, uid: u32) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT raw FROM messages WHERE folder_id = ?1 AND uid = ?2",
                params![folder_id, i64::from(uid)],
                |r| r.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()?
            .flatten())
    }

    /// The Message-IDs of the messages known in a folder (see
    /// [`message_id_key`]).
    ///
    /// # Errors
    /// Database errors.
    pub fn message_ids_in_folder(&self, folder_id: &str) -> Result<HashSet<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT message_id_header FROM messages
             WHERE folder_id = ?1 AND message_id_header IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![folder_id], |r| r.get::<_, String>(0))?;
        let mut out = HashSet::new();
        for id in rows {
            let key = message_id_key(&id?);
            if !key.is_empty() {
                out.insert(key);
            }
        }
        Ok(out)
    }

    /// Every stored event of one calendar.
    ///
    /// # Errors
    /// Database errors.
    pub fn calendar_events_in(&self, calendar_id: &str) -> Result<Vec<StoredEvent>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, calendar_id, href, etag, ics FROM calendar_events
             WHERE calendar_id = ?1 ORDER BY starts_at, id",
        )?;
        let rows = stmt.query_map(params![calendar_id], |r| {
            Ok(StoredEvent {
                id: r.get(0)?,
                calendar_id: r.get(1)?,
                href: r.get(2)?,
                etag: r.get(3)?,
                ics: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The contacts of an address book (not those deleted here), with the
    /// cards they were read from, by name.
    ///
    /// # Errors
    /// Database errors.
    pub fn contacts_in_book(&self, book: Book<'_>) -> Result<Vec<ContactSync>> {
        let ids: Vec<String> = {
            let conn = self.conn();
            let (which, account) = match book {
                Book::Device => ("device", None),
                Book::Account(id) => ("account", Some(id)),
                Book::All => ("all", None),
            };
            let mut stmt = conn.prepare(
                "SELECT id FROM contacts WHERE state != 'deleted'
                   AND (?1 = 'all' OR (?1 = 'device' AND account_id IS NULL)
                        OR (?1 = 'account' AND account_id = ?2))
                 ORDER BY sort_name COLLATE NOCASE, id",
            )?;
            let rows = stmt.query_map(params![which, account], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        ids.iter().map(|id| self.contact_with_card(id)).collect()
    }

    /// One contact with its card.
    fn contact_with_card(&self, id: &str) -> Result<ContactSync> {
        let (account_id, card, state, href, etag, vcard) = self
            .conn()
            .query_row(
                "SELECT account_id, card, state, href, etag, vcard FROM contacts WHERE id = ?1",
                params![id],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()?
            .ok_or(StoreError::NotFound("contact"))?;
        let state = match state.as_str() {
            "synced" => ContactState::Synced,
            "changed" => ContactState::Changed,
            "deleted" => ContactState::Deleted,
            _ => return Err(StoreError::Corrupt("contact state")),
        };
        Ok(ContactSync {
            contact: Contact {
                id: id.to_owned(),
                account_id,
                card: serde_json::from_str(&card)?,
                state,
            },
            href,
            etag,
            vcard,
        })
    }

    /// Add a contact read from a file, keeping its whole card (`vcard`) so
    /// that its addresses, birthday and photo are kept when it is synced
    /// or exported. Like any new contact, one of an account is sent to its
    /// server on the next sync.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown account.
    pub fn import_contact(
        &self,
        account_id: Option<&str>,
        card: ContactCard,
        vcard: &str,
        now: u64,
    ) -> Result<Contact> {
        let saved = self.save_contact(None, account_id, card, now)?;
        self.conn().execute(
            "UPDATE contacts SET vcard = ?2 WHERE id = ?1",
            params![saved.id, vcard],
        )?;
        Ok(saved)
    }

    /// The `UID`s of the cards in an address book, and their (name, first
    /// address) pairs in lower case, for skipping contacts already there.
    ///
    /// # Errors
    /// Database errors.
    pub fn contact_keys(&self, book: Book<'_>) -> Result<ContactKeys> {
        let mut uids = HashSet::new();
        let mut people = HashSet::new();
        for contact in self.contacts_in_book(book)? {
            let card = contact.contact.card;
            if !card.uid.trim().is_empty() {
                uids.insert(card.uid.trim().to_owned());
            }
            people.insert(person_key(&card));
        }
        Ok((uids, people))
    }

    /// A new id for a contact or event brought in without one.
    ///
    /// # Errors
    /// When the system has no randomness to give.
    pub fn new_import_uid(&self) -> Result<String> {
        Ok(format!("{}@mocky.mail", new_id()?))
    }
}

/// A contact's (name, first address) in lower case: two cards with the
/// same are taken as the same person.
#[must_use]
pub fn person_key(card: &ContactCard) -> (String, String) {
    (
        card.name.trim().to_lowercase(),
        card.emails
            .first()
            .map(|e| e.trim().to_lowercase())
            .unwrap_or_default(),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::test_support::{add_account, add_folder, add_message, summary};
    use mail_core::folders::FolderRole;

    #[test]
    fn export_rows_bodies_and_message_ids() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "me@example.com");
        let folder = add_folder(&store, &account, "INBOX", FolderRole::Inbox);
        let mut first = summary("a", 1);
        first.message_id = Some("a@x.example".into());
        let a = add_message(&store, &folder, 7, &first);
        let mut second = summary("b", 2);
        second.message_id = Some("<B@X.example>".into());
        add_message(&store, &folder, 3, &second);
        store.set_raw(&a, b"Subject: a\r\n\r\nx", None).unwrap();
        let rows = store.export_rows(&folder).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.uid).collect::<Vec<_>>(),
            [Some(3), Some(7)]
        );
        assert!(rows[1].has_raw && !rows[0].has_raw);
        assert_eq!(
            store.raw_by_uid(&folder, 7).unwrap().unwrap(),
            b"Subject: a\r\n\r\nx"
        );
        assert_eq!(store.raw_by_uid(&folder, 3).unwrap(), None);
        assert_eq!(store.raw_by_uid(&folder, 99).unwrap(), None);
        let ids = store.message_ids_in_folder(&folder).unwrap();
        assert!(ids.contains("a@x.example") && ids.contains("b@x.example"));
        assert_eq!(message_id_key(" <A@B> "), "a@b");
    }

    #[test]
    fn imported_contacts_keep_their_card() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "me@example.com");
        let card = ContactCard {
            name: "Amina Hassan".into(),
            emails: vec!["Amina@studio.example".into()],
            ..ContactCard::default()
        };
        let vcard =
            "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Amina Hassan\r\nBDAY:1990-04-01\r\nEND:VCARD\r\n";
        let saved = store.import_contact(None, card.clone(), vcard, 1).unwrap();
        store
            .import_contact(Some(&account), card, vcard, 1)
            .unwrap();
        let device = store.contacts_in_book(Book::Device).unwrap();
        assert_eq!(device.len(), 1);
        assert_eq!(device[0].contact.id, saved.id);
        assert_eq!(device[0].vcard.as_deref(), Some(vcard));
        let theirs = store.contacts_in_book(Book::Account(&account)).unwrap();
        assert_eq!(theirs[0].contact.state, ContactState::Changed);
        assert_eq!(store.contacts_in_book(Book::All).unwrap().len(), 2);
        let (uids, people) = store.contact_keys(Book::Device).unwrap();
        assert_eq!(uids.len(), 1);
        assert!(people.contains(&("amina hassan".to_owned(), "amina@studio.example".to_owned())));
    }
}
