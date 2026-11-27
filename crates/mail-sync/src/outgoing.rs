//! Building the RFC 5322 bytes for a message the user wrote.

use base64::Engine as _;
use lettre::message::header::{
    ContentTransferEncoding, ContentType, HeaderName, HeaderValue, InReplyTo, References,
};
use lettre::message::{Attachment, Body, Mailbox as LettreMailbox, MultiPart, SinglePart};
use mail_core::importance::Importance;
use mail_core::message::{Mailbox, display_safe};

use crate::SyncError;

/// An attachment ready to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingAttachment {
    /// File name shown to the recipient.
    pub file_name: String,
    /// MIME type.
    pub mime_type: String,
    /// File contents.
    pub data: Vec<u8>,
}

/// A picture shown inside the HTML body (`<img src="cid:...">`), such as a
/// signature image. Sent as a related part, so it shows without the
/// recipient loading remote images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineImage {
    /// Content-ID without angle brackets, as used after `cid:`.
    pub content_id: String,
    /// MIME type.
    pub mime_type: String,
    /// Image bytes.
    pub data: Vec<u8>,
}

/// A message to build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingMessage {
    /// Sender.
    pub from: Mailbox,
    /// To.
    pub to: Vec<Mailbox>,
    /// Cc.
    pub cc: Vec<Mailbox>,
    /// Bcc: in the envelope only, never in the headers.
    pub bcc: Vec<Mailbox>,
    /// Subject.
    pub subject: String,
    /// Plain-text body.
    pub body_text: String,
    /// HTML version of the body (a full signature), sent alongside the text.
    pub body_html: Option<String>,
    /// Message-ID being replied to (no angle brackets).
    pub in_reply_to: Option<String>,
    /// Thread Message-IDs (no angle brackets).
    pub references: Vec<String>,
    /// Attachments.
    pub attachments: Vec<OutgoingAttachment>,
    /// Pictures the HTML body refers to by `cid:`. Ignored without HTML.
    pub inline_images: Vec<InlineImage>,
    /// High and low importance are sent as `Importance` and `X-Priority`.
    pub importance: Importance,
    /// Ask for a read receipt to this address (`Disposition-Notification-To`).
    pub read_receipt_to: Option<String>,
}

const IMPORTANCE: HeaderName = HeaderName::new_from_ascii_str("Importance");
const X_PRIORITY: HeaderName = HeaderName::new_from_ascii_str("X-Priority");
const DISPOSITION_NOTIFICATION_TO: HeaderName =
    HeaderName::new_from_ascii_str("Disposition-Notification-To");

/// The built message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltMessage {
    /// Bytes to hand to SMTP and to append to Sent.
    pub raw: Vec<u8>,
    /// The generated Message-ID (no angle brackets).
    pub message_id: String,
    /// Envelope sender.
    pub envelope_from: String,
    /// Every recipient, Bcc included.
    pub recipients: Vec<String>,
}

/// A copy of a built message that records its Bcc recipients in a `Bcc`
/// header, for the sender's own Drafts and Sent folders only. Without it
/// a reopened draft has lost its Bcc list, and nobody can later see who a
/// sent message was copied to. What is handed to SMTP must stay the
/// original, which never names them.
#[must_use]
pub fn with_bcc_header(raw: &[u8], bcc: &[String]) -> Vec<u8> {
    let addresses: Vec<&str> = bcc
        .iter()
        .map(String::as_str)
        .filter(|address| mail_core::compose::is_valid_address(address))
        .collect();
    if addresses.is_empty() {
        return raw.to_vec();
    }
    // One address per line (folded), so no line grows too long.
    let mut out = format!("Bcc: {}\r\n", addresses.join(",\r\n ")).into_bytes();
    out.extend_from_slice(raw);
    out
}

/// MIME type for a file name, from a short list of common types.
#[must_use]
pub fn mime_for_file_name(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "zip" => "application/zip",
        "ics" => "text/calendar",
        "eml" => "message/rfc822",
        "vcf" => "text/vcard",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}

fn to_lettre(mailbox: &Mailbox) -> Result<LettreMailbox, SyncError> {
    let address = mailbox.address.parse().map_err(|_| {
        SyncError::Invalid(format!(
            "\"{}\" is not a valid email address.",
            mailbox.address
        ))
    })?;
    Ok(LettreMailbox::new(
        mailbox
            .name
            .as_deref()
            .map(display_safe)
            .filter(|n| !n.is_empty()),
        address,
    ))
}

/// An attachment's body. A message attached whole (`message/rfc822`) goes
/// unencoded when its lines allow (RFC 2046 section 5.2.1); one that
/// doesn't fit 7-bit lines is base64, which mail apps open as well.
fn attachment_body(attachment: &OutgoingAttachment) -> Body {
    if attachment.mime_type.eq_ignore_ascii_case("message/rfc822")
        && let Ok(body) =
            Body::new_with_encoding(attachment.data.clone(), ContentTransferEncoding::SevenBit)
    {
        return body;
    }
    Body::new(attachment.data.clone())
}

fn random_token() -> Result<String, SyncError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|e| SyncError::Invalid(format!("no randomness available: {e}")))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn angle(ids: &[String]) -> String {
    ids.iter()
        .map(|id| format!("<{id}>"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Importance, and the read receipt asked for.
fn request_headers(
    mut builder: lettre::message::MessageBuilder,
    message: &OutgoingMessage,
) -> lettre::message::MessageBuilder {
    if let Some((importance, priority)) = message.importance.outgoing_headers() {
        builder = builder
            .raw_header(HeaderValue::new(IMPORTANCE, importance.to_owned()))
            .raw_header(HeaderValue::new(X_PRIORITY, priority.to_owned()));
    }
    if let Some(address) = message
        .read_receipt_to
        .as_deref()
        .filter(|a| mail_core::compose::is_valid_address(a))
    {
        builder = builder.raw_header(HeaderValue::new(
            DISPOSITION_NOTIFICATION_TO,
            format!("<{address}>"),
        ));
    }
    builder
}

/// Build a message. Header values are cleaned of control characters, so a
/// subject or name cannot inject extra headers.
///
/// # Errors
/// Invalid addresses, no recipients, or a builder failure.
pub fn build(message: &OutgoingMessage) -> Result<BuiltMessage, SyncError> {
    if message.to.is_empty() && message.cc.is_empty() && message.bcc.is_empty() {
        return Err(SyncError::Invalid("Add at least one recipient.".to_owned()));
    }
    let domain = message
        .from
        .address
        .rsplit_once('@')
        .map_or("localhost", |(_, d)| d);
    let message_id = format!("{}@{domain}", random_token()?);

    let mut builder = lettre::Message::builder()
        .from(to_lettre(&message.from)?)
        .subject(display_safe(&message.subject))
        .message_id(Some(format!("<{message_id}>")))
        .user_agent("Mocky Mail".to_owned());
    for m in &message.to {
        builder = builder.to(to_lettre(m)?);
    }
    for m in &message.cc {
        builder = builder.cc(to_lettre(m)?);
    }
    if let Some(parent) = message
        .in_reply_to
        .as_deref()
        .map(display_safe)
        .filter(|p| !p.is_empty())
    {
        builder = builder.header(InReplyTo::from(format!("<{parent}>")));
    }
    let references: Vec<String> = message
        .references
        .iter()
        .map(|r| display_safe(r))
        .filter(|r| !r.is_empty())
        .collect();
    if !references.is_empty() {
        builder = builder.header(References::from(angle(&references)));
    }
    builder = request_headers(builder, message);

    let text = SinglePart::plain(message.body_text.clone());
    // Text and HTML of the same message; readers show the HTML. Pictures the
    // HTML uses ride along with it in a multipart/related part.
    let alternative = |html: &String| -> Result<MultiPart, SyncError> {
        if message.inline_images.is_empty() {
            return Ok(MultiPart::alternative_plain_html(
                message.body_text.clone(),
                html.clone(),
            ));
        }
        let mut related = MultiPart::related().singlepart(SinglePart::html(html.clone()));
        for image in &message.inline_images {
            let content_type = ContentType::parse(&image.mime_type)
                .map_err(|e| SyncError::Invalid(e.to_string()))?;
            related = related.singlepart(
                Attachment::new_inline(display_safe(&image.content_id))
                    .body(image.data.clone(), content_type),
            );
        }
        Ok(MultiPart::alternative()
            .singlepart(SinglePart::plain(message.body_text.clone()))
            .multipart(related))
    };
    let email = match (&message.body_html, message.attachments.is_empty()) {
        (None, true) => builder.singlepart(text),
        (Some(html), true) => builder.multipart(alternative(html)?),
        (html, false) => {
            let mut parts = match html {
                Some(html) => MultiPart::mixed().multipart(alternative(html)?),
                None => MultiPart::mixed().singlepart(text),
            };
            for a in &message.attachments {
                let content_type = ContentType::parse(&a.mime_type)
                    .or_else(|_| ContentType::parse("application/octet-stream"))
                    .map_err(|e| SyncError::Invalid(e.to_string()))?;
                parts = parts.singlepart(
                    Attachment::new(mail_core::message::safe_file_name(&a.file_name))
                        .body(attachment_body(a), content_type),
                );
            }
            builder.multipart(parts)
        }
    }
    .map_err(|e| SyncError::Invalid(e.to_string()))?;

    let recipients = message
        .to
        .iter()
        .chain(&message.cc)
        .chain(&message.bcc)
        .map(|m| m.address.clone())
        .collect();
    Ok(BuiltMessage {
        raw: email.formatted(),
        message_id,
        envelope_from: message.from.address.clone(),
        recipients,
    })
}

/// An invitation, update or cancellation to attendees (iMIP, RFC 6047): a
/// short note and the calendar part (`method` `REQUEST` or `CANCEL`), to
/// everyone in `to`.
///
/// # Errors
/// [`SyncError::Invalid`] for bad addresses, an unknown method or no
/// recipients.
pub fn build_calendar_message(
    from: &Mailbox,
    to: &[Mailbox],
    subject: &str,
    note: &str,
    ics: &str,
    method: &str,
) -> Result<BuiltMessage, SyncError> {
    if !matches!(method, "REQUEST" | "CANCEL" | "REPLY") {
        return Err(SyncError::Invalid(format!(
            "unknown calendar method {method}"
        )));
    }
    if to.is_empty() {
        return Err(SyncError::Invalid(
            "no one to send the invitation to".to_owned(),
        ));
    }
    let domain = from
        .address
        .rsplit_once('@')
        .map_or("localhost", |(_, d)| d);
    let message_id = format!("{}@{domain}", random_token()?);
    let calendar = SinglePart::builder()
        .header(
            ContentType::parse(&format!("text/calendar; charset=utf-8; method={method}"))
                .map_err(|e| SyncError::Invalid(e.to_string()))?,
        )
        .body(ics.to_owned());
    let mut builder = lettre::Message::builder()
        .from(to_lettre(from)?)
        .subject(display_safe(subject))
        .message_id(Some(format!("<{message_id}>")))
        .user_agent("Mocky Mail".to_owned());
    for person in to {
        builder = builder.to(to_lettre(person)?);
    }
    let email = builder
        .multipart(
            MultiPart::alternative()
                .singlepart(SinglePart::plain(note.to_owned()))
                .singlepart(calendar),
        )
        .map_err(|e| SyncError::Invalid(e.to_string()))?;
    Ok(BuiltMessage {
        raw: email.formatted(),
        message_id,
        envelope_from: from.address.clone(),
        recipients: to.iter().map(|m| m.address.clone()).collect(),
    })
}

/// An answer to an invitation (iMIP, RFC 6047): a short note, and the
/// `METHOD:REPLY` calendar part calendar apps act on.
///
/// # Errors
/// [`SyncError::Invalid`] for bad addresses.
pub fn build_calendar_reply(
    from: &Mailbox,
    to: &Mailbox,
    subject: &str,
    note: &str,
    ics: &str,
) -> Result<BuiltMessage, SyncError> {
    let domain = from
        .address
        .rsplit_once('@')
        .map_or("localhost", |(_, d)| d);
    let message_id = format!("{}@{domain}", random_token()?);
    let calendar = SinglePart::builder()
        .header(
            ContentType::parse("text/calendar; charset=utf-8; method=REPLY")
                .map_err(|e| SyncError::Invalid(e.to_string()))?,
        )
        .body(ics.to_owned());
    let email = lettre::Message::builder()
        .from(to_lettre(from)?)
        .to(to_lettre(to)?)
        .subject(display_safe(subject))
        .message_id(Some(format!("<{message_id}>")))
        .user_agent("Mocky Mail".to_owned())
        .multipart(
            MultiPart::alternative()
                .singlepart(SinglePart::plain(note.to_owned()))
                .singlepart(calendar),
        )
        .map_err(|e| SyncError::Invalid(e.to_string()))?;
    Ok(BuiltMessage {
        raw: email.formatted(),
        message_id,
        envelope_from: from.address.clone(),
        recipients: vec![to.address.clone()],
    })
}

/// A read receipt (RFC 8098): `multipart/report;
/// report-type=disposition-notification` with a note and the
/// `message/disposition-notification` fields, to `to` from `from`, in the
/// original's thread.
///
/// # Errors
/// [`SyncError::Invalid`] for bad addresses.
pub fn build_read_receipt(
    from: &Mailbox,
    to: &str,
    receipt: &mail_core::receipts::Receipt<'_>,
) -> Result<BuiltMessage, SyncError> {
    let domain = from
        .address
        .rsplit_once('@')
        .map_or("localhost", |(_, d)| d);
    let message_id = format!("{}@{domain}", random_token()?);
    let to_mailbox = Mailbox {
        name: None,
        address: to.to_owned(),
    };
    let mut builder = lettre::Message::builder()
        .from(to_lettre(from)?)
        .to(to_lettre(&to_mailbox)?)
        .subject(mail_core::receipts::subject(receipt.subject))
        .message_id(Some(format!("<{message_id}>")))
        .user_agent("Mocky Mail".to_owned());
    if let Some(original) = receipt
        .original_message_id
        .map(|id| display_safe(id.trim().trim_start_matches('<').trim_end_matches('>')))
        .filter(|id| !id.is_empty() && !id.contains(char::is_whitespace))
    {
        builder = builder
            .header(InReplyTo::from(format!("<{original}>")))
            .header(References::from(format!("<{original}>")));
    }
    let report_type = ContentType::parse(&format!(
        "multipart/report; report-type=disposition-notification; boundary=\"{}\"",
        random_token()?
    ))
    .map_err(|e| SyncError::Invalid(e.to_string()))?;
    let fields = SinglePart::builder()
        .header(
            ContentType::parse("message/disposition-notification")
                .map_err(|e| SyncError::Invalid(e.to_string()))?,
        )
        .body(mail_core::receipts::disposition_fields(receipt));
    let report = MultiPart::builder()
        .header(report_type)
        .build()
        .singlepart(SinglePart::plain(mail_core::receipts::note(receipt)))
        .singlepart(fields);
    let email = builder
        .multipart(report)
        .map_err(|e| SyncError::Invalid(e.to_string()))?;
    Ok(BuiltMessage {
        raw: email.formatted(),
        message_id,
        envelope_from: from.address.clone(),
        recipients: vec![to.to_owned()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mb(address: &str, name: Option<&str>) -> Mailbox {
        Mailbox {
            name: name.map(str::to_owned),
            address: address.to_owned(),
        }
    }

    #[test]
    fn invitation_answers_carry_the_reply_part() {
        let built = build_calendar_reply(
            &mb("info@mocky.co.ke", Some("Mocky Digital")),
            &mb("peter@client.example", Some("Peter Mwangi")),
            "Accepted: Site visit",
            "Mocky Digital accepted this invitation.",
            "BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nEND:VCALENDAR\r\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let raw = String::from_utf8_lossy(&built.raw);
        assert!(raw.contains("multipart/alternative"));
        assert!(raw.contains("text/calendar; charset=utf-8; method=REPLY"));
        assert!(raw.contains("Subject: Accepted: Site visit"));
        assert_eq!(built.recipients, ["peter@client.example"]);
        let body = mail_core::message::parse_body(&built.raw).unwrap_or_else(|e| panic!("{e}"));
        assert!(body.calendar.unwrap_or_default().contains("METHOD:REPLY"));
    }

    #[test]
    fn invitations_go_to_every_attendee() {
        let built = build_calendar_message(
            &mb("info@mocky.co.ke", Some("Mocky Digital")),
            &[
                mb("peter@client.example", Some("Peter Mwangi")),
                mb("amina@studio.example", None),
            ],
            "Invitation: Site visit",
            "Mocky Digital invites you: Site visit",
            "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nEND:VCALENDAR\r\n",
            "REQUEST",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let raw = String::from_utf8_lossy(&built.raw);
        assert!(raw.contains("text/calendar; charset=utf-8; method=REQUEST"));
        assert_eq!(
            built.recipients,
            ["peter@client.example", "amina@studio.example"]
        );
        assert!(
            build_calendar_message(&mb("a@b.co", None), &[], "s", "n", "x", "REQUEST").is_err()
        );
        assert!(
            build_calendar_message(
                &mb("a@b.co", None),
                &[mb("c@d.co", None)],
                "s",
                "n",
                "x",
                "ADD"
            )
            .is_err()
        );
    }

    fn base() -> OutgoingMessage {
        OutgoingMessage {
            from: mb("me@work.example", Some("Don")),
            to: vec![mb("peter@client.example", Some("Peter Mwangi"))],
            cc: vec![],
            bcc: vec![mb("secret@hidden.example", None)],
            subject: "Invoice INV-0144".into(),
            body_text: "Hi Peter,\n\nInvoice attached.".into(),
            body_html: None,
            in_reply_to: Some("q-1@client.example".into()),
            references: vec!["q-0@client.example".into(), "q-1@client.example".into()],
            attachments: vec![OutgoingAttachment {
                file_name: "INV-0144.pdf".into(),
                mime_type: "application/pdf".into(),
                data: b"%PDF-1.4".to_vec(),
            }],
            inline_images: vec![],
            importance: Importance::Normal,
            read_receipt_to: None,
        }
    }

    #[test]
    fn a_read_receipt_is_asked_for_only_when_wanted() {
        let none = String::from_utf8_lossy(&build(&base()).unwrap().raw).into_owned();
        assert!(!none.contains("Disposition-Notification-To"));
        let built = build(&OutgoingMessage {
            read_receipt_to: Some("me@work.example".into()),
            ..base()
        })
        .unwrap();
        let raw = String::from_utf8_lossy(&built.raw).into_owned();
        assert!(
            raw.contains("Disposition-Notification-To: <me@work.example>\r\n"),
            "{raw}"
        );
        let request = mail_core::receipts::requested(&built.raw).unwrap();
        assert_eq!(request.address, "me@work.example");
        // Nothing that could add a header gets in.
        let bad = build(&OutgoingMessage {
            read_receipt_to: Some("me@work.example>\r\nBcc: x@evil.example".into()),
            ..base()
        })
        .unwrap();
        let raw = String::from_utf8_lossy(&bad.raw).into_owned();
        assert!(!raw.contains("Disposition-Notification-To"));
        assert!(!raw.contains("evil"));
    }

    #[test]
    fn read_receipts_are_disposition_reports() {
        let receipt = mail_core::receipts::Receipt {
            final_recipient: "me@work.example",
            original_message_id: Some("q-1@client.example"),
            subject: "Quote",
            automatic: false,
        };
        let built = build_read_receipt(
            &mb("me@work.example", Some("Don")),
            "peter@client.example",
            &receipt,
        )
        .unwrap();
        let raw = String::from_utf8_lossy(&built.raw).into_owned();
        assert!(
            raw.contains("multipart/report; report-type=disposition-notification;"),
            "{raw}"
        );
        assert!(raw.contains("Content-Type: message/disposition-notification"));
        assert!(raw.contains("Disposition: manual-action/MDN-sent-manually; displayed"));
        assert!(raw.contains("Original-Message-ID: <q-1@client.example>"));
        assert!(raw.contains("Final-Recipient: rfc822;me@work.example"));
        assert!(raw.contains("Subject: Read: Quote"));
        assert!(!raw.contains("Disposition-Notification-To"));
        assert_eq!(built.recipients, ["peter@client.example"]);
        let summary = mail_core::message::parse_summary(&built.raw).unwrap();
        assert_eq!(summary.in_reply_to.as_deref(), Some("q-1@client.example"));
        assert_eq!(summary.to[0].address, "peter@client.example");
        // A receipt never asks for one back.
        assert_eq!(mail_core::receipts::requested(&built.raw), None);
    }

    #[test]
    fn a_rich_text_message_goes_as_text_and_html_with_its_pictures() {
        use base64::Engine as _;
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDRpixels".to_vec();
        let html = format!(
            "<p>Hi <b>Peter</b></p><ul><li>Paint</li></ul><img src=\"data:image/png;base64,{}\" alt=\"plan\"><script>x()</script>",
            base64::engine::general_purpose::STANDARD.encode(&png)
        );
        let rich = mail_core::rich::finish(&mail_core::rich::clean(&html), None).unwrap();
        let mut m = base();
        m.attachments.clear();
        m.body_text = rich.text.clone();
        m.body_html = Some(rich.html.clone());
        m.inline_images = rich
            .inline_images
            .into_iter()
            .map(|i| InlineImage {
                content_id: i.content_id,
                mime_type: i.mime_type,
                data: i.data,
            })
            .collect();
        let built = build(&m).unwrap();
        let raw = String::from_utf8_lossy(&built.raw).into_owned();
        assert!(raw.contains("multipart/alternative"));
        assert!(raw.contains("multipart/related"));
        let body = mail_core::message::parse_body(&built.raw).unwrap();
        let sent_html = body.html.unwrap();
        assert!(sent_html.contains("<b>Peter</b>"));
        assert!(!sent_html.contains("script"));
        assert!(sent_html.contains(mail_core::rich::RICH_MARKER));
        assert_eq!(
            body.text.unwrap().replace("\r\n", "\n").trim_end(),
            "Hi Peter\n\n- Paint\n\n[plan]"
        );
        assert!(body.attachments.is_empty());
        assert_eq!(body.inline_images.len(), 1);
        assert_eq!(body.inline_images[0].data, png);
    }

    #[test]
    fn a_message_attached_whole_is_message_rfc822() {
        assert_eq!(mime_for_file_name("Quote.eml"), "message/rfc822");
        assert_eq!(mime_for_file_name("Peter.VCF"), "text/vcard");
        let inner =
            b"From: a@x.example\r\nTo: b@x.example\r\nSubject: Inner\r\n\r\nHello\r\n".to_vec();
        let mut m = base();
        m.attachments = vec![OutgoingAttachment {
            file_name: "Inner.eml".into(),
            mime_type: mime_for_file_name("Inner.eml").into(),
            data: inner,
        }];
        let built = build(&m).unwrap();
        let raw = String::from_utf8_lossy(&built.raw).into_owned();
        assert!(raw.contains("Content-Type: message/rfc822"), "{raw}");
        assert!(raw.contains("Content-Transfer-Encoding: 7bit"), "{raw}");
        let body = mail_core::message::parse_body(&built.raw).unwrap();
        assert_eq!(body.attachments.len(), 1);
        assert_eq!(body.attachments[0].file_name, "Inner.eml");
    }

    #[test]
    fn importance_is_sent_as_two_headers_and_normal_as_none() {
        let normal = String::from_utf8_lossy(&build(&base()).unwrap().raw).into_owned();
        assert!(!normal.contains("Importance:") && !normal.contains("X-Priority:"));
        for (importance, header, priority) in [
            (
                Importance::High,
                "Importance: High",
                "X-Priority: 1 (Highest)",
            ),
            (Importance::Low, "Importance: Low", "X-Priority: 5 (Lowest)"),
        ] {
            let built = build(&OutgoingMessage {
                importance,
                ..base()
            })
            .unwrap();
            let raw = String::from_utf8_lossy(&built.raw).into_owned();
            assert!(raw.contains(header), "{raw}");
            assert!(raw.contains(priority), "{raw}");
            assert_eq!(
                mail_core::message::parse_summary(&built.raw)
                    .unwrap()
                    .importance,
                importance
            );
        }
    }

    #[test]
    fn builds_a_message_that_parses_back() {
        let built = build(&base()).unwrap();
        let summary = mail_core::message::parse_summary(&built.raw).unwrap();
        assert_eq!(summary.subject, "Invoice INV-0144");
        assert_eq!(summary.to[0].address, "peter@client.example");
        assert_eq!(
            summary.message_id.as_deref(),
            Some(built.message_id.as_str())
        );
        assert_eq!(summary.in_reply_to.as_deref(), Some("q-1@client.example"));
        assert_eq!(
            summary.references,
            vec!["q-0@client.example", "q-1@client.example"]
        );
        assert_eq!(summary.attachments.len(), 1);
        assert_eq!(summary.attachments[0].file_name, "INV-0144.pdf");
        assert!(built.message_id.ends_with("@work.example"));
    }

    #[test]
    fn a_signature_goes_as_html_alongside_the_text() {
        let html = "<!doctype html><html><body><p>Hi Peter,</p><div class=\"signature\"><b>Don</b></div></body></html>";
        for keep_attachments in [false, true] {
            let mut message = base();
            message.body_html = Some(html.to_owned());
            if !keep_attachments {
                message.attachments.clear();
            }
            let built = build(&message).unwrap();
            let raw = String::from_utf8_lossy(&built.raw);
            assert!(raw.contains("multipart/alternative"), "{raw}");
            assert!(raw.contains("text/plain"));
            assert!(raw.contains("text/html"));
            let parsed = mail_core::message::parse_body(&built.raw).unwrap();
            assert!(parsed.html.unwrap().contains("<b>Don</b>"));
            assert!(parsed.text.unwrap().contains("Invoice attached"));
            assert_eq!(parsed.attachments.len(), usize::from(keep_attachments));
        }
    }

    #[test]
    fn bcc_is_in_the_envelope_but_not_the_headers() {
        let built = build(&base()).unwrap();
        assert!(
            built
                .recipients
                .contains(&"secret@hidden.example".to_owned())
        );
        let text = String::from_utf8_lossy(&built.raw).to_ascii_lowercase();
        assert!(!text.contains("bcc:"));
        assert!(!text.contains("secret@hidden.example"));
    }

    #[test]
    fn subject_cannot_inject_headers() {
        let mut m = base();
        m.subject = "Hi\r\nBcc: victim@evil.example".into();
        let built = build(&m).unwrap();
        let text = String::from_utf8_lossy(&built.raw);
        assert!(!text.contains("\r\nBcc: victim"));
    }

    #[test]
    fn needs_a_recipient() {
        let mut m = base();
        m.to.clear();
        m.bcc.clear();
        assert!(matches!(build(&m), Err(SyncError::Invalid(_))));
    }

    #[test]
    fn message_ids_are_unique() {
        assert_ne!(
            build(&base()).unwrap().message_id,
            build(&base()).unwrap().message_id
        );
    }

    #[test]
    fn mime_types() {
        assert_eq!(mime_for_file_name("Report.PDF"), "application/pdf");
        assert_eq!(mime_for_file_name("noext"), "application/octet-stream");
    }

    #[test]
    fn inline_images_are_sent_with_the_html() {
        let mut m = base();
        m.body_html = Some(r#"<p>Hi</p><img src="cid:sig@mocky.mail">"#.into());
        m.inline_images = vec![InlineImage {
            content_id: "sig@mocky.mail".into(),
            mime_type: "image/png".into(),
            data: b"\x89PNG\r\n\x1a\nfake".to_vec(),
        }];
        let built = build(&m).unwrap();
        let raw = String::from_utf8_lossy(&built.raw);
        assert!(raw.contains("multipart/related"));
        assert!(raw.contains("Content-ID: <sig@mocky.mail>"));
        assert!(raw.contains("Content-Disposition: inline"));
        // Still a mixed message with the PDF attached.
        assert!(raw.contains("multipart/mixed"));
        // Readers see the HTML and only the PDF as an attachment.
        let body = mail_core::message::parse_body(&built.raw).unwrap();
        assert!(body.html.unwrap().contains("cid:sig@mocky.mail"));
        assert_eq!(body.attachments.len(), 1);
    }
}
