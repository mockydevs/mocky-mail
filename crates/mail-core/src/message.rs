//! Turning a raw RFC 5322 message into what the message list and reading
//! pane need.
//!
//! Every string that came from the sender and will be shown to the user
//! passes through [`display_safe`], which removes control characters and the
//! invisible Unicode direction marks used to disguise file names and
//! senders.

use mail_parser::{Address, MessageParser, MimeHeaders, PartType};
use serde::Serialize;
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mailbox {
    /// Display name, if any.
    pub name: Option<String>,
    /// Email address.
    pub address: String,
}

/// An attachment, without its contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttachmentInfo {
    /// Safe file name to show and to suggest when saving.
    pub file_name: String,
    /// MIME type such as `application/pdf`.
    pub mime_type: String,
    /// Decoded size in bytes.
    pub size: u64,
}

/// Everything the list and the reading pane header need.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessageSummary {
    /// The Message-ID header, used to find duplicates in Sent.
    pub message_id: Option<String>,
    /// Subject, or an empty string.
    pub subject: String,
    /// First sender.
    pub from: Option<Mailbox>,
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
}

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

/// A file name that is safe to show and to save: no directories, no hidden
/// direction tricks, not empty, and not "." or "..".
#[must_use]
pub fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned = display_safe(base);
    let cleaned = cleaned.trim_start_matches('.').trim();
    if cleaned.is_empty() {
        FALLBACK_FILE_NAME.to_owned()
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

/// Parse a raw message into a [`MessageSummary`].
///
/// # Errors
/// Returns [`ParseError`] for empty, oversized or unparseable input. Never
/// panics, whatever the input.
pub fn parse_summary(raw: &[u8]) -> Result<MessageSummary, ParseError> {
    if raw.is_empty() {
        return Err(ParseError::Empty);
    }
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(ParseError::TooLarge {
            size: raw.len(),
            max: MAX_MESSAGE_BYTES,
        });
    }
    let message = MessageParser::default()
        .parse(raw)
        .ok_or(ParseError::Malformed)?;

    let attachments = message
        .attachments()
        .map(|part| AttachmentInfo {
            file_name: safe_file_name(part.attachment_name().unwrap_or_default()),
            mime_type: part.content_type().map_or_else(
                || "application/octet-stream".to_owned(),
                |ct| match ct.subtype() {
                    Some(sub) => format!("{}/{}", ct.ctype(), sub).to_ascii_lowercase(),
                    None => ct.ctype().to_ascii_lowercase(),
                },
            ),
            size: u64::try_from(part.contents().len()).unwrap_or(u64::MAX),
        })
        .collect();

    let has_html = message
        .html_body
        .iter()
        .filter_map(|&index| message.parts.get(index as usize))
        .any(|part| matches!(part.body, PartType::Html(_)));

    Ok(MessageSummary {
        message_id: message.message_id().map(display_safe),
        subject: message.subject().map(display_safe).unwrap_or_default(),
        from: mailboxes(message.from()).into_iter().next(),
        to: mailboxes(message.to()),
        cc: mailboxes(message.cc()),
        date: message.date().map(mail_parser::DateTime::to_timestamp),
        preview: message
            .body_text(0)
            .map(|text| make_preview(&text, PREVIEW_CHARS))
            .unwrap_or_default(),
        has_html,
        attachments,
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
}
