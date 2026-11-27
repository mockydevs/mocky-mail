//! Contacts as vCards (RFC 6350, and the 3.0 that `SOGo` and most servers
//! still write).
//!
//! [`parse`] reads the fields the app shows. [`write`] produces a card for
//! the server. Given the server's original card, it replaces only those
//! fields and keeps everything else (photos, addresses, birthdays, custom
//! fields), so editing a contact here never loses what another app added.
//! A name, address or number that wasn't edited keeps its line as the
//! server had it: the name's parts, `TYPE=CELL`, Apple's `item1.` groups.

use serde::{Deserialize, Serialize};

/// Largest card accepted from a server, bytes (photos make cards big).
pub const MAX_VCARD_BYTES: usize = 512 * 1024;

/// The fields of a contact the app shows and edits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactCard {
    /// Stable id inside the card (`UID`).
    pub uid: String,
    /// Display name (`FN`).
    pub name: String,
    /// Email addresses, first is the main one.
    pub emails: Vec<String>,
    /// Phone numbers as written.
    pub phones: Vec<String>,
    /// Company (`ORG`, first part).
    pub organization: String,
    /// Job title (`TITLE`).
    pub title: String,
    /// Free text (`NOTE`).
    pub notes: String,
    /// Shown first (`X-MOCKY-FAVORITE`).
    #[serde(default)]
    pub favorite: bool,
    /// Category names (`CATEGORIES`).
    #[serde(default)]
    pub categories: Vec<String>,
    /// A small picture as a `data:` URL (`PHOTO`, when the card holds the
    /// picture itself and it is at most [`MAX_PHOTO_BYTES`]).
    #[serde(default)]
    pub photo: Option<String>,
}

/// Largest picture kept with a contact, bytes (before base64).
pub const MAX_PHOTO_BYTES: usize = 200 * 1024;

/// Picture types a contact photo may be.
const PHOTO_TYPES: &[(&str, &str)] = &[
    ("image/jpeg", "JPEG"),
    ("image/png", "PNG"),
    ("image/gif", "GIF"),
    ("image/webp", "WEBP"),
];

/// A contact group (a list): its `UID`, name and members' `UID`s.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupCard {
    /// Stable id inside the card.
    pub uid: String,
    /// The group's name.
    pub name: String,
    /// Members, by their cards' `UID`.
    pub members: Vec<String>,
}

/// Properties [`write`] owns; everything else in an original card is kept.
const MANAGED: &[&str] = &[
    "FN",
    "N",
    "EMAIL",
    "TEL",
    "ORG",
    "TITLE",
    "NOTE",
    "UID",
    "VERSION",
    "PRODID",
    "REV",
    "CATEGORIES",
    "X-MOCKY-FAVORITE",
];

/// One content line: name (no group, upper case), parameters, raw value.
pub(crate) struct Line<'a> {
    pub(crate) name: String,
    pub(crate) params: &'a str,
    pub(crate) value: &'a str,
}

/// Undo line folding (RFC 6350 3.2): a line starting with a space or tab
/// continues the one before.
pub(crate) fn unfold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(rest) = line.strip_prefix([' ', '\t']) {
            out.push_str(rest);
        } else {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line);
        }
    }
    out
}

pub(crate) fn split_line(line: &str) -> Option<Line<'_>> {
    // The value starts at the first colon outside a quoted parameter.
    let mut quoted = false;
    let colon = line.char_indices().find_map(|(i, c)| match c {
        '"' => {
            quoted = !quoted;
            None
        }
        ':' if !quoted => Some(i),
        _ => None,
    })?;
    let (head, value) = (line.get(..colon)?, line.get(colon.checked_add(1)?..)?);
    let (name, params) = head.split_once(';').unwrap_or((head, ""));
    // "item1.EMAIL" (Apple groups) is still EMAIL.
    let name = name
        .rsplit('.')
        .next()
        .unwrap_or(name)
        .trim()
        .to_ascii_uppercase();
    Some(Line {
        name,
        params,
        value,
    })
}

/// Text value with vCard escapes undone.
pub(crate) fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub(crate) fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Split a structured value (`N`, `ORG`) on unescaped semicolons.
fn components(value: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let (Some(next), Some(last)) = (chars.next(), parts.last_mut()) {
                    last.push('\\');
                    last.push(next);
                }
            }
            ';' => parts.push(String::new()),
            c => {
                if let Some(last) = parts.last_mut() {
                    last.push(c);
                }
            }
        }
    }
    parts
        .iter()
        .map(|p| unescape(p).trim().to_owned())
        .collect()
}

/// Read a vCard. `None` when it isn't one or has nothing to show.
#[must_use]
pub fn parse(text: &str) -> Option<ContactCard> {
    if text.len() > MAX_VCARD_BYTES {
        return None;
    }
    let text = unfold(text);
    let mut card = ContactCard::default();
    let mut structured_name = String::new();
    let mut inside = false;
    for raw in text.lines() {
        let Some(line) = split_line(raw) else {
            continue;
        };
        match line.name.as_str() {
            "BEGIN" if line.value.trim().eq_ignore_ascii_case("VCARD") => inside = true,
            "END" if line.value.trim().eq_ignore_ascii_case("VCARD") => break,
            _ if !inside => {}
            "UID" => unescape(line.value).trim().clone_into(&mut card.uid),
            "FN" => card.name = clean(&unescape(line.value)),
            "N" => {
                // Family; Given; Additional; Prefix; Suffix
                let parts = components(line.value);
                let pick = |i: usize| parts.get(i).map_or("", String::as_str);
                structured_name = [pick(3), pick(1), pick(2), pick(0), pick(4)]
                    .into_iter()
                    .filter(|p| !p.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
            }
            "EMAIL" => {
                let email = email_of(line.value);
                if email.contains('@')
                    && !card.emails.iter().any(|e| e.eq_ignore_ascii_case(&email))
                {
                    // A preferred address goes first.
                    if is_preferred(line.params) {
                        card.emails.insert(0, email);
                    } else {
                        card.emails.push(email);
                    }
                }
            }
            "TEL" => {
                let phone = phone_of(line.value);
                if !phone.is_empty() && !card.phones.contains(&phone) {
                    card.phones.push(phone);
                }
            }
            "ORG" => {
                card.organization = components(line.value)
                    .into_iter()
                    .next()
                    .unwrap_or_default();
            }
            "TITLE" => card.title = clean(&unescape(line.value)),
            "NOTE" => unescape(line.value).trim().clone_into(&mut card.notes),
            "CATEGORIES" => {
                for name in split_list(line.value) {
                    if !card
                        .categories
                        .iter()
                        .any(|c| c.eq_ignore_ascii_case(&name))
                    {
                        card.categories.push(name);
                    }
                }
            }
            "X-MOCKY-FAVORITE" => {
                card.favorite = matches!(
                    line.value.trim().to_ascii_uppercase().as_str(),
                    "1" | "TRUE" | "YES"
                );
            }
            "PHOTO" if card.photo.is_none() => card.photo = photo_of(line.params, line.value),
            "KIND" | "X-ADDRESSBOOKSERVER-KIND"
                if line.value.trim().eq_ignore_ascii_case("group") =>
            {
                // A group, not a person: see [`parse_group`].
                return None;
            }
            _ => {}
        }
    }
    if card.name.is_empty() {
        card.name = clean(&structured_name);
    }
    if card.name.is_empty() && card.emails.is_empty() && card.phones.is_empty() {
        return None;
    }
    Some(card)
}

/// Values of a comma-separated text list (`CATEGORIES`), unescaped.
fn split_list(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    current.push('\\');
                    current.push(next);
                }
            }
            ',' => out.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
    }
    out.push(current);
    out.into_iter()
        .map(|c| clean(&unescape(&c)))
        .filter(|c| !c.is_empty())
        .collect()
}

/// The MIME type of a picture from its first bytes, if it is one the app
/// keeps (JPEG, PNG, GIF, WebP).
fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() > 12
        && bytes.get(..4) == Some(b"RIFF")
        && bytes.get(8..12) == Some(b"WEBP")
    {
        Some("image/webp")
    } else {
        None
    }
}

/// A `PHOTO` line's picture as a `data:` URL, when the card holds the
/// picture itself (base64, vCard 3 or 4) and it is small enough. Pictures
/// given by address (`https:`) aren't fetched.
fn photo_of(params: &str, value: &str) -> Option<String> {
    use base64::Engine as _;
    let value = value.trim();
    let data: String = if let Some(rest) = value.strip_prefix("data:") {
        let (head, data) = rest.split_once(',')?;
        if !head.to_ascii_lowercase().contains(";base64") {
            return None;
        }
        data.to_owned()
    } else {
        let upper = params.to_ascii_uppercase();
        if !(upper.contains("ENCODING=B") || upper.contains("ENCODING=BASE64")) {
            return None;
        }
        value.to_owned()
    };
    let data: String = data.chars().filter(|c| !c.is_whitespace()).collect();
    if data.len() > MAX_PHOTO_BYTES.div_ceil(3).saturating_mul(4) {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .ok()?;
    let kind = sniff_image(&bytes)?;
    Some(format!("data:{kind};base64,{data}"))
}

/// Check a picture from the editor: a `data:` URL of a JPEG, PNG, GIF or
/// WebP of at most [`MAX_PHOTO_BYTES`]. Returns it tidied.
///
/// # Errors
/// A message for the user.
pub fn checked_photo(url: &str) -> Result<String, &'static str> {
    let url = url.trim();
    let Some(rest) = url.strip_prefix("data:") else {
        return Err("Choose a picture file for the contact.");
    };
    let Some((head, data)) = rest.split_once(',') else {
        return Err("Choose a picture file for the contact.");
    };
    if !head.to_ascii_lowercase().ends_with(";base64") {
        return Err("Choose a picture file for the contact.");
    }
    if data.len() > MAX_PHOTO_BYTES.div_ceil(3).saturating_mul(4) {
        return Err("That picture is too large. Choose one under 200 KB.");
    }
    photo_of("", &format!("data:{head},{data}"))
        .ok_or("That picture can't be used. Choose a JPEG, PNG, GIF or WebP.")
}

/// The `PHOTO` line for a picture (`data:` URL) in a vCard 3.0.
fn photo_line(url: &str) -> Option<String> {
    let rest = url.strip_prefix("data:")?;
    let (head, data) = rest.split_once(',')?;
    let mime = head.split(';').next().unwrap_or_default();
    let kind = PHOTO_TYPES
        .iter()
        .find(|(m, _)| m.eq_ignore_ascii_case(mime))
        .map(|(_, k)| *k)?;
    Some(format!("PHOTO;ENCODING=b;TYPE={kind}:{data}"))
}

/// Whether a card is a group (`KIND:group`, or Apple's
/// `X-ADDRESSBOOKSERVER-KIND:group`).
#[must_use]
pub fn is_group(text: &str) -> bool {
    card_lines(&unfold(text)).iter().any(|(line, _)| {
        matches!(line.name.as_str(), "KIND" | "X-ADDRESSBOOKSERVER-KIND")
            && line.value.trim().eq_ignore_ascii_case("group")
    })
}

/// Read a group card. Members are named by `MEMBER` (vCard 4) or
/// `X-ADDRESSBOOKSERVER-MEMBER` (Apple), as `urn:uuid:<UID>`.
#[must_use]
pub fn parse_group(text: &str) -> Option<GroupCard> {
    if text.len() > MAX_VCARD_BYTES || !is_group(text) {
        return None;
    }
    let unfolded = unfold(text);
    let mut group = GroupCard::default();
    for (line, _) in card_lines(&unfolded) {
        match line.name.as_str() {
            "UID" => unescape(line.value).trim().clone_into(&mut group.uid),
            "FN" => group.name = clean(&unescape(line.value)),
            "N" if group.name.is_empty() => {
                group.name = components(line.value)
                    .into_iter()
                    .next()
                    .unwrap_or_default();
            }
            "MEMBER" | "X-ADDRESSBOOKSERVER-MEMBER" => {
                let value = unescape(line.value);
                let value = value.trim();
                let uid = value
                    .get(..9)
                    .filter(|p| p.eq_ignore_ascii_case("urn:uuid:"))
                    .and_then(|_| value.get(9..))
                    .unwrap_or(value)
                    .trim();
                if !uid.is_empty() && !group.members.iter().any(|m| m == uid) {
                    group.members.push(uid.to_owned());
                }
            }
            _ => {}
        }
    }
    (!group.name.is_empty()).then_some(group)
}

/// Write a group card, Apple style (vCard 3.0 with
/// `X-ADDRESSBOOKSERVER-KIND:group`, which `CardDAV` servers and Apple,
/// Thunderbird and `SOGo` clients read). With `original`, properties the
/// app doesn't own are kept.
#[must_use]
pub fn write_group(group: &GroupCard, original: Option<&str>) -> String {
    const OWNED: &[&str] = &[
        "FN",
        "N",
        "UID",
        "VERSION",
        "PRODID",
        "REV",
        "KIND",
        "X-ADDRESSBOOKSERVER-KIND",
        "MEMBER",
        "X-ADDRESSBOOKSERVER-MEMBER",
    ];
    let unfolded = original.map(unfold).unwrap_or_default();
    let mut out = String::new();
    fold("BEGIN:VCARD", &mut out);
    fold("VERSION:3.0", &mut out);
    fold("PRODID:-//Mocky Digital//Mocky Mail//EN", &mut out);
    fold(&format!("UID:{}", escape(&group.uid)), &mut out);
    let name = clean(&group.name);
    fold(&format!("FN:{}", escape(&name)), &mut out);
    fold(&format!("N:{};;;;", escape(&name)), &mut out);
    fold("X-ADDRESSBOOKSERVER-KIND:group", &mut out);
    for member in &group.members {
        let member: String = member.chars().filter(|c| !c.is_control()).collect();
        if !member.trim().is_empty() {
            fold(
                &format!("X-ADDRESSBOOKSERVER-MEMBER:urn:uuid:{}", member.trim()),
                &mut out,
            );
        }
    }
    for (line, raw) in card_lines(&unfolded) {
        if !OWNED.contains(&line.name.as_str()) {
            fold(raw, &mut out);
        }
    }
    fold("END:VCARD", &mut out);
    out
}

/// One line of text: control characters dropped, spaces collapsed.
fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The address in an `EMAIL` value.
fn email_of(value: &str) -> String {
    unescape(value)
        .trim()
        .trim_start_matches("mailto:")
        .to_owned()
}

/// The number in a `TEL` value.
fn phone_of(value: &str) -> String {
    clean(unescape(value).trim_start_matches("tel:"))
}

/// Whether a line's parameters mark it as the preferred one.
fn is_preferred(params: &str) -> bool {
    params.to_ascii_uppercase().contains("PREF")
}

/// The content lines inside the first card of an unfolded text, each with
/// its text as written.
fn card_lines(unfolded: &str) -> Vec<(Line<'_>, &str)> {
    let mut out = Vec::new();
    let mut inside = false;
    for raw in unfolded.lines() {
        let Some(line) = split_line(raw) else {
            continue;
        };
        match line.name.as_str() {
            "BEGIN" if line.value.trim().eq_ignore_ascii_case("VCARD") => inside = true,
            "END" if line.value.trim().eq_ignore_ascii_case("VCARD") => break,
            "BEGIN" | "END" => {}
            _ if inside => out.push((line, raw.trim_end_matches('\r'))),
            _ => {}
        }
    }
    out
}

/// A line as written but no longer marked preferred (`TYPE=work,pref`,
/// `PREF=1`), for an address that is no longer the first. `None` when its
/// parameters are too unusual to edit safely.
fn without_preference(line: &Line<'_>, raw: &str) -> Option<String> {
    if line.params.contains('"') {
        return None;
    }
    let head = raw.get(..raw.len().checked_sub(line.value.len())?.checked_sub(1)?)?;
    let name = head.split_once(';').map_or(head, |(name, _)| name);
    let mut out = name.to_owned();
    for param in line.params.split(';') {
        let (key, value) = param.split_once('=').unwrap_or((param, ""));
        if key.trim().eq_ignore_ascii_case("PREF") {
            continue;
        }
        if key.trim().eq_ignore_ascii_case("TYPE") {
            let kinds: Vec<&str> = value
                .split(',')
                .filter(|kind| !kind.trim().eq_ignore_ascii_case("PREF"))
                .collect();
            if !kinds.is_empty() {
                out.push(';');
                out.push_str(key);
                out.push('=');
                out.push_str(&kinds.join(","));
            }
            continue;
        }
        if !param.is_empty() {
            out.push(';');
            out.push_str(param);
        }
    }
    out.push(':');
    out.push_str(line.value);
    Some(out)
}

/// Fold a content line at 75 octets (RFC 6350 3.2), on character bounds.
pub(crate) fn fold(line: &str, out: &mut String) {
    let mut width = 0usize;
    for c in line.chars() {
        let len = c.len_utf8();
        if width.saturating_add(len) > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width = width.saturating_add(len);
    }
    out.push_str("\r\n");
}

/// Write a card for the server. With `original` (the server's copy), its
/// other fields are kept and only the app's fields are replaced. Of the
/// app's own fields, the structured name (`N`) is kept while the display
/// name is the same, and each address and number that is still there
/// keeps its line as written, with its type and group.
#[must_use]
#[allow(clippy::too_many_lines, reason = "one property after another")]
pub fn write(card: &ContactCard, original: Option<&str>) -> String {
    let unfolded = original.map(unfold).unwrap_or_default();
    let old_lines = card_lines(&unfolded);
    // Each original line is reused once at most.
    let mut used: Vec<usize> = Vec::new();
    let mut reuse = |name: &str, same: &dyn Fn(&str) -> bool| {
        let (i, found) = old_lines
            .iter()
            .enumerate()
            .find(|(i, (line, _))| line.name == name && !used.contains(i) && same(line.value))?;
        used.push(i);
        Some(found)
    };

    let mut out = String::new();
    fold("BEGIN:VCARD", &mut out);
    fold("VERSION:3.0", &mut out);
    fold("PRODID:-//Mocky Digital//Mocky Mail//EN", &mut out);
    fold(&format!("UID:{}", escape(&card.uid)), &mut out);
    let name = clean(&card.name);
    let name = if name.is_empty() {
        card.emails.first().cloned().unwrap_or_default()
    } else {
        name
    };
    fold(&format!("FN:{}", escape(&name)), &mut out);
    // The parts of a name ("de la Cruz;Maria;;Dr.;Jr.") can't be worked
    // out again from the display name, so they stay unless it was edited.
    let kept_name = original
        .and_then(parse)
        .filter(|old| old.name == name)
        .and_then(|_| old_lines.iter().find(|(line, _)| line.name == "N"));
    if let Some((_, raw)) = kept_name {
        fold(raw, &mut out);
    } else {
        // N: split the display name into given and family, best effort.
        let (given, family) = match name.rsplit_once(' ') {
            Some((given, family)) if !name.contains('@') => (given.to_owned(), family.to_owned()),
            _ => (name.clone(), String::new()),
        };
        fold(
            &format!("N:{};{};;;", escape(&family), escape(&given)),
            &mut out,
        );
    }
    for (i, email) in card
        .emails
        .iter()
        .map(|e| e.trim())
        .filter(|e| e.contains('@'))
        .enumerate()
    {
        // Reading puts a preferred address first, so only the first may
        // keep that mark, or the order chosen here would not survive.
        let kept = reuse("EMAIL", &|value| email_of(value) == email).and_then(|(line, raw)| {
            if i > 0 && is_preferred(line.params) {
                without_preference(line, raw)
            } else {
                Some((*raw).to_owned())
            }
        });
        let line = kept.unwrap_or_else(|| {
            let pref = if i == 0 {
                ";TYPE=INTERNET,PREF"
            } else {
                ";TYPE=INTERNET"
            };
            format!("EMAIL{pref}:{}", escape(email))
        });
        fold(&line, &mut out);
    }
    for phone in card
        .phones
        .iter()
        .map(|p| clean(p))
        .filter(|p| !p.is_empty())
    {
        match reuse("TEL", &|value| phone_of(value) == phone) {
            Some((_, raw)) => fold(raw, &mut out),
            None => fold(&format!("TEL:{}", escape(&phone)), &mut out),
        }
    }
    if !card.organization.trim().is_empty() {
        fold(
            &format!("ORG:{}", escape(card.organization.trim())),
            &mut out,
        );
    }
    if !card.title.trim().is_empty() {
        fold(&format!("TITLE:{}", escape(card.title.trim())), &mut out);
    }
    if !card.notes.trim().is_empty() {
        fold(&format!("NOTE:{}", escape(card.notes.trim())), &mut out);
    }
    let categories: Vec<String> = card
        .categories
        .iter()
        .map(|c| clean(c))
        .filter(|c| !c.is_empty())
        .map(|c| escape(&c))
        .collect();
    if !categories.is_empty() {
        fold(&format!("CATEGORIES:{}", categories.join(",")), &mut out);
    }
    if card.favorite {
        fold("X-MOCKY-FAVORITE:1", &mut out);
    }
    // The picture: the server's line while it is the same picture; a new
    // one in its place; none when it was removed here. A picture the app
    // doesn't hold (given by address, or too large) stays as it was.
    let old_photo = original.and_then(parse).and_then(|c| c.photo);
    let photo_unchanged = card.photo.is_some() && card.photo == old_photo;
    let keep_old_photos = photo_unchanged || (card.photo.is_none() && old_photo.is_none());
    if !photo_unchanged && let Some(line) = card.photo.as_deref().and_then(photo_line) {
        fold(&line, &mut out);
    }
    // Everything else the original had (addresses, birthdays…).
    for (line, raw) in &old_lines {
        let photo = line.name == "PHOTO";
        if (!photo && !MANAGED.contains(&line.name.as_str())) || (photo && keep_old_photos) {
            fold(raw, &mut out);
        }
    }
    fold("END:VCARD", &mut out);
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const SOGO: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nPRODID:-//Inverse inc.//SOGo//EN\r\nUID:5A1-6B.vcf\r\nN:Mwangi;Peter;;;\r\nFN:Peter Mwangi\r\nORG:Westlands Fit-out;Projects\r\nTITLE:Site manager\r\nEMAIL;TYPE=work:peter@client.example\r\nEMAIL;TYPE=home,pref:peter.m@gmail.example\r\nTEL;TYPE=cell:+254 700 000 001\r\nNOTE:Prefers calls before 10\\, not after 5.\\nGate code 44.\r\nADR;TYPE=work:;;Ring Rd;Nairobi;;;Kenya\r\nPHOTO;ENCODING=b;TYPE=JPEG:/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAMCAgMCAgMDAwMEAwMEBQgFBQQEBQoHBwYIDAoMDAsKCwsN\r\n DhIQDQ4RDgsLEBYQERMUFRUVDA8XGBYUGBIUFRT/2wBDAQMEBAUEBQkFBQkUDQsNFBQUFBQUFBQU\r\nEND:VCARD\r\n";

    #[test]
    fn reads_what_the_app_shows() {
        let c = parse(SOGO).unwrap();
        assert_eq!(c.uid, "5A1-6B.vcf");
        assert_eq!(c.name, "Peter Mwangi");
        assert_eq!(c.emails, ["peter.m@gmail.example", "peter@client.example"]);
        assert_eq!(c.phones, ["+254 700 000 001"]);
        assert_eq!(c.organization, "Westlands Fit-out");
        assert_eq!(c.title, "Site manager");
        assert_eq!(
            c.notes,
            "Prefers calls before 10, not after 5.\nGate code 44."
        );
    }

    #[test]
    fn uses_the_structured_name_without_fn_and_skips_empty_cards() {
        let c = parse("BEGIN:VCARD\nVERSION:4.0\nN:Hassan;Amina;;Dr.;\nitem1.EMAIL:amina@studio.example\nEND:VCARD").unwrap();
        assert_eq!(c.name, "Dr. Amina Hassan");
        assert_eq!(c.emails, ["amina@studio.example"]);
        assert_eq!(parse("BEGIN:VCARD\nVERSION:3.0\nUID:x\nEND:VCARD"), None);
        assert_eq!(parse("not a card"), None);
    }

    #[test]
    fn editing_keeps_fields_the_app_does_not_show() {
        let mut c = parse(SOGO).unwrap();
        c.title = "Director".into();
        c.phones.push("+254 711 000 002".into());
        let out = write(&c, Some(SOGO));
        assert!(out.contains("TITLE:Director\r\n"));
        assert!(!out.contains("Site manager"));
        assert!(out.contains("ADR;TYPE=work:;;Ring Rd;Nairobi;;;Kenya"));
        assert!(out.contains("PHOTO;ENCODING=b;TYPE=JPEG:/9j/4AAQ"));
        assert_eq!(out.matches("BEGIN:VCARD").count(), 1);
        assert_eq!(out.matches("UID:").count(), 1);
        // Round trip.
        assert_eq!(parse(&out).unwrap(), c);
        // Lines are folded to 75 octets.
        assert!(out.split("\r\n").all(|l| l.len() <= 75));
    }

    const APPLE: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:m-1\r\nN:de la Cruz;Maria;;Dr.;Jr.\r\nFN:Dr. Maria de la Cruz Jr.\r\nitem1.EMAIL;type=INTERNET;type=WORK:maria@clinic.example\r\nitem1.X-ABLabel:Clinic\r\nEMAIL;TYPE=HOME,PREF:maria@home.example\r\nTEL;TYPE=CELL:+254 700 000 009\r\nTEL;TYPE=WORK,VOICE:+254 20 000 0009\r\nEND:VCARD\r\n";

    #[test]
    fn editing_a_note_keeps_the_name_parts_types_and_groups() {
        let mut c = parse(APPLE).unwrap();
        c.notes = "Met at the expo".into();
        let out = write(&c, Some(APPLE));
        assert!(out.contains("N:de la Cruz;Maria;;Dr.;Jr.\r\n"));
        assert!(out.contains("item1.EMAIL;type=INTERNET;type=WORK:maria@clinic.example\r\n"));
        assert!(out.contains("item1.X-ABLabel:Clinic\r\n"));
        assert!(out.contains("EMAIL;TYPE=HOME,PREF:maria@home.example\r\n"));
        assert!(out.contains("TEL;TYPE=CELL:+254 700 000 009\r\n"));
        assert!(out.contains("TEL;TYPE=WORK,VOICE:+254 20 000 0009\r\n"));
        assert!(out.contains("NOTE:Met at the expo\r\n"));
        assert_eq!(out.matches("EMAIL").count(), 2);
        assert_eq!(out.matches("TEL").count(), 2);
        assert_eq!(out.matches("\r\nN:").count(), 1);
        assert_eq!(parse(&out).unwrap(), c);
    }

    #[test]
    fn edited_fields_are_rewritten_and_the_rest_reused() {
        let mut c = parse(APPLE).unwrap();
        assert_eq!(c.emails, ["maria@home.example", "maria@clinic.example"]);
        // A new name, the work address made the main one, one number
        // changed and one address added.
        c.name = "Maria Santos".into();
        c.emails = vec![
            "maria@clinic.example".into(),
            "maria@home.example".into(),
            "maria@new.example".into(),
        ];
        c.phones = vec!["+254 700 000 009".into(), "+254 20 111 1111".into()];
        let out = write(&c, Some(APPLE));
        assert!(out.contains("N:Santos;Maria;;;\r\n"));
        assert!(!out.contains("de la Cruz"));
        assert!(out.contains("item1.EMAIL;type=INTERNET;type=WORK:maria@clinic.example\r\n"));
        // No longer first: its type stays, its preference goes.
        assert!(out.contains("EMAIL;TYPE=HOME:maria@home.example\r\n"));
        assert!(out.contains("EMAIL;TYPE=INTERNET:maria@new.example\r\n"));
        assert!(out.contains("TEL;TYPE=CELL:+254 700 000 009\r\n"));
        assert!(out.contains("TEL:+254 20 111 1111\r\n"));
        assert!(!out.contains("000 0009"));
        assert_eq!(parse(&out).unwrap(), c);
    }

    #[test]
    fn writes_new_cards_and_escapes_text() {
        let c = ContactCard {
            uid: "u-1".into(),
            name: "Wanjiru Kamau".into(),
            emails: vec!["wanjiru@shop.example".into()],
            phones: vec![],
            organization: "Kamau; Sons".into(),
            title: String::new(),
            notes: "Line one\nLine, two".into(),
            ..ContactCard::default()
        };
        let out = write(&c, None);
        assert!(out.starts_with("BEGIN:VCARD\r\nVERSION:3.0\r\n"));
        assert!(out.contains("N:Kamau;Wanjiru;;;\r\n"));
        assert!(out.contains("EMAIL;TYPE=INTERNET,PREF:wanjiru@shop.example\r\n"));
        assert!(out.contains("ORG:Kamau\\; Sons\r\n"));
        assert!(out.contains("NOTE:Line one\\nLine\\, two\r\n"));
        assert_eq!(parse(&out).unwrap(), c);
    }

    /// A 1x1 PNG.
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

    #[test]
    fn favourites_categories_and_photos() {
        let photo = format!("data:image/png;base64,{PNG}");
        let c = ContactCard {
            uid: "p-1".into(),
            name: "Amina Hassan".into(),
            emails: vec!["amina@studio.example".into()],
            favorite: true,
            categories: vec!["Clients".into(), "Red, urgent".into()],
            photo: Some(photo.clone()),
            ..ContactCard::default()
        };
        let out = write(&c, None);
        assert!(out.contains("X-MOCKY-FAVORITE:1\r\n"));
        assert!(out.contains("CATEGORIES:Clients,Red\\, urgent\r\n"));
        assert!(unfold(&out).contains(&format!("PHOTO;ENCODING=b;TYPE=PNG:{PNG}")));
        assert_eq!(parse(&out).unwrap(), c);

        // vCard 4 data URLs are read too; pictures by address are not
        // fetched, and stay on the card when it is edited.
        let v4 = format!(
            "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:A\r\nPHOTO:data:image/png;base64,{PNG}\r\nEND:VCARD\r\n"
        );
        assert_eq!(parse(&v4).unwrap().photo.as_deref(), Some(photo.as_str()));
        let by_address = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:A\r\nPHOTO;VALUE=uri:https://lh3.example/p.jpg\r\nEND:VCARD\r\n";
        let read = parse(by_address).unwrap();
        assert_eq!(read.photo, None);
        assert!(
            write(&read, Some(by_address)).contains("PHOTO;VALUE=uri:https://lh3.example/p.jpg")
        );
        // Removed here: gone from the card; replaced: only the new one.
        let removed = ContactCard {
            photo: None,
            ..parse(&v4).unwrap()
        };
        assert!(!write(&removed, Some(&v4)).contains("PHOTO"));
        let replaced = ContactCard {
            photo: Some(format!("data:image/png;base64,{PNG}")),
            ..parse(by_address).unwrap()
        };
        assert_eq!(
            write(&replaced, Some(by_address)).matches("PHOTO").count(),
            1
        );

        // Pictures from the editor are checked.
        assert_eq!(checked_photo(&photo).unwrap(), photo);
        assert!(checked_photo("https://x.example/a.png").is_err());
        assert!(checked_photo("data:image/png;base64,bm90IGEgcGljdHVyZQ==").is_err());
        let huge = format!("data:image/png;base64,{}", "A".repeat(300_000));
        assert!(checked_photo(&huge).is_err());
    }

    #[test]
    fn group_cards() {
        let apple = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:g-1\r\nFN:Site team\r\nN:Site team;;;;\r\nX-ADDRESSBOOKSERVER-KIND:group\r\nX-ADDRESSBOOKSERVER-MEMBER:urn:uuid:p-1\r\nX-ADDRESSBOOKSERVER-MEMBER:urn:uuid:p-2\r\nEND:VCARD\r\n";
        assert!(is_group(apple));
        // A group is not a person.
        assert_eq!(parse(apple), None);
        let g = parse_group(apple).unwrap();
        assert_eq!(g.uid, "g-1");
        assert_eq!(g.name, "Site team");
        assert_eq!(g.members, ["p-1", "p-2"]);
        let v4 = "BEGIN:VCARD\r\nVERSION:4.0\r\nKIND:group\r\nFN:Board\r\nUID:g-2\r\nMEMBER:urn:uuid:p-9\r\nEND:VCARD\r\n";
        assert_eq!(parse_group(v4).unwrap().members, ["p-9"]);
        let edited = GroupCard {
            members: vec!["p-2".into(), "p-3".into()],
            ..g
        };
        let out = write_group(&edited, Some(apple));
        assert_eq!(parse_group(&out).unwrap(), edited);
        assert_eq!(out.matches("X-ADDRESSBOOKSERVER-KIND").count(), 1);
        assert!(parse_group(SOGO).is_none());
    }

    #[test]
    fn never_panics_on_odd_input() {
        for odd in [
            "",
            ":",
            "BEGIN:VCARD\n:\n;;;\nEMAIL\nEND:VCARD",
            "BEGIN:VCARD\nN:\\\nFN:\\",
            "\u{0}\u{1}BEGIN:VCARD",
        ] {
            let _ = parse(odd);
        }
        assert_eq!(parse(&"x".repeat(MAX_VCARD_BYTES + 1)), None);
    }
}
