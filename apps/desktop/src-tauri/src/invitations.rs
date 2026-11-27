//! Meeting invitations in mail (iMIP, RFC 6047): what an invitation,
//! cancellation or answer says, and acting on it. Answering saves the
//! event in the account's calendar (`CalDAV`) with your answer, and emails
//! the organiser a `METHOD:REPLY` through the Outbox, so it works with any
//! organiser (Google, Outlook, `SOGo`).

use std::sync::Arc;

use mail_core::folders::FolderRole;
use mail_core::ical::{self, Answer, Invitation};
use mail_core::message::{Mailbox, make_preview, parse_body};
use mail_core::outbox::OutboxItem;
use mail_store::{Account, CalendarProvider, NewOutbox, StoredEvent};
use mail_sync::outgoing::{build_calendar_message, build_calendar_reply};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::state::{AppState, now};
use crate::{events, sync};

/// An invitation as the reading pane shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationView {
    #[serde(flatten)]
    invitation: Invitation,
    /// The same event in your calendar, if it is there.
    calendar_event_id: Option<String>,
    /// Your answer as saved in your calendar.
    saved_status: Option<String>,
    /// For an answer to your invitation: your calendar doesn't show it yet.
    reply_pending: bool,
    /// The account has a calendar to save to.
    has_calendar: bool,
}

/// The stored event with this `UID`, in any of the account's calendars.
fn stored_event(state: &AppState, account_id: &str, uid: &str) -> AppResult<Option<StoredEvent>> {
    Ok(state
        .store
        .calendar_events_of_account(account_id)?
        .into_iter()
        .find(|e| ical::uid_of(&e.ics).as_deref() == Some(uid)))
}

/// The invitation in a calendar part, with what your calendar knows of it.
pub fn view(
    state: &AppState,
    account: &Account,
    calendar: &str,
) -> AppResult<Option<InvitationView>> {
    let Some(invitation) = ical::invitation(calendar, &account.email) else {
        return Ok(None);
    };
    let stored = stored_event(state, &account.id, &invitation.event.uid)?;
    let saved_status = stored
        .as_ref()
        .and_then(|e| ical::invitation(&e.ics, &account.email))
        .and_then(|i| i.my_status);
    let reply_pending = invitation.method == "REPLY"
        && stored
            .as_ref()
            .is_some_and(|e| ical::apply_reply(&e.ics, calendar).is_some());
    let has_calendar = state
        .store
        .list_calendars()?
        .iter()
        .any(|c| c.account_id == account.id);
    Ok(Some(InvitationView {
        invitation,
        calendar_event_id: stored.map(|e| e.id),
        saved_status,
        reply_pending,
        has_calendar,
    }))
}

/// The message's account and calendar part.
async fn load(state: &AppState, id: &str) -> AppResult<(Account, String)> {
    let raw = sync::body_now(state, id).await?;
    let stored = state.store.get_message(id)?;
    let account = state.store.get_account(&stored.item.account_id)?;
    let calendar = parse_body(&raw)?
        .calendar
        .ok_or_else(|| AppError::invalid("This message has no invitation."))?;
    Ok((account, calendar))
}

fn current(state: &AppState, account: &Account, calendar: &str) -> AppResult<InvitationView> {
    view(state, account, calendar)?
        .ok_or_else(|| AppError::invalid("This message has no invitation."))
}

/// The calendar to save an answered invitation in: the one already
/// holding it, else the account's main ("personal") calendar. Microsoft
/// calendars are left out: Exchange puts invitations in them itself, so
/// for those only the answer is emailed.
fn target_calendar(
    state: &AppState,
    account: &Account,
    existing: Option<&StoredEvent>,
) -> AppResult<Option<mail_store::Calendar>> {
    if let Some(e) = existing {
        let calendar = state.store.get_calendar(&e.calendar_id)?;
        return Ok((calendar.provider != CalendarProvider::Microsoft).then_some(calendar));
    }
    let calendars: Vec<_> = state
        .store
        .list_calendars()?
        .into_iter()
        .filter(|c| c.account_id == account.id)
        .filter(|c| c.provider != CalendarProvider::Microsoft && c.writable)
        .collect();
    Ok(calendars
        .iter()
        .find(|c| c.url.trim_end_matches('/').ends_with("/personal"))
        // Google: the calendar named after the account is the main one.
        .or_else(|| {
            calendars
                .iter()
                .find(|c| c.remote_id.as_deref() == Some(account.email.as_str()))
        })
        .or_else(|| calendars.first())
        .cloned())
}

/// Whether a stored event is in a calendar Microsoft keeps up to date
/// itself.
fn kept_by_microsoft(state: &AppState, event: &StoredEvent) -> AppResult<bool> {
    Ok(state.store.get_calendar(&event.calendar_id)?.provider == CalendarProvider::Microsoft)
}

/// `answer_invitation`: accept, maybe or decline. Saves it in your
/// calendar (a decline removes it) and, with `notify`, emails the organiser.
#[tauri::command]
pub async fn answer_invitation(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    answer: Answer,
    notify: bool,
) -> AppResult<InvitationView> {
    let (account, calendar) = load(&state, &id).await?;
    let invitation = current(&state, &account, &calendar)?;
    if invitation.invitation.method != "REQUEST" {
        return Err(AppError::invalid(
            "This isn't an invitation you can answer.",
        ));
    }
    let uid = invitation.invitation.event.uid.clone();
    let name = Some(account.display_name.as_str()).filter(|n| !n.trim().is_empty());

    // Your calendar first: if that fails, nothing has been sent.
    let existing = stored_event(&state, &account.id, &uid)?;
    let mut server_tells_organiser = false;
    if let Some(target) = target_calendar(&state, &account, existing.as_ref())? {
        let (client, url) = crate::calendar::client_for(&state, &target).await?;
        // Servers like SOGo email the organiser themselves when your answer
        // is saved (and ignore requests not to), so the app must not too.
        let auto = notify && client.auto_schedules(&url).await.unwrap_or(false);
        let copy = || {
            ical::calendar_copy(&calendar, &account.email, name, answer)
                .ok_or_else(|| AppError::invalid("This invitation has no event."))
        };
        let href = existing.as_ref().map(|e| e.href.as_str());
        let etag = existing.as_ref().and_then(|e| e.etag.as_deref());
        if auto {
            // The decline is saved too (and hidden in the calendar), so the
            // server sends it.
            let copy = copy()?;
            let (href, etag) = client.put_event(&url, href, &uid, &copy, etag).await?;
            state
                .store
                .put_calendar_event(&target.id, &href, etag.as_deref(), &copy)?;
            server_tells_organiser = true;
        } else if answer == Answer::Declined {
            if let Some(stored) = &existing {
                client
                    .delete_answered_event(&url, &stored.href, stored.etag.as_deref())
                    .await?;
                state.store.delete_calendar_event(&stored.id)?;
            }
        } else {
            let copy = copy()?;
            let (href, etag) = client
                .put_answered_event(&url, href, &uid, &copy, etag)
                .await?;
            state
                .store
                .put_calendar_event(&target.id, &href, etag.as_deref(), &copy)?;
        }
        crate::calendar::changed(&app, &[]);
    }

    if notify
        && !server_tells_organiser
        && let Some(organizer) = invitation.invitation.event.organizer.clone()
        && !organizer.email.eq_ignore_ascii_case(&account.email)
    {
        queue_reply(
            &app,
            &state,
            &account,
            &calendar,
            &organizer,
            &invitation.invitation,
            answer,
        )?;
    }
    current(&state, &account, &calendar)
}

fn queue_reply(
    app: &AppHandle,
    state: &Arc<AppState>,
    account: &Account,
    calendar: &str,
    organizer: &ical::Person,
    invitation: &Invitation,
    answer: Answer,
) -> AppResult<()> {
    let name = Some(account.display_name.as_str()).filter(|n| !n.trim().is_empty());
    let stamp = i64::try_from(now()).unwrap_or(0);
    let ics = ical::reply(calendar, &account.email, name, answer, stamp)
        .ok_or_else(|| AppError::invalid("This invitation has no event."))?;
    let (word, verb) = match answer {
        Answer::Accepted => ("Accepted", "accepted"),
        Answer::Tentative => ("Tentative", "tentatively accepted"),
        Answer::Declined => ("Declined", "declined"),
    };
    let title = if invitation.event.title.is_empty() {
        "(No title)"
    } else {
        invitation.event.title.as_str()
    };
    let who = name.unwrap_or(&account.email);
    let note = format!("{who} has {verb} this invitation: {title}");
    let from = Mailbox {
        name: name.map(str::to_owned),
        address: account.email.clone(),
    };
    let to = Mailbox {
        name: organizer.name.clone(),
        address: organizer.email.clone(),
    };
    let subject = format!("{word}: {title}");
    let built = build_calendar_reply(&from, &to, &subject, &note, &ics)?;
    let created_at = now();
    state.store.insert_outbox(&NewOutbox {
        account_id: account.id.clone(),
        created_at,
        to: vec![to],
        subject,
        preview: make_preview(&note, mail_core::message::PREVIEW_CHARS),
        raw: built.raw,
        recipients: built.recipients,
        message_id_header: Some(built.message_id),
        reply_to_message_id: None,
        sent_folder_hint: crate::outbox::folder_path(state, &account.id, FolderRole::Sent)?,
        item: OutboxItem::scheduled(created_at, 0),
    })?;
    state.outbox_wake.notify_one();
    events::outbox_changed(app, state);
    Ok(())
}

/// `remove_cancelled_event`: take a cancelled event out of your calendar.
#[tauri::command]
pub async fn remove_cancelled_event(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> AppResult<InvitationView> {
    let (account, calendar) = load(&state, &id).await?;
    let invitation = current(&state, &account, &calendar)?;
    if invitation.invitation.method != "CANCEL" {
        return Err(AppError::invalid("This event wasn't cancelled."));
    }
    if let Some(stored) = stored_event(&state, &account.id, &invitation.invitation.event.uid)?
        && !kept_by_microsoft(&state, &stored)?
    {
        let target = state.store.get_calendar(&stored.calendar_id)?;
        let (client, url) = crate::calendar::client_for(&state, &target).await?;
        client
            .delete_answered_event(&url, &stored.href, stored.etag.as_deref())
            .await?;
        state.store.delete_calendar_event(&stored.id)?;
        crate::calendar::changed(&app, &[]);
    }
    current(&state, &account, &calendar)
}

/// `apply_invitation_reply`: record someone's answer to your invitation
/// in your calendar copy.
#[tauri::command]
pub async fn apply_invitation_reply(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> AppResult<InvitationView> {
    let (account, calendar) = load(&state, &id).await?;
    let invitation = current(&state, &account, &calendar)?;
    let stored = stored_event(&state, &account.id, &invitation.invitation.event.uid)?
        .ok_or_else(|| AppError::invalid("This event isn't in your calendar."))?;
    if !kept_by_microsoft(&state, &stored)?
        && let Some(updated) = ical::apply_reply(&stored.ics, &calendar)
    {
        let target = state.store.get_calendar(&stored.calendar_id)?;
        let (client, url) = crate::calendar::client_for(&state, &target).await?;
        let uid = invitation.invitation.event.uid.clone();
        let (href, etag) = client
            .put_answered_event(
                &url,
                Some(&stored.href),
                &uid,
                &updated,
                stored.etag.as_deref(),
            )
            .await?;
        state
            .store
            .put_calendar_event(&target.id, &href, etag.as_deref(), &updated)?;
        crate::calendar::changed(&app, &[]);
    }
    current(&state, &account, &calendar)
}

/// Queue an invitation, update or cancellation email (iMIP `REQUEST` or
/// `CANCEL`) to the people on an event, through the Outbox, to send at
/// once.
#[allow(clippy::too_many_arguments, reason = "the parts of one message")]
pub(crate) fn queue_calendar_mail(
    app: &AppHandle,
    state: &Arc<AppState>,
    account: &Account,
    to: Vec<Mailbox>,
    subject: &str,
    note: &str,
    ics: &str,
    method: &str,
) -> AppResult<()> {
    let name = Some(account.display_name.as_str()).filter(|n| !n.trim().is_empty());
    let from = Mailbox {
        name: name.map(str::to_owned),
        address: account.email.clone(),
    };
    let built = build_calendar_message(&from, &to, subject, note, ics, method)?;
    let created_at = now();
    state.store.insert_outbox(&NewOutbox {
        account_id: account.id.clone(),
        created_at,
        to,
        subject: subject.to_owned(),
        preview: make_preview(note, mail_core::message::PREVIEW_CHARS),
        raw: built.raw,
        recipients: built.recipients,
        message_id_header: Some(built.message_id),
        reply_to_message_id: None,
        sent_folder_hint: crate::outbox::folder_path(state, &account.id, FolderRole::Sent)?,
        item: OutboxItem::scheduled(created_at, 0),
    })?;
    state.outbox_wake.notify_one();
    events::outbox_changed(app, state);
    Ok(())
}
