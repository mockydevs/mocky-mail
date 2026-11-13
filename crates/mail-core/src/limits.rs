//! Message size limits.
//!
//! Mail servers limit the size of the *encoded* message, not the size of the
//! files on disk. Attachments are base64-encoded for MIME, which grows them
//! by about a third, plus a line break every 76 characters. A 20 MB file can
//! therefore be rejected by a server that "accepts 25 MB". We estimate the
//! encoded size up front so the user is told before sending, not after.

use serde::Serialize;
use thiserror::Error;

/// Maximum characters per encoded line in MIME base64 (RFC 2045).
const MIME_LINE_LEN: u64 = 76;
/// Bytes added per line break (CRLF).
const CRLF_LEN: u64 = 2;
/// Allowance for MIME headers, boundaries and the message header block.
pub const HEADER_ALLOWANCE: u64 = 16 * 1024;

/// Size in bytes of `raw` bytes once base64-encoded for MIME, with CRLF line
/// breaks. Saturates instead of overflowing for absurd inputs.
#[must_use]
pub const fn base64_mime_len(raw: u64) -> u64 {
    let encoded = raw.div_ceil(3).saturating_mul(4);
    let lines = encoded.div_ceil(MIME_LINE_LEN);
    let breaks = lines.saturating_sub(1).saturating_mul(CRLF_LEN);
    encoded.saturating_add(breaks)
}

/// The message would be larger than the server accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error, Serialize)]
#[error("message is about {estimated} bytes once encoded; the server accepts {limit}")]
pub struct TooLarge {
    /// Estimated encoded size of the whole message, in bytes.
    pub estimated: u64,
    /// The server's limit, in bytes.
    pub limit: u64,
}

/// Estimate the encoded size of a message with a body of `body_len` bytes
/// and attachments of the given raw sizes.
#[must_use]
pub fn estimate_message_size(body_len: u64, attachment_sizes: &[u64]) -> u64 {
    attachment_sizes
        .iter()
        .map(|&size| base64_mime_len(size))
        .fold(
            base64_mime_len(body_len).saturating_add(HEADER_ALLOWANCE),
            u64::saturating_add,
        )
}

/// Check a message against a server limit. Returns the estimated encoded
/// size when it fits.
///
/// # Errors
/// Returns [`TooLarge`] when the estimate exceeds `limit`.
pub fn check_message_size(
    body_len: u64,
    attachment_sizes: &[u64],
    limit: u64,
) -> Result<u64, TooLarge> {
    let estimated = estimate_message_size(body_len, attachment_sizes);
    if estimated > limit {
        Err(TooLarge { estimated, limit })
    } else {
        Ok(estimated)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn base64_len_matches_known_values() {
        assert_eq!(base64_mime_len(0), 0);
        assert_eq!(base64_mime_len(1), 4);
        assert_eq!(base64_mime_len(3), 4);
        assert_eq!(base64_mime_len(4), 8);
        // 57 raw bytes is exactly one full 76-character line.
        assert_eq!(base64_mime_len(57), 76);
        // One byte more starts a second line: 80 chars plus one CRLF.
        assert_eq!(base64_mime_len(58), 82);
    }

    #[test]
    fn base64_len_saturates_instead_of_overflowing() {
        assert_eq!(base64_mime_len(u64::MAX), u64::MAX);
    }

    #[test]
    fn twenty_mb_file_does_not_fit_a_25_mb_limit_once_encoded() {
        let mb = 1024 * 1024;
        let result = check_message_size(2_000, &[20 * mb], 25 * mb);
        assert!(matches!(result, Err(TooLarge { .. })));
    }

    #[test]
    fn small_message_fits() {
        let result = check_message_size(2_000, &[180 * 1024], 25 * 1024 * 1024);
        assert!(result.is_ok());
    }

    #[test]
    fn limit_is_inclusive() {
        let estimated = estimate_message_size(10, &[100]);
        assert_eq!(check_message_size(10, &[100], estimated), Ok(estimated));
        assert!(check_message_size(10, &[100], estimated - 1).is_err());
    }

    proptest! {
        #[test]
        fn encoded_is_never_smaller_than_four_thirds(raw in 0u64..10_000_000) {
            let encoded = base64_mime_len(raw);
            prop_assert!(encoded * 3 >= raw * 4);
        }

        #[test]
        fn encoding_is_monotonic(a in 0u64..10_000_000, b in 0u64..10_000_000) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            prop_assert!(base64_mime_len(lo) <= base64_mime_len(hi));
        }

        #[test]
        fn adding_an_attachment_never_shrinks_the_estimate(
            body in 0u64..1_000_000,
            sizes in proptest::collection::vec(0u64..50_000_000, 0..8),
            extra in 0u64..50_000_000,
        ) {
            let before = estimate_message_size(body, &sizes);
            let mut more = sizes.clone();
            more.push(extra);
            prop_assert!(estimate_message_size(body, &more) >= before);
        }
    }
}
