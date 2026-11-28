//! Reading Exchange's answers: response messages and their errors, folders,
//! item changes, list entries, MIME content, automatic replies and free
//! times.

use base64::Engine as _;

use super::xml::Element;
use crate::SyncError;

/// Largest MIME content taken from one item, bytes of base64.
const MAX_MIME_BASE64: usize = 96 * 1024 * 1024;

/// One response message of an answer (`…ResponseMessage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response<'a> {
    /// `Success`, `Warning` or `Error`.
    pub class: &'a str,
    /// `NoError`, `ErrorItemNotFound`, …
    pub code: &'a str,
    /// The server's words.
    pub text: &'a str,
    /// The element itself.
    pub element: &'a Element,
}

impl Response<'_> {
    /// Whether it worked (a warning counts).
    #[must_use]
    pub fn ok(&self) -> bool {
        self.class != "Error"
    }

    /// The error it stands for, if it failed.
    #[must_use]
    pub fn error(&self) -> Option<SyncError> {
        (!self.ok()).then(|| error_for(self.code, self.text))
    }
}

/// Plain words and a category for an EWS error code.
#[must_use]
pub fn error_for(code: &str, text: &str) -> SyncError {
    let said = if text.is_empty() { code } else { text };
    match code {
        "ErrorServerBusy"
        | "ErrorTimeoutExpired"
        | "ErrorInternalServerTransientError"
        | "ErrorMailboxStoreUnavailable"
        | "ErrorConnectionFailed"
        | "ErrorTooManyObjectsOpened" => SyncError::Protocol(format!("Exchange is busy ({said})")),
        "ErrorAccessDenied" | "ErrorNoRespondingCASInDestinationSite" => {
            SyncError::Refused(format!("Exchange doesn't allow that ({said})"))
        }
        "ErrorNonExistentMailbox" | "ErrorMailboxMoveInProgress" => {
            SyncError::Config(format!("Exchange can't find this mailbox ({said})"))
        }
        _ => SyncError::Refused(said.to_owned()),
    }
}

/// The SOAP body of an answer, or the fault it carries.
///
/// # Errors
/// A SOAP fault, or XML that is no SOAP answer.
pub fn body(root: &Element) -> Result<&Element, SyncError> {
    let body = root
        .child("Body")
        .ok_or_else(|| SyncError::Protocol("Exchange's answer has no SOAP body".into()))?;
    if let Some(fault) = body.child("Fault") {
        let code = fault
            .find("ResponseCode")
            .map(|c| c.text.trim())
            .unwrap_or_default();
        let text = fault.text_of("faultstring").unwrap_or("a SOAP fault");
        return Err(match code {
            "" if text.contains("schema validation") => SyncError::Refused(format!(
                "this Exchange server doesn't support that ({text})"
            )),
            "" => SyncError::Protocol(format!("Exchange answered with a fault: {text}")),
            code => error_for(code, text),
        });
    }
    Ok(body)
}

/// Every response message of an answer, in order.
///
/// # Errors
/// As [`body`].
pub fn responses(root: &Element) -> Result<Vec<Response<'_>>, SyncError> {
    let body = body(root)?;
    let mut out = Vec::new();
    collect_responses(body, &mut out);
    Ok(out)
}

fn collect_responses<'a>(element: &'a Element, out: &mut Vec<Response<'a>>) {
    for child in &element.children {
        if child.name.ends_with("ResponseMessage") && child.attr("ResponseClass").is_some() {
            out.push(Response {
                class: child.attr("ResponseClass").unwrap_or("Error"),
                code: child.text_of("ResponseCode").unwrap_or(""),
                text: child.text_of("MessageText").unwrap_or(""),
                element: child,
            });
        } else {
            collect_responses(child, out);
        }
    }
}

/// The first response message, which must have worked.
///
/// # Errors
/// As [`body`], or the message's own error.
pub fn single(root: &Element) -> Result<&Element, SyncError> {
    let all = responses(root)?;
    let first = all
        .first()
        .ok_or_else(|| SyncError::Protocol("Exchange's answer has no response".into()))?;
    if let Some(error) = first.error() {
        return Err(error);
    }
    Ok(first.element)
}

/// A folder of the mailbox.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folder {
    /// Exchange's id.
    pub id: String,
    /// The folder it is in.
    pub parent_id: Option<String>,
    /// Its name.
    pub name: String,
    /// `IPF.Note` (mail), `IPF.Appointment`, `IPF.Contact`, `IPF.Task`…;
    /// `None` reads as mail.
    pub class: Option<String>,
    /// Items in it.
    pub total: Option<u32>,
    /// Unread items in it.
    pub unread: Option<u32>,
}

impl Folder {
    /// Holds mail.
    #[must_use]
    pub fn is_mail(&self) -> bool {
        self.class
            .as_deref()
            .is_none_or(|c| c == "IPF.Note" || c.starts_with("IPF.Note."))
    }

    /// Holds calendar items.
    #[must_use]
    pub fn is_calendar(&self) -> bool {
        self.class
            .as_deref()
            .is_some_and(|c| c == "IPF.Appointment" || c.starts_with("IPF.Appointment."))
    }

    /// Holds contacts.
    #[must_use]
    pub fn is_contacts(&self) -> bool {
        self.class
            .as_deref()
            .is_some_and(|c| c == "IPF.Contact" || c.starts_with("IPF.Contact."))
    }
}

const FOLDER_ELEMENTS: [&str; 5] = [
    "Folder",
    "CalendarFolder",
    "ContactsFolder",
    "TasksFolder",
    "SearchFolder",
];

fn folder_from(element: &Element) -> Option<Folder> {
    if !FOLDER_ELEMENTS.contains(&element.name.as_str()) || element.name == "SearchFolder" {
        return None;
    }
    let id = element.child("FolderId")?.attr("Id")?.to_owned();
    let number = |name: &str| element.text_of(name).and_then(|t| t.parse().ok());
    Some(Folder {
        id,
        parent_id: element
            .child("ParentFolderId")
            .and_then(|p| p.attr("Id"))
            .map(str::to_owned),
        name: element.text_of("DisplayName").unwrap_or("").to_owned(),
        class: element.text_of("FolderClass").map(str::to_owned),
        total: number("TotalCount"),
        unread: number("UnreadCount"),
    })
}

/// What `SyncFolderHierarchy` brought.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HierarchyChanges {
    /// The state to ask from next time.
    pub sync_state: String,
    /// Every change is in this answer.
    pub done: bool,
    /// Folders new or changed.
    pub changed: Vec<Folder>,
    /// Folders gone (ids).
    pub deleted: Vec<String>,
}

/// Read a `SyncFolderHierarchy` answer.
///
/// # Errors
/// As [`single`]; `ErrorInvalidSyncStateData` comes back as
/// [`SyncError::Refused`] with that code, so the caller can start over.
pub fn hierarchy(root: &Element) -> Result<HierarchyChanges, SyncError> {
    let message = single_keeping_code(root)?;
    let mut out = HierarchyChanges {
        sync_state: message.text_of("SyncState").unwrap_or("").to_owned(),
        done: message.text_of("IncludesLastFolderInRange") != Some("false"),
        ..HierarchyChanges::default()
    };
    if let Some(changes) = message.child("Changes") {
        for change in &changes.children {
            match change.name.as_str() {
                "Create" | "Update" => {
                    if let Some(folder) = change.children.iter().find_map(folder_from) {
                        out.changed.push(folder);
                    }
                }
                "Delete" => {
                    if let Some(id) = change.child("FolderId").and_then(|f| f.attr("Id")) {
                        out.deleted.push(id.to_owned());
                    }
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

/// The error code that means a sync state is no longer any good.
pub const INVALID_SYNC_STATE: &str = "ErrorInvalidSyncStateData";

/// Like [`single`], but a stale sync state is reported with its code.
fn single_keeping_code(root: &Element) -> Result<&Element, SyncError> {
    let all = responses(root)?;
    let first = all
        .first()
        .ok_or_else(|| SyncError::Protocol("Exchange's answer has no response".into()))?;
    if !first.ok() {
        if first.code == INVALID_SYNC_STATE || first.code == "ErrorInvalidSyncStateData" {
            return Err(SyncError::Refused(INVALID_SYNC_STATE.into()));
        }
        return Err(error_for(first.code, first.text));
    }
    Ok(first.element)
}

/// Well-known folders' ids, in the order asked (`None` where the server
/// has none).
///
/// # Errors
/// As [`responses`].
pub fn folder_ids(root: &Element) -> Result<Vec<Option<String>>, SyncError> {
    Ok(responses(root)?
        .iter()
        .map(|r| {
            r.ok()
                .then(|| {
                    r.element
                        .find("FolderId")
                        .and_then(|f| f.attr("Id"))
                        .map(str::to_owned)
                })
                .flatten()
        })
        .collect())
}

/// The id of a folder just made.
///
/// # Errors
/// As [`single`].
pub fn created_folder(root: &Element) -> Result<String, SyncError> {
    single(root)?
        .find("FolderId")
        .and_then(|f| f.attr("Id"))
        .map(str::to_owned)
        .ok_or_else(|| SyncError::Protocol("Exchange made the folder but didn't say its id".into()))
}

/// An item's state, as the list needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemState {
    /// Exchange's id.
    pub id: String,
    /// Its change key.
    pub change_key: Option<String>,
    /// Read.
    pub read: Option<bool>,
    /// Flagged for follow-up.
    pub flagged: Option<bool>,
    /// Its categories.
    pub categories: Option<Vec<String>>,
    /// `IPM.Note`, `IPM.Schedule.Meeting.Request`…
    pub class: Option<String>,
}

fn boolean(text: Option<&str>) -> Option<bool> {
    match text? {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

const ITEM_ELEMENTS: [&str; 10] = [
    "Item",
    "Message",
    "MeetingMessage",
    "MeetingRequest",
    "MeetingResponse",
    "MeetingCancellation",
    "CalendarItem",
    "Contact",
    "DistributionList",
    "PostItem",
];

/// Whether a `PropertyTag` (`0x7d`, `0x007D`, or a decimal number) is
/// this property.
fn tag_is(tag: &str, wanted: u32) -> bool {
    let tag = tag.trim();
    let number = match tag.strip_prefix("0x").or_else(|| tag.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => tag.parse().ok(),
    };
    number == Some(wanted)
}

/// The value of an extended property of an item.
fn extended(item: &Element, tag: u32) -> Option<&Element> {
    item.children_named("ExtendedProperty").find_map(|p| {
        let at = p.child("ExtendedFieldURI")?.attr("PropertyTag")?;
        tag_is(at, tag).then(|| p.child("Value")).flatten()
    })
}

fn flag_of(item: &Element) -> Option<bool> {
    extended(item, 0x1090).map(|v| v.text.trim() == "2")
}

fn state_from(item: &Element) -> Option<ItemState> {
    if !ITEM_ELEMENTS.contains(&item.name.as_str()) && item.name != "Task" {
        return None;
    }
    let id = item.child("ItemId")?;
    Some(ItemState {
        id: id.attr("Id")?.to_owned(),
        change_key: id.attr("ChangeKey").map(str::to_owned),
        read: boolean(item.text_of("IsRead")),
        // A list entry without the property is not flagged.
        flagged: Some(flag_of(item).unwrap_or(false)),
        categories: item.child("Categories").map(|c| {
            c.children_named("String")
                .map(|s| s.text.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect()
        }),
        class: item.text_of("ItemClass").map(str::to_owned),
    })
}

/// One change in a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemChange {
    /// A new item.
    Created(ItemState),
    /// A changed item.
    Updated(ItemState),
    /// An item gone (its id).
    Deleted(String),
    /// Read or unread.
    ReadChanged {
        /// The item.
        id: String,
        /// Read now.
        read: bool,
    },
}

/// What `SyncFolderItems` brought.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemChanges {
    /// The state to ask from next time.
    pub sync_state: String,
    /// Every change is in this answer.
    pub done: bool,
    /// The changes, in order.
    pub changes: Vec<ItemChange>,
}

/// Read a `SyncFolderItems` answer.
///
/// # Errors
/// As [`hierarchy`].
pub fn item_changes(root: &Element) -> Result<ItemChanges, SyncError> {
    let message = single_keeping_code(root)?;
    let mut out = ItemChanges {
        sync_state: message.text_of("SyncState").unwrap_or("").to_owned(),
        done: message.text_of("IncludesLastItemInRange") != Some("false"),
        changes: Vec::new(),
    };
    let Some(changes) = message.child("Changes") else {
        return Ok(out);
    };
    for change in &changes.children {
        let item = || change.children.iter().find_map(state_from);
        let id = || {
            change
                .child("ItemId")
                .and_then(|i| i.attr("Id"))
                .map(str::to_owned)
        };
        let next = match change.name.as_str() {
            "Create" => item().map(ItemChange::Created),
            "Update" => item().map(ItemChange::Updated),
            "Delete" => id().map(ItemChange::Deleted),
            "ReadFlagChange" => id()
                .zip(boolean(change.text_of("IsRead")))
                .map(|(id, read)| ItemChange::ReadChanged { id, read }),
            _ => None,
        };
        out.changes.extend(next);
    }
    Ok(out)
}

/// A page of `FindItem`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoundItems {
    /// The items, in the order asked.
    pub items: Vec<ItemState>,
    /// This page holds the last item.
    pub last: bool,
    /// Items in the whole view.
    pub total: u32,
}

/// Read a `FindItem` answer.
///
/// # Errors
/// As [`single`].
pub fn found_items(root: &Element) -> Result<FoundItems, SyncError> {
    let message = single(root)?;
    let Some(folder) = message.child("RootFolder") else {
        return Ok(FoundItems::default());
    };
    Ok(FoundItems {
        items: folder
            .child("Items")
            .map(|items| items.children.iter().filter_map(state_from).collect())
            .unwrap_or_default(),
        last: folder.attr("IncludesLastItemInRange") != Some("false"),
        total: folder
            .attr("TotalItemsInView")
            .and_then(|t| t.parse().ok())
            .unwrap_or(0),
    })
}

/// A message's list entry as Exchange has it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers {
    /// Its state.
    pub state: ItemState,
    /// The header block it arrived with (received mail only).
    pub transport_headers: Option<String>,
    /// Subject.
    pub subject: String,
    /// Sender: name and address.
    pub from: Option<(String, String)>,
    /// To: names and addresses.
    pub to: Vec<(String, String)>,
    /// Cc.
    pub cc: Vec<(String, String)>,
    /// When sent (or received), `YYYY-MM-DDTHH:MM:SSZ`.
    pub date: Option<String>,
    /// Message-ID.
    pub message_id: Option<String>,
    /// In-Reply-To.
    pub in_reply_to: Option<String>,
    /// References.
    pub references: Option<String>,
    /// `Low`, `Normal` or `High`.
    pub importance: Option<String>,
    /// Bytes.
    pub size: Option<u64>,
}

fn mailbox(element: &Element) -> Option<(String, String)> {
    let mailbox = if element.name == "Mailbox" {
        element
    } else {
        element.child("Mailbox")?
    };
    let address = mailbox.text_of("EmailAddress").unwrap_or("").to_owned();
    let name = mailbox.text_of("Name").unwrap_or("").to_owned();
    (!address.is_empty() || !name.is_empty()).then_some((name, address))
}

fn mailboxes(element: Option<&Element>) -> Vec<(String, String)> {
    element
        .map(|list| list.children_named("Mailbox").filter_map(mailbox).collect())
        .unwrap_or_default()
}

/// Read a `GetItem` answer for list entries. Items that failed (deleted
/// meanwhile) are left out.
///
/// # Errors
/// As [`responses`].
pub fn headers(root: &Element) -> Result<Vec<Headers>, SyncError> {
    let mut out = Vec::new();
    for response in responses(root)?.iter().filter(|r| r.ok()) {
        let Some(items) = response.element.child("Items") else {
            continue;
        };
        for item in &items.children {
            let Some(state) = state_from(item) else {
                continue;
            };
            let transport_headers = extended(item, 0x007D)
                .map(|v| v.text.clone())
                .filter(|v| !v.trim().is_empty());
            out.push(Headers {
                state,
                transport_headers,
                subject: item.text_of("Subject").unwrap_or("").to_owned(),
                from: item.child("From").and_then(mailbox),
                to: mailboxes(item.child("ToRecipients")),
                cc: mailboxes(item.child("CcRecipients")),
                date: item
                    .text_of("DateTimeSent")
                    .or_else(|| item.text_of("DateTimeReceived"))
                    .map(str::to_owned),
                message_id: item.text_of("InternetMessageId").map(str::to_owned),
                in_reply_to: item.text_of("InReplyTo").map(str::to_owned),
                references: item.text_of("References").map(str::to_owned),
                importance: item.text_of("Importance").map(str::to_owned),
                size: item.text_of("Size").and_then(|s| s.parse().ok()),
            });
        }
    }
    Ok(out)
}

/// One item's MIME content, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mime {
    /// The item.
    pub id: String,
    /// Its change key.
    pub change_key: Option<String>,
    /// The bytes.
    pub raw: Vec<u8>,
}

/// Decode `MimeContent` (Base64, line breaks allowed).
#[must_use]
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    if text.len() > MAX_MIME_BASE64 {
        return None;
    }
    let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD.decode(clean).ok()
}

/// Read a `GetItem` answer with MIME content. Items that failed are left
/// out.
///
/// # Errors
/// As [`responses`].
pub fn mime(root: &Element) -> Result<Vec<Mime>, SyncError> {
    let mut out = Vec::new();
    for response in responses(root)?.iter().filter(|r| r.ok()) {
        let Some(items) = response.element.child("Items") else {
            continue;
        };
        for item in &items.children {
            let Some(id) = item.child("ItemId") else {
                continue;
            };
            let Some(raw) = item
                .child("MimeContent")
                .and_then(|m| decode_base64(&m.text))
            else {
                continue;
            };
            out.push(Mime {
                id: id.attr("Id").unwrap_or("").to_owned(),
                change_key: id.attr("ChangeKey").map(str::to_owned),
                raw,
            });
        }
    }
    Ok(out)
}

/// The ids of the items an answer made (`CreateItem`, `CopyItem`…).
///
/// # Errors
/// As [`responses`], or the first failure.
pub fn created_items(root: &Element) -> Result<Vec<(String, Option<String>)>, SyncError> {
    let mut out = Vec::new();
    for response in responses(root)? {
        if let Some(error) = response.error() {
            return Err(error);
        }
        for id in response.element.descendants("ItemId") {
            if let Some(value) = id.attr("Id") {
                out.push((value.to_owned(), id.attr("ChangeKey").map(str::to_owned)));
            }
        }
    }
    Ok(out)
}

/// Check every response of a change to several items. Items already gone
/// count as done (moved or deleted elsewhere).
///
/// # Errors
/// The first other failure.
pub fn all_done(root: &Element) -> Result<(), SyncError> {
    for response in responses(root)? {
        if response.ok() || response.code == "ErrorItemNotFound" {
            continue;
        }
        return Err(error_for(response.code, response.text));
    }
    Ok(())
}

/// A contact's fields as Exchange has them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contact {
    /// Exchange's id.
    pub id: String,
    /// Its change key.
    pub change_key: Option<String>,
    /// Display name (or first and last name).
    pub name: String,
    /// Addresses, `EmailAddress1` first.
    pub emails: Vec<String>,
    /// Phone numbers: mobile, business, home, then the rest.
    pub phones: Vec<String>,
    /// Company.
    pub company: String,
    /// Job title.
    pub title: String,
    /// Notes.
    pub notes: String,
    /// Categories.
    pub categories: Vec<String>,
}

/// Read a `GetItem` answer for contacts (distribution lists are left out).
///
/// # Errors
/// As [`responses`].
pub fn contacts(root: &Element) -> Result<Vec<Contact>, SyncError> {
    let mut out = Vec::new();
    for response in responses(root)?.iter().filter(|r| r.ok()) {
        let Some(items) = response.element.child("Items") else {
            continue;
        };
        for item in items.children_named("Contact") {
            let Some(id) = item.child("ItemId") else {
                continue;
            };
            let entries = |list: &str| -> Vec<(String, String)> {
                item.child(list)
                    .map(|l| {
                        l.children_named("Entry")
                            .map(|e| {
                                (
                                    e.attr("Key").unwrap_or("").to_owned(),
                                    e.text.trim().to_owned(),
                                )
                            })
                            .filter(|(_, v)| !v.is_empty())
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let mut emails = entries("EmailAddresses");
            emails.sort_by(|a, b| a.0.cmp(&b.0));
            let emails: Vec<String> = emails
                .into_iter()
                .map(|(_, v)| {
                    v.strip_prefix("SMTP:")
                        .or_else(|| v.strip_prefix("smtp:"))
                        .map_or(v.clone(), str::to_owned)
                })
                .filter(|v| v.contains('@'))
                .collect();
            let all_phones = entries("PhoneNumbers");
            let mut phones: Vec<String> = Vec::new();
            for key in super::soap::PHONE_KEYS {
                for (k, v) in &all_phones {
                    if k == key && !phones.contains(v) {
                        phones.push(v.clone());
                    }
                }
            }
            for (_, v) in &all_phones {
                if !phones.contains(v) {
                    phones.push(v.clone());
                }
            }
            let mut name = item.text_of("DisplayName").unwrap_or("").to_owned();
            if name.is_empty() {
                name = [item.text_of("GivenName"), item.text_of("Surname")]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" ");
            }
            out.push(Contact {
                id: id.attr("Id").unwrap_or("").to_owned(),
                change_key: id.attr("ChangeKey").map(str::to_owned),
                name,
                emails,
                phones,
                company: item.text_of("CompanyName").unwrap_or("").to_owned(),
                title: item.text_of("JobTitle").unwrap_or("").to_owned(),
                notes: item.text_of("Body").unwrap_or("").to_owned(),
                categories: item
                    .child("Categories")
                    .map(|c| {
                        c.children_named("String")
                            .map(|s| s.text.trim().to_owned())
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
    }
    Ok(out)
}

/// Read a `GetUserOofSettings` answer.
///
/// # Errors
/// As [`body`], or the answer's own error.
pub fn oof(root: &Element) -> Result<super::soap::Oof, SyncError> {
    let body = body(root)?;
    let answer = body.child("GetUserOofSettingsResponse").ok_or_else(|| {
        SyncError::Protocol("Exchange didn't answer with automatic replies".into())
    })?;
    if let Some(message) = answer.child("ResponseMessage")
        && message.attr("ResponseClass") == Some("Error")
    {
        return Err(error_for(
            message.text_of("ResponseCode").unwrap_or(""),
            message.text_of("MessageText").unwrap_or(""),
        ));
    }
    let settings = answer
        .child("OofSettings")
        .ok_or_else(|| SyncError::Protocol("Exchange's automatic replies are missing".into()))?;
    let reply = |name: &str| {
        settings
            .child(name)
            .and_then(|r| r.child("Message"))
            .map(|m| m.text.clone())
            .unwrap_or_default()
    };
    let duration = settings.child("Duration").and_then(|d| {
        Some((
            d.text_of("StartTime")?.to_owned(),
            d.text_of("EndTime")?.to_owned(),
        ))
    });
    Ok(super::soap::Oof {
        state: settings
            .text_of("OofState")
            .unwrap_or("Disabled")
            .to_owned(),
        external_audience: settings
            .text_of("ExternalAudience")
            .unwrap_or("All")
            .to_owned(),
        duration,
        internal: reply("InternalReply"),
        external: reply("ExternalReply"),
    })
}

/// Read a `SetUserOofSettings` answer.
///
/// # Errors
/// As [`body`], or the answer's own error.
pub fn oof_saved(root: &Element) -> Result<(), SyncError> {
    let body = body(root)?;
    match body.find("ResponseMessage") {
        Some(message) if message.attr("ResponseClass") == Some("Error") => Err(error_for(
            message.text_of("ResponseCode").unwrap_or(""),
            message.text_of("MessageText").unwrap_or(""),
        )),
        _ => Ok(()),
    }
}

/// One busy stretch: start and end (as Exchange wrote them, UTC without a
/// zone) and `Busy`, `Tentative`, `OOF` or `Free`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Busy {
    /// Start.
    pub start: String,
    /// End.
    pub end: String,
    /// What kind.
    pub kind: String,
}

/// Read a `GetUserAvailability` answer: per person asked, in order, their
/// busy times, or `None` when Exchange couldn't tell.
///
/// # Errors
/// As [`body`].
pub fn availability(root: &Element) -> Result<Vec<Option<Vec<Busy>>>, SyncError> {
    let body = body(root)?;
    let Some(array) = body.find("FreeBusyResponseArray") else {
        return Ok(Vec::new());
    };
    Ok(array
        .children_named("FreeBusyResponse")
        .map(|response| {
            let ok = response
                .child("ResponseMessage")
                .and_then(|m| m.attr("ResponseClass"))
                != Some("Error");
            ok.then(|| {
                response
                    .find("CalendarEventArray")
                    .map(|events| {
                        events
                            .children_named("CalendarEvent")
                            .filter_map(|e| {
                                Some(Busy {
                                    start: e.text_of("StartTime")?.to_owned(),
                                    end: e.text_of("EndTime")?.to_owned(),
                                    kind: e.text_of("BusyType").unwrap_or("Busy").to_owned(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
        })
        .collect())
}
