//! Microsoft Graph for Outlook.com and Microsoft 365 accounts connected
//! for calendar and contacts: calendars and events, free/busy
//! (`getSchedule`), contacts, Microsoft To Do and automatic replies.
//!
//! Microsoft has no `CalDAV`, so events are turned into iCalendar text
//! here and kept like any other calendar's: a series becomes one event
//! with its rule (from Graph's `recurrence`), changed dates become
//! `RECURRENCE-ID` copies and removed dates `EXDATE`s. The JSON mapping
//! has no I/O, so it is tested against recorded replies.

use std::collections::HashMap;

use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
use mail_core::ical::{self, BusyPeriod, EventInput, Freq, RepeatRule, ShowAs};
use mail_core::importance::Importance;
use mail_core::tasks::TaskFields;
use mail_core::vacation::{ExternalAudience, Vacation};
use mail_core::vcard::ContactCard;
use reqwest::Method;
use serde_json::{Value, json};

use crate::SyncError;
use crate::rest::{RestClient, all_pages, segment, set, text, with_query};

/// Microsoft Graph, version 1.0.
pub const GRAPH: &str = "https://graph.microsoft.com/v1.0";

/// The event fields read.
const EVENT_FIELDS: &str = "id,iCalUId,subject,body,start,end,isAllDay,location,showAs,sensitivity,isReminderOn,reminderMinutesBeforeStart,organizer,attendees,isOrganizer,type,seriesMasterId,originalStart,recurrence,changeKey,isCancelled,responseStatus";

/// A plain-words error for a Graph reply that says the app isn't allowed
/// this (permission not granted, or a mailbox Graph can't reach).
#[must_use]
pub fn setup_problem(body: &str) -> Option<SyncError> {
    if body.contains("MailboxNotEnabledForRESTAPI")
        || body.contains("MailboxNotSupportedForRESTAPI")
    {
        return Some(SyncError::Config(
            "Microsoft can't reach this mailbox's calendar and contacts for Mocky Mail (it may be on a company's own Exchange server).".to_owned(),
        ));
    }
    if body.contains("ErrorAccessDenied")
        || body.contains("Authorization_RequestDenied")
        || body.contains("AccessDenied")
    {
        return Some(SyncError::Config(
            "Microsoft didn't give Mocky Mail access to this. Connect calendar and contacts again and allow everything it asks for; a work account's administrator may need to allow Mocky Mail first.".to_owned(),
        ));
    }
    None
}

fn next_link(page: &Value, _url: &str) -> Option<String> {
    page.get("@odata.nextLink")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// HTML from Outlook as plain text.
#[must_use]
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        out.push_str(rest.get(..open).unwrap_or_default());
        let after = rest.get(open..).unwrap_or_default();
        let close = after.find('>').unwrap_or(after.len().saturating_sub(1));
        let tag = after
            .get(1..close)
            .unwrap_or_default()
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let closing = after.get(1..2) == Some("/");
        if tag == "br" || (closing && matches!(tag.as_str(), "p" | "div" | "li" | "tr")) {
            out.push('\n');
        }
        rest = after.get(close.saturating_add(1)..).unwrap_or_default();
        if !closing && matches!(tag.as_str(), "style" | "script" | "head") {
            let end = format!("</{tag}");
            let skip = rest.to_ascii_lowercase().find(&end).unwrap_or(rest.len());
            rest = rest.get(skip..).unwrap_or_default();
        }
    }
    out.push_str(rest);
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .replace("\r\n", "\n");
    let mut lines: Vec<&str> = decoded.lines().map(str::trim_end).collect();
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    let mut text = lines.join("\n");
    while text.contains("\n\n\n") {
        text = text.replace("\n\n\n", "\n\n");
    }
    text.trim().to_owned()
}

/// Plain text as simple HTML for Outlook.
#[must_use]
pub fn text_to_html(text: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    format!(
        "<html><body>{}</body></html>",
        escaped.replace('\n', "<br>")
    )
}

// -------------------------------------------------------------- calendars

/// A calendar of the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCalendar {
    /// Graph's id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// `#rrggbb`, if it has one.
    pub color: Option<String>,
    /// Events can be added and changed.
    pub writable: bool,
    /// The account's main calendar.
    pub default: bool,
}

/// The calendars in a `/me/calendars` reply.
#[must_use]
pub fn calendars_from_json(items: &[Value]) -> Vec<GraphCalendar> {
    items
        .iter()
        .filter(|c| !text(c, "id").is_empty())
        .map(|c| GraphCalendar {
            id: text(c, "id").to_owned(),
            name: Some(text(c, "name").trim())
                .filter(|n| !n.is_empty())
                .unwrap_or("Calendar")
                .to_owned(),
            color: Some(text(c, "hexColor").to_owned()).filter(|h| h.starts_with('#')),
            writable: c.get("canEdit").and_then(Value::as_bool).unwrap_or(true),
            default: c
                .get("isDefaultCalendar")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
        .collect()
}

/// The account's calendars.
///
/// # Errors
/// Network errors and error replies.
pub async fn list_calendars(client: &RestClient) -> Result<Vec<GraphCalendar>, SyncError> {
    let first = format!("{GRAPH}/me/calendars?$top=100");
    let (items, _) = all_pages(client, first, "value", next_link, 10).await?;
    Ok(calendars_from_json(&items))
}

/// A Graph time (`{dateTime, timeZone}`, UTC as asked for) in Unix seconds.
fn graph_time(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    let raw = text(value, "dateTime");
    let raw = raw.split('.').next().unwrap_or(raw).trim_end_matches('Z');
    let at = NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S").ok()?;
    let zone = text(value, "timeZone");
    if zone.is_empty() || zone.eq_ignore_ascii_case("UTC") {
        return Some(at.and_utc().timestamp());
    }
    let tz = zone
        .parse::<chrono_tz::Tz>()
        .ok()
        .or_else(|| ical::iana_zone(zone).and_then(|z| z.parse().ok()))?;
    tz.from_local_datetime(&at)
        .earliest()
        .map(|d| d.timestamp())
}

/// An ISO time (`2026-11-04T06:00:00Z`, `…+03:00`, or without a zone:
/// UTC) in Unix seconds.
fn iso_time(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Some(d.timestamp());
    }
    let raw = raw.split('.').next().unwrap_or(raw);
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|d| d.and_utc().timestamp())
}

fn stamp(seconds: i64) -> String {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .unwrap_or_default()
        .format("%Y%m%dT%H%M%SZ")
        .to_string()
}

fn date(seconds: i64) -> String {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .unwrap_or_default()
        .format("%Y%m%d")
        .to_string()
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

fn quote(value: &str) -> String {
    let clean: String = value
        .chars()
        .filter(|c| !c.is_control() && *c != '"')
        .collect();
    if clean.contains([':', ';', ',']) {
        format!("\"{clean}\"")
    } else {
        clean
    }
}

const DAYS: [(&str, &str); 7] = [
    ("monday", "MO"),
    ("tuesday", "TU"),
    ("wednesday", "WE"),
    ("thursday", "TH"),
    ("friday", "FR"),
    ("saturday", "SA"),
    ("sunday", "SU"),
];
const INDEXES: [(&str, i32); 5] = [
    ("first", 1),
    ("second", 2),
    ("third", 3),
    ("fourth", 4),
    ("last", -1),
];

/// The `RRULE` value for Graph's `recurrence` (`patternedRecurrence`).
#[must_use]
pub fn recurrence_rule(recurrence: &Value, all_day: bool) -> Option<String> {
    let pattern = recurrence.get("pattern")?;
    let range = recurrence.get("range");
    let interval = pattern
        .get("interval")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1);
    let days: Vec<&str> = pattern
        .get("daysOfWeek")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .filter_map(|d| {
                    DAYS.iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(d))
                        .map(|(_, code)| *code)
                })
                .collect()
        })
        .unwrap_or_default();
    let index = INDEXES
        .iter()
        .find(|(name, _)| *name == text(pattern, "index"))
        .map_or(1, |(_, n)| *n);
    let day_of_month = pattern
        .get("dayOfMonth")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let month = pattern.get("month").and_then(Value::as_u64).unwrap_or(0);
    let mut parts = match text(pattern, "type") {
        "daily" => vec!["FREQ=DAILY".to_owned()],
        "weekly" => {
            let mut p = vec!["FREQ=WEEKLY".to_owned()];
            if !days.is_empty() {
                p.push(format!("BYDAY={}", days.join(",")));
            }
            p
        }
        "absoluteMonthly" => vec![
            "FREQ=MONTHLY".to_owned(),
            format!("BYMONTHDAY={day_of_month}"),
        ],
        "relativeMonthly" => vec![
            "FREQ=MONTHLY".to_owned(),
            format!("BYDAY={}", days.join(",")),
            format!("BYSETPOS={index}"),
        ],
        "absoluteYearly" => vec![
            "FREQ=YEARLY".to_owned(),
            format!("BYMONTH={month}"),
            format!("BYMONTHDAY={day_of_month}"),
        ],
        "relativeYearly" => vec![
            "FREQ=YEARLY".to_owned(),
            format!("BYMONTH={month}"),
            format!("BYDAY={}", days.join(",")),
            format!("BYSETPOS={index}"),
        ],
        _ => return None,
    };
    if interval > 1 {
        parts.insert(1, format!("INTERVAL={interval}"));
    }
    match range.map(|r| text(r, "type")) {
        Some("numbered") => {
            let n = range
                .and_then(|r| r.get("numberOfOccurrences"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if n > 0 {
                parts.push(format!("COUNT={n}"));
            }
        }
        Some("endDate") => {
            if let Some(end) = range
                .map(|r| text(r, "endDate"))
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            {
                if all_day {
                    parts.push(format!("UNTIL={}", end.format("%Y%m%d")));
                } else {
                    parts.push(format!("UNTIL={}T235959Z", end.format("%Y%m%d")));
                }
            }
        }
        _ => {}
    }
    Some(parts.join(";"))
}

/// Graph's `recurrence` for a repeat rule, for a series starting on
/// `start_day` (`YYYY-MM-DD`).
///
/// # Errors
/// A rule Microsoft calendars can't hold (the last day of each month).
pub fn recurrence_json(rule: &RepeatRule, start_day: &str, zone: &str) -> Result<Value, SyncError> {
    let day_name = |code: &str| {
        DAYS.iter()
            .find(|(_, c)| c.eq_ignore_ascii_case(code))
            .map(|(name, _)| *name)
    };
    let start = NaiveDate::parse_from_str(start_day, "%Y-%m-%d")
        .map_err(|e| SyncError::Invalid(e.to_string()))?;
    let weekday_of_start = DAYS
        .get(usize::try_from(chrono::Datelike::weekday(&start).num_days_from_monday()).unwrap_or(0))
        .map_or("monday", |(n, _)| *n);
    let days: Vec<&str> = if rule.by_day.is_empty() {
        vec![weekday_of_start]
    } else {
        rule.by_day.iter().filter_map(|d| day_name(d)).collect()
    };
    let index = INDEXES
        .iter()
        .find(|(_, n)| Some(*n) == rule.set_pos)
        .map_or("first", |(name, _)| *name);
    if rule.month_day.is_some_and(|d| d < 1) {
        return Err(SyncError::Invalid(
            "Microsoft calendars can't repeat on the last day of the month. Choose a date or a weekday instead.".to_owned(),
        ));
    }
    let day_of_month = rule
        .month_day
        .and_then(|d| u32::try_from(d).ok())
        .unwrap_or_else(|| chrono::Datelike::day(&start));
    let month = rule
        .month
        .unwrap_or_else(|| chrono::Datelike::month(&start));
    let interval = rule.interval.max(1);
    let pattern = match (rule.freq, rule.set_pos.is_some()) {
        (Freq::Daily, _) => json!({ "type": "daily", "interval": interval }),
        (Freq::Weekly, _) => json!({
            "type": "weekly", "interval": interval, "daysOfWeek": days, "firstDayOfWeek": "monday"
        }),
        (Freq::Monthly, true) => json!({
            "type": "relativeMonthly", "interval": interval, "daysOfWeek": days, "index": index
        }),
        (Freq::Monthly, false) => json!({
            "type": "absoluteMonthly", "interval": interval, "dayOfMonth": day_of_month
        }),
        (Freq::Yearly, true) => json!({
            "type": "relativeYearly", "interval": interval, "daysOfWeek": days, "index": index, "month": month
        }),
        (Freq::Yearly, false) => json!({
            "type": "absoluteYearly", "interval": interval, "dayOfMonth": day_of_month, "month": month
        }),
    };
    let range = match (rule.count, rule.until) {
        (Some(n), _) => json!({
            "type": "numbered", "startDate": start_day, "numberOfOccurrences": n, "recurrenceTimeZone": zone
        }),
        (None, Some(until)) => json!({
            "type": "endDate", "startDate": start_day,
            "endDate": Utc.timestamp_opt(until, 0).single().unwrap_or_default().format("%Y-%m-%d").to_string(),
            "recurrenceTimeZone": zone
        }),
        (None, None) => {
            json!({ "type": "noEnd", "startDate": start_day, "recurrenceTimeZone": zone })
        }
    };
    Ok(json!({ "pattern": pattern, "range": range }))
}

fn person_line(name: &str, person: &Value, partstat: Option<&str>) -> Option<String> {
    let address = person.get("emailAddress")?;
    let email = text(address, "address").trim();
    if !email.contains('@')
        || email
            .chars()
            .any(|c| c.is_control() || c == ';' || c == ':')
    {
        return None;
    }
    let mut line = name.to_owned();
    if let Some(status) = partstat {
        line.push_str(";PARTSTAT=");
        line.push_str(status);
    }
    let cn = text(address, "name").trim();
    if !cn.is_empty() {
        line.push_str(";CN=");
        line.push_str(&quote(cn));
    }
    line.push_str(":mailto:");
    line.push_str(email);
    Some(line)
}

fn partstat(attendee: &Value) -> &'static str {
    match attendee
        .get("status")
        .map(|s| text(s, "response"))
        .unwrap_or_default()
    {
        "accepted" | "organizer" => "ACCEPTED",
        "tentativelyAccepted" => "TENTATIVE",
        "declined" => "DECLINED",
        _ => "NEEDS-ACTION",
    }
}

/// `VEVENT` lines for one Graph event. `rid` names a changed date of a
/// series; `rule` and `exdates` make it the series itself.
#[allow(clippy::too_many_lines, reason = "one property after another")]
fn vevent(
    event: &Value,
    uid: &str,
    rid: Option<i64>,
    rule: Option<&str>,
    exdates: &[i64],
) -> Option<Vec<String>> {
    let all_day = event
        .get("isAllDay")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let start = graph_time(event.get("start"))?;
    let end = graph_time(event.get("end")).unwrap_or(start).max(start);
    let mut lines = vec!["BEGIN:VEVENT".to_owned(), format!("UID:{}", escape(uid))];
    if let Some(rid) = rid {
        lines.push(if all_day {
            format!("RECURRENCE-ID;VALUE=DATE:{}", date(rid))
        } else {
            format!("RECURRENCE-ID:{}", stamp(rid))
        });
    }
    if all_day {
        lines.push(format!("DTSTART;VALUE=DATE:{}", date(start)));
        lines.push(format!(
            "DTEND;VALUE=DATE:{}",
            date(end.max(start.saturating_add(86_400)))
        ));
    } else {
        lines.push(format!("DTSTART:{}", stamp(start)));
        lines.push(format!("DTEND:{}", stamp(end)));
    }
    if let Some(rule) = rule {
        lines.push(format!("RRULE:{rule}"));
        for ex in exdates {
            lines.push(if all_day {
                format!("EXDATE;VALUE=DATE:{}", date(*ex))
            } else {
                format!("EXDATE:{}", stamp(*ex))
            });
        }
    }
    let subject = text(event, "subject").trim();
    if !subject.is_empty() {
        lines.push(format!("SUMMARY:{}", escape(subject)));
    }
    let place = event
        .get("location")
        .map(|l| text(l, "displayName"))
        .unwrap_or_default()
        .trim();
    if !place.is_empty() {
        lines.push(format!("LOCATION:{}", escape(place)));
    }
    let body = event.get("body");
    let notes = match body.map(|b| text(b, "contentType")) {
        Some("html") => html_to_text(body.map(|b| text(b, "content")).unwrap_or_default()),
        _ => body
            .map(|b| text(b, "content"))
            .unwrap_or_default()
            .trim()
            .to_owned(),
    };
    if !notes.is_empty() {
        lines.push(format!("DESCRIPTION:{}", escape(&notes)));
    }
    let show_as = match text(event, "showAs") {
        "free" => ShowAs::Free,
        "tentative" => ShowAs::Tentative,
        "oof" => ShowAs::Away,
        _ => ShowAs::Busy,
    };
    lines.push(
        if show_as == ShowAs::Free {
            "TRANSP:TRANSPARENT"
        } else {
            "TRANSP:OPAQUE"
        }
        .to_owned(),
    );
    lines.push(format!(
        "X-MICROSOFT-CDO-BUSYSTATUS:{}",
        match show_as {
            ShowAs::Busy => "BUSY",
            ShowAs::Free => "FREE",
            ShowAs::Tentative => "TENTATIVE",
            ShowAs::Away => "OOF",
        }
    ));
    if matches!(text(event, "sensitivity"), "private" | "confidential") {
        lines.push("CLASS:PRIVATE".to_owned());
    }
    if let Some(org) = event
        .get("organizer")
        .and_then(|o| person_line("ORGANIZER", o, None))
    {
        lines.push(org);
    }
    for attendee in event
        .get("attendees")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(line) = person_line("ATTENDEE", attendee, Some(partstat(attendee))) {
            lines.push(line);
        }
    }
    if event
        .get("isCancelled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        lines.push("STATUS:CANCELLED".to_owned());
    } else if text(
        event.get("responseStatus").unwrap_or(&Value::Null),
        "response",
    ) == "tentativelyAccepted"
    {
        lines.push("STATUS:TENTATIVE".to_owned());
    }
    lines.push(format!("X-MOCKY-GRAPH-ID:{}", escape(text(event, "id"))));
    if event
        .get("isReminderOn")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let minutes = event
            .get("reminderMinutesBeforeStart")
            .and_then(Value::as_i64)
            .unwrap_or(15)
            .max(0);
        lines.extend([
            "BEGIN:VALARM".to_owned(),
            "ACTION:DISPLAY".to_owned(),
            "DESCRIPTION:Reminder".to_owned(),
            format!("TRIGGER:-PT{minutes}M"),
            "END:VALARM".to_owned(),
        ]);
    }
    lines.push("END:VEVENT".to_owned());
    Some(lines)
}

fn finish(lines: &[String]) -> String {
    let mut out = String::new();
    for line in lines {
        let mut width = 0usize;
        for c in line.chars() {
            let len = c.len_utf8();
            if width.saturating_add(len) > 75 {
                out.push_str("\r\n ");
                width = 1;
            }
            out.push(c);
            width = width.saturating_add(len);
        }
        out.push_str("\r\n");
    }
    out
}

fn calendar_lines(events: Vec<Vec<String>>) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_owned(),
        "VERSION:2.0".to_owned(),
        "PRODID:-//Mocky Digital//Mocky Mail (Microsoft Graph)//EN".to_owned(),
    ];
    lines.extend(events.into_iter().flatten());
    lines.push("END:VCALENDAR".to_owned());
    finish(&lines)
}

/// The iCalendar text for a single event.
#[must_use]
pub fn single_ics(event: &Value) -> Option<String> {
    let uid = Some(text(event, "iCalUId"))
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| text(event, "id"));
    Some(calendar_lines(vec![vevent(event, uid, None, None, &[])?]))
}

/// The iCalendar text for a series: its master with the rule, its dates
/// between `from` and `to` that were changed (`instances` of type
/// `exception`) as their own copies, and those missing from `instances`
/// (cancelled) as `EXDATE`s.
#[must_use]
pub fn series_ics(master: &Value, instances: &[Value], from: i64, to: i64) -> Option<String> {
    let uid = Some(text(master, "iCalUId"))
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| text(master, "id"));
    let all_day = master
        .get("isAllDay")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let Some(rule) = master
        .get("recurrence")
        .filter(|r| !r.is_null())
        .and_then(|r| recurrence_rule(r, all_day))
    else {
        return single_ics(master);
    };
    let original = |i: &Value| {
        i.get("originalStart")
            .and_then(Value::as_str)
            .and_then(iso_time)
            .or_else(|| graph_time(i.get("start")))
    };
    let present: Vec<i64> = instances.iter().filter_map(original).collect();
    // The dates the rule gives, to see which were cancelled.
    let bare = calendar_lines(vec![vevent(master, uid, None, Some(&rule), &[])?]);
    let exdates: Vec<i64> = ical::occurrences(&bare, from, to)
        .into_iter()
        .filter_map(|o| o.recurrence_id)
        .filter(|rid| !present.contains(rid))
        .collect();
    let mut events = vec![vevent(master, uid, None, Some(&rule), &exdates)?];
    for instance in instances.iter().filter(|i| text(i, "type") == "exception") {
        if let Some(rid) = original(instance)
            && let Some(block) = vevent(instance, uid, Some(rid), None, &[])
        {
            events.push(block);
        }
    }
    Some(calendar_lines(events))
}

/// A resource built from Graph: where it is kept (the event's id, or the
/// series master's), a version tag, and its iCalendar text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphResource {
    /// The event id (single event) or series master id.
    pub id: String,
    /// Changes whenever the event or any of its dates changes.
    pub etag: String,
    /// iCalendar text.
    pub ics: String,
}

/// 64-bit FNV-1a, for a version tag that is the same on every run.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash: u64, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Single events, each with its version tag.
pub type Singles = Vec<(Value, String)>;
/// Series by master id: their instances and version tag.
pub type Series = HashMap<String, (Vec<Value>, String)>;

/// The instances of a `calendarView` grouped into what is kept: single
/// events, and series (by master id) with their instances. Returns
/// `(singles, series)` and each one's version tag.
#[must_use]
pub fn group_instances(items: &[Value]) -> (Singles, Series) {
    let mut singles = Vec::new();
    let mut series: HashMap<String, (Vec<Value>, String)> = HashMap::new();
    for item in items {
        let key = text(item, "changeKey");
        match text(item, "seriesMasterId") {
            "" => singles.push((item.clone(), format!("{:016x}", fnv1a(key.as_bytes())))),
            master => {
                let entry = series.entry(master.to_owned()).or_default();
                entry.0.push(item.clone());
            }
        }
    }
    for (instances, tag) in series.values_mut() {
        let mut keys: Vec<String> = instances
            .iter()
            .map(|i| format!("{}={}", text(i, "id"), text(i, "changeKey")))
            .collect();
        keys.sort();
        *tag = format!("{:016x}", fnv1a(keys.join(",").as_bytes()));
    }
    (singles, series)
}

/// Every event occurrence in a calendar between `from` and `to`.
///
/// # Errors
/// Network errors and error replies.
pub async fn calendar_view(
    client: &RestClient,
    calendar_id: &str,
    from: i64,
    to: i64,
) -> Result<Vec<Value>, SyncError> {
    let iso = |s: i64| {
        Utc.timestamp_opt(s, 0)
            .single()
            .unwrap_or_default()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    };
    let first = with_query(
        &format!("{GRAPH}/me/calendars/{}/calendarView", segment(calendar_id)),
        &[
            ("startDateTime", &iso(from)),
            ("endDateTime", &iso(to)),
            ("$top", "250"),
            ("$select", EVENT_FIELDS),
        ],
    )?;
    let (items, _) = all_pages(client, first, "value", next_link, 100).await?;
    Ok(items)
}

/// One event (a series master, for its rule).
///
/// # Errors
/// Network errors and error replies.
pub async fn get_event(client: &RestClient, id: &str) -> Result<Value, SyncError> {
    client
        .get(&with_query(
            &format!("{GRAPH}/me/events/{}", segment(id)),
            &[("$select", EVENT_FIELDS)],
        )?)
        .await
}

/// The id of one date of a series (by its original start).
///
/// # Errors
/// Network errors and error replies.
pub async fn instance_id(
    client: &RestClient,
    master_id: &str,
    recurrence_id: i64,
) -> Result<Option<String>, SyncError> {
    let iso = |s: i64| {
        Utc.timestamp_opt(s, 0)
            .single()
            .unwrap_or_default()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    };
    let url = with_query(
        &format!("{GRAPH}/me/events/{}/instances", segment(master_id)),
        &[
            ("startDateTime", &iso(recurrence_id.saturating_sub(86_400))),
            (
                "endDateTime",
                &iso(recurrence_id.saturating_add(2 * 86_400)),
            ),
            ("$select", "id,originalStart,start"),
        ],
    )?;
    let page = client.get(&url).await?;
    Ok(page
        .get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|i| {
            i.get("originalStart")
                .and_then(Value::as_str)
                .and_then(iso_time)
                .or_else(|| graph_time(i.get("start")))
                == Some(recurrence_id)
        })
        .map(|i| text(i, "id").to_owned()))
}

/// Graph's JSON for an event from the editor. `rule` is the repeat rule
/// to set (`Some(None)` removes it, `None` leaves it).
///
/// # Errors
/// A rule Microsoft calendars can't hold.
pub fn event_json(
    input: &EventInput,
    rule: Option<Option<&RepeatRule>>,
) -> Result<Value, SyncError> {
    let zone = ical::windows_zone_name(&input.time_zone)
        .map(str::to_owned)
        .filter(|_| !input.all_day)
        .unwrap_or_else(|| "UTC".to_owned());
    let tz: Option<chrono_tz::Tz> = input.time_zone.parse().ok().filter(|_| zone != "UTC");
    let local = |s: i64| {
        let utc = Utc.timestamp_opt(s, 0).single().unwrap_or_default();
        match tz {
            Some(tz) => utc
                .with_timezone(&tz)
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string(),
            None => utc.format("%Y-%m-%dT%H:%M:%S").to_string(),
        }
    };
    let end = input.end.max(if input.all_day {
        input.start.saturating_add(86_400)
    } else {
        input.start
    });
    let mut out = json!({
        "subject": input.title,
        "body": { "contentType": "text", "content": input.notes },
        "start": { "dateTime": local(input.start), "timeZone": zone },
        "end": { "dateTime": local(end), "timeZone": zone },
        "isAllDay": input.all_day,
        "location": { "displayName": input.location },
    });
    if let Some(show_as) = input.show_as {
        let value = match show_as {
            ShowAs::Busy => "busy",
            ShowAs::Free => "free",
            ShowAs::Tentative => "tentative",
            ShowAs::Away => "oof",
        };
        set(&mut out, "showAs", json!(value));
    }
    if let Some(private) = input.private {
        set(
            &mut out,
            "sensitivity",
            json!(if private { "private" } else { "normal" }),
        );
    }
    if let Some(reminders) = &input.reminders {
        match reminders.iter().max() {
            Some(m) => {
                set(&mut out, "isReminderOn", json!(true));
                set(&mut out, "reminderMinutesBeforeStart", json!(m));
            }
            None => set(&mut out, "isReminderOn", json!(false)),
        }
    }
    if let Some(attendees) = &input.attendees {
        let people = attendees
            .iter()
            .filter(|a| a.contains('@'))
            .map(|a| json!({ "emailAddress": { "address": a.trim() }, "type": "required" }))
            .collect();
        set(&mut out, "attendees", Value::Array(people));
    }
    match rule {
        Some(Some(rule)) => {
            let day = local(input.start).get(..10).unwrap_or_default().to_owned();
            set(&mut out, "recurrence", recurrence_json(rule, &day, &zone)?);
        }
        Some(None) => set(&mut out, "recurrence", Value::Null),
        None => {}
    }
    Ok(out)
}

/// Create an event in a calendar. Graph sends invitations to attendees.
///
/// # Errors
/// Network errors and error replies.
pub async fn create_event(
    client: &RestClient,
    calendar_id: &str,
    body: &Value,
) -> Result<Value, SyncError> {
    Ok(client
        .send(
            Method::POST,
            &format!("{GRAPH}/me/calendars/{}/events", segment(calendar_id)),
            Some(body),
        )
        .await?
        .body)
}

/// Change an event (a series, or one date of it by its instance id).
/// Graph sends updates to attendees.
///
/// # Errors
/// Network errors and error replies.
pub async fn update_event(client: &RestClient, id: &str, body: &Value) -> Result<Value, SyncError> {
    Ok(client
        .send(
            Method::PATCH,
            &format!("{GRAPH}/me/events/{}", segment(id)),
            Some(body),
        )
        .await?
        .body)
}

/// Delete an event (or one date of a series by its instance id). For a
/// meeting you organise, Graph sends the cancellation. One already gone
/// counts as deleted.
///
/// # Errors
/// Network errors and error replies.
pub async fn delete_event(client: &RestClient, id: &str) -> Result<(), SyncError> {
    match client
        .send(
            Method::DELETE,
            &format!("{GRAPH}/me/events/{}", segment(id)),
            None,
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(SyncError::Refused(why)) if why.contains("404") => Ok(()),
        Err(other) => Err(other),
    }
}

/// Each address's busy times from a `getSchedule` reply; `None` for those
/// Microsoft couldn't read.
#[must_use]
pub fn schedule_from_json(
    body: &Value,
    emails: &[String],
) -> Vec<(String, Option<Vec<BusyPeriod>>)> {
    let items = body.get("value").and_then(Value::as_array);
    emails
        .iter()
        .map(|email| {
            let entry = items.and_then(|list| {
                list.iter()
                    .find(|s| text(s, "scheduleId").eq_ignore_ascii_case(email))
            });
            let busy = entry
                .filter(|e| e.get("error").is_none_or(Value::is_null))
                .map(|e| {
                    e.get("scheduleItems")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|item| {
                            let show_as = match text(item, "status") {
                                "free" | "unknown" => return None,
                                "tentative" => ShowAs::Tentative,
                                "oof" => ShowAs::Away,
                                _ => ShowAs::Busy,
                            };
                            Some(BusyPeriod {
                                start: graph_time(item.get("start"))?,
                                end: graph_time(item.get("end"))?,
                                show_as,
                            })
                        })
                        .collect()
                });
            (email.clone(), busy)
        })
        .collect()
}

/// When these people are busy (their organisations permitting).
///
/// # Errors
/// Network errors and error replies.
pub async fn get_schedule(
    client: &RestClient,
    emails: &[String],
    from: i64,
    to: i64,
) -> Result<Vec<(String, Option<Vec<BusyPeriod>>)>, SyncError> {
    let at = |s: i64| {
        Utc.timestamp_opt(s, 0)
            .single()
            .unwrap_or_default()
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string()
    };
    let body = json!({
        "schedules": emails,
        "startTime": { "dateTime": at(from), "timeZone": "UTC" },
        "endTime": { "dateTime": at(to), "timeZone": "UTC" },
        "availabilityViewInterval": 30,
    });
    let reply = client
        .send(
            Method::POST,
            &format!("{GRAPH}/me/calendar/getSchedule"),
            Some(&body),
        )
        .await?;
    Ok(schedule_from_json(&reply.body, emails))
}

// --------------------------------------------------------------- contacts

/// A contact from Graph's JSON: (id, version tag, card). Graph keeps no
/// favourites or pictures the app can write cheaply, so those stay as the
/// device has them.
#[must_use]
pub fn contact_from_json(value: &Value) -> Option<(String, Option<String>, ContactCard)> {
    let id = text(value, "id");
    if id.is_empty() {
        return None;
    }
    let strings = |key: &str| -> Vec<String> {
        value
            .get(key)
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut phones: Vec<String> = Vec::new();
    for phone in Some(text(value, "mobilePhone").trim().to_owned())
        .into_iter()
        .filter(|p| !p.is_empty())
        .chain(strings("businessPhones"))
        .chain(strings("homePhones"))
    {
        if !phones.contains(&phone) {
            phones.push(phone);
        }
    }
    let emails: Vec<String> = value
        .get("emailAddresses")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|e| text(e, "address").trim().to_owned())
        .filter(|e| e.contains('@'))
        .collect();
    let mut name = text(value, "displayName").trim().to_owned();
    if name.is_empty() {
        name = [text(value, "givenName"), text(value, "surname")]
            .into_iter()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
    }
    let card = ContactCard {
        uid: id.to_owned(),
        name,
        emails,
        phones,
        organization: text(value, "companyName").trim().to_owned(),
        title: text(value, "jobTitle").trim().to_owned(),
        notes: text(value, "personalNotes").trim().to_owned(),
        categories: strings("categories"),
        ..ContactCard::default()
    };
    if card.name.is_empty() && card.emails.is_empty() && card.phones.is_empty() {
        return None;
    }
    let etag = Some(text(value, "changeKey").to_owned())
        .or_else(|| {
            value
                .get("@odata.etag")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .filter(|e| !e.is_empty());
    Some((id.to_owned(), etag, card))
}

/// Graph's JSON for a contact.
#[must_use]
pub fn contact_json(card: &ContactCard) -> Value {
    let (given, surname) = match card.name.trim().rsplit_once(' ') {
        Some((g, s)) => (g.to_owned(), s.to_owned()),
        None => (card.name.trim().to_owned(), String::new()),
    };
    let mut phones = card.phones.iter();
    let mobile = phones.next().cloned();
    json!({
        "displayName": card.name,
        "givenName": given,
        "surname": surname,
        "emailAddresses": card.emails.iter().map(|e| json!({ "address": e, "name": card.name })).collect::<Vec<_>>(),
        "mobilePhone": mobile,
        "businessPhones": phones.cloned().collect::<Vec<_>>(),
        "companyName": card.organization,
        "jobTitle": card.title,
        "personalNotes": card.notes,
        "categories": card.categories,
    })
}

/// What changed in the address book since `delta` (or everything, the
/// first time): changed contacts, removed ids, and the link for next time.
///
/// # Errors
/// Network errors and error replies; [`SyncError::Refused`] when the
/// delta link expired (start again without one).
pub async fn contacts_delta(
    client: &RestClient,
    delta: Option<&str>,
) -> Result<(Vec<Value>, Vec<String>, Option<String>), SyncError> {
    let first = match delta {
        Some(link) => link.to_owned(),
        None => format!(
            "{GRAPH}/me/contacts/delta?$select=id,displayName,givenName,surname,emailAddresses,mobilePhone,businessPhones,homePhones,companyName,jobTitle,personalNotes,categories,changeKey"
        ),
    };
    let (items, last) = all_pages(client, first, "value", next_link, 200).await?;
    let (removed, changed): (Vec<Value>, Vec<Value>) =
        items.into_iter().partition(|i| i.get("@removed").is_some());
    let link = last
        .get("@odata.deltaLink")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok((
        changed,
        removed.iter().map(|r| text(r, "id").to_owned()).collect(),
        link,
    ))
}

/// Create a contact; returns (id, version tag).
///
/// # Errors
/// Network errors and error replies.
pub async fn create_contact(
    client: &RestClient,
    card: &ContactCard,
) -> Result<(String, Option<String>), SyncError> {
    let reply = client
        .send(
            Method::POST,
            &format!("{GRAPH}/me/contacts"),
            Some(&contact_json(card)),
        )
        .await?;
    contact_from_json(&reply.body)
        .map(|(id, etag, _)| (id, etag))
        .or_else(|| {
            Some((text(&reply.body, "id").to_owned(), None)).filter(|(id, _)| !id.is_empty())
        })
        .ok_or_else(|| SyncError::Protocol("Microsoft didn't return the contact".to_owned()))
}

/// Change a contact; returns its new version tag.
///
/// # Errors
/// Network errors and error replies.
pub async fn update_contact(
    client: &RestClient,
    id: &str,
    card: &ContactCard,
) -> Result<Option<String>, SyncError> {
    let reply = client
        .send(
            Method::PATCH,
            &format!("{GRAPH}/me/contacts/{}", segment(id)),
            Some(&contact_json(card)),
        )
        .await?;
    Ok(Some(text(&reply.body, "changeKey").to_owned()).filter(|e| !e.is_empty()))
}

/// Delete a contact. One already gone counts as deleted.
///
/// # Errors
/// Network errors and error replies.
pub async fn delete_contact(client: &RestClient, id: &str) -> Result<(), SyncError> {
    match client
        .send(
            Method::DELETE,
            &format!("{GRAPH}/me/contacts/{}", segment(id)),
            None,
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(SyncError::Refused(why)) if why.contains("404") => Ok(()),
        Err(other) => Err(other),
    }
}

// ------------------------------------------------------------------ to do

/// A Microsoft To Do task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTodo {
    /// Graph's id.
    pub id: String,
    /// Its version tag.
    pub etag: Option<String>,
    /// Its fields.
    pub fields: TaskFields,
    /// The JSON as Graph sent it.
    pub raw: String,
}

/// A task from Graph's JSON.
#[must_use]
pub fn todo_from_json(value: &Value) -> Option<RemoteTodo> {
    let id = text(value, "id");
    if id.is_empty() {
        return None;
    }
    let completed = text(value, "status") == "completed";
    let body = value.get("body");
    let notes = match body.map(|b| text(b, "contentType")) {
        Some("html") => html_to_text(body.map(|b| text(b, "content")).unwrap_or_default()),
        _ => body
            .map(|b| text(b, "content"))
            .unwrap_or_default()
            .trim()
            .to_owned(),
    };
    // A due "day" from To Do is midnight where it was set; the app keeps
    // days as midnight UTC.
    let due = graph_time(value.get("dueDateTime")).map(|d| {
        let day = Utc
            .timestamp_opt(d.saturating_add(12 * 3600), 0)
            .single()
            .unwrap_or_default()
            .date_naive();
        day.and_hms_opt(0, 0, 0)
            .unwrap_or_default()
            .and_utc()
            .timestamp()
    });
    let reminder = value
        .get("isReminderOn")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        .then(|| graph_time(value.get("reminderDateTime")))
        .flatten();
    Some(RemoteTodo {
        id: id.to_owned(),
        etag: value
            .get("@odata.etag")
            .and_then(Value::as_str)
            .map(str::to_owned),
        fields: TaskFields {
            title: text(value, "title").trim().to_owned(),
            notes,
            due,
            reminder,
            importance: match text(value, "importance") {
                "high" => Importance::High,
                "low" => Importance::Low,
                _ => Importance::Normal,
            },
            completed,
            completed_at: if completed {
                graph_time(value.get("completedDateTime"))
            } else {
                None
            },
            categories: value
                .get("categories")
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        },
        raw: value.to_string(),
    })
}

/// Graph's JSON for a task.
#[must_use]
pub fn todo_json(fields: &TaskFields) -> Value {
    let at = |s: i64| {
        json!({
            "dateTime": Utc.timestamp_opt(s, 0).single().unwrap_or_default().format("%Y-%m-%dT%H:%M:%S").to_string(),
            "timeZone": "UTC"
        })
    };
    let mut out = json!({
        "title": fields.title,
        "body": { "contentType": "text", "content": fields.notes },
        "importance": fields.importance.as_str(),
        "status": if fields.completed { "completed" } else { "notStarted" },
        "categories": fields.categories,
        "isReminderOn": fields.reminder.is_some(),
        "dueDateTime": fields.due.map(at),
        "reminderDateTime": fields.reminder.map(at),
    });
    if fields.completed {
        set(
            &mut out,
            "completedDateTime",
            fields.completed_at.map_or(Value::Null, at),
        );
    }
    out
}

/// The account's To Do lists: (id, name).
///
/// # Errors
/// Network errors and error replies.
pub async fn list_todo_lists(client: &RestClient) -> Result<Vec<(String, String)>, SyncError> {
    let (items, _) = all_pages(
        client,
        format!("{GRAPH}/me/todo/lists?$top=100"),
        "value",
        next_link,
        10,
    )
    .await?;
    Ok(items
        .iter()
        .filter(|l| !text(l, "id").is_empty())
        .map(|l| {
            let name = text(l, "displayName").trim();
            (
                text(l, "id").to_owned(),
                if name.is_empty() { "Tasks" } else { name }.to_owned(),
            )
        })
        .collect())
}

fn todo_url(list_id: &str) -> String {
    format!("{GRAPH}/me/todo/lists/{}/tasks", segment(list_id))
}

/// Every task in a To Do list.
///
/// # Errors
/// Network errors and error replies.
pub async fn list_todos(client: &RestClient, list_id: &str) -> Result<Vec<RemoteTodo>, SyncError> {
    let (items, _) = all_pages(
        client,
        format!("{}?$top=100", todo_url(list_id)),
        "value",
        next_link,
        100,
    )
    .await?;
    Ok(items.iter().filter_map(todo_from_json).collect())
}

/// Create a task.
///
/// # Errors
/// Network errors and error replies.
pub async fn create_todo(
    client: &RestClient,
    list_id: &str,
    fields: &TaskFields,
) -> Result<RemoteTodo, SyncError> {
    let reply = client
        .send(Method::POST, &todo_url(list_id), Some(&todo_json(fields)))
        .await?;
    todo_from_json(&reply.body)
        .ok_or_else(|| SyncError::Protocol("Microsoft didn't return the task".to_owned()))
}

/// Change a task.
///
/// # Errors
/// Network errors and error replies.
pub async fn update_todo(
    client: &RestClient,
    list_id: &str,
    id: &str,
    fields: &TaskFields,
) -> Result<RemoteTodo, SyncError> {
    let reply = client
        .send(
            Method::PATCH,
            &format!("{}/{}", todo_url(list_id), segment(id)),
            Some(&todo_json(fields)),
        )
        .await?;
    todo_from_json(&reply.body)
        .ok_or_else(|| SyncError::Protocol("Microsoft didn't return the task".to_owned()))
}

/// Delete a task. One already gone counts as deleted.
///
/// # Errors
/// Network errors and error replies.
pub async fn delete_todo(client: &RestClient, list_id: &str, id: &str) -> Result<(), SyncError> {
    match client
        .send(
            Method::DELETE,
            &format!("{}/{}", todo_url(list_id), segment(id)),
            None,
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(SyncError::Refused(why)) if why.contains("404") => Ok(()),
        Err(other) => Err(other),
    }
}

// ------------------------------------------------------- automatic replies

fn day_of(seconds: i64, offset: i32) -> Option<String> {
    Utc.timestamp_opt(seconds.saturating_add(i64::from(offset)), 0)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
}

fn day_start(day: &str, offset: i32) -> Option<i64> {
    let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?;
    Some(
        date.and_hms_opt(0, 0, 0)?
            .and_utc()
            .timestamp()
            .saturating_sub(i64::from(offset)),
    )
}

/// Automatic replies from Graph's `automaticRepliesSetting` (`offset`:
/// the user's seconds east of UTC, for the dates). Outlook has no subject
/// or "days before replying again", so those stay as they were.
#[must_use]
pub fn auto_replies_from_json(value: &Value, offset: i32) -> Vacation {
    let status = text(value, "status");
    let internal = html_to_text(text(value, "internalReplyMessage"));
    let external = html_to_text(text(value, "externalReplyMessage"));
    let scheduled = status == "scheduled";
    Vacation {
        enabled: status == "alwaysEnabled" || scheduled,
        message: internal.clone(),
        start: scheduled
            .then(|| graph_time(value.get("scheduledStartDateTime")))
            .flatten()
            .and_then(|s| day_of(s, offset)),
        end: scheduled
            .then(|| graph_time(value.get("scheduledEndDateTime")))
            .flatten()
            .and_then(|e| day_of(e.saturating_sub(1), offset)),
        external_message: Some(external).filter(|e| !e.is_empty() && *e != internal),
        external_audience: Some(match text(value, "externalAudience") {
            "none" => ExternalAudience::None,
            "contactsOnly" => ExternalAudience::Contacts,
            _ => ExternalAudience::All,
        }),
        ..Vacation::default()
    }
}

/// Graph's `automaticRepliesSetting` for automatic replies.
#[must_use]
pub fn auto_replies_json(v: &Vacation, offset: i32, now: i64) -> Value {
    let at = |s: i64| {
        json!({
            "dateTime": Utc.timestamp_opt(s, 0).single().unwrap_or_default().format("%Y-%m-%dT%H:%M:%S").to_string(),
            "timeZone": "UTC"
        })
    };
    let start = v.start.as_deref().and_then(|d| day_start(d, offset));
    let end = v
        .end
        .as_deref()
        .and_then(|d| day_start(d, offset))
        .map(|e| e.saturating_add(86_400));
    let status = match (v.enabled, start, end) {
        (false, _, _) => "disabled",
        (true, None, None) => "alwaysEnabled",
        _ => "scheduled",
    };
    let internal = text_to_html(&v.message);
    let external = text_to_html(v.external_message.as_deref().unwrap_or(&v.message));
    let mut out = json!({
        "status": status,
        "internalReplyMessage": internal,
        "externalReplyMessage": external,
        "externalAudience": match v.external_audience.unwrap_or(ExternalAudience::All) {
            ExternalAudience::None => "none",
            ExternalAudience::Contacts => "contactsOnly",
            ExternalAudience::All => "all",
        },
    });
    if status == "scheduled" {
        let start = start.unwrap_or(now);
        // No last day: a year on.
        let end = end.unwrap_or_else(|| start.saturating_add(365 * 86_400));
        set(&mut out, "scheduledStartDateTime", at(start));
        set(&mut out, "scheduledEndDateTime", at(end));
    }
    out
}

/// The account's automatic replies.
///
/// # Errors
/// Network errors and error replies.
pub async fn get_auto_replies(client: &RestClient, offset: i32) -> Result<Vacation, SyncError> {
    let value = client
        .get(&format!(
            "{GRAPH}/me/mailboxSettings/automaticRepliesSetting"
        ))
        .await?;
    Ok(auto_replies_from_json(&value, offset))
}

/// Save the account's automatic replies.
///
/// # Errors
/// Network errors and error replies.
pub async fn set_auto_replies(
    client: &RestClient,
    v: &Vacation,
    offset: i32,
    now: i64,
) -> Result<Vacation, SyncError> {
    client
        .send(
            Method::PATCH,
            &format!("{GRAPH}/me/mailboxSettings"),
            Some(&json!({ "automaticRepliesSetting": auto_replies_json(v, offset, now) })),
        )
        .await?;
    get_auto_replies(client, offset).await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    // Recorded from GET /me/calendars/{id}/calendarView with
    // Prefer: outlook.timezone="UTC" (ids shortened, fields trimmed).
    const VIEW: &str = r#"{"value":[
      {"id":"AAMk-single","iCalUId":"040000008200E00074C5B7101A82E008-single","changeKey":"ck1","subject":"Site visit, Kilimani","body":{"contentType":"text","content":"Bring the plans"},"start":{"dateTime":"2026-11-18T06:00:00.0000000","timeZone":"UTC"},"end":{"dateTime":"2026-11-18T07:00:00.0000000","timeZone":"UTC"},"isAllDay":false,"location":{"displayName":"Kilimani"},"showAs":"tentative","sensitivity":"private","isReminderOn":true,"reminderMinutesBeforeStart":15,"type":"singleInstance","seriesMasterId":null,"isOrganizer":false,"organizer":{"emailAddress":{"name":"Peter Mwangi","address":"peter@client.example"}},"attendees":[{"type":"required","status":{"response":"accepted","time":"2026-11-10T08:00:00Z"},"emailAddress":{"name":"Don","address":"don@mocky.example"}}],"responseStatus":{"response":"accepted"}},
      {"id":"AAMk-occ1","iCalUId":"040000008200E00074C5B7101A82E008-series","changeKey":"ck2","subject":"Stand-up","start":{"dateTime":"2026-11-02T06:00:00.0000000","timeZone":"UTC"},"end":{"dateTime":"2026-11-02T06:15:00.0000000","timeZone":"UTC"},"isAllDay":false,"type":"occurrence","seriesMasterId":"AAMk-master","originalStart":"2026-11-02T06:00:00Z","showAs":"busy","isReminderOn":false},
      {"id":"AAMk-exc","iCalUId":"040000008200E00074C5B7101A82E008-series","changeKey":"ck3","subject":"Stand-up (late)","start":{"dateTime":"2026-11-03T08:00:00.0000000","timeZone":"UTC"},"end":{"dateTime":"2026-11-03T08:15:00.0000000","timeZone":"UTC"},"isAllDay":false,"type":"exception","seriesMasterId":"AAMk-master","originalStart":"2026-11-03T06:00:00Z","showAs":"busy","isReminderOn":false},
      {"id":"AAMk-occ4","iCalUId":"040000008200E00074C5B7101A82E008-series","changeKey":"ck4","subject":"Stand-up","start":{"dateTime":"2026-11-05T06:00:00.0000000","timeZone":"UTC"},"end":{"dateTime":"2026-11-05T06:15:00.0000000","timeZone":"UTC"},"isAllDay":false,"type":"occurrence","seriesMasterId":"AAMk-master","originalStart":"2026-11-05T06:00:00Z","showAs":"busy","isReminderOn":false}
    ]}"#;

    const MASTER: &str = r#"{"id":"AAMk-master","iCalUId":"040000008200E00074C5B7101A82E008-series","changeKey":"ck0","subject":"Stand-up","start":{"dateTime":"2026-11-02T06:00:00.0000000","timeZone":"UTC"},"end":{"dateTime":"2026-11-02T06:15:00.0000000","timeZone":"UTC"},"isAllDay":false,"type":"seriesMaster","showAs":"busy","isReminderOn":true,"reminderMinutesBeforeStart":5,"recurrence":{"pattern":{"type":"daily","interval":1,"month":0,"dayOfMonth":0,"firstDayOfWeek":"sunday","index":"first"},"range":{"type":"numbered","startDate":"2026-11-02","endDate":"0001-01-01","recurrenceTimeZone":"E. Africa Standard Time","numberOfOccurrences":4}}}"#;

    fn ts(text: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(text)
            .unwrap()
            .timestamp()
    }

    #[test]
    fn calendar_events_become_icalendar() {
        let view: Value = serde_json::from_str(VIEW).unwrap();
        let (singles, series) = group_instances(view["value"].as_array().unwrap());
        assert_eq!(singles.len(), 1);
        assert_eq!(series.len(), 1);
        let ics = single_ics(&singles[0].0).unwrap();
        let shown = ical::master(&ics).unwrap();
        assert_eq!(shown.title, "Site visit, Kilimani");
        assert_eq!(shown.location, "Kilimani");
        assert_eq!(shown.notes, "Bring the plans");
        assert_eq!(shown.start, ts("2026-11-18T06:00:00Z"));
        assert_eq!(shown.show_as, ShowAs::Tentative);
        assert!(shown.private);
        assert_eq!(shown.reminders, [15]);
        assert_eq!(shown.organizer.unwrap().email, "peter@client.example");
        assert_eq!(shown.attendees[0].status.as_deref(), Some("accepted"));
        assert!(ics.contains("X-MOCKY-GRAPH-ID:AAMk-single"));

        // The series: 2 to 5 November, the 3rd moved and the 4th
        // cancelled: the view has 2, 3 (moved) and 5 November.
        let master: Value = serde_json::from_str(MASTER).unwrap();
        let (instances, tag) = &series["AAMk-master"];
        assert_eq!(tag.len(), 16);
        let ics = series_ics(
            &master,
            instances,
            ts("2026-11-01T00:00:00Z"),
            ts("2026-12-01T00:00:00Z"),
        )
        .unwrap();
        assert!(ics.contains("RRULE:FREQ=DAILY;COUNT=4\r\n"));
        assert!(ics.contains("EXDATE:20261104T060000Z\r\n"));
        let dates = ical::occurrences(&ics, ts("2026-11-01T00:00:00Z"), ts("2026-12-01T00:00:00Z"));
        assert_eq!(dates.len(), 3);
        let moved = dates
            .iter()
            .find(|o| o.recurrence_id == Some(ts("2026-11-03T06:00:00Z")))
            .unwrap();
        assert_eq!(moved.title, "Stand-up (late)");
        assert_eq!(moved.start, ts("2026-11-03T08:00:00Z"));
        assert_eq!(ical::master(&ics).unwrap().reminders, [5]);
    }

    #[test]
    fn recurrence_patterns_map_both_ways() {
        let rule = |pattern: Value, range: Value| {
            recurrence_rule(&json!({ "pattern": pattern, "range": range }), false).unwrap()
        };
        let no_end = json!({"type": "noEnd", "startDate": "2026-11-02"});
        assert_eq!(
            rule(
                json!({"type": "weekly", "interval": 2, "daysOfWeek": ["monday", "thursday"]}),
                no_end.clone()
            ),
            "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH"
        );
        assert_eq!(
            rule(
                json!({"type": "relativeMonthly", "interval": 1, "daysOfWeek": ["tuesday"], "index": "second"}),
                no_end.clone()
            ),
            "FREQ=MONTHLY;BYDAY=TU;BYSETPOS=2"
        );
        assert_eq!(
            rule(
                json!({"type": "absoluteYearly", "interval": 1, "month": 12, "dayOfMonth": 25}),
                json!({"type": "endDate", "endDate": "2030-12-25"})
            ),
            "FREQ=YEARLY;BYMONTH=12;BYMONTHDAY=25;UNTIL=20301225T235959Z"
        );
        assert!(recurrence_rule(&json!({"pattern": {"type": "hourly"}}), false).is_none());

        let second_tuesday = RepeatRule {
            freq: Freq::Monthly,
            interval: 1,
            by_day: vec!["TU".into()],
            month_day: None,
            month: None,
            set_pos: Some(2),
            count: Some(6),
            until: None,
        };
        let sent =
            recurrence_json(&second_tuesday, "2026-11-10", "E. Africa Standard Time").unwrap();
        assert_eq!(sent["pattern"]["type"], "relativeMonthly");
        assert_eq!(sent["pattern"]["index"], "second");
        assert_eq!(sent["pattern"]["daysOfWeek"][0], "tuesday");
        assert_eq!(sent["range"]["type"], "numbered");
        assert_eq!(sent["range"]["numberOfOccurrences"], 6);
        // Read back as the same rule.
        let back = recurrence_rule(&sent, false).unwrap();
        assert_eq!(back, "FREQ=MONTHLY;BYDAY=TU;BYSETPOS=2;COUNT=6");
        let weekly = RepeatRule {
            freq: Freq::Weekly,
            by_day: vec![],
            set_pos: None,
            count: None,
            until: Some(ts("2027-01-31T00:00:00Z")),
            ..second_tuesday.clone()
        };
        let sent = recurrence_json(&weekly, "2026-11-10", "UTC").unwrap();
        // No days chosen: the start's own weekday (10 November 2026 is a Tuesday).
        assert_eq!(sent["pattern"]["daysOfWeek"][0], "tuesday");
        assert_eq!(sent["range"]["endDate"], "2027-01-31");
        let last_day = RepeatRule {
            freq: Freq::Monthly,
            set_pos: None,
            by_day: vec![],
            month_day: Some(-1),
            ..second_tuesday
        };
        assert!(recurrence_json(&last_day, "2026-11-30", "UTC").is_err());
    }

    #[test]
    fn editor_events_become_graph_json() {
        let input = EventInput {
            title: "Design review".into(),
            location: "Office".into(),
            notes: "Bring the mock-ups".into(),
            start: ts("2026-11-18T06:00:00Z"),
            end: ts("2026-11-18T07:00:00Z"),
            time_zone: "Africa/Nairobi".into(),
            reminders: Some(vec![10, 30]),
            show_as: Some(ShowAs::Away),
            private: Some(true),
            attendees: Some(vec!["peter@client.example".into()]),
            ..EventInput::default()
        };
        let body = event_json(&input, None).unwrap();
        assert_eq!(body["start"]["dateTime"], "2026-11-18T09:00:00");
        assert_eq!(body["start"]["timeZone"], "E. Africa Standard Time");
        assert_eq!(body["showAs"], "oof");
        assert_eq!(body["sensitivity"], "private");
        assert_eq!(body["reminderMinutesBeforeStart"], 30);
        assert_eq!(
            body["attendees"][0]["emailAddress"]["address"],
            "peter@client.example"
        );
        assert!(body.get("recurrence").is_none());
        let day = EventInput {
            all_day: true,
            start: ts("2026-12-25T00:00:00Z"),
            end: ts("2026-12-26T00:00:00Z"),
            ..input
        };
        let body = event_json(&day, Some(None)).unwrap();
        assert_eq!(body["start"]["dateTime"], "2026-12-25T00:00:00");
        assert_eq!(body["start"]["timeZone"], "UTC");
        assert!(body["recurrence"].is_null());
    }

    #[test]
    fn get_schedule_mapping() {
        let reply: Value = serde_json::from_str(r#"{"value":[
            {"scheduleId":"peter@client.example","availabilityView":"0220","scheduleItems":[
                {"isPrivate":false,"status":"busy","subject":"Call","start":{"dateTime":"2026-11-18T06:00:00.0000000","timeZone":"UTC"},"end":{"dateTime":"2026-11-18T07:00:00.0000000","timeZone":"UTC"}},
                {"status":"tentative","start":{"dateTime":"2026-11-18T09:00:00.0000000","timeZone":"UTC"},"end":{"dateTime":"2026-11-18T09:30:00.0000000","timeZone":"UTC"}},
                {"status":"free","start":{"dateTime":"2026-11-18T10:00:00.0000000","timeZone":"UTC"},"end":{"dateTime":"2026-11-18T11:00:00.0000000","timeZone":"UTC"}}
            ]},
            {"scheduleId":"who@elsewhere.example","error":{"message":"The user is outside your organization.","responseCode":"ErrorFreeBusyGenerationFailed"}}
        ]}"#).unwrap();
        let emails = [
            "peter@client.example".to_owned(),
            "who@elsewhere.example".to_owned(),
        ];
        let out = schedule_from_json(&reply, &emails);
        let peter = out[0].1.as_ref().unwrap();
        assert_eq!(peter.len(), 2);
        assert_eq!(peter[0].start, ts("2026-11-18T06:00:00Z"));
        assert_eq!(peter[1].show_as, ShowAs::Tentative);
        assert!(out[1].1.is_none());
    }

    #[test]
    fn contacts_mapping() {
        let reply: Value = serde_json::from_str(r#"{"value":[
            {"@odata.etag":"W/\"EQAAABYA\"","id":"AAMkContact1","changeKey":"EQAAABYA","displayName":"Peter Mwangi","givenName":"Peter","surname":"Mwangi","emailAddresses":[{"name":"Peter Mwangi","address":"peter@client.example"}],"mobilePhone":"+254 700 000 001","businessPhones":["+254 20 000 0001"],"homePhones":[],"companyName":"Westlands Fit-out","jobTitle":"Site manager","personalNotes":"Gate code 44","categories":["Clients"]},
            {"id":"AAMkGone","@removed":{"reason":"deleted"}}
        ]}"#).unwrap();
        let (id, etag, card) = contact_from_json(&reply["value"][0]).unwrap();
        assert_eq!(id, "AAMkContact1");
        assert_eq!(etag.as_deref(), Some("EQAAABYA"));
        assert_eq!(card.name, "Peter Mwangi");
        assert_eq!(card.emails, ["peter@client.example"]);
        assert_eq!(card.phones, ["+254 700 000 001", "+254 20 000 0001"]);
        assert_eq!(card.organization, "Westlands Fit-out");
        assert_eq!(card.categories, ["Clients"]);
        let sent = contact_json(&card);
        assert_eq!(sent["givenName"], "Peter");
        assert_eq!(sent["surname"], "Mwangi");
        assert_eq!(sent["mobilePhone"], "+254 700 000 001");
        assert_eq!(sent["businessPhones"][0], "+254 20 000 0001");
        assert!(contact_from_json(&json!({"id": "x"})).is_none());
    }

    #[test]
    fn to_do_mapping() {
        let reply: Value = serde_json::from_str(r#"{"value":[
            {"@odata.etag":"W/\"xzyPKP0BiUGgld+lMKXwbQAAgdhkVw==\"","importance":"high","isReminderOn":true,"status":"notStarted","title":"Send the quote","categories":["Clients"],"id":"AAkALgAAAA1","body":{"content":"Kilimani fit-out","contentType":"text"},"dueDateTime":{"dateTime":"2026-11-19T21:00:00.0000000","timeZone":"UTC"},"reminderDateTime":{"dateTime":"2026-11-19T06:00:00.0000000","timeZone":"UTC"}},
            {"importance":"normal","isReminderOn":false,"status":"completed","title":"Call Peter","id":"AAkALgAAAA2","completedDateTime":{"dateTime":"2026-11-17T00:00:00.0000000","timeZone":"UTC"}}
        ]}"#).unwrap();
        let first = todo_from_json(&reply["value"][0]).unwrap();
        assert_eq!(first.fields.title, "Send the quote");
        assert_eq!(first.fields.importance, Importance::High);
        // Due "20 November" set in Nairobi arrives as 21:00 UTC the day before.
        assert_eq!(first.fields.due, Some(ts("2026-11-20T00:00:00Z")));
        assert_eq!(first.fields.reminder, Some(ts("2026-11-19T06:00:00Z")));
        assert_eq!(first.fields.categories, ["Clients"]);
        assert!(first.etag.is_some());
        let done = todo_from_json(&reply["value"][1]).unwrap();
        assert!(done.fields.completed);
        assert_eq!(done.fields.completed_at, Some(ts("2026-11-17T00:00:00Z")));
        let sent = todo_json(&first.fields);
        assert_eq!(sent["importance"], "high");
        assert_eq!(sent["status"], "notStarted");
        assert_eq!(sent["dueDateTime"]["dateTime"], "2026-11-20T00:00:00");
        assert_eq!(sent["isReminderOn"], true);
        assert_eq!(
            todo_json(&done.fields)["completedDateTime"]["dateTime"],
            "2026-11-17T00:00:00"
        );
    }

    #[test]
    fn automatic_replies_mapping() {
        let reply: Value = serde_json::from_str(r#"{"@odata.context":"https://graph.microsoft.com/v1.0/$metadata#users('x')/mailboxSettings/automaticRepliesSetting","status":"scheduled","externalAudience":"contactsOnly","internalReplyMessage":"<html><body><div>I'm away until Monday.<br>Call Peter if urgent.</div></body></html>","externalReplyMessage":"<html><body>Thanks &amp; regards</body></html>","scheduledStartDateTime":{"dateTime":"2026-12-19T21:00:00.0000000","timeZone":"UTC"},"scheduledEndDateTime":{"dateTime":"2027-01-02T21:00:00.0000000","timeZone":"UTC"}}"#).unwrap();
        let v = auto_replies_from_json(&reply, 3 * 3600);
        assert!(v.enabled);
        assert_eq!(v.message, "I'm away until Monday.\nCall Peter if urgent.");
        assert_eq!(v.external_message.as_deref(), Some("Thanks & regards"));
        assert_eq!(v.external_audience, Some(ExternalAudience::Contacts));
        assert_eq!(v.start.as_deref(), Some("2026-12-20"));
        assert_eq!(v.end.as_deref(), Some("2027-01-02"));
        let sent = auto_replies_json(&v, 3 * 3600, 0);
        assert_eq!(sent["status"], "scheduled");
        assert_eq!(sent["externalAudience"], "contactsOnly");
        assert_eq!(
            sent["scheduledStartDateTime"]["dateTime"],
            "2026-12-19T21:00:00"
        );
        assert_eq!(
            sent["scheduledEndDateTime"]["dateTime"],
            "2027-01-02T21:00:00"
        );
        assert!(
            sent["internalReplyMessage"]
                .as_str()
                .unwrap()
                .contains("Monday.<br>Call")
        );
        let always = Vacation {
            start: None,
            end: None,
            ..v.clone()
        };
        assert_eq!(auto_replies_json(&always, 0, 0)["status"], "alwaysEnabled");
        let off = Vacation {
            enabled: false,
            ..v
        };
        assert_eq!(auto_replies_json(&off, 0, 0)["status"], "disabled");
    }

    #[test]
    fn html_becomes_text() {
        assert_eq!(
            html_to_text(
                "<html><head><style>p{}</style></head><body><p>One &amp; two</p><p>Three</p></body></html>"
            ),
            "One & two\nThree"
        );
        assert_eq!(
            text_to_html("a <b>\nc"),
            "<html><body>a &lt;b&gt;<br>c</body></html>"
        );
    }

    #[test]
    fn setup_problems_are_named() {
        assert!(setup_problem(r#"{"error":{"code":"MailboxNotEnabledForRESTAPI"}}"#).is_some());
        assert!(setup_problem(r#"{"error":{"code":"ErrorItemNotFound"}}"#).is_none());
    }
}
