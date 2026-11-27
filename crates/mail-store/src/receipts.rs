//! Read receipts answered here: each one a message asked for is sent or
//! declined once.

use rusqlite::{OptionalExtension, params};

use crate::Store;
use crate::error::Result;
use crate::ids::to_i64;

/// The key a message's read receipt is kept under: its Message-ID, or its
/// local id when it has none.
#[must_use]
pub fn read_receipt_key(message_id_header: Option<&str>, local_id: &str) -> String {
    match message_id_header
        .map(|id| id.trim().trim_start_matches('<').trim_end_matches('>'))
        .filter(|id| !id.is_empty())
    {
        Some(id) => id.to_owned(),
        None => format!("id:{local_id}"),
    }
}

impl Store {
    /// Whether the read receipt under `key` was answered, and if so whether
    /// it was sent.
    ///
    /// # Errors
    /// Database errors.
    pub fn read_receipt_answer(&self, account_id: &str, key: &str) -> Result<Option<bool>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT sent FROM read_receipts WHERE account_id = ?1 AND message_key = ?2",
                params![account_id, key],
                |row| row.get::<_, bool>(0),
            )
            .optional()?)
    }

    /// Record the answer to a read receipt, unless one is recorded already.
    /// Returns whether this call recorded it (so only one caller sends it).
    ///
    /// # Errors
    /// Database errors.
    pub fn answer_read_receipt(
        &self,
        account_id: &str,
        key: &str,
        sent: bool,
        at: u64,
    ) -> Result<bool> {
        let at = to_i64(at, "answered_at")?;
        Ok(self.conn().execute(
            "INSERT INTO read_receipts (account_id, message_key, sent, answered_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (account_id, message_key) DO NOTHING",
            params![account_id, key, sent, at],
        )? > 0)
    }

    /// Forget an answer (a receipt that couldn't be queued after all).
    ///
    /// # Errors
    /// Database errors.
    pub fn forget_read_receipt(&self, account_id: &str, key: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM read_receipts WHERE account_id = ?1 AND message_key = ?2",
            params![account_id, key],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::add_account;

    #[test]
    fn a_receipt_is_answered_once() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "me@example.com");
        let key = read_receipt_key(Some("<q-1@client.example>"), "m1");
        assert_eq!(key, "q-1@client.example");
        assert_eq!(read_receipt_key(None, "m1"), "id:m1");
        assert_eq!(read_receipt_key(Some(" "), "m1"), "id:m1");
        assert_eq!(store.read_receipt_answer(&account, &key).unwrap(), None);
        assert!(store.answer_read_receipt(&account, &key, true, 10).unwrap());
        // A second answer changes nothing.
        assert!(
            !store
                .answer_read_receipt(&account, &key, false, 11)
                .unwrap()
        );
        assert_eq!(
            store.read_receipt_answer(&account, &key).unwrap(),
            Some(true)
        );
        store.forget_read_receipt(&account, &key).unwrap();
        assert_eq!(store.read_receipt_answer(&account, &key).unwrap(), None);
        assert!(
            store
                .answer_read_receipt(&account, &key, false, 12)
                .unwrap()
        );
        assert_eq!(
            store.read_receipt_answer(&account, &key).unwrap(),
            Some(false)
        );
    }
}
