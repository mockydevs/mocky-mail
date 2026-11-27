//! Tasks: commands for the Tasks screen, and syncing task lists: the
//! device's own list, `CalDAV` calendars that hold to-dos (`VTODO`) on
//! password accounts' servers, Google Tasks and Microsoft To Do for
//! connected accounts.
//!
//! As with contacts, changes are saved here first and sent on the next
//! sync (every 15 minutes with the calendar, and soon after a change), so
//! tasks work offline. Google Tasks has no reminders, importance or
//! categories: for its tasks those are kept on this device only.

use std::sync::Arc;

use mail_core::ical;
use mail_core::importance::Importance;
use mail_core::tasks::TaskFields;
use mail_store::{Account, ContactState, ServerTask, Task, TaskList, TaskProvider};
use mail_sync::dav::{DavClient, DavLogin};
use mail_sync::rest::RestClient;
use mail_sync::{Credentials, SyncError, google, graph};
use serde::Deserialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, State, Url};

use crate::connections::{self, Service};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, now};

/// A task as the editor sends it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInput {
    /// `null` for a new task.
    id: Option<String>,
    list_id: String,
    title: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    due: Option<i64>,
    #[serde(default)]
    reminder: Option<i64>,
    #[serde(default)]
    importance: Importance,
    #[serde(default)]
    completed: bool,
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
}

fn stamp() -> i64 {
    i64::try_from(now()).unwrap_or(0)
}

/// `list_task_lists`: the device's list, then each account's.
#[tauri::command]
pub fn list_task_lists(state: State<'_, Arc<AppState>>) -> AppResult<Vec<TaskList>> {
    Ok(state.store.list_task_lists()?)
}

/// `list_tasks`: the tasks in one list (`null`: every list).
#[tauri::command]
pub fn list_tasks(
    state: State<'_, Arc<AppState>>,
    list_id: Option<String>,
    include_completed: bool,
) -> AppResult<Vec<Task>> {
    Ok(state
        .store
        .list_tasks(list_id.as_deref(), include_completed)?)
}

/// `save_task`: create or change a task; sent to its list's server on the
/// next sync.
#[tauri::command]
pub fn save_task(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    task: TaskInput,
) -> AppResult<Task> {
    let was = match task.id.as_deref() {
        Some(id) => Some(state.store.get_task(id)?),
        None => None,
    };
    let fields = mail_core::tasks::checked(
        TaskFields {
            title: task.title,
            notes: task.notes,
            due: task.due,
            reminder: task.reminder,
            importance: task.importance,
            completed: task.completed,
            completed_at: None,
            categories: task.categories,
        },
        was.as_ref().map(|t| &t.fields),
        stamp(),
    )
    .map_err(AppError::invalid)?;
    let message_id: Option<String> = task
        .message_id
        .map(|m| m.chars().filter(|c| !c.is_control()).take(200).collect())
        .filter(|m: &String| !m.trim().is_empty());
    let saved = state.store.save_task(
        task.id.as_deref(),
        &task.list_id,
        &fields,
        message_id.as_deref(),
        now(),
    )?;
    changed(&app);
    crate::reminders::refresh(&app, &state);
    soon(&app, &state);
    Ok(saved)
}

/// `delete_task`: gone here at once, from the server on the next sync.
#[tauri::command]
pub fn delete_task(app: AppHandle, state: State<'_, Arc<AppState>>, id: String) -> AppResult<()> {
    state.store.delete_task(&id)?;
    changed(&app);
    crate::reminders::refresh(&app, &state);
    soon(&app, &state);
    Ok(())
}

/// `sync_tasks`: sync every account's task lists now.
#[tauri::command]
pub async fn sync_tasks(app: AppHandle, state: State<'_, Arc<AppState>>) -> AppResult<()> {
    sync_all(&app, &state).await;
    Ok(())
}

fn changed(app: &AppHandle) {
    if let Err(error) = app.emit("mail://tasks-changed", json!({})) {
        eprintln!("couldn't emit tasks-changed: {error}");
    }
}

/// Sync shortly, after a change made here.
fn soon(app: &AppHandle, state: &Arc<AppState>) {
    let (app, state) = (app.clone(), Arc::clone(state));
    tauri::async_runtime::spawn(async move {
        sync_all(&app, &state).await;
    });
}

/// Sync every account's task lists (run with the calendar's timer).
pub(crate) async fn sync_all(app: &AppHandle, state: &Arc<AppState>) {
    let _guard = state.tasks_sync.lock().await;
    let accounts = match state.store.list_accounts() {
        Ok(accounts) => accounts,
        Err(error) => {
            eprintln!("tasks: couldn't list accounts: {error}");
            return;
        }
    };
    for account in accounts {
        if let Err(error) = sync_account(state, &account).await {
            eprintln!("tasks: {}: {error}", account.email);
        }
    }
    changed(app);
    crate::reminders::refresh(app, state);
}

async fn sync_account(state: &AppState, account: &Account) -> AppResult<()> {
    match connections::service(state, account)? {
        Service::NotConnected => Ok(()),
        Service::Dav => sync_dav(state, account).await,
        Service::Google => {
            if !connections::granted(state, account, "/auth/tasks") {
                return Ok(());
            }
            let api = connections::google_client(state, account).await?;
            let found = google::list_task_lists(&api).await?;
            for list in state
                .store
                .set_task_lists(&account.id, TaskProvider::Google, &found)?
            {
                sync_google_list(state, &api, &list).await?;
            }
            Ok(())
        }
        Service::Microsoft => {
            let api = connections::graph_client(state, account).await?;
            let found = graph::list_todo_lists(&api).await?;
            for list in state
                .store
                .set_task_lists(&account.id, TaskProvider::Microsoft, &found)?
            {
                sync_graph_list(state, &api, &list).await?;
            }
            Ok(())
        }
    }
}

async fn sync_dav(state: &AppState, account: &Account) -> AppResult<()> {
    let Credentials::Password { username, password } =
        crate::credentials::for_account(state, account).await?
    else {
        return Ok(());
    };
    let client = DavClient::new(DavLogin { username, password })?;
    let server = Url::parse(&format!("https://{}/", account.imap.host))
        .map_err(|e| AppError::internal(e.to_string()))?;
    let found = match client.discover_task_lists(&server).await {
        Ok(found) => found,
        // No CalDAV at all: the device list is still there.
        Err(SyncError::Config(_) | SyncError::Protocol(_) | SyncError::Refused(_)) => {
            return Ok(());
        }
        Err(other) => return Err(other.into()),
    };
    let lists = state.store.set_task_lists(
        &account.id,
        TaskProvider::Caldav,
        &found
            .iter()
            .filter(|c| c.writable)
            .map(|c| {
                (
                    c.url.to_string(),
                    c.name.clone().unwrap_or_else(|| "Tasks".to_owned()),
                )
            })
            .collect::<Vec<_>>(),
    )?;
    for list in lists {
        sync_dav_list(state, &client, &list).await?;
    }
    Ok(())
}

/// Send changes to a `CalDAV` to-do calendar, then read what changed.
async fn sync_dav_list(state: &AppState, client: &DavClient, list: &TaskList) -> AppResult<()> {
    let Some(remote) = list.remote_id.as_deref() else {
        return Ok(());
    };
    let url = Url::parse(remote).map_err(|e| AppError::internal(e.to_string()))?;
    for pending in state.store.tasks_to_push(&list.id)? {
        let id = pending.task.id.clone();
        match pending.task.state {
            ContactState::Deleted => {
                if let Some(href) = &pending.remote_id {
                    let mut deleted = client
                        .delete_event(&url, href, pending.etag.as_deref())
                        .await;
                    if matches!(deleted, Err(SyncError::Refused(_))) {
                        deleted = client.delete_event(&url, href, None).await;
                    }
                    match deleted {
                        Err(SyncError::Refused(why)) => {
                            eprintln!("tasks: the server kept a deleted task: {why}");
                        }
                        other => other?,
                    }
                }
                state.store.forget_task(&id)?;
            }
            ContactState::Changed => {
                let text = ical::write_todo(
                    &pending.uid,
                    &pending.task.fields,
                    pending.raw.as_deref(),
                    stamp(),
                );
                let put = client
                    .put_event(
                        &url,
                        pending.remote_id.as_deref(),
                        &pending.uid,
                        &text,
                        pending.etag.as_deref(),
                    )
                    .await;
                let (href, etag) = match put {
                    // Changed elsewhere since: the newest edit (this one) wins.
                    Err(SyncError::Refused(_)) if pending.remote_id.is_some() => {
                        client
                            .put_event(
                                &url,
                                pending.remote_id.as_deref(),
                                &pending.uid,
                                &text,
                                None,
                            )
                            .await?
                    }
                    other => other?,
                };
                state.store.task_pushed(
                    &id,
                    &href,
                    etag.as_deref(),
                    &text,
                    &pending.task.fields,
                )?;
            }
            ContactState::Synced => {}
        }
    }
    let listed = client.list_todos(&url).await?;
    let known = state.store.task_etags(&list.id)?;
    let wanted: Vec<String> = listed
        .iter()
        .filter(|(href, etag)| match known.get(href) {
            Some((known_etag, _)) => etag.is_none() || known_etag != etag,
            None => true,
        })
        .map(|(href, _)| href.clone())
        .collect();
    for remote in client.fetch_events(&url, &wanted).await? {
        if let Some((uid, fields)) = ical::parse_todo(&remote.vcard) {
            state.store.apply_server_task(
                &list.id,
                &ServerTask {
                    remote_id: &remote.href,
                    etag: remote.etag.as_deref(),
                    raw: &remote.vcard,
                    uid: &uid,
                    fields: &fields,
                },
                now(),
            )?;
        }
    }
    let on_server: Vec<String> = listed.into_iter().map(|(href, _)| href).collect();
    state
        .store
        .forget_tasks_not_on_server(&list.id, &on_server)?;
    Ok(())
}

/// Google Tasks keeps no reminder, importance or categories: those of the
/// task kept here stay.
fn with_local_fields(
    state: &AppState,
    list: &TaskList,
    remote_id: &str,
    mut fields: TaskFields,
) -> TaskFields {
    if let Ok(Some(local)) = state.store.task_by_remote_id(&list.id, remote_id) {
        fields.reminder = local.task.fields.reminder;
        fields.importance = local.task.fields.importance;
        fields.categories = local.task.fields.categories;
    }
    fields
}

async fn sync_google_list(state: &AppState, api: &RestClient, list: &TaskList) -> AppResult<()> {
    let Some(remote) = list.remote_id.as_deref() else {
        return Ok(());
    };
    for pending in state.store.tasks_to_push(&list.id)? {
        let id = pending.task.id.clone();
        match pending.task.state {
            ContactState::Deleted => {
                if let Some(task) = &pending.remote_id {
                    google::delete_task(api, remote, task).await?;
                }
                state.store.forget_task(&id)?;
            }
            ContactState::Changed => {
                let saved = match &pending.remote_id {
                    Some(task) => {
                        google::update_task(api, remote, task, &pending.task.fields).await?
                    }
                    None => google::create_task(api, remote, &pending.task.fields).await?,
                };
                state.store.task_pushed(
                    &id,
                    &saved.id,
                    saved.etag.as_deref(),
                    &saved.raw,
                    &pending.task.fields,
                )?;
            }
            ContactState::Synced => {}
        }
    }
    let tasks = google::list_tasks(api, remote).await?;
    let known = state.store.task_etags(&list.id)?;
    for task in &tasks {
        if known
            .get(&task.id)
            .is_some_and(|(etag, _)| etag.is_some() && *etag == task.etag)
        {
            continue;
        }
        let fields = with_local_fields(state, list, &task.id, task.fields.clone());
        state.store.apply_server_task(
            &list.id,
            &ServerTask {
                remote_id: &task.id,
                etag: task.etag.as_deref(),
                raw: &task.raw,
                uid: &task.id,
                fields: &fields,
            },
            now(),
        )?;
    }
    let on_server: Vec<String> = tasks.into_iter().map(|t| t.id).collect();
    state
        .store
        .forget_tasks_not_on_server(&list.id, &on_server)?;
    Ok(())
}

async fn sync_graph_list(state: &AppState, api: &RestClient, list: &TaskList) -> AppResult<()> {
    let Some(remote) = list.remote_id.as_deref() else {
        return Ok(());
    };
    for pending in state.store.tasks_to_push(&list.id)? {
        let id = pending.task.id.clone();
        match pending.task.state {
            ContactState::Deleted => {
                if let Some(task) = &pending.remote_id {
                    graph::delete_todo(api, remote, task).await?;
                }
                state.store.forget_task(&id)?;
            }
            ContactState::Changed => {
                let saved = match &pending.remote_id {
                    Some(task) => {
                        graph::update_todo(api, remote, task, &pending.task.fields).await?
                    }
                    None => graph::create_todo(api, remote, &pending.task.fields).await?,
                };
                state.store.task_pushed(
                    &id,
                    &saved.id,
                    saved.etag.as_deref(),
                    &saved.raw,
                    &pending.task.fields,
                )?;
            }
            ContactState::Synced => {}
        }
    }
    let tasks = graph::list_todos(api, remote).await?;
    let known = state.store.task_etags(&list.id)?;
    for task in &tasks {
        if known
            .get(&task.id)
            .is_some_and(|(etag, _)| etag.is_some() && *etag == task.etag)
        {
            continue;
        }
        state.store.apply_server_task(
            &list.id,
            &ServerTask {
                remote_id: &task.id,
                etag: task.etag.as_deref(),
                raw: &task.raw,
                uid: &task.id,
                fields: &task.fields,
            },
            now(),
        )?;
    }
    let on_server: Vec<String> = tasks.into_iter().map(|t| t.id).collect();
    state
        .store
        .forget_tasks_not_on_server(&list.id, &on_server)?;
    Ok(())
}
