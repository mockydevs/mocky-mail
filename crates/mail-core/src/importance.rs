//! Message importance: read from the `Importance`, `X-Priority` and
//! `X-MSMail-Priority` headers senders write, and written on mail sent from
//! here.

use serde::{Deserialize, Serialize};

/// How important the sender (or the user, here) says a message is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Importance {
    /// Low importance.
    Low,
    /// The usual.
    #[default]
    Normal,
    /// High importance.
    High,
}

impl Importance {
    /// The stored and serialized name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }

    /// The value from [`Importance::as_str`]; anything else is `None`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "low" => Some(Self::Low),
            "normal" => Some(Self::Normal),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    /// From a message's headers. `Importance` (RFC 2156) wins, then
    /// `X-Priority` (1 and 2 high, 4 and 5 low), then Outlook's
    /// `X-MSMail-Priority`. Unreadable values are ignored.
    #[must_use]
    pub fn from_headers(
        importance: Option<&str>,
        x_priority: Option<&str>,
        x_msmail_priority: Option<&str>,
    ) -> Self {
        let word = |value: &str| match value.trim().to_ascii_lowercase().as_str() {
            "high" | "urgent" => Some(Self::High),
            "low" | "non-urgent" => Some(Self::Low),
            "normal" | "medium" => Some(Self::Normal),
            _ => None,
        };
        if let Some(found) = importance.and_then(word) {
            return found;
        }
        if let Some(found) = x_priority.and_then(|value| {
            // "1 (Highest)", "5 (Lowest)", or a bare digit.
            match value.trim().chars().next() {
                Some('1' | '2') => Some(Self::High),
                Some('3') => Some(Self::Normal),
                Some('4' | '5') => Some(Self::Low),
                _ => word(value),
            }
        }) {
            return found;
        }
        x_msmail_priority.and_then(word).unwrap_or_default()
    }

    /// The `Importance` and `X-Priority` values to send, or `None` for
    /// normal mail (no headers at all, as other mail apps do).
    #[must_use]
    pub const fn outgoing_headers(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::High => Some(("High", "1 (Highest)")),
            Self::Low => Some(("Low", "5 (Lowest)")),
            Self::Normal => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_headers_senders_write() {
        assert_eq!(
            Importance::from_headers(Some("High"), None, None),
            Importance::High
        );
        assert_eq!(
            Importance::from_headers(Some(" low "), Some("1"), None),
            Importance::Low
        );
        assert_eq!(
            Importance::from_headers(None, Some("1 (Highest)"), None),
            Importance::High
        );
        assert_eq!(
            Importance::from_headers(None, Some("2"), None),
            Importance::High
        );
        assert_eq!(
            Importance::from_headers(None, Some("5 (Lowest)"), None),
            Importance::Low
        );
        assert_eq!(
            Importance::from_headers(None, Some("3 (Normal)"), Some("High")),
            Importance::Normal
        );
        assert_eq!(
            Importance::from_headers(None, None, Some("High")),
            Importance::High
        );
        assert_eq!(
            Importance::from_headers(Some("whatever"), Some("x"), None),
            Importance::Normal
        );
        assert_eq!(
            Importance::from_headers(None, None, None),
            Importance::Normal
        );
    }

    #[test]
    fn names_round_trip_and_match_the_interface() {
        for importance in [Importance::Low, Importance::Normal, Importance::High] {
            assert_eq!(Importance::parse(importance.as_str()), Some(importance));
            assert_eq!(
                serde_json::to_value(importance).unwrap(),
                serde_json::Value::from(importance.as_str())
            );
        }
        assert_eq!(Importance::parse("High"), None);
    }

    #[test]
    fn only_high_and_low_add_headers() {
        assert_eq!(
            Importance::High.outgoing_headers(),
            Some(("High", "1 (Highest)"))
        );
        assert_eq!(
            Importance::Low.outgoing_headers(),
            Some(("Low", "5 (Lowest)"))
        );
        assert_eq!(Importance::Normal.outgoing_headers(), None);
    }
}
