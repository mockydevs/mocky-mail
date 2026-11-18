//! Event reminders: a notification before each calendar event, from the
//! event's own alarms (`VALARM`, as set here, in webmail or by whoever sent
//! the invitation).
//!
//! Every half minute the app looks for reminders that fell due since the
//! last look. That time is kept in the database, so a reminder that came
//! due while the app was closed still shows when it opens, as long as the
//! event hasn't ended; and nothing shows twice.

use std::sync::Arc;
use std::time::Duration;

use chrono::{Local, NaiveTime, TimeZone, Utc};
use mail_core::ical;
use tauri::AppHandle;

use crate::error::AppResult;
use crate::state::{AppState, now};

const TICK: Duration = Duration::from_secs(30);
/// When reminders were last looked for (Unix seconds).
const CHECKED_KEY: &str = "reminders:checked";
/// Reminders can be up to four weeks ahead; events that far out are read.
const AHEAD: i64 = 29 * 86_400;
/// Reminders missed while the app was closed: at most a day back.
const MISSED: i64 = 86_400;

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

fn tick(app: &AppHandle, state: &AppState) -> AppResult<()> {
    let now = i64::try_from(now()).unwrap_or(0);
    let last = state
        .store
        .get_setting(CHECKED_KEY)?
        .and_then(|t| t.parse::<i64>().ok())
        .filter(|t| *t <= now)
        .map_or(now.saturating_sub(60), |t| {
            t.max(now.saturating_sub(MISSED))
        });
    state.store.set_setting(CHECKED_KEY, &now.to_string())?;
    if !crate::settings::load(state)?.event_reminders {
        return Ok(());
    }
    for due in due(state, last, now)? {
        crate::events::notify(app, &due.title, &due.body);
    }
    Ok(())
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

/// Reminders that came due after `last`, up to `now`: one per occurrence.
fn due(state: &AppState, last: i64, now: i64) -> AppResult<Vec<Due>> {
    let calendars = state.store.list_calendars()?;
    let accounts = state.store.list_accounts()?;
    let from = now.saturating_sub(MISSED);
    let to = now.saturating_add(AHEAD);
    let mut out = Vec::new();
    for stored in state.store.calendar_events_between(from, to)? {
        let me = calendars
            .iter()
            .find(|c| c.id == stored.calendar_id)
            .and_then(|c| accounts.iter().find(|a| a.id == c.account_id))
            .map(|a| a.email.clone());
        for o in ical::occurrences(&stored.ics, from, to) {
            let declined = o.attendees.iter().any(|p| {
                me.as_deref()
                    .is_some_and(|me| p.email.eq_ignore_ascii_case(me))
                    && p.status.as_deref() == Some("declined")
            });
            let (start, end) = if o.all_day {
                (local_midnight(o.start), local_midnight(o.end))
            } else {
                (o.start, o.end)
            };
            if declined || end.max(start) <= now {
                continue;
            }
            let fired = o.reminders.iter().any(|m| {
                let at = start.saturating_sub(i64::from(*m).saturating_mul(60));
                at > last && at <= now
            });
            if fired {
                out.push(describe(
                    &o.title,
                    &o.location,
                    o.all_day,
                    (start, end),
                    now,
                    local_time,
                ));
            }
        }
    }
    Ok(out)
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
