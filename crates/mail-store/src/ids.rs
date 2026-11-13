//! Random record IDs and checked integer conversions for `SQLite` columns.

use crate::error::{Result, StoreError};

/// A new random 128-bit ID as 32 lowercase hex characters.
pub(crate) fn new_id() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| StoreError::Random(e.to_string()))?;
    let mut out = String::with_capacity(32);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    Ok(out)
}

fn hex_digit(nibble: u8) -> char {
    char::from_digit(u32::from(nibble), 16).unwrap_or('0')
}

/// An unsigned value as an `SQLite` integer, refusing values above `i64::MAX`.
pub(crate) fn to_i64(value: u64, what: &'static str) -> Result<i64> {
    i64::try_from(value).map_err(|_| StoreError::Invalid(what))
}

/// A stored integer as `u64`, refusing negatives.
pub(crate) fn to_u64(value: i64, what: &'static str) -> Result<u64> {
    u64::try_from(value).map_err(|_| StoreError::Corrupt(what))
}

/// A stored integer as `u32`, refusing values out of range.
pub(crate) fn to_u32(value: i64, what: &'static str) -> Result<u32> {
    u32::try_from(value).map_err(|_| StoreError::Corrupt(what))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_32_hex_chars_and_unique() {
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            let id = new_id().unwrap();
            assert_eq!(id.len(), 32);
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            );
            assert!(seen.insert(id));
        }
    }

    #[test]
    fn conversions_are_checked() {
        assert_eq!(to_i64(5, "x").unwrap(), 5);
        assert!(matches!(
            to_i64(u64::MAX, "x"),
            Err(StoreError::Invalid("x"))
        ));
        assert!(matches!(to_u64(-1, "y"), Err(StoreError::Corrupt("y"))));
        assert_eq!(to_u32(7, "z").unwrap(), 7);
        assert!(to_u32(i64::from(u32::MAX) + 1, "z").is_err());
    }
}
