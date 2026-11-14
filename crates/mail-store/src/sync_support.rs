//! Queries the sync engine needs: updating flags by server UID, finding
//! messages whose body has not been downloaded, and caching a body together
//! with its list preview.

use rusqlite::params;

use crate::messages::MessageFlags;
use crate::{Result, Store, StoreError};

impl Store {
    /// Apply flags fetched from the server to the message with this UID.
    /// Returns whether a row changed (false when the UID is unknown).
    ///
    /// # Errors
    /// Database errors.
    pub fn set_flags_by_uid(&self, folder_id: &str, uid: u32, flags: MessageFlags) -> Result<bool> {
        let changed = self.conn().execute(
            "UPDATE messages SET seen = ?3, flagged = ?4, answered = ?5
             WHERE folder_id = ?1 AND uid = ?2
               AND (seen != ?3 OR flagged != ?4 OR answered != ?5)",
            params![folder_id, uid, flags.seen, flags.flagged, flags.answered],
        )?;
        Ok(changed > 0)
    }

    /// Newest messages in a folder whose body is not cached yet, skipping
    /// any larger than `max_size` bytes. Returns `(id, uid)` pairs.
    ///
    /// # Errors
    /// Database errors.
    pub fn messages_missing_raw(
        &self,
        folder_id: &str,
        limit: u32,
        max_size: u64,
    ) -> Result<Vec<(String, u32)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, uid FROM messages
             WHERE folder_id = ?1 AND raw IS NULL AND uid IS NOT NULL AND size <= ?3
             ORDER BY date IS NULL, date DESC LIMIT ?2",
        )?;
        let max_size = i64::try_from(max_size).unwrap_or(i64::MAX);
        let rows = stmt.query_map(params![folder_id, limit, max_size], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Cache the full message, its searchable text and a list preview
    /// (header-only sync leaves the preview empty). An empty `preview`
    /// keeps the current one.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if there is no such message.
    pub fn set_body(
        &self,
        id: &str,
        raw: &[u8],
        body_text: Option<&str>,
        preview: &str,
    ) -> Result<()> {
        let changed = self.conn().execute(
            "UPDATE messages SET raw = ?2, body_text = ?3,
                    preview = CASE WHEN ?4 = '' THEN preview ELSE ?4 END
             WHERE id = ?1",
            params![id, raw, body_text, preview],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound("message"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use mail_core::folders::FolderRole;

    use super::*;
    use crate::messages::ListQuery;
    use crate::test_support::{add_account, add_folder, add_message, summary};

    #[test]
    fn flags_by_uid_only_touch_that_message() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@x.co");
        let inbox = add_folder(&store, &account, "INBOX", FolderRole::Inbox);
        add_message(&store, &inbox, 1, &summary("one", 10));
        add_message(&store, &inbox, 2, &summary("two", 20));
        let read = MessageFlags {
            seen: true,
            flagged: true,
            answered: false,
        };
        assert!(store.set_flags_by_uid(&inbox, 2, read).unwrap());
        assert!(
            !store.set_flags_by_uid(&inbox, 2, read).unwrap(),
            "no-op when unchanged"
        );
        assert!(!store.set_flags_by_uid(&inbox, 99, read).unwrap());
        let page = store
            .list_messages(&ListQuery {
                folder_id: Some(inbox.clone()),
                limit: 10,
                ..ListQuery::default()
            })
            .unwrap();
        let two = page.items.iter().find(|m| m.subject == "two").unwrap();
        let one = page.items.iter().find(|m| m.subject == "one").unwrap();
        assert!(!two.unread && two.flagged);
        assert!(one.unread && !one.flagged);
    }

    #[test]
    fn missing_bodies_are_listed_newest_first_and_cached_with_preview() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@x.co");
        let inbox = add_folder(&store, &account, "INBOX", FolderRole::Inbox);
        let old = add_message(&store, &inbox, 1, &summary("old", 10));
        let new = add_message(&store, &inbox, 2, &summary("new", 20));
        let missing = store.messages_missing_raw(&inbox, 10, u64::MAX).unwrap();
        assert_eq!(missing, vec![(new.clone(), 2), (old.clone(), 1)]);

        store
            .set_body(&new, b"raw", Some("hello world"), "hello world")
            .unwrap();
        assert_eq!(
            store.messages_missing_raw(&inbox, 10, u64::MAX).unwrap(),
            vec![(old, 1)]
        );
        assert_eq!(store.get_raw(&new).unwrap().as_deref(), Some(&b"raw"[..]));
        let page = store
            .list_messages(&ListQuery {
                search: Some("hello".into()),
                limit: 10,
                ..ListQuery::default()
            })
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].preview, "hello world");
        assert!(store.set_body("nope", b"x", None, "").is_err());
    }
}
