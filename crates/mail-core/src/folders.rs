//! Folder names and roles.
//!
//! Servers name the same folders differently ("Sent", "Sent Items",
//! "[Gmail]/Sent Mail", "INBOX.Sent"). We prefer the server's RFC 6154
//! special-use flags and fall back to well-known names. Folder names on the
//! wire use IMAP's modified UTF-7 (RFC 3501 §5.1.3), decoded here for display.

use serde::{Deserialize, Serialize};

/// What a folder is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderRole {
    /// Incoming mail.
    Inbox,
    /// Copies of sent mail.
    Sent,
    /// Unsent drafts.
    Drafts,
    /// Archived mail.
    Archive,
    /// Spam.
    Junk,
    /// Deleted mail.
    Trash,
    /// A virtual folder with all mail (Gmail).
    All,
    /// A virtual folder with flagged mail.
    Flagged,
    /// Any other user folder.
    Other,
}

impl FolderRole {
    /// Sort position in the sidebar.
    #[must_use]
    pub const fn sort_rank(self) -> u8 {
        match self {
            Self::Inbox => 0,
            Self::Drafts => 1,
            Self::Sent => 2,
            Self::Archive => 3,
            Self::Flagged => 4,
            Self::All => 5,
            Self::Junk => 6,
            Self::Trash => 7,
            Self::Other => 8,
        }
    }
}

/// Work out a folder's role from its IMAP attributes and name.
///
/// `attributes` are the LIST attributes such as `\Sent` (any case).
/// `delimiter` is the server's hierarchy separator, if any.
///
/// A well-known name counts only at the top level (or directly under
/// `INBOX`), so `Clients/Deleted` is an ordinary folder. To also keep a
/// name from taking a role the server gave another folder, use
/// [`detect_role_unclaimed`].
#[must_use]
pub fn detect_role(name: &str, attributes: &[&str], delimiter: Option<&str>) -> FolderRole {
    detect_role_unclaimed(name, attributes, delimiter, &[])
}

/// The role the server gives a folder with a special-use flag (RFC 6154)
/// among its LIST attributes, if it has one.
#[must_use]
pub fn special_use_role(attributes: &[&str]) -> Option<FolderRole> {
    attributes
        .iter()
        .find_map(|attribute| match attribute.to_ascii_lowercase().as_str() {
            "\\sent" => Some(FolderRole::Sent),
            "\\drafts" => Some(FolderRole::Drafts),
            "\\archive" => Some(FolderRole::Archive),
            "\\junk" => Some(FolderRole::Junk),
            "\\trash" => Some(FolderRole::Trash),
            "\\all" => Some(FolderRole::All),
            "\\flagged" => Some(FolderRole::Flagged),
            _ => None,
        })
}

/// Like [`detect_role`], for an account whose other folders are known:
/// `claimed` are the roles the server's special-use flags give to any of
/// its folders (collect them with [`special_use_role`]). A folder without
/// a flag doesn't take one of those from its name: where the server marks
/// its Trash, a folder called "Deleted Items" is the user's own.
#[must_use]
pub fn detect_role_unclaimed(
    name: &str,
    attributes: &[&str],
    delimiter: Option<&str>,
    claimed: &[FolderRole],
) -> FolderRole {
    if name.eq_ignore_ascii_case("INBOX") {
        return FolderRole::Inbox;
    }
    if let Some(role) = special_use_role(attributes) {
        return role;
    }
    let role = role_from_name(name, delimiter);
    if claimed.contains(&role) {
        FolderRole::Other
    } else {
        role
    }
}

/// The role a well-known name stands for. Only where servers put such
/// folders: at the top level, or one level under `INBOX` or Gmail's own
/// prefix. Deeper down ("2023/Sent", "Lists/Spam") the name is the user's.
fn role_from_name(name: &str, delimiter: Option<&str>) -> FolderRole {
    let (parent, leaf) = delimiter
        .filter(|d| !d.is_empty())
        .and_then(|d| name.rsplit_once(d))
        .map_or((None, name), |(parent, leaf)| (Some(parent), leaf));
    if parent.is_some_and(|p| {
        !["INBOX", "[Gmail]", "[Google Mail]"]
            .iter()
            .any(|prefix| p.eq_ignore_ascii_case(prefix))
    }) {
        return FolderRole::Other;
    }
    match leaf.trim().to_ascii_lowercase().as_str() {
        "sent" | "sent items" | "sent messages" | "sent mail" => FolderRole::Sent,
        "drafts" | "draft" => FolderRole::Drafts,
        "archive" | "archives" => FolderRole::Archive,
        "junk" | "spam" | "junk e-mail" | "junk email" | "bulk mail" => FolderRole::Junk,
        "trash" | "deleted items" | "deleted messages" | "bin" | "deleted" => FolderRole::Trash,
        "all mail" => FolderRole::All,
        "starred" | "flagged" => FolderRole::Flagged,
        _ => FolderRole::Other,
    }
}

/// Decode an IMAP modified UTF-7 folder name. Returns the input unchanged
/// when it is not valid modified UTF-7, so a bad server name is still shown.
#[must_use]
pub fn decode_folder_name(encoded: &str) -> String {
    decode_modified_utf7(encoded).unwrap_or_else(|| encoded.to_owned())
}

fn decode_modified_utf7(input: &str) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('&') {
        out.push_str(rest.get(..start)?);
        let after = rest.get(start.checked_add(1)?..)?;
        let end = after.find('-')?;
        let chunk = after.get(..end)?;
        if chunk.is_empty() {
            out.push('&');
        } else {
            out.push_str(&decode_base64_utf16(chunk)?);
        }
        rest = after.get(end.checked_add(1)?..)?;
    }
    out.push_str(rest);
    Some(out)
}

fn base64_value(c: u8) -> Option<u32> {
    let value = match c {
        b'A'..=b'Z' => c.checked_sub(b'A')?,
        b'a'..=b'z' => c.checked_sub(b'a')?.checked_add(26)?,
        b'0'..=b'9' => c.checked_sub(b'0')?.checked_add(52)?,
        b'+' => 62,
        b',' => 63,
        _ => return None,
    };
    Some(u32::from(value))
}

fn decode_base64_utf16(chunk: &str) -> Option<String> {
    let mut bits: u32 = 0;
    let mut bit_count: u32 = 0;
    let mut units: Vec<u16> = Vec::new();
    for c in chunk.bytes() {
        bits = bits.checked_shl(6)? | base64_value(c)?;
        bit_count = bit_count.checked_add(6)?;
        if bit_count >= 16 {
            bit_count = bit_count.checked_sub(16)?;
            let unit = u16::try_from(bits.checked_shr(bit_count)? & 0xFFFF).ok()?;
            units.push(unit);
            bits &= 1u32.checked_shl(bit_count)?.checked_sub(1)?;
        }
    }
    if units.is_empty() {
        return None;
    }
    String::from_utf16(&units).ok()
}

/// Longest folder name accepted, in characters.
pub const MAX_FOLDER_NAME_CHARS: usize = 100;

/// A folder name as typed, checked and tidied. `delimiter` is the server's
/// level separator (`/` or `.`), which a name can't contain.
///
/// # Errors
/// Plain words for the user when the name can't be used.
pub fn clean_folder_name(name: &str, delimiter: Option<&str>) -> Result<String, String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("Type a name for the folder.".to_owned());
    }
    if name.chars().count() > MAX_FOLDER_NAME_CHARS {
        return Err(format!(
            "Use a shorter name (up to {MAX_FOLDER_NAME_CHARS} characters)."
        ));
    }
    if name.eq_ignore_ascii_case("inbox") {
        return Err("Inbox is taken; choose another name.".to_owned());
    }
    if let Some(d) = delimiter.filter(|d| !d.is_empty())
        && name.contains(d)
    {
        return Err(format!(
            "Folder names can't contain \"{d}\" on this server."
        ));
    }
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '*' | '%' | '"' | '\\'))
    {
        return Err("Folder names can't contain * % \" or \\.".to_owned());
    }
    Ok(name)
}

/// Encode a folder name for IMAP (modified UTF-7, RFC 3501 5.1.3).
#[must_use]
pub fn encode_folder_name(name: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";
    let mut out = String::with_capacity(name.len());
    let mut pending: Vec<u16> = Vec::new();
    let flush = |pending: &mut Vec<u16>, out: &mut String| {
        if pending.is_empty() {
            return;
        }
        out.push('&');
        let mut bits: u32 = 0;
        let mut count: u32 = 0;
        for unit in pending.drain(..) {
            bits = (bits << 16) | u32::from(unit);
            count = count.saturating_add(16);
            while count >= 6 {
                count = count.saturating_sub(6);
                let index = usize::try_from((bits >> count) & 0x3f).unwrap_or(0);
                out.push(char::from(ALPHABET.get(index).copied().unwrap_or(b'A')));
            }
            bits &= (1u32 << count).saturating_sub(1);
        }
        if count > 0 {
            let index = usize::try_from((bits << (6u32.saturating_sub(count))) & 0x3f).unwrap_or(0);
            out.push(char::from(ALPHABET.get(index).copied().unwrap_or(b'A')));
        }
        out.push('-');
    };
    for c in name.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut pending, &mut out);
            if c == '&' {
                out.push_str("&-");
            } else {
                out.push(c);
            }
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn inbox_is_always_inbox() {
        assert_eq!(detect_role("inbox", &[], None), FolderRole::Inbox);
        assert_eq!(detect_role("INBOX", &["\\Trash"], None), FolderRole::Inbox);
    }

    #[test]
    fn special_use_flags_win_over_names() {
        assert_eq!(
            detect_role("Papierkorb", &["\\Trash"], Some("/")),
            FolderRole::Trash
        );
        assert_eq!(
            detect_role(
                "[Gmail]/Sent Mail",
                &["\\HasNoChildren", "\\Sent"],
                Some("/")
            ),
            FolderRole::Sent
        );
    }

    #[test]
    fn falls_back_to_common_names() {
        assert_eq!(detect_role("INBOX.Sent", &[], Some(".")), FolderRole::Sent);
        assert_eq!(
            detect_role("Deleted Items", &[], Some("/")),
            FolderRole::Trash
        );
        assert_eq!(detect_role("Junk E-mail", &[], None), FolderRole::Junk);
        assert_eq!(detect_role("Clients", &[], Some("/")), FolderRole::Other);
    }

    #[test]
    fn names_count_only_at_the_top_level() {
        for (name, delimiter) in [
            ("Clients/Deleted", "/"),
            ("Lists/Spam", "/"),
            ("2023/Sent", "/"),
            ("INBOX.Projects.Trash", "."),
        ] {
            assert_eq!(
                detect_role(name, &[], Some(delimiter)),
                FolderRole::Other,
                "{name}"
            );
        }
        assert_eq!(
            detect_role("INBOX/Trash", &[], Some("/")),
            FolderRole::Trash
        );
        assert_eq!(
            detect_role("[Gmail]/Spam", &[], Some("/")),
            FolderRole::Junk
        );
        // A flag still wins at any depth.
        assert_eq!(
            detect_role("Clients/Deleted", &["\\Trash"], Some("/")),
            FolderRole::Trash
        );
    }

    #[test]
    fn names_do_not_take_a_role_the_server_gave_another_folder() {
        let listed: [(&str, &[&str]); 3] = [
            ("Papierkorb", &["\\HasNoChildren", "\\Trash"]),
            ("Deleted Items", &["\\HasNoChildren"]),
            ("Sent", &[]),
        ];
        let claimed: Vec<FolderRole> = listed
            .iter()
            .filter_map(|(_, attributes)| special_use_role(attributes))
            .collect();
        assert_eq!(claimed, [FolderRole::Trash]);
        let roles: Vec<FolderRole> = listed
            .iter()
            .map(|(name, attributes)| detect_role_unclaimed(name, attributes, Some("/"), &claimed))
            .collect();
        assert_eq!(
            roles,
            [FolderRole::Trash, FolderRole::Other, FolderRole::Sent]
        );
    }

    #[test]
    fn decodes_modified_utf7() {
        assert_eq!(decode_folder_name("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(decode_folder_name("&ZeVnLIqe-"), "日本語");
        assert_eq!(decode_folder_name("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(decode_folder_name("Plain"), "Plain");
    }

    #[test]
    fn invalid_utf7_is_shown_as_is() {
        assert_eq!(decode_folder_name("Bad&***-"), "Bad&***-");
        assert_eq!(decode_folder_name("Open&APw"), "Open&APw");
    }

    proptest! {
        #[test]
        fn decoding_never_panics(name in ".{0,80}") {
            let _ = decode_folder_name(&name);
        }

        #[test]
        fn ascii_without_ampersand_is_unchanged(name in "[ -%'-~]{0,60}") {
            prop_assert_eq!(decode_folder_name(&name), name);
        }
    }

    #[test]
    fn folder_names_encode_for_imap_and_back() {
        assert_eq!(encode_folder_name("Clients"), "Clients");
        assert_eq!(encode_folder_name("R&D"), "R&-D");
        assert_eq!(encode_folder_name("Entwürfe"), "Entw&APw-rfe");
        assert_eq!(encode_folder_name("台北"), "&U,BTFw-");
        for name in [
            "Wateja 2026",
            "Café ☕ orders",
            "Tickets & Quotes",
            "😀 Fun",
            "日本語/テスト",
        ] {
            assert_eq!(decode_folder_name(&encode_folder_name(name)), name);
        }
    }

    #[test]
    fn folder_names_are_checked() {
        assert_eq!(
            clean_folder_name("  Client   Orders ", Some("/")).unwrap(),
            "Client Orders"
        );
        assert!(clean_folder_name("", Some("/")).is_err());
        assert!(clean_folder_name("a/b", Some("/")).is_err());
        assert!(clean_folder_name("a.b", Some(".")).is_err());
        assert!(clean_folder_name("a.b", Some("/")).is_ok());
        assert!(clean_folder_name("INBOX", None).is_err());
        assert!(clean_folder_name("x*", None).is_err());
        assert!(clean_folder_name(&"x".repeat(101), None).is_err());
    }

    proptest! {
        #[test]
        fn any_name_round_trips(name in "\\PC{1,30}") {
            prop_assert_eq!(decode_folder_name(&encode_folder_name(&name)), name);
        }
    }
}
