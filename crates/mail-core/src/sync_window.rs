//! The per-account sync window: how many months of mail are kept in full
//! (bodies downloaded for offline reading and search). Older mail keeps its
//! headers.

use chrono::{DateTime, Months, NaiveDate};

/// The longest window that can be chosen, in months (20 years).
pub const MAX_MONTHS: u32 = 240;

/// Whether `months` is a window that can be chosen.
#[must_use]
pub const fn is_valid(months: u32) -> bool {
    months >= 1 && months <= MAX_MONTHS
}

/// The first day inside a window of `months` months ending at `now` (Unix
/// seconds, UTC).
#[must_use]
pub fn first_day(now: i64, months: u32) -> Option<NaiveDate> {
    DateTime::from_timestamp(now, 0)?
        .date_naive()
        .checked_sub_months(Months::new(months))
}

/// That day as an IMAP date for `SEARCH SINCE` (RFC 3501 `date`, such as
/// `5-Mar-2026`).
#[must_use]
pub fn imap_since(now: i64, months: u32) -> Option<String> {
    first_day(now, months).map(|day| day.format("%-d-%b-%Y").to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn window_dates_for_imap_search() {
        // 2026-11-26 12:00 UTC.
        let now = 1_795_694_400;
        assert_eq!(imap_since(now, 1).as_deref(), Some("26-Oct-2026"));
        assert_eq!(imap_since(now, 12).as_deref(), Some("26-Nov-2025"));
        assert_eq!(imap_since(now, 3).as_deref(), Some("26-Aug-2026"));
        // The 31st falls back to the end of a shorter month.
        let may_31 = 1_780_228_800; // 2026-05-31 12:00 UTC
        assert_eq!(imap_since(may_31, 1).as_deref(), Some("30-Apr-2026"));
        assert_eq!(imap_since(1_772_668_800, 0).as_deref(), Some("5-Mar-2026"));
    }

    #[test]
    fn only_sensible_windows_are_accepted() {
        assert!(!is_valid(0));
        assert!(is_valid(1));
        assert!(is_valid(MAX_MONTHS));
        assert!(!is_valid(MAX_MONTHS + 1));
    }
}
