//! Tasks: lists of to-dos on this device and on accounts (`CalDAV`
//! `VTODO`, Google Tasks, Microsoft To Do), kept here so they work offline.
//!
//! As with contacts, a change is made here first and marked `changed` or
//! `deleted`; the sync sends it and marks it `synced`. Until then a server
//! copy never overwrites it. The device's own list (`device`) always
//! exists and never syncs.

use mail_core::tasks::TaskFields;
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::Store;
use crate::contacts::ContactState;
use crate::error::{Result, StoreError};
use crate::ids::new_id;

/// The id of the device's own list.
pub const DEVICE_TASK_LIST: &str = "device";
/// Most tasks one list returns.
pub const MAX_TASKS: usize = 5000;

/// Who keeps a task list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskProvider {
    /// This device only.
    Device,
    /// A `CalDAV` calendar that holds to-dos.
    Caldav,
    /// Google Tasks.
    Google,
    /// Microsoft To Do.
    Microsoft,
}

impl TaskProvider {
    /// The stored name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Caldav => "caldav",
            Self::Google => "google",
            Self::Microsoft => "microsoft",
        }
    }

    fn parse(text: &str) -> Result<Self> {
        match text {
            "device" => Ok(Self::Device),
            "caldav" => Ok(Self::Caldav),
            "google" => Ok(Self::Google),
            "microsoft" => Ok(Self::Microsoft),
            _ => Err(StoreError::Corrupt("task list provider")),
        }
    }
}

/// A list of tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskList {
    /// Local id (`device` for the device's own list).
    pub id: String,
    /// The account it belongs to; `None` for the device's list.
    pub account_id: Option<String>,
    /// Its name.
    pub name: String,
    /// Who keeps it.
    pub provider: TaskProvider,
    /// The provider's id for it (a `CalDAV` URL, a Google or Microsoft id).
    #[serde(skip)]
    pub remote_id: Option<String>,
}

/// A task as the app shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    /// Local id.
    pub id: String,
    /// The list it is in.
    pub list_id: String,
    /// Its fields.
    #[serde(flatten)]
    pub fields: TaskFields,
    /// The flagged message it came from (this device only).
    pub message_id: Option<String>,
    /// Whether a change is still to be sent.
    pub state: ContactState,
}

/// A task's server side, for syncing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSync {
    /// The task.
    pub task: Task,
    /// Stable id inside the task (`UID`).
    pub uid: String,
    /// The provider's id for it (a `CalDAV` path, a Google or Microsoft id).
    pub remote_id: Option<String>,
    /// The server's version tag.
    pub etag: Option<String>,
    /// The server's copy (`VTODO` text, or the provider's JSON).
    pub raw: Option<String>,
}

/// A task found on a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerTask<'a> {
    /// Its id there.
    pub remote_id: &'a str,
    /// Its version tag.
    pub etag: Option<&'a str>,
    /// Its text there.
    pub raw: &'a str,
    /// Its `UID` (or the provider id).
    pub uid: &'a str,
    /// Its fields.
    pub fields: &'a TaskFields,
}

const fn state_name(state: ContactState) -> &'static str {
    match state {
        ContactState::Synced => "synced",
        ContactState::Changed => "changed",
        ContactState::Deleted => "deleted",
    }
}

fn state_of(text: &str) -> Result<ContactState> {
    match text {
        "synced" => Ok(ContactState::Synced),
        "changed" => Ok(ContactState::Changed),
        "deleted" => Ok(ContactState::Deleted),
        _ => Err(StoreError::Corrupt("task state")),
    }
}

/// id, account, name, provider, remote id.
type RawList = (String, Option<String>, String, String, Option<String>);

fn list_row(row: &Row<'_>) -> rusqlite::Result<RawList> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}

fn to_list(raw: RawList) -> Result<TaskList> {
    let (id, account_id, name, provider, remote_id) = raw;
    Ok(TaskList {
        id,
        account_id,
        name,
        provider: TaskProvider::parse(&provider)?,
        remote_id,
    })
}

/// id, list, fields, state, message, uid, remote, etag, raw.
type RawTask = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

const TASK_COLUMNS: &str = "id, list_id, fields, state, message_id, uid, remote_id, etag, raw";

fn task_row(row: &Row<'_>) -> rusqlite::Result<RawTask> {
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

fn to_task(raw: RawTask) -> Result<TaskSync> {
    let (id, list_id, fields, state, message_id, uid, remote_id, etag, raw) = raw;
    Ok(TaskSync {
        task: Task {
            id,
            list_id,
            fields: serde_json::from_str(&fields)?,
            message_id,
            state: state_of(&state)?,
        },
        uid,
        remote_id,
        etag,
        raw,
    })
}

impl Store {
    /// Every task list: the device's first, then accounts' by name.
    ///
    /// # Errors
    /// Database errors.
    pub fn list_task_lists(&self) -> Result<Vec<TaskList>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, name, provider, remote_id FROM task_lists
             ORDER BY provider != 'device', account_id, name COLLATE NOCASE, id",
        )?;
        let rows = stmt.query_map([], list_row)?;
        rows.map(|r| to_list(r?)).collect()
    }

    /// One task list.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] when there is none.
    pub fn get_task_list(&self, id: &str) -> Result<TaskList> {
        let raw = self
            .conn()
            .query_row(
                "SELECT id, account_id, name, provider, remote_id FROM task_lists WHERE id = ?1",
                params![id],
                list_row,
            )
            .optional()?
            .ok_or(StoreError::NotFound("task list"))?;
        to_list(raw)
    }

    /// Record an account's task lists with one provider as found on its
    /// server: `(remote id, name)`. Lists no longer there go, with their
    /// tasks. Returns the account's lists of that provider.
    ///
    /// # Errors
    /// Database errors.
    pub fn set_task_lists(
        &self,
        account_id: &str,
        provider: TaskProvider,
        found: &[(String, String)],
    ) -> Result<Vec<TaskList>> {
        {
            let mut conn = self.conn();
            let tx = conn.transaction()?;
            for (remote_id, name) in found {
                tx.execute(
                    "INSERT INTO task_lists (id, account_id, provider, remote_id, name)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (account_id, remote_id) DO UPDATE SET name = excluded.name,
                         provider = excluded.provider",
                    params![new_id()?, account_id, provider.as_str(), remote_id, name],
                )?;
            }
            let existing: Vec<(String, Option<String>)> = {
                let mut stmt = tx.prepare(
                    "SELECT id, remote_id FROM task_lists WHERE account_id = ?1 AND provider = ?2",
                )?;
                let rows = stmt.query_map(params![account_id, provider.as_str()], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            for (id, remote) in existing {
                if !found.iter().any(|(r, _)| Some(r) == remote.as_ref()) {
                    tx.execute("DELETE FROM task_lists WHERE id = ?1", params![id])?;
                }
            }
            tx.commit()?;
        }
        Ok(self
            .list_task_lists()?
            .into_iter()
            .filter(|l| l.account_id.as_deref() == Some(account_id) && l.provider == provider)
            .collect())
    }

    /// Tasks in one list (or every list), not deleted. Open tasks first by
    /// due date, then done ones by when they were done.
    ///
    /// # Errors
    /// Database errors.
    pub fn list_tasks(&self, list_id: Option<&str>, include_completed: bool) -> Result<Vec<Task>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks
             WHERE state != 'deleted' AND (?1 IS NULL OR list_id = ?1) AND (?2 OR completed = 0)
             ORDER BY completed, due_at IS NULL, due_at, updated_at DESC, id LIMIT ?3"
        ))?;
        let limit = i64::try_from(MAX_TASKS).unwrap_or(i64::MAX);
        let rows = stmt.query_map(params![list_id, include_completed, limit], task_row)?;
        rows.map(|r| Ok(to_task(r?)?.task)).collect()
    }

    fn task_sync(&self, id: &str) -> Result<TaskSync> {
        let raw = self
            .conn()
            .query_row(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1 AND state != 'deleted'"),
                params![id],
                task_row,
            )
            .optional()?
            .ok_or(StoreError::NotFound("task"))?;
        to_task(raw)
    }

    /// One task.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] when there is none.
    pub fn get_task(&self, id: &str) -> Result<Task> {
        Ok(self.task_sync(id)?.task)
    }

    fn write_task_row(
        &self,
        id: &str,
        fields: &TaskFields,
        state: ContactState,
        now: i64,
    ) -> Result<()> {
        self.conn().execute(
            "UPDATE tasks SET fields = ?2, state = ?3, due_at = ?4, reminder_at = ?5,
                 completed = ?6, updated_at = ?7 WHERE id = ?1",
            params![
                id,
                serde_json::to_string(fields)?,
                state_name(state),
                fields.due,
                fields.reminder,
                fields.completed,
                now
            ],
        )?;
        Ok(())
    }

    /// Create (`id` `None`) or change a task. Moving it to another list
    /// deletes it from the old one (on its server too) and creates it in
    /// the new one. Tasks of an account's list are marked to be sent.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown task or list.
    pub fn save_task(
        &self,
        id: Option<&str>,
        list_id: &str,
        fields: &TaskFields,
        message_id: Option<&str>,
        now: u64,
    ) -> Result<Task> {
        let now = crate::ids::to_i64(now, "time")?;
        let list = self.get_task_list(list_id)?;
        let state = if list.provider == TaskProvider::Device {
            ContactState::Synced
        } else {
            ContactState::Changed
        };
        if let Some(id) = id {
            let old = self.task_sync(id)?;
            if old.task.list_id == list_id {
                self.write_task_row(id, fields, state, now)?;
                if message_id.is_some() {
                    self.conn().execute(
                        "UPDATE tasks SET message_id = ?2 WHERE id = ?1",
                        params![id, message_id],
                    )?;
                }
                return self.get_task(id);
            }
            // Another list: a new task there, the old one deleted.
            let keep_message = message_id
                .map(str::to_owned)
                .or_else(|| old.task.message_id.clone());
            self.delete_task(id)?;
            return self.save_task(
                None,
                list_id,
                fields,
                keep_message.as_deref(),
                u64::try_from(now).unwrap_or(0),
            );
        }
        let new = new_id()?;
        self.conn().execute(
            "INSERT INTO tasks (id, list_id, uid, fields, state, message_id, updated_at)
             VALUES (?1, ?2, ?3, '{}', 'synced', ?4, ?5)",
            params![new, list_id, format!("{new}@mocky.mail"), message_id, now],
        )?;
        self.write_task_row(&new, fields, state, now)?;
        self.get_task(&new)
    }

    /// Delete a task. One on a server (or on its way there) is kept,
    /// marked `deleted`, until the sync deletes the server copy.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown task.
    pub fn delete_task(&self, id: &str) -> Result<()> {
        let sync = self.task_sync(id)?;
        if sync.remote_id.is_some() || sync.task.state == ContactState::Changed {
            self.conn().execute(
                "UPDATE tasks SET state = 'deleted' WHERE id = ?1",
                params![id],
            )?;
        } else {
            self.conn()
                .execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        }
        Ok(())
    }

    /// A list's tasks with changes to send.
    ///
    /// # Errors
    /// Database errors.
    pub fn tasks_to_push(&self, list_id: &str) -> Result<Vec<TaskSync>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE list_id = ?1 AND state != 'synced'"
        ))?;
        let rows = stmt.query_map(params![list_id], task_row)?;
        rows.map(|r| to_task(r?)).collect()
    }

    /// The server copies known in a list: remote id to (version tag,
    /// state).
    ///
    /// # Errors
    /// Database errors.
    pub fn task_etags(
        &self,
        list_id: &str,
    ) -> Result<std::collections::HashMap<String, (Option<String>, ContactState)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT remote_id, etag, state FROM tasks WHERE list_id = ?1 AND remote_id IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![list_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut out = std::collections::HashMap::new();
        for row in rows {
            let (remote, etag, state) = row?;
            out.insert(remote, (etag, state_of(&state)?));
        }
        Ok(out)
    }

    /// A change reached the server: remember where and its new version.
    /// The task is marked `synced` only if its fields are still `sent`.
    ///
    /// # Errors
    /// Database errors.
    pub fn task_pushed(
        &self,
        id: &str,
        remote_id: &str,
        etag: Option<&str>,
        raw: &str,
        sent: &TaskFields,
    ) -> Result<()> {
        let conn = self.conn();
        let row: Option<(String, String)> = conn
            .query_row(
                "SELECT fields, state FROM tasks WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((fields, state)) = row else {
            return Ok(());
        };
        let unchanged = state == "changed"
            && serde_json::from_str::<TaskFields>(&fields).is_ok_and(|f| f == *sent);
        let state = if unchanged { "synced" } else { state.as_str() };
        conn.execute(
            "UPDATE tasks SET remote_id = ?2, etag = ?3, raw = ?4, state = ?5 WHERE id = ?1",
            params![id, remote_id, etag, raw, state],
        )?;
        Ok(())
    }

    /// Store a task read from the server. One with a change still to send
    /// is left alone. Returns its local id.
    ///
    /// # Errors
    /// Database errors.
    pub fn apply_server_task(
        &self,
        list_id: &str,
        task: &ServerTask<'_>,
        now: u64,
    ) -> Result<String> {
        let now = crate::ids::to_i64(now, "time")?;
        let existing: Option<(String, String)> = self
            .conn()
            .query_row(
                "SELECT id, state FROM tasks WHERE list_id = ?1 AND remote_id = ?2",
                params![list_id, task.remote_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let id = match existing {
            Some((id, state)) if state != "synced" => return Ok(id),
            Some((id, _)) => id,
            None => {
                let id = new_id()?;
                self.conn().execute(
                    "INSERT INTO tasks (id, list_id, uid, remote_id, fields, state, updated_at)
                     VALUES (?1, ?2, ?3, ?4, '{}', 'synced', ?5)",
                    params![id, list_id, task.uid, task.remote_id, now],
                )?;
                id
            }
        };
        self.conn().execute(
            "UPDATE tasks SET etag = ?2, raw = ?3, uid = ?4 WHERE id = ?1",
            params![id, task.etag, task.raw, task.uid],
        )?;
        self.write_task_row(&id, task.fields, ContactState::Synced, now)?;
        Ok(id)
    }

    /// The server copy and local-only fields of a task, by its id there.
    ///
    /// # Errors
    /// Database errors.
    pub fn task_by_remote_id(&self, list_id: &str, remote_id: &str) -> Result<Option<TaskSync>> {
        let raw = self
            .conn()
            .query_row(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE list_id = ?1 AND remote_id = ?2"),
                params![list_id, remote_id],
                task_row,
            )
            .optional()?;
        raw.map(to_task).transpose()
    }

    /// Remove a task for good (its server copy is gone).
    ///
    /// # Errors
    /// Database errors.
    pub fn forget_task(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Drop synced tasks whose server copy is gone.
    ///
    /// # Errors
    /// Database errors.
    pub fn forget_tasks_not_on_server(&self, list_id: &str, on_server: &[String]) -> Result<usize> {
        let mut removed = 0usize;
        for (remote, (_, state)) in self.task_etags(list_id)? {
            if state == ContactState::Synced && !on_server.contains(&remote) {
                removed = removed.saturating_add(self.conn().execute(
                    "DELETE FROM tasks WHERE list_id = ?1 AND remote_id = ?2",
                    params![list_id, remote],
                )?);
            }
        }
        Ok(removed)
    }

    /// Open tasks with a reminder at or before `upto`, for the due list.
    ///
    /// # Errors
    /// Database errors.
    pub fn tasks_reminding_before(&self, upto: i64) -> Result<Vec<Task>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks
             WHERE reminder_at IS NOT NULL AND reminder_at <= ?1 AND completed = 0
               AND state != 'deleted'
             ORDER BY reminder_at LIMIT 500"
        ))?;
        let rows = stmt.query_map(params![upto], task_row)?;
        rows.map(|r| Ok(to_task(r?)?.task)).collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::test_support;

    fn fields(title: &str) -> TaskFields {
        TaskFields {
            title: title.into(),
            ..TaskFields::default()
        }
    }

    #[test]
    fn the_device_list_always_exists() {
        let store = Store::open_in_memory().unwrap();
        let lists = store.list_task_lists().unwrap();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].id, DEVICE_TASK_LIST);
        assert_eq!(lists[0].provider, TaskProvider::Device);
        let task = store
            .save_task(
                None,
                DEVICE_TASK_LIST,
                &fields("Call Peter"),
                Some("msg-1"),
                5,
            )
            .unwrap();
        assert_eq!(task.state, ContactState::Synced);
        assert_eq!(task.message_id.as_deref(), Some("msg-1"));
        // Device tasks are never sent anywhere.
        assert!(store.tasks_to_push(DEVICE_TASK_LIST).unwrap().is_empty());
        let done = TaskFields {
            completed: true,
            completed_at: Some(9),
            ..fields("Call Peter")
        };
        store
            .save_task(Some(&task.id), DEVICE_TASK_LIST, &done, None, 9)
            .unwrap();
        assert!(store.list_tasks(None, false).unwrap().is_empty());
        let all = store.list_tasks(Some(DEVICE_TASK_LIST), true).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].message_id.as_deref(), Some("msg-1"));
        store.delete_task(&task.id).unwrap();
        assert!(store.list_tasks(None, true).unwrap().is_empty());
        let json = serde_json::to_value(&all[0]).unwrap();
        assert_eq!(json["listId"], "device");
        assert_eq!(json["completedAt"], 9);
        assert_eq!(json["state"], "synced");
    }

    #[test]
    fn account_tasks_sync_and_wait_offline() {
        let store = Store::open_in_memory().unwrap();
        let account = test_support::add_account(&store, "me@work.example");
        let lists = store
            .set_task_lists(
                &account,
                TaskProvider::Caldav,
                &[("https://x/tasks/".into(), "Tasks".into())],
            )
            .unwrap();
        let list = lists[0].id.clone();
        let mine = store
            .save_task(None, &list, &fields("Quote"), None, 5)
            .unwrap();
        assert_eq!(mine.state, ContactState::Changed);
        let pending = store.tasks_to_push(&list).unwrap();
        assert_eq!(pending.len(), 1);
        // Edited while being sent: stays changed.
        store
            .save_task(Some(&mine.id), &list, &fields("Quote v2"), None, 6)
            .unwrap();
        store
            .task_pushed(
                &mine.id,
                "/tasks/a.ics",
                Some("\"1\""),
                "ics",
                &pending[0].task.fields,
            )
            .unwrap();
        assert_eq!(store.tasks_to_push(&list).unwrap().len(), 1);
        let pending = store.tasks_to_push(&list).unwrap();
        store
            .task_pushed(
                &mine.id,
                "/tasks/a.ics",
                Some("\"2\""),
                "ics",
                &pending[0].task.fields,
            )
            .unwrap();
        assert!(store.tasks_to_push(&list).unwrap().is_empty());
        // From the server: new, then changed; a local change wins.
        let theirs = fields("From webmail");
        store
            .apply_server_task(
                &list,
                &ServerTask {
                    remote_id: "/tasks/b.ics",
                    etag: Some("\"1\""),
                    raw: "ics",
                    uid: "b",
                    fields: &theirs,
                },
                7,
            )
            .unwrap();
        assert_eq!(store.list_tasks(Some(&list), true).unwrap().len(), 2);
        assert_eq!(store.task_etags(&list).unwrap().len(), 2);
        store
            .save_task(Some(&mine.id), &list, &fields("Mine"), None, 8)
            .unwrap();
        store
            .apply_server_task(
                &list,
                &ServerTask {
                    remote_id: "/tasks/a.ics",
                    etag: Some("\"3\""),
                    raw: "ics",
                    uid: "a",
                    fields: &fields("Theirs"),
                },
                9,
            )
            .unwrap();
        assert_eq!(store.get_task(&mine.id).unwrap().fields.title, "Mine");
        // Gone on the server: dropped, unless changed here.
        store
            .forget_tasks_not_on_server(&list, &["/tasks/a.ics".to_owned()])
            .unwrap();
        assert_eq!(store.list_tasks(Some(&list), true).unwrap().len(), 1);
        // Deleted here: kept until the server copy goes.
        store.delete_task(&mine.id).unwrap();
        assert!(store.list_tasks(None, true).unwrap().is_empty());
        assert_eq!(
            store.tasks_to_push(&list).unwrap()[0].task.state,
            ContactState::Deleted
        );
        store.forget_task(&mine.id).unwrap();
        // A list gone from the server takes its tasks.
        store
            .set_task_lists(&account, TaskProvider::Caldav, &[])
            .unwrap();
        assert_eq!(store.list_task_lists().unwrap().len(), 1);
    }

    #[test]
    fn moving_a_task_to_another_list() {
        let store = Store::open_in_memory().unwrap();
        let account = test_support::add_account(&store, "me@work.example");
        let list = store
            .set_task_lists(
                &account,
                TaskProvider::Google,
                &[("g1".into(), "My Tasks".into())],
            )
            .unwrap()[0]
            .id
            .clone();
        let task = store
            .save_task(None, DEVICE_TASK_LIST, &fields("Move me"), Some("m"), 1)
            .unwrap();
        let moved = store
            .save_task(Some(&task.id), &list, &fields("Move me"), None, 2)
            .unwrap();
        assert_eq!(moved.list_id, list);
        assert_eq!(moved.message_id.as_deref(), Some("m"));
        assert_eq!(store.list_tasks(None, true).unwrap().len(), 1);
    }

    #[test]
    fn reminders_come_from_open_tasks() {
        let store = Store::open_in_memory().unwrap();
        let soon = TaskFields {
            reminder: Some(100),
            ..fields("Soon")
        };
        let later = TaskFields {
            reminder: Some(900),
            ..fields("Later")
        };
        let done = TaskFields {
            reminder: Some(50),
            completed: true,
            ..fields("Done")
        };
        for f in [&soon, &later, &done] {
            store.save_task(None, DEVICE_TASK_LIST, f, None, 1).unwrap();
        }
        let due = store.tasks_reminding_before(500).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].fields.title, "Soon");
    }
}
