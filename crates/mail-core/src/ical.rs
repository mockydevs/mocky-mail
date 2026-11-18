//! Calendar events in iCalendar (RFC 5545): read the events in a calendar
//! resource, list their occurrences in a time window, and write new or
//! edited events.
//!
//! Repeating events are worked out here, with the `rrule` crate: servers
//! such as `SOGo` return a series as one event with a rule, not as its
//! occurrences. Times keep their time zone (`TZID`), so a weekly 9:00
//! meeting stays at 9:00 across daylight-saving changes.
//!
//! Editing keeps everything the app doesn't show (alarms, attendees,
//! other properties): only the fields the editor changes are replaced.

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Offset, TimeDelta, TimeZone, Utc};
use rrule::{RRule, RRuleSet, Tz, Unvalidated};
use serde::{Deserialize, Serialize};

use crate::vcard::{escape, fold, split_line, unescape, unfold};

/// Most occurrences one event lists in one window.
pub const MAX_OCCURRENCES: u16 = 1000;
/// Largest calendar resource read, bytes.
const MAX_BYTES: usize = 4 * 1024 * 1024;
/// Most lines read from one resource.
const MAX_LINES: usize = 50_000;
/// Deepest nesting read (`VCALENDAR` > `VEVENT` > `VALARM` ...).
const MAX_DEPTH: usize = 8;
const DAY: i64 = 86_400;
const PRODID: &str = "PRODID:-//Mocky Digital//Mocky Mail//EN";

/// How an event repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    /// Once.
    None,
    /// Every day.
    Daily,
    /// Every week on the same weekday.
    Weekly,
    /// Every month on the same day.
    Monthly,
    /// Every year on the same date.
    Yearly,
    /// A rule the editor can't show (every 2 weeks, until a date, ...).
    Custom,
    /// Editor only: leave the event's rule as it is.
    Keep,
}

/// Someone on an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    /// Email address.
    pub email: String,
    /// Display name, if given.
    pub name: Option<String>,
    /// Their answer: `accepted`, `declined`, `tentative`, `needs-action`.
    pub status: Option<String>,
}

/// One occurrence of an event: a single event, or one date of a series.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Occurrence {
    /// The event's `UID`.
    pub uid: String,
    /// For one date of a series: its original start (Unix seconds), which
    /// names it even if it was moved.
    pub recurrence_id: Option<i64>,
    /// `SUMMARY`.
    pub title: String,
    /// `LOCATION`.
    pub location: String,
    /// `DESCRIPTION`.
    pub notes: String,
    /// Unix seconds. All-day events: midnight UTC of the first day.
    pub start: i64,
    /// Unix seconds, exclusive. All-day events: midnight UTC after the last day.
    pub end: i64,
    /// No times, whole days only.
    pub all_day: bool,
    /// How the series repeats (`None` for a single event).
    pub repeat: Repeat,
    /// Part of a series.
    pub recurring: bool,
    /// Who organises it, for invitations.
    pub organizer: Option<Person>,
    /// Who is invited.
    pub attendees: Vec<Person>,
    /// `STATUS:TENTATIVE`.
    pub tentative: bool,
}

/// An event as typed in the editor.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventInput {
    /// Title.
    pub title: String,
    /// Place.
    pub location: String,
    /// Notes.
    pub notes: String,
    /// Whole days, no times.
    pub all_day: bool,
    /// Unix seconds. All-day: midnight UTC of the first day.
    pub start: i64,
    /// Unix seconds, exclusive. All-day: midnight UTC after the last day.
    pub end: i64,
    /// How it repeats; `Keep` leaves an edited event's rule alone.
    pub repeat: Repeat,
    /// The device's time zone (IANA name, such as `Africa/Nairobi`), so
    /// times stay put for people elsewhere and across daylight saving.
    pub time_zone: String,
}

// ---------------------------------------------------------------- reading

#[derive(Debug, Clone)]
struct Prop {
    name: String,
    params: String,
    value: String,
}

impl Prop {
    fn param(&self, key: &str) -> Option<String> {
        param_list(&self.params)
            .into_iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }
}

/// `TZID=Europe/Berlin;VALUE=DATE` as pairs, quotes removed.
fn param_list(params: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut quoted = false;
    let mut current = String::new();
    let mut push = |part: &str| {
        if let Some((k, v)) = part.split_once('=') {
            out.push((
                k.trim().to_ascii_uppercase(),
                v.trim().trim_matches('"').to_owned(),
            ));
        }
    };
    for c in params.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            ';' if !quoted => {
                push(&current);
                current.clear();
            }
            c => current.push(c),
        }
    }
    push(&current);
    out
}

#[derive(Debug, Clone, Default)]
struct Component {
    name: String,
    props: Vec<Prop>,
    children: Vec<Component>,
}

impl Component {
    fn prop(&self, name: &str) -> Option<&Prop> {
        self.props.iter().find(|p| p.name == name)
    }

    fn text(&self, name: &str) -> String {
        self.prop(name)
            .map(|p| unescape(&p.value).trim().to_owned())
            .unwrap_or_default()
    }

    fn cancelled(&self) -> bool {
        self.prop("STATUS")
            .is_some_and(|p| p.value.trim().eq_ignore_ascii_case("CANCELLED"))
    }
}

/// The resource's top-level components (normally one `VCALENDAR`).
/// Unbalanced or oversized input gives what was read so far.
fn parse(text: &str) -> Vec<Component> {
    if text.len() > MAX_BYTES {
        return Vec::new();
    }
    let unfolded = unfold(text);
    let mut stack: Vec<Component> = Vec::new();
    let mut top = Vec::new();
    for raw in unfolded.lines().take(MAX_LINES) {
        let Some(line) = split_line(raw) else {
            continue;
        };
        match line.name.as_str() {
            "BEGIN" => {
                if stack.len() >= MAX_DEPTH {
                    return top;
                }
                stack.push(Component {
                    name: line.value.trim().to_ascii_uppercase(),
                    ..Component::default()
                });
            }
            "END" => {
                let Some(done) = stack.pop() else {
                    continue;
                };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(done),
                    None => top.push(done),
                }
            }
            _ => {
                if let Some(current) = stack.last_mut() {
                    current.props.push(Prop {
                        name: line.name,
                        params: line.params.to_owned(),
                        value: line.value.to_owned(),
                    });
                }
            }
        }
    }
    top
}

fn vevents(text: &str) -> Vec<Component> {
    parse(text)
        .into_iter()
        .filter(|c| c.name == "VCALENDAR")
        .flat_map(|c| c.children)
        .filter(|c| c.name == "VEVENT")
        .collect()
}

/// A time zone from a `TZID`: an IANA name, a name with a vendor prefix
/// (`/mozilla.org/.../Europe/Berlin`), or a common Windows name.
fn zone_for(id: &str) -> Option<chrono_tz::Tz> {
    let id = id.trim().trim_matches('"');
    if let Ok(tz) = id.parse::<chrono_tz::Tz>() {
        return Some(tz);
    }
    let parts: Vec<&str> = id.split('/').filter(|p| !p.is_empty()).collect();
    for take in [3usize, 2] {
        if let Some(tail) = parts
            .len()
            .checked_sub(take)
            .and_then(|from| parts.get(from..))
            && let Ok(tz) = tail.join("/").parse::<chrono_tz::Tz>()
        {
            return Some(tz);
        }
    }
    let windows = match id {
        "E. Africa Standard Time" => "Africa/Nairobi",
        "South Africa Standard Time" => "Africa/Johannesburg",
        "W. Central Africa Standard Time" => "Africa/Lagos",
        "Egypt Standard Time" => "Africa/Cairo",
        "GMT Standard Time" => "Europe/London",
        "W. Europe Standard Time" => "Europe/Berlin",
        "Romance Standard Time" => "Europe/Paris",
        "Arabian Standard Time" => "Asia/Dubai",
        "India Standard Time" => "Asia/Kolkata",
        "China Standard Time" => "Asia/Shanghai",
        "Eastern Standard Time" => "America/New_York",
        "Central Standard Time" => "America/Chicago",
        "Pacific Standard Time" => "America/Los_Angeles",
        "UTC" | "Coordinated Universal Time" | "Greenwich Standard Time" => "UTC",
        _ => return None,
    };
    windows.parse().ok()
}

fn tz_of(prop: &Prop) -> Tz {
    prop.param("TZID")
        .and_then(|id| zone_for(&id))
        .map_or(Tz::LOCAL, Tz::Tz)
}

/// A date or date-time value in the given zone. Dates are midnight UTC.
fn parse_value(raw: &str, all_day: bool, tz: Tz) -> Option<DateTime<Tz>> {
    let raw = raw.trim();
    if all_day || raw.len() == 8 {
        let date = NaiveDate::parse_from_str(raw.get(..8)?, "%Y%m%d").ok()?;
        return Tz::UTC
            .from_utc_datetime(&date.and_hms_opt(0, 0, 0)?)
            .into();
    }
    if let Some(utc) = raw.strip_suffix(['Z', 'z']) {
        let at = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        return Some(Tz::UTC.from_utc_datetime(&at));
    }
    let at = NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M%S").ok()?;
    local_to(tz, at)
}

/// A wall-clock time in a zone; a time skipped by a clock change moves
/// an hour on.
fn local_to(tz: Tz, at: NaiveDateTime) -> Option<DateTime<Tz>> {
    tz.from_local_datetime(&at).earliest().or_else(|| {
        let later = at.checked_add_signed(TimeDelta::hours(1))?;
        tz.from_local_datetime(&later).earliest()
    })
}

fn is_date(prop: &Prop) -> bool {
    prop.param("VALUE")
        .is_some_and(|v| v.eq_ignore_ascii_case("DATE"))
        || prop.value.trim().len() == 8
}

fn when(prop: &Prop) -> Option<(DateTime<Tz>, bool)> {
    let all_day = is_date(prop);
    Some((parse_value(&prop.value, all_day, tz_of(prop))?, all_day))
}

/// An iCalendar duration (`PT1H30M`, `P1D`, `-PT15M`) in seconds.
fn parse_duration(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    let (negative, raw) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, raw.strip_prefix('+').unwrap_or(raw)),
    };
    let raw = raw.strip_prefix(['P', 'p'])?;
    let mut total: i64 = 0;
    let mut number = String::new();
    for c in raw.chars() {
        match c.to_ascii_uppercase() {
            'T' => {}
            d if d.is_ascii_digit() => number.push(d),
            unit => {
                let n: i64 = number.parse().ok()?;
                number.clear();
                let per = match unit {
                    'W' => 7 * DAY,
                    'D' => DAY,
                    'H' => 3600,
                    'M' => 60,
                    'S' => 1,
                    _ => return None,
                };
                total = total.checked_add(n.checked_mul(per)?)?;
            }
        }
    }
    if !number.is_empty() {
        return None;
    }
    Some(if negative {
        total.checked_neg()?
    } else {
        total
    })
}

fn person(prop: &Prop) -> Option<Person> {
    let value = prop.value.trim();
    let email = value
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("mailto:"))
        .and_then(|_| value.get(7..))
        .unwrap_or(value)
        .trim();
    if email.is_empty() || email.chars().any(char::is_control) || !email.contains('@') {
        return None;
    }
    Some(Person {
        email: email.to_owned(),
        name: prop.param("CN").filter(|n| !n.trim().is_empty()),
        status: prop.param("PARTSTAT").map(|s| s.to_ascii_lowercase()),
    })
}

/// How a rule repeats, in the editor's terms.
fn repeat_of(rule: &str) -> Repeat {
    let mut freq = None;
    for part in rule.split(';') {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => freq = Some(value.trim().to_ascii_uppercase()),
            "WKST" => {}
            "INTERVAL" if value.trim() == "1" => {}
            _ => return Repeat::Custom,
        }
    }
    match freq.as_deref() {
        Some("DAILY") => Repeat::Daily,
        Some("WEEKLY") => Repeat::Weekly,
        Some("MONTHLY") => Repeat::Monthly,
        Some("YEARLY") => Repeat::Yearly,
        _ => Repeat::Custom,
    }
}

/// An event's own fields, with its first start and its length.
struct Base {
    start: DateTime<Tz>,
    length: i64,
    occurrence: Occurrence,
}

fn base(event: &Component) -> Option<Base> {
    let (start, all_day) = when(event.prop("DTSTART")?)?;
    let begin = start.timestamp();
    let end = match (event.prop("DTEND"), event.prop("DURATION")) {
        (Some(end), _) => parse_value(&end.value, all_day, tz_of(end))
            .map(|e| e.timestamp())
            .filter(|e| *e >= begin),
        (None, Some(length)) => parse_duration(&length.value).and_then(|d| begin.checked_add(d)),
        (None, None) => None,
    }
    .unwrap_or(if all_day {
        begin.saturating_add(DAY)
    } else {
        begin
    });
    let organizer = event.prop("ORGANIZER").and_then(person);
    let attendees = event
        .props
        .iter()
        .filter(|p| p.name == "ATTENDEE")
        .filter_map(person)
        .collect();
    let rule = event.prop("RRULE").map(|p| p.value.trim().to_owned());
    Some(Base {
        start,
        length: end.saturating_sub(begin),
        occurrence: Occurrence {
            uid: event.text("UID"),
            recurrence_id: None,
            title: event.text("SUMMARY"),
            location: event.text("LOCATION"),
            notes: event.text("DESCRIPTION"),
            start: begin,
            end,
            all_day,
            repeat: rule.as_deref().map_or(Repeat::None, repeat_of),
            recurring: rule.is_some(),
            organizer,
            attendees,
            tentative: event
                .prop("STATUS")
                .is_some_and(|p| p.value.trim().eq_ignore_ascii_case("TENTATIVE")),
        },
    })
}

fn at(tz: Tz, seconds: i64) -> Option<DateTime<Tz>> {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .map(|t| t.with_timezone(&tz))
}

/// `UNTIL` in the form `rrule` accepts for this start: the rule as
/// written first, then with `UNTIL` in UTC.
fn rule_variants(rule: &str, start: &DateTime<Tz>) -> Vec<String> {
    let mut variants = vec![rule.to_owned()];
    let mut changed = false;
    let parts: Vec<String> = rule
        .split(';')
        .map(|part| {
            let Some((key, value)) = part.split_once('=') else {
                return part.to_owned();
            };
            if !key.trim().eq_ignore_ascii_case("UNTIL") {
                return part.to_owned();
            }
            let value = value.trim();
            let tz = start.timezone();
            let utc = if value.len() == 8 {
                // The whole last day counts.
                NaiveDate::parse_from_str(value, "%Y%m%d")
                    .ok()
                    .and_then(|d| d.and_hms_opt(23, 59, 59))
                    .and_then(|d| local_to(tz, d))
            } else if value.ends_with(['Z', 'z']) {
                None
            } else {
                NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S")
                    .ok()
                    .and_then(|d| local_to(tz, d))
            };
            match utc {
                Some(u) => {
                    changed = true;
                    format!("UNTIL={}", u.with_timezone(&Utc).format("%Y%m%dT%H%M%SZ"))
                }
                None => part.to_owned(),
            }
        })
        .collect();
    if changed {
        variants.push(parts.join(";"));
    }
    variants
}

fn rule_set(event: &Component, start: DateTime<Tz>) -> Option<RRuleSet> {
    let rule = event.prop("RRULE")?.value.trim().to_owned();
    let valid = rule_variants(&rule, &start)
        .into_iter()
        .find_map(|r| r.parse::<RRule<Unvalidated>>().ok()?.validate(start).ok())?;
    let mut set = RRuleSet::new(start).rrule(valid);
    for (name, add) in [("EXDATE", false), ("RDATE", true)] {
        for prop in event.props.iter().filter(|p| p.name == name) {
            let all_day = prop
                .param("VALUE")
                .is_some_and(|v| v.eq_ignore_ascii_case("DATE"));
            if prop
                .param("VALUE")
                .is_some_and(|v| v.eq_ignore_ascii_case("PERIOD"))
            {
                continue;
            }
            for value in prop.value.split(',') {
                if let Some(when) = parse_value(value, all_day, tz_of(prop)) {
                    set = if add {
                        set.rdate(when)
                    } else {
                        set.exdate(when)
                    };
                }
            }
        }
    }
    Some(set)
}

/// The starts of a repeating event between `from` and `to`, or `None`
/// when its rule can't be read (then only its first date shows).
fn starts_between(event: &Component, base: &Base, from: i64, to: i64) -> Option<Vec<i64>> {
    // Years 1 to 9999: what dates can hold.
    let (from, to) = (from.max(-62_135_596_800), to.min(253_402_300_799));
    let tz = base.start.timezone();
    let set = rule_set(event, base.start)?
        .after(at(tz, from.saturating_sub(base.length).saturating_sub(1))?)
        .before(at(tz, to)?);
    Some(
        set.all(MAX_OCCURRENCES)
            .dates
            .iter()
            .map(DateTime::timestamp)
            .collect(),
    )
}

fn overlaps(start: i64, end: i64, from: i64, to: i64) -> bool {
    start < to && (end > from || (end == start && start >= from))
}

/// Every occurrence of the events in `ics` that overlaps `from..to`
/// (Unix seconds). Cancelled events and dates are left out.
#[must_use]
pub fn occurrences(ics: &str, from: i64, to: i64) -> Vec<Occurrence> {
    let events = vevents(ics);
    let (masters, overrides): (Vec<_>, Vec<_>) = events
        .iter()
        .partition(|e| e.prop("RECURRENCE-ID").is_none());
    // Dates of a series changed or cancelled on their own.
    let moved: Vec<(String, i64)> = overrides
        .iter()
        .filter_map(|e| {
            let (rid, _) = when(e.prop("RECURRENCE-ID")?)?;
            Some((e.text("UID"), rid.timestamp()))
        })
        .collect();
    let mut out = Vec::new();
    for master in masters {
        if master.cancelled() {
            continue;
        }
        let Some(base) = base(master) else {
            continue;
        };
        let first = base.occurrence.start;
        if !base.occurrence.recurring {
            if overlaps(first, base.occurrence.end, from, to) {
                out.push(base.occurrence);
            }
            continue;
        }
        let starts = starts_between(master, &base, from, to).unwrap_or_else(|| vec![first]);
        for start in starts {
            let end = start.saturating_add(base.length);
            if !overlaps(start, end, from, to)
                || moved
                    .iter()
                    .any(|(uid, rid)| *uid == base.occurrence.uid && *rid == start)
            {
                continue;
            }
            out.push(Occurrence {
                recurrence_id: Some(start),
                start,
                end,
                ..base.occurrence.clone()
            });
        }
    }
    for event in overrides {
        if event.cancelled() {
            continue;
        }
        let (Some(base), Some((rid, _))) =
            (base(event), event.prop("RECURRENCE-ID").and_then(when))
        else {
            continue;
        };
        let mut occurrence = base.occurrence;
        if overlaps(occurrence.start, occurrence.end, from, to) {
            let series = events
                .iter()
                .find(|e| e.prop("RECURRENCE-ID").is_none() && e.text("UID") == occurrence.uid)
                .and_then(|e| e.prop("RRULE"))
                .map_or(Repeat::Custom, |r| repeat_of(&r.value));
            occurrence.recurrence_id = Some(rid.timestamp());
            occurrence.recurring = true;
            occurrence.repeat = series;
            out.push(occurrence);
        }
    }
    out.sort_by_key(|o| (o.start, o.end));
    out
}

/// The event itself (a series' first date and rule), for the editor.
#[must_use]
pub fn master(ics: &str) -> Option<Occurrence> {
    let events = vevents(ics);
    let event = events
        .iter()
        .find(|e| e.prop("RECURRENCE-ID").is_none())
        .or_else(|| events.first())?;
    base(event).map(|b| b.occurrence)
}

/// When the resource's events start and end, for finding it by date:
/// `(first start, last end)`, with `None` for a series without an end.
#[must_use]
pub fn span(ics: &str) -> (Option<i64>, Option<i64>) {
    let mut first: Option<i64> = None;
    let mut last: Option<i64> = Some(i64::MIN);
    for event in vevents(ics) {
        let Some(base) = base(&event) else {
            continue;
        };
        first = Some(first.map_or(base.occurrence.start, |f| f.min(base.occurrence.start)));
        let end = if base.occurrence.recurring && event.prop("RECURRENCE-ID").is_none() {
            let rule = event
                .prop("RRULE")
                .map(|p| p.value.to_ascii_uppercase())
                .unwrap_or_default();
            if rule.contains("COUNT=") || rule.contains("UNTIL=") {
                rule_set(&event, base.start).and_then(|set| {
                    let all = set.all(MAX_OCCURRENCES);
                    if all.limited {
                        None
                    } else {
                        all.dates
                            .last()
                            .map(|d| d.timestamp().saturating_add(base.length))
                            .or(Some(base.occurrence.end))
                    }
                })
            } else {
                None
            }
        } else {
            Some(base.occurrence.end)
        };
        last = match (last, end) {
            (Some(l), Some(e)) => Some(l.max(e)),
            _ => None,
        };
    }
    (first, last.filter(|l| *l != i64::MIN))
}

/// The resource's event `UID`.
#[must_use]
pub fn uid_of(ics: &str) -> Option<String> {
    vevents(ics)
        .first()
        .map(|e| e.text("UID"))
        .filter(|u| !u.is_empty())
}

// ---------------------------------------------------------------- writing

fn utc_stamp(seconds: i64) -> String {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .unwrap_or_default()
        .format("%Y%m%dT%H%M%SZ")
        .to_string()
}

fn utc_date(seconds: i64) -> String {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .unwrap_or_default()
        .format("%Y%m%d")
        .to_string()
}

/// `DTSTART`/`DTEND` lines for the editor's times, and the zone used.
fn time_lines(input: &EventInput) -> (Vec<String>, Option<chrono_tz::Tz>) {
    let end = input.end.max(if input.all_day {
        input.start.saturating_add(DAY)
    } else {
        input.start
    });
    if input.all_day {
        return (
            vec![
                format!("DTSTART;VALUE=DATE:{}", utc_date(input.start)),
                format!("DTEND;VALUE=DATE:{}", utc_date(end)),
            ],
            None,
        );
    }
    let zone = zone_for(&input.time_zone).filter(|z| *z != chrono_tz::Tz::UTC);
    let lines = match zone {
        Some(zone) => {
            let local = |s: i64| {
                Utc.timestamp_opt(s, 0)
                    .single()
                    .unwrap_or_default()
                    .with_timezone(&zone)
                    .format("%Y%m%dT%H%M%S")
                    .to_string()
            };
            vec![
                format!("DTSTART;TZID={}:{}", zone.name(), local(input.start)),
                format!("DTEND;TZID={}:{}", zone.name(), local(end)),
            ]
        }
        None => vec![
            format!("DTSTART:{}", utc_stamp(input.start)),
            format!("DTEND:{}", utc_stamp(end)),
        ],
    };
    (lines, zone)
}

fn rule_line(repeat: Repeat) -> Option<String> {
    let freq = match repeat {
        Repeat::Daily => "DAILY",
        Repeat::Weekly => "WEEKLY",
        Repeat::Monthly => "MONTHLY",
        Repeat::Yearly => "YEARLY",
        Repeat::None | Repeat::Custom | Repeat::Keep => return None,
    };
    Some(format!("RRULE:FREQ={freq}"))
}

fn offset_text(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let s = seconds.unsigned_abs();
    let (h, m, rest) = (s / 3600, (s % 3600) / 60, s % 60);
    if rest == 0 {
        format!("{sign}{h:02}{m:02}")
    } else {
        format!("{sign}{h:02}{m:02}{rest:02}")
    }
}

/// A `VTIMEZONE` for `zone`, with its clock changes from the year before
/// `year` to the year after, so other apps read the times the same way.
fn vtimezone(zone: chrono_tz::Tz, year: i32) -> Vec<String> {
    let offset_at = |t: NaiveDateTime| {
        let offset = zone.offset_from_utc_datetime(&t);
        (offset.fix().local_minus_utc(), offset.to_string())
    };
    let start = NaiveDate::from_ymd_opt(year.saturating_sub(1), 1, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .unwrap_or_default();
    let stop = NaiveDate::from_ymd_opt(year.saturating_add(2), 1, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .unwrap_or_default();
    let (mut previous, first_name) = offset_at(start);
    let mut changes = Vec::new();
    let mut t = start;
    while t < stop {
        let Some(next) = t.checked_add_signed(TimeDelta::minutes(15)) else {
            break;
        };
        let (offset, name) = offset_at(next);
        if offset != previous {
            changes.push((next, previous, offset, name));
            previous = offset;
        }
        t = next;
    }
    let mut lines = vec![
        "BEGIN:VTIMEZONE".to_owned(),
        format!("TZID:{}", zone.name()),
    ];
    if changes.is_empty() {
        let (offset, _) = offset_at(start);
        lines.extend([
            "BEGIN:STANDARD".to_owned(),
            "DTSTART:19700101T000000".to_owned(),
            format!("TZOFFSETFROM:{}", offset_text(offset)),
            format!("TZOFFSETTO:{}", offset_text(offset)),
            format!("TZNAME:{first_name}"),
            "END:STANDARD".to_owned(),
        ]);
    } else {
        let standard = changes.iter().map(|c| c.2).min().unwrap_or_default();
        for (when, from, to, name) in changes {
            let kind = if to > standard {
                "DAYLIGHT"
            } else {
                "STANDARD"
            };
            let local = when
                .checked_add_signed(TimeDelta::seconds(i64::from(from)))
                .unwrap_or(when);
            lines.extend([
                format!("BEGIN:{kind}"),
                format!("DTSTART:{}", local.format("%Y%m%dT%H%M%S")),
                format!("TZOFFSETFROM:{}", offset_text(from)),
                format!("TZOFFSETTO:{}", offset_text(to)),
                format!("TZNAME:{name}"),
                format!("END:{kind}"),
            ]);
        }
    }
    lines.push("END:VTIMEZONE".to_owned());
    lines
}

fn year_of(seconds: i64) -> i32 {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .unwrap_or_default()
        .year()
}

fn text_line(name: &str, value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| format!("{name}:{}", escape(value)))
}

fn finish(lines: &[String]) -> String {
    let mut out = String::new();
    for line in lines {
        fold(line, &mut out);
    }
    out
}

/// The editor's fields as event lines (after `BEGIN:VEVENT`).
fn field_lines(
    input: &EventInput,
    now: i64,
    sequence: u32,
) -> (Vec<String>, Option<chrono_tz::Tz>) {
    let (times, zone) = time_lines(input);
    let mut lines = vec![
        format!("DTSTAMP:{}", utc_stamp(now)),
        format!("LAST-MODIFIED:{}", utc_stamp(now)),
        format!("SEQUENCE:{sequence}"),
    ];
    lines.extend(times);
    lines.extend(text_line("SUMMARY", &input.title));
    lines.extend(text_line("LOCATION", &input.location));
    lines.extend(text_line("DESCRIPTION", &input.notes));
    (lines, zone)
}

/// A new calendar resource with one event.
#[must_use]
pub fn new_event(uid: &str, input: &EventInput, now: i64) -> String {
    let (fields, zone) = field_lines(input, now, 0);
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_owned(),
        "VERSION:2.0".to_owned(),
        PRODID.to_owned(),
        "CALSCALE:GREGORIAN".to_owned(),
    ];
    if let Some(zone) = zone {
        lines.extend(vtimezone(zone, year_of(input.start)));
    }
    lines.push("BEGIN:VEVENT".to_owned());
    lines.push(format!("UID:{}", escape(uid)));
    lines.push(format!("CREATED:{}", utc_stamp(now)));
    lines.extend(fields);
    lines.extend(rule_line(input.repeat));
    lines.push("END:VEVENT".to_owned());
    lines.push("END:VCALENDAR".to_owned());
    finish(&lines)
}

/// Where each `VEVENT` block is in a list of unfolded lines.
struct Block {
    begin: usize,
    end: usize,
    is_override: bool,
}

fn blocks(lines: &[&str]) -> Vec<Block> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut open: Option<(usize, bool)> = None;
    for (i, raw) in lines.iter().enumerate() {
        let Some(line) = split_line(raw) else {
            continue;
        };
        match line.name.as_str() {
            "BEGIN" => {
                depth = depth.saturating_add(1);
                if depth == 2 && line.value.trim().eq_ignore_ascii_case("VEVENT") {
                    open = Some((i, false));
                }
            }
            "END" => {
                if depth == 2
                    && let Some((begin, is_override)) = open.take()
                {
                    out.push(Block {
                        begin,
                        end: i,
                        is_override,
                    });
                }
                depth = depth.saturating_sub(1);
            }
            "RECURRENCE-ID" if depth == 2 => {
                if let Some((_, o)) = open.as_mut() {
                    *o = true;
                }
            }
            _ => {}
        }
    }
    out
}

/// Lines of a `VEVENT` (between `BEGIN` and `END`), with the event's own
/// properties named in `drop` removed (alarms and other parts are kept).
fn kept_lines<'a>(lines: &[&'a str], block: &Block, drop: &[&str]) -> Vec<&'a str> {
    let mut depth = 0usize;
    let mut out = Vec::new();
    for raw in lines
        .get(block.begin.saturating_add(1)..block.end)
        .unwrap_or_default()
    {
        let name = split_line(raw).map(|l| l.name).unwrap_or_default();
        match name.as_str() {
            "BEGIN" => depth = depth.saturating_add(1),
            "END" => depth = depth.saturating_sub(1),
            n if depth == 0 && drop.contains(&n) => continue,
            _ => {}
        }
        out.push(*raw);
    }
    out
}

/// Replace the event's fields with the editor's, keeping everything else.
/// `None` when `original` has no event.
#[must_use]
pub fn edit_event(original: &str, input: &EventInput, now: i64) -> Option<String> {
    let unfolded = unfold(original);
    let lines: Vec<&str> = unfolded.lines().filter(|l| !l.trim().is_empty()).collect();
    let found = blocks(&lines);
    let master = found
        .iter()
        .find(|b| !b.is_override)
        .or_else(|| found.first())?;
    let old = vevents(original)
        .into_iter()
        .find(|e| e.prop("RECURRENCE-ID").is_none());
    let sequence = old
        .as_ref()
        .and_then(|e| e.prop("SEQUENCE"))
        .and_then(|p| p.value.trim().parse::<u32>().ok())
        .unwrap_or(0)
        .saturating_add(1);
    let (fields, zone) = field_lines(input, now, sequence);
    let mut drop = vec![
        "SUMMARY",
        "LOCATION",
        "DESCRIPTION",
        "DTSTART",
        "DTEND",
        "DURATION",
        "DTSTAMP",
        "LAST-MODIFIED",
        "SEQUENCE",
    ];
    if input.repeat != Repeat::Keep {
        drop.push("RRULE");
        if input.repeat == Repeat::None {
            drop.extend(["EXDATE", "RDATE"]);
        }
    }
    let has_zone = |name: &str| {
        parse(original).iter().any(|cal| {
            cal.children
                .iter()
                .any(|c| c.name == "VTIMEZONE" && c.text("TZID") == name)
        })
    };
    let mut out: Vec<String> = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        if i == master.begin {
            if let Some(zone) = zone
                && !has_zone(zone.name())
                && found.first().is_some_and(|b| b.begin == i)
            {
                out.extend(vtimezone(zone, year_of(input.start)));
            }
            out.push((*raw).to_owned());
            out.extend(fields.iter().cloned());
            if input.repeat != Repeat::Keep {
                out.extend(rule_line(input.repeat));
            }
            out.extend(
                kept_lines(&lines, master, &drop)
                    .into_iter()
                    .map(str::to_owned),
            );
            continue;
        }
        if i > master.begin && i < master.end {
            continue;
        }
        // A zone for the first event goes before it, even when the master
        // comes after an override.
        if let Some(zone) = zone
            && !has_zone(zone.name())
            && master.begin != found.first().map_or(master.begin, |b| b.begin)
            && found.first().is_some_and(|b| b.begin == i)
        {
            out.extend(vtimezone(zone, year_of(input.start)));
        }
        out.push((*raw).to_owned());
    }
    Some(finish(&out))
}

/// Remove one date of a series: an `EXDATE` on the series, and that
/// date's own changed copy if it has one. `None` when there's no series.
#[must_use]
pub fn skip_occurrence(original: &str, recurrence_id: i64) -> Option<String> {
    let unfolded = unfold(original);
    let lines: Vec<&str> = unfolded.lines().filter(|l| !l.trim().is_empty()).collect();
    let found = blocks(&lines);
    let master = found.iter().find(|b| !b.is_override)?;
    let start = lines
        .get(master.begin..master.end)?
        .iter()
        .filter_map(|l| split_line(l))
        .find(|l| l.name == "DTSTART")
        .map(|l| Prop {
            name: l.name,
            params: l.params.to_owned(),
            value: l.value.to_owned(),
        })?;
    let exdate = if is_date(&start) {
        format!("EXDATE;VALUE=DATE:{}", utc_date(recurrence_id))
    } else if start.value.trim().ends_with(['Z', 'z']) {
        format!("EXDATE:{}", utc_stamp(recurrence_id))
    } else {
        let tz = tz_of(&start);
        let local = at(tz, recurrence_id)?.format("%Y%m%dT%H%M%S").to_string();
        match start.param("TZID") {
            Some(id) if !tz.is_local() => format!("EXDATE;TZID={id}:{local}"),
            _ => format!("EXDATE:{local}"),
        }
    };
    // Overrides for that date are dropped.
    let skipped: Vec<&Block> = found
        .iter()
        .filter(|b| {
            b.is_override
                && lines
                    .get(b.begin..b.end)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|l| split_line(l))
                    .find(|l| l.name == "RECURRENCE-ID")
                    .and_then(|l| {
                        when(&Prop {
                            name: l.name,
                            params: l.params.to_owned(),
                            value: l.value.to_owned(),
                        })
                    })
                    .is_some_and(|(rid, _)| rid.timestamp() == recurrence_id)
        })
        .collect();
    let mut out = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        if skipped.iter().any(|b| i >= b.begin && i <= b.end) {
            continue;
        }
        out.push((*raw).to_owned());
        if i == master.begin {
            out.push(exdate.clone());
        }
    }
    Some(finish(&out))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn ts(text: &str) -> i64 {
        DateTime::parse_from_rfc3339(text).unwrap().timestamp()
    }

    fn cal(body: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}END:VCALENDAR\r\n")
    }

    #[test]
    fn reads_a_single_event() {
        let ics = cal(
            "BEGIN:VEVENT\r\nUID:a1\r\nDTSTART:20261118T060000Z\r\nDTEND:20261118T070000Z\r\nSUMMARY:Client call\\, Kamau\r\nLOCATION:Westlands\r\nDESCRIPTION:Line one\\nLine two\r\nORGANIZER;CN=Don:mailto:don@mocky.example\r\nATTENDEE;CN=\"Peter M\";PARTSTAT=ACCEPTED:mailto:peter@client.example\r\nBEGIN:VALARM\r\nTRIGGER:-PT15M\r\nACTION:DISPLAY\r\nEND:VALARM\r\nEND:VEVENT\r\n",
        );
        let all = occurrences(&ics, ts("2026-11-01T00:00:00Z"), ts("2026-12-01T00:00:00Z"));
        assert_eq!(all.len(), 1);
        let e = &all[0];
        assert_eq!(e.title, "Client call, Kamau");
        assert_eq!(e.notes, "Line one\nLine two");
        assert_eq!(e.start, ts("2026-11-18T06:00:00Z"));
        assert_eq!(e.end, ts("2026-11-18T07:00:00Z"));
        assert!(!e.recurring && !e.all_day);
        assert_eq!(e.organizer.as_ref().unwrap().name.as_deref(), Some("Don"));
        assert_eq!(e.attendees[0].name.as_deref(), Some("Peter M"));
        assert_eq!(e.attendees[0].status.as_deref(), Some("accepted"));
        assert!(
            occurrences(&ics, ts("2026-12-01T00:00:00Z"), ts("2027-01-01T00:00:00Z")).is_empty()
        );
    }

    #[test]
    fn time_zones_and_all_day_events() {
        let ics = cal(
            "BEGIN:VEVENT\r\nUID:b\r\nDTSTART;TZID=Africa/Nairobi:20261118T090000\r\nDURATION:PT1H30M\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:c\r\nDTSTART;VALUE=DATE:20261225\r\nEND:VEVENT\r\n",
        );
        let all = occurrences(&ics, ts("2026-11-01T00:00:00Z"), ts("2027-01-01T00:00:00Z"));
        assert_eq!(all[0].start, ts("2026-11-18T06:00:00Z"));
        assert_eq!(all[0].end, ts("2026-11-18T07:30:00Z"));
        assert!(all[1].all_day);
        assert_eq!(all[1].start, ts("2026-12-25T00:00:00Z"));
        assert_eq!(all[1].end, ts("2026-12-26T00:00:00Z"));
        // Vendor-prefixed and Windows zone names.
        assert_eq!(
            zone_for("/mozilla.org/20070129_1/Europe/Berlin"),
            Some(chrono_tz::Tz::Europe__Berlin)
        );
        assert_eq!(
            zone_for("E. Africa Standard Time"),
            Some(chrono_tz::Tz::Africa__Nairobi)
        );
    }

    #[test]
    fn weekly_series_keep_local_time_across_clock_changes() {
        let ics = cal(
            "BEGIN:VEVENT\r\nUID:w\r\nDTSTART;TZID=Europe/London:20261020T090000\r\nDTEND;TZID=Europe/London:20261020T100000\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\nEND:VEVENT\r\n",
        );
        let all = occurrences(&ics, ts("2026-10-01T00:00:00Z"), ts("2026-12-01T00:00:00Z"));
        let starts: Vec<i64> = all.iter().map(|o| o.start).collect();
        // 9:00 BST is 08:00 UTC; after the change 9:00 GMT is 09:00 UTC.
        assert_eq!(
            starts,
            [
                ts("2026-10-20T08:00:00Z"),
                ts("2026-10-27T09:00:00Z"),
                ts("2026-11-03T09:00:00Z")
            ]
        );
        // A count makes it a rule the editor keeps as it is.
        assert!(
            all.iter()
                .all(|o| o.recurring && o.repeat == Repeat::Custom)
        );
        assert_eq!(all[1].recurrence_id, Some(ts("2026-10-27T09:00:00Z")));
        let (first, last) = span(&ics);
        assert_eq!(first, Some(ts("2026-10-20T08:00:00Z")));
        assert_eq!(last, Some(ts("2026-11-03T10:00:00Z")));
    }

    #[test]
    fn exceptions_moved_dates_and_open_series() {
        let ics = cal(
            "BEGIN:VEVENT\r\nUID:d\r\nDTSTART:20261102T070000Z\r\nDTEND:20261102T073000Z\r\nRRULE:FREQ=DAILY\r\nEXDATE:20261103T070000Z\r\nSUMMARY:Stand-up\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:d\r\nRECURRENCE-ID:20261104T070000Z\r\nDTSTART:20261104T100000Z\r\nDTEND:20261104T103000Z\r\nSUMMARY:Stand-up (moved)\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:d\r\nRECURRENCE-ID:20261105T070000Z\r\nDTSTART:20261105T070000Z\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n",
        );
        let all = occurrences(&ics, ts("2026-11-02T00:00:00Z"), ts("2026-11-07T00:00:00Z"));
        let titles: Vec<(&str, i64)> = all.iter().map(|o| (o.title.as_str(), o.start)).collect();
        assert_eq!(
            titles,
            [
                ("Stand-up", ts("2026-11-02T07:00:00Z")),
                ("Stand-up (moved)", ts("2026-11-04T10:00:00Z")),
                ("Stand-up", ts("2026-11-06T07:00:00Z")),
            ]
        );
        assert_eq!(all[1].recurrence_id, Some(ts("2026-11-04T07:00:00Z")));
        assert_eq!(span(&ics).1, None);
        assert_eq!(repeat_of("FREQ=WEEKLY;INTERVAL=2"), Repeat::Custom);
        assert_eq!(repeat_of("FREQ=MONTHLY;INTERVAL=1"), Repeat::Monthly);
    }

    #[test]
    fn until_as_a_date_or_local_time_is_read() {
        let ics = cal(
            "BEGIN:VEVENT\r\nUID:u\r\nDTSTART;VALUE=DATE:20261101\r\nRRULE:FREQ=DAILY;UNTIL=20261103\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:v\r\nDTSTART;TZID=Africa/Nairobi:20261101T090000\r\nRRULE:FREQ=DAILY;UNTIL=20261102T090000\r\nEND:VEVENT\r\n",
        );
        let all = occurrences(&ics, ts("2026-10-01T00:00:00Z"), ts("2026-12-01T00:00:00Z"));
        assert_eq!(all.iter().filter(|o| o.uid == "u").count(), 3);
        assert_eq!(all.iter().filter(|o| o.uid == "v").count(), 2);
    }

    fn input(repeat: Repeat) -> EventInput {
        EventInput {
            title: "Design review".into(),
            location: "Office; room 2".into(),
            notes: "Bring the mock-ups".into(),
            all_day: false,
            start: ts("2026-11-18T06:00:00Z"),
            end: ts("2026-11-18T07:00:00Z"),
            repeat,
            time_zone: "Africa/Nairobi".into(),
        }
    }

    #[test]
    fn writes_events_that_read_back_the_same() {
        let now = ts("2026-11-17T12:00:00Z");
        let ics = new_event("uid-1@mocky.mail", &input(Repeat::Weekly), now);
        assert!(ics.contains("DTSTART;TZID=Africa/Nairobi:20261118T090000\r\n"));
        assert!(ics.contains("BEGIN:VTIMEZONE\r\nTZID:Africa/Nairobi\r\n"));
        assert!(ics.contains("TZOFFSETTO:+0300\r\n"));
        assert!(ics.contains("LOCATION:Office\\; room 2\r\n"));
        assert!(ics.contains("RRULE:FREQ=WEEKLY\r\n"));
        let back = occurrences(&ics, ts("2026-11-18T00:00:00Z"), ts("2026-11-26T00:00:00Z"));
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].title, "Design review");
        assert_eq!(back[0].location, "Office; room 2");
        assert_eq!(back[1].start, ts("2026-11-25T06:00:00Z"));
        assert_eq!(uid_of(&ics).as_deref(), Some("uid-1@mocky.mail"));

        let day = EventInput {
            all_day: true,
            start: ts("2026-12-25T00:00:00Z"),
            end: ts("2026-12-26T00:00:00Z"),
            ..input(Repeat::Yearly)
        };
        let ics = new_event("uid-2", &day, now);
        assert!(ics.contains("DTSTART;VALUE=DATE:20261225\r\n"));
        assert!(!ics.contains("VTIMEZONE"));
        let next_year = occurrences(&ics, ts("2027-12-01T00:00:00Z"), ts("2028-01-01T00:00:00Z"));
        assert_eq!(next_year[0].start, ts("2027-12-25T00:00:00Z"));
    }

    #[test]
    fn zones_with_clock_changes_get_both_offsets() {
        let ics = new_event(
            "u",
            &EventInput {
                time_zone: "Europe/London".into(),
                ..input(Repeat::None)
            },
            0,
        );
        assert!(ics.contains("BEGIN:DAYLIGHT\r\n"));
        assert!(ics.contains("TZOFFSETTO:+0100\r\n"));
        assert!(ics.contains("TZOFFSETTO:+0000\r\n"));
        // An unknown zone falls back to UTC times.
        let utc = new_event(
            "u",
            &EventInput {
                time_zone: "Mars/Olympus".into(),
                ..input(Repeat::None)
            },
            0,
        );
        assert!(utc.contains("DTSTART:20261118T060000Z\r\n"));
    }

    #[test]
    fn editing_keeps_what_the_app_does_not_show() {
        let original = cal(
            "BEGIN:VEVENT\r\nUID:e1\r\nSEQUENCE:2\r\nDTSTART:20261118T060000Z\r\nDTEND:20261118T070000Z\r\nSUMMARY:Old\r\nRRULE:FREQ=WEEKLY;INTERVAL=2\r\nATTENDEE;CN=Peter:mailto:peter@client.example\r\nX-CUSTOM:kept\r\nBEGIN:VALARM\r\nTRIGGER:-PT15M\r\nACTION:DISPLAY\r\nDESCRIPTION:Reminder\r\nEND:VALARM\r\nEND:VEVENT\r\n",
        );
        let edited = edit_event(&original, &input(Repeat::Keep), 0).unwrap();
        assert!(edited.contains("SUMMARY:Design review\r\n"));
        assert!(!edited.contains("SUMMARY:Old"));
        assert!(edited.contains("SEQUENCE:3\r\n"));
        assert!(edited.contains("RRULE:FREQ=WEEKLY;INTERVAL=2\r\n"));
        assert!(edited.contains("X-CUSTOM:kept\r\n"));
        assert!(edited.contains("ATTENDEE;CN=Peter:mailto:peter@client.example\r\n"));
        // The alarm's own DESCRIPTION is not the event's.
        assert!(edited.contains("DESCRIPTION:Reminder\r\n"));
        assert!(edited.contains("BEGIN:VTIMEZONE"));
        assert_eq!(edited.matches("BEGIN:VEVENT").count(), 1);
        assert_eq!(master(&edited).unwrap().repeat, Repeat::Custom);

        let once = edit_event(&original, &input(Repeat::None), 0).unwrap();
        assert!(!once.contains("RRULE"));
        assert!(!master(&once).unwrap().recurring);
    }

    #[test]
    fn skipping_one_date_of_a_series() {
        let original = cal(
            "BEGIN:VEVENT\r\nUID:s\r\nDTSTART;TZID=Africa/Nairobi:20261102T090000\r\nDTEND;TZID=Africa/Nairobi:20261102T093000\r\nRRULE:FREQ=DAILY;COUNT=4\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:s\r\nRECURRENCE-ID;TZID=Africa/Nairobi:20261103T090000\r\nDTSTART;TZID=Africa/Nairobi:20261103T110000\r\nSUMMARY:moved\r\nEND:VEVENT\r\n",
        );
        let skip = ts("2026-11-03T06:00:00Z");
        let edited = skip_occurrence(&original, skip).unwrap();
        assert!(edited.contains("EXDATE;TZID=Africa/Nairobi:20261103T090000\r\n"));
        assert!(!edited.contains("moved"));
        let left = occurrences(
            &edited,
            ts("2026-11-01T00:00:00Z"),
            ts("2026-12-01T00:00:00Z"),
        );
        assert_eq!(left.len(), 3);
        assert!(left.iter().all(|o| o.start != skip));
        assert!(
            skip_occurrence(
                &cal("BEGIN:VEVENT\r\nRECURRENCE-ID:20261103T090000Z\r\nEND:VEVENT\r\n"),
                skip
            )
            .is_none()
        );
    }

    #[test]
    fn never_panics_on_odd_input() {
        for text in [
            "",
            "BEGIN:VCALENDAR",
            "END:VEVENT\nEND:VCALENDAR",
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:garbage\nRRULE:FREQ=NEVER\nEND:VEVENT\nEND:VCALENDAR",
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:20261101T000000Z\nRRULE:FREQ=SECONDLY\nEND:VEVENT\nEND:VCALENDAR",
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:20261101T000000Z\nDURATION:P99999999999999W\nEND:VEVENT\nEND:VCALENDAR",
        ] {
            let _ = occurrences(text, 0, i64::MAX);
            let _ = span(text);
            let _ = master(text);
            let _ = edit_event(text, &input(Repeat::Keep), 0);
            let _ = skip_occurrence(text, 0);
        }
        assert_eq!(parse_duration("P1DT2H"), Some(93_600));
        assert_eq!(parse_duration("-PT15M"), Some(-900));
        assert_eq!(parse_duration("PT"), Some(0));
        assert_eq!(parse_duration("P1Q"), None);
        // A runaway series stops at the limit.
        let busy = cal(
            "BEGIN:VEVENT\r\nUID:x\r\nDTSTART:20260101T000000Z\r\nRRULE:FREQ=MINUTELY\r\nEND:VEVENT\r\n",
        );
        assert!(
            occurrences(&busy, 0, ts("2027-01-01T00:00:00Z")).len() <= usize::from(MAX_OCCURRENCES)
        );
    }
}
