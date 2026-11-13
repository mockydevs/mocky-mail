//! Server folders, their sync markers (UIDVALIDITY, highest UID) and counts.

use mail_core::folders::FolderRole;
use rusqlite::{OptionalExtension, Row, params};
use serde::Serialize;

use crate::Store;
use crate::error::{Result, StoreError};
use crate::ids::{new_id, to_u32};

/// The stored name of a role.
pub(crate) const fn role_str(role: FolderRole) -> &'static str {
    match role {
        FolderRole::Inbox => "inbox",
        FolderRole::Sent => "sent",
        FolderRole::Drafts => "drafts",
        FolderRole::Archive => "archive",
        FolderRole::Junk => "junk",
        FolderRole::Trash => "trash",
        FolderRole::All => "all",
        FolderRole::Flagged => "flagged",
        FolderRole::Other => "other",
    }
}

fn parse_role(value: &str) -> Result<FolderRole> {
    Ok(match value {
        "inbox" => FolderRole::Inbox,
        "sent" => FolderRole::Sent,
        "drafts" => FolderRole::Drafts,
        "archive" => FolderRole::Archive,
        "junk" => FolderRole::Junk,
        "trash" => FolderRole::Trash,
        "all" => FolderRole::All,
        "flagged" => FolderRole::Flagged,
        "other" => FolderRole::Other,
        _ => return Err(StoreError::Corrupt("folder role")),
    })
}

/// A folder as reported by the server's LIST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewFolder<'a> {
    /// Server name (raw, as used in IMAP commands).
    pub path: &'a str,
    /// Name to show.
    pub name: &'a str,
    /// What the folder is for.
    pub role: FolderRole,
    /// Hierarchy separator, if the server has one.
    pub delimiter: Option<&'a str>,
}

/// A stored folder with message counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    /// Opaque ID.
    pub id: String,
    /// Owning account.
    pub account_id: String,
    /// Server name.
    pub path: String,
    /// Name to show.
    pub name: String,
    /// What the folder is for.
    pub role: FolderRole,
    /// Hierarchy separator.
    #[serde(skip)]
    pub delimiter: Option<String>,
    /// Last UIDVALIDITY seen, if synced.
    #[serde(skip)]
    pub uidvalidity: Option<u32>,
    /// Highest UID synced so far.
    #[serde(skip)]
    pub highest_uid: Option<u32>,
    /// Messages not yet seen.
    pub unread: u32,
    /// All messages stored locally.
    pub total: u32,
}

struct FolderRow {
    folder: Folder,
    role: String,
    uidvalidity: Option<i64>,
    highest_uid: Option<i64>,
    unread: i64,
    total: i64,
}

impl FolderRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            folder: Folder {
                id: row.get(0)?,
                account_id: row.get(1)?,
                path: row.get(2)?,
                name: row.get(3)?,
                role: FolderRole::Other,
                delimiter: row.get(5)?,
                uidvalidity: None,
                highest_uid: None,
                unread: 0,
                total: 0,
            },
            role: row.get(4)?,
            uidvalidity: row.get(6)?,
            highest_uid: row.get(7)?,
            unread: row.get(8)?,
            total: row.get(9)?,
        })
    }

    fn into_folder(self) -> Result<Folder> {
        let mut folder = self.folder;
        folder.role = parse_role(&self.role)?;
        folder.uidvalidity = self
            .uidvalidity
            .map(|v| to_u32(v, "uidvalidity"))
            .transpose()?;
        folder.highest_uid = self
            .highest_uid
            .map(|v| to_u32(v, "highest uid"))
            .transpose()?;
        folder.unread = to_u32(self.unread, "unread count")?;
        folder.total = to_u32(self.total, "total count")?;
        Ok(folder)
    }
}

macro_rules! folder_select {
    () => {
        "SELECT f.id, f.account_id, f.path, f.name, f.role, f.delimiter, f.uidvalidity,
                f.highest_uid,
                (SELECT count(*) FROM messages m WHERE m.folder_id = f.id AND m.seen = 0),
                (SELECT count(*) FROM messages m WHERE m.folder_id = f.id)
         FROM folders f "
    };
}

impl Store {
    /// Save a folder by account and server path, updating its name, role and
    /// delimiter if it exists. Returns the folder ID, which never changes.
    ///
    /// # Errors
    /// Fails on a database error, for example an unknown account.
    pub fn upsert_folder(&self, account_id: &str, folder: &NewFolder<'_>) -> Result<String> {
        let id = new_id()?;
        Ok(self.conn().query_row(
            "INSERT INTO folders (id, account_id, path, name, role, delimiter)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (account_id, path) DO UPDATE SET
                 name = excluded.name, role = excluded.role, delimiter = excluded.delimiter
             RETURNING id",
            params![
                id,
                account_id,
                folder.path,
                folder.name,
                role_str(folder.role),
                folder.delimiter
            ],
            |row| row.get(0),
        )?)
    }

    /// One folder by ID.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such folder.
    pub fn get_folder(&self, id: &str) -> Result<Folder> {
        self.conn()
            .query_row(
                concat!(folder_select!(), "WHERE f.id = ?1"),
                params![id],
                FolderRow::read,
            )
            .optional()?
            .ok_or(StoreError::NotFound("folder"))?
            .into_folder()
    }

    /// An account's folders with unread and total counts, sorted for the
    /// sidebar: by role, then by name.
    ///
    /// # Errors
    /// Fails on a database error or an unreadable row.
    pub fn list_folders(&self, account_id: &str) -> Result<Vec<Folder>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(concat!(folder_select!(), "WHERE f.account_id = ?1"))?;
        let mut folders = stmt
            .query_map(params![account_id], FolderRow::read)?
            .map(|row| row?.into_folder())
            .collect::<Result<Vec<_>>>()?;
        folders.sort_by(|a, b| {
            a.role
                .sort_rank()
                .cmp(&b.role.sort_rank())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(folders)
    }

    /// Delete an account's folders whose path is not in `keep` (they were
    /// removed on the server), with their messages. Returns how many went.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn delete_folders_not_in<S: AsRef<str>>(
        &self,
        account_id: &str,
        keep: &[S],
    ) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut removed = 0usize;
        {
            let mut select = tx.prepare("SELECT id, path FROM folders WHERE account_id = ?1")?;
            let existing = select
                .query_map(params![account_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut delete = tx.prepare("DELETE FROM folders WHERE id = ?1")?;
            for (id, path) in existing {
                if !keep.iter().any(|k| k.as_ref() == path) {
                    removed = removed.saturating_add(delete.execute(params![id])?);
                }
            }
        }
        tx.commit()?;
        Ok(removed)
    }

    /// Record the folder's UIDVALIDITY. If it differs from the stored value,
    /// the server has renumbered the folder: every local message in it is
    /// deleted and the highest UID is cleared, so the next sync refetches.
    /// Returns `true` when that reset happened.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such folder.
    pub fn set_uidvalidity(&self, folder_id: &str, value: u32) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let current: Option<i64> = tx
            .query_row(
                "SELECT uidvalidity FROM folders WHERE id = ?1",
                params![folder_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound("folder"))?;
        let value = i64::from(value);
        let reset = current.is_some_and(|old| old != value);
        if reset {
            tx.execute(
                "DELETE FROM messages WHERE folder_id = ?1",
                params![folder_id],
            )?;
            tx.execute(
                "UPDATE folders SET highest_uid = NULL WHERE id = ?1",
                params![folder_id],
            )?;
        }
        if current != Some(value) {
            tx.execute(
                "UPDATE folders SET uidvalidity = ?2 WHERE id = ?1",
                params![folder_id, value],
            )?;
        }
        tx.commit()?;
        Ok(reset)
    }

    /// Record the highest UID synced in a folder.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such folder.
    pub fn set_highest_uid(&self, folder_id: &str, uid: u32) -> Result<()> {
        let changed = self.conn().execute(
            "UPDATE folders SET highest_uid = ?2 WHERE id = ?1",
            params![folder_id, uid],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound("folder"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MessageFlags;
    use crate::test_support::{add_account, add_folder, add_message, summary};

    #[test]
    fn role_names_round_trip() {
        for role in [
            FolderRole::Inbox,
            FolderRole::Sent,
            FolderRole::Drafts,
            FolderRole::Archive,
            FolderRole::Junk,
            FolderRole::Trash,
            FolderRole::All,
            FolderRole::Flagged,
            FolderRole::Other,
        ] {
            assert_eq!(parse_role(role_str(role)).unwrap(), role);
            // Matches mail-core's serde names, which the interface uses.
            assert_eq!(
                serde_json::to_value(role).unwrap(),
                serde_json::Value::from(role_str(role))
            );
        }
        assert!(parse_role("bogus").is_err());
    }

    #[test]
    fn upsert_keeps_the_id_and_updates_details() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let first = store
            .upsert_folder(
                &account,
                &NewFolder {
                    path: "INBOX.Sent",
                    name: "Sent",
                    role: FolderRole::Other,
                    delimiter: Some("."),
                },
            )
            .unwrap();
        let second = store
            .upsert_folder(
                &account,
                &NewFolder {
                    path: "INBOX.Sent",
                    name: "Sent Mail",
                    role: FolderRole::Sent,
                    delimiter: Some("."),
                },
            )
            .unwrap();
        assert_eq!(first, second);
        let folder = store.get_folder(&first).unwrap();
        assert_eq!(folder.name, "Sent Mail");
        assert_eq!(folder.role, FolderRole::Sent);
        assert_eq!(folder.delimiter.as_deref(), Some("."));
        assert_eq!(store.list_folders(&account).unwrap().len(), 1);
    }

    #[test]
    fn same_path_in_two_accounts_is_two_folders() {
        let store = Store::open_in_memory().unwrap();
        let a = add_account(&store, "a@example.com");
        let b = add_account(&store, "b@example.com");
        assert_ne!(
            add_folder(&store, &a, "INBOX", FolderRole::Inbox),
            add_folder(&store, &b, "INBOX", FolderRole::Inbox)
        );
    }

    #[test]
    fn upsert_for_unknown_account_fails() {
        let store = Store::open_in_memory().unwrap();
        let result = store.upsert_folder(
            "nope",
            &NewFolder {
                path: "INBOX",
                name: "Inbox",
                role: FolderRole::Inbox,
                delimiter: None,
            },
        );
        assert!(matches!(result, Err(StoreError::Sqlite(_))));
    }

    #[test]
    fn get_unknown_folder_is_not_found() {
        let store = Store::open_in_memory().unwrap();
        assert!(matches!(
            store.get_folder("nope"),
            Err(StoreError::NotFound("folder"))
        ));
    }

    #[test]
    fn lists_sorted_by_role_then_name_with_counts() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let other = add_account(&store, "b@example.com");
        add_folder(&store, &other, "Elsewhere", FolderRole::Other);
        add_folder(&store, &account, "zeta", FolderRole::Other);
        add_folder(&store, &account, "Trash", FolderRole::Trash);
        add_folder(&store, &account, "Alpha", FolderRole::Other);
        add_folder(&store, &account, "Sent", FolderRole::Sent);
        let inbox = add_folder(&store, &account, "INBOX", FolderRole::Inbox);
        add_folder(&store, &account, "Drafts", FolderRole::Drafts);
        add_message(&store, &inbox, 1, &summary("one", 1));
        add_message(&store, &inbox, 2, &summary("two", 2));
        store
            .upsert_message(
                &inbox,
                3,
                &summary("three", 3),
                MessageFlags {
                    seen: true,
                    ..MessageFlags::default()
                },
                0,
            )
            .unwrap();

        let folders = store.list_folders(&account).unwrap();
        let names: Vec<&str> = folders.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["INBOX", "Drafts", "Sent", "Trash", "Alpha", "zeta"]);
        let inbox = folders.first().unwrap();
        assert_eq!((inbox.unread, inbox.total), (2, 3));
        assert!(
            folders
                .iter()
                .skip(1)
                .all(|f| f.total == 0 && f.unread == 0)
        );
    }

    #[test]
    fn deletes_folders_missing_from_the_server() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let other = add_account(&store, "b@example.com");
        add_folder(&store, &account, "INBOX", FolderRole::Inbox);
        let old = add_folder(&store, &account, "Old", FolderRole::Other);
        add_folder(&store, &account, "Kept", FolderRole::Other);
        let other_old = add_folder(&store, &other, "Old", FolderRole::Other);
        add_message(&store, &old, 1, &summary("in old", 1));

        let removed = store
            .delete_folders_not_in(&account, &["INBOX", "Kept"])
            .unwrap();
        assert_eq!(removed, 1);
        let paths: Vec<String> = store
            .list_folders(&account)
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .collect();
        assert_eq!(paths, ["INBOX", "Kept"]);
        assert!(store.uids_in_folder(&old).unwrap().is_empty());
        // Other accounts are untouched.
        assert!(store.get_folder(&other_old).is_ok());
        assert_eq!(
            store
                .delete_folders_not_in(&account, &["INBOX", "Kept"])
                .unwrap(),
            0
        );
        let none: [&str; 0] = [];
        assert_eq!(store.delete_folders_not_in(&account, &none).unwrap(), 2);
    }

    #[test]
    fn uidvalidity_change_resets_the_folder() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let folder = add_folder(&store, &account, "INBOX", FolderRole::Inbox);
        let neighbour = add_folder(&store, &account, "Other", FolderRole::Other);

        // First value: nothing to reset.
        assert!(!store.set_uidvalidity(&folder, 100).unwrap());
        add_message(&store, &folder, 1, &summary("a", 1));
        add_message(&store, &folder, 2, &summary("b", 2));
        add_message(&store, &neighbour, 1, &summary("c", 3));
        store.set_highest_uid(&folder, 2).unwrap();

        // Same value: nothing happens.
        assert!(!store.set_uidvalidity(&folder, 100).unwrap());
        assert_eq!(store.uids_in_folder(&folder).unwrap(), [1, 2]);

        // New value: the folder's messages are gone, the neighbour's stay.
        assert!(store.set_uidvalidity(&folder, 200).unwrap());
        assert!(store.uids_in_folder(&folder).unwrap().is_empty());
        assert_eq!(store.uids_in_folder(&neighbour).unwrap(), [1]);
        let stored = store.get_folder(&folder).unwrap();
        assert_eq!(stored.uidvalidity, Some(200));
        assert_eq!(stored.highest_uid, None);

        assert!(matches!(
            store.set_uidvalidity("nope", 1),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn sets_highest_uid() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let folder = add_folder(&store, &account, "INBOX", FolderRole::Inbox);
        store.set_highest_uid(&folder, u32::MAX).unwrap();
        assert_eq!(
            store.get_folder(&folder).unwrap().highest_uid,
            Some(u32::MAX)
        );
        assert!(matches!(
            store.set_highest_uid("nope", 1),
            Err(StoreError::NotFound(_))
        ));
    }
}
