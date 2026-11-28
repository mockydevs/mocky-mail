//! The SOAP requests Mocky Mail sends to Exchange Web Services, as text.
//! Every value is escaped; element order follows the EWS schema
//! (`types.xsd`, `messages.xsd`) of Exchange 2010 SP2, the version asked for,
//! which every later Exchange Server answers.

use std::fmt::Write as _;

use super::xml::escape;

/// The schema version asked for.
pub const SERVER_VERSION: &str = "Exchange2010_SP2";

/// Extended MAPI properties read and written.
pub mod props {
    /// `PR_TRANSPORT_MESSAGE_HEADERS`: a received message's header block.
    pub const HEADERS: &str = r#"<t:ExtendedFieldURI PropertyTag="0x007D" PropertyType="String"/>"#;
    /// `PR_FLAG_STATUS`: 2 flagged, 1 complete, absent not flagged.
    pub const FLAG: &str = r#"<t:ExtendedFieldURI PropertyTag="0x1090" PropertyType="Integer"/>"#;
    /// `PR_MESSAGE_FLAGS`, set when a draft is made (8: unsent).
    pub const MESSAGE_FLAGS: &str =
        r#"<t:ExtendedFieldURI PropertyTag="0x0E07" PropertyType="Integer"/>"#;
}

/// A whole SOAP request around one operation.
#[must_use]
pub fn envelope(body: &str) -> String {
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" "#,
            r#"xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types" "#,
            r#"xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages">"#,
            r#"<soap:Header><t:RequestServerVersion Version="{version}"/></soap:Header>"#,
            "<soap:Body>{body}</soap:Body></soap:Envelope>"
        ),
        version = SERVER_VERSION,
        body = body
    )
}

/// A folder named by its id, or one of Exchange's well-known names
/// (`inbox`, `sentitems`, `msgfolderroot`…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderRef<'a> {
    /// A folder id.
    Id(&'a str),
    /// A distinguished folder id.
    Distinguished(&'a str),
}

impl FolderRef<'_> {
    fn xml(&self) -> String {
        match self {
            Self::Id(id) => format!(r#"<t:FolderId Id="{}"/>"#, escape(id)),
            Self::Distinguished(name) => {
                format!(r#"<t:DistinguishedFolderId Id="{}"/>"#, escape(name))
            }
        }
    }
}

fn item_ids(ids: &[String]) -> String {
    ids.iter().fold(String::new(), |mut out, id| {
        let _ = write!(out, r#"<t:ItemId Id="{}"/>"#, escape(id));
        out
    })
}

/// `GetFolder` for well-known folders (their ids, to know which is which).
#[must_use]
pub fn get_folders(distinguished: &[&str]) -> String {
    let ids = distinguished.iter().fold(String::new(), |mut out, name| {
        let _ = write!(out, r#"<t:DistinguishedFolderId Id="{}"/>"#, escape(name));
        out
    });
    format!(
        "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds>{ids}</m:FolderIds></m:GetFolder>"
    )
}

fn sync_state(state: Option<&str>) -> String {
    state
        .filter(|s| !s.is_empty())
        .map(|s| format!("<m:SyncState>{}</m:SyncState>", escape(s)))
        .unwrap_or_default()
}

/// `SyncFolderHierarchy` from the mailbox's root.
#[must_use]
pub fn sync_folder_hierarchy(state: Option<&str>) -> String {
    format!(
        concat!(
            "<m:SyncFolderHierarchy><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>",
            r#"<t:FieldURI FieldURI="folder:ParentFolderId"/><t:FieldURI FieldURI="folder:DisplayName"/>"#,
            r#"<t:FieldURI FieldURI="folder:FolderClass"/><t:FieldURI FieldURI="folder:TotalCount"/>"#,
            r#"<t:FieldURI FieldURI="folder:UnreadCount"/>"#,
            "</t:AdditionalProperties></m:FolderShape>",
            r#"<m:SyncFolderId><t:DistinguishedFolderId Id="msgfolderroot"/></m:SyncFolderId>"#,
            "{state}</m:SyncFolderHierarchy>"
        ),
        state = sync_state(state)
    )
}

/// The properties a mail item's list entry needs.
const MAIL_STATE_PROPS: &str = concat!(
    r#"<t:FieldURI FieldURI="message:IsRead"/><t:FieldURI FieldURI="item:Categories"/>"#,
    r#"<t:FieldURI FieldURI="item:ItemClass"/>"#,
    r#"<t:ExtendedFieldURI PropertyTag="0x1090" PropertyType="Integer"/>"#
);

/// `SyncFolderItems`: what changed in a folder since `state`. Mail folders
/// get read, flag and categories with each change; others ids only.
#[must_use]
pub fn sync_folder_items(
    folder: &FolderRef<'_>,
    state: Option<&str>,
    max: u32,
    mail: bool,
) -> String {
    let shape = if mail {
        format!(
            "<t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{MAIL_STATE_PROPS}</t:AdditionalProperties>"
        )
    } else {
        "<t:BaseShape>IdOnly</t:BaseShape>".to_owned()
    };
    format!(
        "<m:SyncFolderItems><m:ItemShape>{shape}</m:ItemShape><m:SyncFolderId>{folder}</m:SyncFolderId>{state}<m:MaxChangesReturned>{max}</m:MaxChangesReturned><m:SyncScope>NormalItems</m:SyncScope></m:SyncFolderItems>",
        folder = folder.xml(),
        state = sync_state(state),
        max = max.clamp(1, 512)
    )
}

/// `FindItem`: the newest items of a folder (by when they arrived), a page
/// at a time, ids with read, flag and categories. With `since`, only items
/// that arrived then or later.
#[must_use]
pub fn find_newest(folder: &FolderRef<'_>, offset: u32, max: u32, since: Option<&str>) -> String {
    let restriction = since
        .map(|since| {
            format!(
                r#"<m:Restriction><t:IsGreaterThanOrEqualTo><t:FieldURI FieldURI="item:DateTimeReceived"/><t:FieldURIOrConstant><t:Constant Value="{}"/></t:FieldURIOrConstant></t:IsGreaterThanOrEqualTo></m:Restriction>"#,
                escape(since)
            )
        })
        .unwrap_or_default();
    format!(
        concat!(
            r#"<m:FindItem Traversal="Shallow"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{props}</t:AdditionalProperties></m:ItemShape>"#,
            r#"<m:IndexedPageItemView MaxEntriesReturned="{max}" Offset="{offset}" BasePoint="Beginning"/>"#,
            "{restriction}",
            r#"<m:SortOrder><t:FieldOrder Order="Descending"><t:FieldURI FieldURI="item:DateTimeReceived"/></t:FieldOrder></m:SortOrder>"#,
            "<m:ParentFolderIds>{folder}</m:ParentFolderIds></m:FindItem>"
        ),
        props = MAIL_STATE_PROPS,
        max = max.clamp(1, 1000),
        offset = offset,
        restriction = restriction,
        folder = folder.xml()
    )
}

/// `FindItem`: the items in a folder with this Message-ID.
#[must_use]
pub fn find_by_message_id(folder: &FolderRef<'_>, message_id: &str) -> String {
    let bare = message_id.trim().trim_matches(['<', '>']);
    format!(
        concat!(
            r#"<m:FindItem Traversal="Shallow"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape>"#,
            r#"<m:IndexedPageItemView MaxEntriesReturned="20" Offset="0" BasePoint="Beginning"/>"#,
            r#"<m:Restriction><t:IsEqualTo><t:FieldURI FieldURI="message:InternetMessageId"/><t:FieldURIOrConstant><t:Constant Value="&lt;{id}&gt;"/></t:FieldURIOrConstant></t:IsEqualTo></m:Restriction>"#,
            "<m:ParentFolderIds>{folder}</m:ParentFolderIds></m:FindItem>"
        ),
        id = escape(bare),
        folder = folder.xml()
    )
}

/// `GetItem` with what a message's list entry is made from: its received
/// header block, or the properties to write one, and its state.
#[must_use]
pub fn get_headers(ids: &[String]) -> String {
    format!(
        concat!(
            "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>",
            r#"<t:FieldURI FieldURI="item:Subject"/><t:FieldURI FieldURI="message:From"/>"#,
            r#"<t:FieldURI FieldURI="message:ToRecipients"/><t:FieldURI FieldURI="message:CcRecipients"/>"#,
            r#"<t:FieldURI FieldURI="item:DateTimeSent"/><t:FieldURI FieldURI="item:DateTimeReceived"/>"#,
            r#"<t:FieldURI FieldURI="message:InternetMessageId"/><t:FieldURI FieldURI="item:InReplyTo"/>"#,
            r#"<t:FieldURI FieldURI="message:References"/><t:FieldURI FieldURI="item:HasAttachments"/>"#,
            r#"<t:FieldURI FieldURI="item:Importance"/><t:FieldURI FieldURI="item:Size"/>"#,
            "{state}{headers}",
            "</t:AdditionalProperties></m:ItemShape><m:ItemIds>{ids}</m:ItemIds></m:GetItem>"
        ),
        state = MAIL_STATE_PROPS,
        headers = props::HEADERS,
        ids = item_ids(ids)
    )
}

/// `GetItem` with each item's whole MIME content (an iCalendar for a
/// calendar item).
#[must_use]
pub fn get_mime(ids: &[String]) -> String {
    format!(
        "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:IncludeMimeContent>true</t:IncludeMimeContent></m:ItemShape><m:ItemIds>{}</m:ItemIds></m:GetItem>",
        item_ids(ids)
    )
}

/// A change to one field of several messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageChange<'a> {
    /// Read or unread.
    Read(bool),
    /// Flagged (follow up) or not.
    Flagged(bool),
    /// The whole list of categories.
    Categories(&'a [String]),
}

fn message_update(change: &MessageChange<'_>) -> String {
    match change {
        MessageChange::Read(read) => format!(
            r#"<t:SetItemField><t:FieldURI FieldURI="message:IsRead"/><t:Message><t:IsRead>{read}</t:IsRead></t:Message></t:SetItemField>"#
        ),
        MessageChange::Flagged(true) => format!(
            "<t:SetItemField>{flag}<t:Message><t:ExtendedProperty>{flag}<t:Value>2</t:Value></t:ExtendedProperty></t:Message></t:SetItemField>",
            flag = props::FLAG
        ),
        MessageChange::Flagged(false) => {
            format!("<t:DeleteItemField>{}</t:DeleteItemField>", props::FLAG)
        }
        MessageChange::Categories([]) => {
            r#"<t:DeleteItemField><t:FieldURI FieldURI="item:Categories"/></t:DeleteItemField>"#
                .to_owned()
        }
        MessageChange::Categories(names) => {
            let list = names.iter().fold(String::new(), |mut out, name| {
                let _ = write!(out, "<t:String>{}</t:String>", escape(name));
                out
            });
            format!(
                r#"<t:SetItemField><t:FieldURI FieldURI="item:Categories"/><t:Message><t:Categories>{list}</t:Categories></t:Message></t:SetItemField>"#
            )
        }
    }
}

/// `UpdateItem`: one change on several messages.
#[must_use]
pub fn update_messages(ids: &[String], change: &MessageChange<'_>) -> String {
    let update = message_update(change);
    let changes = ids.iter().fold(String::new(), |mut out, id| {
        let _ = write!(
            out,
            r#"<t:ItemChange><t:ItemId Id="{}"/><t:Updates>{update}</t:Updates></t:ItemChange>"#,
            escape(id)
        );
        out
    });
    format!(
        r#"<m:UpdateItem MessageDisposition="SaveOnly" ConflictResolution="AlwaysOverwrite"><m:ItemChanges>{changes}</m:ItemChanges></m:UpdateItem>"#
    )
}

/// `MoveItem` or `CopyItem`.
#[must_use]
pub fn move_or_copy(ids: &[String], to: &FolderRef<'_>, copy: bool) -> String {
    let op = if copy { "CopyItem" } else { "MoveItem" };
    format!(
        "<m:{op}><m:ToFolderId>{}</m:ToFolderId><m:ItemIds>{}</m:ItemIds></m:{op}>",
        to.xml(),
        item_ids(ids)
    )
}

/// `DeleteItem`: for good, or to Deleted Items.
#[must_use]
pub fn delete_items(ids: &[String], hard: bool) -> String {
    let kind = if hard {
        "HardDelete"
    } else {
        "MoveToDeletedItems"
    };
    format!(
        r#"<m:DeleteItem DeleteType="{kind}" SendMeetingCancellations="SendToNone" AffectedTaskOccurrences="AllOccurrences"><m:ItemIds>{}</m:ItemIds></m:DeleteItem>"#,
        item_ids(ids)
    )
}

fn mailboxes(addresses: &[String]) -> String {
    addresses.iter().fold(String::new(), |mut out, address| {
        let _ = write!(
            out,
            "<t:Mailbox><t:EmailAddress>{}</t:EmailAddress></t:Mailbox>",
            escape(address)
        );
        out
    })
}

/// `CreateItem` sending a whole MIME message (signed, encrypted and HTML
/// mail go as they are), with a copy kept in Sent Items. `bcc` are the
/// blind copies, which the MIME text doesn't name.
#[must_use]
pub fn send_mime(mime_base64: &str, bcc: &[String]) -> String {
    let bcc = if bcc.is_empty() {
        String::new()
    } else {
        format!("<t:BccRecipients>{}</t:BccRecipients>", mailboxes(bcc))
    };
    format!(
        concat!(
            r#"<m:CreateItem MessageDisposition="SendAndSaveCopy">"#,
            r#"<m:SavedItemFolderId><t:DistinguishedFolderId Id="sentitems"/></m:SavedItemFolderId>"#,
            r#"<m:Items><t:Message><t:MimeContent CharacterSet="UTF-8">{mime}</t:MimeContent>{bcc}</t:Message></m:Items></m:CreateItem>"#
        ),
        mime = mime_base64,
        bcc = bcc
    )
}

/// `CreateItem` saving a MIME message in a folder: a draft (marked
/// unsent, so Outlook opens it for editing) or an imported message.
#[must_use]
pub fn save_mime(mime_base64: &str, folder: &FolderRef<'_>, draft: bool) -> String {
    let flags = if draft {
        format!(
            "<t:ExtendedProperty>{}<t:Value>8</t:Value></t:ExtendedProperty>",
            props::MESSAGE_FLAGS
        )
    } else {
        format!(
            "<t:ExtendedProperty>{}<t:Value>1</t:Value></t:ExtendedProperty>",
            props::MESSAGE_FLAGS
        )
    };
    format!(
        concat!(
            r#"<m:CreateItem MessageDisposition="SaveOnly"><m:SavedItemFolderId>{folder}</m:SavedItemFolderId>"#,
            r#"<m:Items><t:Message><t:MimeContent CharacterSet="UTF-8">{mime}</t:MimeContent>{flags}</t:Message></m:Items></m:CreateItem>"#
        ),
        folder = folder.xml(),
        mime = mime_base64,
        flags = flags
    )
}

/// `CreateFolder`: a mail folder.
#[must_use]
pub fn create_folder(parent: &FolderRef<'_>, name: &str) -> String {
    format!(
        "<m:CreateFolder><m:ParentFolderId>{}</m:ParentFolderId><m:Folders><t:Folder><t:FolderClass>IPF.Note</t:FolderClass><t:DisplayName>{}</t:DisplayName></t:Folder></m:Folders></m:CreateFolder>",
        parent.xml(),
        escape(name)
    )
}

/// `UpdateFolder`: a new name.
#[must_use]
pub fn rename_folder(id: &str, name: &str) -> String {
    format!(
        r#"<m:UpdateFolder><m:FolderChanges><t:FolderChange><t:FolderId Id="{}"/><t:Updates><t:SetFolderField><t:FieldURI FieldURI="folder:DisplayName"/><t:Folder><t:DisplayName>{}</t:DisplayName></t:Folder></t:SetFolderField></t:Updates></t:FolderChange></m:FolderChanges></m:UpdateFolder>"#,
        escape(id),
        escape(name)
    )
}

/// `DeleteFolder`: to Deleted Items, with what is in it.
#[must_use]
pub fn delete_folder(id: &str) -> String {
    format!(
        r#"<m:DeleteFolder DeleteType="MoveToDeletedItems"><m:FolderIds><t:FolderId Id="{}"/></m:FolderIds></m:DeleteFolder>"#,
        escape(id)
    )
}

/// `GetUserOofSettings`.
#[must_use]
pub fn get_oof(address: &str) -> String {
    format!(
        "<m:GetUserOofSettingsRequest><t:Mailbox><t:Address>{}</t:Address></t:Mailbox></m:GetUserOofSettingsRequest>",
        escape(address)
    )
}

/// Automatic replies as Exchange keeps them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Oof {
    /// `Disabled`, `Enabled` or `Scheduled`.
    pub state: String,
    /// `None`, `Known` or `All`.
    pub external_audience: String,
    /// When scheduled: start and end, `YYYY-MM-DDTHH:MM:SSZ`.
    pub duration: Option<(String, String)>,
    /// The reply to people in the organisation (HTML).
    pub internal: String,
    /// The reply to people outside (HTML).
    pub external: String,
}

/// `SetUserOofSettings`.
#[must_use]
pub fn set_oof(address: &str, oof: &Oof) -> String {
    let duration = oof
        .duration
        .as_ref()
        .map(|(start, end)| {
            format!(
                "<t:Duration><t:StartTime>{}</t:StartTime><t:EndTime>{}</t:EndTime></t:Duration>",
                escape(start),
                escape(end)
            )
        })
        .unwrap_or_default();
    format!(
        concat!(
            "<m:SetUserOofSettingsRequest><t:Mailbox><t:Address>{address}</t:Address></t:Mailbox>",
            "<t:UserOofSettings><t:OofState>{state}</t:OofState><t:ExternalAudience>{audience}</t:ExternalAudience>",
            "{duration}<t:InternalReply><t:Message>{internal}</t:Message></t:InternalReply>",
            "<t:ExternalReply><t:Message>{external}</t:Message></t:ExternalReply></t:UserOofSettings>",
            "</m:SetUserOofSettingsRequest>"
        ),
        address = escape(address),
        state = escape(&oof.state),
        audience = escape(&oof.external_audience),
        duration = duration,
        internal = escape(&oof.internal),
        external = escape(&oof.external)
    )
}

/// `GetUserAvailability`: when these people are busy between two UTC
/// times (`YYYY-MM-DDTHH:MM:SS`, no zone: the request's zone is UTC).
#[must_use]
pub fn get_availability(emails: &[String], start: &str, end: &str) -> String {
    let zone = concat!(
        "<t:TimeZone><t:Bias>0</t:Bias>",
        "<t:StandardTime><t:Bias>0</t:Bias><t:Time>00:00:00</t:Time><t:DayOrder>1</t:DayOrder><t:Month>1</t:Month><t:DayOfWeek>Sunday</t:DayOfWeek></t:StandardTime>",
        "<t:DaylightTime><t:Bias>0</t:Bias><t:Time>00:00:00</t:Time><t:DayOrder>1</t:DayOrder><t:Month>1</t:Month><t:DayOfWeek>Sunday</t:DayOfWeek></t:DaylightTime>",
        "</t:TimeZone>"
    );
    let people = emails.iter().fold(String::new(), |mut out, email| {
        let _ = write!(
            out,
            "<t:MailboxData><t:Email><t:Address>{}</t:Address></t:Email><t:AttendeeType>Required</t:AttendeeType><t:ExcludeConflicts>false</t:ExcludeConflicts></t:MailboxData>",
            escape(email)
        );
        out
    });
    format!(
        concat!(
            "<m:GetUserAvailabilityRequest>{zone}<m:MailboxDataArray>{people}</m:MailboxDataArray>",
            "<t:FreeBusyViewOptions><t:TimeWindow><t:StartTime>{start}</t:StartTime><t:EndTime>{end}</t:EndTime></t:TimeWindow>",
            "<t:MergedFreeBusyIntervalInMinutes>15</t:MergedFreeBusyIntervalInMinutes><t:RequestedView>FreeBusy</t:RequestedView></t:FreeBusyViewOptions>",
            "</m:GetUserAvailabilityRequest>"
        ),
        zone = zone,
        people = people,
        start = escape(start),
        end = escape(end)
    )
}

/// An event's fields for `CreateItem`/`UpdateItem` (times as EWS
/// `DateTime`s: UTC with `Z`, or for all-day events local midnight in
/// `zone`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventFields {
    /// Subject.
    pub subject: String,
    /// Notes, as text.
    pub body: String,
    /// Private.
    pub private: bool,
    /// Minutes before the start; `None`: no reminder.
    pub reminder: Option<u32>,
    /// Start.
    pub start: String,
    /// End.
    pub end: String,
    /// All day.
    pub all_day: bool,
    /// `Free`, `Tentative`, `Busy` or `OOF`.
    pub show_as: String,
    /// Where.
    pub location: String,
    /// People invited.
    pub attendees: Vec<String>,
    /// The `<t:Recurrence>` element, or empty for a single event.
    pub recurrence: String,
    /// The Windows time zone of an all-day event (`StartTimeZone`).
    pub zone: Option<String>,
}

fn zone_xml(element: &str, zone: Option<&str>) -> String {
    zone.map(|z| format!(r#"<t:{element} Id="{}"/>"#, escape(z)))
        .unwrap_or_default()
}

fn attendees(addresses: &[String]) -> String {
    addresses.iter().fold(String::new(), |mut out, address| {
        let _ = write!(
            out,
            "<t:Attendee><t:Mailbox><t:EmailAddress>{}</t:EmailAddress></t:Mailbox></t:Attendee>",
            escape(address)
        );
        out
    })
}

fn sending(notify: bool) -> &'static str {
    if notify {
        "SendToAllAndSaveCopy"
    } else {
        "SendToNone"
    }
}

/// `CreateItem`: a calendar item, invitations sent when `notify`.
#[must_use]
pub fn create_event(calendar: &FolderRef<'_>, fields: &EventFields, notify: bool) -> String {
    let sensitivity = if fields.private { "Private" } else { "Normal" };
    let reminder = match fields.reminder {
        Some(minutes) => format!(
            "<t:ReminderIsSet>true</t:ReminderIsSet><t:ReminderMinutesBeforeStart>{minutes}</t:ReminderMinutesBeforeStart>"
        ),
        None => "<t:ReminderIsSet>false</t:ReminderIsSet>".to_owned(),
    };
    let people = if fields.attendees.is_empty() {
        String::new()
    } else {
        format!(
            "<t:RequiredAttendees>{}</t:RequiredAttendees>",
            attendees(&fields.attendees)
        )
    };
    format!(
        concat!(
            r#"<m:CreateItem SendMeetingInvitations="{send}"><m:SavedItemFolderId>{calendar}</m:SavedItemFolderId><m:Items><t:CalendarItem>"#,
            "<t:Subject>{subject}</t:Subject><t:Sensitivity>{sensitivity}</t:Sensitivity>",
            r#"<t:Body BodyType="Text">{body}</t:Body>{reminder}"#,
            "<t:Start>{start}</t:Start><t:End>{end}</t:End><t:IsAllDayEvent>{all_day}</t:IsAllDayEvent>",
            "<t:LegacyFreeBusyStatus>{show_as}</t:LegacyFreeBusyStatus><t:Location>{location}</t:Location>",
            "{people}{recurrence}{start_zone}{end_zone}",
            "</t:CalendarItem></m:Items></m:CreateItem>"
        ),
        send = sending(notify),
        calendar = calendar.xml(),
        subject = escape(&fields.subject),
        sensitivity = sensitivity,
        body = escape(&fields.body),
        reminder = reminder,
        start = escape(&fields.start),
        end = escape(&fields.end),
        all_day = fields.all_day,
        show_as = escape(&fields.show_as),
        location = escape(&fields.location),
        people = people,
        recurrence = fields.recurrence,
        start_zone = zone_xml("StartTimeZone", fields.zone.as_deref()),
        end_zone = zone_xml("EndTimeZone", fields.zone.as_deref()),
    )
}

fn set_field(uri: &str, element: &str) -> String {
    format!(
        r#"<t:SetItemField><t:FieldURI FieldURI="{uri}"/><t:CalendarItem>{element}</t:CalendarItem></t:SetItemField>"#
    )
}

/// `UpdateItem`: every field of a calendar item (the repeat rule only when
/// `recurrence` is `Some`: empty removes it). Updates and cancellations go
/// to the people invited when `notify`.
#[must_use]
pub fn update_event(
    id: &str,
    fields: &EventFields,
    recurrence: Option<&str>,
    notify: bool,
) -> String {
    let sensitivity = if fields.private { "Private" } else { "Normal" };
    let mut updates = vec![
        set_field(
            "item:Subject",
            &format!("<t:Subject>{}</t:Subject>", escape(&fields.subject)),
        ),
        set_field(
            "item:Sensitivity",
            &format!("<t:Sensitivity>{sensitivity}</t:Sensitivity>"),
        ),
        set_field(
            "item:Body",
            &format!(
                r#"<t:Body BodyType="Text">{}</t:Body>"#,
                escape(&fields.body)
            ),
        ),
    ];
    match fields.reminder {
        Some(minutes) => {
            updates.push(set_field(
                "item:ReminderIsSet",
                "<t:ReminderIsSet>true</t:ReminderIsSet>",
            ));
            updates.push(set_field(
                "item:ReminderMinutesBeforeStart",
                &format!("<t:ReminderMinutesBeforeStart>{minutes}</t:ReminderMinutesBeforeStart>"),
            ));
        }
        None => updates.push(set_field(
            "item:ReminderIsSet",
            "<t:ReminderIsSet>false</t:ReminderIsSet>",
        )),
    }
    if let Some(zone) = fields.zone.as_deref() {
        updates.push(set_field(
            "calendar:StartTimeZone",
            &zone_xml("StartTimeZone", Some(zone)),
        ));
        updates.push(set_field(
            "calendar:EndTimeZone",
            &zone_xml("EndTimeZone", Some(zone)),
        ));
    }
    updates.push(set_field(
        "calendar:Start",
        &format!("<t:Start>{}</t:Start>", escape(&fields.start)),
    ));
    updates.push(set_field(
        "calendar:End",
        &format!("<t:End>{}</t:End>", escape(&fields.end)),
    ));
    updates.push(set_field(
        "calendar:IsAllDayEvent",
        &format!("<t:IsAllDayEvent>{}</t:IsAllDayEvent>", fields.all_day),
    ));
    updates.push(set_field(
        "calendar:LegacyFreeBusyStatus",
        &format!(
            "<t:LegacyFreeBusyStatus>{}</t:LegacyFreeBusyStatus>",
            escape(&fields.show_as)
        ),
    ));
    updates.push(set_field(
        "calendar:Location",
        &format!("<t:Location>{}</t:Location>", escape(&fields.location)),
    ));
    if fields.attendees.is_empty() {
        updates.push(r#"<t:DeleteItemField><t:FieldURI FieldURI="calendar:RequiredAttendees"/></t:DeleteItemField>"#.to_owned());
    } else {
        updates.push(set_field(
            "calendar:RequiredAttendees",
            &format!(
                "<t:RequiredAttendees>{}</t:RequiredAttendees>",
                attendees(&fields.attendees)
            ),
        ));
    }
    match recurrence {
        Some("") => updates.push(r#"<t:DeleteItemField><t:FieldURI FieldURI="calendar:Recurrence"/></t:DeleteItemField>"#.to_owned()),
        Some(rule) => updates.push(set_field("calendar:Recurrence", rule)),
        None => {}
    }
    format!(
        concat!(
            r#"<m:UpdateItem ConflictResolution="AlwaysOverwrite" SendMeetingInvitationsOrCancellations="{send}">"#,
            r#"<m:ItemChanges><t:ItemChange><t:ItemId Id="{id}"/><t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>"#
        ),
        send = if notify {
            "SendToChangedAndSaveCopy"
        } else {
            "SendToNone"
        },
        id = escape(id),
        updates = updates.concat()
    )
}

/// `DeleteItem`: a calendar item, cancellations sent when `notify`.
#[must_use]
pub fn delete_event(id: &str, notify: bool) -> String {
    format!(
        r#"<m:DeleteItem DeleteType="MoveToDeletedItems" SendMeetingCancellations="{}"><m:ItemIds><t:ItemId Id="{}"/></m:ItemIds></m:DeleteItem>"#,
        sending(notify),
        escape(id)
    )
}

/// `GetItem` with the fields of contacts.
#[must_use]
pub fn get_contacts(ids: &[String]) -> String {
    format!(
        concat!(
            "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:BodyType>Text</t:BodyType><t:AdditionalProperties>",
            r#"<t:FieldURI FieldURI="item:ItemClass"/><t:FieldURI FieldURI="item:Body"/><t:FieldURI FieldURI="item:Categories"/>"#,
            r#"<t:FieldURI FieldURI="contacts:DisplayName"/><t:FieldURI FieldURI="contacts:GivenName"/>"#,
            r#"<t:FieldURI FieldURI="contacts:Surname"/><t:FieldURI FieldURI="contacts:CompanyName"/>"#,
            r#"<t:FieldURI FieldURI="contacts:EmailAddresses"/><t:FieldURI FieldURI="contacts:PhoneNumbers"/>"#,
            r#"<t:FieldURI FieldURI="contacts:JobTitle"/>"#,
            "</t:AdditionalProperties></m:ItemShape><m:ItemIds>{ids}</m:ItemIds></m:GetItem>"
        ),
        ids = item_ids(ids)
    )
}

/// A contact's simple fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactFields {
    /// Display name.
    pub name: String,
    /// Up to three addresses.
    pub emails: Vec<String>,
    /// Up to three phone numbers (mobile, business, home).
    pub phones: Vec<String>,
    /// Company.
    pub company: String,
    /// Job title.
    pub title: String,
    /// Notes.
    pub notes: String,
}

/// Exchange's slots for addresses and phone numbers, in the order used.
pub const EMAIL_KEYS: [&str; 3] = ["EmailAddress1", "EmailAddress2", "EmailAddress3"];
/// The phone slots used.
pub const PHONE_KEYS: [&str; 3] = ["MobilePhone", "BusinessPhone", "HomePhone"];

fn entries(values: &[String], keys: &[&str]) -> String {
    values
        .iter()
        .zip(keys)
        .fold(String::new(), |mut out, (value, key)| {
            let _ = write!(out, r#"<t:Entry Key="{key}">{}</t:Entry>"#, escape(value));
            out
        })
}

/// `CreateItem`: a contact in a contacts folder.
#[must_use]
pub fn create_contact(folder: &FolderRef<'_>, contact: &ContactFields) -> String {
    let (given, surname) = split_name(&contact.name);
    let emails = entries(&contact.emails, &EMAIL_KEYS);
    let phones = entries(&contact.phones, &PHONE_KEYS);
    format!(
        concat!(
            "<m:CreateItem><m:SavedItemFolderId>{folder}</m:SavedItemFolderId><m:Items><t:Contact>",
            r#"<t:Body BodyType="Text">{notes}</t:Body>"#,
            "<t:FileAs>{name}</t:FileAs><t:DisplayName>{name}</t:DisplayName><t:GivenName>{given}</t:GivenName>",
            "<t:CompanyName>{company}</t:CompanyName>",
            "{emails}{phones}",
            "<t:JobTitle>{title}</t:JobTitle><t:Surname>{surname}</t:Surname>",
            "</t:Contact></m:Items></m:CreateItem>"
        ),
        folder = folder.xml(),
        notes = escape(&contact.notes),
        name = escape(&contact.name),
        given = escape(&given),
        company = escape(&contact.company),
        emails = if emails.is_empty() {
            String::new()
        } else {
            format!("<t:EmailAddresses>{emails}</t:EmailAddresses>")
        },
        phones = if phones.is_empty() {
            String::new()
        } else {
            format!("<t:PhoneNumbers>{phones}</t:PhoneNumbers>")
        },
        title = escape(&contact.title),
        surname = escape(&surname),
    )
}

/// First name and surname from a display name (the last word is the
/// surname).
#[must_use]
pub fn split_name(name: &str) -> (String, String) {
    match name.trim().rsplit_once(' ') {
        Some((given, surname)) => (given.trim().to_owned(), surname.trim().to_owned()),
        None => (name.trim().to_owned(), String::new()),
    }
}

fn contact_field(uri: &str, element: &str, value: &str) -> String {
    if value.is_empty() {
        format!(r#"<t:DeleteItemField><t:FieldURI FieldURI="{uri}"/></t:DeleteItemField>"#)
    } else {
        format!(
            r#"<t:SetItemField><t:FieldURI FieldURI="{uri}"/><t:Contact><t:{element}>{}</t:{element}></t:Contact></t:SetItemField>"#,
            escape(value)
        )
    }
}

fn indexed_field(uri: &str, index: &str, list: &str, value: Option<&String>) -> String {
    match value.filter(|v| !v.is_empty()) {
        Some(value) => format!(
            r#"<t:SetItemField><t:IndexedFieldURI FieldURI="{uri}" FieldIndex="{index}"/><t:Contact><t:{list}><t:Entry Key="{index}">{}</t:Entry></t:{list}></t:Contact></t:SetItemField>"#,
            escape(value)
        ),
        None => format!(
            r#"<t:DeleteItemField><t:IndexedFieldURI FieldURI="{uri}" FieldIndex="{index}"/></t:DeleteItemField>"#
        ),
    }
}

/// `UpdateItem`: a contact's simple fields, all of them.
#[must_use]
pub fn update_contact(id: &str, contact: &ContactFields) -> String {
    let (given, surname) = split_name(&contact.name);
    let mut updates = vec![
        format!(
            r#"<t:SetItemField><t:FieldURI FieldURI="item:Body"/><t:Contact><t:Body BodyType="Text">{}</t:Body></t:Contact></t:SetItemField>"#,
            escape(&contact.notes)
        ),
        contact_field("contacts:FileAs", "FileAs", &contact.name),
        contact_field("contacts:DisplayName", "DisplayName", &contact.name),
        contact_field("contacts:GivenName", "GivenName", &given),
        contact_field("contacts:CompanyName", "CompanyName", &contact.company),
    ];
    for (index, key) in EMAIL_KEYS.iter().enumerate() {
        updates.push(indexed_field(
            "contacts:EmailAddress",
            key,
            "EmailAddresses",
            contact.emails.get(index),
        ));
    }
    for (index, key) in PHONE_KEYS.iter().enumerate() {
        updates.push(indexed_field(
            "contacts:PhoneNumber",
            key,
            "PhoneNumbers",
            contact.phones.get(index),
        ));
    }
    updates.push(contact_field(
        "contacts:JobTitle",
        "JobTitle",
        &contact.title,
    ));
    updates.push(contact_field("contacts:Surname", "Surname", &surname));
    format!(
        r#"<m:UpdateItem ConflictResolution="AlwaysOverwrite"><m:ItemChanges><t:ItemChange><t:ItemId Id="{}"/><t:Updates>{}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>"#,
        escape(id),
        updates.concat()
    )
}
