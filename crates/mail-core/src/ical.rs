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

use chrono::{
    DateTime, Datelike, NaiveDate, NaiveDateTime, Offset, TimeDelta, TimeZone, Utc, Weekday,
};
use rrule::{RRule, RRuleSet, Tz, Unvalidated};
use serde::{Deserialize, Serialize};

use crate::importance::Importance;
use crate::tasks::TaskFields;
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    /// Once.
    #[default]
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

/// How often a custom rule repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Freq {
    /// Every N days.
    Daily,
    /// Every N weeks.
    Weekly,
    /// Every N months.
    Monthly,
    /// Every N years.
    Yearly,
}

impl Freq {
    const fn rule_name(self) -> &'static str {
        match self {
            Self::Daily => "DAILY",
            Self::Weekly => "WEEKLY",
            Self::Monthly => "MONTHLY",
            Self::Yearly => "YEARLY",
        }
    }
}

/// A repeat rule (`RRULE`) in the terms of the repeat dialog. Rules it
/// can't hold (hourly, several days of the month, week numbers, ...) are
/// not turned into one; those series show as `custom` and keep their rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepeatRule {
    /// Days, weeks, months or years.
    pub freq: Freq,
    /// Every N of them (1 or more).
    pub interval: u32,
    /// Weekdays (`MO`..`SU`): for weekly, or one with `set_pos`.
    #[serde(default)]
    pub by_day: Vec<String>,
    /// Day of the month (1 to 31, or -1 for the last).
    #[serde(default)]
    pub month_day: Option<i32>,
    /// Month (1 to 12), for yearly.
    #[serde(default)]
    pub month: Option<u32>,
    /// Which one of `by_day` in the month: 1 to 4, or -1 (the last).
    #[serde(default)]
    pub set_pos: Option<i32>,
    /// Ends after this many times.
    #[serde(default)]
    pub count: Option<u32>,
    /// Ends on this day (Unix seconds: midnight UTC of the day).
    #[serde(default)]
    pub until: Option<i64>,
}

/// How an event shows you in free/busy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShowAs {
    /// Busy.
    #[default]
    Busy,
    /// Free (`TRANSP:TRANSPARENT`).
    Free,
    /// Tentative.
    Tentative,
    /// Away (out of office).
    Away,
}

impl ShowAs {
    /// The `X-MICROSOFT-CDO-BUSYSTATUS` value.
    const fn cdo(self) -> &'static str {
        match self {
            Self::Busy => "BUSY",
            Self::Free => "FREE",
            Self::Tentative => "TENTATIVE",
            Self::Away => "OOF",
        }
    }
}

/// Which dates of a series an edit is for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Only the date named by `occurrence`.
    This,
    /// The whole series.
    #[default]
    All,
}

/// Someone on an event.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[allow(
    clippy::struct_excessive_bools,
    reason = "the event's own yes/no fields"
)]
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
    /// Reminders: minutes before the start (0: at the start), soonest last.
    pub reminders: Vec<u32>,
    /// How it shows you in free/busy.
    pub show_as: ShowAs,
    /// `CLASS:PRIVATE` (or `CONFIDENTIAL`).
    pub private: bool,
    /// The series' rule as the repeat dialog shows it; `None` when not
    /// repeating or when the rule is one it can't show.
    pub repeat_rule: Option<RepeatRule>,
    /// You organise it: there is no organiser, or it is you. Read without
    /// knowing who "you" are, this is `organizer.is_none()`; see
    /// [`Occurrence::mark_organizer`].
    pub is_organizer: bool,
}

impl Occurrence {
    /// Set `is_organizer` for the account `me`.
    pub fn mark_organizer(&mut self, me: &str) {
        self.is_organizer = self
            .organizer
            .as_ref()
            .is_none_or(|p| same_address(&p.email, me));
    }
}

/// An event as typed in the editor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
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
    /// Reminders, minutes before the start. `None` leaves an edited event's
    /// reminders as they are.
    #[serde(default)]
    pub reminders: Option<Vec<u32>>,
    /// A custom rule, used instead of `repeat` when set.
    #[serde(default)]
    pub repeat_rule: Option<RepeatRule>,
    /// Free/busy; `None` leaves an edited event's as it is.
    #[serde(default)]
    pub show_as: Option<ShowAs>,
    /// Private; `None` leaves an edited event's as it is.
    #[serde(default)]
    pub private: Option<bool>,
    /// Invite these addresses; `None` leaves the attendees as they are.
    #[serde(default)]
    pub attendees: Option<Vec<String>>,
    /// Send invitations, updates or cancellations to the attendees.
    #[serde(default)]
    pub notify_attendees: bool,
    /// Editing one date of a series: its `recurrence_id`.
    #[serde(default)]
    pub occurrence: Option<i64>,
    /// With `occurrence`: only that date, or the whole series.
    #[serde(default)]
    pub scope: Scope,
    /// Who you are, for the `ORGANIZER` line added when inviting people.
    /// Set by the app, never by the interface.
    #[serde(skip)]
    pub organizer: Option<Person>,
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

/// The resource's events and its own time zone definitions.
fn calendar(text: &str) -> (Vec<Component>, Zones) {
    let mut events = Vec::new();
    let mut zones = Vec::new();
    for child in parse(text)
        .into_iter()
        .filter(|c| c.name == "VCALENDAR")
        .flat_map(|c| c.children)
    {
        match child.name.as_str() {
            "VEVENT" => events.push(child),
            "VTIMEZONE" => zones.push(child),
            _ => {}
        }
    }
    (events, Zones(zones))
}

fn vevents(text: &str) -> Vec<Component> {
    calendar(text).0
}

/// The calendar's own zone definitions (`VTIMEZONE`), for a `TZID` that is
/// neither an IANA name nor a known Windows one (Exchange writes
/// "Customized Time Zone", other servers their own labels).
#[derive(Debug, Default)]
struct Zones(Vec<Component>);

impl Zones {
    /// Seconds east of UTC in zone `id` at the wall-clock time `at`: the
    /// `TZOFFSETTO` of the observance that started last before it. A time
    /// before every observance gets the standard one.
    fn offset(&self, id: &str, at: NaiveDateTime) -> Option<i32> {
        let zone = self.0.iter().find(|z| z.text("TZID") == id.trim())?;
        // (start, is standard, offset) of the best so far.
        let mut chosen: Option<(NaiveDateTime, bool, i32)> = None;
        let mut fallback: Option<(bool, i32)> = None;
        for observance in zone
            .children
            .iter()
            .filter(|c| c.name == "STANDARD" || c.name == "DAYLIGHT")
        {
            let Some(offset) = observance
                .prop("TZOFFSETTO")
                .and_then(|p| parse_offset(&p.value))
            else {
                continue;
            };
            let standard = observance.name == "STANDARD";
            if fallback.is_none_or(|(s, _)| standard && !s) {
                fallback = Some((standard, offset));
            }
            if let Some(from) = onset(observance, at)
                && chosen.is_none_or(|(best, s, _)| from > best || (from == best && standard && !s))
            {
                chosen = Some((from, standard, offset));
            }
        }
        chosen.map(|(_, _, o)| o).or(fallback.map(|(_, o)| o))
    }
}

/// A wall-clock value (`20261118T090000`, a trailing `Z` ignored).
fn naive(raw: &str) -> Option<NaiveDateTime> {
    let raw = raw.trim();
    let raw = raw.strip_suffix(['Z', 'z']).unwrap_or(raw);
    NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M%S").ok()
}

/// `+0300`, `-0330` or `+053000` in seconds.
fn parse_offset(raw: &str) -> Option<i32> {
    let raw = raw.trim();
    let (sign, digits) = match raw.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, raw.strip_prefix('+').unwrap_or(raw)),
    };
    if !matches!(digits.len(), 4 | 6) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let part = |range: std::ops::Range<usize>| digits.get(range)?.parse::<i32>().ok();
    let seconds = if digits.len() == 6 { part(4..6)? } else { 0 };
    part(0..2)?
        .checked_mul(3600)?
        .checked_add(part(2..4)?.checked_mul(60)?)?
        .checked_add(seconds)?
        .checked_mul(sign)
}

fn weekday_of(code: &str) -> Option<Weekday> {
    Some(match code.to_ascii_uppercase().as_str() {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

fn weekday_code(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "MO",
        Weekday::Tue => "TU",
        Weekday::Wed => "WE",
        Weekday::Thu => "TH",
        Weekday::Fri => "FR",
        Weekday::Sat => "SA",
        Weekday::Sun => "SU",
    }
}

/// The `n`th `weekday` of a month; negative `n` counts from its end.
fn nth_weekday(year: i32, month: u32, weekday: Weekday, n: i32) -> Option<NaiveDate> {
    if n > 0 {
        return NaiveDate::from_weekday_of_month_opt(year, month, weekday, u8::try_from(n).ok()?);
    }
    if !(-5..0).contains(&n) {
        return None;
    }
    let (next_year, next_month) = if month == 12 {
        (year.checked_add(1)?, 1)
    } else {
        (year, month.checked_add(1)?)
    };
    let last_day = NaiveDate::from_ymd_opt(next_year, next_month, 1)?.pred_opt()?;
    let back = last_day
        .weekday()
        .num_days_from_monday()
        .checked_add(7)?
        .checked_sub(weekday.num_days_from_monday())?
        % 7;
    let weeks = i64::from(n.checked_neg()?.checked_sub(1)?);
    last_day.checked_sub_signed(TimeDelta::days(
        i64::from(back).checked_add(weeks.checked_mul(7)?)?,
    ))
}

/// When a yearly observance rule (`FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU`, the
/// form zone definitions use) starts in `year`. `None` for other rules.
fn yearly_onset(rule: &str, start: NaiveDateTime, year: i32) -> Option<NaiveDateTime> {
    let mut yearly = false;
    let mut month = start.month();
    let mut by_day: Option<(i32, Weekday)> = None;
    let mut month_days: Vec<u32> = Vec::new();
    for part in rule.split(';') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => yearly = value.eq_ignore_ascii_case("YEARLY"),
            "INTERVAL" if value != "1" => return None,
            "BYMONTH" => month = value.parse().ok()?,
            "BYDAY" => {
                let split = value.len().checked_sub(2)?;
                let (n, day) = (value.get(..split)?, value.get(split..)?);
                let n = if n.is_empty() { 0 } else { n.parse().ok()? };
                by_day = Some((n, weekday_of(day)?));
            }
            "BYMONTHDAY" => {
                month_days = value
                    .split(',')
                    .filter_map(|d| d.trim().parse().ok())
                    .collect();
            }
            _ => {}
        }
    }
    if !yearly {
        return None;
    }
    let date = match by_day {
        // "The Sunday among the 8th to the 14th".
        Some((0, weekday)) => month_days
            .iter()
            .filter_map(|d| NaiveDate::from_ymd_opt(year, month, *d))
            .find(|d| d.weekday() == weekday)?,
        Some((n, weekday)) => nth_weekday(year, month, weekday, n)?,
        None => NaiveDate::from_ymd_opt(
            year,
            month,
            month_days.first().copied().unwrap_or(start.day()),
        )?,
    };
    Some(date.and_time(start.time()))
}

/// The last time an observance (`STANDARD`/`DAYLIGHT`) started at or
/// before `at`; `None` when it only starts later.
fn onset(observance: &Component, at: NaiveDateTime) -> Option<NaiveDateTime> {
    let start = observance
        .prop("DTSTART")
        .and_then(|p| naive(&p.value))
        .unwrap_or(NaiveDateTime::MIN);
    if start > at {
        return None;
    }
    let mut best = start;
    for prop in observance.props.iter().filter(|p| p.name == "RDATE") {
        for when in prop.value.split(',').filter_map(naive) {
            if when <= at && when > best {
                best = when;
            }
        }
    }
    if let Some(rule) = observance.prop("RRULE") {
        // UNTIL is in UTC and the starts are wall-clock: a day's slack.
        let until = rule
            .value
            .split(';')
            .filter_map(|part| part.split_once('='))
            .find(|(key, _)| key.trim().eq_ignore_ascii_case("UNTIL"))
            .and_then(|(_, value)| {
                naive(value).or_else(|| {
                    NaiveDate::parse_from_str(value.trim(), "%Y%m%d")
                        .ok()?
                        .and_hms_opt(0, 0, 0)
                })
            })
            .and_then(|u| u.checked_add_signed(TimeDelta::days(1)));
        let mut years = vec![at.year(), at.year().saturating_sub(1)];
        if let Some(until) = until {
            years.extend([until.year(), until.year().saturating_sub(1)]);
        }
        for year in years {
            if let Some(when) = yearly_onset(&rule.value, start, year)
                && when >= start
                && when <= at
                && until.is_none_or(|u| when <= u)
                && when > best
            {
                best = when;
            }
        }
    }
    Some(best)
}

/// Zones half or three quarters of an hour off, to stand in for such an
/// offset from a `VTIMEZONE`: those that keep one offset all year first.
const ODD_OFFSET_ZONES: &[chrono_tz::Tz] = &[
    chrono_tz::Tz::Asia__Kolkata,
    chrono_tz::Tz::Asia__Kathmandu,
    chrono_tz::Tz::Asia__Kabul,
    chrono_tz::Tz::Asia__Yangon,
    chrono_tz::Tz::Australia__Darwin,
    chrono_tz::Tz::Australia__Eucla,
    chrono_tz::Tz::Pacific__Marquesas,
    chrono_tz::Tz::Asia__Tehran,
    chrono_tz::Tz::America__St_Johns,
    chrono_tz::Tz::Australia__Adelaide,
    chrono_tz::Tz::Australia__Lord_Howe,
    chrono_tz::Tz::Pacific__Chatham,
];

/// A zone that is `offset` seconds east of UTC at the wall-clock time
/// `at`. Whole hours have exact fixed zones (`Etc/GMT-3` is UTC+3); the
/// time type here can't carry any other offset, so the rest borrow a real
/// zone with that offset on that date.
fn fixed_zone(offset: i32, at: NaiveDateTime) -> Option<chrono_tz::Tz> {
    if offset % 3600 == 0 {
        let hours = offset / 3600;
        if hours == 0 {
            return Some(chrono_tz::Tz::UTC);
        }
        // The sign in these names is the other way round.
        return format!("Etc/GMT{:+}", hours.checked_neg()?).parse().ok();
    }
    ODD_OFFSET_ZONES.iter().copied().find(|zone| {
        zone.offset_from_local_datetime(&at)
            .earliest()
            .is_some_and(|o| o.fix().local_minus_utc() == offset)
    })
}

/// Common Windows zone names (Exchange and Outlook write them) and the
/// IANA zones they stand for.
const WINDOWS_ZONES: &[(&str, &str)] = &[
    // Africa.
    ("E. Africa Standard Time", "Africa/Nairobi"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("W. Central Africa Standard Time", "Africa/Lagos"),
    ("Egypt Standard Time", "Africa/Cairo"),
    ("Morocco Standard Time", "Africa/Casablanca"),
    ("Libya Standard Time", "Africa/Tripoli"),
    ("Namibia Standard Time", "Africa/Windhoek"),
    ("Sudan Standard Time", "Africa/Khartoum"),
    ("South Sudan Standard Time", "Africa/Juba"),
    ("Mauritius Standard Time", "Indian/Mauritius"),
    // Europe.
    ("GMT Standard Time", "Europe/London"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Romance Standard Time", "Europe/Paris"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("FLE Standard Time", "Europe/Kyiv"),
    ("GTB Standard Time", "Europe/Bucharest"),
    ("Turkey Standard Time", "Europe/Istanbul"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("Belarus Standard Time", "Europe/Minsk"),
    // Middle East and Asia.
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("Jordan Standard Time", "Asia/Amman"),
    ("Middle East Standard Time", "Asia/Beirut"),
    ("Arab Standard Time", "Asia/Riyadh"),
    ("Arabic Standard Time", "Asia/Baghdad"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("Iran Standard Time", "Asia/Tehran"),
    ("Pakistan Standard Time", "Asia/Karachi"),
    ("India Standard Time", "Asia/Kolkata"),
    ("Sri Lanka Standard Time", "Asia/Colombo"),
    ("Nepal Standard Time", "Asia/Kathmandu"),
    ("Bangladesh Standard Time", "Asia/Dhaka"),
    ("SE Asia Standard Time", "Asia/Bangkok"),
    ("China Standard Time", "Asia/Shanghai"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("Taipei Standard Time", "Asia/Taipei"),
    ("W. Australia Standard Time", "Australia/Perth"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("Korea Standard Time", "Asia/Seoul"),
    // Australia and the Pacific.
    ("Cen. Australia Standard Time", "Australia/Adelaide"),
    ("AUS Central Standard Time", "Australia/Darwin"),
    ("E. Australia Standard Time", "Australia/Brisbane"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("Tasmania Standard Time", "Australia/Hobart"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    // The Americas.
    ("Alaskan Standard Time", "America/Anchorage"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("US Mountain Standard Time", "America/Phoenix"),
    ("Mountain Standard Time", "America/Denver"),
    ("Central Standard Time", "America/Chicago"),
    ("Canada Central Standard Time", "America/Regina"),
    ("Central Standard Time (Mexico)", "America/Mexico_City"),
    ("Eastern Standard Time", "America/New_York"),
    ("US Eastern Standard Time", "America/Indiana/Indianapolis"),
    ("SA Pacific Standard Time", "America/Bogota"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("Venezuela Standard Time", "America/Caracas"),
    ("Pacific SA Standard Time", "America/Santiago"),
    ("Newfoundland Standard Time", "America/St_Johns"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("Argentina Standard Time", "America/Argentina/Buenos_Aires"),
    ("UTC", "UTC"),
    ("Coordinated Universal Time", "UTC"),
    ("Greenwich Standard Time", "UTC"),
];

/// The IANA zone for a Windows time zone name, as Outlook and Exchange
/// write them in a `TZID`.
fn windows_zone(id: &str) -> Option<&'static str> {
    WINDOWS_ZONES
        .iter()
        .find(|(windows, _)| *windows == id)
        .map(|(_, iana)| *iana)
}

/// The IANA zone for a Windows zone name (as Microsoft Graph writes them).
#[must_use]
pub fn iana_zone(windows: &str) -> Option<&'static str> {
    windows_zone(windows.trim())
}

/// The Windows name of an IANA zone (for Microsoft Graph), if it has a
/// common one.
#[must_use]
pub fn windows_zone_name(iana: &str) -> Option<&'static str> {
    WINDOWS_ZONES
        .iter()
        .find(|(_, zone)| *zone == iana)
        .map(|(windows, _)| *windows)
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
    windows_zone(id).and_then(|name| name.parse().ok())
}

/// The zone a value is in. A `TZID` that names no known zone is read from
/// the calendar's own definition of it, as the offset it has on that date;
/// only without one are the times taken as the device's.
fn tz_of(prop: &Prop, zones: &Zones) -> Tz {
    let Some(id) = prop.param("TZID") else {
        return Tz::LOCAL;
    };
    if let Some(zone) = zone_for(&id) {
        return Tz::Tz(zone);
    }
    prop.value
        .split(',')
        .next()
        .and_then(naive)
        .and_then(|at| fixed_zone(zones.offset(&id, at)?, at))
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

fn when(prop: &Prop, zones: &Zones) -> Option<(DateTime<Tz>, bool)> {
    let all_day = is_date(prop);
    Some((
        parse_value(&prop.value, all_day, tz_of(prop, zones))?,
        all_day,
    ))
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

fn base(event: &Component, zones: &Zones) -> Option<Base> {
    let (start, all_day) = when(event.prop("DTSTART")?, zones)?;
    let begin = start.timestamp();
    let end = match (event.prop("DTEND"), event.prop("DURATION")) {
        (Some(end), _) => parse_value(&end.value, all_day, tz_of(end, zones))
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
    let repeat_rule = rule.as_deref().and_then(|r| repeat_rule_of(r, &start));
    let is_organizer = organizer.is_none();
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
            reminders: reminders_of(event, begin, end),
            show_as: show_as_of(event),
            private: event.prop("CLASS").is_some_and(|p| {
                matches!(
                    p.value.trim().to_ascii_uppercase().as_str(),
                    "PRIVATE" | "CONFIDENTIAL"
                )
            }),
            repeat_rule,
            is_organizer,
        },
    })
}

/// Free/busy from Outlook's `X-MICROSOFT-CDO-BUSYSTATUS`, else `TRANSP`
/// and `STATUS`.
fn show_as_of(event: &Component) -> ShowAs {
    if let Some(cdo) = event.prop("X-MICROSOFT-CDO-BUSYSTATUS") {
        match cdo.value.trim().to_ascii_uppercase().as_str() {
            "FREE" => return ShowAs::Free,
            "TENTATIVE" => return ShowAs::Tentative,
            "OOF" => return ShowAs::Away,
            "BUSY" | "WORKINGELSEWHERE" => return ShowAs::Busy,
            _ => {}
        }
    }
    if event
        .prop("TRANSP")
        .is_some_and(|p| p.value.trim().eq_ignore_ascii_case("TRANSPARENT"))
    {
        return ShowAs::Free;
    }
    if event
        .prop("STATUS")
        .is_some_and(|p| p.value.trim().eq_ignore_ascii_case("TENTATIVE"))
    {
        return ShowAs::Tentative;
    }
    ShowAs::Busy
}

const WEEKDAY_CODES: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

/// `2TU` as (Some(2), "TU"); `TU` as (None, "TU").
fn split_by_day(part: &str) -> Option<(Option<i32>, String)> {
    let part = part.trim().to_ascii_uppercase();
    let code = part.get(part.len().checked_sub(2)?..)?.to_owned();
    if !WEEKDAY_CODES.contains(&code.as_str()) {
        return None;
    }
    let number = part.get(..part.len().checked_sub(2)?)?;
    if number.is_empty() {
        return Some((None, code));
    }
    let n: i32 = number.trim_start_matches('+').parse().ok()?;
    Some((Some(n), code))
}

/// The day an `UNTIL` value falls on, as midnight UTC of that day (in the
/// series' own zone).
fn until_day(value: &str, start: &DateTime<Tz>) -> Option<i64> {
    let value = value.trim();
    let date = if value.len() == 8 {
        NaiveDate::parse_from_str(value, "%Y%m%d").ok()?
    } else if let Some(utc) = value.strip_suffix(['Z', 'z']) {
        let at = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        Utc.from_utc_datetime(&at)
            .with_timezone(&start.timezone())
            .date_naive()
    } else {
        NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S")
            .ok()?
            .date()
    };
    Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp())
}

/// A rule in the repeat dialog's terms, if it can show it.
fn repeat_rule_of(rule: &str, start: &DateTime<Tz>) -> Option<RepeatRule> {
    let mut out = RepeatRule {
        freq: Freq::Daily,
        interval: 1,
        by_day: Vec::new(),
        month_day: None,
        month: None,
        set_pos: None,
        count: None,
        until: None,
    };
    let mut freq = None;
    let mut day_pos: Option<i32> = None;
    for part in rule.split(';').filter(|p| !p.trim().is_empty()) {
        let (key, value) = part.split_once('=')?;
        let value = value.trim();
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = Some(match value.to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    _ => return None,
                });
            }
            "INTERVAL" => out.interval = value.parse().ok().filter(|n| *n >= 1)?,
            "COUNT" => out.count = Some(value.parse().ok().filter(|n| *n >= 1)?),
            "UNTIL" => out.until = Some(until_day(value, start)?),
            "BYDAY" => {
                for day in value.split(',') {
                    let (pos, code) = split_by_day(day)?;
                    if pos.is_some() {
                        if day_pos.is_some() || !out.by_day.is_empty() {
                            return None;
                        }
                        day_pos = pos;
                    } else if day_pos.is_some() {
                        return None;
                    }
                    if !out.by_day.contains(&code) {
                        out.by_day.push(code);
                    }
                }
            }
            "BYMONTHDAY" => {
                let n: i32 = value.parse().ok()?;
                if !(n == -1 || (1..=31).contains(&n)) {
                    return None;
                }
                out.month_day = Some(n);
            }
            "BYMONTH" => {
                let n: u32 = value.parse().ok()?;
                if !(1..=12).contains(&n) {
                    return None;
                }
                out.month = Some(n);
            }
            "BYSETPOS" => out.set_pos = Some(value.parse().ok()?),
            "WKST" => {}
            _ => return None,
        }
    }
    out.freq = freq?;
    if day_pos.is_some() {
        if out.set_pos.is_some() {
            return None;
        }
        out.set_pos = day_pos;
    }
    if let Some(pos) = out.set_pos
        && (!(pos == -1 || (1..=4).contains(&pos)) || out.by_day.len() != 1)
    {
        return None;
    }
    let fits = match out.freq {
        Freq::Daily => out.by_day.is_empty() && out.month_day.is_none() && out.month.is_none(),
        Freq::Weekly => out.month_day.is_none() && out.month.is_none() && out.set_pos.is_none(),
        Freq::Monthly => {
            out.month.is_none()
                && (out.by_day.is_empty() || out.set_pos.is_some())
                && !(out.month_day.is_some() && out.set_pos.is_some())
        }
        Freq::Yearly => {
            (out.by_day.is_empty() || out.set_pos.is_some())
                && !(out.month_day.is_some() && out.set_pos.is_some())
        }
    };
    (fits && !(out.count.is_some() && out.until.is_some())).then_some(out)
}

/// The `RRULE` value for a rule from the repeat dialog. `UNTIL` is the end
/// of that day: a date for all-day series, else the last second of the day
/// in `zone` (UTC without one).
#[must_use]
pub fn rule_text(rule: &RepeatRule, all_day: bool, zone: Option<chrono_tz::Tz>) -> String {
    let mut parts = vec![format!("FREQ={}", rule.freq.rule_name())];
    if rule.interval > 1 {
        parts.push(format!("INTERVAL={}", rule.interval));
    }
    let days: Vec<String> = rule
        .by_day
        .iter()
        .map(|d| d.trim().to_ascii_uppercase())
        .filter(|d| WEEKDAY_CODES.contains(&d.as_str()))
        .collect();
    match (rule.set_pos, days.as_slice()) {
        (Some(pos), [day]) if !matches!(rule.freq, Freq::Daily | Freq::Weekly) => {
            parts.push(format!("BYDAY={pos}{day}"));
        }
        (_, []) => {}
        (_, days) => parts.push(format!("BYDAY={}", days.join(","))),
    }
    if let Some(day) = rule.month_day {
        parts.push(format!("BYMONTHDAY={day}"));
    }
    if let Some(month) = rule.month {
        parts.push(format!("BYMONTH={month}"));
    }
    if let Some(count) = rule.count {
        parts.push(format!("COUNT={count}"));
    } else if let Some(until) = rule.until {
        let day = Utc
            .timestamp_opt(until, 0)
            .single()
            .unwrap_or_default()
            .date_naive();
        if all_day {
            parts.push(format!("UNTIL={}", day.format("%Y%m%d")));
        } else {
            let last = day.and_hms_opt(23, 59, 59).unwrap_or_default();
            let utc = zone
                .and_then(|z| z.from_local_datetime(&last).earliest())
                .map_or_else(|| last.and_utc(), |d| d.with_timezone(&Utc));
            parts.push(format!("UNTIL={}", utc.format("%Y%m%dT%H%M%SZ")));
        }
    }
    parts.join(";")
}

/// Most reminder lead time read: four weeks, in minutes.
const MAX_REMINDER_MINS: i64 = 4 * 7 * 24 * 60;

/// The event's alarms as minutes before its start. Alarms after the start,
/// emails and procedures are left out.
fn reminders_of(event: &Component, start: i64, end: i64) -> Vec<u32> {
    let mut out: Vec<u32> = event
        .children
        .iter()
        .filter(|c| c.name == "VALARM")
        .filter(|alarm| {
            alarm.prop("ACTION").is_none_or(|a| {
                matches!(
                    a.value.trim().to_ascii_uppercase().as_str(),
                    "DISPLAY" | "AUDIO"
                )
            })
        })
        .filter_map(|alarm| {
            let trigger = alarm.prop("TRIGGER")?;
            let at = if trigger
                .param("VALUE")
                .is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME"))
            {
                parse_value(&trigger.value, false, Tz::UTC)?.timestamp()
            } else {
                let offset = parse_duration(&trigger.value)?;
                let from_end = trigger
                    .param("RELATED")
                    .is_some_and(|r| r.eq_ignore_ascii_case("END"));
                (if from_end { end } else { start }).checked_add(offset)?
            };
            let before = start.checked_sub(at)?.checked_div(60)?;
            (0..=MAX_REMINDER_MINS)
                .contains(&before)
                .then(|| u32::try_from(before).ok())
                .flatten()
        })
        .collect();
    out.sort_unstable_by(|a, b| b.cmp(a));
    out.dedup();
    out
}

/// `VALARM` blocks for reminders, minutes before the start.
fn alarm_lines(reminders: &[u32]) -> Vec<String> {
    let mut sorted: Vec<u32> = reminders
        .iter()
        .copied()
        .filter(|m| i64::from(*m) <= MAX_REMINDER_MINS)
        .collect();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    sorted.dedup();
    sorted
        .into_iter()
        .flat_map(|m| {
            [
                "BEGIN:VALARM".to_owned(),
                "ACTION:DISPLAY".to_owned(),
                "DESCRIPTION:Reminder".to_owned(),
                format!("TRIGGER:-PT{m}M"),
                "END:VALARM".to_owned(),
            ]
        })
        .collect()
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

fn rule_set(event: &Component, start: DateTime<Tz>, zones: &Zones) -> Option<RRuleSet> {
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
                if let Some(when) = parse_value(value, all_day, tz_of(prop, zones)) {
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
fn starts_between(
    event: &Component,
    base: &Base,
    from: i64,
    to: i64,
    zones: &Zones,
) -> Option<Vec<i64>> {
    // Years 1 to 9999: what dates can hold.
    let (from, to) = (from.max(-62_135_596_800), to.min(253_402_300_799));
    let tz = base.start.timezone();
    let set = rule_set(event, base.start, zones)?
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
    let (events, zones) = calendar(ics);
    let (masters, overrides): (Vec<_>, Vec<_>) = events
        .iter()
        .partition(|e| e.prop("RECURRENCE-ID").is_none());
    // Dates of a series changed or cancelled on their own.
    let moved: Vec<(String, i64)> = overrides
        .iter()
        .filter_map(|e| {
            let (rid, _) = when(e.prop("RECURRENCE-ID")?, &zones)?;
            Some((e.text("UID"), rid.timestamp()))
        })
        .collect();
    let mut out = Vec::new();
    for master in masters {
        if master.cancelled() {
            continue;
        }
        let Some(base) = base(master, &zones) else {
            continue;
        };
        let first = base.occurrence.start;
        if !base.occurrence.recurring {
            if overlaps(first, base.occurrence.end, from, to) {
                out.push(base.occurrence);
            }
            continue;
        }
        let starts = starts_between(master, &base, from, to, &zones).unwrap_or_else(|| vec![first]);
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
        let (Some(base), Some((rid, _))) = (
            base(event, &zones),
            event.prop("RECURRENCE-ID").and_then(|p| when(p, &zones)),
        ) else {
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
    let (events, zones) = calendar(ics);
    let event = events
        .iter()
        .find(|e| e.prop("RECURRENCE-ID").is_none())
        .or_else(|| events.first())?;
    base(event, &zones).map(|b| b.occurrence)
}

/// When the resource's events start and end, for finding it by date:
/// `(first start, last end)`, with `None` for a series without an end.
#[must_use]
pub fn span(ics: &str) -> (Option<i64>, Option<i64>) {
    let mut first: Option<i64> = None;
    let mut last: Option<i64> = Some(i64::MIN);
    let (events, zones) = calendar(ics);
    for event in events {
        let Some(base) = base(&event, &zones) else {
            continue;
        };
        first = Some(first.map_or(base.occurrence.start, |f| f.min(base.occurrence.start)));
        let end = if base.occurrence.recurring && event.prop("RECURRENCE-ID").is_none() {
            let rule = event
                .prop("RRULE")
                .map(|p| p.value.to_ascii_uppercase())
                .unwrap_or_default();
            if rule.contains("COUNT=") || rule.contains("UNTIL=") {
                rule_set(&event, base.start, &zones).and_then(|set| {
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

/// One clock change of a zone.
struct Change {
    /// When, in UTC.
    at: NaiveDateTime,
    /// Seconds east of UTC before and after.
    from: i32,
    to: i32,
    /// The zone's short name after it (`BST`).
    name: String,
}

impl Change {
    /// The wall-clock time it happens at, on the clock before the change.
    fn local(&self) -> NaiveDateTime {
        self.at
            .checked_add_signed(TimeDelta::seconds(i64::from(self.from)))
            .unwrap_or(self.at)
    }
}

fn new_year(year: i32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(year, 1, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .unwrap_or_default()
}

/// The zone's clock changes in `years` years from `first_year`, to the
/// quarter hour.
fn clock_changes(zone: chrono_tz::Tz, first_year: i32, years: i32) -> Vec<Change> {
    let offset_at = |t: NaiveDateTime| {
        let offset = zone.offset_from_utc_datetime(&t);
        (offset.fix().local_minus_utc(), offset.to_string())
    };
    let stop = new_year(first_year.saturating_add(years));
    let mut out = Vec::new();
    let mut day = new_year(first_year);
    while day < stop {
        let Some(next_day) = day.checked_add_signed(TimeDelta::days(1)) else {
            break;
        };
        let (mut previous, _) = offset_at(day);
        // Only a day with a change is looked at closely.
        if offset_at(next_day).0 != previous {
            let mut t = day;
            while t < next_day {
                let Some(next) = t.checked_add_signed(TimeDelta::minutes(15)) else {
                    break;
                };
                let (offset, name) = offset_at(next);
                if offset != previous {
                    out.push(Change {
                        at: next,
                        from: previous,
                        to: offset,
                        name,
                    });
                    previous = offset;
                }
                t = next;
            }
        }
        day = next_day;
    }
    out
}

/// The yearly rule these changes follow (`FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU`):
/// one each year from `first_year`, in the same month, on the same weekday
/// (the same one counted from the start of the month, or always the last)
/// at the same time. `None` when they don't, as for zones that move their
/// clocks by decree or with Ramadan.
fn yearly_rule(changes: &[&Change], first_year: i32) -> Option<String> {
    let first = changes.first()?;
    let start = first.local();
    let nth = |t: &NaiveDateTime| t.day0() / 7;
    let mut year = first_year;
    let mut same_nth = true;
    let mut all_last = true;
    for change in changes {
        let local = change.local();
        if local.year() != year
            || local.month() != start.month()
            || local.weekday() != start.weekday()
            || local.time() != start.time()
            || change.from != first.from
            || change.to != first.to
        {
            return None;
        }
        year = year.saturating_add(1);
        same_nth &= nth(&local) == nth(&start);
        all_last &= local
            .checked_add_signed(TimeDelta::days(7))
            .is_some_and(|next| next.month() != local.month());
    }
    let position = if same_nth {
        i64::from(nth(&start)).saturating_add(1)
    } else if all_last {
        -1
    } else {
        return None;
    };
    Some(format!(
        "FREQ=YEARLY;BYMONTH={};BYDAY={position}{}",
        start.month(),
        weekday_code(start.weekday())
    ))
}

/// How many years of clock changes must agree before a yearly rule is
/// written for them.
const RULE_YEARS: i32 = 12;

/// A `VTIMEZONE` for `zone`, so other apps read the times the same way.
/// Zones that change their clocks on a regular day get a yearly rule,
/// which holds for a repeating event's later years too; others get their
/// changes listed from the year before `year` to the year after.
fn vtimezone(zone: chrono_tz::Tz, year: i32) -> Vec<String> {
    let first_year = year.saturating_sub(1);
    let changes = clock_changes(zone, first_year, RULE_YEARS);
    let block =
        |kind: &str, start: String, from: i32, to: i32, name: &str, rule: Option<String>| {
            let mut lines = vec![
                format!("BEGIN:{kind}"),
                format!("DTSTART:{start}"),
                format!("TZOFFSETFROM:{}", offset_text(from)),
                format!("TZOFFSETTO:{}", offset_text(to)),
                format!("TZNAME:{name}"),
            ];
            lines.extend(rule.map(|r| format!("RRULE:{r}")));
            lines.push(format!("END:{kind}"));
            lines
        };
    let stamp = |t: NaiveDateTime| t.format("%Y%m%dT%H%M%S").to_string();
    let standard = changes.iter().map(|c| c.to).min().unwrap_or_default();
    let kind = |c: &Change| {
        if c.to > standard {
            "DAYLIGHT"
        } else {
            "STANDARD"
        }
    };
    let (daylight, winter): (Vec<&Change>, Vec<&Change>) =
        changes.iter().partition(|c| c.to > standard);
    let mut lines = vec![
        "BEGIN:VTIMEZONE".to_owned(),
        format!("TZID:{}", zone.name()),
    ];
    if changes.len() == usize::try_from(RULE_YEARS).unwrap_or(0).saturating_mul(2)
        && let (Some(daylight_rule), Some(winter_rule)) = (
            yearly_rule(&daylight, first_year),
            yearly_rule(&winter, first_year),
        )
    {
        let mut rules = [(daylight, daylight_rule), (winter, winter_rule)];
        rules.sort_by_key(|(list, _)| list.first().map(|c| c.at));
        for (list, rule) in rules {
            if let Some(c) = list.first() {
                lines.extend(block(
                    kind(c),
                    stamp(c.local()),
                    c.from,
                    c.to,
                    &c.name,
                    Some(rule),
                ));
            }
        }
    } else {
        // Three years of them, listed one by one.
        let stop = new_year(first_year.saturating_add(3));
        let listed: Vec<&Change> = changes.iter().filter(|c| c.at < stop).collect();
        for c in &listed {
            lines.extend(block(
                kind(c),
                stamp(c.local()),
                c.from,
                c.to,
                &c.name,
                None,
            ));
        }
        if listed.is_empty() {
            let offset = zone.offset_from_utc_datetime(&new_year(first_year));
            let seconds = offset.fix().local_minus_utc();
            lines.extend(block(
                "STANDARD",
                "19700101T000000".to_owned(),
                seconds,
                seconds,
                &offset.to_string(),
                None,
            ));
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
    if let Some(show_as) = input.show_as {
        lines.push(
            if show_as == ShowAs::Free {
                "TRANSP:TRANSPARENT"
            } else {
                "TRANSP:OPAQUE"
            }
            .to_owned(),
        );
        lines.push(format!("X-MICROSOFT-CDO-BUSYSTATUS:{}", show_as.cdo()));
    }
    if let Some(private) = input.private {
        lines.push(
            if private {
                "CLASS:PRIVATE"
            } else {
                "CLASS:PUBLIC"
            }
            .to_owned(),
        );
    }
    (lines, zone)
}

/// Properties of an edited event the editor's fields replace.
fn replaced_props(input: &EventInput) -> Vec<&'static str> {
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
    if input.show_as.is_some() {
        drop.extend(["TRANSP", "X-MICROSOFT-CDO-BUSYSTATUS"]);
    }
    if input.private.is_some() {
        drop.push("CLASS");
    }
    if input.attendees.is_some() {
        drop.push("ATTENDEE");
    }
    drop
}

/// What an edit does to the series' rule: `None` keeps it, `Some(None)`
/// removes it, `Some(Some(line))` writes this one.
#[allow(clippy::option_option, reason = "keep, remove or replace")]
fn rule_change(input: &EventInput, zone: Option<chrono_tz::Tz>) -> Option<Option<String>> {
    if let Some(rule) = &input.repeat_rule {
        return Some(Some(format!(
            "RRULE:{}",
            rule_text(rule, input.all_day, zone)
        )));
    }
    match input.repeat {
        Repeat::Keep | Repeat::Custom => None,
        other => Some(rule_line(other)),
    }
}

/// The email address in an `ATTENDEE` or `ORGANIZER` line.
fn line_address(raw: &str) -> Option<String> {
    let line = split_line(raw)?;
    person(&Prop {
        name: line.name.clone(),
        params: line.params.to_owned(),
        value: line.value.to_owned(),
    })
    .map(|p| p.email)
}

/// `ATTENDEE` lines (and an `ORGANIZER` if none) for the editor's list of
/// addresses. People already invited keep their line with their answer.
fn people_lines(input: &EventInput, block_lines: &[&str]) -> Vec<String> {
    let Some(wanted) = &input.attendees else {
        return Vec::new();
    };
    let top_level = |name: &str| -> Vec<&str> {
        let mut depth = 0usize;
        block_lines
            .iter()
            .filter(|raw| {
                let n = split_line(raw).map(|l| l.name).unwrap_or_default();
                match n.as_str() {
                    "BEGIN" => {
                        depth = depth.saturating_add(1);
                        false
                    }
                    "END" => {
                        depth = depth.saturating_sub(1);
                        false
                    }
                    _ => depth == 0 && n == name,
                }
            })
            .copied()
            .collect()
    };
    let existing = top_level("ATTENDEE");
    let organizer = top_level("ORGANIZER")
        .first()
        .and_then(|raw| line_address(raw));
    let me = input.organizer.as_ref().map(|p| p.email.clone());
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for address in wanted {
        let address = address.trim();
        if address.is_empty()
            || !address.contains('@')
            || address
                .chars()
                .any(|c| c.is_control() || c == ':' || c == ';')
            || seen.iter().any(|s| same_address(s, address))
        {
            continue;
        }
        seen.push(address.to_owned());
        let kept = existing
            .iter()
            .find(|raw| line_address(raw).is_some_and(|a| same_address(&a, address)));
        match kept {
            Some(raw) => out.push((*raw).to_owned()),
            None => out.push(format!(
                "ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:{address}"
            )),
        }
    }
    // You, as organiser, stay listed if you were.
    if let Some(me) = &me
        && let Some(raw) = existing
            .iter()
            .find(|raw| line_address(raw).is_some_and(|a| same_address(&a, me)))
        && !seen.iter().any(|s| same_address(s, me))
    {
        out.insert(0, (*raw).to_owned());
    }
    if organizer.is_none()
        && !seen.is_empty()
        && let Some(me) = &input.organizer
    {
        let mut line = "ORGANIZER".to_owned();
        if let Some(name) = me.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            line.push_str(";CN=");
            line.push_str(&quote_param(name));
        }
        line.push_str(":mailto:");
        line.push_str(me.email.trim());
        out.insert(0, line);
    }
    out
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
    lines.extend(rule_change(input, zone).flatten());
    lines.extend(people_lines(input, &[]));
    lines.extend(alarm_lines(input.reminders.as_deref().unwrap_or_default()));
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
/// properties named in `drop` removed. Other parts are kept, and alarms
/// too unless `drop_alarms`.
fn kept_lines<'a>(
    lines: &[&'a str],
    block: &Block,
    drop: &[&str],
    drop_alarms: bool,
) -> Vec<&'a str> {
    let mut depth = 0usize;
    // Inside an alarm being dropped: its depth.
    let mut skipping: Option<usize> = None;
    let mut out = Vec::new();
    for raw in lines
        .get(block.begin.saturating_add(1)..block.end)
        .unwrap_or_default()
    {
        let line = split_line(raw);
        let name = line.as_ref().map(|l| l.name.clone()).unwrap_or_default();
        match name.as_str() {
            "BEGIN" => {
                depth = depth.saturating_add(1);
                if drop_alarms
                    && depth == 1
                    && line.is_some_and(|l| l.value.trim().eq_ignore_ascii_case("VALARM"))
                {
                    skipping = Some(depth);
                }
            }
            "END" => {
                let closing = depth;
                depth = depth.saturating_sub(1);
                if skipping == Some(closing) {
                    skipping = None;
                    continue;
                }
            }
            n if depth == 0 && drop.contains(&n) => continue,
            _ => {}
        }
        if skipping.is_none() {
            out.push(*raw);
        }
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
    let rule = rule_change(input, zone);
    let no_longer_a_series = matches!(rule, Some(None));
    let mut drop = replaced_props(input);
    if rule.is_some() {
        drop.push("RRULE");
        if no_longer_a_series {
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
    // No longer a series: the changed copies of single dates
    // (`RECURRENCE-ID`) go with the rule, or each would show as an event
    // of its own.
    let drop_overrides = no_longer_a_series;
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
            if let Some(rule) = &rule {
                out.extend(rule.iter().cloned());
            }
            out.extend(people_lines(
                input,
                lines
                    .get(master.begin.saturating_add(1)..master.end)
                    .unwrap_or_default(),
            ));
            out.extend(
                kept_lines(&lines, master, &drop, input.reminders.is_some())
                    .into_iter()
                    .map(str::to_owned),
            );
            if let Some(reminders) = &input.reminders {
                out.extend(alarm_lines(reminders));
            }
            continue;
        }
        if i > master.begin && i < master.end {
            continue;
        }
        let dropped = drop_overrides
            && found
                .iter()
                .any(|b| b.is_override && !std::ptr::eq(b, master) && i >= b.begin && i <= b.end);
        // A zone for the first event goes before it, even when the master
        // comes after an override.
        if let Some(zone) = zone
            && !has_zone(zone.name())
            && master.begin != found.first().map_or(master.begin, |b| b.begin)
            && found.first().is_some_and(|b| b.begin == i)
        {
            out.extend(vtimezone(zone, year_of(input.start)));
        }
        if !dropped {
            out.push((*raw).to_owned());
        }
    }
    Some(finish(&out))
}

/// A date line (`EXDATE`, `RECURRENCE-ID`, `DTSTART`) naming `seconds` the
/// way the series' `DTSTART` is written: a date, UTC, or local time in its
/// zone.
fn date_line(name: &str, start: &Prop, zones: &Zones, seconds: i64) -> Option<String> {
    Some(if is_date(start) {
        format!("{name};VALUE=DATE:{}", utc_date(seconds))
    } else if start.value.trim().ends_with(['Z', 'z']) {
        format!("{name}:{}", utc_stamp(seconds))
    } else {
        let tz = tz_of(start, zones);
        let local = at(tz, seconds)?.format("%Y%m%dT%H%M%S").to_string();
        match start.param("TZID") {
            Some(id) if !tz.is_local() => format!("{name};TZID={}:{local}", quote_param(&id)),
            _ => format!("{name}:{local}"),
        }
    })
}

/// Whether a block is the changed copy of the date `recurrence_id`.
fn override_for(lines: &[&str], block: &Block, zones: &Zones, recurrence_id: i64) -> bool {
    block.is_override
        && lines
            .get(block.begin..block.end)
            .unwrap_or_default()
            .iter()
            .filter_map(|l| split_line(l))
            .find(|l| l.name == "RECURRENCE-ID")
            .and_then(|l| {
                when(
                    &Prop {
                        name: l.name,
                        params: l.params.to_owned(),
                        value: l.value.to_owned(),
                    },
                    zones,
                )
            })
            .is_some_and(|(rid, _)| rid.timestamp() == recurrence_id)
}

/// The `DTSTART` of a block, as a property.
fn block_start(lines: &[&str], block: &Block) -> Option<Prop> {
    lines
        .get(block.begin..block.end)?
        .iter()
        .filter_map(|l| split_line(l))
        .find(|l| l.name == "DTSTART")
        .map(|l| Prop {
            name: l.name,
            params: l.params.to_owned(),
            value: l.value.to_owned(),
        })
}

/// Change one date of a series only: its changed copy (`RECURRENCE-ID`)
/// is written, or rewritten, with the editor's fields; the series and its
/// other dates stay as they are. `None` when there is no series.
#[must_use]
pub fn edit_occurrence(
    original: &str,
    recurrence_id: i64,
    input: &EventInput,
    now: i64,
) -> Option<String> {
    let unfolded = unfold(original);
    let lines: Vec<&str> = unfolded.lines().filter(|l| !l.trim().is_empty()).collect();
    let found = blocks(&lines);
    let zones = calendar(original).1;
    let master = found.iter().find(|b| !b.is_override)?;
    let start = block_start(&lines, master)?;
    let rid = date_line("RECURRENCE-ID", &start, &zones, recurrence_id)?;
    let existing = found
        .iter()
        .find(|b| override_for(&lines, b, &zones, recurrence_id));
    let source = existing.unwrap_or(master);
    let sequence = vevents(original)
        .iter()
        .filter_map(|e| e.prop("SEQUENCE"))
        .filter_map(|p| p.value.trim().parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    // One date can't repeat on its own.
    let single = EventInput {
        repeat: Repeat::Keep,
        repeat_rule: None,
        ..input.clone()
    };
    let (fields, zone) = field_lines(&single, now, sequence);
    let mut drop = replaced_props(&single);
    drop.extend(["RRULE", "EXDATE", "RDATE", "RECURRENCE-ID"]);
    let mut block = vec!["BEGIN:VEVENT".to_owned(), rid];
    block.extend(fields);
    block.extend(people_lines(
        &single,
        lines
            .get(source.begin.saturating_add(1)..source.end)
            .unwrap_or_default(),
    ));
    block.extend(
        kept_lines(&lines, source, &drop, single.reminders.is_some())
            .into_iter()
            .map(str::to_owned),
    );
    if let Some(reminders) = &single.reminders {
        block.extend(alarm_lines(reminders));
    }
    block.push("END:VEVENT".to_owned());
    let has_zone = |name: &str| {
        parse(original).iter().any(|cal| {
            cal.children
                .iter()
                .any(|c| c.name == "VTIMEZONE" && c.text("TZID") == name)
        })
    };
    let mut out = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        if let Some(zone) = zone
            && !has_zone(zone.name())
            && found.first().is_some_and(|b| b.begin == i)
        {
            out.extend(vtimezone(zone, year_of(single.start)));
        }
        if existing.is_some_and(|b| i >= b.begin && i <= b.end) {
            continue;
        }
        out.push((*raw).to_owned());
        if i == master.end {
            out.extend(block.iter().cloned());
        }
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
    let zones = calendar(original).1;
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
    let exdate = date_line("EXDATE", &start, &zones, recurrence_id)?;
    // Overrides for that date are dropped.
    let skipped: Vec<&Block> = found
        .iter()
        .filter(|b| override_for(&lines, b, &zones, recurrence_id))
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

// ------------------------------------------------------------ invitations

/// An invitation or an answer to one, found in an email (iMIP, RFC 6047).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Invitation {
    /// `REQUEST` (an invitation or update), `CANCEL`, `REPLY` (someone's
    /// answer), `PUBLISH`, ...
    pub method: String,
    /// The event (a series, or one changed date of it).
    pub event: Occurrence,
    /// `SEQUENCE`: higher is newer.
    pub sequence: u32,
    /// Your answer as listed in it, if you are an attendee.
    pub my_status: Option<String>,
    /// For a `REPLY`: who answered, with their answer in `status`.
    pub replier: Option<Person>,
}

/// Your answer to an invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Answer {
    /// Yes.
    Accepted,
    /// Maybe.
    Tentative,
    /// No.
    Declined,
}

impl Answer {
    fn partstat(self) -> &'static str {
        match self {
            Self::Accepted => "ACCEPTED",
            Self::Tentative => "TENTATIVE",
            Self::Declined => "DECLINED",
        }
    }
}

fn same_address(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn calendar_method(ics: &str) -> String {
    parse(ics)
        .into_iter()
        .find(|c| c.name == "VCALENDAR")
        .and_then(|c| {
            c.prop("METHOD")
                .map(|p| p.value.trim().to_ascii_uppercase())
        })
        .unwrap_or_else(|| "PUBLISH".to_owned())
}

/// The invitation in a calendar part. `me` is the account's address.
#[must_use]
pub fn invitation(ics: &str, me: &str) -> Option<Invitation> {
    let (events, zones) = calendar(ics);
    let event = events
        .iter()
        .find(|e| e.prop("RECURRENCE-ID").is_none())
        .or_else(|| events.first())?;
    let mut occurrence = base(event, &zones)?.occurrence;
    if let Some((rid, _)) = event.prop("RECURRENCE-ID").and_then(|p| when(p, &zones)) {
        occurrence.recurrence_id = Some(rid.timestamp());
    }
    if occurrence.uid.is_empty() {
        return None;
    }
    let method = calendar_method(ics);
    let my_status = occurrence
        .attendees
        .iter()
        .find(|p| same_address(&p.email, me))
        .and_then(|p| p.status.clone());
    let replier = (method == "REPLY")
        .then(|| occurrence.attendees.first().cloned())
        .flatten();
    Some(Invitation {
        method,
        sequence: event
            .prop("SEQUENCE")
            .and_then(|p| p.value.trim().parse().ok())
            .unwrap_or(0),
        event: occurrence,
        my_status,
        replier,
    })
}

/// Lines from `lines[from..=to]`.
fn copy_block(lines: &[&str], from: usize, to: usize) -> Vec<String> {
    lines
        .get(from..=to)
        .unwrap_or_default()
        .iter()
        .map(|l| (*l).to_owned())
        .collect()
}

/// `VTIMEZONE` blocks (start and end line), at the top of the calendar.
fn zone_blocks(lines: &[&str]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut open = None;
    for (i, raw) in lines.iter().enumerate() {
        let Some(line) = split_line(raw) else {
            continue;
        };
        match line.name.as_str() {
            "BEGIN" => {
                depth = depth.saturating_add(1);
                if depth == 2 && line.value.trim().eq_ignore_ascii_case("VTIMEZONE") {
                    open = Some(i);
                }
            }
            "END" => {
                if depth == 2
                    && let Some(start) = open.take()
                {
                    out.push((start, i));
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    out
}

fn quote_param(value: &str) -> String {
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

/// An `ATTENDEE` line for `me` with this answer.
fn my_attendee(me: &str, name: Option<&str>, answer: Answer) -> String {
    let mut line = format!("ATTENDEE;PARTSTAT={}", answer.partstat());
    if let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) {
        line.push_str(";CN=");
        line.push_str(&quote_param(name));
    }
    line.push_str(":mailto:");
    line.push_str(me.trim());
    line
}

/// An `ATTENDEE` line with its answer changed and `RSVP` dropped.
fn answered_line(raw: &str, answer: Answer) -> Option<String> {
    let line = split_line(raw)?;
    let mut params: Vec<String> = param_list(line.params)
        .into_iter()
        .filter(|(k, _)| k != "PARTSTAT" && k != "RSVP")
        .map(|(k, v)| format!("{k}={}", quote_param(&v)))
        .collect();
    params.insert(0, format!("PARTSTAT={}", answer.partstat()));
    Some(format!("ATTENDEE;{}:{}", params.join(";"), line.value))
}

fn is_me(raw: &str, me: &str) -> bool {
    split_line(raw).is_some_and(|l| {
        l.name == "ATTENDEE"
            && person(&Prop {
                name: l.name.clone(),
                params: l.params.to_owned(),
                value: l.value.to_owned(),
            })
            .is_some_and(|p| same_address(&p.email, me))
    })
}

/// What a reply keeps of each event.
const REPLY_KEEPS: &[&str] = &[
    "UID",
    "SEQUENCE",
    "RECURRENCE-ID",
    "DTSTART",
    "DTEND",
    "DURATION",
    "SUMMARY",
    "ORGANIZER",
];

/// The answer to send the organiser (`METHOD:REPLY`): each event in the
/// invitation with only you as attendee, and your answer.
#[must_use]
pub fn reply(
    ics: &str,
    me: &str,
    my_name: Option<&str>,
    answer: Answer,
    now: i64,
) -> Option<String> {
    let unfolded = unfold(ics);
    let lines: Vec<&str> = unfolded.lines().filter(|l| !l.trim().is_empty()).collect();
    let events = blocks(&lines);
    if events.is_empty() {
        return None;
    }
    let mut out = vec![
        "BEGIN:VCALENDAR".to_owned(),
        "VERSION:2.0".to_owned(),
        PRODID.to_owned(),
        "METHOD:REPLY".to_owned(),
    ];
    for (from, to) in zone_blocks(&lines) {
        out.extend(copy_block(&lines, from, to));
    }
    for block in &events {
        out.push("BEGIN:VEVENT".to_owned());
        let mut depth = 0usize;
        let mut found_me: Option<String> = None;
        for raw in lines
            .get(block.begin.saturating_add(1)..block.end)
            .unwrap_or_default()
        {
            let name = split_line(raw).map(|l| l.name).unwrap_or_default();
            match name.as_str() {
                "BEGIN" => depth = depth.saturating_add(1),
                "END" => depth = depth.saturating_sub(1),
                _ if depth > 0 => {}
                "ATTENDEE" if is_me(raw, me) => found_me = answered_line(raw, answer),
                n if REPLY_KEEPS.contains(&n) => out.push((*raw).to_owned()),
                _ => {}
            }
        }
        out.push(found_me.unwrap_or_else(|| my_attendee(me, my_name, answer)));
        out.push(format!("DTSTAMP:{}", utc_stamp(now)));
        out.push("END:VEVENT".to_owned());
    }
    out.push("END:VCALENDAR".to_owned());
    Some(finish(&out))
}

/// The invitation as it goes in your calendar: without `METHOD`, and with
/// your answer on your `ATTENDEE` line (added if you weren't listed).
#[must_use]
pub fn calendar_copy(ics: &str, me: &str, my_name: Option<&str>, answer: Answer) -> Option<String> {
    let unfolded = unfold(ics);
    let lines: Vec<&str> = unfolded.lines().filter(|l| !l.trim().is_empty()).collect();
    let events = blocks(&lines);
    if events.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut listed = false;
    for raw in &lines {
        let name = split_line(raw).map(|l| l.name).unwrap_or_default();
        match name.as_str() {
            "BEGIN" => depth = depth.saturating_add(1),
            "END" => {
                // Not listed (invited through a group): add yourself.
                if depth == 2 && !listed && raw.trim().eq_ignore_ascii_case("END:VEVENT") {
                    out.push(my_attendee(me, my_name, answer));
                }
                if depth == 2 {
                    listed = false;
                }
                depth = depth.saturating_sub(1);
            }
            "METHOD" if depth == 1 => continue,
            "ATTENDEE" if depth == 2 && is_me(raw, me) => {
                listed = true;
                if let Some(line) = answered_line(raw, answer) {
                    out.push(line);
                }
                continue;
            }
            _ => {}
        }
        out.push((*raw).to_owned());
    }
    Some(finish(&out))
}

/// Your calendar copy updated with someone's answer (`REPLY`): their
/// `ATTENDEE` line takes the answer. `None` when nothing changes.
#[must_use]
pub fn apply_reply(stored: &str, reply: &str) -> Option<String> {
    let answer = invitation(reply, "")?;
    let who = answer.replier?;
    let status = who.status?.to_ascii_uppercase();
    let unfolded = unfold(stored);
    let lines: Vec<&str> = unfolded.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut changed = false;
    let out: Vec<String> = lines
        .iter()
        .map(|raw| {
            if is_me(raw, &who.email)
                && let Some(line) = split_line(raw)
            {
                let mut params: Vec<String> = param_list(line.params)
                    .into_iter()
                    .filter(|(k, _)| k != "PARTSTAT" && k != "RSVP")
                    .map(|(k, v)| format!("{k}={}", quote_param(&v)))
                    .collect();
                params.insert(0, format!("PARTSTAT={status}"));
                let new = format!("ATTENDEE;{}:{}", params.join(";"), line.value);
                if new != *raw {
                    changed = true;
                }
                return new;
            }
            (*raw).to_owned()
        })
        .collect();
    changed.then(|| finish(&out))
}

// ------------------------------------------------------------- scheduling

/// Lines of a resource without alarms or `METHOD`, unfolded.
fn without_alarms(lines: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut skipping: Option<usize> = None;
    for raw in lines {
        let line = split_line(raw);
        let name = line.as_ref().map(|l| l.name.clone()).unwrap_or_default();
        match name.as_str() {
            "BEGIN" => {
                depth = depth.saturating_add(1);
                if skipping.is_none()
                    && line.is_some_and(|l| l.value.trim().eq_ignore_ascii_case("VALARM"))
                {
                    skipping = Some(depth);
                }
            }
            "END" => {
                let closing = depth;
                depth = depth.saturating_sub(1);
                if skipping == Some(closing) {
                    skipping = None;
                    continue;
                }
            }
            "METHOD" if depth == 1 => continue,
            _ => {}
        }
        if skipping.is_none() {
            out.push((*raw).to_owned());
        }
    }
    out
}

/// An invitation or update to email the attendees (iMIP `METHOD:REQUEST`):
/// the stored event as it is, without your own alarms. `None` when it has
/// no event.
#[must_use]
pub fn itip_request(ics: &str) -> Option<String> {
    let unfolded = unfold(ics);
    let lines: Vec<&str> = unfolded.lines().filter(|l| !l.trim().is_empty()).collect();
    if blocks(&lines).is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for line in without_alarms(&lines) {
        let calendar_start = line.trim().eq_ignore_ascii_case("BEGIN:VCALENDAR");
        out.push(line);
        if calendar_start {
            out.push("METHOD:REQUEST".to_owned());
        }
    }
    Some(finish(&out))
}

/// What a cancellation keeps of each event.
const CANCEL_KEEPS: &[&str] = &[
    "UID",
    "RECURRENCE-ID",
    "DTSTART",
    "DTEND",
    "DURATION",
    "SUMMARY",
    "LOCATION",
    "ORGANIZER",
];

/// A cancellation to email (iMIP `METHOD:CANCEL`): of the whole event, or
/// with `recurrence_id` of that one date. `only` limits the `ATTENDEE`s
/// named (people taken off the list); `None` names everyone. `None` when
/// there is no event.
#[must_use]
pub fn itip_cancel(
    ics: &str,
    recurrence_id: Option<i64>,
    only: Option<&[String]>,
    now: i64,
) -> Option<String> {
    let unfolded = unfold(ics);
    let lines: Vec<&str> = unfolded.lines().filter(|l| !l.trim().is_empty()).collect();
    let events = blocks(&lines);
    let zones = calendar(ics).1;
    let master = events
        .iter()
        .find(|b| !b.is_override)
        .or_else(|| events.first())?;
    let sequence = vevents(ics)
        .iter()
        .filter_map(|e| e.prop("SEQUENCE"))
        .filter_map(|p| p.value.trim().parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let named = |raw: &str| {
        only.is_none_or(|list| {
            line_address(raw).is_some_and(|a| list.iter().any(|w| same_address(w, &a)))
        })
    };
    let mut out = vec![
        "BEGIN:VCALENDAR".to_owned(),
        "VERSION:2.0".to_owned(),
        PRODID.to_owned(),
        "METHOD:CANCEL".to_owned(),
    ];
    for (from, to) in zone_blocks(&lines) {
        out.extend(copy_block(&lines, from, to));
    }
    let chosen: Vec<&Block> = match recurrence_id {
        Some(_) => vec![master],
        None => events.iter().collect(),
    };
    for block in chosen {
        out.push("BEGIN:VEVENT".to_owned());
        let mut depth = 0usize;
        for raw in lines
            .get(block.begin.saturating_add(1)..block.end)
            .unwrap_or_default()
        {
            let name = split_line(raw).map(|l| l.name).unwrap_or_default();
            match name.as_str() {
                "BEGIN" => depth = depth.saturating_add(1),
                "END" => depth = depth.saturating_sub(1),
                _ if depth > 0 => {}
                "ATTENDEE" if named(raw) => out.push((*raw).to_owned()),
                "DTSTART" | "DTEND" | "DURATION" | "RECURRENCE-ID" if recurrence_id.is_some() => {}
                n if CANCEL_KEEPS.contains(&n) => out.push((*raw).to_owned()),
                _ => {}
            }
        }
        if let Some(rid) = recurrence_id {
            let start = block_start(&lines, block)?;
            out.push(date_line("RECURRENCE-ID", &start, &zones, rid)?);
            out.push(date_line("DTSTART", &start, &zones, rid)?);
        }
        out.push(format!("SEQUENCE:{sequence}"));
        out.push(format!("DTSTAMP:{}", utc_stamp(now)));
        out.push("STATUS:CANCELLED".to_owned());
        out.push("END:VEVENT".to_owned());
    }
    out.push("END:VCALENDAR".to_owned());
    Some(finish(&out))
}

/// A busy stretch in free/busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BusyPeriod {
    /// Unix seconds.
    pub start: i64,
    /// Unix seconds, exclusive.
    pub end: i64,
    /// Busy, tentative or away.
    pub show_as: ShowAs,
}

/// The busy periods in a `VFREEBUSY` (a `free-busy-query` answer, or one
/// recipient's part of a scheduling answer). Free periods are left out.
#[must_use]
pub fn free_busy_periods(ics: &str) -> Vec<BusyPeriod> {
    let mut out = Vec::new();
    for fb in parse(ics)
        .into_iter()
        .filter(|c| c.name == "VCALENDAR")
        .flat_map(|c| c.children)
        .filter(|c| c.name == "VFREEBUSY")
    {
        for prop in fb.props.iter().filter(|p| p.name == "FREEBUSY") {
            let show_as = match prop
                .param("FBTYPE")
                .unwrap_or_default()
                .to_ascii_uppercase()
                .as_str()
            {
                "FREE" => continue,
                "BUSY-TENTATIVE" => ShowAs::Tentative,
                "BUSY-UNAVAILABLE" => ShowAs::Away,
                _ => ShowAs::Busy,
            };
            for period in prop.value.split(',') {
                let Some((from, to)) = period.trim().split_once('/') else {
                    continue;
                };
                let Some(start) = parse_value(from, false, Tz::UTC).map(|d| d.timestamp()) else {
                    continue;
                };
                let end = if to.trim_start().starts_with(['P', 'p', '+', '-']) {
                    parse_duration(to).and_then(|d| start.checked_add(d))
                } else {
                    parse_value(to, false, Tz::UTC).map(|d| d.timestamp())
                };
                if let Some(end) = end.filter(|e| *e > start) {
                    out.push(BusyPeriod {
                        start,
                        end,
                        show_as,
                    });
                }
            }
        }
    }
    out.sort_by_key(|p| (p.start, p.end));
    out
}

/// The `ATTENDEE` a `VFREEBUSY` answer is for, if it names one.
#[must_use]
pub fn free_busy_attendee(ics: &str) -> Option<String> {
    parse(ics)
        .into_iter()
        .filter(|c| c.name == "VCALENDAR")
        .flat_map(|c| c.children)
        .find(|c| c.name == "VFREEBUSY")?
        .prop("ATTENDEE")
        .and_then(person)
        .map(|p| p.email)
}

/// A free/busy request (`METHOD:REQUEST` `VFREEBUSY`, RFC 6638) for the
/// scheduling outbox: when `attendees` are busy between `from` and `to`.
#[must_use]
pub fn free_busy_request(
    organizer: &str,
    attendees: &[String],
    from: i64,
    to: i64,
    uid: &str,
    now: i64,
) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_owned(),
        "VERSION:2.0".to_owned(),
        PRODID.to_owned(),
        "METHOD:REQUEST".to_owned(),
        "BEGIN:VFREEBUSY".to_owned(),
        format!("UID:{}", escape(uid)),
        format!("DTSTAMP:{}", utc_stamp(now)),
        format!("DTSTART:{}", utc_stamp(from)),
        format!("DTEND:{}", utc_stamp(to)),
        format!("ORGANIZER:mailto:{}", organizer.trim()),
    ];
    for attendee in attendees {
        lines.push(format!("ATTENDEE:mailto:{}", attendee.trim()));
    }
    lines.push("END:VFREEBUSY".to_owned());
    lines.push("END:VCALENDAR".to_owned());
    finish(&lines)
}

// ------------------------------------------------------------------ tasks

/// Where each `VTODO` block is in a list of unfolded lines.
fn todo_block(lines: &[&str]) -> Option<(usize, usize)> {
    let mut depth = 0usize;
    let mut open = None;
    for (i, raw) in lines.iter().enumerate() {
        let Some(line) = split_line(raw) else {
            continue;
        };
        match line.name.as_str() {
            "BEGIN" => {
                depth = depth.saturating_add(1);
                if depth == 2 && line.value.trim().eq_ignore_ascii_case("VTODO") {
                    open = Some(i);
                }
            }
            "END" => {
                if depth == 2
                    && let Some(begin) = open.take()
                {
                    return Some((begin, i));
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    None
}

/// The to-do in a calendar resource (`VTODO`): its `UID` and fields.
/// `None` when the resource has none.
#[must_use]
pub fn parse_todo(ics: &str) -> Option<(String, TaskFields)> {
    let todo = parse(ics)
        .into_iter()
        .filter(|c| c.name == "VCALENDAR")
        .flat_map(|c| c.children)
        .find(|c| c.name == "VTODO")?;
    let zones = calendar(ics).1;
    let uid = todo.text("UID");
    let due = todo
        .prop("DUE")
        .and_then(|p| when(p, &zones))
        .map(|(d, _)| d.timestamp());
    let start = todo
        .prop("DTSTART")
        .and_then(|p| when(p, &zones))
        .map(|(d, _)| d.timestamp());
    let completed_at = todo
        .prop("COMPLETED")
        .and_then(|p| parse_value(&p.value, false, Tz::UTC))
        .map(|d| d.timestamp());
    let status = todo.text("STATUS").to_ascii_uppercase();
    let completed = status == "COMPLETED"
        || (status.is_empty() && completed_at.is_some())
        || todo.text("PERCENT-COMPLETE") == "100";
    let importance = match todo.text("PRIORITY").parse::<u8>().unwrap_or(0) {
        1..=4 => Importance::High,
        6..=9 => Importance::Low,
        _ => Importance::Normal,
    };
    let reminder = todo
        .children
        .iter()
        .filter(|c| c.name == "VALARM")
        .filter_map(|alarm| {
            let trigger = alarm.prop("TRIGGER")?;
            if trigger
                .param("VALUE")
                .is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME"))
            {
                return parse_value(&trigger.value, false, Tz::UTC).map(|d| d.timestamp());
            }
            let offset = parse_duration(&trigger.value)?;
            let from_end = trigger
                .param("RELATED")
                .is_some_and(|r| r.eq_ignore_ascii_case("END"));
            let base = if from_end { due } else { start.or(due) }?;
            base.checked_add(offset)
        })
        .min();
    let categories = todo
        .props
        .iter()
        .filter(|p| p.name == "CATEGORIES")
        .flat_map(|p| split_categories(&p.value))
        .collect();
    Some((
        uid,
        TaskFields {
            title: todo.text("SUMMARY"),
            notes: todo.text("DESCRIPTION"),
            due,
            reminder,
            importance,
            completed,
            completed_at: if completed { completed_at } else { None },
            categories,
        },
    ))
}

/// Category names in a `CATEGORIES` value (commas, with `\,` kept).
fn split_categories(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    current.push('\\');
                    current.push(next);
                }
            }
            ',' => out.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
    }
    out.push(current);
    out.into_iter()
        .map(|c| unescape(&c).trim().to_owned())
        .filter(|c| !c.is_empty())
        .collect()
}

/// The properties of a to-do that [`write_todo`] replaces.
const TODO_MANAGED: &[&str] = &[
    "SUMMARY",
    "DESCRIPTION",
    "DUE",
    "PRIORITY",
    "STATUS",
    "COMPLETED",
    "PERCENT-COMPLETE",
    "CATEGORIES",
    "DTSTAMP",
    "LAST-MODIFIED",
    "SEQUENCE",
    "UID",
];

/// A calendar resource for a to-do. With `original` (the server's copy),
/// everything the app doesn't edit is kept (start date, URL, related
/// items, other properties); its alarms are replaced by the reminder.
#[must_use]
#[allow(clippy::too_many_lines, reason = "one property after another")]
pub fn write_todo(uid: &str, task: &TaskFields, original: Option<&str>, now: i64) -> String {
    let unfolded = original.map(unfold).unwrap_or_default();
    let lines: Vec<&str> = unfolded.lines().filter(|l| !l.trim().is_empty()).collect();
    let block = todo_block(&lines);
    let sequence = original
        .and_then(|o| {
            parse(o)
                .into_iter()
                .filter(|c| c.name == "VCALENDAR")
                .flat_map(|c| c.children)
                .find(|c| c.name == "VTODO")
        })
        .map_or(0, |t| {
            t.text("SEQUENCE")
                .parse::<u32>()
                .unwrap_or(0)
                .saturating_add(1)
        });
    let mut fields = vec![
        format!("UID:{}", escape(uid)),
        format!("DTSTAMP:{}", utc_stamp(now)),
        format!("LAST-MODIFIED:{}", utc_stamp(now)),
        format!("SEQUENCE:{sequence}"),
    ];
    fields.extend(text_line("SUMMARY", &task.title));
    fields.extend(text_line("DESCRIPTION", &task.notes));
    let due_is_date = task.due.is_some_and(|d| d.rem_euclid(DAY) == 0);
    if let Some(due) = task.due {
        fields.push(if due_is_date {
            format!("DUE;VALUE=DATE:{}", utc_date(due))
        } else {
            format!("DUE:{}", utc_stamp(due))
        });
    }
    match task.importance {
        Importance::High => fields.push("PRIORITY:1".to_owned()),
        Importance::Low => fields.push("PRIORITY:9".to_owned()),
        Importance::Normal => {}
    }
    if task.completed {
        fields.push("STATUS:COMPLETED".to_owned());
        fields.push("PERCENT-COMPLETE:100".to_owned());
        fields.push(format!(
            "COMPLETED:{}",
            utc_stamp(task.completed_at.unwrap_or(now))
        ));
    } else {
        fields.push("STATUS:NEEDS-ACTION".to_owned());
    }
    if !task.categories.is_empty() {
        let names: Vec<String> = task.categories.iter().map(|c| escape(c)).collect();
        fields.push(format!("CATEGORIES:{}", names.join(",")));
    }
    let alarm = task.reminder.map(|at| {
        vec![
            "BEGIN:VALARM".to_owned(),
            "ACTION:DISPLAY".to_owned(),
            "DESCRIPTION:Reminder".to_owned(),
            format!("TRIGGER;VALUE=DATE-TIME:{}", utc_stamp(at)),
            "END:VALARM".to_owned(),
        ]
    });
    let Some((begin, end)) = block else {
        let mut out = vec![
            "BEGIN:VCALENDAR".to_owned(),
            "VERSION:2.0".to_owned(),
            PRODID.to_owned(),
            "BEGIN:VTODO".to_owned(),
            format!("CREATED:{}", utc_stamp(now)),
        ];
        out.extend(fields);
        out.extend(alarm.unwrap_or_default());
        out.push("END:VTODO".to_owned());
        out.push("END:VCALENDAR".to_owned());
        return finish(&out);
    };
    let found = Block {
        begin,
        end,
        is_override: false,
    };
    let mut drop: Vec<&str> = TODO_MANAGED.to_vec();
    // A start written as a date-time beside a due date (or the other way)
    // would make the to-do invalid.
    let start_mismatch = lines
        .get(begin..end)
        .unwrap_or_default()
        .iter()
        .filter_map(|l| split_line(l))
        .find(|l| l.name == "DTSTART")
        .is_some_and(|l| {
            let params = l.params.to_ascii_uppercase();
            let date = l.value.trim().len() == 8
                || (params.contains("VALUE=DATE") && !params.contains("DATE-TIME"));
            task.due.is_some() && date != due_is_date
        });
    if start_mismatch {
        drop.push("DTSTART");
    }
    let mut out = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        if i == begin {
            out.push((*raw).to_owned());
            out.extend(fields.iter().cloned());
            out.extend(
                kept_lines(&lines, &found, &drop, true)
                    .into_iter()
                    .map(str::to_owned),
            );
            out.extend(alarm.iter().flatten().cloned());
            continue;
        }
        if i > begin && i < end {
            continue;
        }
        out.push((*raw).to_owned());
    }
    finish(&out)
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
        assert_eq!(
            zone_for("\"Tokyo Standard Time\""),
            Some(chrono_tz::Tz::Asia__Tokyo)
        );
        assert_eq!(
            zone_for("AUS Eastern Standard Time"),
            Some(chrono_tz::Tz::Australia__Sydney)
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
            reminders: None,
            ..EventInput::default()
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
    fn regular_zones_get_a_yearly_rule_and_irregular_ones_a_list() {
        let london = vtimezone(chrono_tz::Tz::Europe__London, 2026).join("\n");
        assert!(london.contains(
            "BEGIN:DAYLIGHT\nDTSTART:20250330T010000\nTZOFFSETFROM:+0000\nTZOFFSETTO:+0100\nTZNAME:BST\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\nEND:DAYLIGHT"
        ));
        assert!(london.contains(
            "BEGIN:STANDARD\nDTSTART:20251026T020000\nTZOFFSETFROM:+0100\nTZOFFSETTO:+0000\nTZNAME:GMT\nRRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\nEND:STANDARD"
        ));
        assert_eq!(london.matches("BEGIN:DAYLIGHT").count(), 1);
        let new_york = vtimezone(chrono_tz::Tz::America__New_York, 2026).join("\n");
        assert!(new_york.contains("RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU"));
        assert!(new_york.contains("RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU"));
        // The southern half of the year works the same way.
        let sydney = vtimezone(chrono_tz::Tz::Australia__Sydney, 2026).join("\n");
        assert!(sydney.contains("RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=1SU"));
        assert!(sydney.contains("RRULE:FREQ=YEARLY;BYMONTH=4;BYDAY=1SU"));
        // Morocco moves its clocks with Ramadan: each change is listed.
        let casablanca = vtimezone(chrono_tz::Tz::Africa__Casablanca, 2026).join("\n");
        assert!(!casablanca.contains("RRULE"));
        assert!(casablanca.matches("DTSTART:").count() >= 4);
        // No clock changes: one fixed offset.
        let nairobi = vtimezone(chrono_tz::Tz::Africa__Nairobi, 2026).join("\n");
        assert!(!nairobi.contains("RRULE") && !nairobi.contains("DAYLIGHT"));
        assert!(nairobi.contains("TZOFFSETTO:+0300"));
    }

    /// A zone definition under a name no zone table knows.
    fn custom_zone(zone: chrono_tz::Tz) -> String {
        let mut text = String::new();
        for line in vtimezone(zone, 2026) {
            if line.starts_with("TZID:") {
                text.push_str("TZID:Customized Time Zone\r\n");
            } else {
                text.push_str(&line);
                text.push_str("\r\n");
            }
        }
        text
    }

    #[test]
    fn an_unknown_zone_name_is_read_from_its_own_definition() {
        // New York's rules under Exchange's label: summer is UTC-4, winter UTC-5.
        let ics = cal(&format!(
            "{}BEGIN:VEVENT\r\nUID:s\r\nDTSTART;TZID=\"Customized Time Zone\":20260715T090000\r\nDTEND;TZID=\"Customized Time Zone\":20260715T100000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:w\r\nDTSTART;TZID=Customized Time Zone:20270115T090000\r\nEND:VEVENT\r\n",
            custom_zone(chrono_tz::Tz::America__New_York)
        ));
        let all = occurrences(&ics, ts("2026-01-01T00:00:00Z"), ts("2028-01-01T00:00:00Z"));
        assert_eq!(all[0].start, ts("2026-07-15T13:00:00Z"));
        assert_eq!(all[0].end, ts("2026-07-15T14:00:00Z"));
        assert_eq!(all[1].start, ts("2027-01-15T14:00:00Z"));

        // Standard time only, and not on a whole hour.
        let ics = cal(
            "BEGIN:VTIMEZONE\r\nTZID:Head Office\r\nBEGIN:STANDARD\r\nDTSTART:16010101T000000\r\nTZOFFSETFROM:+0530\r\nTZOFFSETTO:+0530\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:i\r\nDTSTART;TZID=Head Office:20261118T090000\r\nRRULE:FREQ=DAILY;COUNT=2\r\nEND:VEVENT\r\n",
        );
        let all = occurrences(&ics, ts("2026-11-01T00:00:00Z"), ts("2026-12-01T00:00:00Z"));
        assert_eq!(
            all.iter().map(|o| o.start).collect::<Vec<_>>(),
            [ts("2026-11-18T03:30:00Z"), ts("2026-11-19T03:30:00Z")]
        );
        // Skipping a date keeps the zone's own name.
        let skipped = skip_occurrence(&ics, ts("2026-11-19T03:30:00Z")).unwrap();
        assert!(skipped.contains("EXDATE;TZID=Head Office:20261119T090000\r\n"));

        // Outlook's form: both halves from 1601, each with a yearly rule.
        let zones = calendar(&cal(
            "BEGIN:VTIMEZONE\r\nTZID:X\r\nBEGIN:STANDARD\r\nDTSTART:16010101T030000\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nRRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=10\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\nDTSTART:16010101T020000\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nRRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=3\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\n",
        ))
        .1;
        let at = |text: &str| naive(text).unwrap();
        assert_eq!(zones.offset("X", at("20260328T120000")), Some(3600));
        assert_eq!(zones.offset("X", at("20260329T120000")), Some(7200));
        assert_eq!(zones.offset("X", at("20261025T120000")), Some(3600));
        assert_eq!(zones.offset("X", at("20270101T000000")), Some(3600));
        assert_eq!(zones.offset("Y", at("20270101T000000")), None);
        assert_eq!(parse_offset("-0330"), Some(-12_600));
        assert_eq!(parse_offset("+053015"), Some(19_815));
        assert_eq!(parse_offset("0300"), Some(10_800));
        assert_eq!(parse_offset("+3"), None);
        assert_eq!(
            fixed_zone(-18_000, at("20260101T000000")).map(chrono_tz::Tz::name),
            Some("Etc/GMT+5")
        );
    }

    #[test]
    fn a_series_made_single_loses_its_changed_dates() {
        let original = cal(
            "BEGIN:VEVENT\r\nUID:d\r\nRECURRENCE-ID:20261104T070000Z\r\nDTSTART:20261104T100000Z\r\nSUMMARY:Stand-up (moved)\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:d\r\nDTSTART:20261102T070000Z\r\nDTEND:20261102T073000Z\r\nRRULE:FREQ=DAILY\r\nEXDATE:20261103T070000Z\r\nSUMMARY:Stand-up\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:d\r\nRECURRENCE-ID:20261105T070000Z\r\nDTSTART:20261105T090000Z\r\nSUMMARY:Stand-up (late)\r\nEND:VEVENT\r\n",
        );
        let once = edit_event(&original, &input(Repeat::None), 0).unwrap();
        assert_eq!(once.matches("BEGIN:VEVENT").count(), 1);
        assert_eq!(once.matches("END:VEVENT").count(), 1);
        assert!(!once.contains("RECURRENCE-ID") && !once.contains("moved"));
        assert!(!once.contains("RRULE") && !once.contains("EXDATE"));
        assert!(once.contains("BEGIN:VTIMEZONE"));
        let all = occurrences(&once, 0, ts("2030-01-01T00:00:00Z"));
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].title, "Design review");
        // Still a series: its changed dates stay.
        for repeat in [Repeat::Keep, Repeat::Weekly] {
            let kept = edit_event(&original, &input(repeat), 0).unwrap();
            assert_eq!(kept.matches("BEGIN:VEVENT").count(), 3);
        }
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

    const INVITE: &str = "BEGIN:VCALENDAR\r\nPRODID:-//Google Inc//Google Calendar 70.9054//EN\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VTIMEZONE\r\nTZID:Africa/Nairobi\r\nBEGIN:STANDARD\r\nDTSTART:19700101T000000\r\nTZOFFSETFROM:+0300\r\nTZOFFSETTO:+0300\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nDTSTART;TZID=Africa/Nairobi:20261124T140000\r\nDTEND;TZID=Africa/Nairobi:20261124T150000\r\nDTSTAMP:20261118T090000Z\r\nORGANIZER;CN=Peter Mwangi:mailto:peter@client.example\r\nUID:abc123@google.com\r\nATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN=info@mocky.co.ke:mailto:info@mocky.co.ke\r\nATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED;RSVP=TRUE;CN=Peter Mwangi:mailto:peter@client.example\r\nSEQUENCE:0\r\nSTATUS:CONFIRMED\r\nSUMMARY:Site visit: Kilimani\r\nLOCATION:Kilimani\\, Nairobi\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT10M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn reads_an_invitation() {
        let inv = invitation(INVITE, "INFO@mocky.co.ke").unwrap();
        assert_eq!(inv.method, "REQUEST");
        assert_eq!(inv.event.title, "Site visit: Kilimani");
        assert_eq!(inv.event.location, "Kilimani, Nairobi");
        assert_eq!(inv.event.start, ts("2026-11-24T11:00:00Z"));
        assert_eq!(
            inv.event.organizer.as_ref().unwrap().email,
            "peter@client.example"
        );
        assert_eq!(inv.my_status.as_deref(), Some("needs-action"));
        assert!(inv.replier.is_none());
        assert!(invitation("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n", "x@y").is_none());
    }

    #[test]
    fn replies_name_only_you_with_your_answer() {
        let text = reply(
            INVITE,
            "info@mocky.co.ke",
            Some("Mocky Digital"),
            Answer::Accepted,
            ts("2026-11-18T10:00:00Z"),
        )
        .unwrap();
        assert!(text.contains("METHOD:REPLY\r\n"));
        assert!(text.contains("UID:abc123@google.com\r\n"));
        assert!(text.contains("ORGANIZER;CN=Peter Mwangi:mailto:peter@client.example\r\n"));
        assert!(text.contains("BEGIN:VTIMEZONE"));
        // Long lines are folded; compare the unfolded text.
        assert!(unfold(&text).contains("ATTENDEE;PARTSTAT=ACCEPTED;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;CN=info@mocky.co.ke:mailto:info@mocky.co.ke\n"));
        assert!(!text.contains("peter@client.example\r\nATTENDEE"));
        assert_eq!(text.matches("ATTENDEE").count(), 1);
        assert!(!text.contains("VALARM"));
        let back = invitation(&text, "peter@client.example").unwrap();
        assert_eq!(back.method, "REPLY");
        assert_eq!(back.replier.unwrap().status.as_deref(), Some("accepted"));

        // Not listed by name (a group invite): you are added.
        let group = INVITE.replace("mailto:info@mocky.co.ke", "mailto:team@mocky.co.ke");
        let text = reply(
            &group,
            "info@mocky.co.ke",
            Some("Mocky, Digital"),
            Answer::Declined,
            0,
        )
        .unwrap();
        assert!(text.contains(
            "ATTENDEE;PARTSTAT=DECLINED;CN=\"Mocky, Digital\":mailto:info@mocky.co.ke\r\n"
        ));
    }

    #[test]
    fn the_calendar_copy_keeps_the_invitation_with_your_answer() {
        let copy = calendar_copy(INVITE, "info@mocky.co.ke", None, Answer::Tentative).unwrap();
        assert!(!copy.contains("METHOD:"));
        assert!(copy.contains("PARTSTAT=TENTATIVE"));
        assert!(!copy.contains("RSVP=TRUE;CN=info"));
        assert!(copy.contains("BEGIN:VALARM"));
        assert_eq!(copy.matches("ATTENDEE").count(), 2);
        let shown = master(&copy).unwrap();
        assert_eq!(shown.title, "Site visit: Kilimani");
        assert_eq!(
            invitation(&copy, "info@mocky.co.ke")
                .unwrap()
                .my_status
                .as_deref(),
            Some("tentative")
        );

        // An answer from Peter updates his line in your copy.
        let answer = reply(
            &copy.replace(
                "PARTSTAT=ACCEPTED;RSVP=TRUE;CN=Peter",
                "PARTSTAT=NEEDS-ACTION;CN=Peter",
            ),
            "peter@client.example",
            None,
            Answer::Declined,
            0,
        )
        .unwrap();
        let updated = apply_reply(&copy, &answer).unwrap();
        let people = master(&updated).unwrap().attendees;
        let peter = people
            .iter()
            .find(|p| p.email == "peter@client.example")
            .unwrap();
        assert_eq!(peter.status.as_deref(), Some("declined"));
        assert!(apply_reply(&updated, &answer).is_none());
    }

    #[test]
    fn reminders_are_read_and_written() {
        let ics = cal(
            "BEGIN:VEVENT\r\nUID:r\r\nDTSTART:20261118T060000Z\r\nDTEND:20261118T070000Z\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nBEGIN:VALARM\r\nACTION:AUDIO\r\nTRIGGER;RELATED=START:-P1D\r\nEND:VALARM\r\nBEGIN:VALARM\r\nACTION:EMAIL\r\nTRIGGER:-PT5M\r\nEND:VALARM\r\nBEGIN:VALARM\r\nTRIGGER;VALUE=DATE-TIME:20261118T055500Z\r\nEND:VALARM\r\nBEGIN:VALARM\r\nTRIGGER;RELATED=END:-PT90M\r\nEND:VALARM\r\nBEGIN:VALARM\r\nTRIGGER:PT10M\r\nEND:VALARM\r\nEND:VEVENT\r\n",
        );
        // 1 day, 30 min (an hour before the end, 90 min), 15 min, 5 min
        // (absolute); not the email or the one after the start.
        assert_eq!(master(&ics).unwrap().reminders, [1440, 30, 15, 5]);

        let mut with = input(Repeat::None);
        with.reminders = Some(vec![10, 60, 10]);
        let made = new_event("u", &with, 0);
        assert_eq!(made.matches("BEGIN:VALARM").count(), 2);
        assert_eq!(master(&made).unwrap().reminders, [60, 10]);

        // Editing: None keeps them; Some replaces them, other parts stay.
        let kept = edit_event(&ics, &input(Repeat::Keep), 0).unwrap();
        assert_eq!(master(&kept).unwrap().reminders, [1440, 30, 15, 5]);
        let mut change = input(Repeat::Keep);
        change.reminders = Some(vec![0]);
        let changed = edit_event(&ics, &change, 0).unwrap();
        assert_eq!(master(&changed).unwrap().reminders, [0]);
        assert_eq!(changed.matches("BEGIN:VALARM").count(), 1);
        assert_eq!(changed.matches("END:VALARM").count(), 1);
        change.reminders = Some(vec![]);
        assert!(!edit_event(&ics, &change, 0).unwrap().contains("VALARM"));
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
            let _ = invitation(text, "a@b");
            let _ = reply(text, "a@b", None, Answer::Accepted, 0);
            let _ = calendar_copy(text, "a@b", None, Answer::Declined);
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

    fn rule_of(ics: &str) -> Option<RepeatRule> {
        master(ics).unwrap().repeat_rule
    }

    fn series(rule: &str) -> String {
        cal(&format!(
            "BEGIN:VEVENT\r\nUID:r\r\nDTSTART;TZID=Africa/Nairobi:20261103T090000\r\nDTEND;TZID=Africa/Nairobi:20261103T100000\r\nRRULE:{rule}\r\nEND:VEVENT\r\n"
        ))
    }

    #[test]
    fn rules_the_repeat_dialog_can_show() {
        let every_two_weeks =
            rule_of(&series("FREQ=WEEKLY;INTERVAL=2;BYDAY=TU,TH;WKST=MO")).unwrap();
        assert_eq!(every_two_weeks.freq, Freq::Weekly);
        assert_eq!(every_two_weeks.interval, 2);
        assert_eq!(every_two_weeks.by_day, ["TU", "TH"]);
        // "The second Tuesday", both ways of writing it.
        for rule in ["FREQ=MONTHLY;BYDAY=2TU", "FREQ=MONTHLY;BYDAY=TU;BYSETPOS=2"] {
            let r = rule_of(&series(rule)).unwrap();
            assert_eq!(r.by_day, ["TU"]);
            assert_eq!(r.set_pos, Some(2));
        }
        let last_friday = rule_of(&series("FREQ=MONTHLY;BYDAY=-1FR;COUNT=6")).unwrap();
        assert_eq!(
            (last_friday.set_pos, last_friday.count),
            (Some(-1), Some(6))
        );
        let yearly = rule_of(&series(
            "FREQ=YEARLY;BYMONTH=12;BYMONTHDAY=25;UNTIL=20301225T060000Z",
        ))
        .unwrap();
        assert_eq!((yearly.month, yearly.month_day), (Some(12), Some(25)));
        assert_eq!(yearly.until, Some(ts("2030-12-25T00:00:00Z")));
        // A plain daily rule is a rule too (repeat says "daily").
        let daily = master(&series("FREQ=DAILY")).unwrap();
        assert_eq!(daily.repeat, Repeat::Daily);
        assert_eq!(daily.repeat_rule.unwrap().interval, 1);
        // Rules it can't show: custom, kept as they are.
        for odd in [
            "FREQ=HOURLY",
            "FREQ=MONTHLY;BYMONTHDAY=1,15",
            "FREQ=YEARLY;BYWEEKNO=20",
            "FREQ=MONTHLY;BYDAY=MO",
            "FREQ=DAILY;BYDAY=MO,TU",
            "FREQ=MONTHLY;BYDAY=5TU",
        ] {
            let shown = master(&series(odd)).unwrap();
            assert_eq!(shown.repeat_rule, None, "{odd}");
            assert!(shown.recurring);
        }
        let single = cal("BEGIN:VEVENT\r\nUID:x\r\nDTSTART:20261103T090000Z\r\nEND:VEVENT\r\n");
        assert_eq!(master(&single).unwrap().repeat_rule, None);
    }

    #[test]
    fn repeat_rules_are_written_and_read_back() {
        let rules = [
            RepeatRule {
                freq: Freq::Weekly,
                interval: 2,
                by_day: vec!["MO".into(), "WE".into()],
                month_day: None,
                month: None,
                set_pos: None,
                count: Some(10),
                until: None,
            },
            RepeatRule {
                freq: Freq::Monthly,
                interval: 1,
                by_day: vec!["TU".into()],
                month_day: None,
                month: None,
                set_pos: Some(2),
                count: None,
                until: Some(ts("2027-06-30T00:00:00Z")),
            },
            RepeatRule {
                freq: Freq::Yearly,
                interval: 1,
                by_day: vec![],
                month_day: Some(18),
                month: Some(11),
                set_pos: None,
                count: None,
                until: None,
            },
        ];
        for rule in rules {
            let made = new_event(
                "u",
                &EventInput {
                    repeat_rule: Some(rule.clone()),
                    ..input(Repeat::None)
                },
                0,
            );
            assert_eq!(rule_of(&made).as_ref(), Some(&rule), "{made}");
        }
        // UNTIL is the end of that day where the event is.
        let text = rule_text(
            &RepeatRule {
                freq: Freq::Daily,
                interval: 1,
                by_day: vec![],
                month_day: None,
                month: None,
                set_pos: None,
                count: None,
                until: Some(ts("2026-11-30T00:00:00Z")),
            },
            false,
            Some(chrono_tz::Africa::Nairobi),
        );
        assert_eq!(text, "FREQ=DAILY;UNTIL=20261130T205959Z");
        // The last date counts.
        let ics = new_event(
            "u",
            &EventInput {
                start: ts("2026-11-03T06:00:00Z"),
                end: ts("2026-11-03T07:00:00Z"),
                repeat_rule: rule_of(&series("FREQ=DAILY;UNTIL=20261105")),
                ..input(Repeat::None)
            },
            0,
        );
        let dates = occurrences(&ics, ts("2026-11-01T00:00:00Z"), ts("2026-12-01T00:00:00Z"));
        assert_eq!(dates.len(), 3);
        // A custom rule replaces the series' rule when editing.
        let edited = edit_event(
            &series("FREQ=DAILY"),
            &EventInput {
                repeat_rule: rule_of(&series("FREQ=WEEKLY;INTERVAL=3")),
                ..input(Repeat::Keep)
            },
            0,
        )
        .unwrap();
        assert!(edited.contains("RRULE:FREQ=WEEKLY;INTERVAL=3\r\n"));
        assert_eq!(edited.matches("RRULE").count(), 1);
    }

    #[test]
    fn show_as_and_private() {
        let read = |props: &str| {
            master(&cal(&format!(
                "BEGIN:VEVENT\r\nUID:x\r\nDTSTART:20261103T090000Z\r\n{props}END:VEVENT\r\n"
            )))
            .unwrap()
        };
        assert_eq!(read("").show_as, ShowAs::Busy);
        assert_eq!(read("TRANSP:TRANSPARENT\r\n").show_as, ShowAs::Free);
        assert_eq!(read("STATUS:TENTATIVE\r\n").show_as, ShowAs::Tentative);
        assert_eq!(
            read("X-MICROSOFT-CDO-BUSYSTATUS:OOF\r\nTRANSP:OPAQUE\r\n").show_as,
            ShowAs::Away
        );
        assert!(read("CLASS:PRIVATE\r\n").private);
        assert!(read("CLASS:CONFIDENTIAL\r\n").private);
        assert!(!read("CLASS:PUBLIC\r\n").private);

        let made = new_event(
            "u",
            &EventInput {
                show_as: Some(ShowAs::Free),
                private: Some(true),
                ..input(Repeat::None)
            },
            0,
        );
        let shown = master(&made).unwrap();
        assert_eq!((shown.show_as, shown.private), (ShowAs::Free, true));
        // None leaves them; Some replaces them.
        let kept = edit_event(&made, &input(Repeat::Keep), 0).unwrap();
        assert_eq!(master(&kept).unwrap().show_as, ShowAs::Free);
        let away = edit_event(
            &made,
            &EventInput {
                show_as: Some(ShowAs::Away),
                private: Some(false),
                ..input(Repeat::Keep)
            },
            0,
        )
        .unwrap();
        let shown = master(&away).unwrap();
        assert_eq!((shown.show_as, shown.private), (ShowAs::Away, false));
        assert_eq!(away.matches("TRANSP").count(), 1);
        assert_eq!(away.matches("CLASS").count(), 1);
    }

    #[allow(clippy::unnecessary_wraps, reason = "as the editor field holds it")]
    fn me() -> Option<Person> {
        Some(Person {
            email: "info@mocky.co.ke".into(),
            name: Some("Mocky Digital".into()),
            status: None,
        })
    }

    #[test]
    fn inviting_people() {
        let made = new_event(
            "u",
            &EventInput {
                attendees: Some(vec![
                    "peter@client.example".into(),
                    "PETER@client.example".into(),
                    "bad;value@x".into(),
                ]),
                organizer: me(),
                ..input(Repeat::None)
            },
            0,
        );
        let mut shown = master(&made).unwrap();
        assert_eq!(shown.organizer.as_ref().unwrap().email, "info@mocky.co.ke");
        assert_eq!(shown.attendees.len(), 1);
        assert_eq!(shown.attendees[0].status.as_deref(), Some("needs-action"));
        shown.mark_organizer("Info@Mocky.co.ke");
        assert!(shown.is_organizer);
        shown.mark_organizer("peter@client.example");
        assert!(!shown.is_organizer);
        // Someone who answered keeps their answer; a new person is added;
        // someone taken off goes.
        let answered = made.replace("PARTSTAT=NEEDS-ACTION", "PARTSTAT=ACCEPTED");
        let edited = edit_event(
            &answered,
            &EventInput {
                attendees: Some(vec![
                    "peter@client.example".into(),
                    "amina@studio.example".into(),
                ]),
                organizer: me(),
                ..input(Repeat::Keep)
            },
            0,
        )
        .unwrap();
        let people = master(&edited).unwrap().attendees;
        assert_eq!(people.len(), 2);
        assert_eq!(people[0].status.as_deref(), Some("accepted"));
        assert_eq!(edited.matches("ORGANIZER").count(), 1);
        let fewer = edit_event(
            &edited,
            &EventInput {
                attendees: Some(vec!["amina@studio.example".into()]),
                organizer: me(),
                ..input(Repeat::Keep)
            },
            0,
        )
        .unwrap();
        assert_eq!(master(&fewer).unwrap().attendees.len(), 1);
        // No list: the attendees stay as they are.
        let same = edit_event(&edited, &input(Repeat::Keep), 0).unwrap();
        assert_eq!(master(&same).unwrap().attendees.len(), 2);
        // Someone else's meeting keeps its organiser, whoever edits it.
        let theirs = made.replace("mailto:info@mocky.co.ke", "mailto:lead@client.example");
        assert_eq!(
            master(&theirs)
                .unwrap()
                .organizer
                .map(|p| p.email)
                .as_deref(),
            Some("lead@client.example")
        );
        for organizer in [None, me()] {
            let edited = edit_event(
                &theirs,
                &EventInput {
                    attendees: Some(vec!["amina@studio.example".into()]),
                    organizer,
                    ..input(Repeat::Keep)
                },
                0,
            )
            .unwrap();
            let shown = master(&edited).unwrap();
            assert_eq!(
                shown.organizer.map(|p| p.email).as_deref(),
                Some("lead@client.example")
            );
            assert_eq!(edited.matches("ORGANIZER").count(), 1);
        }
    }

    #[test]
    fn editing_one_date_of_a_series() {
        let original = series("FREQ=DAILY;COUNT=5");
        let rid = ts("2026-11-04T06:00:00Z");
        let moved = EventInput {
            title: "Stand-up (moved)".into(),
            start: ts("2026-11-04T08:00:00Z"),
            end: ts("2026-11-04T08:30:00Z"),
            occurrence: Some(rid),
            scope: Scope::This,
            ..input(Repeat::Keep)
        };
        let edited = edit_occurrence(&original, rid, &moved, 0).unwrap();
        assert!(edited.contains("RECURRENCE-ID;TZID=Africa/Nairobi:20261104T090000\r\n"));
        assert_eq!(edited.matches("RRULE").count(), 1);
        let all = occurrences(
            &edited,
            ts("2026-11-01T00:00:00Z"),
            ts("2026-12-01T00:00:00Z"),
        );
        assert_eq!(all.len(), 5);
        let one = all.iter().find(|o| o.recurrence_id == Some(rid)).unwrap();
        assert_eq!(one.title, "Stand-up (moved)");
        assert_eq!(one.start, ts("2026-11-04T08:00:00Z"));
        assert!(one.recurring);
        // Edited again: the same changed copy is replaced, not added.
        let again = edit_occurrence(
            &edited,
            rid,
            &EventInput {
                title: "Stand-up (again)".into(),
                ..moved.clone()
            },
            0,
        )
        .unwrap();
        assert_eq!(again.matches("RECURRENCE-ID").count(), 1);
        assert!(again.contains("Stand-up (again)"));
        let dates = occurrences(
            &again,
            ts("2026-11-01T00:00:00Z"),
            ts("2026-12-01T00:00:00Z"),
        );
        assert_eq!(dates.len(), 5);
        // No series: nothing to change one date of.
        let lone = cal("BEGIN:VEVENT\r\nRECURRENCE-ID:20261103T090000Z\r\nEND:VEVENT\r\n");
        assert!(edit_occurrence(&lone, rid, &moved, 0).is_none());
    }

    #[test]
    fn invitations_and_cancellations_to_send() {
        let made = new_event(
            "meet-1",
            &EventInput {
                attendees: Some(vec![
                    "peter@client.example".into(),
                    "amina@studio.example".into(),
                ]),
                organizer: me(),
                reminders: Some(vec![15]),
                ..input(Repeat::Weekly)
            },
            0,
        );
        let request = itip_request(&made).unwrap();
        assert!(request.starts_with("BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\n"));
        assert!(!request.contains("VALARM"));
        let read = invitation(&request, "peter@client.example").unwrap();
        assert_eq!(read.method, "REQUEST");
        assert_eq!(read.my_status.as_deref(), Some("needs-action"));

        let cancel = itip_cancel(&made, None, None, ts("2026-11-20T00:00:00Z")).unwrap();
        assert!(cancel.contains("METHOD:CANCEL\r\n"));
        assert!(cancel.contains("STATUS:CANCELLED\r\n"));
        assert!(cancel.contains("SEQUENCE:1\r\n"));
        assert_eq!(cancel.matches("ATTENDEE").count(), 2);
        let read = invitation(&cancel, "amina@studio.example").unwrap();
        assert_eq!(read.method, "CANCEL");
        // Only the people taken off the list.
        let only = ["amina@studio.example".to_owned()];
        let cancel = itip_cancel(&made, None, Some(&only), 0).unwrap();
        assert_eq!(cancel.matches("ATTENDEE").count(), 1);
        // One date of the series.
        let rid = ts("2026-11-25T06:00:00Z");
        let one = itip_cancel(&made, Some(rid), None, 0).unwrap();
        assert!(one.contains("RECURRENCE-ID;TZID=Africa/Nairobi:20261125T090000\r\n"));
        assert!(!one.contains("RRULE"));
        assert_eq!(
            invitation(&one, "x@y").unwrap().event.recurrence_id,
            Some(rid)
        );
        assert!(itip_request("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n").is_none());
    }

    #[test]
    fn free_busy_answers_are_read() {
        let answer = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REPLY\r\nBEGIN:VFREEBUSY\r\nATTENDEE:mailto:peter@client.example\r\nDTSTART:20261118T000000Z\r\nDTEND:20261119T000000Z\r\nFREEBUSY:20261118T060000Z/20261118T070000Z,20261118T090000Z/PT30M\r\nFREEBUSY;FBTYPE=BUSY-TENTATIVE:20261118T120000Z/20261118T130000Z\r\nFREEBUSY;FBTYPE=FREE:20261118T140000Z/20261118T150000Z\r\nFREEBUSY;FBTYPE=BUSY-UNAVAILABLE:20261118T150000Z/20261118T160000Z\r\nEND:VFREEBUSY\r\nEND:VCALENDAR\r\n";
        let busy = free_busy_periods(answer);
        assert_eq!(busy.len(), 4);
        assert_eq!(
            busy[0],
            BusyPeriod {
                start: ts("2026-11-18T06:00:00Z"),
                end: ts("2026-11-18T07:00:00Z"),
                show_as: ShowAs::Busy
            }
        );
        assert_eq!(busy[1].end - busy[1].start, 1800);
        assert_eq!(busy[2].show_as, ShowAs::Tentative);
        assert_eq!(busy[3].show_as, ShowAs::Away);
        assert_eq!(
            free_busy_attendee(answer).as_deref(),
            Some("peter@client.example")
        );
        let request = free_busy_request(
            "info@mocky.co.ke",
            &["peter@client.example".into()],
            ts("2026-11-18T00:00:00Z"),
            ts("2026-11-19T00:00:00Z"),
            "fb-1",
            0,
        );
        assert!(request.contains("BEGIN:VFREEBUSY\r\n"));
        assert!(request.contains("ATTENDEE:mailto:peter@client.example\r\n"));
        assert!(request.contains("DTSTART:20261118T000000Z\r\n"));
        assert!(free_busy_periods("nonsense").is_empty());
    }

    #[test]
    fn to_dos_are_read_and_written() {
        let sogo = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Inverse inc./SOGo//EN\r\nBEGIN:VTODO\r\nUID:t-1\r\nSUMMARY:Send the quote\r\nDESCRIPTION:Kilimani fit-out\r\nDUE;VALUE=DATE:20261120\r\nPRIORITY:1\r\nCATEGORIES:Clients,Red\r\nSTATUS:NEEDS-ACTION\r\nX-SOGO-COLOR:red\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER;RELATED=END:-PT60M\r\nEND:VALARM\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let (uid, task) = parse_todo(sogo).unwrap();
        assert_eq!(uid, "t-1");
        assert_eq!(task.title, "Send the quote");
        assert_eq!(task.notes, "Kilimani fit-out");
        assert_eq!(task.due, Some(ts("2026-11-20T00:00:00Z")));
        assert_eq!(task.reminder, Some(ts("2026-11-19T23:00:00Z")));
        assert_eq!(task.importance, Importance::High);
        assert_eq!(task.categories, ["Clients", "Red"]);
        assert!(!task.completed);

        let done = TaskFields {
            completed: true,
            completed_at: Some(ts("2026-11-19T10:00:00Z")),
            reminder: Some(ts("2026-11-19T08:00:00Z")),
            importance: Importance::Low,
            ..task
        };
        let written = write_todo("t-1", &done, Some(sogo), ts("2026-11-19T10:00:00Z"));
        assert!(written.contains("X-SOGO-COLOR:red\r\n"));
        assert!(written.contains("STATUS:COMPLETED\r\n"));
        assert!(written.contains("PRIORITY:9\r\n"));
        assert!(written.contains("SEQUENCE:1\r\n"));
        assert_eq!(written.matches("BEGIN:VALARM").count(), 1);
        assert_eq!(written.matches("UID:").count(), 1);
        let (_, back) = parse_todo(&written).unwrap();
        assert_eq!(back, done);

        // A new one, due at a time.
        let fresh = TaskFields {
            title: "Call Peter".into(),
            due: Some(ts("2026-11-20T07:30:00Z")),
            ..TaskFields::default()
        };
        let written = write_todo("t-2", &fresh, None, 0);
        assert!(written.contains("BEGIN:VTODO\r\n"));
        assert!(written.contains("DUE:20261120T073000Z\r\n"));
        assert_eq!(parse_todo(&written).unwrap().1, fresh);
        // Events are not to-dos.
        assert!(parse_todo(&series("FREQ=DAILY")).is_none());
    }
}
