//! Read receipts: message disposition notifications (MDN, RFC 8098).
//!
//! A sender asks for one with `Disposition-Notification-To`. The answer is
//! a `multipart/report; report-type=disposition-notification` message: a
//! short note for people, then a `message/disposition-notification` part
//! with the fields below for software.

use std::fmt::Write as _;

use crate::compose::parse_address_list;
use crate::message::display_safe;

/// A read receipt a received message asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptRequest {
    /// Where the receipt goes (the one address in the header).
    pub address: String,
    /// The message's `Return-Path` is at the same domain, or there is none.
    /// When it differs, RFC 8098 says not to answer without asking.
    pub return_path_matches: bool,
}

fn domain(address: &str) -> Option<String> {
    address
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim().trim_end_matches('>').to_ascii_lowercase())
}

/// The read receipt a message asks for: its `Disposition-Notification-To`
/// names exactly one valid address. Receipts are never asked for receipts,
/// so a message that is itself one asks for nothing.
#[must_use]
pub fn requested(raw: &[u8]) -> Option<ReceiptRequest> {
    let message = crate::message::parse(raw).ok()?;
    let header = message.header_raw("Disposition-Notification-To")?;
    let unfolded = header.replace(['\r', '\n'], " ");
    let list = parse_address_list(unfolded.trim()).ok()?;
    let [only] = list.as_slice() else {
        return None;
    };
    let is_report = message.header_raw("Content-Type").is_some_and(|ct| {
        let ct = ct.to_ascii_lowercase();
        ct.contains("multipart/report") && ct.contains("disposition-notification")
    });
    if is_report {
        return None;
    }
    let return_path_matches = match message.header_raw("Return-Path") {
        None => true,
        Some(path) => {
            let path = path.trim().trim_start_matches('<').trim_end_matches('>');
            !path.is_empty() && domain(path) == domain(&only.address)
        }
    };
    Some(ReceiptRequest {
        address: only.address.clone(),
        return_path_matches,
    })
}

/// What a read receipt says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt<'a> {
    /// The address the message was delivered to (the account's).
    pub final_recipient: &'a str,
    /// The original's Message-ID, without angle brackets.
    pub original_message_id: Option<&'a str>,
    /// The original's subject.
    pub subject: &'a str,
    /// Sent because the user chose to send receipts always (true), or
    /// because they said yes to this one (false).
    pub automatic: bool,
}

/// The receipt's subject: `Read: ` and the original's.
#[must_use]
pub fn subject(original: &str) -> String {
    format!("Read: {}", display_safe(original))
}

/// The note for people reading the receipt.
#[must_use]
pub fn note(receipt: &Receipt<'_>) -> String {
    let subject = display_safe(receipt.subject);
    let to = display_safe(receipt.final_recipient);
    let subject = if subject.is_empty() {
        "(no subject)".to_owned()
    } else {
        subject
    };
    format!(
        "This is a read receipt for the message you sent to {to} with the subject \"{subject}\".\r\n\r\nIt was opened on the recipient's screen. This doesn't mean it was read or understood.\r\n"
    )
}

/// The `message/disposition-notification` part (RFC 8098 section 3.1):
/// "displayed", by the user's action, sent by hand or by their choice.
#[must_use]
pub fn disposition_fields(receipt: &Receipt<'_>) -> String {
    let recipient = display_safe(receipt.final_recipient);
    let domain = domain(&recipient).unwrap_or_else(|| "localhost".to_owned());
    let mut out =
        format!("Reporting-UA: {domain}; Mocky Mail\r\nFinal-Recipient: rfc822;{recipient}\r\n");
    if let Some(id) = receipt
        .original_message_id
        .map(|id| display_safe(id.trim().trim_start_matches('<').trim_end_matches('>')))
        .filter(|id| !id.is_empty() && !id.contains(char::is_whitespace))
    {
        let _ = write!(out, "Original-Message-ID: <{id}>\r\n");
    }
    let mode = if receipt.automatic {
        "MDN-sent-automatically"
    } else {
        "MDN-sent-manually"
    };
    let _ = write!(out, "Disposition: manual-action/{mode}; displayed\r\n");
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn message(headers: &str) -> Vec<u8> {
        format!("From: Peter <peter@client.example>\r\nTo: me@work.example\r\nSubject: Quote\r\n{headers}\r\nHello\r\n").into_bytes()
    }

    #[test]
    fn reads_a_single_address_request() {
        let r = requested(&message(
            "Disposition-Notification-To: \"Peter M\" <peter@client.example>\r\nReturn-Path: <bounce@CLIENT.example>\r\n",
        ))
        .unwrap();
        assert_eq!(r.address, "peter@client.example");
        assert!(r.return_path_matches);
        let folded = requested(&message(
            "Disposition-Notification-To:\r\n peter@client.example\r\n",
        ))
        .unwrap();
        assert_eq!(folded.address, "peter@client.example");
        assert!(folded.return_path_matches);
    }

    #[test]
    fn ignores_odd_requests() {
        assert_eq!(requested(&message("")), None);
        assert_eq!(
            requested(&message(
                "Disposition-Notification-To: a@x.example, b@x.example\r\n"
            )),
            None
        );
        assert_eq!(
            requested(&message("Disposition-Notification-To: not an address\r\n")),
            None
        );
        // A receipt never asks for another.
        assert_eq!(
            requested(&message(
                "Disposition-Notification-To: a@x.example\r\nContent-Type: multipart/report; report-type=disposition-notification; boundary=b\r\n"
            )),
            None
        );
        let elsewhere = requested(&message(
            "Disposition-Notification-To: a@x.example\r\nReturn-Path: <bulk@mailer.example>\r\n",
        ))
        .unwrap();
        assert!(!elsewhere.return_path_matches);
        let bounce = requested(&message(
            "Disposition-Notification-To: a@x.example\r\nReturn-Path: <>\r\n",
        ))
        .unwrap();
        assert!(!bounce.return_path_matches);
    }

    #[test]
    fn writes_the_report_fields() {
        let receipt = Receipt {
            final_recipient: "me@work.example",
            original_message_id: Some("<q-1@client.example>"),
            subject: "Quote\r\nBcc: x@evil.example",
            automatic: false,
        };
        assert_eq!(
            disposition_fields(&receipt),
            "Reporting-UA: work.example; Mocky Mail\r\nFinal-Recipient: rfc822;me@work.example\r\nOriginal-Message-ID: <q-1@client.example>\r\nDisposition: manual-action/MDN-sent-manually; displayed\r\n"
        );
        let auto = Receipt {
            automatic: true,
            original_message_id: None,
            ..receipt.clone()
        };
        let fields = disposition_fields(&auto);
        assert!(
            fields.ends_with("Disposition: manual-action/MDN-sent-automatically; displayed\r\n")
        );
        assert!(!fields.contains("Original-Message-ID"));
        assert!(note(&receipt).contains("me@work.example"));
        assert!(!subject(receipt.subject).contains('\n'));
        assert!(subject("Quote").starts_with("Read: Quote"));
    }
}
