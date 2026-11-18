//! The calendar: commands for the Calendar screen, and syncing each
//! password account's calendars with its server over `CalDAV` (for example
//! `SOGo` on the account's mail server).
//!
//! The device keeps a copy of every event, so the calendar shows without
//! a connection. Adding, editing and deleting go to the server straight
//! away (and need a connection), so nothing is ever half-saved. Gmail and
//! Microsoft accounts would need the provider's calendar API, so they have
//! no calendar here yet.

use std::sync::Arc;
use std::time::Duration;

use mail_core::ical::{self, EventInput, Occurrence, Repeat};
use mail_store::{AuthKind, Calendar, FoundCalendar};
use mail_sync::dav::{DavClient, DavLogin};
use mail_sync::{Credentials, SyncError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, State, Url};

use crate::error::{AppError, AppResult};
use crate::state::{AppState, now};

/// How often calendars are synced in the background.
const SYNC_EVERY: Duration = Duration::from_mins(15);
/// Longest stretch one `list_events` covers: a little over a year.
const MAX_WINDOW: i64 = 400 * 86_400;
const MAX_TITLE: usize = 500;
const MAX_TEXT: usize = 8000;

/// Whether an account's calendars sync with its server.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarStatus {
    account_id: String,
    /// `synced`, `unavailable` (no calendar on the server, or not supported
    /// yet), or `error`.
    state: &'static str,
    /// Plain words for the user.
    message: Option<String>,
}

/// One occurrence of an event, with where it lives.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    /// The stored event (a single event or a whole series).
    id: String,
    calendar_id: String,
    account_id: String,
    /// The calendar's colour, if the server gave one.
    color: Option<String>,
    #[serde(flatten)]
    occurrence: Occurrence,
}

/// An event from the editor.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSave {
    /// Missing for a new event.
    id: Option<String>,
    /// The calendar to keep it in.
    calendar_id: String,
    #[serde(flatten)]
    event: EventInput,
}

fn tidy(text: &str, max: usize, lines: bool) -> String {
    text.chars()
        .filter(|c| !c.is_control() || (lines && *c == '\n'))
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Check and tidy what the editor sent.
fn checked(mut input: EventInput) -> AppResult<EventInput> {
    // 1970 to 2200: anything else is a mistake.
    const LATEST: i64 = 7_258_118_400;
    input.title = tidy(&input.title, MAX_TITLE, false);
    input.location = tidy(&input.location, MAX_TITLE, false);
    input.notes = tidy(&input.notes, MAX_TEXT, true);
    input.time_zone = tidy(&input.time_zone, 64, false);
    if !(0..LATEST).contains(&input.start) || !(0..LATEST).contains(&input.end) {
        return Err(AppError::invalid("Choose a date for the event."));
    }
    if input.end < input.start {
        return Err(AppError::invalid("The event ends before it starts."));
    }
    if input.all_day && (input.start % 86_400 != 0 || input.end % 86_400 != 0) {
        return Err(AppError::invalid(
            "All-day events start and end on whole days.",
        ));
    }
    if input.repeat == Repeat::Custom {
        input.repeat = Repeat::Keep;
    }
    Ok(input)
}

fn with_place(calendar: &Calendar, id: String, occurrence: Occurrence) -> CalendarEvent {
    CalendarEvent {
        id,
        calendar_id: calendar.id.clone(),
        account_id: calendar.account_id.clone(),
        color: calendar.color.clone(),
        occurrence,
    }
}

/// `list_calendars`: every account's calendars.
#[tauri::command]
pub fn list_calendars(state: State<'_, Arc<AppState>>) -> AppResult<Vec<Calendar>> {
    Ok(state.store.list_calendars()?)
}

/// `list_events`: every occurrence between `from` and `to` (Unix seconds).
#[tauri::command]
pub fn list_events(
    state: State<'_, Arc<AppState>>,
    from: i64,
    to: i64,
) -> AppResult<Vec<CalendarEvent>> {
    if to <= from || to.saturating_sub(from) > MAX_WINDOW {
        return Err(AppError::invalid("That date range is too long."));
    }
    let calendars = state.store.list_calendars()?;
    let mut out = Vec::new();
    for stored in state.store.calendar_events_between(from, to)? {
        let Some(calendar) = calendars.iter().find(|c| c.id == stored.calendar_id) else {
            continue;
        };
        for occurrence in ical::occurrences(&stored.ics, from, to) {
            out.push(with_place(calendar, stored.id.clone(), occurrence));
        }
    }
    out.sort_by(|a, b| {
        (b.occurrence.all_day, a.occurrence.start, a.occurrence.end).cmp(&(
            a.occurrence.all_day,
            b.occurrence.start,
            b.occurrence.end,
        ))
    });
    Ok(out)
}

/// `get_event`: the event itself (a series' first date and its rule), for
/// the editor.
#[tauri::command]
pub fn get_event(state: State<'_, Arc<AppState>>, id: String) -> AppResult<CalendarEvent> {
    let stored = state.store.get_calendar_event(&id)?;
    let calendar = state.store.get_calendar(&stored.calendar_id)?;
    let occurrence = ical::master(&stored.ics)
        .ok_or_else(|| AppError::invalid("This calendar item isn't an event."))?;
    Ok(with_place(&calendar, stored.id, occurrence))
}

/// A connection to the calendar server of a calendar's account.
async fn client_for(state: &AppState, calendar: &Calendar) -> AppResult<(DavClient, Url)> {
    let account = state
        .store
        .list_accounts()?
        .into_iter()
        .find(|a| a.id == calendar.account_id)
        .ok_or_else(|| AppError::invalid("That account was removed."))?;
    let Credentials::Password { username, password } =
        crate::credentials::for_account(state, &account).await?
    else {
        return Err(AppError::invalid(
            "This account's calendar can't be changed here.",
        ));
    };
    let url = Url::parse(&calendar.url).map_err(|e| AppError::internal(e.to_string()))?;
    Ok((DavClient::new(DavLogin { username, password })?, url))
}

fn changed_elsewhere(error: SyncError) -> AppError {
    match error {
        SyncError::Refused(_) => AppError::invalid(
            "The calendar server didn't accept the change. The event may have been changed elsewhere; it has been refreshed, so try again.",
        ),
        other => other.into(),
    }
}

/// `save_event`: create or change an event on the server, then here.
#[tauri::command]
pub async fn save_event(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    event: EventSave,
) -> AppResult<CalendarEvent> {
    let input = checked(event.event)?;
    let calendar = state.store.get_calendar(&event.calendar_id)?;
    let (client, url) = client_for(&state, &calendar).await?;
    let stamp = i64::try_from(now()).unwrap_or(0);
    let existing = match &event.id {
        Some(id) => Some(state.store.get_calendar_event(id)?),
        None => None,
    };
    let result = match existing {
        // Edited in place.
        Some(stored) if stored.calendar_id == calendar.id => {
            let ics = ical::edit_event(&stored.ics, &input, stamp)
                .ok_or_else(|| AppError::invalid("This calendar item isn't an event."))?;
            let uid = ical::uid_of(&ics).unwrap_or_default();
            client
                .put_event(&url, Some(&stored.href), &uid, &ics, stored.etag.as_deref())
                .await
                .map(|(href, etag)| (href, etag, ics))
        }
        // Moved to another calendar: saved there, then removed here.
        Some(stored) => {
            let ics = ical::edit_event(&stored.ics, &input, stamp)
                .ok_or_else(|| AppError::invalid("This calendar item isn't an event."))?;
            let uid = ical::uid_of(&ics).unwrap_or_default();
            let saved = client.put_event(&url, None, &uid, &ics, None).await;
            if saved.is_ok() {
                let old = state.store.get_calendar(&stored.calendar_id)?;
                let (old_client, old_url) = client_for(&state, &old).await?;
                old_client
                    .delete_event(&old_url, &stored.href, stored.etag.as_deref())
                    .await?;
                state.store.delete_calendar_event(&stored.id)?;
            }
            saved.map(|(href, etag)| (href, etag, ics))
        }
        None => {
            let uid = mail_store::new_event_uid()?;
            let ics = ical::new_event(&uid, &input, stamp);
            client
                .put_event(&url, None, &uid, &ics, None)
                .await
                .map(|(href, etag)| (href, etag, ics))
        }
    };
    let (href, etag, ics) = match result {
        Ok(saved) => saved,
        Err(error) => {
            soon(&app, &state);
            return Err(changed_elsewhere(error));
        }
    };
    let id = state
        .store
        .put_calendar_event(&calendar.id, &href, etag.as_deref(), &ics)?;
    changed(&app, &[]);
    let occurrence =
        ical::master(&ics).ok_or_else(|| AppError::internal("saved event unreadable"))?;
    Ok(with_place(&calendar, id, occurrence))
}

/// `delete_event`: delete an event (or a whole series), or with
/// `occurrence` just that date of a series.
#[tauri::command]
pub async fn delete_event(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    occurrence: Option<i64>,
) -> AppResult<()> {
    let stored = state.store.get_calendar_event(&id)?;
    let calendar = state.store.get_calendar(&stored.calendar_id)?;
    let (client, url) = client_for(&state, &calendar).await?;
    let one_date = occurrence.and_then(|rid| ical::skip_occurrence(&stored.ics, rid));
    let result = match one_date {
        Some(ics) => {
            let uid = ical::uid_of(&ics).unwrap_or_default();
            client
                .put_event(&url, Some(&stored.href), &uid, &ics, stored.etag.as_deref())
                .await
                .and_then(|(href, etag)| {
                    state
                        .store
                        .put_calendar_event(&calendar.id, &href, etag.as_deref(), &ics)
                        .map(|_| ())
                        .map_err(|e| SyncError::Protocol(e.to_string()))
                })
        }
        None => client
            .delete_event(&url, &stored.href, stored.etag.as_deref())
            .await
            .and_then(|()| {
                state
                    .store
                    .delete_calendar_event(&stored.id)
                    .map_err(|e| SyncError::Protocol(e.to_string()))
            }),
    };
    if let Err(error) = result {
        soon(&app, &state);
        return Err(changed_elsewhere(error));
    }
    changed(&app, &[]);
    Ok(())
}

/// `sync_calendars`: sync every account's calendars now.
#[tauri::command]
pub async fn sync_calendars(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> AppResult<Vec<CalendarStatus>> {
    Ok(sync_all(&app, &state).await)
}

fn changed(app: &AppHandle, statuses: &[CalendarStatus]) {
    if let Err(error) = app.emit("mail://calendar-changed", json!({ "calendars": statuses })) {
        eprintln!("couldn't emit calendar-changed: {error}");
    }
}

/// Sync shortly, after a change was refused.
fn soon(app: &AppHandle, state: &Arc<AppState>) {
    let (app, state) = (app.clone(), Arc::clone(state));
    tauri::async_runtime::spawn(async move {
        sync_all(&app, &state).await;
    });
}

/// Background sync: once at start-up, then every 15 minutes.
pub fn start(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        loop {
            sync_all(&app, &state).await;
            tokio::time::sleep(SYNC_EVERY).await;
        }
    });
}

async fn sync_all(app: &AppHandle, state: &Arc<AppState>) -> Vec<CalendarStatus> {
    let _guard = state.calendar_sync.lock().await;
    let accounts = match state.store.list_accounts() {
        Ok(accounts) => accounts,
        Err(error) => {
            eprintln!("calendar: couldn't list accounts: {error}");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for account in accounts {
        let (status, message) = match sync_account(state, &account).await {
            Ok(None) => ("synced", None),
            Ok(Some(reason)) => ("unavailable", Some(reason)),
            Err(error) => {
                eprintln!("calendar: {}: {error}", account.email);
                ("error", Some(error.message))
            }
        };
        out.push(CalendarStatus {
            account_id: account.id.clone(),
            state: status,
            message,
        });
    }
    changed(app, &out);
    out
}

/// Sync one account. `Ok(Some(reason))` when it has no calendar here.
async fn sync_account(
    state: &AppState,
    account: &mail_store::Account,
) -> AppResult<Option<String>> {
    if account.auth != AuthKind::Password {
        return Ok(Some(
            "Calendars for Gmail and Microsoft accounts aren't available here yet.".to_owned(),
        ));
    }
    let Credentials::Password { username, password } =
        crate::credentials::for_account(state, account).await?
    else {
        return Ok(Some(
            "This account has no password to sync its calendar with.".to_owned(),
        ));
    };
    let client = DavClient::new(DavLogin { username, password })?;
    let server = Url::parse(&format!("https://{}/", account.imap.host))
        .map_err(|e| AppError::internal(e.to_string()))?;
    let found = match client.discover_calendars(&server).await {
        Ok(found) => found,
        // No CalDAV service at all (404, a web page, no principal).
        Err(SyncError::Config(_) | SyncError::Protocol(_) | SyncError::Refused(_)) => {
            return Ok(Some("The mail server doesn't offer a calendar.".to_owned()));
        }
        Err(other) => return Err(other.into()),
    };
    if found.is_empty() {
        return Ok(Some("The mail server doesn't offer a calendar.".to_owned()));
    }
    let calendars = state.store.set_calendars(
        &account.id,
        &found
            .iter()
            .map(|c| FoundCalendar {
                url: c.url.to_string(),
                name: c.name.clone().unwrap_or_else(|| "Calendar".to_owned()),
                color: c.color.clone(),
            })
            .collect::<Vec<_>>(),
    )?;
    for calendar in calendars {
        let url = Url::parse(&calendar.url).map_err(|e| AppError::internal(e.to_string()))?;
        let listed = client.list_resources(&url).await?;
        let known = state.store.calendar_etags(&calendar.id)?;
        let wanted: Vec<String> = listed
            .iter()
            .filter(|(href, etag)| match known.get(href) {
                Some(known_etag) => etag.is_none() || known_etag != etag,
                None => true,
            })
            .map(|(href, _)| href.clone())
            .collect();
        for remote in client.fetch_events(&url, &wanted).await? {
            state.store.put_calendar_event(
                &calendar.id,
                &remote.href,
                remote.etag.as_deref(),
                &remote.vcard,
            )?;
        }
        let on_server: Vec<String> = listed.into_iter().map(|(href, _)| href).collect();
        state
            .store
            .forget_calendar_events_not_on_server(&calendar.id, &on_server)?;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> EventInput {
        EventInput {
            title: "  Review\u{0} ".into(),
            location: String::new(),
            notes: "a\nb".into(),
            all_day: false,
            start: 1_795_000_000,
            end: 1_795_003_600,
            repeat: Repeat::Custom,
            time_zone: "Africa/Nairobi".into(),
        }
    }

    #[test]
    fn editor_input_is_tidied_and_checked() {
        let ok = checked(input()).unwrap_or_else(|_| input());
        assert_eq!(ok.title, "Review");
        assert_eq!(ok.notes, "a\nb");
        // A rule the editor can't show is kept as it is.
        assert_eq!(ok.repeat, Repeat::Keep);
        assert!(
            checked(EventInput {
                end: 1_794_999_000,
                ..input()
            })
            .is_err()
        );
        assert!(
            checked(EventInput {
                start: -5,
                ..input()
            })
            .is_err()
        );
        assert!(
            checked(EventInput {
                all_day: true,
                ..input()
            })
            .is_err()
        );
        assert!(
            checked(EventInput {
                all_day: true,
                start: 1_795_046_400,
                end: 1_795_132_800,
                ..input()
            })
            .is_ok()
        );
    }
}
