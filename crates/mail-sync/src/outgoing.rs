//! Building the RFC 5322 bytes for a message the user wrote.

use base64::Engine as _;
use lettre::message::header::{ContentType, InReplyTo, References};
use lettre::message::{Attachment, Mailbox as LettreMailbox, MultiPart, SinglePart};
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
    /// Message-ID being replied to (no angle brackets).
    pub in_reply_to: Option<String>,
    /// Thread Message-IDs (no angle brackets).
    pub references: Vec<String>,
    /// Attachments.
    pub attachments: Vec<OutgoingAttachment>,
}

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

    let text = SinglePart::plain(message.body_text.clone());
    let email = if message.attachments.is_empty() {
        builder.singlepart(text)
    } else {
        let mut parts = MultiPart::mixed().singlepart(text);
        for a in &message.attachments {
            let content_type = ContentType::parse(&a.mime_type)
                .or_else(|_| ContentType::parse("application/octet-stream"))
                .map_err(|e| SyncError::Invalid(e.to_string()))?;
            parts = parts.singlepart(
                Attachment::new(mail_core::message::safe_file_name(&a.file_name))
                    .body(a.data.clone(), content_type),
            );
        }
        builder.multipart(parts)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mb(address: &str, name: Option<&str>) -> Mailbox {
        Mailbox {
            name: name.map(str::to_owned),
            address: address.to_owned(),
        }
    }

    fn base() -> OutgoingMessage {
        OutgoingMessage {
            from: mb("me@work.example", Some("Don")),
            to: vec![mb("peter@client.example", Some("Peter Mwangi"))],
            cc: vec![],
            bcc: vec![mb("secret@hidden.example", None)],
            subject: "Invoice INV-0144".into(),
            body_text: "Hi Peter,\n\nInvoice attached.".into(),
            in_reply_to: Some("q-1@client.example".into()),
            references: vec!["q-0@client.example".into(), "q-1@client.example".into()],
            attachments: vec![OutgoingAttachment {
                file_name: "INV-0144.pdf".into(),
                mime_type: "application/pdf".into(),
                data: b"%PDF-1.4".to_vec(),
            }],
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
}
