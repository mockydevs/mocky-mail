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
}

/// Properties [`write`] owns; everything else in an original card is kept.
const MANAGED: &[&str] = &[
    "FN", "N", "EMAIL", "TEL", "ORG", "TITLE", "NOTE", "UID", "VERSION", "PRODID", "REV",
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
    // Everything else the original had (photos, addresses, birthdays…).
    for (line, raw) in &old_lines {
        if !MANAGED.contains(&line.name.as_str()) {
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
        };
        let out = write(&c, None);
        assert!(out.starts_with("BEGIN:VCARD\r\nVERSION:3.0\r\n"));
        assert!(out.contains("N:Kamau;Wanjiru;;;\r\n"));
        assert!(out.contains("EMAIL;TYPE=INTERNET,PREF:wanjiru@shop.example\r\n"));
        assert!(out.contains("ORG:Kamau\\; Sons\r\n"));
        assert!(out.contains("NOTE:Line one\\nLine\\, two\r\n"));
        assert_eq!(parse(&out).unwrap(), c);
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
