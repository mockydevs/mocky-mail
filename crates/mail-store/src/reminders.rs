//! Reminders dismissed or snoozed (so they stay that way after a restart),
//! flagged messages whose follow-up date came, and which Gmail and
//! Microsoft accounts are connected for calendar, contacts and tasks.

use std::collections::HashMap;

use mail_core::reminders::ReminderState;
use rusqlite::{OptionalExtension, params};

use crate::Store;
use crate::error::Result;

/// How long a dismissed or snoozed reminder is remembered (its event has
/// long passed by then): 60 days.
const KEEP_STATES_SECS: i64 = 60 * 86_400;

impl Store {
    /// Every reminder dismissed or snoozed, by key.
    ///
    /// # Errors
    /// Database errors.
    pub fn reminder_states(&self) -> Result<HashMap<String, ReminderState>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT key, dismissed, snoozed_until FROM reminder_states")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                ReminderState {
                    dismissed: row.get(1)?,
                    snoozed_until: row.get(2)?,
                },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Remember a reminder dismissed or snoozed. Old entries are dropped.
    ///
    /// # Errors
    /// Database errors.
    pub fn set_reminder_state(&self, key: &str, state: ReminderState, now: i64) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO reminder_states (key, dismissed, snoozed_until, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (key) DO UPDATE SET dismissed = excluded.dismissed,
                 snoozed_until = excluded.snoozed_until, updated_at = excluded.updated_at",
            params![key, state.dismissed, state.snoozed_until, now],
        )?;
        conn.execute(
            "DELETE FROM reminder_states WHERE updated_at < ?1",
            params![now.saturating_sub(KEEP_STATES_SECS)],
        )?;
        Ok(())
    }

    /// Flagged messages whose follow-up date is after `since` and at or
    /// before `upto`: (id, subject, date).
    ///
    /// # Errors
    /// Database errors.
    pub fn flags_due_between(&self, since: i64, upto: i64) -> Result<Vec<(String, String, i64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, subject, flag_due FROM messages
             WHERE flag_due > ?1 AND flag_due <= ?2 AND flagged = 1
             ORDER BY flag_due, seq LIMIT 500",
        )?;
        let rows = stmt.query_map(params![since, upto], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Record that an account was connected for calendar, contacts and
    /// tasks, with the permissions granted.
    ///
    /// # Errors
    /// Database errors.
    pub fn set_account_connection(
        &self,
        account_id: &str,
        scopes: &[String],
        now: i64,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO account_connections (account_id, scopes, connected_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (account_id) DO UPDATE SET scopes = excluded.scopes,
                 connected_at = excluded.connected_at",
            params![account_id, scopes.join(" "), now],
        )?;
        Ok(())
    }

    /// The permissions granted when an account was connected, or `None`
    /// when it hasn't been.
    ///
    /// # Errors
    /// Database errors.
    pub fn account_connection(&self, account_id: &str) -> Result<Option<Vec<String>>> {
        let scopes: Option<String> = self
            .conn()
            .query_row(
                "SELECT scopes FROM account_connections WHERE account_id = ?1",
                params![account_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(scopes.map(|s| s.split_whitespace().map(str::to_owned).collect()))
    }

    /// Forget an account's connection (its sign-in was refused).
    ///
    /// # Errors
    /// Database errors.
    pub fn clear_account_connection(&self, account_id: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM account_connections WHERE account_id = ?1",
            params![account_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::test_support;

    #[test]
    fn dismissed_and_snoozed_reminders_are_remembered() {
        let store = Store::open_in_memory().unwrap();
        let snoozed = ReminderState {
            dismissed: false,
            snoozed_until: Some(1_000),
        };
        store.set_reminder_state("event:a:1", snoozed, 100).unwrap();
        store
            .set_reminder_state(
                "task:b:2",
                ReminderState {
                    dismissed: true,
                    snoozed_until: None,
                },
                100,
            )
            .unwrap();
        let states = store.reminder_states().unwrap();
        assert_eq!(states.len(), 2);
        assert_eq!(states["event:a:1"], snoozed);
        assert!(states["task:b:2"].dismissed);
        // Much later, old ones are let go.
        store
            .set_reminder_state(
                "flag:c:3",
                ReminderState::default(),
                100 + KEEP_STATES_SECS + 1,
            )
            .unwrap();
        assert_eq!(store.reminder_states().unwrap().len(), 1);
    }

    #[test]
    fn connections_are_recorded_per_account() {
        let store = Store::open_in_memory().unwrap();
        let account = test_support::add_account(&store, "me@gmail.example");
        assert!(store.account_connection(&account).unwrap().is_none());
        store
            .set_account_connection(&account, &["a".into(), "b".into()], 5)
            .unwrap();
        assert_eq!(
            store.account_connection(&account).unwrap().unwrap(),
            ["a", "b"]
        );
        store.clear_account_connection(&account).unwrap();
        assert!(store.account_connection(&account).unwrap().is_none());
    }
}
