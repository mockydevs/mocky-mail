//! Contact groups (lists): a name and the contacts in it. Typing a
//! group's name in a recipient field adds everyone in it.
//!
//! A group of an account's address book syncs like its contacts (marked
//! `changed` or `deleted` until sent); a group with no account stays on
//! this device and may hold anyone.

use rusqlite::{OptionalExtension, Row, params};
use serde::Serialize;

use crate::Store;
use crate::contacts::{Contact, ContactState};
use crate::error::{Result, StoreError};
use crate::ids::new_id;

/// Most groups kept.
pub const MAX_GROUPS: usize = 500;
/// Most people in one group.
pub const MAX_GROUP_MEMBERS: usize = 1000;

/// A contact group as the app shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactGroup {
    /// Local id.
    pub id: String,
    /// The account whose address book holds it; `None`: this device only.
    pub account_id: Option<String>,
    /// Its name.
    pub name: String,
    /// The contacts in it (local ids).
    pub member_ids: Vec<String>,
}

/// A group's server side, for syncing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupSync {
    /// The group.
    pub group: ContactGroup,
    /// Stable id inside the group card.
    pub uid: String,
    /// Its place on the server, once it is there.
    pub href: Option<String>,
    /// The server's version tag.
    pub etag: Option<String>,
    /// The server's card text, if the server keeps groups as cards.
    pub vcard: Option<String>,
    /// Whether a change is still to be sent.
    pub state: ContactState,
}

const COLUMNS: &str = "id, account_id, name, members, uid, href, etag, vcard, state";

fn state_of(text: &str) -> Result<ContactState> {
    match text {
        "synced" => Ok(ContactState::Synced),
        "changed" => Ok(ContactState::Changed),
        "deleted" => Ok(ContactState::Deleted),
        _ => Err(StoreError::Corrupt("group state")),
    }
}

const fn state_name(state: ContactState) -> &'static str {
    match state {
        ContactState::Synced => "synced",
        ContactState::Changed => "changed",
        ContactState::Deleted => "deleted",
    }
}

/// id, account, name, members JSON, uid, href, etag, vcard, state.
type RawRow = (
    String,
    Option<String>,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

fn raw_row(row: &Row<'_>) -> rusqlite::Result<RawRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}

fn to_sync(raw: RawRow) -> Result<GroupSync> {
    let (id, account_id, name, members, uid, href, etag, vcard, state) = raw;
    Ok(GroupSync {
        group: ContactGroup {
            id,
            account_id,
            name,
            member_ids: serde_json::from_str(&members)?,
        },
        uid,
        href,
        etag,
        vcard,
        state: state_of(&state)?,
    })
}

impl Store {
    /// Every group (deleted ones left out), by name. Members that no
    /// longer exist are left out.
    ///
    /// # Errors
    /// Database errors.
    pub fn list_contact_groups(&self) -> Result<Vec<ContactGroup>> {
        let rows: Vec<RawRow> = {
            let conn = self.conn();
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM contact_groups WHERE state != 'deleted'
                 ORDER BY name COLLATE NOCASE, id"
            ))?;
            let rows = stmt.query_map([], raw_row)?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut out = Vec::new();
        for raw in rows {
            let mut group = to_sync(raw)?.group;
            group.member_ids = self.existing_contacts(&group.member_ids)?;
            out.push(group);
        }
        Ok(out)
    }

    /// The ids among these that are saved, not deleted, contacts.
    fn existing_contacts(&self, ids: &[String]) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare_cached("SELECT 1 FROM contacts WHERE id = ?1 AND state != 'deleted'")?;
        let mut out = Vec::new();
        for id in ids {
            if stmt
                .query_row(params![id], |_| Ok(()))
                .optional()?
                .is_some()
                && !out.contains(id)
            {
                out.push(id.clone());
            }
        }
        Ok(out)
    }

    fn group_sync(&self, id: &str) -> Result<GroupSync> {
        let raw = self
            .conn()
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM contact_groups WHERE id = ?1 AND state != 'deleted'"
                ),
                params![id],
                raw_row,
            )
            .optional()?
            .ok_or(StoreError::NotFound("group"))?;
        to_sync(raw)
    }

    /// One group.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] when there is none.
    pub fn get_contact_group(&self, id: &str) -> Result<ContactGroup> {
        let mut group = self.group_sync(id)?.group;
        group.member_ids = self.existing_contacts(&group.member_ids)?;
        Ok(group)
    }

    /// Create (`id` `None`) or change a group. A group of an account is
    /// marked to be sent to its server, and may only hold that account's
    /// contacts; a device group may hold anyone.
    ///
    /// # Errors
    /// [`StoreError::Invalid`] for a member from another address book, an
    /// empty name or too many members; [`StoreError::NotFound`] for an
    /// unknown group, account or contact.
    pub fn save_contact_group(
        &self,
        id: Option<&str>,
        account_id: Option<&str>,
        name: &str,
        member_ids: &[String],
        now: u64,
    ) -> Result<ContactGroup> {
        let now = crate::ids::to_i64(now, "time")?;
        let name: String = name.chars().filter(|c| !c.is_control()).take(200).collect();
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Invalid("a group needs a name"));
        }
        let mut members: Vec<String> = Vec::new();
        for member in member_ids {
            if members.contains(member) {
                continue;
            }
            let contact: Contact = self.get_contact(member)?;
            if account_id.is_some() && contact.account_id.as_deref() != account_id {
                return Err(StoreError::Invalid(
                    "a group of an account's address book can only hold its contacts",
                ));
            }
            members.push(member.clone());
        }
        if members.len() > MAX_GROUP_MEMBERS {
            return Err(StoreError::Invalid("too many people in one group"));
        }
        let state = if account_id.is_some() {
            ContactState::Changed
        } else {
            ContactState::Synced
        };
        let members_json = serde_json::to_string(&members)?;
        let id = if let Some(id) = id {
            let old = self.group_sync(id)?;
            if old.group.account_id.as_deref() != account_id {
                return Err(StoreError::Invalid(
                    "a group can't move between address books",
                ));
            }
            self.conn().execute(
                "UPDATE contact_groups SET name = ?2, members = ?3, state = ?4, updated_at = ?5
                 WHERE id = ?1",
                params![id, name, members_json, state_name(state), now],
            )?;
            id.to_owned()
        } else {
            if let Some(account) = account_id {
                self.get_account(account)?;
            }
            let count: i64 = self.conn().query_row(
                "SELECT count(*) FROM contact_groups WHERE state != 'deleted'",
                [],
                |row| row.get(0),
            )?;
            if usize::try_from(count).unwrap_or(usize::MAX) >= MAX_GROUPS {
                return Err(StoreError::Invalid("too many groups"));
            }
            let id = new_id()?;
            self.conn().execute(
                "INSERT INTO contact_groups (id, account_id, uid, name, members, state, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id,
                    account_id,
                    format!("{id}-mocky-group"),
                    name,
                    members_json,
                    state_name(state),
                    now
                ],
            )?;
            id
        };
        self.get_contact_group(&id)
    }

    /// Delete a group (not its people). One on a server is kept, marked
    /// `deleted`, until the sync deletes the server copy.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown group.
    pub fn delete_contact_group(&self, id: &str) -> Result<()> {
        let sync = self.group_sync(id)?;
        let sending = sync.group.account_id.is_some() && sync.state == ContactState::Changed;
        if sync.href.is_some() || sending {
            self.conn().execute(
                "UPDATE contact_groups SET state = 'deleted' WHERE id = ?1",
                params![id],
            )?;
        } else {
            self.conn()
                .execute("DELETE FROM contact_groups WHERE id = ?1", params![id])?;
        }
        Ok(())
    }

    /// An account's groups with changes to send.
    ///
    /// # Errors
    /// Database errors.
    pub fn groups_to_push(&self, account_id: &str) -> Result<Vec<GroupSync>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM contact_groups WHERE account_id = ?1 AND state != 'synced'"
        ))?;
        let rows = stmt.query_map(params![account_id], raw_row)?;
        rows.map(|row| to_sync(row?)).collect()
    }

    /// The server copies of an account's groups: href to version tag.
    ///
    /// # Errors
    /// Database errors.
    pub fn group_etags(
        &self,
        account_id: &str,
    ) -> Result<std::collections::HashMap<String, Option<String>>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT href, etag FROM contact_groups WHERE account_id = ?1 AND href IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![account_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A change reached the server. The group is marked `synced` only if
    /// it is still what was sent (`name`, `member_ids`).
    ///
    /// # Errors
    /// Database errors.
    pub fn group_pushed(
        &self,
        id: &str,
        href: &str,
        etag: Option<&str>,
        vcard: Option<&str>,
        sent: &ContactGroup,
    ) -> Result<()> {
        let conn = self.conn();
        let row: Option<(String, String, String)> = conn
            .query_row(
                "SELECT name, members, state FROM contact_groups WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((name, members, state)) = row else {
            return Ok(());
        };
        let unchanged = state == "changed"
            && name == sent.name
            && serde_json::from_str::<Vec<String>>(&members).is_ok_and(|m| m == sent.member_ids);
        let state = if unchanged { "synced" } else { state.as_str() };
        conn.execute(
            "UPDATE contact_groups SET href = ?2, etag = ?3, vcard = ?4, state = ?5 WHERE id = ?1",
            params![id, href, etag, vcard, state],
        )?;
        Ok(())
    }

    /// Store a group read from the server. One with a change still to send
    /// is left alone.
    ///
    /// # Errors
    /// Database errors.
    #[allow(clippy::too_many_arguments, reason = "the server copy's parts")]
    pub fn apply_server_group(
        &self,
        account_id: &str,
        href: &str,
        etag: Option<&str>,
        vcard: Option<&str>,
        uid: &str,
        name: &str,
        member_ids: &[String],
        now: u64,
    ) -> Result<()> {
        let now = crate::ids::to_i64(now, "time")?;
        let members = serde_json::to_string(member_ids)?;
        let existing: Option<(String, String)> = self
            .conn()
            .query_row(
                "SELECT id, state FROM contact_groups WHERE account_id = ?1 AND href = ?2",
                params![account_id, href],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match existing {
            Some((_, state)) if state != "synced" => {}
            Some((id, _)) => {
                self.conn().execute(
                    "UPDATE contact_groups SET name = ?2, members = ?3, etag = ?4, vcard = ?5,
                        uid = ?6, updated_at = ?7 WHERE id = ?1",
                    params![id, name, members, etag, vcard, uid, now],
                )?;
            }
            None => {
                self.conn().execute(
                    "INSERT INTO contact_groups (id, account_id, uid, name, members, href, etag, vcard, state, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'synced', ?9)",
                    params![new_id()?, account_id, uid, name, members, href, etag, vcard, now],
                )?;
            }
        }
        Ok(())
    }

    /// Remove a group for good (its server copy is gone).
    ///
    /// # Errors
    /// Database errors.
    pub fn forget_contact_group(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM contact_groups WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Drop synced groups whose server copy is gone.
    ///
    /// # Errors
    /// Database errors.
    pub fn forget_groups_not_on_server(
        &self,
        account_id: &str,
        on_server: &[String],
    ) -> Result<usize> {
        let known: Vec<(String, String)> = {
            let conn = self.conn();
            let mut stmt = conn.prepare(
                "SELECT href, state FROM contact_groups WHERE account_id = ?1 AND href IS NOT NULL",
            )?;
            let rows = stmt.query_map(params![account_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut removed = 0usize;
        for (href, state) in known {
            if state == "synced" && !on_server.contains(&href) {
                removed = removed.saturating_add(self.conn().execute(
                    "DELETE FROM contact_groups WHERE account_id = ?1 AND href = ?2",
                    params![account_id, href],
                )?);
            }
        }
        Ok(removed)
    }

    /// The people in the group with this name (ignoring case), for a
    /// recipient field. `None` when no group has that name.
    ///
    /// # Errors
    /// Database errors.
    pub fn group_members_named(&self, name: &str) -> Result<Option<Vec<Contact>>> {
        let ids: Option<String> = self
            .conn()
            .query_row(
                "SELECT members FROM contact_groups WHERE state != 'deleted'
                 AND name = ?1 COLLATE NOCASE ORDER BY id LIMIT 1",
                params![name.trim()],
                |row| row.get(0),
            )
            .optional()?;
        let Some(ids) = ids else {
            return Ok(None);
        };
        let ids: Vec<String> = serde_json::from_str(&ids)?;
        Ok(Some(self.contacts_by_id(&ids)?))
    }

    /// Groups whose name contains `query` (ignoring case), for recipient
    /// suggestions.
    ///
    /// # Errors
    /// Database errors.
    pub fn groups_matching(&self, query: &str, limit: usize) -> Result<Vec<ContactGroup>> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .list_contact_groups()?
            .into_iter()
            .filter(|g| g.name.to_lowercase().contains(&query))
            .take(limit)
            .collect())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::test_support;
    use mail_core::vcard::ContactCard;

    fn person(store: &Store, account: Option<&str>, name: &str, email: &str) -> String {
        store
            .save_contact(
                None,
                account,
                ContactCard {
                    name: name.into(),
                    emails: vec![email.into()],
                    ..ContactCard::default()
                },
                1,
            )
            .unwrap()
            .id
    }

    #[test]
    fn device_groups_hold_anyone_and_expand_by_name() {
        let store = Store::open_in_memory().unwrap();
        let account = test_support::add_account(&store, "me@work.example");
        let peter = person(&store, None, "Peter", "peter@client.example");
        let amina = person(&store, Some(&account), "Amina", "amina@studio.example");
        let group = store
            .save_contact_group(
                None,
                None,
                " Site team ",
                &[peter.clone(), amina.clone(), peter.clone()],
                5,
            )
            .unwrap();
        assert_eq!(group.name, "Site team");
        assert_eq!(group.member_ids, [peter.clone(), amina.clone()]);
        let members = store.group_members_named("SITE TEAM").unwrap().unwrap();
        assert_eq!(
            members
                .iter()
                .map(|c| c.card.emails[0].as_str())
                .collect::<Vec<_>>(),
            ["peter@client.example", "amina@studio.example"]
        );
        assert!(store.group_members_named("nobody").unwrap().is_none());
        assert_eq!(store.groups_matching("site", 10).unwrap().len(), 1);
        // A deleted contact leaves the group.
        store.delete_contact(&peter).unwrap();
        assert_eq!(
            store.get_contact_group(&group.id).unwrap().member_ids,
            [amina]
        );
        // Device groups are deleted at once.
        store.delete_contact_group(&group.id).unwrap();
        assert!(store.list_contact_groups().unwrap().is_empty());
        assert!(store.save_contact_group(None, None, " ", &[], 5).is_err());
    }

    #[test]
    fn account_groups_sync_like_contacts() {
        let store = Store::open_in_memory().unwrap();
        let account = test_support::add_account(&store, "me@work.example");
        let amina = person(&store, Some(&account), "Amina", "amina@studio.example");
        let device = person(&store, None, "Peter", "peter@client.example");
        // Only the account's own contacts.
        assert!(
            store
                .save_contact_group(None, Some(&account), "Team", &[device], 5)
                .is_err()
        );
        let group = store
            .save_contact_group(
                None,
                Some(&account),
                "Team",
                std::slice::from_ref(&amina),
                5,
            )
            .unwrap();
        let pending = store.groups_to_push(&account).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].state, ContactState::Changed);
        // Renamed while being sent: stays changed.
        store
            .save_contact_group(
                Some(&group.id),
                Some(&account),
                "Team B",
                std::slice::from_ref(&amina),
                6,
            )
            .unwrap();
        store
            .group_pushed(
                &group.id,
                "/book/g.vcf",
                Some("\"1\""),
                Some("card"),
                &pending[0].group,
            )
            .unwrap();
        assert_eq!(store.groups_to_push(&account).unwrap().len(), 1);
        let pending = store.groups_to_push(&account).unwrap();
        store
            .group_pushed(
                &group.id,
                "/book/g.vcf",
                Some("\"2\""),
                Some("card"),
                &pending[0].group,
            )
            .unwrap();
        assert!(store.groups_to_push(&account).unwrap().is_empty());
        assert_eq!(store.group_etags(&account).unwrap().len(), 1);
        // From the server.
        store
            .apply_server_group(
                &account,
                "/book/h.vcf",
                Some("\"1\""),
                None,
                "h",
                "Board",
                &[],
                7,
            )
            .unwrap();
        assert_eq!(store.list_contact_groups().unwrap().len(), 2);
        store
            .forget_groups_not_on_server(&account, &["/book/g.vcf".to_owned()])
            .unwrap();
        assert_eq!(store.list_contact_groups().unwrap().len(), 1);
        // Deleted: kept until the server copy goes.
        store.delete_contact_group(&group.id).unwrap();
        assert!(store.list_contact_groups().unwrap().is_empty());
        assert_eq!(
            store.groups_to_push(&account).unwrap()[0].state,
            ContactState::Deleted
        );
        store.forget_contact_group(&group.id).unwrap();
        assert!(store.groups_to_push(&account).unwrap().is_empty());
    }
}
