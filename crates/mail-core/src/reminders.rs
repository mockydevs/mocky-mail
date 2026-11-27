//! The due-reminders list: event alarms, task reminders and flagged
//! messages' follow-up dates that have come, less those dismissed or
//! snoozed, and which of them to show a notification for.
//!
//! A reminder's key names the thing and its time (the event date, the
//! task's reminder, the flag's date), so moving it makes a new reminder
//! that comes due again, while dismissing or snoozing sticks to that one.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// What a reminder is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReminderKind {
    /// A calendar event.
    Event,
    /// A task.
    Task,
    /// A flagged message.
    Flag,
}

/// A reminder in the due list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DueReminder {
    /// Stable while it is due; for dismissing and snoozing.
    pub key: String,
    /// Event, task or flag.
    pub kind: ReminderKind,
    /// The event's title, the task, or the message's subject.
    pub title: String,
    /// When the event starts or the task or flag is due (Unix seconds).
    pub at: i64,
    /// Where (events).
    pub location: Option<String>,
    /// The stored event.
    pub event_id: Option<String>,
    /// The date of a series.
    pub recurrence_id: Option<i64>,
    /// The task.
    pub task_id: Option<String>,
    /// The message.
    pub message_id: Option<String>,
}

/// Something that may be due, with when its reminders go off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The reminder as listed once due.
    pub reminder: DueReminder,
    /// When it goes off (several alarms of one event: each).
    pub fires: Vec<i64>,
    /// Gone from the list once this time passes (an event that ended).
    pub until: Option<i64>,
}

/// What the user did with a reminder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderState {
    /// Dismissed: never listed again.
    pub dismissed: bool,
    /// Snoozed: listed again from this time (Unix seconds).
    pub snoozed_until: Option<i64>,
}

/// Longest snooze, minutes (a week).
pub const MAX_SNOOZE_MINS: u32 = 7 * 24 * 60;

/// The key of one date of an event.
#[must_use]
pub fn event_key(event_id: &str, start: i64) -> String {
    format!("event:{event_id}:{start}")
}

/// The key of a task's reminder.
#[must_use]
pub fn task_key(task_id: &str, reminder: i64) -> String {
    format!("task:{task_id}:{reminder}")
}

/// The key of a flagged message's follow-up date.
#[must_use]
pub fn flag_key(message_id: &str, due: i64) -> String {
    format!("flag:{message_id}:{due}")
}

fn live(candidate: &Candidate, now: i64) -> bool {
    candidate.until.is_none_or(|until| until > now)
}

/// The reminders due at `now`, soonest first.
#[must_use]
pub fn due_list<S: std::hash::BuildHasher>(
    candidates: &[Candidate],
    states: &HashMap<String, ReminderState, S>,
    now: i64,
) -> Vec<DueReminder> {
    let mut out: Vec<DueReminder> = candidates
        .iter()
        .filter(|c| live(c, now))
        .filter(|c| c.fires.iter().any(|at| *at <= now))
        .filter(|c| {
            states
                .get(&c.reminder.key)
                .is_none_or(|s| !s.dismissed && s.snoozed_until.is_none_or(|until| until <= now))
        })
        .map(|c| c.reminder.clone())
        .collect();
    out.sort_by(|a, b| (a.at, &a.key).cmp(&(b.at, &b.key)));
    out.dedup_by(|a, b| a.key == b.key);
    out
}

/// The reminders to show a notification for: those that went off, or
/// whose snooze ended, after `last` and up to `now`.
#[must_use]
pub fn to_notify<'a, S: std::hash::BuildHasher>(
    candidates: &'a [Candidate],
    states: &HashMap<String, ReminderState, S>,
    last: i64,
    now: i64,
) -> Vec<&'a Candidate> {
    let mut seen: Vec<&str> = Vec::new();
    candidates
        .iter()
        .filter(|c| live(c, now))
        .filter(|c| {
            let state = states.get(&c.reminder.key).copied().unwrap_or_default();
            if state.dismissed {
                return false;
            }
            match state.snoozed_until {
                Some(until) if until > now => false,
                Some(until) if until > last => true,
                _ => c.fires.iter().any(|at| *at > last && *at <= now),
            }
        })
        .filter(|c| {
            let fresh = !seen.contains(&c.reminder.key.as_str());
            seen.push(&c.reminder.key);
            fresh
        })
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;

    fn event(id: &str, start: i64, alarms: &[i64], end: i64) -> Candidate {
        Candidate {
            reminder: DueReminder {
                key: event_key(id, start),
                kind: ReminderKind::Event,
                title: id.into(),
                at: start,
                location: None,
                event_id: Some(id.into()),
                recurrence_id: None,
                task_id: None,
                message_id: None,
            },
            fires: alarms.iter().map(|m| start - m * 60).collect(),
            until: Some(end),
        }
    }

    #[test]
    fn reminders_come_due_and_go_when_the_event_ends() {
        let all = [
            event("a", 1000 * 60, &[15, 5], 1060 * 60),
            event("b", 2000 * 60, &[15], 2060 * 60),
        ];
        let none = HashMap::new();
        // 20 minutes before a: nothing.
        assert!(due_list(&all, &none, 980 * 60).is_empty());
        // 15 minutes before: a.
        let due = due_list(&all, &none, 985 * 60);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].key, "event:a:60000");
        // Still listed during the event, gone after.
        assert_eq!(due_list(&all, &none, 1030 * 60).len(), 1);
        assert!(due_list(&all, &none, 1060 * 60).is_empty());
    }

    #[test]
    fn each_alarm_notifies_once() {
        let all = [event("a", 1000 * 60, &[15, 5], 1060 * 60)];
        let none = HashMap::new();
        assert_eq!(to_notify(&all, &none, 980 * 60, 985 * 60).len(), 1);
        assert!(to_notify(&all, &none, 985 * 60, 990 * 60).is_empty());
        // The 5-minute alarm.
        assert_eq!(to_notify(&all, &none, 990 * 60, 995 * 60).len(), 1);
        // Both in one look (the app was closed): one notification.
        assert_eq!(to_notify(&all, &none, 900 * 60, 999 * 60).len(), 1);
    }

    #[test]
    fn dismissed_and_snoozed_reminders() {
        let all = [event("a", 1000 * 60, &[15, 5], 1060 * 60)];
        let key = all[0].reminder.key.clone();
        let mut states = HashMap::new();
        states.insert(
            key.clone(),
            ReminderState {
                dismissed: false,
                snoozed_until: Some(992 * 60),
            },
        );
        // Snoozed: off the list, and the 5-minute alarm doesn't notify
        // before the snooze ends.
        assert!(due_list(&all, &states, 990 * 60).is_empty());
        assert!(to_notify(&all, &states, 989 * 60, 990 * 60).is_empty());
        // The snooze ends: back, with a notification.
        assert_eq!(due_list(&all, &states, 992 * 60).len(), 1);
        assert_eq!(to_notify(&all, &states, 991 * 60, 992 * 60).len(), 1);
        assert!(to_notify(&all, &states, 992 * 60, 993 * 60).is_empty());
        // The later alarm goes off after the snooze: notifies again.
        assert_eq!(to_notify(&all, &states, 993 * 60, 995 * 60).len(), 1);
        states.insert(
            key,
            ReminderState {
                dismissed: true,
                snoozed_until: None,
            },
        );
        assert!(due_list(&all, &states, 996 * 60).is_empty());
        assert!(to_notify(&all, &states, 994 * 60, 996 * 60).is_empty());
    }

    #[test]
    fn tasks_and_flags_stay_due_until_done() {
        let task = Candidate {
            reminder: DueReminder {
                key: task_key("t", 500),
                kind: ReminderKind::Task,
                title: "Send quote".into(),
                at: 900,
                location: None,
                event_id: None,
                recurrence_id: None,
                task_id: Some("t".into()),
                message_id: None,
            },
            fires: vec![500],
            until: None,
        };
        let flag = Candidate {
            reminder: DueReminder {
                key: flag_key("m", 400),
                kind: ReminderKind::Flag,
                title: "Follow up: Quote".into(),
                at: 400,
                task_id: None,
                message_id: Some("m".into()),
                ..task.reminder.clone()
            },
            fires: vec![400],
            until: None,
        };
        let all = [task, flag];
        let due = due_list(&all, &HashMap::new(), 1_000_000);
        assert_eq!(due.len(), 2);
        // Soonest first.
        assert_eq!(due[0].kind, ReminderKind::Flag);
        assert_eq!(task_key("t", 500), "task:t:500");
    }
}
