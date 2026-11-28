//! What Exchange Server accounts keep beyond what IMAP ones do: each
//! folder's Exchange id and where its sync left off, and each message's
//! Exchange id beside the UID it is given here.

use std::collections::HashMap;

use mail_core::folders::FolderRole;
use rusqlite::{OptionalExtension, params};

use crate::Store;
use crate::error::{Result, StoreError};
use crate::folders::role_str;
use crate::ids::{new_id, to_u32};

/// A message of an Exchange folder as kept here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EwsItem {
    /// The message's id here.
    pub id: String,
    /// Its UID here.
    pub uid: u32,
}

impl Store {
    /// Save an Exchange folder by its Exchange id: renamed or moved on the
    /// server, it keeps its id (and mail) here. A folder made here first
    /// (Archive, when the server had none) is adopted by its path. Returns
    /// the folder's id here.
    ///
    /// # Errors
    /// Database errors.
    pub fn upsert_ews_folder(
        &self,
        account_id: &str,
        remote_id: &str,
        path: &str,
        name: &str,
        role: FolderRole,
    ) -> Result<String> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let known: Option<String> = tx
            .query_row(
                "SELECT id FROM folders WHERE account_id = ?1 AND remote_id = ?2",
                params![account_id, remote_id],
                |row| row.get(0),
            )
            .optional()?;
        let id = if let Some(id) = known {
            // Another folder may hold the path now (two swapped names): it
            // gives way for the moment, and gets its own path in turn.
            tx.execute(
                "UPDATE folders SET path = path || char(1) || id
                 WHERE account_id = ?1 AND path = ?2 AND id <> ?3",
                params![account_id, path, id],
            )?;
            tx.execute(
                "UPDATE folders SET path = ?2, name = ?3, role = ?4, delimiter = '/' WHERE id = ?1",
                params![id, path, name, role_str(role)],
            )?;
            id
        } else {
            let fresh = new_id()?;
            tx.query_row(
                "INSERT INTO folders (id, account_id, path, name, role, delimiter, remote_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, '/', ?6)
                 ON CONFLICT (account_id, path) DO UPDATE SET
                     name = excluded.name, role = excluded.role, remote_id = excluded.remote_id,
                     delimiter = '/'
                 RETURNING id",
                params![fresh, account_id, path, name, role_str(role), remote_id],
                |row| row.get(0),
            )?
        };
        tx.commit()?;
        Ok(id)
    }

    /// An Exchange folder's id there.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such folder.
    pub fn folder_remote_id(&self, folder_id: &str) -> Result<Option<String>> {
        self.conn()
            .query_row(
                "SELECT remote_id FROM folders WHERE id = ?1",
                params![folder_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound("folder"))
    }

    /// The folder (id here) with this path in an account, and its Exchange
    /// id if it has one yet.
    ///
    /// # Errors
    /// Database errors.
    pub fn folder_by_path(
        &self,
        account_id: &str,
        path: &str,
    ) -> Result<Option<(String, Option<String>)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, remote_id FROM folders WHERE account_id = ?1 AND path = ?2",
                params![account_id, path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    /// Record a folder's Exchange id (one made here, then on the server).
    ///
    /// # Errors
    /// Database errors.
    pub fn set_folder_remote_id(&self, folder_id: &str, remote_id: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE folders SET remote_id = ?2 WHERE id = ?1",
            params![folder_id, remote_id],
        )?;
        Ok(())
    }

    /// Delete an account's Exchange folders whose Exchange id is not in
    /// `keep`, with their mail. Folders made here and not yet on the server
    /// (no Exchange id) stay. Returns how many went.
    ///
    /// # Errors
    /// Database errors.
    pub fn delete_ews_folders_not_in<S: AsRef<str>>(
        &self,
        account_id: &str,
        keep: &[S],
    ) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut removed = 0usize;
        {
            let mut select = tx.prepare(
                "SELECT id, remote_id FROM folders WHERE account_id = ?1 AND remote_id IS NOT NULL",
            )?;
            let existing = select
                .query_map(params![account_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut delete = tx.prepare("DELETE FROM folders WHERE id = ?1")?;
            for (id, remote) in existing {
                if !keep.iter().any(|k| k.as_ref() == remote) {
                    removed = removed.saturating_add(delete.execute(params![id])?);
                }
            }
        }
        tx.commit()?;
        Ok(removed)
    }

    /// Where a folder's sync left off.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such folder.
    pub fn folder_sync_state(&self, folder_id: &str) -> Result<Option<String>> {
        self.conn()
            .query_row(
                "SELECT sync_state FROM folders WHERE id = ?1",
                params![folder_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound("folder"))
    }

    /// Record where a folder's sync left off (`None` to start over).
    ///
    /// # Errors
    /// Database errors.
    pub fn set_folder_sync_state(&self, folder_id: &str, state: Option<&str>) -> Result<()> {
        self.conn().execute(
            "UPDATE folders SET sync_state = ?2 WHERE id = ?1",
            params![folder_id, state],
        )?;
        Ok(())
    }

    /// A folder's messages that are on the server, by their Exchange id.
    ///
    /// # Errors
    /// Database errors.
    pub fn ews_items(&self, folder_id: &str) -> Result<HashMap<String, EwsItem>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT remote_id, id, uid FROM messages
             WHERE folder_id = ?1 AND remote_id IS NOT NULL AND uid IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![folder_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let (remote, id, uid) = row?;
            out.insert(
                remote,
                EwsItem {
                    id,
                    uid: to_u32(uid, "message uid")?,
                },
            );
        }
        Ok(out)
    }

    /// Record a message's Exchange id.
    ///
    /// # Errors
    /// Database errors.
    pub fn set_message_remote_id(&self, id: &str, remote_id: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE messages SET remote_id = ?2 WHERE id = ?1",
            params![id, remote_id],
        )?;
        Ok(())
    }

    /// The Exchange ids of a folder's messages with these UIDs (those
    /// without one left out).
    ///
    /// # Errors
    /// Database errors.
    pub fn remote_ids_for_uids(&self, folder_id: &str, uids: &[u32]) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT remote_id FROM messages WHERE folder_id = ?1 AND uid = ?2 AND remote_id IS NOT NULL",
        )?;
        let mut out = Vec::new();
        for uid in uids {
            if let Some(remote) = stmt
                .query_row(params![folder_id, uid], |row| row.get::<_, String>(0))
                .optional()?
            {
                out.push(remote);
            }
        }
        Ok(out)
    }

    /// A message's Exchange id, while it is where the server has it (not
    /// moved here since).
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such message.
    pub fn message_remote_id(&self, id: &str) -> Result<Option<String>> {
        let (remote, uid): (Option<String>, Option<i64>) = self
            .conn()
            .query_row(
                "SELECT remote_id, uid FROM messages WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(StoreError::NotFound("message"))?;
        Ok(remote.filter(|_| uid.is_some()))
    }

    /// Forget a folder's messages and where its sync left off, so the next
    /// sync reads it again (its sync state was refused).
    ///
    /// # Errors
    /// Database errors.
    pub fn reset_ews_folder(&self, folder_id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM messages WHERE folder_id = ?1 AND uid IS NOT NULL",
            params![folder_id],
        )?;
        tx.execute(
            "UPDATE folders SET sync_state = NULL, highest_uid = NULL WHERE id = ?1",
            params![folder_id],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{add_account, summary};
    use crate::{EwsSettings, MessageFlags, Protocol, test_support::new_account};

    #[test]
    fn exchange_accounts_are_told_apart_and_keep_their_server() {
        let store = Store::open_in_memory().unwrap();
        let imap = add_account(&store, "a@x.co");
        assert_eq!(store.get_account(&imap).unwrap().protocol, Protocol::Imap);
        assert_eq!(store.ews_settings(&imap).unwrap(), None);
        let mut new = new_account("don@corp.example");
        new.auth = crate::AuthKind::Exchange;
        let ews = EwsSettings {
            url: "https://mail.corp.example/EWS/Exchange.asmx".into(),
            username: Some("CORP\\don".into()),
        };
        let account = store.insert_exchange_account(&new, &ews).unwrap();
        assert_eq!(account.protocol, Protocol::Ews);
        assert_eq!(account.auth, crate::AuthKind::Exchange);
        assert!(account.is_ews());
        assert_eq!(store.ews_settings(&account.id).unwrap(), Some(ews));
        let json = serde_json::to_value(&account).unwrap();
        assert_eq!(json["protocol"], "ews");
        assert_eq!(json["auth"], "exchange");
    }

    #[test]
    fn folders_keep_their_id_when_renamed_on_the_server() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@x.co");
        let first = store
            .upsert_ews_folder(
                &account,
                "R1",
                "Inbox/Clients",
                "Clients",
                FolderRole::Other,
            )
            .unwrap();
        let again = store
            .upsert_ews_folder(
                &account,
                "R1",
                "Inbox/Customers",
                "Customers",
                FolderRole::Other,
            )
            .unwrap();
        assert_eq!(first, again);
        assert_eq!(store.get_folder(&first).unwrap().path, "Inbox/Customers");
        assert_eq!(
            store.folder_remote_id(&first).unwrap().as_deref(),
            Some("R1")
        );
        // A folder made here first is adopted by its path.
        let local = store
            .upsert_folder(
                &account,
                &crate::NewFolder {
                    path: "Archive",
                    name: "Archive",
                    role: FolderRole::Archive,
                    delimiter: Some("/"),
                },
            )
            .unwrap();
        assert_eq!(
            store.folder_by_path(&account, "Archive").unwrap(),
            Some((local.clone(), None))
        );
        let adopted = store
            .upsert_ews_folder(&account, "R2", "Archive", "Archive", FolderRole::Archive)
            .unwrap();
        assert_eq!(adopted, local);
        // Gone on the server: gone here; one not on the server yet stays.
        let pending = store
            .upsert_folder(
                &account,
                &crate::NewFolder {
                    path: "New",
                    name: "New",
                    role: FolderRole::Other,
                    delimiter: Some("/"),
                },
            )
            .unwrap();
        assert_eq!(
            store.delete_ews_folders_not_in(&account, &["R2"]).unwrap(),
            1
        );
        assert!(store.get_folder(&first).is_err());
        assert!(store.get_folder(&pending).is_ok());
    }

    #[test]
    fn items_map_between_exchange_ids_and_uids() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@x.co");
        let folder = store
            .upsert_ews_folder(&account, "R1", "Inbox", "Inbox", FolderRole::Inbox)
            .unwrap();
        let flags = MessageFlags::default();
        let one = store
            .upsert_message(&folder, 1, &summary("one", 1), flags, 10)
            .unwrap();
        let two = store
            .upsert_message(&folder, 2, &summary("two", 2), flags, 10)
            .unwrap();
        store.set_message_remote_id(&one, "I1").unwrap();
        store.set_message_remote_id(&two, "I2").unwrap();
        let items = store.ews_items(&folder).unwrap();
        assert_eq!(
            items.get("I2"),
            Some(&EwsItem {
                id: two.clone(),
                uid: 2
            })
        );
        assert_eq!(
            store.remote_ids_for_uids(&folder, &[2, 1, 9]).unwrap(),
            vec!["I2", "I1"]
        );
        assert_eq!(
            store.message_remote_id(&one).unwrap().as_deref(),
            Some("I1")
        );
        store.set_folder_sync_state(&folder, Some("S1")).unwrap();
        assert_eq!(
            store.folder_sync_state(&folder).unwrap().as_deref(),
            Some("S1")
        );
        // Moved here: no longer where the server has it.
        let other = store
            .upsert_ews_folder(&account, "R2", "Archive", "Archive", FolderRole::Archive)
            .unwrap();
        store.move_local(&[&one], &other).unwrap();
        assert_eq!(store.message_remote_id(&one).unwrap(), None);
        store.reset_ews_folder(&folder).unwrap();
        assert!(store.ews_items(&folder).unwrap().is_empty());
        assert_eq!(store.folder_sync_state(&folder).unwrap(), None);
    }
}
