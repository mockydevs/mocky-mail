//! Focused Inbox: which messages are from people (Focused) and which are
//! newsletters, notifications and other automated mail (Other).
//!
//! Decided, in order, by the user's own rule for the sender, then by
//! whether the user has written to the sender, then by the message's
//! mailing-list headers and the sender's address. Ideas adapted from
//! Overlook's `focus.rs` (MIT License, Copyright (c) 2026 Last Refuge).

use serde::{Deserialize, Serialize};

/// Where a message goes in a Focused Inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Focus {
    /// From people.
    Focused,
    /// Newsletters, notifications, automated mail.
    Other,
}

impl Focus {
    /// The stored and wire form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Focused => "focused",
            Self::Other => "other",
        }
    }

    /// From the stored form.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "focused" => Some(Self::Focused),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

/// Starts of the local part (before `@`) that automated senders use.
/// Kept in step with the SQL in `mail-store`'s list query.
pub const AUTOMATED_PREFIXES: &[&str] = &[
    "noreply",
    "no-reply",
    "no_reply",
    "donotreply",
    "do-not-reply",
    "do_not_reply",
    "notification",
    "newsletter",
    "mailer-daemon",
    "postmaster",
    "bounce",
    "marketing",
    "news@",
    "alerts@",
    "alert@",
    "updates@",
    "digest@",
];

/// Whether an address looks like a machine's rather than a person's.
#[must_use]
pub fn automated_address(address: &str) -> bool {
    let address = address.trim().to_ascii_lowercase();
    AUTOMATED_PREFIXES.iter().any(|p| address.starts_with(p))
}

/// Whether headers mark a message as bulk or automated mail.
#[must_use]
pub fn bulk_headers(
    list_unsubscribe: bool,
    list_id: bool,
    precedence: Option<&str>,
    auto_submitted: Option<&str>,
) -> bool {
    let precedence = precedence.map(|p| p.trim().to_ascii_lowercase());
    let auto = auto_submitted.map(|a| a.trim().to_ascii_lowercase());
    list_unsubscribe
        || list_id
        || matches!(precedence.as_deref(), Some("bulk" | "list" | "junk"))
        || auto.is_some_and(|a| !a.is_empty() && a != "no")
}

/// Where a message goes.
#[must_use]
pub fn classify(rule: Option<Focus>, written_to: bool, bulk: bool, sender: &str) -> Focus {
    if let Some(rule) = rule {
        return rule;
    }
    if written_to {
        return Focus::Focused;
    }
    if bulk || automated_address(sender) {
        Focus::Other
    } else {
        Focus::Focused
    }
}

/// The rule key for a sender: the address, or `@domain` for a whole domain.
#[must_use]
pub fn rule_key(sender: &str, whole_domain: bool) -> Option<String> {
    let sender = sender.trim().to_ascii_lowercase();
    let (_, domain) = sender.rsplit_once('@')?;
    if domain.is_empty() {
        return None;
    }
    Some(if whole_domain {
        format!("@{domain}")
    } else {
        sender
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn people_are_focused_and_machines_are_other() {
        assert_eq!(
            classify(None, false, false, "peter@client.example"),
            Focus::Focused
        );
        assert_eq!(
            classify(None, false, true, "peter@client.example"),
            Focus::Other
        );
        assert_eq!(
            classify(None, false, false, "noreply@bank.example"),
            Focus::Other
        );
        assert_eq!(
            classify(None, false, false, "notifications@github.com"),
            Focus::Other
        );
        assert_eq!(
            classify(None, false, false, "News@paper.example"),
            Focus::Other
        );
        // Someone you write to stays Focused even if they send through a list.
        assert_eq!(
            classify(None, true, true, "amina@studio.example"),
            Focus::Focused
        );
        // Your rule wins over everything.
        assert_eq!(
            classify(Some(Focus::Focused), false, true, "noreply@x.example"),
            Focus::Focused
        );
        assert_eq!(
            classify(Some(Focus::Other), true, false, "peter@client.example"),
            Focus::Other
        );
    }

    #[test]
    fn list_headers_mark_bulk_mail() {
        assert!(bulk_headers(true, false, None, None));
        assert!(bulk_headers(false, true, None, None));
        assert!(bulk_headers(false, false, Some(" Bulk"), None));
        assert!(bulk_headers(false, false, None, Some("auto-generated")));
        assert!(!bulk_headers(false, false, None, Some("no")));
        assert!(!bulk_headers(false, false, Some("first-class"), None));
    }

    #[test]
    fn rule_keys_are_addresses_or_domains() {
        assert_eq!(
            rule_key("Peter@Client.Example", false).as_deref(),
            Some("peter@client.example")
        );
        assert_eq!(
            rule_key("peter@client.example", true).as_deref(),
            Some("@client.example")
        );
        assert_eq!(rule_key("not an address", false), None);
        assert_eq!(rule_key("x@", true), None);
    }
}
