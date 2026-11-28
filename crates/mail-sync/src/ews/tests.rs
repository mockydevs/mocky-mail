//! EWS requests and answers, against answers recorded from Exchange Server
//! 2016 and 2019 (ids shortened, personal data replaced), Autodiscover's
//! rules, and the conversions.

use mail_core::ical::{EventInput, Freq, Repeat, RepeatRule, ShowAs};
use mail_core::vacation::{ExternalAudience, Vacation};

use super::autodiscover::{self, Answer, Hops};
use super::convert;
use super::parse::{self, ItemChange};
use super::soap::{self, FolderRef, MessageChange};
use super::xml::Element;
use crate::SyncError;

fn envelope(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Header><h:ServerVersionInfo MajorVersion="15" MinorVersion="1" MajorBuildNumber="2507" MinorBuildNumber="6" Version="V2017_07_11" xmlns:h="http://schemas.microsoft.com/exchange/services/2006/types"/></s:Header><s:Body xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">{body}</s:Body></s:Envelope>"#
    )
}

const HIERARCHY: &str = r#"<m:SyncFolderHierarchyResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages" xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types"><m:ResponseMessages><m:SyncFolderHierarchyResponseMessage ResponseClass="Success"><m:ResponseCode>NoError</m:ResponseCode><m:SyncState>H4sIAAAAAAAEAO29B2</m:SyncState><m:IncludesLastFolderInRange>true</m:IncludesLastFolderInRange><m:Changes>
<t:Create><t:Folder><t:FolderId Id="AAMkInbox=" ChangeKey="AQAAABYA"/><t:ParentFolderId Id="AAMkRoot=" ChangeKey="AQAAAA=="/><t:FolderClass>IPF.Note</t:FolderClass><t:DisplayName>Inbox</t:DisplayName><t:TotalCount>12</t:TotalCount><t:UnreadCount>3</t:UnreadCount></t:Folder></t:Create>
<t:Create><t:Folder><t:FolderId Id="AAMkClients=" ChangeKey="AQAAABYB"/><t:ParentFolderId Id="AAMkInbox=" ChangeKey="AQAAAA=="/><t:FolderClass>IPF.Note</t:FolderClass><t:DisplayName>Clients &amp; Partners</t:DisplayName><t:TotalCount>4</t:TotalCount><t:UnreadCount>0</t:UnreadCount></t:Folder></t:Create>
<t:Create><t:CalendarFolder><t:FolderId Id="AAMkCalendar=" ChangeKey="AgAAABYA"/><t:ParentFolderId Id="AAMkRoot=" ChangeKey="AQAAAA=="/><t:FolderClass>IPF.Appointment</t:FolderClass><t:DisplayName>Calendar</t:DisplayName><t:TotalCount>40</t:TotalCount></t:CalendarFolder></t:Create>
<t:Create><t:ContactsFolder><t:FolderId Id="AAMkContacts=" ChangeKey="AwAAABYA"/><t:ParentFolderId Id="AAMkRoot=" ChangeKey="AQAAAA=="/><t:FolderClass>IPF.Contact</t:FolderClass><t:DisplayName>Contacts</t:DisplayName><t:TotalCount>7</t:TotalCount></t:ContactsFolder></t:Create>
<t:Update><t:Folder><t:FolderId Id="AAMkArchive=" ChangeKey="AQAAABYC"/><t:ParentFolderId Id="AAMkRoot=" ChangeKey="AQAAAA=="/><t:DisplayName>Archive</t:DisplayName><t:TotalCount>900</t:TotalCount><t:UnreadCount>0</t:UnreadCount></t:Folder></t:Update>
<t:Delete><t:FolderId Id="AAMkOld=" ChangeKey="AQAAAA=="/></t:Delete>
</m:Changes></m:SyncFolderHierarchyResponseMessage></m:ResponseMessages></m:SyncFolderHierarchyResponse>"#;

#[test]
fn the_folder_hierarchy_is_read() {
    let root = Element::parse(&envelope(HIERARCHY)).unwrap();
    let changes = parse::hierarchy(&root).unwrap();
    assert_eq!(changes.sync_state, "H4sIAAAAAAAEAO29B2");
    assert!(changes.done);
    assert_eq!(changes.changed.len(), 5);
    let clients = &changes.changed[1];
    assert_eq!(clients.name, "Clients & Partners");
    assert_eq!(clients.parent_id.as_deref(), Some("AAMkInbox="));
    assert!(clients.is_mail());
    assert_eq!(changes.changed[0].unread, Some(3));
    assert!(changes.changed[2].is_calendar() && !changes.changed[2].is_mail());
    assert!(changes.changed[3].is_contacts());
    // No class: mail.
    assert!(changes.changed[4].is_mail());
    assert_eq!(changes.deleted, vec!["AAMkOld=".to_owned()]);
}

const ITEMS: &str = r#"<m:SyncFolderItemsResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages" xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types"><m:ResponseMessages><m:SyncFolderItemsResponseMessage ResponseClass="Success"><m:ResponseCode>NoError</m:ResponseCode><m:SyncState>H4sIAAAAAAAEAGNgYGBk</m:SyncState><m:IncludesLastItemInRange>false</m:IncludesLastItemInRange><m:Changes>
<t:Create><t:Message><t:ItemId Id="AAMkM1=" ChangeKey="CQAAABYA"/><t:ItemClass>IPM.Note</t:ItemClass><t:Categories><t:String>Red team</t:String></t:Categories><t:ExtendedProperty><t:ExtendedFieldURI PropertyTag="0x1090" PropertyType="Integer"/><t:Value>2</t:Value></t:ExtendedProperty><t:IsRead>false</t:IsRead></t:Message></t:Create>
<t:Create><t:MeetingRequest><t:ItemId Id="AAMkM2=" ChangeKey="CwAAABYA"/><t:ItemClass>IPM.Schedule.Meeting.Request</t:ItemClass><t:IsRead>true</t:IsRead></t:MeetingRequest></t:Create>
<t:Update><t:Message><t:ItemId Id="AAMkM3=" ChangeKey="CQAAABYB"/><t:IsRead>true</t:IsRead></t:Message></t:Update>
<t:Delete><t:ItemId Id="AAMkM4=" ChangeKey="CQAAAA=="/></t:Delete>
<t:ReadFlagChange><t:ItemId Id="AAMkM5=" ChangeKey="CQAAAA=="/><t:IsRead>false</t:IsRead></t:ReadFlagChange>
</m:Changes></m:SyncFolderItemsResponseMessage></m:ResponseMessages></m:SyncFolderItemsResponse>"#;

#[test]
fn folder_changes_are_read_in_order() {
    let root = Element::parse(&envelope(ITEMS)).unwrap();
    let changes = parse::item_changes(&root).unwrap();
    assert_eq!(changes.sync_state, "H4sIAAAAAAAEAGNgYGBk");
    assert!(!changes.done);
    assert_eq!(changes.changes.len(), 5);
    let ItemChange::Created(first) = &changes.changes[0] else {
        panic!("{:?}", changes.changes[0]);
    };
    assert_eq!(first.id, "AAMkM1=");
    assert_eq!(first.read, Some(false));
    assert_eq!(first.flagged, Some(true));
    assert_eq!(first.categories, Some(vec!["Red team".to_owned()]));
    let ItemChange::Created(second) = &changes.changes[1] else {
        panic!();
    };
    assert_eq!(second.flagged, Some(false));
    assert_eq!(
        second.class.as_deref(),
        Some("IPM.Schedule.Meeting.Request")
    );
    assert!(matches!(&changes.changes[2], ItemChange::Updated(s) if s.read == Some(true)));
    assert_eq!(changes.changes[3], ItemChange::Deleted("AAMkM4=".into()));
    assert_eq!(
        changes.changes[4],
        ItemChange::ReadChanged {
            id: "AAMkM5=".into(),
            read: false
        }
    );
}

#[test]
fn a_stale_sync_state_is_told_apart() {
    let stale = r#"<m:SyncFolderItemsResponse xmlns:m="m" xmlns:t="t"><m:ResponseMessages><m:SyncFolderItemsResponseMessage ResponseClass="Error"><m:MessageText>Synchronization state data is corrupt or otherwise invalid.</m:MessageText><m:ResponseCode>ErrorInvalidSyncStateData</m:ResponseCode><m:DescriptiveLinkKey>0</m:DescriptiveLinkKey></m:SyncFolderItemsResponseMessage></m:ResponseMessages></m:SyncFolderItemsResponse>"#;
    let root = Element::parse(&envelope(stale)).unwrap();
    assert!(matches!(
        parse::item_changes(&root),
        Err(SyncError::Refused(code)) if code == parse::INVALID_SYNC_STATE
    ));
}

const HEADERS: &str = r#"<m:GetItemResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages" xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types"><m:ResponseMessages>
<m:GetItemResponseMessage ResponseClass="Success"><m:ResponseCode>NoError</m:ResponseCode><m:Items><t:Message><t:ItemId Id="AAMkM1=" ChangeKey="CQAAABYA"/><t:ItemClass>IPM.Note</t:ItemClass><t:Subject>Site visit, Kilimani</t:Subject><t:DateTimeReceived>2026-11-18T06:01:00Z</t:DateTimeReceived><t:Size>5120</t:Size><t:Importance>High</t:Importance><t:InReplyTo>&lt;a1@client.example&gt;</t:InReplyTo><t:HasAttachments>false</t:HasAttachments><t:ExtendedProperty><t:ExtendedFieldURI PropertyTag="0x7d" PropertyType="String"/><t:Value>Received: from mail.client.example
From: Peter Mwangi &lt;peter@client.example&gt;
To: Don &lt;don@mocky.example&gt;
Subject: Site visit, Kilimani
Date: Wed, 18 Nov 2026 09:00:00 +0300
Message-ID: &lt;b2@client.example&gt;
In-Reply-To: &lt;a1@client.example&gt;
List-Unsubscribe: &lt;mailto:leave@client.example&gt;
Content-Type: text/plain; charset="utf-8"
</t:Value></t:ExtendedProperty><t:DateTimeSent>2026-11-18T06:00:00Z</t:DateTimeSent><t:IsRead>false</t:IsRead><t:From><t:Mailbox><t:Name>Peter Mwangi</t:Name><t:EmailAddress>peter@client.example</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType></t:Mailbox></t:From><t:InternetMessageId>&lt;b2@client.example&gt;</t:InternetMessageId></t:Message></m:Items></m:GetItemResponseMessage>
<m:GetItemResponseMessage ResponseClass="Success"><m:ResponseCode>NoError</m:ResponseCode><m:Items><t:Message><t:ItemId Id="AAMkS1=" ChangeKey="CQAAABYB"/><t:ItemClass>IPM.Note</t:ItemClass><t:Subject>Quote: "phase 2"</t:Subject><t:Size>2048</t:Size><t:Categories><t:String>Clients</t:String></t:Categories><t:Importance>Normal</t:Importance><t:DateTimeSent>2026-11-19T10:30:00Z</t:DateTimeSent><t:IsRead>true</t:IsRead><t:From><t:Mailbox><t:Name>Don</t:Name><t:EmailAddress>don@mocky.example</t:EmailAddress></t:Mailbox></t:From><t:ToRecipients><t:Mailbox><t:Name>Peter Mwangi</t:Name><t:EmailAddress>peter@client.example</t:EmailAddress></t:Mailbox><t:Mailbox><t:EmailAddress>ann@client.example</t:EmailAddress></t:Mailbox></t:ToRecipients><t:CcRecipients><t:Mailbox><t:Name>Office, Nairobi</t:Name><t:EmailAddress>office@mocky.example</t:EmailAddress></t:Mailbox></t:CcRecipients><t:InternetMessageId>&lt;s1@mocky.example&gt;</t:InternetMessageId><t:References>&lt;a1@client.example&gt; &lt;b2@client.example&gt;</t:References></t:Message></m:Items></m:GetItemResponseMessage>
<m:GetItemResponseMessage ResponseClass="Error"><m:MessageText>The specified object was not found in the store.</m:MessageText><m:ResponseCode>ErrorItemNotFound</m:ResponseCode><m:DescriptiveLinkKey>0</m:DescriptiveLinkKey><m:Items/></m:GetItemResponseMessage>
</m:ResponseMessages></m:GetItemResponse>"#;

#[test]
fn list_entries_come_from_the_received_headers_or_the_properties() {
    let root = Element::parse(&envelope(HEADERS)).unwrap();
    let entries = parse::headers(&root).unwrap();
    // The one deleted meanwhile is left out.
    assert_eq!(entries.len(), 2);
    let received = &entries[0];
    assert_eq!(received.state.read, Some(false));
    assert_eq!(received.size, Some(5120));
    let block = convert::header_block(received);
    assert!(block.starts_with("Received: from mail.client.example\r\n"));
    let summary = mail_core::message::parse_summary(block.as_bytes()).unwrap();
    assert_eq!(summary.subject, "Site visit, Kilimani");
    assert_eq!(summary.message_id.as_deref(), Some("b2@client.example"));
    assert_eq!(
        summary.from.as_ref().map(|m| m.address.as_str()),
        Some("peter@client.example")
    );
    assert!(summary.bulk, "List-Unsubscribe marks it bulk");

    // Sent from the mailbox: no header block, so one is written.
    let sent = &entries[1];
    assert!(sent.transport_headers.is_none());
    assert_eq!(sent.state.categories, Some(vec!["Clients".to_owned()]));
    let block = convert::header_block(sent);
    let summary = mail_core::message::parse_summary(block.as_bytes()).unwrap();
    assert_eq!(summary.subject, "Quote: \"phase 2\"");
    assert_eq!(summary.to.len(), 2);
    assert_eq!(
        summary.cc.first().and_then(|m| m.name.as_deref()),
        Some("Office, Nairobi")
    );
    assert_eq!(summary.references.len(), 2);
    assert_eq!(summary.date, Some(1_795_084_200));
}

#[test]
fn written_headers_cant_be_broken_by_what_they_carry() {
    let entry = parse::Headers {
        subject: "Hi\r\nBcc: evil@example.com".into(),
        from: Some(("A \"B\"\nC".into(), "a@example.com".into())),
        ..parse::Headers::default()
    };
    let block = convert::header_block(&entry);
    assert!(!block.contains("\r\nBcc:"), "{block}");
    let summary = mail_core::message::parse_summary(block.as_bytes()).unwrap();
    assert!(summary.subject.starts_with("Hi"));
    assert_eq!(
        summary.from.map(|m| m.address),
        Some("a@example.com".to_owned())
    );
}

const MIME: &str = r#"<m:GetItemResponse xmlns:m="m" xmlns:t="t"><m:ResponseMessages><m:GetItemResponseMessage ResponseClass="Success"><m:ResponseCode>NoError</m:ResponseCode><m:Items><t:Message><t:MimeContent CharacterSet="UTF-8">RnJvbTogUGV0ZXIgPHBldGVyQGNsaWVudC5leGFtcGxlPg0KU3ViamVjdDogSGkNCg0K
SGVsbG8gdGhlcmUNCg==</t:MimeContent><t:ItemId Id="AAMkM1=" ChangeKey="CQAAABYA"/></t:Message></m:Items></m:GetItemResponseMessage></m:ResponseMessages></m:GetItemResponse>"#;

#[test]
fn mime_content_is_decoded_whole() {
    let root = Element::parse(&envelope(MIME)).unwrap();
    let items = parse::mime(&root).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "AAMkM1=");
    assert_eq!(
        String::from_utf8(items[0].raw.clone()).unwrap(),
        "From: Peter <peter@client.example>\r\nSubject: Hi\r\n\r\nHello there\r\n"
    );
}

#[test]
fn faults_and_item_errors_become_plain_errors() {
    let fault = r#"<s:Fault><faultcode xmlns:a="http://schemas.microsoft.com/exchange/services/2006/types">a:ErrorSchemaValidation</faultcode><faultstring xml:lang="en-US">The request failed schema validation: The 'Id' attribute is invalid.</faultstring><detail><e:ResponseCode xmlns:e="http://schemas.microsoft.com/exchange/services/2006/errors">ErrorSchemaValidation</e:ResponseCode></detail></s:Fault>"#;
    let root = Element::parse(&envelope(fault)).unwrap();
    assert!(matches!(
        parse::responses(&root),
        Err(SyncError::Refused(_))
    ));
    let busy = r#"<s:Fault><faultcode>a:ErrorServerBusy</faultcode><faultstring>The server cannot service this request right now.</faultstring><detail><e:ResponseCode xmlns:e="e">ErrorServerBusy</e:ResponseCode></detail></s:Fault>"#;
    let root = Element::parse(&envelope(busy)).unwrap();
    let error = parse::responses(&root).unwrap_err();
    assert!(error.is_transient(), "{error}");

    // One message moved elsewhere meanwhile: the rest still count.
    let update = r#"<m:UpdateItemResponse xmlns:m="m"><m:ResponseMessages><m:UpdateItemResponseMessage ResponseClass="Success"><m:ResponseCode>NoError</m:ResponseCode></m:UpdateItemResponseMessage><m:UpdateItemResponseMessage ResponseClass="Error"><m:MessageText>The specified object was not found in the store.</m:MessageText><m:ResponseCode>ErrorItemNotFound</m:ResponseCode></m:UpdateItemResponseMessage></m:ResponseMessages></m:UpdateItemResponse>"#;
    let root = Element::parse(&envelope(update)).unwrap();
    assert!(parse::all_done(&root).is_ok());
    let denied = update.replace("ErrorItemNotFound", "ErrorAccessDenied");
    let root = Element::parse(&envelope(&denied)).unwrap();
    assert!(matches!(parse::all_done(&root), Err(SyncError::Refused(_))));
}

#[test]
fn found_items_and_made_items() {
    let found = r#"<m:FindItemResponse xmlns:m="m" xmlns:t="t"><m:ResponseMessages><m:FindItemResponseMessage ResponseClass="Success"><m:ResponseCode>NoError</m:ResponseCode><m:RootFolder IndexedPagingOffset="2" TotalItemsInView="1500" IncludesLastItemInRange="false"><t:Items><t:Message><t:ItemId Id="AAMkN1=" ChangeKey="a"/><t:IsRead>true</t:IsRead></t:Message><t:Message><t:ItemId Id="AAMkN2=" ChangeKey="b"/><t:IsRead>false</t:IsRead></t:Message></t:Items></m:RootFolder></m:FindItemResponseMessage></m:ResponseMessages></m:FindItemResponse>"#;
    let root = Element::parse(&envelope(found)).unwrap();
    let page = parse::found_items(&root).unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.total, 1500);
    assert!(!page.last);
    let made = r#"<m:CreateItemResponse xmlns:m="m" xmlns:t="t"><m:ResponseMessages><m:CreateItemResponseMessage ResponseClass="Success"><m:ResponseCode>NoError</m:ResponseCode><m:Items><t:Message><t:ItemId Id="AAMkD1=" ChangeKey="CQAAABYA"/></t:Message></m:Items></m:CreateItemResponseMessage></m:ResponseMessages></m:CreateItemResponse>"#;
    let root = Element::parse(&envelope(made)).unwrap();
    assert_eq!(
        parse::created_items(&root).unwrap(),
        vec![("AAMkD1=".to_owned(), Some("CQAAABYA".to_owned()))]
    );
}

const OOF: &str = r#"<GetUserOofSettingsResponse xmlns="http://schemas.microsoft.com/exchange/services/2006/messages"><ResponseMessage ResponseClass="Success"><ResponseCode>NoError</ResponseCode></ResponseMessage><OofSettings xmlns="http://schemas.microsoft.com/exchange/services/2006/types"><OofState>Scheduled</OofState><ExternalAudience>Known</ExternalAudience><Duration><StartTime>2026-12-19T21:00:00Z</StartTime><EndTime>2027-01-04T21:00:00Z</EndTime></Duration><InternalReply><Message>&lt;html&gt;&lt;body&gt;Back on 5 January.&lt;/body&gt;&lt;/html&gt;</Message></InternalReply><ExternalReply><Message>&lt;html&gt;&lt;body&gt;Our office is closed until 5 January.&lt;/body&gt;&lt;/html&gt;</Message></ExternalReply></OofSettings><AllowExternalOof>All</AllowExternalOof></GetUserOofSettingsResponse>"#;

#[test]
fn automatic_replies_map_both_ways() {
    let root = Element::parse(&envelope(OOF)).unwrap();
    let oof = parse::oof(&root).unwrap();
    assert_eq!(oof.state, "Scheduled");
    // Nairobi is three hours ahead.
    let offset = 3 * 3600;
    let vacation = convert::vacation(&oof, offset);
    assert!(vacation.enabled);
    assert_eq!(vacation.start.as_deref(), Some("2026-12-20"));
    assert_eq!(vacation.end.as_deref(), Some("2027-01-04"));
    assert_eq!(vacation.message, "Back on 5 January.");
    assert_eq!(
        vacation.external_message.as_deref(),
        Some("Our office is closed until 5 January.")
    );
    assert_eq!(vacation.external_audience, Some(ExternalAudience::Contacts));
    let back = convert::oof(&vacation, offset, 0);
    assert_eq!(back.state, "Scheduled");
    assert_eq!(back.external_audience, "Known");
    assert_eq!(
        back.duration,
        Some((
            "2026-12-19T21:00:00Z".to_owned(),
            "2027-01-04T21:00:00Z".to_owned()
        ))
    );
    let request = soap::set_oof("don@mocky.example", &back);
    assert!(Element::parse(&soap::envelope(&request)).is_ok());
    assert!(request.contains("<t:OofState>Scheduled</t:OofState>"));
    // Always on, no dates.
    let always = convert::oof(
        &Vacation {
            enabled: true,
            message: "Away".into(),
            ..Vacation::default()
        },
        offset,
        0,
    );
    assert_eq!(always.state, "Enabled");
    assert!(always.duration.is_none());
    let off = convert::oof(&Vacation::default(), offset, 0);
    assert_eq!(off.state, "Disabled");
}

const AVAILABILITY: &str = r#"<GetUserAvailabilityResponse xmlns="http://schemas.microsoft.com/exchange/services/2006/messages"><FreeBusyResponseArray>
<FreeBusyResponse><ResponseMessage ResponseClass="Success"><ResponseCode>NoError</ResponseCode></ResponseMessage><FreeBusyView><FreeBusyViewType xmlns="http://schemas.microsoft.com/exchange/services/2006/types">FreeBusy</FreeBusyViewType><CalendarEventArray xmlns="http://schemas.microsoft.com/exchange/services/2006/types"><CalendarEvent><StartTime>2026-12-01T06:00:00</StartTime><EndTime>2026-12-01T07:00:00</EndTime><BusyType>Busy</BusyType></CalendarEvent><CalendarEvent><StartTime>2026-12-01T09:00:00</StartTime><EndTime>2026-12-01T09:30:00</EndTime><BusyType>Tentative</BusyType></CalendarEvent><CalendarEvent><StartTime>2026-12-01T10:00:00</StartTime><EndTime>2026-12-01T11:00:00</EndTime><BusyType>Free</BusyType></CalendarEvent></CalendarEventArray></FreeBusyView></FreeBusyResponse>
<FreeBusyResponse><ResponseMessage ResponseClass="Error"><MessageText>Unable to resolve e-mail address to a valid SMTP address.</MessageText><ResponseCode>ErrorMailRecipientNotFound</ResponseCode></ResponseMessage><FreeBusyView><FreeBusyViewType xmlns="http://schemas.microsoft.com/exchange/services/2006/types">None</FreeBusyViewType></FreeBusyView></FreeBusyResponse>
</FreeBusyResponseArray></GetUserAvailabilityResponse>"#;

#[test]
fn free_busy_is_read_per_person() {
    let root = Element::parse(&envelope(AVAILABILITY)).unwrap();
    let answers = parse::availability(&root).unwrap();
    assert_eq!(answers.len(), 2);
    let busy = convert::busy_periods(answers[0].as_ref().unwrap());
    assert_eq!(busy.len(), 2);
    assert_eq!(busy[0].start, 1_796_104_800);
    assert_eq!(busy[0].show_as, ShowAs::Busy);
    assert_eq!(busy[1].show_as, ShowAs::Tentative);
    assert!(answers[1].is_none());
    let request = soap::get_availability(
        &["peter@client.example".into()],
        &convert::utc_plain(1_796_104_800),
        &convert::utc_plain(1_796_191_200),
    );
    assert!(request.contains("<t:StartTime>2026-12-01T06:00:00</t:StartTime>"));
    assert!(Element::parse(&soap::envelope(&request)).is_ok());
}

#[test]
fn contacts_are_read_with_their_slots() {
    let answer = r#"<m:GetItemResponse xmlns:m="m" xmlns:t="t"><m:ResponseMessages><m:GetItemResponseMessage ResponseClass="Success"><m:ResponseCode>NoError</m:ResponseCode><m:Items><t:Contact><t:ItemId Id="AAMkC1=" ChangeKey="EQAAABYA"/><t:ItemClass>IPM.Contact</t:ItemClass><t:Body BodyType="Text">Met at the site.</t:Body><t:Categories><t:String>Clients</t:String></t:Categories><t:DisplayName>Peter Mwangi</t:DisplayName><t:GivenName>Peter</t:GivenName><t:CompanyName>Client Ltd</t:CompanyName><t:EmailAddresses><t:Entry Key="EmailAddress2">p.mwangi@home.example</t:Entry><t:Entry Key="EmailAddress1">SMTP:peter@client.example</t:Entry></t:EmailAddresses><t:PhoneNumbers><t:Entry Key="BusinessPhone">+254 20 1234567</t:Entry><t:Entry Key="MobilePhone">+254 712 345678</t:Entry><t:Entry Key="HomePhone"/></t:PhoneNumbers><t:JobTitle>Director</t:JobTitle><t:Surname>Mwangi</t:Surname></t:Contact><t:DistributionList><t:ItemId Id="AAMkL1="/></t:DistributionList></m:Items></m:GetItemResponseMessage></m:ResponseMessages></m:GetItemResponse>"#;
    let root = Element::parse(&envelope(answer)).unwrap();
    let contacts = parse::contacts(&root).unwrap();
    assert_eq!(contacts.len(), 1);
    let card = convert::card(&contacts[0]);
    assert_eq!(card.uid, "AAMkC1=");
    assert_eq!(card.name, "Peter Mwangi");
    assert_eq!(
        card.emails,
        vec!["peter@client.example", "p.mwangi@home.example"]
    );
    assert_eq!(card.phones, vec!["+254 712 345678", "+254 20 1234567"]);
    assert_eq!(card.organization, "Client Ltd");
    assert_eq!(card.notes, "Met at the site.");
    let fields = convert::contact_fields(&card);
    let create = soap::create_contact(&FolderRef::Id("AAMkContacts="), &fields);
    assert!(Element::parse(&soap::envelope(&create)).is_ok());
    assert!(create.contains(r#"<t:Entry Key="EmailAddress1">peter@client.example</t:Entry>"#));
    assert!(create.contains("<t:GivenName>Peter</t:GivenName>"));
    let update = soap::update_contact("AAMkC1=", &fields);
    assert!(Element::parse(&soap::envelope(&update)).is_ok());
    // An empty third slot is removed, not left as it was.
    assert!(update.contains(r#"<t:DeleteItemField><t:IndexedFieldURI FieldURI="contacts:EmailAddress" FieldIndex="EmailAddress3"/></t:DeleteItemField>"#));
}

#[test]
fn requests_are_well_formed_and_escape_what_they_carry() {
    let hostile = r#"x"/><t:Evil a="&"#;
    for request in [
        soap::get_folders(&["inbox", "sentitems"]),
        soap::sync_folder_hierarchy(Some(hostile)),
        soap::sync_folder_items(&FolderRef::Id(hostile), None, 9999, true),
        soap::find_newest(&FolderRef::Id("f"), 0, 100, Some("2026-01-01T00:00:00Z")),
        soap::find_by_message_id(&FolderRef::Id("f"), "<a&b@example.com>"),
        soap::get_headers(&[hostile.to_owned()]),
        soap::get_mime(&["a".into(), "b".into()]),
        soap::update_messages(&["a".into()], &MessageChange::Read(true)),
        soap::update_messages(&["a".into()], &MessageChange::Flagged(true)),
        soap::update_messages(&["a".into()], &MessageChange::Flagged(false)),
        soap::update_messages(
            &["a".into()],
            &MessageChange::Categories(&[hostile.to_owned()]),
        ),
        soap::update_messages(&["a".into()], &MessageChange::Categories(&[])),
        soap::move_or_copy(&["a".into()], &FolderRef::Id("t"), false),
        soap::move_or_copy(
            &["a".into()],
            &FolderRef::Distinguished("deleteditems"),
            true,
        ),
        soap::delete_items(&["a".into()], true),
        soap::send_mime("QQ==", &["hidden@example.com".into()]),
        soap::save_mime("QQ==", &FolderRef::Id("drafts"), true),
        soap::create_folder(&FolderRef::Distinguished("msgfolderroot"), hostile),
        soap::rename_folder("f", hostile),
        soap::delete_folder("f"),
        soap::get_oof(hostile),
        soap::delete_event("e", true),
    ] {
        let whole = soap::envelope(&request);
        let root = Element::parse(&whole).unwrap_or_else(|e| panic!("{e}: {request}"));
        assert_eq!(root.name, "Envelope");
        assert!(!request.contains("<t:Evil"), "{request}");
    }
    let items = soap::sync_folder_items(&FolderRef::Id("f"), Some("state"), 9999, true);
    assert!(items.contains("<m:SyncState>state</m:SyncState>"));
    assert!(items.contains("<m:MaxChangesReturned>512</m:MaxChangesReturned>"));
    assert!(soap::envelope("").contains(r#"Version="Exchange2010_SP2""#));
    let sent = soap::send_mime("QQ==", &["hidden@example.com".into()]);
    assert!(sent.contains(r#"MessageDisposition="SendAndSaveCopy""#));
    assert!(sent.contains("<t:BccRecipients><t:Mailbox><t:EmailAddress>hidden@example.com</t:EmailAddress></t:Mailbox></t:BccRecipients>"));
    let found = soap::find_by_message_id(&FolderRef::Id("f"), "<a@b.c>");
    assert!(found.contains(r#"Value="&lt;a@b.c&gt;""#));
    let flagged = soap::update_messages(&["a".into()], &MessageChange::Flagged(true));
    assert!(
        flagged.contains(r#"PropertyTag="0x1090""#) && flagged.contains("<t:Value>2</t:Value>")
    );
}

fn event(all_day: bool) -> EventInput {
    EventInput {
        title: "Stand-up".into(),
        location: "Room 2".into(),
        notes: "Agenda <b>".into(),
        all_day,
        start: 1_796_104_800,
        end: 1_796_105_700,
        repeat: Repeat::Weekly,
        time_zone: "Africa/Nairobi".into(),
        reminders: Some(vec![5, 15]),
        repeat_rule: None,
        show_as: Some(ShowAs::Tentative),
        private: Some(true),
        attendees: Some(vec!["peter@client.example".into(), "not an address".into()]),
        notify_attendees: true,
        occurrence: None,
        scope: mail_core::ical::Scope::default(),
        organizer: None,
    }
}

#[test]
fn events_become_exchange_fields() {
    let rule = RepeatRule {
        freq: Freq::Weekly,
        interval: 2,
        by_day: vec!["MO".into(), "WE".into()],
        month_day: None,
        month: None,
        set_pos: None,
        count: Some(6),
        until: None,
    };
    let (fields, recurrence) = convert::event_fields(&event(false), Some(Some(&rule))).unwrap();
    assert_eq!(fields.start, "2026-12-01T06:00:00Z");
    assert_eq!(fields.reminder, Some(15));
    assert_eq!(fields.show_as, "Tentative");
    assert_eq!(fields.attendees, vec!["peter@client.example"]);
    assert_eq!(fields.zone.as_deref(), Some("E. Africa Standard Time"));
    let recurrence = recurrence.unwrap();
    assert_eq!(
        recurrence,
        "<t:Recurrence><t:WeeklyRecurrence><t:Interval>2</t:Interval><t:DaysOfWeek>Monday Wednesday</t:DaysOfWeek></t:WeeklyRecurrence><t:NumberedRecurrence><t:StartDate>2026-12-01</t:StartDate><t:NumberOfOccurrences>6</t:NumberOfOccurrences></t:NumberedRecurrence></t:Recurrence>"
    );
    let create = soap::create_event(&FolderRef::Id("cal"), &fields, true);
    assert!(Element::parse(&soap::envelope(&create)).is_ok());
    assert!(create.contains(r#"SendMeetingInvitations="SendToAllAndSaveCopy""#));
    assert!(create.contains("Agenda &lt;b&gt;"));
    assert!(create.contains(r#"<t:StartTimeZone Id="E. Africa Standard Time"/>"#));
    let update = soap::update_event("e1", &fields, Some(""), false);
    assert!(Element::parse(&soap::envelope(&update)).is_ok());
    assert!(update.contains(r#"SendMeetingInvitationsOrCancellations="SendToNone""#));
    assert!(update.contains(
        r#"<t:DeleteItemField><t:FieldURI FieldURI="calendar:Recurrence"/></t:DeleteItemField>"#
    ));
    // All day: a day in UTC, at least one long.
    let mut whole_day = event(true);
    whole_day.start = 1_796_083_200;
    whole_day.end = whole_day.start;
    let (fields, none) = convert::event_fields(&whole_day, None).unwrap();
    assert!(none.is_none());
    assert_eq!(fields.start, "2026-12-01T00:00:00Z");
    assert_eq!(fields.end, "2026-12-02T00:00:00Z");
    assert_eq!(fields.zone.as_deref(), Some("UTC"));
}

#[test]
fn repeat_rules_become_recurrences() {
    let rule = |freq, set_pos, month_day| RepeatRule {
        freq,
        interval: 1,
        by_day: vec!["TU".into()],
        month_day,
        month: None,
        set_pos,
        count: None,
        until: Some(1_798_675_200),
    };
    let monthly = convert::recurrence(&rule(Freq::Monthly, Some(2), None), "2026-12-01").unwrap();
    assert!(monthly.contains("<t:RelativeMonthlyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Tuesday</t:DaysOfWeek><t:DayOfWeekIndex>Second</t:DayOfWeekIndex>"));
    assert!(monthly.contains("<t:EndDateRecurrence><t:StartDate>2026-12-01</t:StartDate><t:EndDate>2026-12-31</t:EndDate></t:EndDateRecurrence>"));
    let yearly = convert::recurrence(&rule(Freq::Yearly, None, Some(25)), "2026-12-01").unwrap();
    assert!(yearly.contains(
        "<t:AbsoluteYearlyRecurrence><t:DayOfMonth>25</t:DayOfMonth><t:Month>December</t:Month>"
    ));
    assert!(convert::recurrence(&rule(Freq::Monthly, None, Some(-1)), "2026-12-01").is_err());
}

#[test]
fn calendar_items_are_read_as_icalendar() {
    let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:x\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    assert_eq!(convert::calendar_text(ics.as_bytes()).as_deref(), Some(ics));
    let message = format!(
        "From: a@b.c\r\nSubject: x\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nhi\r\n--b\r\nContent-Type: text/calendar; method=REQUEST\r\n\r\n{ics}--b--\r\n"
    );
    assert!(convert::calendar_text(message.as_bytes()).is_some_and(|c| c.contains("UID:x")));
    assert_eq!(
        convert::calendar_text(b"From: a@b.c\r\n\r\nno calendar"),
        None
    );
}

#[test]
fn times_are_read_in_every_form_exchange_writes() {
    assert_eq!(
        convert::parse_time("2026-12-01T06:00:00Z"),
        Some(1_796_104_800)
    );
    assert_eq!(
        convert::parse_time("2026-12-01T09:00:00+03:00"),
        Some(1_796_104_800)
    );
    assert_eq!(
        convert::parse_time("2026-12-01T06:00:00"),
        Some(1_796_104_800)
    );
    assert_eq!(
        convert::parse_time("2026-12-01T06:00:00.123"),
        Some(1_796_104_800)
    );
    assert_eq!(convert::parse_time("soon"), None);
    assert_eq!(convert::utc(1_796_104_800), "2026-12-01T06:00:00Z");
}

const SETTINGS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/responseschema/2006"><Response xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a"><User><DisplayName>Don</DisplayName><AutoDiscoverSMTPAddress>don@mocky.example</AutoDiscoverSMTPAddress></User><Account><AccountType>email</AccountType><Action>settings</Action>
<Protocol><Type>EXCH</Type><Server>EXCH01.corp.mocky.example</Server><ASUrl>https://exch01.corp.mocky.example/EWS/Exchange.asmx</ASUrl><EwsUrl>https://exch01.corp.mocky.example/EWS/Exchange.asmx</EwsUrl></Protocol>
<Protocol><Type>EXPR</Type><Server>mail.mocky.example</Server><EwsUrl>https://mail.mocky.example/EWS/Exchange.asmx</EwsUrl><ExternalEwsUrl>https://mail.mocky.example/EWS/Exchange.asmx</ExternalEwsUrl></Protocol>
<Protocol><Type>WEB</Type><Internal><OWAUrl AuthenticationMethod="Basic, Fba">https://mail.mocky.example/owa/</OWAUrl></Internal></Protocol>
</Account></Response></Autodiscover>"#;

#[test]
fn autodiscover_answers_are_read() {
    assert_eq!(
        autodiscover::parse(SETTINGS).unwrap(),
        Answer::Settings {
            ews_url: Some("https://exch01.corp.mocky.example/EWS/Exchange.asmx".into()),
            others: vec!["https://mail.mocky.example/EWS/Exchange.asmx".into()],
            address: Some("don@mocky.example".into()),
        }
    );
    let redirect_addr = r#"<Autodiscover xmlns="a"><Response xmlns="b"><Account><Action>redirectAddr</Action><RedirectAddr>don@corp.mocky.example</RedirectAddr></Account></Response></Autodiscover>"#;
    assert_eq!(
        autodiscover::parse(redirect_addr).unwrap(),
        Answer::RedirectAddr("don@corp.mocky.example".into())
    );
    let redirect_url = r#"<Autodiscover xmlns="a"><Response xmlns="b"><Account><Action>redirectUrl</Action><RedirectUrl>https://autodiscover.corp.mocky.example/autodiscover/autodiscover.xml</RedirectUrl></Account></Response></Autodiscover>"#;
    assert!(
        matches!(autodiscover::parse(redirect_url).unwrap(), Answer::RedirectUrl(u) if u.starts_with("https://autodiscover.corp"))
    );
    let error = r#"<Autodiscover xmlns="a"><Response><Error Time="10:00:00" Id="1"><ErrorCode>500</ErrorCode><Message>The email address can't be found.</Message></Error></Response></Autodiscover>"#;
    assert_eq!(
        autodiscover::parse(error).unwrap(),
        Answer::Error("The email address can't be found.".into())
    );
    // A plain web page is no answer.
    assert!(autodiscover::parse("<html><body>Hello</body></html>").is_err());
    // Insecure EWS addresses are never taken.
    let insecure = SETTINGS.replace("https://", "http://");
    assert!(matches!(
        autodiscover::parse(&insecure).unwrap(),
        Answer::Settings { ews_url: None, .. }
    ));
}

#[test]
fn autodiscover_asks_the_right_places_and_follows_only_safe_redirects() {
    assert_eq!(
        autodiscover::candidates("Mocky.Example."),
        vec![
            "https://mocky.example/autodiscover/autodiscover.xml".to_owned(),
            "https://autodiscover.mocky.example/autodiscover/autodiscover.xml".to_owned(),
        ]
    );
    assert_eq!(
        autodiscover::redirect_probe("mocky.example"),
        "http://autodiscover.mocky.example/autodiscover/autodiscover.xml"
    );
    assert_eq!(
        autodiscover::srv_candidate("mail.mocky.example.", 443).as_deref(),
        Some("https://mail.mocky.example/autodiscover/autodiscover.xml")
    );
    assert_eq!(
        autodiscover::srv_candidate("mail.mocky.example", 8443).as_deref(),
        Some("https://mail.mocky.example:8443/autodiscover/autodiscover.xml")
    );
    assert_eq!(autodiscover::srv_candidate("bad host/", 443), None);
    assert!(
        autodiscover::acceptable("https://mail.mocky.example/autodiscover/autodiscover.xml")
            .is_some()
    );
    for unsafe_url in [
        "http://mail.mocky.example/autodiscover/autodiscover.xml",
        "https://user:pass@mail.mocky.example/",
        "ftp://mail.mocky.example/",
        "javascript:alert(1)",
        "https://",
    ] {
        assert!(
            autodiscover::acceptable(unsafe_url).is_none(),
            "{unsafe_url}"
        );
    }
    let mut hops = Hops::default();
    for n in 0..autodiscover::MAX_HOPS {
        hops.take(&format!("user{n}@example.com")).unwrap();
    }
    assert!(matches!(
        hops.take("one-more@example.com"),
        Err(SyncError::Config(_))
    ));
    let mut circle = Hops::default();
    circle.take("a@example.com").unwrap();
    assert!(circle.take("A@example.com").is_err());
    assert_eq!(
        autodiscover::domain_of("don@Mocky.Example").as_deref(),
        Some("mocky.example")
    );
    assert_eq!(autodiscover::domain_of("nobody"), None);
    let request = autodiscover::request("don&co@mocky.example");
    assert!(request.contains("<EMailAddress>don&amp;co@mocky.example</EMailAddress>"));
    assert_eq!(
        autodiscover::ews_url_for("mail.mocky.example").as_deref(),
        Some("https://mail.mocky.example/EWS/Exchange.asmx")
    );
    assert_eq!(
        autodiscover::ews_url_for("https://mail.mocky.example/EWS/Exchange.asmx").as_deref(),
        Some("https://mail.mocky.example/EWS/Exchange.asmx")
    );
    assert_eq!(autodiscover::ews_url_for("http://mail.mocky.example"), None);
}

#[test]
fn xml_answers_are_read_carefully() {
    let deep = format!("{}{}", "<a>".repeat(100), "</a>".repeat(100));
    assert!(Element::parse(&deep).is_err());
    assert!(Element::parse("<a><b></a>").is_err());
    assert!(Element::parse("").is_err());
    let root =
        Element::parse(r#"<x:A xmlns:x="u" x:k="v &amp; w"><x:B>one &lt;two&gt;</x:B><B/></x:A>"#)
            .unwrap();
    assert_eq!(root.attr("k"), Some("v & w"));
    assert_eq!(root.text_of("B"), Some("one <two>"));
    assert_eq!(root.children_named("B").count(), 2);
    assert!(
        super::EwsClient::new(
            "http://mail.mocky.example/EWS/Exchange.asmx",
            super::EwsLogin {
                username: "u".into(),
                password: "p".into(),
            }
        )
        .is_err()
            || cfg!(feature = "insecure-plaintext")
    );
    let login = super::EwsLogin {
        username: "CORP\\don".into(),
        password: "hunter2".into(),
    };
    assert!(!format!("{login:?}").contains("hunter2"));
}
