//! Between Exchange's shapes and the app's: times, list entries as header
//! blocks, events from the editor, repeat rules, automatic replies,
//! contacts and free times.

use std::fmt::Write as _;

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use mail_core::ical::{self, BusyPeriod, EventInput, Freq, RepeatRule, ShowAs};
use mail_core::vacation::{ExternalAudience, Vacation};
use mail_core::vcard::ContactCard;

use super::parse::{Busy, Contact, Headers};
use super::soap::{ContactFields, EventFields, Oof};
use super::xml::escape;
use crate::SyncError;
use crate::graph::{day_of, day_start, html_to_text, text_to_html};

/// An EWS `DateTime` as Unix seconds: with `Z`, an offset, or none (UTC,
/// the zone every request here asks in).
#[must_use]
pub fn parse_time(text: &str) -> Option<i64> {
    let text = text.trim();
    if let Ok(at) = DateTime::parse_from_rfc3339(text) {
        return Some(at.timestamp());
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(at) = NaiveDateTime::parse_from_str(text, format) {
            return Some(at.and_utc().timestamp());
        }
    }
    None
}

/// Unix seconds as an EWS `DateTime` in UTC.
#[must_use]
pub fn utc(seconds: i64) -> String {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// Unix seconds as an EWS `DateTime` without a zone (UTC).
#[must_use]
pub fn utc_plain(seconds: i64) -> String {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string()
}

/// A header value with nothing that could end the line early.
fn clean(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_owned()
}

fn mailbox(name: &str, address: &str) -> String {
    let name = clean(name);
    let address = clean(address);
    if name.is_empty() || name.eq_ignore_ascii_case(&address) {
        address
    } else {
        format!(
            "\"{}\" <{address}>",
            name.replace('\\', "\\\\").replace('"', "\\\"")
        )
    }
}

fn mailbox_list(people: &[(String, String)]) -> String {
    people
        .iter()
        .filter(|(_, address)| !address.is_empty())
        .map(|(name, address)| mailbox(name, address))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A message's header block for the list: the one it arrived with, or one
/// written from Exchange's properties (mail sent or saved in the mailbox
/// has none of its own).
#[must_use]
pub fn header_block(headers: &Headers) -> String {
    if let Some(block) = &headers.transport_headers
        && block.contains(':')
    {
        let mut block = block.replace("\r\n", "\n").replace('\n', "\r\n");
        if !block.ends_with("\r\n") {
            block.push_str("\r\n");
        }
        block.push_str("\r\n");
        return block;
    }
    let mut out = String::new();
    if let Some((name, address)) = &headers.from {
        let _ = write!(out, "From: {}\r\n", mailbox(name, address));
    }
    if !headers.to.is_empty() {
        let _ = write!(out, "To: {}\r\n", mailbox_list(&headers.to));
    }
    if !headers.cc.is_empty() {
        let _ = write!(out, "Cc: {}\r\n", mailbox_list(&headers.cc));
    }
    let _ = write!(out, "Subject: {}\r\n", clean(&headers.subject));
    if let Some(date) = headers.date.as_deref().and_then(parse_time) {
        let at = Utc.timestamp_opt(date, 0).single().unwrap_or_default();
        let _ = write!(out, "Date: {}\r\n", at.to_rfc2822());
    }
    for (name, value) in [
        ("Message-ID", &headers.message_id),
        ("In-Reply-To", &headers.in_reply_to),
        ("References", &headers.references),
    ] {
        if let Some(value) = value.as_deref().map(clean).filter(|v| !v.is_empty()) {
            let _ = write!(out, "{name}: {value}\r\n");
        }
    }
    match headers.importance.as_deref() {
        Some("High") => out.push_str("Importance: high\r\n"),
        Some("Low") => out.push_str("Importance: low\r\n"),
        _ => {}
    }
    out.push_str("MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n");
    out
}

/// The text of an iCalendar from an item's MIME content: the content
/// itself, or the calendar part of a message.
#[must_use]
pub fn calendar_text(raw: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    if text.trim_start().starts_with("BEGIN:VCALENDAR") {
        return Some(text.into_owned());
    }
    mail_core::message::parse_body(raw)
        .ok()
        .and_then(|body| body.calendar)
        .filter(|c| c.contains("BEGIN:VCALENDAR"))
}

const DAYS: [(&str, &str); 7] = [
    ("Monday", "MO"),
    ("Tuesday", "TU"),
    ("Wednesday", "WE"),
    ("Thursday", "TH"),
    ("Friday", "FR"),
    ("Saturday", "SA"),
    ("Sunday", "SU"),
];
const INDEXES: [(&str, i32); 5] = [
    ("First", 1),
    ("Second", 2),
    ("Third", 3),
    ("Fourth", 4),
    ("Last", -1),
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// The `<t:Recurrence>` element for a repeat rule, for a series starting
/// on `start_day` (`YYYY-MM-DD`).
///
/// # Errors
/// A rule Exchange can't hold (the last day of each month).
pub fn recurrence(rule: &RepeatRule, start_day: &str) -> Result<String, SyncError> {
    let start = NaiveDate::parse_from_str(start_day, "%Y-%m-%d")
        .map_err(|e| SyncError::Invalid(e.to_string()))?;
    let weekday = DAYS
        .get(usize::try_from(chrono::Datelike::weekday(&start).num_days_from_monday()).unwrap_or(0))
        .map_or("Monday", |(name, _)| *name);
    let days: Vec<&str> = if rule.by_day.is_empty() {
        vec![weekday]
    } else {
        rule.by_day
            .iter()
            .filter_map(|code| {
                DAYS.iter()
                    .find(|(_, c)| c.eq_ignore_ascii_case(code.trim()))
                    .map(|(name, _)| *name)
            })
            .collect()
    };
    let days = days.join(" ");
    if rule.month_day.is_some_and(|d| d < 1) {
        return Err(SyncError::Invalid(
            "Exchange calendars can't repeat on the last day of the month. Choose a date or a weekday instead.".to_owned(),
        ));
    }
    let index = INDEXES
        .iter()
        .find(|(_, n)| Some(*n) == rule.set_pos)
        .map_or("First", |(name, _)| *name);
    let day_of_month = rule
        .month_day
        .and_then(|d| u32::try_from(d).ok())
        .unwrap_or_else(|| chrono::Datelike::day(&start));
    let month_number = rule
        .month
        .unwrap_or_else(|| chrono::Datelike::month(&start));
    let month = MONTHS
        .get(usize::try_from(month_number.saturating_sub(1)).unwrap_or(0))
        .copied()
        .unwrap_or("January");
    let interval = rule.interval.max(1);
    let pattern = match (rule.freq, rule.set_pos.is_some()) {
        (Freq::Daily, _) => {
            format!("<t:DailyRecurrence><t:Interval>{interval}</t:Interval></t:DailyRecurrence>")
        }
        (Freq::Weekly, _) => format!(
            "<t:WeeklyRecurrence><t:Interval>{interval}</t:Interval><t:DaysOfWeek>{days}</t:DaysOfWeek></t:WeeklyRecurrence>"
        ),
        (Freq::Monthly, true) => format!(
            "<t:RelativeMonthlyRecurrence><t:Interval>{interval}</t:Interval><t:DaysOfWeek>{days}</t:DaysOfWeek><t:DayOfWeekIndex>{index}</t:DayOfWeekIndex></t:RelativeMonthlyRecurrence>"
        ),
        (Freq::Monthly, false) => format!(
            "<t:AbsoluteMonthlyRecurrence><t:Interval>{interval}</t:Interval><t:DayOfMonth>{day_of_month}</t:DayOfMonth></t:AbsoluteMonthlyRecurrence>"
        ),
        (Freq::Yearly, true) => format!(
            "<t:RelativeYearlyRecurrence><t:DaysOfWeek>{days}</t:DaysOfWeek><t:DayOfWeekIndex>{index}</t:DayOfWeekIndex><t:Month>{month}</t:Month></t:RelativeYearlyRecurrence>"
        ),
        (Freq::Yearly, false) => format!(
            "<t:AbsoluteYearlyRecurrence><t:DayOfMonth>{day_of_month}</t:DayOfMonth><t:Month>{month}</t:Month></t:AbsoluteYearlyRecurrence>"
        ),
    };
    let range = match (rule.count, rule.until) {
        (Some(count), _) => format!(
            "<t:NumberedRecurrence><t:StartDate>{start_day}</t:StartDate><t:NumberOfOccurrences>{}</t:NumberOfOccurrences></t:NumberedRecurrence>",
            count.max(1)
        ),
        (None, Some(until)) => format!(
            "<t:EndDateRecurrence><t:StartDate>{start_day}</t:StartDate><t:EndDate>{}</t:EndDate></t:EndDateRecurrence>",
            Utc.timestamp_opt(until, 0)
                .single()
                .unwrap_or_default()
                .format("%Y-%m-%d")
        ),
        (None, None) => {
            format!("<t:NoEndRecurrence><t:StartDate>{start_day}</t:StartDate></t:NoEndRecurrence>")
        }
    };
    Ok(format!("<t:Recurrence>{pattern}{range}</t:Recurrence>"))
}

/// An event from the editor as EWS fields. `rule`: the repeat rule to set
/// (`Some(None)` removes it, `None` leaves it); the second value is the
/// `<t:Recurrence>` to set (empty: remove), when `rule` asks for a change.
///
/// # Errors
/// A rule Exchange can't hold.
pub fn event_fields(
    input: &EventInput,
    rule: Option<Option<&RepeatRule>>,
) -> Result<(EventFields, Option<String>), SyncError> {
    // Timed events keep their zone, so a series stays at the same local
    // time across daylight saving; all-day events are days in UTC, as the
    // app stores them.
    let zone = ical::windows_zone_name(&input.time_zone)
        .filter(|_| !input.all_day)
        .unwrap_or("UTC");
    let tz: Option<chrono_tz::Tz> = input.time_zone.parse().ok().filter(|_| zone != "UTC");
    let end = input.end.max(if input.all_day {
        input.start.saturating_add(86_400)
    } else {
        input.start
    });
    let start_day = {
        let at = Utc
            .timestamp_opt(input.start, 0)
            .single()
            .unwrap_or_default();
        match tz {
            Some(tz) => at.with_timezone(&tz).format("%Y-%m-%d").to_string(),
            None => at.format("%Y-%m-%d").to_string(),
        }
    };
    let recurrence = match rule {
        Some(Some(rule)) => Some(recurrence(rule, &start_day)?),
        Some(None) => Some(String::new()),
        None => None,
    };
    let fields = EventFields {
        subject: input.title.clone(),
        body: input.notes.clone(),
        private: input.private.unwrap_or(false),
        reminder: input
            .reminders
            .as_ref()
            .and_then(|r| r.iter().max().copied()),
        start: utc(input.start),
        end: utc(end),
        all_day: input.all_day,
        show_as: match input.show_as.unwrap_or_default() {
            ShowAs::Busy => "Busy",
            ShowAs::Free => "Free",
            ShowAs::Tentative => "Tentative",
            ShowAs::Away => "OOF",
        }
        .to_owned(),
        location: input.location.clone(),
        attendees: input
            .attendees
            .iter()
            .flatten()
            .map(|a| a.trim().to_owned())
            .filter(|a| a.contains('@'))
            .collect(),
        recurrence: recurrence.clone().unwrap_or_default(),
        zone: Some(zone.to_owned()),
    };
    Ok((fields, recurrence))
}

/// Exchange's automatic replies as the app's (`offset`: the user's seconds
/// east of UTC, for the dates).
#[must_use]
pub fn vacation(oof: &Oof, offset: i32) -> Vacation {
    let scheduled = oof.state == "Scheduled";
    let internal = html_to_text(&oof.internal);
    let external = html_to_text(&oof.external);
    let (start, end) = match (&oof.duration, scheduled) {
        (Some((start, end)), true) => (
            parse_time(start).and_then(|s| day_of(s, offset)),
            parse_time(end).and_then(|e| day_of(e.saturating_sub(1), offset)),
        ),
        _ => (None, None),
    };
    Vacation {
        enabled: oof.state == "Enabled" || scheduled,
        message: internal.clone(),
        start,
        end,
        external_message: Some(external).filter(|e| !e.is_empty() && *e != internal),
        external_audience: Some(match oof.external_audience.as_str() {
            "None" => ExternalAudience::None,
            "Known" => ExternalAudience::Contacts,
            _ => ExternalAudience::All,
        }),
        ..Vacation::default()
    }
}

/// The app's automatic replies as Exchange's.
#[must_use]
pub fn oof(v: &Vacation, offset: i32, now: i64) -> Oof {
    let start = v.start.as_deref().and_then(|d| day_start(d, offset));
    let end = v
        .end
        .as_deref()
        .and_then(|d| day_start(d, offset))
        .map(|e| e.saturating_add(86_400));
    let state = match (v.enabled, start, end) {
        (false, _, _) => "Disabled",
        (true, None, None) => "Enabled",
        _ => "Scheduled",
    };
    let duration = (state == "Scheduled").then(|| {
        let start = start.unwrap_or(now);
        let end = end.unwrap_or_else(|| start.saturating_add(365 * 86_400));
        (utc(start), utc(end))
    });
    Oof {
        state: state.to_owned(),
        external_audience: match v.external_audience.unwrap_or(ExternalAudience::All) {
            ExternalAudience::None => "None",
            ExternalAudience::Contacts => "Known",
            ExternalAudience::All => "All",
        }
        .to_owned(),
        duration,
        internal: text_to_html(&v.message),
        external: text_to_html(v.external_message.as_deref().unwrap_or(&v.message)),
    }
}

/// A contact as the app's card (its id is the card's id).
#[must_use]
pub fn card(contact: &Contact) -> ContactCard {
    ContactCard {
        uid: contact.id.clone(),
        name: contact.name.trim().to_owned(),
        emails: contact.emails.clone(),
        phones: contact.phones.clone(),
        organization: contact.company.trim().to_owned(),
        title: contact.title.trim().to_owned(),
        notes: contact.notes.trim().to_owned(),
        categories: contact.categories.clone(),
        ..ContactCard::default()
    }
}

/// The app's card as Exchange's simple fields (three addresses and three
/// phone numbers at most: Exchange has slots for that many).
#[must_use]
pub fn contact_fields(card: &ContactCard) -> ContactFields {
    ContactFields {
        name: card.name.trim().to_owned(),
        emails: card.emails.iter().take(3).cloned().collect(),
        phones: card.phones.iter().take(3).cloned().collect(),
        company: card.organization.clone(),
        title: card.title.clone(),
        notes: card.notes.clone(),
    }
}

/// Exchange's busy times as the app's (free ones left out).
#[must_use]
pub fn busy_periods(busy: &[Busy]) -> Vec<BusyPeriod> {
    busy.iter()
        .filter_map(|b| {
            let show_as = match b.kind.as_str() {
                "Free" | "NoData" | "WorkingElsewhere" => return None,
                "Tentative" => ShowAs::Tentative,
                "OOF" => ShowAs::Away,
                _ => ShowAs::Busy,
            };
            Some(BusyPeriod {
                start: parse_time(&b.start)?,
                end: parse_time(&b.end)?,
                show_as,
            })
        })
        .collect()
}

/// Escape helper re-exported for callers building small elements.
#[must_use]
pub fn xml_text(text: &str) -> String {
    escape(text)
}
