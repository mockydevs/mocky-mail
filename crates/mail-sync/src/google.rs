//! Google APIs for Gmail accounts connected for calendar and contacts:
//! the calendar list and free/busy (Calendar API v3), Google Tasks,
//! contact groups (People API) and automatic replies (Gmail settings).
//! Events and contacts themselves go over Google's `CalDAV` and `CardDAV`
//! ([`crate::dav`] with a bearer token).
//!
//! The JSON is turned into the app's own types here, without I/O, so it
//! can be tested against recorded replies.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use mail_core::ical::{BusyPeriod, ShowAs};
use mail_core::tasks::TaskFields;
use mail_core::vacation::Vacation;
use reqwest::Method;
use serde_json::{Value, json};

use crate::SyncError;
use crate::rest::{RestClient, all_pages, segment, set, text, with_query};

/// Google's `CalDAV` collection of a calendar's events.
#[must_use]
pub fn caldav_events_url(calendar_id: &str) -> String {
    format!(
        "https://apidata.googleusercontent.com/caldav/v2/{}/events/",
        segment(calendar_id)
    )
}

/// Google's `CardDAV` address book of an account.
#[must_use]
pub fn carddav_book_url(email: &str) -> String {
    format!(
        "https://www.googleapis.com/carddav/v1/principals/{}/lists/default/",
        segment(email)
    )
}

/// A plain-words error for a Google reply that says the app isn't set up
/// for this (an API switched off in its Google Cloud project, or a
/// permission not granted), if it is one.
#[must_use]
pub fn setup_problem(body: &str) -> Option<SyncError> {
    if body.contains("SERVICE_DISABLED")
        || body.contains("accessNotConfigured")
        || body.contains("has not been used in project")
    {
        return Some(SyncError::Config(
            "Google hasn't enabled calendar and contacts access for Mocky Mail yet (one of its Google APIs is switched off). This is for Mocky Digital to fix; your mail is not affected.".to_owned(),
        ));
    }
    if body.contains("ACCESS_TOKEN_SCOPE_INSUFFICIENT")
        || body.contains("insufficientPermissions")
        || body.contains("insufficient authentication scopes")
    {
        return Some(SyncError::Config(
            "Mocky Mail wasn't given access to this in Google. Connect calendar and contacts again and allow everything it asks for.".to_owned(),
        ));
    }
    None
}

fn next_page(page: &Value, url: &str) -> Option<String> {
    let token = page.get("nextPageToken")?.as_str()?;
    let mut parsed = reqwest::Url::parse(url).ok()?;
    let kept: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| k != "pageToken")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    {
        let mut q = parsed.query_pairs_mut();
        q.clear();
        for (k, v) in &kept {
            q.append_pair(k, v);
        }
        q.append_pair("pageToken", token);
    }
    Some(parsed.to_string())
}

// -------------------------------------------------------------- calendars

/// A calendar in the account's calendar list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleCalendar {
    /// Its id (an address, or `…@group.calendar.google.com`).
    pub id: String,
    /// Its name as the user sees it.
    pub name: String,
    /// `#rrggbb`.
    pub color: Option<String>,
    /// Owner or writer.
    pub writable: bool,
    /// The account's main calendar.
    pub primary: bool,
}

/// The calendars in a `calendarList` reply.
#[must_use]
pub fn calendars_from_json(items: &[Value]) -> Vec<GoogleCalendar> {
    items
        .iter()
        .filter(|c| !c.get("deleted").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|c| {
            let id = text(c, "id");
            if id.is_empty() {
                return None;
            }
            let name = [text(c, "summaryOverride"), text(c, "summary"), id]
                .into_iter()
                .find(|n| !n.trim().is_empty())
                .unwrap_or(id);
            Some(GoogleCalendar {
                id: id.to_owned(),
                name: name.trim().to_owned(),
                color: Some(text(c, "backgroundColor").to_owned()).filter(|c| c.starts_with('#')),
                writable: matches!(text(c, "accessRole"), "owner" | "writer"),
                primary: c.get("primary").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

/// The account's calendars.
///
/// # Errors
/// Network errors and error replies.
pub async fn list_calendars(client: &RestClient) -> Result<Vec<GoogleCalendar>, SyncError> {
    let first = with_query(
        "https://www.googleapis.com/calendar/v3/users/me/calendarList",
        &[("maxResults", "250")],
    )?;
    let (items, _) = all_pages(client, first, "items", next_page, 20).await?;
    Ok(calendars_from_json(&items))
}

fn rfc3339(seconds: i64) -> String {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

fn parse_time(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|d| d.timestamp())
}

/// Each address's busy times from a `freeBusy` reply; `None` for those
/// Google couldn't read.
#[must_use]
pub fn free_busy_from_json(
    body: &Value,
    emails: &[String],
) -> Vec<(String, Option<Vec<BusyPeriod>>)> {
    let calendars = body.get("calendars");
    emails
        .iter()
        .map(|email| {
            let entry = calendars.and_then(|c| {
                c.as_object()?
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(email))
                    .map(|(_, v)| v)
            });
            let busy = entry.and_then(|e| {
                if e.get("errors")
                    .and_then(Value::as_array)
                    .is_some_and(|a| !a.is_empty())
                {
                    return None;
                }
                Some(
                    e.get("busy")
                        .and_then(Value::as_array)
                        .map(|list| {
                            list.iter()
                                .filter_map(|b| {
                                    Some(BusyPeriod {
                                        start: parse_time(text(b, "start"))?,
                                        end: parse_time(text(b, "end"))?,
                                        show_as: ShowAs::Busy,
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                )
            });
            (email.clone(), busy)
        })
        .collect()
}

/// When these people are busy (their calendars permitting).
///
/// # Errors
/// Network errors and error replies.
pub async fn free_busy(
    client: &RestClient,
    emails: &[String],
    from: i64,
    to: i64,
) -> Result<Vec<(String, Option<Vec<BusyPeriod>>)>, SyncError> {
    let body = json!({
        "timeMin": rfc3339(from),
        "timeMax": rfc3339(to),
        "items": emails.iter().map(|e| json!({ "id": e })).collect::<Vec<_>>(),
    });
    let reply = client
        .send(
            Method::POST,
            "https://www.googleapis.com/calendar/v3/freeBusy",
            Some(&body),
        )
        .await?;
    Ok(free_busy_from_json(&reply.body, emails))
}

// ------------------------------------------------------------------ tasks

/// A task as Google keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTask {
    /// Google's id.
    pub id: String,
    /// Its version tag.
    pub etag: Option<String>,
    /// Its fields. Google Tasks has no reminders, importance or
    /// categories, so those stay as the device has them.
    pub fields: TaskFields,
    /// The JSON as Google sent it.
    pub raw: String,
}

/// A task from Google's JSON; `None` for a deleted one.
#[must_use]
pub fn task_from_json(value: &Value) -> Option<RemoteTask> {
    if value
        .get("deleted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let id = text(value, "id");
    if id.is_empty() {
        return None;
    }
    // Only the date of `due` means anything to Google.
    let due = value
        .get("due")
        .and_then(Value::as_str)
        .and_then(|d| NaiveDate::parse_from_str(d.get(..10)?, "%Y-%m-%d").ok())
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|d| d.and_utc().timestamp());
    let completed = text(value, "status") == "completed";
    Some(RemoteTask {
        id: id.to_owned(),
        etag: Some(text(value, "etag").to_owned()).filter(|e| !e.is_empty()),
        fields: TaskFields {
            title: text(value, "title").trim().to_owned(),
            notes: text(value, "notes").trim().to_owned(),
            due,
            completed,
            completed_at: if completed {
                value
                    .get("completed")
                    .and_then(Value::as_str)
                    .and_then(parse_time)
            } else {
                None
            },
            ..TaskFields::default()
        },
        raw: value.to_string(),
    })
}

/// Google's JSON for a task's fields.
#[must_use]
pub fn task_json(fields: &TaskFields) -> Value {
    let due = fields.due.map(|d| {
        Utc.timestamp_opt(d, 0)
            .single()
            .unwrap_or_default()
            .format("%Y-%m-%dT00:00:00.000Z")
            .to_string()
    });
    let mut out = json!({
        "title": fields.title,
        "notes": fields.notes,
        "status": if fields.completed { "completed" } else { "needsAction" },
        "due": due,
    });
    if fields.completed
        && let Some(at) = fields.completed_at
    {
        set(&mut out, "completed", Value::String(rfc3339(at)));
    } else {
        set(&mut out, "completed", Value::Null);
    }
    out
}

/// The account's task lists: (id, name).
///
/// # Errors
/// Network errors and error replies.
pub async fn list_task_lists(client: &RestClient) -> Result<Vec<(String, String)>, SyncError> {
    let first = with_query(
        "https://tasks.googleapis.com/tasks/v1/users/@me/lists",
        &[("maxResults", "100")],
    )?;
    let (items, _) = all_pages(client, first, "items", next_page, 20).await?;
    Ok(items
        .iter()
        .filter(|l| !text(l, "id").is_empty())
        .map(|l| {
            let name = text(l, "title").trim();
            (
                text(l, "id").to_owned(),
                if name.is_empty() { "Tasks" } else { name }.to_owned(),
            )
        })
        .collect())
}

fn tasks_url(list_id: &str) -> String {
    format!(
        "https://tasks.googleapis.com/tasks/v1/lists/{}/tasks",
        segment(list_id)
    )
}

/// Every task in a list, done ones too.
///
/// # Errors
/// Network errors and error replies.
pub async fn list_tasks(client: &RestClient, list_id: &str) -> Result<Vec<RemoteTask>, SyncError> {
    let first = with_query(
        &tasks_url(list_id),
        &[
            ("maxResults", "100"),
            ("showCompleted", "true"),
            ("showHidden", "true"),
        ],
    )?;
    let (items, _) = all_pages(client, first, "items", next_page, 50).await?;
    Ok(items.iter().filter_map(task_from_json).collect())
}

/// Create a task.
///
/// # Errors
/// Network errors and error replies.
pub async fn create_task(
    client: &RestClient,
    list_id: &str,
    fields: &TaskFields,
) -> Result<RemoteTask, SyncError> {
    let reply = client
        .send(Method::POST, &tasks_url(list_id), Some(&task_json(fields)))
        .await?;
    task_from_json(&reply.body)
        .ok_or_else(|| SyncError::Protocol("Google didn't return the task".to_owned()))
}

/// Change a task.
///
/// # Errors
/// Network errors and error replies.
pub async fn update_task(
    client: &RestClient,
    list_id: &str,
    id: &str,
    fields: &TaskFields,
) -> Result<RemoteTask, SyncError> {
    let url = format!("{}/{}", tasks_url(list_id), segment(id));
    let reply = client
        .send(Method::PATCH, &url, Some(&task_json(fields)))
        .await?;
    task_from_json(&reply.body)
        .ok_or_else(|| SyncError::Protocol("Google didn't return the task".to_owned()))
}

/// Delete a task. One already gone counts as deleted.
///
/// # Errors
/// Network errors and error replies.
pub async fn delete_task(client: &RestClient, list_id: &str, id: &str) -> Result<(), SyncError> {
    let url = format!("{}/{}", tasks_url(list_id), segment(id));
    match client.send(Method::DELETE, &url, None).await {
        Ok(_) => Ok(()),
        Err(SyncError::Refused(why)) if why.contains("404") => Ok(()),
        Err(other) => Err(other),
    }
}

// ------------------------------------------------------- automatic replies

/// Midnight starting a day where the user is (`offset` seconds east of UTC).
fn day_start(day: &str, offset: i32) -> Option<i64> {
    let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?;
    Some(
        date.and_hms_opt(0, 0, 0)?
            .and_utc()
            .timestamp()
            .saturating_sub(i64::from(offset)),
    )
}

/// The day (`YYYY-MM-DD`) a time falls on where the user is.
fn day_of(seconds: i64, offset: i32) -> Option<String> {
    Utc.timestamp_opt(seconds.saturating_add(i64::from(offset)), 0)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
}

/// Automatic replies from Gmail's `vacation` settings. Gmail has no
/// "days before replying again" (it waits four days), so `days` stays 1.
#[must_use]
pub fn vacation_from_json(value: &Value, offset: i32) -> Vacation {
    let ms = |key: &str| -> Option<i64> {
        value
            .get(key)
            .and_then(|v| {
                v.as_str()
                    .and_then(|s| s.parse::<i64>().ok())
                    .or_else(|| v.as_i64())
            })
            .map(|ms| ms / 1000)
    };
    let body = text(value, "responseBodyPlainText");
    let message = if body.trim().is_empty() {
        crate::graph::html_to_text(text(value, "responseBodyHtml"))
    } else {
        body.to_owned()
    };
    Vacation {
        enabled: value
            .get("enableAutoReply")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        subject: Some(text(value, "responseSubject").trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| Vacation::default().subject),
        message: message.trim().to_owned(),
        start: ms("startTime").and_then(|s| day_of(s, offset)),
        // The reply stops at the end of the last day.
        end: ms("endTime").and_then(|e| day_of(e.saturating_sub(1), offset)),
        ..Vacation::default()
    }
}

/// Gmail's `vacation` settings for automatic replies.
#[must_use]
pub fn vacation_json(v: &Vacation, offset: i32) -> Value {
    let start = v.start.as_deref().and_then(|d| day_start(d, offset));
    let end = v
        .end
        .as_deref()
        .and_then(|d| day_start(d, offset))
        .map(|e| e.saturating_add(86_400));
    json!({
        "enableAutoReply": v.enabled,
        "responseSubject": v.subject,
        "responseBodyPlainText": v.message,
        "restrictToContacts": false,
        "restrictToDomain": false,
        "startTime": start.map(|s| s.saturating_mul(1000).to_string()),
        "endTime": end.map(|e| e.saturating_mul(1000).to_string()),
    })
}

const VACATION_URL: &str = "https://gmail.googleapis.com/gmail/v1/users/me/settings/vacation";

/// The account's automatic replies.
///
/// # Errors
/// Network errors and error replies.
pub async fn get_vacation(client: &RestClient, offset: i32) -> Result<Vacation, SyncError> {
    Ok(vacation_from_json(&client.get(VACATION_URL).await?, offset))
}

/// Save the account's automatic replies.
///
/// # Errors
/// Network errors and error replies.
pub async fn set_vacation(
    client: &RestClient,
    v: &Vacation,
    offset: i32,
) -> Result<Vacation, SyncError> {
    let reply = client
        .send(Method::PUT, VACATION_URL, Some(&vacation_json(v, offset)))
        .await?;
    Ok(vacation_from_json(&reply.body, offset))
}

// ---------------------------------------------------------- contact groups

/// A contact group the user made (`contactGroups/…`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleGroup {
    /// `contactGroups/…`.
    pub resource_name: String,
    /// Its version tag.
    pub etag: Option<String>,
    /// Its name.
    pub name: String,
}

/// The user's own groups in a `contactGroups` reply (Google's system
/// groups, such as "starred", are left out).
#[must_use]
pub fn groups_from_json(items: &[Value]) -> Vec<GoogleGroup> {
    items
        .iter()
        .filter(|g| text(g, "groupType") == "USER_CONTACT_GROUP")
        .filter(|g| !text(g, "resourceName").is_empty())
        .map(|g| GoogleGroup {
            resource_name: text(g, "resourceName").to_owned(),
            etag: Some(text(g, "etag").to_owned()).filter(|e| !e.is_empty()),
            name: [text(g, "formattedName"), text(g, "name")]
                .into_iter()
                .find(|n| !n.trim().is_empty())
                .unwrap_or_default()
                .trim()
                .to_owned(),
        })
        .collect()
}

/// From a `people/me/connections` listing: each person's resource name
/// with their addresses, and each group's members (resource names).
#[must_use]
pub fn people_from_json(
    items: &[Value],
) -> (HashMap<String, Vec<String>>, HashMap<String, Vec<String>>) {
    let mut emails: HashMap<String, Vec<String>> = HashMap::new();
    let mut members: HashMap<String, Vec<String>> = HashMap::new();
    for person in items {
        let name = text(person, "resourceName");
        if name.is_empty() {
            continue;
        }
        let addresses: Vec<String> = person
            .get("emailAddresses")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .map(|e| text(e, "value").trim().to_owned())
                    .filter(|e| e.contains('@'))
                    .collect()
            })
            .unwrap_or_default();
        emails.insert(name.to_owned(), addresses);
        for membership in person
            .get("memberships")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let group = membership
                .get("contactGroupMembership")
                .map(|m| text(m, "contactGroupResourceName"))
                .unwrap_or_default();
            if !group.is_empty() {
                members
                    .entry(group.to_owned())
                    .or_default()
                    .push(name.to_owned());
            }
        }
    }
    (emails, members)
}

/// The user's groups, and from their contacts: addresses by person and
/// members by group.
///
/// # Errors
/// Network errors and error replies.
pub async fn list_groups(
    client: &RestClient,
) -> Result<
    (
        Vec<GoogleGroup>,
        HashMap<String, Vec<String>>,
        HashMap<String, Vec<String>>,
    ),
    SyncError,
> {
    let first = with_query(
        "https://people.googleapis.com/v1/contactGroups",
        &[("pageSize", "1000"), ("groupFields", "name,groupType")],
    )?;
    let (groups, _) = all_pages(client, first, "contactGroups", next_page, 10).await?;
    let first = with_query(
        "https://people.googleapis.com/v1/people/me/connections",
        &[
            ("pageSize", "1000"),
            ("personFields", "emailAddresses,memberships"),
        ],
    )?;
    let (people, _) = all_pages(client, first, "connections", next_page, 30).await?;
    let (emails, members) = people_from_json(&people);
    Ok((groups_from_json(&groups), emails, members))
}

/// Create a group.
///
/// # Errors
/// Network errors and error replies.
pub async fn create_group(client: &RestClient, name: &str) -> Result<GoogleGroup, SyncError> {
    let reply = client
        .send(
            Method::POST,
            "https://people.googleapis.com/v1/contactGroups",
            Some(&json!({ "contactGroup": { "name": name } })),
        )
        .await?;
    groups_from_json(std::slice::from_ref(&reply.body))
        .into_iter()
        .next()
        .ok_or_else(|| SyncError::Protocol("Google didn't return the group".to_owned()))
}

/// Rename a group.
///
/// # Errors
/// Network errors and error replies.
pub async fn rename_group(
    client: &RestClient,
    group: &GoogleGroup,
    name: &str,
) -> Result<GoogleGroup, SyncError> {
    let url = format!("https://people.googleapis.com/v1/{}", group.resource_name);
    let reply = client
        .send(
            Method::PUT,
            &url,
            Some(&json!({ "contactGroup": { "name": name, "etag": group.etag } })),
        )
        .await?;
    groups_from_json(std::slice::from_ref(&reply.body))
        .into_iter()
        .next()
        .ok_or_else(|| SyncError::Protocol("Google didn't return the group".to_owned()))
}

/// Add and remove people (`people/…`) in a group.
///
/// # Errors
/// Network errors and error replies.
pub async fn change_members(
    client: &RestClient,
    resource_name: &str,
    add: &[String],
    remove: &[String],
) -> Result<(), SyncError> {
    if add.is_empty() && remove.is_empty() {
        return Ok(());
    }
    let url = format!("https://people.googleapis.com/v1/{resource_name}/members:modify");
    client
        .send(
            Method::POST,
            &url,
            Some(&json!({ "resourceNamesToAdd": add, "resourceNamesToRemove": remove })),
        )
        .await?;
    Ok(())
}

/// Delete a group, keeping its people.
///
/// # Errors
/// Network errors and error replies.
pub async fn delete_group(client: &RestClient, resource_name: &str) -> Result<(), SyncError> {
    let url = with_query(
        &format!("https://people.googleapis.com/v1/{resource_name}"),
        &[("deleteContacts", "false")],
    )?;
    match client.send(Method::DELETE, &url, None).await {
        Ok(_) => Ok(()),
        Err(SyncError::Refused(why)) if why.contains("404") => Ok(()),
        Err(other) => Err(other),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn calendar_list_mapping() {
        // Recorded from calendarList.list (trimmed).
        let reply: Value = serde_json::from_str(r##"{"kind":"calendar#calendarList","items":[
            {"kind":"calendar#calendarListEntry","id":"don@gmail.com","summary":"don@gmail.com","timeZone":"Africa/Nairobi","backgroundColor":"#9fe1e7","foregroundColor":"#000000","selected":true,"accessRole":"owner","primary":true},
            {"kind":"calendar#calendarListEntry","id":"en.ke#holiday@group.v.calendar.google.com","summary":"Holidays in Kenya","backgroundColor":"#16a765","accessRole":"reader"},
            {"kind":"calendar#calendarListEntry","id":"team@group.calendar.google.com","summary":"Team","summaryOverride":"Site team","backgroundColor":"#ff7537","accessRole":"writer"},
            {"id":"gone@group.calendar.google.com","summary":"Old","accessRole":"owner","deleted":true}
        ]}"##).unwrap();
        let cals = calendars_from_json(reply["items"].as_array().unwrap());
        assert_eq!(cals.len(), 3);
        assert!(cals[0].primary && cals[0].writable);
        assert_eq!(cals[0].color.as_deref(), Some("#9fe1e7"));
        assert!(!cals[1].writable);
        assert_eq!(cals[2].name, "Site team");
        assert_eq!(
            caldav_events_url(&cals[1].id),
            "https://apidata.googleusercontent.com/caldav/v2/en.ke%23holiday%40group.v.calendar.google.com/events/"
        );
        assert_eq!(
            carddav_book_url("don@gmail.com"),
            "https://www.googleapis.com/carddav/v1/principals/don%40gmail.com/lists/default/"
        );
    }

    #[test]
    fn free_busy_mapping() {
        let reply: Value = serde_json::from_str(r#"{"kind":"calendar#freeBusy","timeMin":"2026-11-18T00:00:00.000Z","timeMax":"2026-11-19T00:00:00.000Z","calendars":{
            "peter@client.example":{"busy":[{"start":"2026-11-18T06:00:00Z","end":"2026-11-18T07:00:00Z"},{"start":"2026-11-18T12:00:00+03:00","end":"2026-11-18T13:00:00+03:00"}]},
            "who@elsewhere.example":{"errors":[{"domain":"global","reason":"notFound"}],"busy":[]}
        }}"#).unwrap();
        let emails = [
            "peter@client.example".to_owned(),
            "who@elsewhere.example".to_owned(),
            "missing@x.example".to_owned(),
        ];
        let out = free_busy_from_json(&reply, &emails);
        let peter = out[0].1.as_ref().unwrap();
        assert_eq!(peter.len(), 2);
        // 12:00 in Nairobi is 09:00 UTC on 18 November.
        assert_eq!(peter[1].start, 1_794_992_400);
        assert_eq!(peter[0].end - peter[0].start, 3600);
        assert!(out[1].1.is_none());
        assert!(out[2].1.is_none());
    }

    #[test]
    fn google_tasks_mapping() {
        // Recorded from tasks.list (trimmed).
        let reply: Value = serde_json::from_str(r#"{"kind":"tasks#tasks","items":[
            {"kind":"tasks#task","id":"MTIz","etag":"\"LTE5\"","title":"Send the quote","updated":"2026-11-18T09:00:00.000Z","notes":"Kilimani fit-out","status":"needsAction","due":"2026-11-20T00:00:00.000Z"},
            {"kind":"tasks#task","id":"NDU2","etag":"\"LTIw\"","title":"Call Peter","status":"completed","completed":"2026-11-17T08:30:00.000Z"},
            {"kind":"tasks#task","id":"Nzg5","title":"Gone","status":"needsAction","deleted":true}
        ]}"#).unwrap();
        let tasks: Vec<RemoteTask> = reply["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(task_from_json)
            .collect();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "MTIz");
        assert_eq!(tasks[0].etag.as_deref(), Some("\"LTE5\""));
        assert_eq!(tasks[0].fields.due, Some(1_795_132_800));
        assert_eq!(tasks[0].fields.notes, "Kilimani fit-out");
        assert!(tasks[1].fields.completed);
        assert_eq!(tasks[1].fields.completed_at, Some(1_794_904_200));
        // And back.
        let sent = task_json(&tasks[0].fields);
        assert_eq!(sent["due"], "2026-11-20T00:00:00.000Z");
        assert_eq!(sent["status"], "needsAction");
        assert!(sent["completed"].is_null());
        let back = task_json(&tasks[1].fields);
        assert_eq!(back["status"], "completed");
        assert_eq!(back["completed"], "2026-11-17T08:30:00Z");
    }

    #[test]
    fn gmail_vacation_mapping() {
        let reply: Value = serde_json::from_str(r#"{"enableAutoReply":true,"responseSubject":"Away","responseBodyPlainText":"Back on Monday.","restrictToContacts":false,"restrictToDomain":false,"startTime":"1797714000000","endTime":"1798923600000"}"#).unwrap();
        // Nairobi, three hours east of UTC.
        let v = vacation_from_json(&reply, 3 * 3600);
        assert!(v.enabled);
        assert_eq!(v.subject, "Away");
        assert_eq!(v.message, "Back on Monday.");
        assert_eq!(v.start.as_deref(), Some("2026-12-20"));
        assert_eq!(v.end.as_deref(), Some("2027-01-02"));
        let sent = vacation_json(&v, 3 * 3600);
        assert_eq!(sent["startTime"], "1797714000000");
        assert_eq!(sent["endTime"], "1798923600000");
        let off = vacation_from_json(&serde_json::json!({"enableAutoReply": false}), 0);
        assert!(!off.enabled);
        assert_eq!(off.start, None);
    }

    #[test]
    fn contact_groups_mapping() {
        let groups: Value = serde_json::from_str(r#"{"contactGroups":[
            {"resourceName":"contactGroups/starred","groupType":"SYSTEM_CONTACT_GROUP","name":"starred","formattedName":"Starred"},
            {"resourceName":"contactGroups/5b1f","etag":"g1","groupType":"USER_CONTACT_GROUP","name":"Site team","formattedName":"Site team"}
        ]}"#).unwrap();
        let mine = groups_from_json(groups["contactGroups"].as_array().unwrap());
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].name, "Site team");
        let people: Value = serde_json::from_str(r#"{"connections":[
            {"resourceName":"people/c1","emailAddresses":[{"value":"peter@client.example"}],"memberships":[{"contactGroupMembership":{"contactGroupResourceName":"contactGroups/5b1f"}},{"contactGroupMembership":{"contactGroupResourceName":"contactGroups/myContacts"}}]},
            {"resourceName":"people/c2","emailAddresses":[{"value":"amina@studio.example"}]}
        ]}"#).unwrap();
        let (emails, members) = people_from_json(people["connections"].as_array().unwrap());
        assert_eq!(emails["people/c1"], ["peter@client.example"]);
        assert_eq!(members["contactGroups/5b1f"], ["people/c1"]);
        assert!(!members.contains_key("contactGroups/other"));
    }

    #[test]
    fn pages_follow_the_token() {
        let next = next_page(
            &serde_json::json!({"nextPageToken": "abc"}),
            "https://x.example/l?maxResults=100&pageToken=old",
        )
        .unwrap();
        assert_eq!(next, "https://x.example/l?maxResults=100&pageToken=abc");
        assert!(next_page(&serde_json::json!({}), "https://x.example/l").is_none());
    }
}
