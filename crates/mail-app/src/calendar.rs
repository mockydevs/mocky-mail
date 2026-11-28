//! The calendar: commands for the Calendar screen, and syncing each
//! account's calendars: password accounts over `CalDAV` (for example
//! `SOGo` on the account's mail server), connected Gmail accounts over
//! Google's `CalDAV` (with a bearer token), connected Microsoft accounts
//! over Microsoft Graph.
//!
//! The device keeps a copy of every event, so the calendar shows without
//! a connection. Adding, editing and deleting go to the server straight
//! away (and need a connection), so nothing is ever half-saved.

use std::sync::Arc;
use std::time::Duration;

use mail_core::ical::{self, EventInput, Occurrence, Person, Repeat, RepeatRule, Scope, ShowAs};
use mail_core::message::Mailbox;
use mail_store::{Account, Calendar, CalendarProvider, FoundCalendar, StoredEvent};
use mail_sync::dav::{DavClient, DavLogin};
use mail_sync::{Credentials, SyncError, google, graph};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::Ctx;
use crate::connections::{self, Service};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, now};

/// How often calendars (and tasks) are synced in the background.
const SYNC_EVERY: Duration = Duration::from_mins(15);
/// Longest stretch one `list_events` covers: a little over a year.
const MAX_WINDOW: i64 = 400 * 86_400;
/// Longest stretch one free/busy question covers.
const MAX_FREE_BUSY_WINDOW: i64 = 62 * 86_400;
/// Most people in one free/busy question.
const MAX_FREE_BUSY_PEOPLE: usize = 50;
/// Most people invited to one event.
const MAX_ATTENDEES: usize = 100;
/// The stretch of a Microsoft calendar kept here: 90 days back, a year on.
const GRAPH_BACK: i64 = 90 * 86_400;
const GRAPH_AHEAD: i64 = 366 * 86_400;
const MAX_TITLE: usize = 500;
const MAX_TEXT: usize = 8000;

/// Whether an account's calendars sync with its server.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarStatus {
    account_id: String,
    /// `synced`, `unavailable` (no calendar on the server, or not
    /// connected), or `error`.
    state: &'static str,
    /// Plain words for the user.
    message: Option<String>,
    /// A Gmail or Microsoft account that can be connected.
    connectable: bool,
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

/// A busy stretch for Find a Time.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeBusy {
    email: String,
    busy: Vec<ical::BusyPeriod>,
    /// The person's calendar couldn't be read.
    unknown: bool,
}

fn tidy(text: &str, max: usize, lines: bool) -> String {
    text.chars()
        .filter(|c| !c.is_control() || (lines && *c == '\n'))
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Check a custom repeat rule from the repeat dialog.
fn checked_rule(rule: &mut RepeatRule, start: i64) -> AppResult<()> {
    const DAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];
    if !(1..=999).contains(&rule.interval) {
        return Err(AppError::invalid("Choose how often it repeats (1 to 999)."));
    }
    let mut days: Vec<String> = Vec::new();
    for day in &rule.by_day {
        let day = day.trim().to_ascii_uppercase();
        if !DAYS.contains(&day.as_str()) {
            return Err(AppError::invalid("Choose the days again."));
        }
        if !days.contains(&day) {
            days.push(day);
        }
    }
    rule.by_day = days;
    if rule
        .month_day
        .is_some_and(|d| !(d == -1 || (1..=31).contains(&d)))
        || rule.month.is_some_and(|m| !(1..=12).contains(&m))
        || rule
            .set_pos
            .is_some_and(|p| !(p == -1 || (1..=4).contains(&p)))
        || (rule.set_pos.is_some() && rule.by_day.len() != 1)
    {
        return Err(AppError::invalid("Choose the repeat again."));
    }
    if rule.count.is_some_and(|c| !(1..=999).contains(&c)) {
        return Err(AppError::invalid("Choose how many times (1 to 999)."));
    }
    if rule.count.is_some() && rule.until.is_some() {
        rule.until = None;
    }
    if rule.until.is_some_and(|u| u < start.saturating_sub(86_400)) {
        return Err(AppError::invalid(
            "The repeat ends before the event starts.",
        ));
    }
    Ok(())
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
    if let Some(reminders) = &input.reminders
        && (reminders.len() > 5 || reminders.iter().any(|m| *m > 4 * 7 * 24 * 60))
    {
        return Err(AppError::invalid(
            "Choose up to five reminders, at most four weeks before.",
        ));
    }
    if let Some(rule) = input.repeat_rule.as_mut() {
        checked_rule(rule, input.start)?;
    }
    if let Some(people) = &input.attendees {
        if people.len() > MAX_ATTENDEES {
            return Err(AppError::invalid("Invite up to 100 people."));
        }
        let mut clean = Vec::new();
        for person in people {
            let person = tidy(person, 320, false);
            if person.is_empty() {
                continue;
            }
            let parsed = mail_core::compose::parse_address_list(&person)
                .ok()
                .filter(|list| list.len() == 1)
                .and_then(|list| list.into_iter().next())
                .ok_or_else(|| AppError::invalid(format!("{person} isn't an email address.")))?;
            clean.push(parsed.address);
        }
        input.attendees = Some(clean);
    }
    if input.scope == Scope::This && input.occurrence.is_none() {
        input.scope = Scope::All;
    }
    input.organizer = None;
    Ok(input)
}

fn with_place(
    calendar: &Calendar,
    id: String,
    mut occurrence: Occurrence,
    me: Option<&str>,
) -> CalendarEvent {
    if let Some(me) = me {
        occurrence.mark_organizer(me);
    }
    CalendarEvent {
        id,
        calendar_id: calendar.id.clone(),
        account_id: calendar.account_id.clone(),
        color: calendar.color.clone(),
        occurrence,
    }
}

/// `list_calendars`: every account's calendars.
pub fn list_calendars(ctx: &Ctx) -> AppResult<Vec<Calendar>> {
    let state = &ctx.state;
    Ok(state.store.list_calendars()?)
}

/// `set_calendar_visible`: show or hide a calendar (this device only).
pub fn set_calendar_visible(
    ctx: &Ctx,
    calendar_id: String,
    visible: bool,
) -> AppResult<Vec<Calendar>> {
    let state = &ctx.state;
    state.store.set_calendar_visible(&calendar_id, visible)?;
    changed(ctx, &[]);
    Ok(state.store.list_calendars()?)
}

/// Whether an account (by address) declined this occurrence.
fn declined(occurrence: &Occurrence, me: Option<&str>) -> bool {
    me.is_some_and(|me| {
        occurrence
            .attendees
            .iter()
            .any(|p| p.email.eq_ignore_ascii_case(me) && p.status.as_deref() == Some("declined"))
    })
}

/// `list_events`: every occurrence between `from` and `to` (Unix seconds)
/// in the calendars that are shown.
pub fn list_events(ctx: &Ctx, from: i64, to: i64) -> AppResult<Vec<CalendarEvent>> {
    let state = &ctx.state;
    if to <= from || to.saturating_sub(from) > MAX_WINDOW {
        return Err(AppError::invalid("That date range is too long."));
    }
    let calendars = state.store.list_calendars()?;
    let accounts = state.store.list_accounts()?;
    let overrides = state.store.reminder_overrides()?;
    let mut out = Vec::new();
    for stored in state.store.calendar_events_between(from, to)? {
        let Some(calendar) = calendars
            .iter()
            .find(|c| c.id == stored.calendar_id && c.visible)
        else {
            continue;
        };
        let me = accounts
            .iter()
            .find(|a| a.id == calendar.account_id)
            .map(|a| a.email.as_str());
        for mut occurrence in ical::occurrences(&stored.ics, from, to) {
            overrides.apply(&stored.id, &mut occurrence);
            // Invitations you declined stay on the server (so it could tell
            // the organiser) but aren't shown.
            if !declined(&occurrence, me) {
                out.push(with_place(calendar, stored.id.clone(), occurrence, me));
            }
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
pub fn get_event(ctx: &Ctx, id: String) -> AppResult<CalendarEvent> {
    let state = &ctx.state;
    let stored = state.store.get_calendar_event(&id)?;
    let calendar = state.store.get_calendar(&stored.calendar_id)?;
    let account = state.store.get_account(&calendar.account_id)?;
    shown_event(state, &calendar, &account, &stored)
}

fn account_of(state: &AppState, calendar: &Calendar) -> AppResult<Account> {
    state
        .store
        .list_accounts()?
        .into_iter()
        .find(|a| a.id == calendar.account_id)
        .ok_or_else(|| AppError::invalid("That account was removed."))
}

/// A connection to the `CalDAV` server of a calendar's account (the mail
/// server's, or Google's).
pub(crate) async fn client_for(
    state: &AppState,
    calendar: &Calendar,
) -> AppResult<(DavClient, Url)> {
    if crate::ews_sync::is_ews_calendar(calendar) {
        return Err(AppError::invalid(
            "Exchange calendars are changed through Exchange: open the event in the calendar to change it.",
        ));
    }
    let account = account_of(state, calendar)?;
    let url = Url::parse(&calendar.url).map_err(|e| AppError::internal(e.to_string()))?;
    match calendar.provider {
        CalendarProvider::Google => Ok((
            DavClient::bearer(connections::google_token(state, &account).await?)?,
            url,
        )),
        CalendarProvider::Microsoft => Err(AppError::invalid(
            "Microsoft calendars are changed through Microsoft, not here.",
        )),
        CalendarProvider::Caldav => {
            let Credentials::Password { username, password } =
                crate::credentials::for_account(state, &account).await?
            else {
                return Err(AppError::invalid(
                    "This account's calendar can't be changed here.",
                ));
            };
            Ok((DavClient::new(DavLogin { username, password })?, url))
        }
    }
}

fn changed_elsewhere(error: SyncError) -> AppError {
    match error {
        SyncError::Refused(_) => AppError::invalid(
            "The calendar server didn't accept the change. The event may have been changed elsewhere; it has been refreshed, so try again.",
        ),
        other => other.into(),
    }
}

/// You, as organiser of invitations sent from this account.
fn me_of(account: &Account) -> Person {
    Person {
        email: account.email.clone(),
        name: Some(account.display_name.clone()).filter(|n| !n.trim().is_empty()),
        status: None,
    }
}

/// The people on an event other than you.
fn others(ics: &str, me: &str) -> Vec<Mailbox> {
    ical::master(ics)
        .map(|o| o.attendees)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| !p.email.eq_ignore_ascii_case(me))
        .map(|p| Mailbox {
            name: p.name,
            address: p.email,
        })
        .collect()
}

/// Whether you organise the stored event (or it has no organiser).
fn organises(ics: &str, me: &str) -> bool {
    ical::master(ics).is_none_or(|o| {
        o.organizer
            .as_ref()
            .is_none_or(|p| p.email.eq_ignore_ascii_case(me))
    })
}

/// Email invitations, updates and cancellations for a saved event: a
/// `REQUEST` to everyone on it, and a `CANCEL` to people taken off.
#[allow(clippy::too_many_arguments, reason = "the parts of one change")]
fn send_invitations(
    ctx: &Ctx,
    state: &Arc<AppState>,
    account: &Account,
    before: Option<&str>,
    after: &str,
    title: &str,
    occurrence: Option<i64>,
    stamp: i64,
) -> AppResult<()> {
    let me = account.email.as_str();
    let now_on = others(after, me);
    let removed: Vec<Mailbox> = before
        .map(|b| others(b, me))
        .unwrap_or_default()
        .into_iter()
        .filter(|p| {
            !now_on
                .iter()
                .any(|n| n.address.eq_ignore_ascii_case(&p.address))
        })
        .collect();
    let title = if title.trim().is_empty() {
        "(No title)"
    } else {
        title.trim()
    };
    if !now_on.is_empty()
        && let Some(request) = ical::itip_request(after)
    {
        let (word, note) = if before.is_some() {
            (
                "Updated invitation",
                format!("The invitation was updated: {title}"),
            )
        } else {
            ("Invitation", format!("You're invited: {title}"))
        };
        crate::invitations::queue_calendar_mail(
            ctx,
            state,
            account,
            now_on,
            &format!("{word}: {title}"),
            &note,
            &request,
            "REQUEST",
        )?;
    }
    if !removed.is_empty() {
        let only: Vec<String> = removed.iter().map(|m| m.address.clone()).collect();
        if let Some(cancel) =
            ical::itip_cancel(before.unwrap_or(after), occurrence, Some(&only), stamp)
        {
            crate::invitations::queue_calendar_mail(
                ctx,
                state,
                account,
                removed,
                &format!("Cancelled: {title}"),
                &format!("You're no longer invited: {title}"),
                &cancel,
                "CANCEL",
            )?;
        }
    }
    Ok(())
}

/// Whether a save of a stored event can only change its reminders here:
/// its calendar is read-only, or someone else organises it.
fn reminders_only(calendar: &Calendar, stored: &StoredEvent, me: &str) -> bool {
    !calendar.writable || !organises(&stored.ics, me)
}

/// Who is passed as organiser for a save: you, for a new event or one you
/// organise (or with no organiser yet). Never someone else's meeting, so
/// its organiser is never replaced.
fn organizer_for(existing: Option<&StoredEvent>, account: &Account) -> Option<Person> {
    existing
        .is_none_or(|stored| organises(&stored.ics, &account.email))
        .then(|| me_of(account))
}

/// The stored event as shown, with the reminders kept here.
fn shown_event(
    state: &AppState,
    calendar: &Calendar,
    account: &Account,
    stored: &StoredEvent,
) -> AppResult<CalendarEvent> {
    let mut occurrence = ical::master(&stored.ics)
        .ok_or_else(|| AppError::invalid("This calendar item isn't an event."))?;
    state
        .store
        .reminder_overrides()?
        .apply(&stored.id, &mut occurrence);
    Ok(with_place(
        calendar,
        stored.id.clone(),
        occurrence,
        Some(&account.email),
    ))
}

/// For an event that can't be changed on the server (see
/// [`reminders_only`]): save only its reminders, on this device, and give
/// it back. Nothing else the editor sent is used, and nothing goes to the
/// server. `None` when the event can be changed on the server.
fn save_reminders_here(
    ctx: &Ctx,
    state: &Arc<AppState>,
    stored: &StoredEvent,
    input: &EventInput,
) -> AppResult<Option<CalendarEvent>> {
    let calendar = state.store.get_calendar(&stored.calendar_id)?;
    let account = account_of(state, &calendar)?;
    if !reminders_only(&calendar, stored, &account.email) {
        return Ok(None);
    }
    let shown = keep_reminders_here(state, &calendar, &account, stored, input)?;
    if input.reminders.is_some() {
        changed(ctx, &[]);
        crate::reminders::refresh(ctx, state);
    }
    Ok(Some(shown))
}

/// The calendar an event is saved to on the server, and its account: one
/// that events can be added to.
fn writable_calendar(state: &AppState, calendar_id: &str) -> AppResult<(Calendar, Account)> {
    let calendar = state.store.get_calendar(calendar_id)?;
    if !calendar.writable {
        return Err(AppError::invalid(
            "Events can't be added to this calendar. Choose another one.",
        ));
    }
    let account = account_of(state, &calendar)?;
    Ok((calendar, account))
}

/// After saving on the server: reminders sent there are no longer kept
/// here (for that date, or the whole event).
fn reminders_on_server(state: &AppState, event_id: &str, input: &EventInput) -> AppResult<()> {
    if input.reminders.is_some() {
        let one_date = input.occurrence.filter(|_| input.scope == Scope::This);
        state.store.clear_reminder_override(event_id, one_date)?;
    }
    Ok(())
}

/// The store side of [`save_reminders_here`]: `reminders` (`None`: as
/// they are) for the whole event, or with `scope: this` for that date.
fn keep_reminders_here(
    state: &AppState,
    calendar: &Calendar,
    account: &Account,
    stored: &StoredEvent,
    input: &EventInput,
) -> AppResult<CalendarEvent> {
    if let Some(minutes) = &input.reminders {
        let one_date = input.occurrence.filter(|_| input.scope == Scope::This);
        state
            .store
            .set_reminder_override(&stored.id, one_date, minutes)?;
    }
    shown_event(state, calendar, account, stored)
}

/// `save_event`: create or change an event on the server, then here.
#[allow(
    clippy::too_many_lines,
    reason = "the CalDAV save, step by step, after the Microsoft and Exchange ones"
)]
pub async fn save_event(ctx: &Ctx, event: EventSave) -> AppResult<CalendarEvent> {
    let state = &ctx.state;
    let mut input = checked(event.event)?;
    let existing = match &event.id {
        Some(id) => Some(state.store.get_calendar_event(id)?),
        None => None,
    };
    // An event you can't change on the server (read-only calendar, or a
    // meeting someone else organises): only its reminders, kept here.
    if let Some(stored) = &existing
        && let Some(shown) = save_reminders_here(ctx, state, stored, &input)?
    {
        return Ok(shown);
    }
    let (calendar, account) = writable_calendar(state, &event.calendar_id)?;
    input.organizer = organizer_for(existing.as_ref(), &account);
    if calendar.provider == CalendarProvider::Microsoft {
        return save_graph_event(ctx, state, &account, &calendar, existing, &input).await;
    }
    if account.is_ews() {
        return save_ews_event(ctx, &account, &calendar, existing.as_ref(), &input).await;
    }
    let (client, url) = client_for(state, &calendar).await?;
    let stamp = i64::try_from(now()).unwrap_or(0);
    let one_date = input.scope == Scope::This;
    let edit = |stored: &StoredEvent| -> AppResult<String> {
        let text = match input.occurrence.filter(|_| one_date) {
            Some(rid) => ical::edit_occurrence(&stored.ics, rid, &input, stamp),
            None => ical::edit_event(&stored.ics, &input, stamp),
        };
        text.ok_or_else(|| AppError::invalid("This calendar item isn't an event."))
    };
    let result = match &existing {
        // Edited in place.
        Some(stored) if stored.calendar_id == calendar.id => {
            let ics = edit(stored)?;
            let uid = ical::uid_of(&ics).unwrap_or_default();
            client
                .put_event(&url, Some(&stored.href), &uid, &ics, stored.etag.as_deref())
                .await
                .map(|(href, etag)| (href, etag, ics))
        }
        // Moved to another calendar: saved there, then removed here.
        Some(stored) => {
            let old = state.store.get_calendar(&stored.calendar_id)?;
            if old.provider == CalendarProvider::Microsoft {
                return Err(AppError::invalid(
                    "Events can't move between a Microsoft calendar and another one. Create it again in the other calendar.",
                ));
            }
            let ics = edit(stored)?;
            let uid = ical::uid_of(&ics).unwrap_or_default();
            let saved = client.put_event(&url, None, &uid, &ics, None).await;
            if saved.is_ok() {
                let (old_client, old_url) = client_for(state, &old).await?;
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
            soon(ctx, state);
            return Err(changed_elsewhere(error));
        }
    };
    let id = state
        .store
        .put_calendar_event(&calendar.id, &href, etag.as_deref(), &ics)?;
    reminders_on_server(state, &id, &input)?;
    // Invitations: the server sends them itself if it schedules (SOGo,
    // Google); otherwise they go out through the Outbox.
    if input.notify_attendees
        && organises(&ics, &account.email)
        && !client.auto_schedules(&url).await.unwrap_or(false)
    {
        send_invitations(
            ctx,
            state,
            &account,
            existing.as_ref().map(|e| e.ics.as_str()),
            &ics,
            &input.title,
            input.occurrence.filter(|_| one_date),
            stamp,
        )?;
    }
    changed(ctx, &[]);
    let occurrence =
        ical::master(&ics).ok_or_else(|| AppError::internal("saved event unreadable"))?;
    let mut shown = with_place(&calendar, id, occurrence, Some(&account.email));
    state
        .store
        .reminder_overrides()?
        .apply(&shown.id, &mut shown.occurrence);
    Ok(shown)
}

/// Save an event in an Exchange calendar (Exchange sends the invitations),
/// then show it.
async fn save_ews_event(
    ctx: &Ctx,
    account: &Account,
    calendar: &Calendar,
    existing: Option<&StoredEvent>,
    input: &EventInput,
) -> AppResult<CalendarEvent> {
    let state = &ctx.state;
    let rule = graph_rule(input);
    let stored = crate::ews_sync::save_event(
        state,
        account,
        calendar,
        existing,
        input,
        rule.as_ref().map(Option::as_ref),
    )
    .await?;
    changed(ctx, &[]);
    reminders_on_server(state, &stored.id, input)?;
    shown_event(state, calendar, account, &stored)
}

/// What an edit asks of a Microsoft series' rule: `None` leaves it,
/// `Some(None)` removes it.
#[allow(clippy::option_option, reason = "keep, remove or replace")]
fn graph_rule(input: &EventInput) -> Option<Option<RepeatRule>> {
    if let Some(rule) = &input.repeat_rule {
        return Some(Some(rule.clone()));
    }
    let simple = |freq| RepeatRule {
        freq,
        interval: 1,
        by_day: Vec::new(),
        month_day: None,
        month: None,
        set_pos: None,
        count: None,
        until: None,
    };
    match input.repeat {
        Repeat::Keep | Repeat::Custom => None,
        Repeat::None => Some(None),
        Repeat::Daily => Some(Some(simple(ical::Freq::Daily))),
        Repeat::Weekly => Some(Some(simple(ical::Freq::Weekly))),
        Repeat::Monthly => Some(Some(simple(ical::Freq::Monthly))),
        Repeat::Yearly => Some(Some(simple(ical::Freq::Yearly))),
    }
}

/// Save an event in a Microsoft calendar through Graph (which sends the
/// invitations and updates itself), then read it back.
async fn save_graph_event(
    ctx: &Ctx,
    state: &Arc<AppState>,
    account: &Account,
    calendar: &Calendar,
    existing: Option<StoredEvent>,
    input: &EventInput,
) -> AppResult<CalendarEvent> {
    let client = connections::graph_client(state, account).await?;
    let remote = calendar
        .remote_id
        .clone()
        .ok_or_else(|| AppError::internal("Microsoft calendar without an id"))?;
    let rule = graph_rule(input);
    let href = match &existing {
        Some(stored) if stored.calendar_id != calendar.id => {
            return Err(AppError::invalid(
                "Events can't move between a Microsoft calendar and another one. Create it again in the other calendar.",
            ));
        }
        Some(stored) => {
            if let (Scope::This, Some(rid)) = (input.scope, input.occurrence) {
                let instance = graph::instance_id(&client, &stored.href, rid)
                    .await?
                    .ok_or_else(|| {
                        AppError::invalid("That date of the series is gone. Refresh and try again.")
                    })?;
                graph::update_event(&client, &instance, &graph::event_json(input, None)?).await?;
            } else {
                let body = graph::event_json(input, rule.as_ref().map(Option::as_ref))?;
                graph::update_event(&client, &stored.href, &body).await?;
            }
            stored.href.clone()
        }
        None => {
            let body = graph::event_json(input, rule.as_ref().and_then(Option::as_ref).map(Some))?;
            let made = graph::create_event(&client, &remote, &body).await?;
            mail_sync::rest::text(&made, "id").to_owned()
        }
    };
    sync_graph_calendar(state, &client, calendar).await?;
    changed(ctx, &[]);
    let stored = state
        .store
        .calendar_event_at(&calendar.id, &href)?
        .ok_or_else(|| AppError::invalid("The event was saved, but couldn't be read back yet. It will show after the next sync."))?;
    reminders_on_server(state, &stored.id, input)?;
    shown_event(state, calendar, account, &stored)
}

/// `delete_event`: delete an event (or a whole series), or with
/// `occurrence` just that date of a series. With `notify`, people invited
/// to a meeting you organise are sent a cancellation.
pub async fn delete_event(
    ctx: &Ctx,
    id: String,
    occurrence: Option<i64>,
    notify: Option<bool>,
) -> AppResult<()> {
    let state = &ctx.state;
    let stored = state.store.get_calendar_event(&id)?;
    let calendar = state.store.get_calendar(&stored.calendar_id)?;
    let account = account_of(state, &calendar)?;
    if calendar.provider == CalendarProvider::Microsoft {
        // Graph sends cancellations for meetings you organise.
        let client = connections::graph_client(state, &account).await?;
        let target = match occurrence {
            Some(rid) => graph::instance_id(&client, &stored.href, rid)
                .await?
                .ok_or_else(|| AppError::invalid("That date of the series is already gone."))?,
            None => stored.href.clone(),
        };
        graph::delete_event(&client, &target).await?;
        sync_graph_calendar(state, &client, &calendar).await?;
        changed(ctx, &[]);
        return Ok(());
    }
    if account.is_ews() {
        crate::ews_sync::delete_event(
            state,
            &account,
            &stored,
            occurrence,
            notify.unwrap_or(false),
        )
        .await?;
        changed(ctx, &[]);
        return Ok(());
    }
    let (client, url) = client_for(state, &calendar).await?;
    let one_date = occurrence.and_then(|rid| ical::skip_occurrence(&stored.ics, rid));
    let result = match &one_date {
        Some(ics) => {
            let uid = ical::uid_of(ics).unwrap_or_default();
            client
                .put_event(&url, Some(&stored.href), &uid, ics, stored.etag.as_deref())
                .await
                .and_then(|(href, etag)| {
                    state
                        .store
                        .put_calendar_event(&calendar.id, &href, etag.as_deref(), ics)
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
        soon(ctx, state);
        return Err(changed_elsewhere(error));
    }
    let people = others(&stored.ics, &account.email);
    if notify.unwrap_or(false)
        && !people.is_empty()
        && organises(&stored.ics, &account.email)
        && !client.auto_schedules(&url).await.unwrap_or(false)
    {
        let stamp = i64::try_from(now()).unwrap_or(0);
        let rid = occurrence.filter(|_| one_date.is_some());
        if let Some(cancel) = ical::itip_cancel(&stored.ics, rid, None, stamp) {
            let title = ical::master(&stored.ics)
                .map(|o| o.title)
                .unwrap_or_default();
            let title = if title.trim().is_empty() {
                "(No title)".to_owned()
            } else {
                title
            };
            crate::invitations::queue_calendar_mail(
                ctx,
                state,
                &account,
                people,
                &format!("Cancelled: {title}"),
                &format!("This meeting was cancelled: {title}"),
                &cancel,
                "CANCEL",
            )?;
        }
    }
    changed(ctx, &[]);
    Ok(())
}

/// When an account's own calendars are busy, from the copy kept here.
fn local_busy(
    state: &AppState,
    account: &Account,
    from: i64,
    to: i64,
) -> AppResult<Vec<ical::BusyPeriod>> {
    let calendars: Vec<Calendar> = state
        .store
        .list_calendars()?
        .into_iter()
        .filter(|c| c.account_id == account.id)
        .collect();
    let mut out = Vec::new();
    for stored in state.store.calendar_events_between(from, to)? {
        if !calendars.iter().any(|c| c.id == stored.calendar_id) {
            continue;
        }
        for o in ical::occurrences(&stored.ics, from, to) {
            if o.show_as == ShowAs::Free || declined(&o, Some(&account.email)) {
                continue;
            }
            out.push(ical::BusyPeriod {
                start: o.start,
                end: o.end.max(o.start),
                show_as: if o.tentative {
                    ShowAs::Tentative
                } else {
                    o.show_as
                },
            });
        }
    }
    out.sort_by_key(|p| (p.start, p.end));
    Ok(out)
}

/// Other people's free/busy through the account's server. `None` for
/// those it couldn't read.
async fn remote_busy(
    state: &AppState,
    account: &Account,
    emails: &[String],
    from: i64,
    to: i64,
) -> AppResult<Vec<(String, Option<Vec<ical::BusyPeriod>>)>> {
    let unknown = || emails.iter().map(|e| (e.clone(), None)).collect();
    match connections::service(state, account)? {
        Service::NotConnected => Ok(unknown()),
        Service::Google => {
            let client = connections::google_client(state, account).await?;
            Ok(google::free_busy(&client, emails, from, to).await?)
        }
        Service::Microsoft => {
            let client = connections::graph_client(state, account).await?;
            Ok(graph::get_schedule(&client, emails, from, to).await?)
        }
        Service::Exchange => crate::ews_sync::free_busy(state, account, emails, from, to).await,
        Service::Dav => {
            let Credentials::Password { username, password } =
                crate::credentials::for_account(state, account).await?
            else {
                return Ok(unknown());
            };
            let client = DavClient::new(DavLogin { username, password })?;
            let server = Url::parse(&format!("https://{}/", account.imap.host))
                .map_err(|e| AppError::internal(e.to_string()))?;
            let Some(outbox) = client.scheduling_outbox(&server).await? else {
                return Ok(unknown());
            };
            let uid = mail_store::new_event_uid()?;
            let stamp = i64::try_from(now()).unwrap_or(0);
            let request = ical::free_busy_request(&account.email, emails, from, to, &uid, stamp);
            let answers = match client
                .schedule_free_busy(&outbox, &account.email, emails, &request)
                .await
            {
                Ok(answers) => answers,
                Err(SyncError::Refused(_) | SyncError::Protocol(_)) => return Ok(unknown()),
                Err(other) => return Err(other.into()),
            };
            Ok(emails
                .iter()
                .map(|email| {
                    let busy = answers
                        .iter()
                        .find(|a| a.recipient.eq_ignore_ascii_case(email))
                        .filter(|a| a.status.starts_with("2."))
                        .and_then(|a| a.data.as_deref())
                        .map(ical::free_busy_periods);
                    (email.clone(), busy)
                })
                .collect())
        }
    }
}

/// `get_free_busy`: when these people are busy, for Find a Time. Your own
/// accounts come from the calendars kept here; others from the account's
/// server (`CalDAV` scheduling, Google's free/busy, Microsoft's
/// `getSchedule`), and are `unknown` when it can't tell.
pub async fn get_free_busy(
    ctx: &Ctx,
    account_id: String,
    emails: Vec<String>,
    from: i64,
    to: i64,
) -> AppResult<Vec<FreeBusy>> {
    let state = &ctx.state;
    if to <= from || to.saturating_sub(from) > MAX_FREE_BUSY_WINDOW {
        return Err(AppError::invalid("Choose up to two months at a time."));
    }
    if emails.len() > MAX_FREE_BUSY_PEOPLE {
        return Err(AppError::invalid("Choose up to 50 people at a time."));
    }
    let account = state.store.get_account(&account_id)?;
    let accounts = state.store.list_accounts()?;
    let mut out: Vec<FreeBusy> = Vec::new();
    let mut ask: Vec<String> = Vec::new();
    for email in emails {
        let email = email.trim().to_owned();
        if email.is_empty() || out.iter().any(|f| f.email.eq_ignore_ascii_case(&email)) {
            continue;
        }
        if let Some(mine) = accounts
            .iter()
            .find(|a| a.email.eq_ignore_ascii_case(&email))
        {
            out.push(FreeBusy {
                email,
                busy: local_busy(state, mine, from, to)?,
                unknown: false,
            });
        } else if email.contains('@') {
            ask.push(email);
        } else {
            out.push(FreeBusy {
                email,
                busy: Vec::new(),
                unknown: true,
            });
        }
    }
    if !ask.is_empty() {
        let answers = match remote_busy(state, &account, &ask, from, to).await {
            Ok(answers) => answers,
            Err(error) if error.kind == "connect" || error.kind == "internal" => {
                ask.iter().map(|e| (e.clone(), None)).collect()
            }
            Err(error) => return Err(error),
        };
        for (email, busy) in answers {
            out.push(FreeBusy {
                email,
                unknown: busy.is_none(),
                busy: busy.unwrap_or_default(),
            });
        }
    }
    Ok(out)
}

/// `sync_calendars`: sync every account's calendars now.
pub async fn sync_calendars(ctx: &Ctx) -> AppResult<Vec<CalendarStatus>> {
    let state = &ctx.state;
    Ok(sync_all(ctx, state).await)
}

pub(crate) fn changed(ctx: &Ctx, statuses: &[CalendarStatus]) {
    if let Err(error) = ctx.emit("mail://calendar-changed", json!({ "calendars": statuses })) {
        eprintln!("couldn't emit calendar-changed: {error}");
    }
}

/// Sync shortly, after a change was refused.
fn soon(ctx: &Ctx, state: &Arc<AppState>) {
    let (task_ctx, state) = (ctx.clone(), Arc::clone(state));
    ctx.spawn(async move {
        sync_all(&task_ctx, &state).await;
    });
}

/// Background sync of calendars and tasks: once at start-up, then every
/// 15 minutes.
pub fn start(ctx: Ctx, state: Arc<AppState>) {
    ctx.clone().spawn(async move {
        loop {
            sync_all(&ctx, &state).await;
            crate::tasks::sync_all(&ctx, &state).await;
            tokio::time::sleep(SYNC_EVERY).await;
        }
    });
}

pub(crate) async fn sync_all(ctx: &Ctx, state: &Arc<AppState>) -> Vec<CalendarStatus> {
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
            connectable: connections::connectable(state, &account),
        });
    }
    changed(ctx, &out);
    out
}

/// Sync one account. `Ok(Some(reason))` when it has no calendar here.
async fn sync_account(state: &AppState, account: &Account) -> AppResult<Option<String>> {
    match connections::service(state, account)? {
        Service::NotConnected => Ok(Some(connections::NOT_CONNECTED.to_owned())),
        Service::Google => sync_google(state, account).await.map(|()| None),
        Service::Microsoft => sync_microsoft(state, account).await.map(|()| None),
        Service::Exchange => crate::ews_sync::sync_calendars(state, account)
            .await
            .map(|()| None),
        Service::Dav => sync_dav(state, account).await,
    }
}

async fn sync_dav(state: &AppState, account: &Account) -> AppResult<Option<String>> {
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
                writable: c.writable,
                provider: CalendarProvider::Caldav,
                remote_id: None,
            })
            .collect::<Vec<_>>(),
    )?;
    fetch_dav_events(state, &client, &calendars).await?;
    Ok(None)
}

/// Read what changed in `CalDAV` calendars.
async fn fetch_dav_events(
    state: &AppState,
    client: &DavClient,
    calendars: &[Calendar],
) -> AppResult<()> {
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
    Ok(())
}

async fn sync_google(state: &AppState, account: &Account) -> AppResult<()> {
    let api = connections::google_client(state, account).await?;
    let found = google::list_calendars(&api).await?;
    let calendars = state.store.set_calendars(
        &account.id,
        &found
            .iter()
            .map(|c| FoundCalendar {
                url: google::caldav_events_url(&c.id),
                name: c.name.clone(),
                color: c.color.clone(),
                writable: c.writable,
                provider: CalendarProvider::Google,
                remote_id: Some(c.id.clone()),
            })
            .collect::<Vec<_>>(),
    )?;
    let dav = DavClient::bearer(connections::google_token(state, account).await?)?;
    fetch_dav_events(state, &dav, &calendars).await
}

async fn sync_microsoft(state: &AppState, account: &Account) -> AppResult<()> {
    let client = connections::graph_client(state, account).await?;
    let found = graph::list_calendars(&client).await?;
    let calendars = state.store.set_calendars(
        &account.id,
        &found
            .iter()
            .map(|c| FoundCalendar {
                url: format!("{}/me/calendars/{}", graph::GRAPH, c.id),
                name: c.name.clone(),
                color: c.color.clone(),
                writable: c.writable,
                provider: CalendarProvider::Microsoft,
                remote_id: Some(c.id.clone()),
            })
            .collect::<Vec<_>>(),
    )?;
    for calendar in &calendars {
        sync_graph_calendar(state, &client, calendar).await?;
    }
    Ok(())
}

/// Read a Microsoft calendar's events (90 days back to a year on) and keep
/// them as iCalendar text: one resource per single event or series.
pub(crate) async fn sync_graph_calendar(
    state: &AppState,
    client: &mail_sync::rest::RestClient,
    calendar: &Calendar,
) -> AppResult<()> {
    let Some(remote) = calendar.remote_id.as_deref() else {
        return Ok(());
    };
    let now = i64::try_from(now()).unwrap_or(0);
    let (from, to) = (
        now.saturating_sub(GRAPH_BACK),
        now.saturating_add(GRAPH_AHEAD),
    );
    let items = graph::calendar_view(client, remote, from, to).await?;
    let (singles, series) = graph::group_instances(&items);
    let known = state.store.calendar_etags(&calendar.id)?;
    let mut on_server = Vec::new();
    for (event, tag) in singles {
        let id = mail_sync::rest::text(&event, "id").to_owned();
        if known
            .get(&id)
            .is_none_or(|k| k.as_deref() != Some(tag.as_str()))
            && let Some(ics) = graph::single_ics(&event)
        {
            state
                .store
                .put_calendar_event(&calendar.id, &id, Some(&tag), &ics)?;
        }
        on_server.push(id);
    }
    for (master_id, (instances, tag)) in series {
        if known
            .get(&master_id)
            .is_none_or(|k| k.as_deref() != Some(tag.as_str()))
        {
            let master = graph::get_event(client, &master_id).await?;
            if let Some(ics) = graph::series_ics(&master, &instances, from, to) {
                state
                    .store
                    .put_calendar_event(&calendar.id, &master_id, Some(&tag), &ics)?;
            }
        }
        on_server.push(master_id);
    }
    state
        .store
        .forget_calendar_events_not_on_server(&calendar.id, &on_server)?;
    Ok(())
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
            reminders: None,
            ..EventInput::default()
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

    #[test]
    fn invitations_and_rules_are_checked() {
        let people = checked(EventInput {
            attendees: Some(vec![" Peter <peter@client.example> ".into(), String::new()]),
            ..input()
        })
        .map(|i| i.attendees)
        .unwrap_or_default();
        assert_eq!(people, Some(vec!["peter@client.example".to_owned()]));
        assert!(
            checked(EventInput {
                attendees: Some(vec!["not an address".into()]),
                ..input()
            })
            .is_err()
        );
        let rule = RepeatRule {
            freq: ical::Freq::Weekly,
            interval: 0,
            by_day: vec![],
            month_day: None,
            month: None,
            set_pos: None,
            count: None,
            until: None,
        };
        assert!(
            checked(EventInput {
                repeat_rule: Some(rule.clone()),
                ..input()
            })
            .is_err()
        );
        let fine = checked(EventInput {
            repeat_rule: Some(RepeatRule {
                interval: 2,
                by_day: vec!["mo".into(), "MO".into()],
                ..rule
            }),
            ..input()
        })
        .ok()
        .and_then(|i| i.repeat_rule);
        assert_eq!(fine.map(|r| r.by_day), Some(vec!["MO".to_owned()]));
        // "This date" without a date is the whole series.
        let all = checked(EventInput {
            scope: Scope::This,
            ..input()
        })
        .map(|i| i.scope)
        .ok();
        assert_eq!(all, Some(Scope::All));
    }

    const THEIRS: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:m\r\nDTSTART:20261118T060000Z\r\nDTEND:20261118T070000Z\r\nSUMMARY:Site visit\r\nORGANIZER;CN=Peter:mailto:peter@client.example\r\nATTENDEE;PARTSTAT=ACCEPTED:mailto:me@example.com\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    const MINE: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:o\r\nDTSTART:20261118T060000Z\r\nDTEND:20261118T070000Z\r\nSUMMARY:Review\r\nORGANIZER:mailto:Me@Example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    const PLAIN: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:p\r\nDTSTART:20261118T060000Z\r\nDTEND:20261118T070000Z\r\nSUMMARY:Dentist\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn account() -> Account {
        let server = mail_store::ServerConfig {
            host: "mail.example".into(),
            port: 993,
            security: mail_store::Security::Tls,
        };
        Account {
            id: "a".into(),
            email: "me@example.com".into(),
            display_name: "Me".into(),
            auth: mail_store::AuthKind::Password,
            color: "#2450c8".into(),
            imap: server.clone(),
            smtp: server,
            size_limit_bytes: None,
            created_at: 0,
            sort_order: 0,
            sync_months: None,
            protocol: mail_store::Protocol::Imap,
        }
    }

    fn stored(ics: &str) -> StoredEvent {
        StoredEvent {
            id: "e".into(),
            calendar_id: "c".into(),
            href: "/c/e.ics".into(),
            etag: None,
            ics: ics.into(),
        }
    }

    fn calendar(writable: bool) -> Calendar {
        Calendar {
            id: "c".into(),
            account_id: "a".into(),
            url: "https://x/c/".into(),
            name: "Personal".into(),
            color: None,
            visible: true,
            writable,
            provider: CalendarProvider::Caldav,
            remote_id: None,
        }
    }

    #[test]
    fn only_reminders_change_when_the_server_copy_cant() {
        let me = "me@example.com";
        // Read-only calendar: even your own event.
        assert!(reminders_only(&calendar(false), &stored(MINE), me));
        assert!(reminders_only(&calendar(false), &stored(PLAIN), me));
        // Someone else's meeting, even in a calendar you can write to.
        assert!(reminders_only(&calendar(true), &stored(THEIRS), me));
        // Yours, or with no organiser: saved on the server as before.
        assert!(!reminders_only(&calendar(true), &stored(MINE), me));
        assert!(!reminders_only(&calendar(true), &stored(PLAIN), me));
    }

    #[test]
    fn someone_elses_organiser_is_never_replaced() {
        let account = account();
        // New events, and events with no organiser yet: you.
        assert_eq!(
            organizer_for(None, &account).map(|p| p.email),
            Some("me@example.com".to_owned())
        );
        assert!(organizer_for(Some(&stored(PLAIN)), &account).is_some());
        assert!(organizer_for(Some(&stored(MINE)), &account).is_some());
        // Someone else's meeting: nobody is passed, so Peter stays.
        assert_eq!(organizer_for(Some(&stored(THEIRS)), &account), None);
        let input = EventInput {
            attendees: Some(vec!["me@example.com".into(), "ann@x.example".into()]),
            organizer: organizer_for(Some(&stored(THEIRS)), &account),
            repeat: Repeat::Keep,
            ..input()
        };
        let edited = ical::edit_event(THEIRS, &input, 1_795_000_000)
            .and_then(|ics| ical::master(&ics))
            .and_then(|o| o.organizer)
            .map(|p| p.email);
        assert_eq!(edited.as_deref(), Some("peter@client.example"));
    }

    #[test]
    fn reminders_of_a_read_only_event_are_kept_here() {
        let state = AppState::new(
            mail_store::Store::open_in_memory().unwrap(),
            Arc::new(mail_sync::secrets::MemoryStore::default()),
            std::env::temp_dir(),
        );
        let template = account();
        let account = state
            .store
            .insert_account(&mail_store::NewAccount {
                email: template.email,
                display_name: template.display_name,
                auth: template.auth,
                color: template.color,
                imap: template.imap,
                smtp: template.smtp,
                size_limit_bytes: None,
                created_at: 0,
            })
            .unwrap();
        let found = FoundCalendar {
            url: "https://x/c/".into(),
            name: "Shared".into(),
            writable: false,
            ..FoundCalendar::default()
        };
        let calendar = state.store.set_calendars(&account.id, &[found]).unwrap()[0].clone();
        let id = state
            .store
            .put_calendar_event(&calendar.id, "/c/m.ics", Some("\"1\""), THEIRS)
            .unwrap();
        let event = state.store.get_calendar_event(&id).unwrap();
        assert!(reminders_only(&calendar, &event, &account.email));
        // Everything else the editor sent is ignored.
        let edit = EventInput {
            title: "Renamed".into(),
            start: 1_795_100_000,
            end: 1_795_103_600,
            reminders: Some(vec![5, 60]),
            ..input()
        };
        let shown = keep_reminders_here(&state, &calendar, &account, &event, &edit).unwrap();
        assert_eq!(shown.occurrence.reminders, [60, 5]);
        assert_eq!(shown.occurrence.title, "Site visit");
        assert_eq!(shown.occurrence.start, 1_794_981_600);
        assert!(!shown.occurrence.is_organizer);
        // The server copy is untouched; the reminders show from here.
        let after = state.store.get_calendar_event(&id).unwrap();
        assert_eq!(after.ics, THEIRS);
        let again = shown_event(&state, &calendar, &account, &after).unwrap();
        assert_eq!(again.occurrence.reminders, [60, 5]);
        // No reminders sent: they stay as they are.
        let unchanged = EventInput {
            reminders: None,
            ..input()
        };
        let shown = keep_reminders_here(&state, &calendar, &account, &event, &unchanged).unwrap();
        assert_eq!(shown.occurrence.reminders, [60, 5]);
    }
}
