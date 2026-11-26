//! Colour categories: names, colours, and how they are kept on the server.
//!
//! A category is a name (unique ignoring case, across all accounts) and a
//! colour. On Gmail a message's categories are its labels (`X-GM-LABELS`).
//! Elsewhere each one is an IMAP keyword: the name, with every character
//! that is not an ASCII letter or digit written as `_` and two upper-case
//! hex digits per UTF-8 byte, after the prefix `$Cat_`. So "Red team" is
//! `$Cat_Red_20team` and "Café" is `$Cat_Caf_C3_A9`. The encoding is
//! reversible and every keyword it makes is a valid IMAP atom.

use crate::folders::{decode_folder_name, encode_folder_name};
use crate::message::display_safe;

/// The prefix of the IMAP keywords that hold categories.
pub const KEYWORD_PREFIX: &str = "$Cat_";
/// Longest category name, in characters.
pub const MAX_NAME_CHARS: usize = 60;
/// Most categories one message can carry.
pub const MAX_PER_MESSAGE: usize = 50;
/// The keyword marking a message as junk (RFC 5788 registry).
pub const JUNK_KEYWORD: &str = "$Junk";
/// The keyword marking a message as not junk.
pub const NOT_JUNK_KEYWORD: &str = "$NotJunk";

/// Colours given to categories that arrive from the server (Gmail labels,
/// or keywords another device set), in turn.
pub const PALETTE: &[&str] = &[
    "#d13438", "#ca5010", "#c19c00", "#107c10", "#038387", "#0078d4", "#8764b8", "#e3008c",
    "#69797e", "#498205",
];

/// The palette colour for the `n`th category.
#[must_use]
pub fn palette_color(n: usize) -> &'static str {
    PALETTE
        .get(n.checked_rem(PALETTE.len()).unwrap_or(0))
        .copied()
        .unwrap_or("#0078d4")
}

/// A category name as typed, cleaned: control characters and direction
/// marks removed, spaces collapsed, trimmed.
///
/// # Errors
/// A message to show when the name is empty, too long, or starts with a
/// backslash (Gmail's system labels look like that).
pub fn clean_name(name: &str) -> Result<String, &'static str> {
    let spaced: String = name
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    let cleaned = display_safe(&spaced)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty() {
        return Err("Give the category a name.");
    }
    if cleaned.chars().count() > MAX_NAME_CHARS {
        return Err("That category name is too long.");
    }
    if cleaned.starts_with('\\') {
        return Err("A category name can't start with a backslash.");
    }
    Ok(cleaned)
}

/// A `#rrggbb` colour, lower-cased.
///
/// # Errors
/// A message to show for anything else.
pub fn clean_color(color: &str) -> Result<String, &'static str> {
    let trimmed = color.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or("");
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(trimmed.to_ascii_lowercase())
    } else {
        Err("Choose a colour like #1a73e8.")
    }
}

/// Whether two category names are the same one (case is ignored).
#[must_use]
pub fn same_name(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

fn hex_digit(nibble: u8) -> char {
    char::from_digit(u32::from(nibble), 16)
        .unwrap_or('0')
        .to_ascii_uppercase()
}

/// The IMAP keyword for a category name.
#[must_use]
pub fn keyword_for(name: &str) -> String {
    let mut out = String::from(KEYWORD_PREFIX);
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            for byte in c.encode_utf8(&mut buf).bytes() {
                out.push('_');
                out.push(hex_digit(byte >> 4));
                out.push(hex_digit(byte & 0x0f));
            }
        }
    }
    out
}

/// The category name an IMAP keyword holds, or `None` for keywords that
/// aren't categories (`$Junk`, other apps' own keywords, or a broken one).
#[must_use]
pub fn name_from_keyword(keyword: &str) -> Option<String> {
    let prefix = keyword.get(..KEYWORD_PREFIX.len())?;
    if !prefix.eq_ignore_ascii_case(KEYWORD_PREFIX) {
        return None;
    }
    let rest = keyword.get(KEYWORD_PREFIX.len()..)?;
    let mut bytes = Vec::with_capacity(rest.len());
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        if c == '_' {
            let high = chars.next()?.to_digit(16)?;
            let low = chars.next()?.to_digit(16)?;
            bytes.push(u8::try_from(high.checked_mul(16)?.checked_add(low)?).ok()?);
        } else if c.is_ascii_alphanumeric() {
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        } else {
            return None;
        }
    }
    let name = String::from_utf8(bytes).ok()?;
    clean_name(&name).ok()
}

/// The category a Gmail label stands for: user labels only, decoded from
/// modified UTF-7. System labels (`\Inbox`, `\Important`, `\Sent`,
/// `\Starred`, `\Draft`...) are `None`.
#[must_use]
pub fn name_from_label(label: &str) -> Option<String> {
    if label.starts_with('\\') {
        return None;
    }
    clean_name(&decode_folder_name(label)).ok()
}

/// The Gmail label (modified UTF-7) for a category name.
#[must_use]
pub fn label_for(name: &str) -> String {
    encode_folder_name(name)
}

/// A message's categories after adding and removing some: removals first,
/// then additions not already there go at the end, so the order they were
/// added in is kept. Case is ignored.
#[must_use]
pub fn edited(current: &[String], add: &[String], remove: &[String]) -> Vec<String> {
    let mut out: Vec<String> = current
        .iter()
        .filter(|name| !remove.iter().any(|r| same_name(r, name)))
        .cloned()
        .collect();
    for name in add {
        if out.len() >= MAX_PER_MESSAGE {
            break;
        }
        if !out.iter().any(|have| same_name(have, name)) {
            out.push(name.clone());
        }
    }
    out
}

/// A message's categories as the server now has them: the ones still
/// there keep their place, new ones go at the end.
#[must_use]
pub fn merged(current: &[String], server: &[String]) -> Vec<String> {
    let mut out: Vec<String> = current
        .iter()
        .filter(|name| server.iter().any(|s| same_name(s, name)))
        .cloned()
        .collect();
    for name in server {
        if out.len() >= MAX_PER_MESSAGE {
            break;
        }
        if !out.iter().any(|have| same_name(have, name)) {
            out.push(name.clone());
        }
    }
    out
}

/// A message's categories with one renamed (in place) or, with `None`,
/// removed. `None` when the message doesn't have it.
#[must_use]
pub fn renamed(current: &[String], from: &str, to: Option<&str>) -> Option<Vec<String>> {
    if !current.iter().any(|name| same_name(name, from)) {
        return None;
    }
    let mut out: Vec<String> = Vec::with_capacity(current.len());
    for name in current {
        let replacement = if same_name(name, from) {
            to.map(str::to_owned)
        } else {
            Some(name.clone())
        };
        if let Some(replacement) = replacement
            && !out.iter().any(|have| same_name(have, &replacement))
        {
            out.push(replacement);
        }
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn keywords_are_atoms_and_read_back() {
        assert_eq!(keyword_for("Red team"), "$Cat_Red_20team");
        assert_eq!(keyword_for("Café"), "$Cat_Caf_C3_A9");
        assert_eq!(keyword_for("a_b"), "$Cat_a_5Fb");
        for name in [
            "Red team",
            "Café",
            "a_b",
            "Q3 (draft)",
            "\"quoted\"",
            "日本",
        ] {
            let keyword = keyword_for(name);
            assert!(
                keyword
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$'),
                "{keyword}"
            );
            assert_eq!(name_from_keyword(&keyword).as_deref(), Some(name));
        }
        // A server that changes the case of keywords still gives the name.
        assert_eq!(
            name_from_keyword("$cat_Red_20team").as_deref(),
            Some("Red team")
        );
    }

    #[test]
    fn other_keywords_are_not_categories() {
        for keyword in [
            "$Junk",
            "$NotJunk",
            "$label1",
            "$Cat_",
            "$Cat_bad_Z1",
            "$Cat_x_4",
            "$Cat_a-b",
            "$Cat__FF",
            "NonJunk",
        ] {
            assert_eq!(name_from_keyword(keyword), None, "{keyword}");
        }
    }

    #[test]
    fn gmail_system_labels_are_skipped() {
        for label in ["\\Inbox", "\\Important", "\\Sent", "\\Starred", "\\Draft"] {
            assert_eq!(name_from_label(label), None);
        }
        assert_eq!(name_from_label("Clients").as_deref(), Some("Clients"));
        assert_eq!(name_from_label("Entw&APw-rfe").as_deref(), Some("Entwürfe"));
        assert_eq!(label_for("Entwürfe"), "Entw&APw-rfe");
    }

    #[test]
    fn names_and_colours_are_checked() {
        assert_eq!(clean_name("  Red \t team\u{202E} ").unwrap(), "Red team");
        assert!(clean_name("   ").is_err());
        assert!(clean_name("\\Inbox").is_err());
        assert!(clean_name(&"x".repeat(MAX_NAME_CHARS + 1)).is_err());
        assert_eq!(clean_color(" #1A73E8 ").unwrap(), "#1a73e8");
        for bad in ["1a73e8", "#1a73e", "#zzzzzz", "red", ""] {
            assert!(clean_color(bad).is_err(), "{bad}");
        }
        assert_eq!(Some(palette_color(0)), PALETTE.first().copied());
        assert_eq!(Some(palette_color(PALETTE.len())), PALETTE.first().copied());
    }

    #[test]
    fn edits_keep_the_order_categories_were_added_in() {
        let current = names(&["Red", "Blue"]);
        assert_eq!(
            edited(&current, &names(&["green", "red"]), &[]),
            names(&["Red", "Blue", "green"])
        );
        assert_eq!(edited(&current, &[], &names(&["RED"])), names(&["Blue"]));
        assert_eq!(
            merged(&current, &names(&["blue", "Yellow"])),
            names(&["Blue", "Yellow"])
        );
        assert_eq!(
            renamed(&current, "red", Some("Crimson")),
            Some(names(&["Crimson", "Blue"]))
        );
        assert_eq!(renamed(&current, "Red", None), Some(names(&["Blue"])));
        // Renamed into one it already has: listed once.
        assert_eq!(
            renamed(&current, "Red", Some("blue")),
            Some(names(&["blue"]))
        );
        assert_eq!(renamed(&current, "Green", Some("x")), None);
    }

    proptest! {
        #[test]
        fn any_name_survives_the_keyword_round_trip(name in "\\PC{1,40}") {
            if let Ok(clean) = clean_name(&name) {
                prop_assert_eq!(name_from_keyword(&keyword_for(&clean)), Some(clean));
            }
        }
    }
}
