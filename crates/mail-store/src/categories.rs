//! Colour categories and which messages carry them.
//!
//! The categories themselves are kept here only; a message's categories are
//! also kept on its server (see `mail_core::categories`), which the sync
//! engine does from the message ids these functions return.

use mail_core::categories::{edited, palette_color, renamed, same_name};
use rusqlite::{Transaction, params};
use serde::{Deserialize, Serialize};

use crate::Store;
use crate::error::{Result, StoreError};

/// A colour category (the interface's `Category`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    /// Unique, ignoring case.
    pub name: String,
    /// `#rrggbb`.
    pub color: String,
}

/// Rewrite the categories of every message that has `from`: renamed to
/// `to`, or removed with `None`. Returns those messages' ids.
fn rewrite_messages(tx: &Transaction<'_>, from: &str, to: Option<&str>) -> Result<Vec<String>> {
    let rows: Vec<(String, String)> = tx
        .prepare("SELECT id, categories FROM messages WHERE categories <> '[]'")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut changed = Vec::new();
    let mut update = tx.prepare("UPDATE messages SET categories = ?2 WHERE id = ?1")?;
    for (id, json) in rows {
        let have: Vec<String> = serde_json::from_str(&json)?;
        if let Some(now) = renamed(&have, from, to) {
            update.execute(params![id, serde_json::to_string(&now)?])?;
            changed.push(id);
        }
    }
    Ok(changed)
}

fn find(tx: &Transaction<'_>, name: &str) -> Result<Option<Category>> {
    let all: Vec<Category> = tx
        .prepare("SELECT name, color FROM categories")?
        .query_map([], |row| {
            Ok(Category {
                name: row.get(0)?,
                color: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(all.into_iter().find(|c| same_name(&c.name, name)))
}

impl Store {
    /// Every category, by name.
    ///
    /// # Errors
    /// Database errors.
    pub fn list_categories(&self) -> Result<Vec<Category>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT name, color FROM categories")?;
        let mut all: Vec<Category> = stmt
            .query_map([], |row| {
                Ok(Category {
                    name: row.get(0)?,
                    color: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        all.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(all)
    }

    /// Add a category or change its colour; with `previous`, rename that
    /// one (on every message that has it, too). Names must already be
    /// clean (`mail_core::categories::clean_name`). Returns the ids of the
    /// messages whose categories changed.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] when `previous` doesn't exist;
    /// [`StoreError::Invalid`] when another category already has the name.
    pub fn save_category(
        &self,
        category: &Category,
        previous: Option<&str>,
    ) -> Result<Vec<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let existing = find(&tx, &category.name)?;
        let changed = match previous {
            None => {
                if let Some(existing) = existing {
                    // The same one again: a new colour.
                    tx.execute(
                        "UPDATE categories SET color = ?2 WHERE name = ?1",
                        params![existing.name, category.color],
                    )?;
                } else {
                    tx.execute(
                        "INSERT INTO categories (name, color) VALUES (?1, ?2)",
                        params![category.name, category.color],
                    )?;
                }
                Vec::new()
            }
            Some(previous) => {
                let old = find(&tx, previous)?.ok_or(StoreError::NotFound("category"))?;
                if existing.as_ref().is_some_and(|e| e.name != old.name) {
                    return Err(StoreError::Invalid("category name in use"));
                }
                tx.execute("DELETE FROM categories WHERE name = ?1", params![old.name])?;
                tx.execute(
                    "INSERT INTO categories (name, color) VALUES (?1, ?2)",
                    params![category.name, category.color],
                )?;
                if old.name == category.name {
                    Vec::new()
                } else {
                    rewrite_messages(&tx, &old.name, Some(&category.name))?
                }
            }
        };
        tx.commit()?;
        Ok(changed)
    }

    /// Remove a category, from every message too. Returns the ids of the
    /// messages that had it.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] when there is no such category.
    pub fn delete_category(&self, name: &str) -> Result<Vec<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let old = find(&tx, name)?.ok_or(StoreError::NotFound("category"))?;
        tx.execute("DELETE FROM categories WHERE name = ?1", params![old.name])?;
        let changed = rewrite_messages(&tx, &old.name, None)?;
        tx.commit()?;
        Ok(changed)
    }

    /// Add categories that came from the server (Gmail labels, keywords
    /// another device set) and aren't known here, with palette colours.
    /// Returns whether any were added.
    ///
    /// # Errors
    /// Database errors.
    pub fn ensure_categories(&self, names: &[String]) -> Result<bool> {
        if names.is_empty() {
            return Ok(false);
        }
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut added = false;
        for name in names {
            if find(&tx, name)?.is_some() {
                continue;
            }
            let count: i64 =
                tx.query_row("SELECT count(*) FROM categories", [], |row| row.get(0))?;
            tx.execute(
                "INSERT INTO categories (name, color) VALUES (?1, ?2)",
                params![
                    name,
                    palette_color(usize::try_from(count).unwrap_or_default())
                ],
            )?;
            added = true;
        }
        tx.commit()?;
        Ok(added)
    }

    /// Add and remove categories on messages (names as the user picked
    /// them; case is ignored when matching). Returns the ids that changed.
    ///
    /// # Errors
    /// Database errors.
    pub fn set_categories<S: AsRef<str>>(
        &self,
        ids: &[S],
        add: &[String],
        remove: &[String],
    ) -> Result<Vec<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut changed = Vec::new();
        {
            let mut read = tx.prepare("SELECT categories FROM messages WHERE id = ?1")?;
            let mut write = tx.prepare("UPDATE messages SET categories = ?2 WHERE id = ?1")?;
            for id in ids {
                let id = id.as_ref();
                let json: Option<String> = read
                    .query_map(params![id], |row| row.get(0))?
                    .next()
                    .transpose()?;
                let Some(json) = json else { continue };
                let have: Vec<String> = serde_json::from_str(&json)?;
                let now = edited(&have, add, remove);
                if now != have {
                    write.execute(params![id, serde_json::to_string(&now)?])?;
                    changed.push(id.to_owned());
                }
            }
        }
        tx.commit()?;
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{add_account, add_folder, add_message, summary};
    use mail_core::folders::FolderRole;

    fn cat(name: &str, color: &str) -> Category {
        Category {
            name: name.to_owned(),
            color: color.to_owned(),
        }
    }

    fn categories_of(store: &Store, id: &str) -> Vec<String> {
        store.get_message(id).unwrap().item.categories
    }

    #[test]
    fn categories_are_unique_ignoring_case() {
        let store = Store::open_in_memory().unwrap();
        store.save_category(&cat("Work", "#ff0000"), None).unwrap();
        store.save_category(&cat("home", "#00ff00"), None).unwrap();
        // The same name again changes its colour.
        store.save_category(&cat("WORK", "#0000ff"), None).unwrap();
        assert_eq!(
            store.list_categories().unwrap(),
            [cat("home", "#00ff00"), cat("Work", "#0000ff")]
        );
        // Renaming onto another one is refused.
        assert!(matches!(
            store.save_category(&cat("Home", "#000000"), Some("work")),
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            store.save_category(&cat("x", "#000000"), Some("nope")),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            store.delete_category("nope"),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn renames_and_deletes_reach_every_message() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let inbox = add_folder(&store, &account, "INBOX", FolderRole::Inbox);
        let one = add_message(&store, &inbox, 1, &summary("one", 1));
        let two = add_message(&store, &inbox, 2, &summary("two", 2));
        let three = add_message(&store, &inbox, 3, &summary("three", 3));
        store.save_category(&cat("Work", "#ff0000"), None).unwrap();
        store
            .save_category(&cat("Urgent", "#00ff00"), None)
            .unwrap();
        let names =
            |list: &[&str]| -> Vec<String> { list.iter().map(|s| (*s).to_owned()).collect() };

        let changed = store
            .set_categories(&[&one, &two], &names(&["Work", "Urgent"]), &[])
            .unwrap();
        assert_eq!(changed, [one.clone(), two.clone()]);
        // Nothing to change: not reported.
        assert!(
            store
                .set_categories(&[&one], &names(&["work"]), &[])
                .unwrap()
                .is_empty()
        );
        store
            .set_categories(&[&two], &[], &names(&["WORK"]))
            .unwrap();
        assert_eq!(categories_of(&store, &one), ["Work", "Urgent"]);
        assert_eq!(categories_of(&store, &two), ["Urgent"]);

        let renamed = store
            .save_category(&cat("Clients", "#123456"), Some("Work"))
            .unwrap();
        assert_eq!(renamed, std::slice::from_ref(&one));
        assert_eq!(categories_of(&store, &one), ["Clients", "Urgent"]);

        let mut deleted = store.delete_category("urgent").unwrap();
        deleted.sort();
        let mut expected = vec![one.clone(), two.clone()];
        expected.sort();
        assert_eq!(deleted, expected);
        assert_eq!(categories_of(&store, &one), ["Clients"]);
        assert!(categories_of(&store, &two).is_empty());
        assert!(categories_of(&store, &three).is_empty());
        assert_eq!(
            store.list_categories().unwrap(),
            [cat("Clients", "#123456")]
        );
    }

    #[test]
    fn server_categories_are_added_with_palette_colours() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let inbox = add_folder(&store, &account, "INBOX", FolderRole::Inbox);
        let id = add_message(&store, &inbox, 7, &summary("labelled", 1));
        store.save_category(&cat("Work", "#ff0000"), None).unwrap();
        let from_server = vec!["work".to_owned(), "Receipts".to_owned()];
        assert!(store.ensure_categories(&from_server).unwrap());
        assert!(!store.ensure_categories(&from_server).unwrap());
        let all = store.list_categories().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "Receipts");
        assert_eq!(all[0].color, mail_core::categories::palette_color(1));

        assert!(
            store
                .set_categories_by_uid(&inbox, 7, &from_server)
                .unwrap()
        );
        assert!(
            !store
                .set_categories_by_uid(&inbox, 7, &from_server)
                .unwrap()
        );
        assert_eq!(categories_of(&store, &id), ["work", "Receipts"]);
        assert!(store.set_categories_by_uid(&inbox, 7, &[]).unwrap());
        assert!(categories_of(&store, &id).is_empty());
        assert!(
            !store
                .set_categories_by_uid(&inbox, 99, &from_server)
                .unwrap()
        );
    }
}
