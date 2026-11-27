//! Reminders: the due list (event alarms, task reminders and flagged
//! messages' follow-up dates that have come), with dismiss and snooze, and
//! a desktop notification for each as it comes due.
//!
//! Event reminders come from the event's own alarms (`VALARM`, as set
//! here, in webmail or by whoever sent the invitation). Every half minute
//! the app looks for reminders that went off since the last look. That
//! time is kept in the database, so a reminder that came due while the app
//! was closed still shows when it opens, as long as the event hasn't
//! ended; and nothing shows twice. Dismissed and snoozed reminders are
//! kept in the database too, so they stay that way after a restart.
//!
//! The due list's logic is `mail_core::reminders`; this gathers what may
//! be due and shows it.

use std::sync::Arc;
use std::time::Duration;

use chrono::{Local, NaiveTime, TimeZone, Utc};
use mail_core::ical;
use mail_core::reminders::{
    self as due_logic, Candidate, DueReminder, MAX_SNOOZE_MINS, ReminderKind, ReminderState,
};
use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use crate::error::{AppError, AppResult};
use crate::state::{AppState, lock, now};

const TICK: Duration = Duration::from_secs(30);
/// When reminders were last looked for (Unix seconds).
const CHECKED_KEY: &str = "reminders:checked";
/// Reminders can be up to four weeks ahead; events that far out are read.
const AHEAD: i64 = 29 * 86_400;
/// Reminders missed while the app was closed: at most a day back.
const MISSED: i64 = 86_400;
/// Flags whose date passed this long ago drop off the due list.
const FLAGS_BACK: i64 = 30 * 86_400;

/// A notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    /// The event's title.
    pub title: String,
    /// When and where.
    pub body: String,
}

/// Look for due reminders every half minute.
pub fn start(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(error) = tick(&app, &state) {
                eprintln!("reminders: {error}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

fn unix_now() -> i64 {
    i64::try_from(now()).unwrap_or(0)
}

/// Everything that may be due around `now`, with when it goes off.
fn candidates(state: &AppState, now: i64) -> AppResult<Vec<Candidate>> {
    let mut out = Vec::new();
    if crate::settings::load(state)?.event_reminders {
        let calendars = state.store.list_calendars()?;
        let accounts = state.store.list_accounts()?;
        let from = now.saturating_sub(MISSED);
        let to = now.saturating_add(AHEAD);
        // Reminders chosen here for events that can't be changed on the
        // server take the place of the event's own alarms.
        let overrides = state.store.reminder_overrides()?;
        for stored in state.store.calendar_events_between(from, to)? {
            let me = calendars
                .iter()
                .find(|c| c.id == stored.calendar_id)
                .and_then(|c| accounts.iter().find(|a| a.id == c.account_id))
                .map(|a| a.email.clone());
            for mut o in ical::occurrences(&stored.ics, from, to) {
                overrides.apply(&stored.id, &mut o);
                if o.reminders.is_empty() {
                    continue;
                }
                let declined = o.attendees.iter().any(|p| {
                    me.as_deref()
                        .is_some_and(|me| p.email.eq_ignore_ascii_case(me))
                        && p.status.as_deref() == Some("declined")
                });
                if declined {
                    continue;
                }
                let (start, end) = if o.all_day {
                    (local_midnight(o.start), local_midnight(o.end))
                } else {
                    (o.start, o.end)
                };
                out.push(Candidate {
                    reminder: DueReminder {
                        key: due_logic::event_key(&stored.id, o.start),
                        kind: ReminderKind::Event,
                        title: o.title.clone(),
                        at: start,
                        location: Some(o.location.trim().to_owned()).filter(|l| !l.is_empty()),
                        event_id: Some(stored.id.clone()),
                        recurrence_id: o.recurrence_id,
                        task_id: None,
                        message_id: None,
                    },
                    fires: o
                        .reminders
                        .iter()
                        .map(|m| start.saturating_sub(i64::from(*m).saturating_mul(60)))
                        .collect(),
                    until: Some(end.max(start).max(start.saturating_add(1))),
                });
            }
        }
    }
    for task in state.store.tasks_reminding_before(now)? {
        let Some(reminder) = task.fields.reminder else {
            continue;
        };
        out.push(Candidate {
            reminder: DueReminder {
                key: due_logic::task_key(&task.id, reminder),
                kind: ReminderKind::Task,
                title: task.fields.title.clone(),
                at: task.fields.due.unwrap_or(reminder),
                location: None,
                event_id: None,
                recurrence_id: None,
                task_id: Some(task.id.clone()),
                message_id: task.message_id.clone(),
            },
            fires: vec![reminder],
            until: None,
        });
    }
    for (id, subject, due) in state
        .store
        .flags_due_between(now.saturating_sub(FLAGS_BACK), now)?
    {
        out.push(Candidate {
            reminder: DueReminder {
                key: due_logic::flag_key(&id, due),
                kind: ReminderKind::Flag,
                title: follow_up(&subject).title,
                at: due,
                location: None,
                event_id: None,
                recurrence_id: None,
                task_id: None,
                message_id: Some(id),
            },
            fires: vec![due],
            until: None,
        });
    }
    Ok(out)
}

/// The due list now.
fn due_now(state: &AppState) -> AppResult<Vec<DueReminder>> {
    let now = unix_now();
    let states = state.store.reminder_states()?;
    Ok(due_logic::due_list(&candidates(state, now)?, &states, now))
}

/// Tell the interface the due list, if it changed since it was last told.
fn publish(app: &AppHandle, state: &AppState, due: &[DueReminder], force: bool) {
    let keys: Vec<String> = due.iter().map(|d| d.key.clone()).collect();
    {
        let mut last = lock(&state.due_reminders);
        if !force && *last == keys {
            return;
        }
        *last = keys;
    }
    if let Err(error) = app.emit("mail://reminders", json!({ "reminders": due })) {
        eprintln!("couldn't emit reminders: {error}");
    }
}

/// Work the due list out again now (after tasks or events changed).
pub fn refresh(app: &AppHandle, state: &AppState) {
    match due_now(state) {
        Ok(due) => publish(app, state, &due, false),
        Err(error) => eprintln!("reminders: {error}"),
    }
}

fn tick(app: &AppHandle, state: &AppState) -> AppResult<()> {
    let now = unix_now();
    let last = state
        .store
        .get_setting(CHECKED_KEY)?
        .and_then(|t| t.parse::<i64>().ok())
        .filter(|t| *t <= now)
        .map_or(now.saturating_sub(60), |t| {
            t.max(now.saturating_sub(MISSED))
        });
    state.store.set_setting(CHECKED_KEY, &now.to_string())?;
    let all = candidates(state, now)?;
    let states = state.store.reminder_states()?;
    for candidate in due_logic::to_notify(&all, &states, last, now) {
        let shown = notification(candidate, now);
        crate::events::notify(app, &shown.title, &shown.body);
    }
    publish(app, state, &due_logic::due_list(&all, &states, now), false);
    Ok(())
}

/// The notification for a reminder.
fn notification(candidate: &Candidate, now: i64) -> Due {
    let r = &candidate.reminder;
    match r.kind {
        ReminderKind::Event => describe(
            &r.title,
            r.location.as_deref().unwrap_or_default(),
            candidate
                .until
                .is_some_and(|u| u.saturating_sub(r.at) % 86_400 == 0)
                && Utc
                    .timestamp_opt(r.at, 0)
                    .single()
                    .is_some_and(|d| d.with_timezone(&Local).time() == NaiveTime::MIN),
            (r.at, candidate.until.unwrap_or(r.at)),
            now,
            local_time,
        ),
        ReminderKind::Task => Due {
            title: if r.title.trim().is_empty() {
                "(No title)".to_owned()
            } else {
                r.title.trim().to_owned()
            },
            body: "Task reminder".to_owned(),
        },
        ReminderKind::Flag => Due {
            title: r.title.clone(),
            body: "This message is flagged for follow-up now.".to_owned(),
        },
    }
}

/// `list_due_reminders`: what is due now, soonest first.
#[tauri::command]
pub fn list_due_reminders(state: State<'_, Arc<AppState>>) -> AppResult<Vec<DueReminder>> {
    due_now(&state)
}

/// `dismiss_reminder`: never list it again.
#[tauri::command]
pub fn dismiss_reminder(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    key: String,
) -> AppResult<Vec<DueReminder>> {
    if key.is_empty() || key.len() > 500 {
        return Err(AppError::invalid("That reminder is gone."));
    }
    state.store.set_reminder_state(
        &key,
        ReminderState {
            dismissed: true,
            snoozed_until: None,
        },
        unix_now(),
    )?;
    let due = due_now(&state)?;
    publish(&app, &state, &due, true);
    Ok(due)
}

/// `snooze_reminder`: list it again in `minutes`.
#[tauri::command]
pub fn snooze_reminder(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    key: String,
    minutes: u32,
) -> AppResult<Vec<DueReminder>> {
    if key.is_empty() || key.len() > 500 {
        return Err(AppError::invalid("That reminder is gone."));
    }
    if !(1..=MAX_SNOOZE_MINS).contains(&minutes) {
        return Err(AppError::invalid("Snooze for a minute to a week."));
    }
    let now = unix_now();
    state.store.set_reminder_state(
        &key,
        ReminderState {
            dismissed: false,
            snoozed_until: Some(now.saturating_add(i64::from(minutes).saturating_mul(60))),
        },
        now,
    )?;
    let due = due_now(&state)?;
    publish(&app, &state, &due, true);
    Ok(due)
}

/// The notification for a flagged message whose follow-up date came.
fn follow_up(subject: &str) -> Due {
    let subject = subject.trim();
    Due {
        title: format!(
            "Follow up: {}",
            if subject.is_empty() {
                "(no subject)"
            } else {
                subject
            }
        ),
        body: "This message is flagged for follow-up now.".to_owned(),
    }
}

/// Midnight on that day here, for an all-day event (stored as midnight UTC).
fn local_midnight(utc_midnight: i64) -> i64 {
    Utc.timestamp_opt(utc_midnight, 0)
        .single()
        .map(|d| d.date_naive().and_time(NaiveTime::MIN))
        .and_then(|d| Local.from_local_datetime(&d).earliest())
        .map_or(utc_midnight, |d| d.timestamp())
}

fn local_time(seconds: i64) -> String {
    Local
        .timestamp_opt(seconds, 0)
        .single()
        .map(|d| d.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// The notification for an event: its title, and when and where.
fn describe(
    title: &str,
    location: &str,
    all_day: bool,
    (start, end): (i64, i64),
    now: i64,
    time: impl Fn(i64) -> String,
) -> Due {
    let mins = start.saturating_sub(now).saturating_add(30) / 60;
    let lead = match mins {
        ..=0 => "Now".to_owned(),
        1 => "In 1 minute".to_owned(),
        2..=59 => format!("In {mins} minutes"),
        60..=89 => "In 1 hour".to_owned(),
        90..=1439 => format!("In {} hours", mins.saturating_add(30) / 60),
        1440..=2879 => "Tomorrow".to_owned(),
        _ => format!("In {} days", mins.saturating_add(720) / 1440),
    };
    let when = if all_day {
        "all day".to_owned()
    } else if end > start {
        format!("{}–{}", time(start), time(end))
    } else {
        time(start)
    };
    let mut body = format!("{lead} · {when}");
    if !location.trim().is_empty() {
        body.push_str(" · ");
        body.push_str(location.trim());
    }
    Due {
        title: if title.trim().is_empty() {
            "(No title)".to_owned()
        } else {
            title.trim().to_owned()
        },
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hhmm(s: i64) -> String {
        Utc.timestamp_opt(s, 0)
            .single()
            .map(|d| d.format("%H:%M").to_string())
            .unwrap_or_default()
    }

    #[test]
    fn follow_ups_name_the_message() {
        assert_eq!(
            follow_up(" Quote for Kilimani ").title,
            "Follow up: Quote for Kilimani"
        );
        assert_eq!(follow_up("").title, "Follow up: (no subject)");
    }

    #[test]
    fn notifications_say_when_and_where() {
        let start = 1_795_168_800; // 10:00 UTC
        let at = |all_day, end, now| {
            describe("Site visit", " Kilimani ", all_day, (start, end), now, hhmm)
        };
        let d = at(false, start + 1800, start - 15 * 60);
        assert_eq!(d.title, "Site visit");
        assert_eq!(d.body, "In 15 minutes · 10:00–10:30 · Kilimani");
        let plain = |all_day, end, now| describe("", "", all_day, (start, end), now, hhmm);
        assert_eq!(plain(false, start, start).body, "Now · 10:00");
        assert_eq!(plain(false, start, start).title, "(No title)");
        assert_eq!(
            plain(false, start + 60, start - 3600).body,
            "In 1 hour · 10:00–10:01"
        );
        assert_eq!(
            plain(false, start, start - 2 * 3600).body,
            "In 2 hours · 10:00"
        );
        assert_eq!(
            plain(true, start + 86_400, start - 86_400).body,
            "Tomorrow · all day"
        );
        assert_eq!(
            plain(true, start + 86_400, start - 7 * 86_400).body,
            "In 7 days · all day"
        );
    }
}
