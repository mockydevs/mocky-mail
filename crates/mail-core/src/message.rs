//! Turning a raw RFC 5322 message into what the message list and reading
//! pane need.
//!
//! Every string that came from the sender and will be shown to the user
//! passes through [`display_safe`], which removes control characters and the
//! invisible Unicode direction marks used to disguise file names and
//! senders.

use mail_parser::{
    Address, HeaderValue, Message, MessageParser, MessagePart, MimeHeaders, PartType,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Largest message we will parse in one go (100 MiB). Bigger ones are
/// refused rather than risking memory exhaustion on low-end machines.
pub const MAX_MESSAGE_BYTES: usize = 100 * 1024 * 1024;
/// Characters of body text kept for the message-list preview.
pub const PREVIEW_CHARS: usize = 160;
/// Name used when an attachment has no usable file name.
pub const FALLBACK_FILE_NAME: &str = "attachment";

/// Why a message could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    /// The input was empty.
    #[error("message is empty")]
    Empty,
    /// The input is larger than [`MAX_MESSAGE_BYTES`].
    #[error("message is {size} bytes; the limit is {max}")]
    TooLarge {
        /// Input size in bytes.
        size: usize,
        /// The limit in bytes.
        max: usize,
    },
    /// The input is not a mail message.
    #[error("message could not be parsed")]
    Malformed,
}

/// A sender or recipient.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mailbox {
    /// Display name, if any.
    pub name: Option<String>,
    /// Email address.
    pub address: String,
}

/// An attachment, without its contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentInfo {
    /// Safe file name to show and to suggest when saving.
    pub file_name: String,
    /// MIME type such as `application/pdf`.
    pub mime_type: String,
    /// Decoded size in bytes.
    pub size: u64,
}

/// Everything the list and the reading pane header need.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSummary {
    /// The Message-ID header, used to find duplicates in Sent.
    pub message_id: Option<String>,
    /// Subject, or an empty string.
    pub subject: String,
    /// First sender.
    pub from: Option<Mailbox>,
    /// Where replies should go, if the sender asked for somewhere else.
    pub reply_to: Vec<Mailbox>,
    /// Recipients in To.
    pub to: Vec<Mailbox>,
    /// Recipients in Cc.
    pub cc: Vec<Mailbox>,
    /// Date header as Unix seconds.
    pub date: Option<i64>,
    /// Start of the body text, whitespace collapsed.
    pub preview: String,
    /// Whether the sender included an HTML body.
    pub has_html: bool,
    /// Attachments (not inline images).
    pub attachments: Vec<AttachmentInfo>,
    /// The Message-ID this message replies to.
    pub in_reply_to: Option<String>,
    /// Message-IDs of the thread so far, oldest first.
    pub references: Vec<String>,
    /// Mailing-list or automated mail (List-Unsubscribe, List-Id,
    /// Precedence: bulk, Auto-Submitted): goes to Other in a Focused Inbox.
    #[serde(default)]
    pub bulk: bool,
    /// From the `Importance`, `X-Priority` or `X-MSMail-Priority` header.
    #[serde(default)]
    pub importance: crate::importance::Importance,
}

/// An image embedded in an HTML body and referenced as `cid:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineImage {
    /// Content-ID without angle brackets.
    pub content_id: String,
    /// Image MIME type (raster formats only).
    pub mime_type: String,
    /// Decoded image bytes.
    pub data: Vec<u8>,
}

/// The readable content of a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageBody {
    /// Plain-text body, if the sender included one.
    pub text: Option<String>,
    /// HTML body, unsanitized. Always pass through `sanitize` before showing.
    pub html: Option<String>,
    /// Attachments, in the same order as [`attachment_bytes`] indexes them.
    pub attachments: Vec<AttachmentInfo>,
    /// Images referenced from the HTML body.
    pub inline_images: Vec<InlineImage>,
    /// The first calendar part (`text/calendar`, an invitation or reply),
    /// as text.
    pub calendar: Option<String>,
}

/// Largest calendar part read (1 MiB).
const MAX_CALENDAR_BYTES: usize = 1024 * 1024;

/// The first `text/calendar` (or `application/ics`) part's text.
fn calendar_part(message: &Message<'_>) -> Option<String> {
    message.parts.iter().find_map(|part| {
        let mime = mime_type(part);
        let named_ics = part
            .attachment_name()
            .is_some_and(|n| n.to_ascii_lowercase().ends_with(".ics"));
        if mime != "text/calendar" && mime != "application/ics" && !named_ics {
            return None;
        }
        let data = part.contents();
        (!data.is_empty() && data.len() <= MAX_CALENDAR_BYTES)
            .then(|| String::from_utf8_lossy(data).into_owned())
    })
}

/// Image types that may be shown inline. SVG is excluded: it can carry script.
const INLINE_IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];
/// Largest inline image kept (5 MiB).
pub const MAX_INLINE_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// True for characters that must never be shown from untrusted text:
/// control characters and invisible bidirectional formatting marks.
fn is_unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{061C}'
        )
}

/// Remove control characters and direction overrides, and trim.
#[must_use]
pub fn display_safe(text: &str) -> String {
    text.chars()
        .filter(|&c| !is_unsafe_char(c))
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Whether Windows treats this name as a device (`CON`, `NUL`, `COM1`...)
/// whatever its extension, so a file can't be called that.
fn is_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix)
            .is_some_and(|rest| matches!(rest, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
    };
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered("COM") || numbered("LPT")
}

/// A file name that is safe to show and to save on every system: no
/// directories, no hidden direction tricks, not empty, not "." or "..",
/// and nothing Windows refuses in a name (`: * ? " < > |`, a trailing dot
/// or space, device names such as `CON`). A colon is the dangerous one: on
/// Windows `Scan 12:30.pdf` would be written into a hidden stream of a
/// file called `Scan 12`.
#[must_use]
pub fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = display_safe(base)
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .trim();
    if cleaned.is_empty() {
        FALLBACK_FILE_NAME.to_owned()
    } else if is_device_name(cleaned) {
        format!("_{cleaned}")
    } else {
        cleaned.to_owned()
    }
}

/// Collapse whitespace and keep the first `max_chars` characters.
#[must_use]
pub fn make_preview(text: &str, max_chars: usize) -> String {
    // Line breaks are control characters; turn them into spaces first so
    // display_safe does not glue words from separate lines together.
    let spaced: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    let safe = display_safe(&spaced);
    let mut out = String::new();
    for (count, word) in safe.split_whitespace().enumerate() {
        if count > 0 {
            out.push(' ');
        }
        out.push_str(word);
        if out.chars().count() >= max_chars {
            break;
        }
    }
    out.chars().take(max_chars).collect()
}

fn mailboxes(address: Option<&Address<'_>>) -> Vec<Mailbox> {
    address
        .into_iter()
        .flat_map(Address::iter)
        .filter_map(|addr| {
            let address = display_safe(addr.address()?);
            if address.is_empty() {
                return None;
            }
            let name = addr.name().map(display_safe).filter(|n| !n.is_empty());
            Some(Mailbox { name, address })
        })
        .collect()
}

fn mime_type(part: &MessagePart<'_>) -> String {
    part.content_type().map_or_else(
        || "application/octet-stream".to_owned(),
        |ct| match ct.subtype() {
            Some(sub) => format!("{}/{}", ct.ctype(), sub).to_ascii_lowercase(),
            None => ct.ctype().to_ascii_lowercase(),
        },
    )
}

fn clean_id(id: &str) -> String {
    display_safe(id.trim().trim_start_matches('<').trim_end_matches('>'))
}

/// An image shown inside the HTML body rather than offered as a download.
fn inline_image(part: &MessagePart<'_>) -> Option<InlineImage> {
    let content_id = clean_id(part.content_id()?);
    let explicit_attachment = part
        .content_disposition()
        .is_some_and(mail_parser::ContentType::is_attachment);
    let mime = mime_type(part);
    if content_id.is_empty() || explicit_attachment || !INLINE_IMAGE_TYPES.contains(&mime.as_str())
    {
        return None;
    }
    let data = part.contents();
    (data.len() <= MAX_INLINE_IMAGE_BYTES).then(|| InlineImage {
        content_id,
        mime_type: mime,
        data: data.to_vec(),
    })
}

/// Whether the HTML body shows this part through a `cid:` reference.
fn shown_inline(part: &MessagePart<'_>, html: &str) -> bool {
    inline_image(part).is_some_and(|image| html.contains(&format!("cid:{}", image.content_id)))
}

fn attachment_parts<'a>(message: &'a Message<'a>) -> impl Iterator<Item = &'a MessagePart<'a>> {
    // An image with a Content-ID is only "inline" when the body really
    // shows it. In plain-text mail, or HTML that never refers to it, it
    // would be neither displayed nor listed, so there it stays a download.
    let html = message
        .body_html(0)
        .map(std::borrow::Cow::into_owned)
        .unwrap_or_default();
    message
        .attachments()
        .filter(move |part| matches!(part.body, PartType::Message(_)) || !shown_inline(part, &html))
}

/// An attachment as listed: a part of the message, or a file packed inside
/// a `winmail.dat` (TNEF) part, which Outlook sends in place of the files.
enum Listed<'a> {
    Part(&'a MessagePart<'a>),
    Packed(mailcraft_tnef::File),
}

/// Whether a part is a TNEF stream (`winmail.dat`, `application/ms-tnef`).
fn is_tnef(part: &MessagePart<'_>) -> bool {
    let mime = mime_type(part);
    let named = part
        .attachment_name()
        .is_some_and(|n| n.trim().eq_ignore_ascii_case("winmail.dat"));
    (named || mime == "application/ms-tnef" || mime == "application/vnd.ms-tnef")
        && mailcraft_tnef::is_tnef(part.contents())
}

/// What a TNEF part holds, when it can be read.
fn tnef_of(part: &MessagePart<'_>) -> Option<mailcraft_tnef::Tnef> {
    if !is_tnef(part) {
        return None;
    }
    mailcraft_tnef::read(part.contents(), &mailcraft_tnef::Limits::default()).ok()
}

/// The attachments to list, in order. A `winmail.dat` is opened: its files
/// are listed instead of it, and it stays listed too when something in it
/// couldn't be turned into a file (an attached message, an OLE object), so
/// nothing is lost.
fn listed<'a>(message: &'a Message<'a>) -> Vec<Listed<'a>> {
    let mut out = Vec::new();
    for part in attachment_parts(message) {
        if let Some(tnef) = tnef_of(part).filter(|t| !t.files.is_empty()) {
            let complete = tnef.is_complete();
            out.extend(tnef.files.into_iter().map(Listed::Packed));
            if !complete {
                out.push(Listed::Part(part));
            }
            continue;
        }
        out.push(Listed::Part(part));
    }
    out
}

fn listed_info(entry: &Listed<'_>) -> AttachmentInfo {
    match entry {
        Listed::Part(part) => attachment_info(part),
        Listed::Packed(file) => AttachmentInfo {
            file_name: safe_file_name(file.name.as_deref().unwrap_or_default()),
            mime_type: file
                .mime_type
                .as_deref()
                .map(|m| m.trim().to_ascii_lowercase())
                .filter(|m| m.contains('/') && m.len() < 128 && !m.contains(char::is_whitespace))
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
            size: u64::try_from(file.data.len()).unwrap_or(u64::MAX),
        },
    }
}

/// The text of a `winmail.dat`'s message, for mail that has no other body.
fn tnef_body(message: &Message<'_>) -> Option<String> {
    attachment_parts(message)
        .filter_map(tnef_of)
        .find_map(|t| t.body)
        .filter(|b| !b.trim().is_empty())
}

fn attachment_info(part: &MessagePart<'_>) -> AttachmentInfo {
    let fallback = if matches!(part.body, PartType::Message(_)) {
        "message.eml"
    } else {
        ""
    };
    AttachmentInfo {
        file_name: safe_file_name(part.attachment_name().unwrap_or(fallback)),
        mime_type: mime_type(part),
        size: u64::try_from(part.len()).unwrap_or(u64::MAX),
    }
}

fn id_list(value: &HeaderValue<'_>) -> Vec<String> {
    // Check the list form first: for a multi-value header `as_text` would
    // return only the first ID and silently drop the rest of the thread.
    let ids: Vec<String> = match (value.as_text_list(), value.as_text()) {
        (Some(many), _) => many.iter().map(|id| clean_id(id)).collect(),
        (None, Some(one)) => one.split_whitespace().map(clean_id).collect(),
        (None, None) => Vec::new(),
    };
    ids.into_iter().filter(|id| !id.is_empty()).collect()
}

pub(crate) fn parse(raw: &[u8]) -> Result<Message<'_>, ParseError> {
    if raw.is_empty() {
        return Err(ParseError::Empty);
    }
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(ParseError::TooLarge {
            size: raw.len(),
            max: MAX_MESSAGE_BYTES,
        });
    }
    MessageParser::default()
        .parse(raw)
        .ok_or(ParseError::Malformed)
}

/// Extract the readable body, attachments and inline images.
///
/// # Errors
/// Same as [`parse_summary`].
pub fn parse_body(raw: &[u8]) -> Result<MessageBody, ParseError> {
    let message = parse(raw)?;
    let html = has_real_html(&message)
        .then(|| message.body_html(0))
        .flatten()
        .map(std::borrow::Cow::into_owned);
    let text = message
        .text_body
        .iter()
        .filter_map(|&i| message.parts.get(i as usize))
        .any(|part| matches!(part.body, PartType::Text(_)))
        .then(|| message.body_text(0))
        .flatten()
        .map(std::borrow::Cow::into_owned);
    // Outlook may put the only text in the winmail.dat.
    let blank = text.as_deref().is_none_or(|t| t.trim().is_empty());
    let text = if blank && html.is_none() {
        tnef_body(&message).or(text)
    } else {
        text
    };
    Ok(MessageBody {
        text,
        html,
        attachments: listed(&message).iter().map(listed_info).collect(),
        inline_images: message.parts.iter().filter_map(inline_image).collect(),
        calendar: calendar_part(&message),
    })
}

/// The bytes of attachment number `index` (as listed by [`parse_body`]),
/// for saving to disk.
///
/// # Errors
/// Same as [`parse_summary`]. `Ok(None)` when there is no such attachment.
pub fn attachment_bytes(
    raw: &[u8],
    index: usize,
) -> Result<Option<(AttachmentInfo, Vec<u8>)>, ParseError> {
    let message = parse(raw)?;
    Ok(listed(&message).into_iter().nth(index).map(|entry| {
        let info = listed_info(&entry);
        let bytes = match entry {
            Listed::Part(part) => part.contents().to_vec(),
            Listed::Packed(file) => file.data,
        };
        (info, bytes)
    }))
}

fn has_real_html(message: &Message<'_>) -> bool {
    message
        .html_body
        .iter()
        .filter_map(|&index| message.parts.get(index as usize))
        .any(|part| matches!(part.body, PartType::Html(_)))
}

/// Parse a raw message into a [`MessageSummary`].
///
/// # Errors
/// Returns [`ParseError`] for empty, oversized or unparseable input. Never
/// panics, whatever the input.
pub fn parse_summary(raw: &[u8]) -> Result<MessageSummary, ParseError> {
    let message = parse(raw)?;
    Ok(MessageSummary {
        message_id: message.message_id().map(clean_id),
        subject: message.subject().map(display_safe).unwrap_or_default(),
        from: mailboxes(message.from()).into_iter().next(),
        reply_to: mailboxes(message.reply_to()),
        to: mailboxes(message.to()),
        cc: mailboxes(message.cc()),
        date: message.date().map(mail_parser::DateTime::to_timestamp),
        preview: message
            .body_text(0)
            .map(|text| make_preview(&text, PREVIEW_CHARS))
            .unwrap_or_default(),
        has_html: has_real_html(&message),
        attachments: listed(&message).iter().map(listed_info).collect(),
        in_reply_to: id_list(message.in_reply_to()).into_iter().next(),
        references: id_list(message.references()),
        bulk: crate::focus::bulk_headers(
            message.header_raw("List-Unsubscribe").is_some(),
            message.header_raw("List-Id").is_some(),
            message.header_raw("Precedence"),
            message.header_raw("Auto-Submitted"),
        ),
        importance: crate::importance::Importance::from_headers(
            message.header_raw("Importance"),
            message.header_raw("X-Priority"),
            message.header_raw("X-MSMail-Priority"),
        ),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const PLAIN: &[u8] = b"From: Wanjiru Kamau <wanjiru@fitout.example>\r\n\
To: You <you@work.example>, accounts@work.example\r\n\
Cc: Amina <amina@studio.example>\r\n\
Subject: Revised quotation\r\n\
Date: Thu, 13 Nov 2026 09:42:00 +0300\r\n\
Message-ID: <q-123@fitout.example>\r\n\
\r\n\
Hi,\r\n\r\nAttached is the   revised quotation.\r\n";

    #[test]
    fn parses_a_plain_message() {
        let s = parse_summary(PLAIN).unwrap();
        assert_eq!(s.subject, "Revised quotation");
        assert_eq!(
            s.from,
            Some(Mailbox {
                name: Some("Wanjiru Kamau".into()),
                address: "wanjiru@fitout.example".into()
            })
        );
        assert_eq!(s.to.len(), 2);
        assert_eq!(s.to[1].name, None);
        assert_eq!(s.cc[0].address, "amina@studio.example");
        assert_eq!(s.message_id.as_deref(), Some("q-123@fitout.example"));
        assert_eq!(s.date, Some(1_794_552_120));
        assert_eq!(s.preview, "Hi, Attached is the revised quotation.");
        assert!(!s.has_html);
        assert!(s.attachments.is_empty());
    }

    #[test]
    fn decodes_encoded_word_subjects() {
        let raw = b"From: a@b.example\r\nSubject: =?UTF-8?B?SmFtYm8gcmFmaWtpIPCfkYs=?=\r\n\r\nbody";
        assert_eq!(
            parse_summary(raw).unwrap().subject,
            "Jambo rafiki \u{1F44B}"
        );
    }

    #[test]
    fn lists_attachments_with_safe_names() {
        let raw = b"From: a@b.example\r\n\
Subject: files\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"XX\"\r\n\
\r\n\
--XX\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>See attached</p>\r\n\
--XX\r\n\
Content-Type: application/pdf\r\n\
Content-Disposition: attachment; filename=\"../../invoice\xE2\x80\xAEfdp.exe\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
SGVsbG8=\r\n\
--XX--\r\n";
        let s = parse_summary(raw).unwrap();
        assert!(s.has_html);
        assert_eq!(s.preview, "See attached");
        assert_eq!(s.attachments.len(), 1);
        let att = &s.attachments[0];
        assert_eq!(att.file_name, "invoicefdp.exe");
        assert_eq!(att.mime_type, "application/pdf");
        assert_eq!(att.size, 5);
    }

    #[test]
    fn keeps_every_reference_in_a_thread() {
        let raw = b"From: a@b.example\r\nSubject: Re: x\r\nIn-Reply-To: <c@x>\r\nReferences: <a@x> <b@x>\r\n <c@x>\r\n\r\nbody";
        let s = parse_summary(raw).unwrap();
        assert_eq!(s.in_reply_to.as_deref(), Some("c@x"));
        assert_eq!(s.references, vec!["a@x", "b@x", "c@x"]);
    }

    #[test]
    fn strips_direction_tricks_from_sender_names() {
        let raw = "From: \"Bank\u{202E}lieve\" <x@evil.example>\r\nSubject: hi\r\n\r\nx".as_bytes();
        let from = parse_summary(raw).unwrap().from.unwrap();
        assert_eq!(from.name.as_deref(), Some("Banklieve"));
    }

    #[test]
    fn refuses_empty_and_oversized_input() {
        assert_eq!(parse_summary(b""), Err(ParseError::Empty));
        let big = vec![b'a'; MAX_MESSAGE_BYTES + 1];
        assert!(matches!(
            parse_summary(&big),
            Err(ParseError::TooLarge { .. })
        ));
    }

    #[test]
    fn safe_file_name_handles_edge_cases() {
        assert_eq!(safe_file_name(""), FALLBACK_FILE_NAME);
        assert_eq!(safe_file_name(".."), FALLBACK_FILE_NAME);
        assert_eq!(safe_file_name("C:\\Users\\x\\report.pdf"), "report.pdf");
        assert_eq!(safe_file_name(".bashrc"), "bashrc");
        // Names Windows can't store, or stores somewhere unexpected.
        assert_eq!(safe_file_name("Scan 12:30.pdf"), "Scan 12_30.pdf");
        assert_eq!(safe_file_name("what?*.txt"), "what__.txt");
        assert_eq!(safe_file_name("notes. "), "notes");
        assert_eq!(safe_file_name("CON.txt"), "_CON.txt");
        assert_eq!(safe_file_name("com1"), "_com1");
        assert_eq!(safe_file_name("COM10.log"), "COM10.log");
        assert_eq!(safe_file_name("console.log"), "console.log");
        assert_eq!(safe_file_name("a\u{0000}b.txt"), "ab.txt");
    }

    #[test]
    fn preview_is_cut_on_character_boundaries() {
        let text = "\u{1F600}".repeat(500);
        let preview = make_preview(&text, 10);
        assert_eq!(preview.chars().count(), 10);
    }

    proptest! {
        #[test]
        fn never_panics_on_arbitrary_bytes(raw in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let _ = parse_summary(&raw);
        }

        #[test]
        fn display_safe_output_has_no_unsafe_chars(text in "\\PC{0,200}|[\u{0}-\u{1F}\u{202A}-\u{202E}a-z]{0,50}") {
            prop_assert!(!display_safe(&text).chars().any(is_unsafe_char));
        }

        #[test]
        fn safe_file_name_is_never_empty_or_a_path(name in ".{0,100}") {
            let safe = safe_file_name(&name);
            prop_assert!(!safe.is_empty());
            prop_assert!(!safe.contains('/') && !safe.contains('\\'));
            prop_assert!(!safe.starts_with('.'));
        }

        #[test]
        fn preview_respects_the_limit(text in ".{0,1000}", max in 0usize..200) {
            prop_assert!(make_preview(&text, max).chars().count() <= max);
        }
    }

    fn with_winmail(dat: &[u8], body: &str) -> Vec<u8> {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(dat);
        format!(
            "From: a@outlook.example\r\nSubject: Report\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"B\"\r\n\r\n--B\r\nContent-Type: text/plain\r\n\r\n{body}\r\n--B\r\nContent-Type: application/ms-tnef; name=\"winmail.dat\"\r\nContent-Disposition: attachment; filename=\"winmail.dat\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{encoded}\r\n--B--\r\n"
        )
        .into_bytes()
    }

    #[test]
    fn files_inside_winmail_dat_are_listed_as_attachments() {
        use mailcraft_tnef::test_writer::Writer;
        let pdf = b"%PDF-1.4 a small report".to_vec();
        let dat = Writer::new()
            .file("Q3 report.pdf", "application/pdf", &pdf)
            .file("figures.csv", "", b"a,b\n1,2\n")
            .finish();
        let raw = with_winmail(&dat, "See attached.");
        let body = parse_body(&raw).unwrap();
        let names: Vec<&str> = body
            .attachments
            .iter()
            .map(|a| a.file_name.as_str())
            .collect();
        assert_eq!(names, ["Q3 report.pdf", "figures.csv"]);
        assert_eq!(body.attachments[0].mime_type, "application/pdf");
        assert_eq!(body.attachments[1].mime_type, "application/octet-stream");
        assert_eq!(body.attachments[0].size, pdf.len() as u64);
        let (info, bytes) = attachment_bytes(&raw, 0).unwrap().unwrap();
        assert_eq!(info.file_name, "Q3 report.pdf");
        assert_eq!(bytes, pdf);
        assert_eq!(attachment_bytes(&raw, 1).unwrap().unwrap().1, b"a,b\n1,2\n");
        assert!(attachment_bytes(&raw, 2).unwrap().is_none());
        assert_eq!(parse_summary(&raw).unwrap().attachments.len(), 2);
    }

    #[test]
    fn a_winmail_dat_that_is_not_all_files_stays_listed() {
        use mailcraft_tnef::test_writer::{Prop, Writer};
        let dat = Writer::new()
            .file("photo.png", "image/png", b"\x89PNG\r\n\x1a\nxx")
            .attachment(2, &[Prop::Int32(0x3705, 6)])
            .finish();
        let body = parse_body(&with_winmail(&dat, "x")).unwrap();
        let names: Vec<&str> = body
            .attachments
            .iter()
            .map(|a| a.file_name.as_str())
            .collect();
        assert_eq!(names, ["photo.png", "winmail.dat"]);
        // Not TNEF at all: listed as it is.
        let body = parse_body(&with_winmail(b"not tnef", "x")).unwrap();
        assert_eq!(body.attachments[0].file_name, "winmail.dat");
    }

    #[test]
    fn the_text_of_a_winmail_dat_is_the_body_when_there_is_no_other() {
        use mailcraft_tnef::test_writer::{Prop, Writer};
        let dat = Writer::new()
            .message_props(&[Prop::Unicode(0x1000, "Text only in TNEF".into())])
            .finish();
        let body = parse_body(&with_winmail(&dat, "")).unwrap();
        assert_eq!(body.text.as_deref(), Some("Text only in TNEF"));
        let body = parse_body(&with_winmail(&dat, "Own text")).unwrap();
        assert_eq!(body.text.as_deref().map(str::trim), Some("Own text"));
    }

    #[test]
    fn legacy_charsets_are_decoded() {
        let cases: &[(&str, &[u8], &str)] = &[
            (
                "windows-1251",
                b"\xcf\xf0\xe8\xe2\xe5\xf2",
                "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}",
            ),
            (
                "koi8-r",
                b"\xf0\xd2\xc9\xd7\xc5\xd4",
                "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}",
            ),
            ("iso-8859-2", b"\xbe\xb9\xe8", "\u{17e}\u{161}\u{10d}"),
            ("windows-1250", b"\x9e\x9a\xe8", "\u{17e}\u{161}\u{10d}"),
            ("shift_jis", b"\x93\xfa\x96\x7b", "\u{65e5}\u{672c}"),
            ("iso-2022-jp", b"\x1b$BF|K\\\x1b(B", "\u{65e5}\u{672c}"),
            ("gb2312", b"\xd6\xd0\xce\xc4", "\u{4e2d}\u{6587}"),
            ("gbk", b"\xd6\xd0\xce\xc4", "\u{4e2d}\u{6587}"),
            ("big5", b"\xa4\xa4\xa4\xe5", "\u{4e2d}\u{6587}"),
            ("euc-kr", b"\xc7\xd1\xb1\xb9", "\u{d55c}\u{ad6d}"),
            ("euc-jp", b"\xc6\xfc\xcb\xdc", "\u{65e5}\u{672c}"),
            (
                "windows-1256",
                b"\xe3\xd1\xcd\xc8\xc7",
                "\u{645}\u{631}\u{62d}\u{628}\u{627}",
            ),
        ];
        for (charset, bytes, text) in cases {
            let mut raw = format!(
                "From: a@b.example\r\nSubject: =?{charset}?B?{}?=\r\nContent-Type: text/plain; charset={charset}\r\n\r\n",
                {
                    use base64::Engine as _;
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                }
            )
            .into_bytes();
            raw.extend_from_slice(bytes);
            let summary = parse_summary(&raw).unwrap();
            assert_eq!(summary.subject, *text, "subject in {charset}");
            let body = parse_body(&raw).unwrap();
            assert_eq!(
                body.text.as_deref().map(str::trim),
                Some(*text),
                "body in {charset}"
            );
        }
    }

    #[test]
    fn list_headers_mark_mail_as_bulk() {
        let news = b"From: editor@paper.example\r\nTo: me@x.example\r\nSubject: News\r\nList-Unsubscribe: <mailto:u@paper.example>\r\n\r\nHi";
        assert!(parse_summary(news).unwrap().bulk);
        let person = b"From: peter@client.example\r\nTo: me@x.example\r\nSubject: Quote\r\n\r\nHi";
        assert!(!parse_summary(person).unwrap().bulk);
    }

    #[test]
    fn importance_comes_from_the_headers() {
        use crate::importance::Importance;
        let urgent = b"From: a@x.example\r\nSubject: Now\r\nX-Priority: 1 (Highest)\r\n\r\nHi";
        assert_eq!(parse_summary(urgent).unwrap().importance, Importance::High);
        let low = b"From: a@x.example\r\nSubject: Later\r\nImportance: low\r\n\r\nHi";
        assert_eq!(parse_summary(low).unwrap().importance, Importance::Low);
        assert_eq!(parse_summary(PLAIN).unwrap().importance, Importance::Normal);
    }
}
