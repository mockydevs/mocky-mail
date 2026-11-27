//! `.vcf` files: any number of contacts, in vCard 2.1 (Outlook, older
//! phones), 3.0 or 4.0, for importing; and an address book written out as
//! vCard 3.0.
//!
//! vCard 2.1 is not 3.0 with another number: its values may be
//! quoted-printable (with soft line breaks that aren't folded lines) and in
//! any character set (`CHARSET=`), and its photos are base64 continued
//! over indented lines. Each card is turned into plain UTF-8 lines first,
//! as 3.0 writes them, and then read like any other card, so every field
//! [`crate::vcard::write`] doesn't manage (addresses, birthdays, photos) is
//! kept with it.

use crate::vcard::{self, ContactCard, MAX_VCARD_BYTES};

/// One card read from a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCard {
    /// What the app shows of it.
    pub card: ContactCard,
    /// The whole card as UTF-8 vCard lines, kept so that saving it keeps
    /// every field.
    pub vcard: String,
}

/// What a file held.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CardFile {
    /// The cards that could be read.
    pub cards: Vec<FileCard>,
    /// Cards left out (empty, or too large even without their pictures).
    pub skipped: usize,
}

/// Properties dropped from a card too large to keep whole.
const LARGE: &[&str] = &["PHOTO", "LOGO", "SOUND", "KEY"];

/// The file as text: UTF-16 by its byte order mark, else bytes as they
/// are (each value is decoded on its own, by its `CHARSET`).
fn file_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.starts_with(b"\xFF\xFE") {
        return mailcraft_charset::decode(bytes, "utf-16le").into_bytes();
    }
    if bytes.starts_with(b"\xFE\xFF") {
        return mailcraft_charset::decode(bytes, "utf-16be").into_bytes();
    }
    bytes
        .strip_prefix(b"\xEF\xBB\xBF")
        .unwrap_or(bytes)
        .to_vec()
}

/// A logical line: the physical lines it was folded over, joined.
struct RawLine {
    bytes: Vec<u8>,
    quoted_printable: bool,
}

fn is_quoted_printable(head: &[u8]) -> bool {
    let head = String::from_utf8_lossy(head).to_ascii_uppercase();
    head.split(';')
        .skip(1)
        .any(|p| p.trim() == "QUOTED-PRINTABLE" || p.trim() == "ENCODING=QUOTED-PRINTABLE")
}

/// The part of a line before its value (name and parameters).
fn head_of(line: &[u8]) -> &[u8] {
    let mut quoted = false;
    for (i, b) in line.iter().enumerate() {
        match b {
            b'"' => quoted = !quoted,
            b':' if !quoted => return line.get(..i).unwrap_or(line),
            _ => {}
        }
    }
    line
}

fn logical_lines(text: &[u8]) -> Vec<RawLine> {
    let mut out: Vec<RawLine> = Vec::new();
    for physical in text.split(|b| *b == b'\n') {
        let physical = physical.strip_suffix(b"\r").unwrap_or(physical);
        if let Some(last) = out.last_mut() {
            // A quoted-printable value goes on after a soft line break
            // (`=` at the end), whatever the next line starts with.
            if last.quoted_printable && last.bytes.ends_with(b"=") {
                last.bytes.pop();
                last.bytes.extend_from_slice(physical);
                continue;
            }
            if let Some(rest) = physical
                .strip_prefix(b" ")
                .or_else(|| physical.strip_prefix(b"\t"))
            {
                last.bytes.extend_from_slice(rest);
                continue;
            }
        }
        if physical.iter().all(u8::is_ascii_whitespace) {
            // Blank lines end a 2.1 base64 value; they hold nothing.
            continue;
        }
        let quoted_printable = is_quoted_printable(head_of(physical));
        out.push(RawLine {
            bytes: physical.to_vec(),
            quoted_printable,
        });
    }
    out
}

fn hex(b: u8) -> Option<u8> {
    char::from(b)
        .to_digit(16)
        .and_then(|d| u8::try_from(d).ok())
}

/// Quoted-printable undone; a broken escape is kept as written.
fn decode_quoted_printable(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len());
    let mut i = 0usize;
    while let Some(&b) = value.get(i) {
        if b == b'='
            && let (Some(h), Some(l)) = (
                value.get(i.saturating_add(1)).copied().and_then(hex),
                value.get(i.saturating_add(2)).copied().and_then(hex),
            )
        {
            out.push(h.saturating_mul(16).saturating_add(l));
            i = i.saturating_add(3);
            continue;
        }
        out.push(b);
        i = i.saturating_add(1);
    }
    out
}

/// One logical line as UTF-8 vCard 3.0 text: its value decoded (quoted-
/// printable, `CHARSET`), and the parameters that said how dropped.
fn normalize(line: &RawLine) -> String {
    let head = head_of(&line.bytes);
    let value = line
        .bytes
        .get(head.len().saturating_add(1)..)
        .unwrap_or_default();
    let head_text = String::from_utf8_lossy(head).into_owned();
    let mut parts = head_text.split(';');
    let name = parts.next().unwrap_or_default().trim().to_owned();
    let mut charset: Option<String> = None;
    let mut base64 = false;
    let mut kept: Vec<String> = Vec::new();
    for param in parts {
        let upper = param.trim().to_ascii_uppercase();
        if let Some(set) = upper.strip_prefix("CHARSET=") {
            charset = Some(set.trim_matches('"').to_owned());
            continue;
        }
        match upper.as_str() {
            "QUOTED-PRINTABLE" | "ENCODING=QUOTED-PRINTABLE" | "" => {}
            "BASE64" | "ENCODING=BASE64" | "ENCODING=B" => base64 = true,
            _ => kept.push(param.trim().to_owned()),
        }
    }
    let bytes = if line.quoted_printable {
        decode_quoted_printable(value)
    } else {
        value.to_vec()
    };
    let mut text = match charset.as_deref() {
        Some(set) => mailcraft_charset::decode(&bytes, set),
        None => mailcraft_charset::guess(&bytes),
    };
    if base64 {
        text.retain(|c| !c.is_whitespace());
        kept.push("ENCODING=b".to_owned());
    } else if line.quoted_printable {
        // Line breaks inside a value are written `\n` from 3.0 on.
        text = text.replace("\r\n", "\\n").replace(['\n', '\r'], "\\n");
    }
    let mut out = name;
    for param in kept {
        out.push(';');
        out.push_str(&param);
    }
    out.push(':');
    out.push_str(&text);
    out
}

fn name_of(line: &str) -> String {
    let head = line.split(':').next().unwrap_or_default();
    let name = head.split(';').next().unwrap_or_default();
    name.rsplit('.')
        .next()
        .unwrap_or(name)
        .trim()
        .to_ascii_uppercase()
}

fn card_text(lines: &[String], leave_out: &[&str]) -> String {
    let mut out = String::new();
    for line in lines {
        if !leave_out.contains(&name_of(line).as_str()) {
            vcard::fold(line, &mut out);
        }
    }
    out
}

/// Read every card in a `.vcf` file.
#[must_use]
pub fn read_cards(bytes: &[u8]) -> CardFile {
    let text = file_bytes(bytes);
    let mut file = CardFile::default();
    let mut current: Vec<String> = Vec::new();
    let mut depth = 0usize;
    for raw in logical_lines(&text) {
        let line = normalize(&raw);
        let name = name_of(&line);
        let value = line
            .split_once(':')
            .map(|(_, v)| v.trim().to_ascii_uppercase())
            .unwrap_or_default();
        if name == "BEGIN" && value == "VCARD" {
            depth = depth.saturating_add(1);
            if depth == 1 {
                current.clear();
            }
        }
        if depth > 0 {
            current.push(line);
        }
        if name == "END" && value == "VCARD" && depth > 0 {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                let mut text = card_text(&current, &[]);
                if text.len() > MAX_VCARD_BYTES {
                    text = card_text(&current, LARGE);
                }
                match vcard::parse(&text) {
                    Some(card) if text.len() <= MAX_VCARD_BYTES => {
                        file.cards.push(FileCard { card, vcard: text });
                    }
                    _ => file.skipped = file.skipped.saturating_add(1),
                }
                current.clear();
            }
        }
    }
    file
}

/// Write cards as one `.vcf` file (vCard 3.0): each from what the app
/// shows and, when known, the card it was read from (`original`), whose
/// other fields are kept.
#[must_use]
pub fn write_cards<'a>(
    cards: impl IntoIterator<Item = (&'a ContactCard, Option<&'a str>)>,
) -> String {
    let mut out = String::new();
    for (card, original) in cards {
        out.push_str(&vcard::write(card, original));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn reads_outlook_vcard_2_1_with_quoted_printable_and_charsets() {
        let file: &[u8] = b"BEGIN:VCARD\r\nVERSION:2.1\r\nN;CHARSET=Windows-1252:M\xFCller;J\xFCrgen\r\nFN;CHARSET=Windows-1252:J\xFCrgen M\xFCller\r\nTEL;WORK;VOICE:+49 30 1234\r\nEMAIL;PREF;INTERNET:juergen@firma.example\r\nNOTE;ENCODING=QUOTED-PRINTABLE;CHARSET=UTF-8:Erste Zeile=0D=0AZweite Zeile mit =C3=BC und ein sehr langer Text der=\r\n weitergeht\r\nADR;WORK;ENCODING=QUOTED-PRINTABLE:;;Hauptstr. 1=0D=0AHof;Berlin;;10115;Deutschland\r\nPHOTO;TYPE=JPEG;ENCODING=BASE64:\r\n /9j/4AAQSkZJRgAB\r\n AQAAAQABAAD\r\n\r\nEND:VCARD\r\n";
        let read = read_cards(file);
        assert_eq!(read.skipped, 0);
        let c = &read.cards[0];
        assert_eq!(c.card.name, "J\u{fc}rgen M\u{fc}ller");
        assert_eq!(c.card.emails, ["juergen@firma.example"]);
        assert_eq!(c.card.phones, ["+49 30 1234"]);
        assert_eq!(
            c.card.notes,
            "Erste Zeile\nZweite Zeile mit \u{fc} und ein sehr langer Text der weitergeht"
        );
        assert!(
            c.vcard
                .contains("ADR;WORK:;;Hauptstr. 1\\nHof;Berlin;;10115;Deutschland")
        );
        assert!(
            c.vcard
                .contains("PHOTO;TYPE=JPEG;ENCODING=b:/9j/4AAQSkZJRgABAQAAAQABAAD")
        );
        assert!(!c.vcard.contains("CHARSET") && !c.vcard.contains("QUOTED-PRINTABLE"));
    }

    #[test]
    fn reads_legacy_charsets() {
        let file: &[u8] = b"BEGIN:VCARD\nVERSION:2.1\nFN;CHARSET=KOI8-R:\xF0\xC5\xD4\xD2 \xE9\xD7\xC1\xCE\xCF\xD7\nEND:VCARD\nBEGIN:VCARD\nVERSION:2.1\nFN;CHARSET=SHIFT_JIS:\x93\xFA\x96\x7B\nEND:VCARD\nBEGIN:VCARD\nVERSION:2.1\nFN;CHARSET=BIG5:\xA4\xA4\xA4\xE5\nEND:VCARD\nBEGIN:VCARD\nVERSION:2.1\nFN;CHARSET=GB2312:\xD6\xD0\xCE\xC4\nEND:VCARD\nBEGIN:VCARD\nVERSION:2.1\nFN;CHARSET=EUC-KR:\xC7\xD1\xB1\xB9\nEND:VCARD\n";
        let names: Vec<String> = read_cards(file)
            .cards
            .into_iter()
            .map(|c| c.card.name)
            .collect();
        assert_eq!(
            names,
            [
                "\u{41f}\u{435}\u{442}\u{440} \u{418}\u{432}\u{430}\u{43d}\u{43e}\u{432}",
                "\u{65e5}\u{672c}",
                "\u{4e2d}\u{6587}",
                "\u{4e2d}\u{6587}",
                "\u{d55c}\u{ad6d}"
            ]
        );
    }

    #[test]
    fn reads_3_0_and_4_0_files_and_utf16() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Amina Hassan\r\nEMAIL;TYPE=work:amina@studio.example\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:4.0\r\nFN:Peter\r\n  Mwangi\r\nTEL;VALUE=uri:tel:+254700000001\r\nEND:VCARD\r\n";
        let read = read_cards(text.as_bytes());
        assert_eq!(read.cards.len(), 2);
        assert_eq!(read.cards[1].card.name, "Peter Mwangi");
        assert_eq!(read.cards[1].card.phones, ["+254700000001"]);
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(read_cards(&utf16), read);
    }

    #[test]
    fn empty_cards_are_skipped_and_large_photos_dropped() {
        let empty = b"BEGIN:VCARD\nVERSION:3.0\nUID:x\nEND:VCARD\n";
        assert_eq!(read_cards(empty).skipped, 1);
        let photo = "A".repeat(MAX_VCARD_BYTES + 10);
        let big = format!(
            "BEGIN:VCARD\nVERSION:3.0\nFN:Big Photo\nPHOTO;ENCODING=b:{photo}\nEND:VCARD\n"
        );
        let read = read_cards(big.as_bytes());
        assert_eq!(read.cards[0].card.name, "Big Photo");
        assert!(!read.cards[0].vcard.contains("PHOTO"));
        assert_eq!(read_cards(b"\xFF\x00garbage"), CardFile::default());
    }

    #[test]
    fn export_and_import_round_trip() {
        let file: &[u8] = b"BEGIN:VCARD\r\nVERSION:2.1\r\nN:Kamau;Wanjiru\r\nFN:Wanjiru Kamau\r\nORG:Kamau Sons\r\nTITLE:Owner\r\nTEL;CELL:+254 711 000 002\r\nEMAIL;INTERNET:wanjiru@shop.example\r\nBDAY:1990-04-01\r\nEND:VCARD\r\n";
        let first = read_cards(file);
        let written = write_cards(
            first
                .cards
                .iter()
                .map(|c| (&c.card, Some(c.vcard.as_str()))),
        );
        assert!(written.starts_with("BEGIN:VCARD\r\nVERSION:3.0\r\n"));
        assert!(written.contains("BDAY:1990-04-01\r\n"));
        let second = read_cards(written.as_bytes());
        assert_eq!(second.cards.len(), 1);
        assert_eq!(second.cards[0].card, first.cards[0].card);
        // Written again from the second read: the same file.
        let again = write_cards(
            second
                .cards
                .iter()
                .map(|c| (&c.card, Some(c.vcard.as_str()))),
        );
        assert_eq!(again, written);
    }

    proptest::proptest! {
        #[test]
        fn any_bytes_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..1024)) {
            let mut file = b"BEGIN:VCARD\nN;ENCODING=QUOTED-PRINTABLE;CHARSET=x:".to_vec();
            file.extend_from_slice(&bytes);
            file.extend_from_slice(b"\nEND:VCARD\n");
            let _ = read_cards(&file);
            let _ = read_cards(&bytes);
        }
    }
}
