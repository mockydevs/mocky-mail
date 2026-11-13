//! Rules for writing mail: address lists, reply and forward subjects,
//! who a reply goes to, quoting, and threading headers.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::message::{Mailbox, display_safe};

/// Longest address allowed (RFC 5321 path limit).
const MAX_ADDRESS_LEN: usize = 254;
/// Longest local part allowed (RFC 5321).
const MAX_LOCAL_LEN: usize = 64;
/// Most recipients accepted in one field.
pub const MAX_RECIPIENTS: usize = 100;

/// An address the user typed that cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize)]
#[error("\"{input}\" is not a valid email address")]
pub struct InvalidAddress {
    /// The offending entry, as typed.
    pub input: String,
}

/// Problems with a recipient list.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AddressListError {
    /// One entry is not an address.
    #[error(transparent)]
    Invalid(InvalidAddress),
    /// Too many recipients.
    #[error("too many recipients ({count}); the limit is {MAX_RECIPIENTS}")]
    TooMany {
        /// How many were given.
        count: usize,
    },
}

/// Whether `address` is a plausible mailbox address: one `@`, sensible
/// lengths, a dotted domain, and no spaces, quotes or angle brackets.
#[must_use]
pub fn is_valid_address(address: &str) -> bool {
    if address.len() > MAX_ADDRESS_LEN {
        return false;
    }
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    let bad = |c: char| c.is_whitespace() || c.is_control() || "<>\"(),;:\\[]@".contains(c);
    !local.is_empty()
        && local.len() <= MAX_LOCAL_LEN
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && !local.chars().any(bad)
        && domain.contains('.')
        && !domain.starts_with(['.', '-'])
        && !domain.ends_with(['.', '-'])
        && !domain.contains("..")
        && domain
            .chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == '-')
}

fn split_entries(input: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut in_angle = false;
    for c in input.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            }
            '<' if !in_quotes => {
                in_angle = true;
                current.push(c);
            }
            '>' if !in_quotes => {
                in_angle = false;
                current.push(c);
            }
            ',' | ';' if !in_quotes && !in_angle => {
                entries.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    entries.push(current);
    entries
        .into_iter()
        .map(|e| e.trim().to_owned())
        .filter(|e| !e.is_empty())
        .collect()
}

fn parse_entry(entry: &str) -> Result<Mailbox, InvalidAddress> {
    let invalid = || InvalidAddress {
        input: entry.to_owned(),
    };
    let (name, address) = match (entry.rfind('<'), entry.ends_with('>')) {
        (Some(open), true) => {
            let name = entry
                .get(..open)
                .unwrap_or_default()
                .trim()
                .trim_matches('"')
                .trim();
            let inner = entry
                .get(open.checked_add(1).ok_or_else(invalid)?..entry.len().saturating_sub(1))
                .ok_or_else(invalid)?;
            (name, inner.trim())
        }
        (None, false) => ("", entry),
        _ => return Err(invalid()),
    };
    if !is_valid_address(address) {
        return Err(invalid());
    }
    let name = display_safe(name);
    Ok(Mailbox {
        name: (!name.is_empty()).then_some(name),
        address: address.to_owned(),
    })
}

/// Parse what the user typed in To, Cc or Bcc. Entries are separated by
/// commas or semicolons; each is `address` or `Name <address>`.
/// Duplicates (case-insensitive) are dropped.
///
/// # Errors
/// The first invalid entry, or too many recipients.
pub fn parse_address_list(input: &str) -> Result<Vec<Mailbox>, AddressListError> {
    let mut out: Vec<Mailbox> = Vec::new();
    for entry in split_entries(input) {
        let mailbox = parse_entry(&entry).map_err(AddressListError::Invalid)?;
        if !out
            .iter()
            .any(|m| m.address.eq_ignore_ascii_case(&mailbox.address))
        {
            out.push(mailbox);
        }
    }
    if out.len() > MAX_RECIPIENTS {
        return Err(AddressListError::TooMany { count: out.len() });
    }
    Ok(out)
}

fn strip_prefixes(subject: &str) -> &str {
    let mut rest = subject.trim();
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(len) = ["re:", "fwd:", "fw:", "aw:", "sv:", "wg:"]
            .iter()
            .find(|p| lower.starts_with(**p))
            .map(|p| p.len())
        else {
            return rest;
        };
        rest = rest.get(len..).unwrap_or_default().trim_start();
    }
}

/// Subject for a reply: `Re: ` plus the original without stacked prefixes.
#[must_use]
pub fn reply_subject(subject: &str) -> String {
    format!("Re: {}", strip_prefixes(subject))
}

/// Subject for a forward: `Fwd: ` plus the original without stacked prefixes.
#[must_use]
pub fn forward_subject(subject: &str) -> String {
    format!("Fwd: {}", strip_prefixes(subject))
}

/// The message being replied to, as far as recipients are concerned.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReplySource {
    /// Original sender.
    pub from: Option<Mailbox>,
    /// Original Reply-To, which takes precedence over From.
    pub reply_to: Vec<Mailbox>,
    /// Original To.
    pub to: Vec<Mailbox>,
    /// Original Cc.
    pub cc: Vec<Mailbox>,
}

/// Recipients for a reply.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ReplyRecipients {
    /// To.
    pub to: Vec<Mailbox>,
    /// Cc.
    pub cc: Vec<Mailbox>,
}

/// Work out who a reply goes to. `my_addresses` are the user's own
/// addresses, which are never included. With `reply_all`, the original To
/// and Cc are added to Cc.
#[must_use]
pub fn reply_recipients(
    source: &ReplySource,
    my_addresses: &[String],
    reply_all: bool,
) -> ReplyRecipients {
    let is_me = |m: &Mailbox| {
        my_addresses
            .iter()
            .any(|a| a.eq_ignore_ascii_case(&m.address))
    };
    let mut seen: Vec<String> = Vec::new();
    let mut take = |list: &mut Vec<Mailbox>, m: &Mailbox| {
        let key = m.address.to_ascii_lowercase();
        if !is_me(m) && !seen.contains(&key) {
            seen.push(key);
            list.push(m.clone());
        }
    };

    let mut to = Vec::new();
    let primary: Vec<Mailbox> = if source.reply_to.is_empty() {
        source.from.iter().cloned().collect()
    } else {
        source.reply_to.clone()
    };
    for m in &primary {
        take(&mut to, m);
    }

    let mut cc = Vec::new();
    if reply_all {
        for m in source.to.iter().chain(&source.cc) {
            take(&mut cc, m);
        }
    }
    // Replying to your own sent message: send to its original recipients.
    if to.is_empty() {
        for m in &source.to {
            take(&mut to, m);
        }
    }
    ReplyRecipients { to, cc }
}

/// Quote a plain-text body for a reply.
#[must_use]
pub fn quote_for_reply(body: &str, author: &str, date: &str) -> String {
    let mut out = format!("\n\nOn {date}, {author} wrote:\n");
    for line in body.lines() {
        if line.is_empty() {
            out.push_str(">\n");
        } else {
            out.push_str("> ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The header block shown above a forwarded message.
#[must_use]
pub fn forward_header(from: &str, date: &str, subject: &str, to: &str) -> String {
    format!(
        "\n\n---------- Forwarded message ----------\nFrom: {from}\nDate: {date}\nSubject: {subject}\nTo: {to}\n\n"
    )
}

/// `In-Reply-To` and `References` values for a reply (RFC 5322 §3.6.4).
/// Message IDs are given without angle brackets.
#[must_use]
pub fn thread_headers(
    parent_id: Option<&str>,
    parent_references: &[String],
) -> (Option<String>, Vec<String>) {
    let Some(parent) = parent_id.filter(|id| !id.is_empty()) else {
        return (None, parent_references.to_vec());
    };
    let mut references = parent_references.to_vec();
    if !references.iter().any(|r| r == parent) {
        references.push(parent.to_owned());
    }
    (Some(parent.to_owned()), references)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn mb(address: &str) -> Mailbox {
        Mailbox {
            name: None,
            address: address.into(),
        }
    }

    #[test]
    fn validates_addresses() {
        for ok in [
            "a@b.co",
            "first.last+tag@mail.example.co.ke",
            "x@xn--bcher-kva.example",
        ] {
            assert!(is_valid_address(ok), "{ok}");
        }
        for bad in [
            "",
            "a",
            "a@b",
            "@b.co",
            "a@.co",
            "a@b..co",
            "a b@c.co",
            "a@b.co>",
            ".a@b.co",
            "a..b@c.co",
            "a@b@c.co",
            "a@-b.co",
        ] {
            assert!(!is_valid_address(bad), "{bad}");
        }
    }

    #[test]
    fn parses_mixed_address_lists() {
        let list = parse_address_list(
            r#"Peter Mwangi <peter@client.example>; amina@studio.example, "Kamau, Wanjiru" <wanjiru@fitout.example>, PETER@client.example"#,
        )
        .unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].name.as_deref(), Some("Peter Mwangi"));
        assert_eq!(list[1].name, None);
        assert_eq!(list[2].name.as_deref(), Some("Kamau, Wanjiru"));
    }

    #[test]
    fn reports_the_bad_entry() {
        let err = parse_address_list("ok@a.co, not-an-address").unwrap_err();
        assert_eq!(
            err,
            AddressListError::Invalid(InvalidAddress {
                input: "not-an-address".into()
            })
        );
    }

    #[test]
    fn empty_input_is_an_empty_list() {
        assert_eq!(parse_address_list(" , ;").unwrap(), vec![]);
    }

    #[test]
    fn limits_recipient_count() {
        let many = (0..=MAX_RECIPIENTS)
            .map(|i| format!("u{i}@a.co"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(matches!(
            parse_address_list(&many),
            Err(AddressListError::TooMany { .. })
        ));
    }

    #[test]
    fn subjects_do_not_stack_prefixes() {
        assert_eq!(reply_subject("Invoice"), "Re: Invoice");
        assert_eq!(reply_subject("RE: Re: Fwd: Invoice"), "Re: Invoice");
        assert_eq!(forward_subject("Re: Invoice"), "Fwd: Invoice");
        assert_eq!(reply_subject(""), "Re: ");
    }

    #[test]
    fn reply_goes_to_sender_and_reply_all_excludes_me() {
        let source = ReplySource {
            from: Some(mb("wanjiru@fitout.example")),
            reply_to: vec![],
            to: vec![mb("me@work.example"), mb("amina@studio.example")],
            cc: vec![mb("Wanjiru@Fitout.example"), mb("peter@client.example")],
        };
        let me = vec!["ME@work.example".to_owned()];
        let single = reply_recipients(&source, &me, false);
        assert_eq!(single.to, vec![mb("wanjiru@fitout.example")]);
        assert!(single.cc.is_empty());

        let all = reply_recipients(&source, &me, true);
        assert_eq!(all.to, vec![mb("wanjiru@fitout.example")]);
        assert_eq!(
            all.cc,
            vec![mb("amina@studio.example"), mb("peter@client.example")]
        );
    }

    #[test]
    fn reply_to_header_wins() {
        let source = ReplySource {
            from: Some(mb("noreply@shop.example")),
            reply_to: vec![mb("support@shop.example")],
            ..ReplySource::default()
        };
        assert_eq!(
            reply_recipients(&source, &[], false).to,
            vec![mb("support@shop.example")]
        );
    }

    #[test]
    fn replying_to_my_own_sent_message_goes_to_its_recipients() {
        let source = ReplySource {
            from: Some(mb("me@work.example")),
            to: vec![mb("peter@client.example")],
            ..ReplySource::default()
        };
        let r = reply_recipients(&source, &["me@work.example".to_owned()], false);
        assert_eq!(r.to, vec![mb("peter@client.example")]);
    }

    #[test]
    fn quotes_every_line() {
        let q = quote_for_reply("Hi\n\nThanks", "Peter", "Tue 11 Nov");
        assert_eq!(q, "\n\nOn Tue 11 Nov, Peter wrote:\n> Hi\n>\n> Thanks\n");
    }

    #[test]
    fn threading_headers_append_parent_once() {
        let refs = vec!["a@x".to_owned()];
        assert_eq!(
            thread_headers(Some("b@x"), &refs),
            (Some("b@x".into()), vec!["a@x".into(), "b@x".into()])
        );
        assert_eq!(
            thread_headers(Some("a@x"), &refs),
            (Some("a@x".into()), vec!["a@x".into()])
        );
        assert_eq!(thread_headers(None, &refs), (None, refs.clone()));
    }

    proptest! {
        #[test]
        fn parsing_never_panics_and_output_is_valid(input in ".{0,300}") {
            if let Ok(list) = parse_address_list(&input) {
                for m in list {
                    prop_assert!(is_valid_address(&m.address));
                }
            }
        }

        #[test]
        fn reply_subject_is_idempotent(subject in "[ -~]{0,80}") {
            let once = reply_subject(&subject);
            prop_assert_eq!(reply_subject(&once), once);
        }
    }
}
