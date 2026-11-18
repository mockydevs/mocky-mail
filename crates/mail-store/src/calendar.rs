//! Calendars: each account's calendars on its server (`CalDAV`), and a
//! copy of their events so the calendar shows without a connection.
//!
//! Events are kept as the server's iCalendar text; occurrences are worked
//! out when shown (`mail_core::ical`). `starts_at` and `ends_at` let a
//! month be read without opening every event; `ends_at` is `NULL` for a
//! series that repeats forever.

use std::collections::HashMap;

use rusqlite::{OptionalExtension, Row, params};
use serde::Serialize;

use crate::Store;
use crate::error::{Result, StoreError};
use crate::ids::new_id;

/// A calendar on an account's server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Calendar {
    /// Local id.
    pub id: String,
    /// The account it belongs to.
    pub account_id: String,
    /// Its address on the server.
    #[serde(skip)]
    pub url: String,
    /// Its name.
    pub name: String,
    /// Its colour from the server (`#RRGGBB` or `#RRGGBBAA`), if any.
    pub color: Option<String>,
}

/// A calendar found on the server: address, name, colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundCalendar {
    /// Its address.
    pub url: String,
    /// Its name.
    pub name: String,
    /// Its colour.
    pub color: Option<String>,
}

/// One calendar item (an event or series) as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    /// Local id.
    pub id: String,
    /// The calendar it is in.
    pub calendar_id: String,
    /// Its path on the server.
    pub href: String,
    /// The server's version tag.
    pub etag: Option<String>,
    /// The iCalendar text.
    pub ics: String,
}

fn calendar_row(row: &Row<'_>) -> rusqlite::Result<Calendar> {
    Ok(Calendar {
        id: row.get(0)?,
        account_id: row.get(1)?,
        url: row.get(2)?,
        name: row.get(3)?,
        color: row.get(4)?,
    })
}

fn event_row(row: &Row<'_>) -> rusqlite::Result<StoredEvent> {
    Ok(StoredEvent {
        id: row.get(0)?,
        calendar_id: row.get(1)?,
        href: row.get(2)?,
        etag: row.get(3)?,
        ics: row.get(4)?,
    })
}

const CALENDAR_COLUMNS: &str = "id, account_id, url, name, color";
const EVENT_COLUMNS: &str = "id, calendar_id, href, etag, ics";

/// A new event's `UID`: random, and unique everywhere.
///
/// # Errors
/// When the system has no randomness to give.
pub fn new_event_uid() -> Result<String> {
    Ok(format!("{}@mocky.mail", new_id()?))
}

impl Store {
    /// Record an account's calendars as found on its server. Calendars no
    /// longer there are dropped with their events. Returns them all.
    ///
    /// # Errors
    /// Database errors.
    pub fn set_calendars(
        &self,
        account_id: &str,
        found: &[FoundCalendar],
    ) -> Result<Vec<Calendar>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for calendar in found {
            tx.execute(
                "INSERT INTO calendars (id, account_id, url, name, color) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (account_id, url) DO UPDATE SET name = excluded.name, color = excluded.color",
                params![new_id()?, account_id, calendar.url, calendar.name, calendar.color],
            )?;
        }
        let keep: Vec<&str> = found.iter().map(|c| c.url.as_str()).collect();
        let existing: Vec<(String, String)> = {
            let mut stmt = tx.prepare("SELECT id, url FROM calendars WHERE account_id = ?1")?;
            let rows = stmt.query_map(params![account_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for (id, url) in existing {
            if !keep.contains(&url.as_str()) {
                tx.execute("DELETE FROM calendars WHERE id = ?1", params![id])?;
            }
        }
        tx.commit()?;
        drop(conn);
        Ok(self
            .list_calendars()?
            .into_iter()
            .filter(|c| c.account_id == account_id)
            .collect())
    }

    /// Every calendar, by account and name.
    ///
    /// # Errors
    /// Database errors.
    pub fn list_calendars(&self) -> Result<Vec<Calendar>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {CALENDAR_COLUMNS} FROM calendars ORDER BY account_id, name COLLATE NOCASE"
        ))?;
        let rows = stmt.query_map([], calendar_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One calendar.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] when there is none.
    pub fn get_calendar(&self, id: &str) -> Result<Calendar> {
        self.conn()
            .query_row(
                &format!("SELECT {CALENDAR_COLUMNS} FROM calendars WHERE id = ?1"),
                params![id],
                calendar_row,
            )
            .optional()?
            .ok_or(StoreError::NotFound("calendar"))
    }

    /// The events known in a calendar: path to version tag.
    ///
    /// # Errors
    /// Database errors.
    pub fn calendar_etags(&self, calendar_id: &str) -> Result<HashMap<String, Option<String>>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT href, etag FROM calendar_events WHERE calendar_id = ?1")?;
        let rows = stmt.query_map(params![calendar_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Store an event as the server has it, replacing an older copy.
    /// Returns its local id (kept across updates).
    ///
    /// # Errors
    /// Database errors.
    pub fn put_calendar_event(
        &self,
        calendar_id: &str,
        href: &str,
        etag: Option<&str>,
        ics: &str,
    ) -> Result<String> {
        // Items with no event (tasks) are kept, so they aren't fetched
        // again, but never come up for a date.
        let (starts_at, ends_at) = if mail_core::ical::master(ics).is_some() {
            mail_core::ical::span(ics)
        } else {
            (Some(0), Some(0))
        };
        Ok(self.conn().query_row(
            "INSERT INTO calendar_events (id, calendar_id, href, etag, ics, starts_at, ends_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (calendar_id, href) DO UPDATE SET etag = excluded.etag,
                 ics = excluded.ics, starts_at = excluded.starts_at, ends_at = excluded.ends_at
             RETURNING id",
            params![new_id()?, calendar_id, href, etag, ics, starts_at, ends_at],
            |r| r.get(0),
        )?)
    }

    /// Drop events whose server copy is gone (deleted elsewhere).
    ///
    /// # Errors
    /// Database errors.
    pub fn forget_calendar_events_not_on_server(
        &self,
        calendar_id: &str,
        on_server: &[String],
    ) -> Result<usize> {
        let mut removed = 0usize;
        for href in self.calendar_etags(calendar_id)?.into_keys() {
            if !on_server.contains(&href) {
                removed = removed.saturating_add(self.conn().execute(
                    "DELETE FROM calendar_events WHERE calendar_id = ?1 AND href = ?2",
                    params![calendar_id, href],
                )?);
            }
        }
        Ok(removed)
    }

    /// One event.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] when there is none.
    pub fn get_calendar_event(&self, id: &str) -> Result<StoredEvent> {
        self.conn()
            .query_row(
                &format!("SELECT {EVENT_COLUMNS} FROM calendar_events WHERE id = ?1"),
                params![id],
                event_row,
            )
            .optional()?
            .ok_or(StoreError::NotFound("event"))
    }

    /// Remove an event here (after it was deleted on the server).
    ///
    /// # Errors
    /// Database errors.
    pub fn delete_calendar_event(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM calendar_events WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Events that may have occurrences between `from` and `to` (Unix
    /// seconds): the caller works out the exact dates.
    ///
    /// # Errors
    /// Database errors.
    pub fn calendar_events_between(&self, from: i64, to: i64) -> Result<Vec<StoredEvent>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM calendar_events
             WHERE (starts_at IS NULL OR starts_at < ?2) AND (ends_at IS NULL OR ends_at >= ?1)"
        ))?;
        let rows = stmt.query_map(params![from, to], event_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::test_support;

    fn found(url: &str, name: &str) -> FoundCalendar {
        FoundCalendar {
            url: url.into(),
            name: name.into(),
            color: Some("#3A87ADFF".into()),
        }
    }

    const ONE: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nDTSTART:20261118T060000Z\r\nDTEND:20261118T070000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    const DAILY: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:b\r\nDTSTART:20260101T060000Z\r\nRRULE:FREQ=DAILY\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn calendars_follow_the_server() {
        let store = Store::open_in_memory().unwrap();
        let account = test_support::add_account(&store, "me@work.example");
        let first = store
            .set_calendars(
                &account,
                &[
                    found("https://x/cal/personal/", "Personal"),
                    found("https://x/cal/team/", "Team"),
                ],
            )
            .unwrap();
        assert_eq!(first.len(), 2);
        let personal = first
            .iter()
            .find(|c| c.name == "Personal")
            .unwrap()
            .id
            .clone();
        store
            .put_calendar_event(&personal, "/cal/personal/a.ics", Some("\"1\""), ONE)
            .unwrap();
        // Renamed on the server: same id, events kept. Team removed with its events.
        let next = store
            .set_calendars(&account, &[found("https://x/cal/personal/", "Mine")])
            .unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].id, personal);
        assert_eq!(next[0].name, "Mine");
        assert_eq!(store.calendar_etags(&personal).unwrap().len(), 1);
        assert!(store.get_calendar(&personal).is_ok());
    }

    #[test]
    fn events_are_found_by_date_and_updated_in_place() {
        let store = Store::open_in_memory().unwrap();
        let account = test_support::add_account(&store, "me@work.example");
        let cal = store
            .set_calendars(&account, &[found("https://x/c/", "Personal")])
            .unwrap()[0]
            .id
            .clone();
        let id = store
            .put_calendar_event(&cal, "/c/a.ics", Some("\"1\""), ONE)
            .unwrap();
        store
            .put_calendar_event(&cal, "/c/b.ics", None, DAILY)
            .unwrap();
        let nov = (1_793_491_200, 1_796_083_200); // 2026-11-01 .. 2026-12-01
        assert_eq!(
            store.calendar_events_between(nov.0, nov.1).unwrap().len(),
            2
        );
        // 2030: only the series that never ends.
        let later = store
            .calendar_events_between(1_893_456_000, 1_896_134_400)
            .unwrap();
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].href, "/c/b.ics");

        let again = store
            .put_calendar_event(&cal, "/c/a.ics", Some("\"2\""), ONE)
            .unwrap();
        assert_eq!(again, id);
        assert_eq!(
            store.get_calendar_event(&id).unwrap().etag.as_deref(),
            Some("\"2\"")
        );

        store
            .forget_calendar_events_not_on_server(&cal, &["/c/b.ics".to_owned()])
            .unwrap();
        assert!(store.get_calendar_event(&id).is_err());
        let left = store.calendar_etags(&cal).unwrap();
        assert_eq!(left.len(), 1);
        let only = store.calendar_events_between(0, i64::MAX).unwrap();
        store.delete_calendar_event(&only[0].id).unwrap();
        assert!(
            store
                .calendar_events_between(0, i64::MAX)
                .unwrap()
                .is_empty()
        );
    }
}
