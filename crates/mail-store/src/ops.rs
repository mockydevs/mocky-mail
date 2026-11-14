//! Offline actions waiting to be replayed on the server by the sync engine.

use mail_core::outbox::UnixSeconds;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::Store;
use crate::error::{Result, StoreError};
use crate::ids::{new_id, to_i64, to_u64};

/// What kind of action a pending operation replays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    /// Change flags.
    SetFlags,
    /// Move to another folder.
    Move,
    /// Delete for good.
    Delete,
}

impl OpKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::SetFlags => "set_flags",
            Self::Move => "move",
            Self::Delete => "delete",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "set_flags" => Ok(Self::SetFlags),
            "move" => Ok(Self::Move),
            "delete" => Ok(Self::Delete),
            _ => Err(StoreError::Corrupt("pending op kind")),
        }
    }
}

/// One queued offline action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOp {
    /// Opaque ID.
    pub id: String,
    /// Account it applies to.
    pub account_id: String,
    /// When it was queued.
    pub created_at: UnixSeconds,
    /// What to replay.
    pub kind: OpKind,
    /// Details defined by the sync engine (folder, UIDs, flags).
    pub payload: serde_json::Value,
}

impl Store {
    /// Queue an offline action. Returns its ID.
    ///
    /// # Errors
    /// Fails on a database error, for example an unknown account.
    pub fn enqueue_op(
        &self,
        account_id: &str,
        kind: OpKind,
        payload: &serde_json::Value,
        now: UnixSeconds,
    ) -> Result<String> {
        let id = new_id()?;
        let payload = serde_json::to_string(payload)?;
        self.conn().execute(
            "INSERT INTO pending_ops (id, account_id, created_at, kind, payload)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                id,
                account_id,
                to_i64(now, "created_at")?,
                kind.as_str(),
                payload
            ],
        )?;
        Ok(id)
    }

    /// An account's pending actions, oldest first (the order to replay).
    ///
    /// # Errors
    /// Fails on a database error or an unreadable row.
    pub fn list_ops(&self, account_id: &str) -> Result<Vec<PendingOp>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, account_id, created_at, kind, payload FROM pending_ops
             WHERE account_id = ?1 ORDER BY created_at, seq",
        )?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        rows.map(|row| {
            let (id, account_id, created_at, kind, payload) = row?;
            Ok(PendingOp {
                id,
                account_id,
                created_at: to_u64(created_at, "op created_at")?,
                kind: OpKind::parse(&kind)?,
                payload: serde_json::from_str(&payload)?,
            })
        })
        .collect()
    }

    /// Count one more refusal from the server for this action and return
    /// the new total. Returns 0 when the action no longer exists.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn record_op_refusal(&self, id: &str) -> Result<u32> {
        let refusals: Option<i64> = self
            .conn()
            .query_row(
                "UPDATE pending_ops SET refusals = refusals + 1 WHERE id = ?1
                 RETURNING refusals",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(refusals.map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX)))
    }

    /// Remove an action once the server has applied it. Returns whether it
    /// existed.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn delete_op(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM pending_ops WHERE id = ?1", params![id])?
            > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::add_account;
    use serde_json::json;

    #[test]
    fn lists_oldest_first_per_account() {
        let store = Store::open_in_memory().unwrap();
        let a = add_account(&store, "a@example.com");
        let b = add_account(&store, "b@example.com");
        let late = store
            .enqueue_op(&a, OpKind::Delete, &json!({"uids": [3]}), 20)
            .unwrap();
        let early = store
            .enqueue_op(&a, OpKind::SetFlags, &json!({"seen": true}), 10)
            .unwrap();
        let same_time = store
            .enqueue_op(&a, OpKind::Move, &json!({"to": "Archive"}), 20)
            .unwrap();
        store.enqueue_op(&b, OpKind::Move, &json!({}), 1).unwrap();

        let ops = store.list_ops(&a).unwrap();
        let ids: Vec<&str> = ops.iter().map(|op| op.id.as_str()).collect();
        assert_eq!(ids, [early.as_str(), late.as_str(), same_time.as_str()]);
        let first = ops.first().unwrap();
        assert_eq!(first.kind, OpKind::SetFlags);
        assert_eq!(first.payload, json!({"seen": true}));
        assert_eq!(first.created_at, 10);
        assert_eq!(first.account_id, a);
    }

    #[test]
    fn delete_op_removes_one() {
        let store = Store::open_in_memory().unwrap();
        let a = add_account(&store, "a@example.com");
        let first = store.enqueue_op(&a, OpKind::Delete, &json!(1), 1).unwrap();
        store.enqueue_op(&a, OpKind::Delete, &json!(2), 2).unwrap();
        assert!(store.delete_op(&first).unwrap());
        assert!(!store.delete_op(&first).unwrap());
        assert_eq!(store.list_ops(&a).unwrap().len(), 1);
    }

    #[test]
    fn refusals_are_counted_per_change() {
        let store = Store::open_in_memory().unwrap();
        let a = add_account(&store, "a@example.com");
        let first = store.enqueue_op(&a, OpKind::Move, &json!(1), 1).unwrap();
        let second = store.enqueue_op(&a, OpKind::Move, &json!(2), 2).unwrap();
        assert_eq!(store.record_op_refusal(&first).unwrap(), 1);
        assert_eq!(store.record_op_refusal(&first).unwrap(), 2);
        assert_eq!(store.record_op_refusal(&second).unwrap(), 1);
        store.delete_op(&first).unwrap();
        assert_eq!(store.record_op_refusal(&first).unwrap(), 0);
    }

    #[test]
    fn enqueue_for_unknown_account_fails() {
        let store = Store::open_in_memory().unwrap();
        assert!(
            store
                .enqueue_op("nope", OpKind::Move, &json!({}), 1)
                .is_err()
        );
    }

    #[test]
    fn kind_names_round_trip() {
        for kind in [OpKind::SetFlags, OpKind::Move, OpKind::Delete] {
            assert_eq!(OpKind::parse(kind.as_str()).unwrap(), kind);
        }
        assert!(OpKind::parse("other").is_err());
    }
}
