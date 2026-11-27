//! Outlook `.pst` files, read for an import: their folders, and each item
//! turned into what the app keeps. Mail becomes an RFC 5322 message (with
//! the headers it was received with when Outlook kept them), contacts
//! become vCards and calendar items become iCalendar resources. Tasks,
//! notes and journal entries are counted, for a later update.
//!
//! The file is read with `mailcraft-pst` over any `Read + Seek`, a block
//! at a time: never the whole file. A `.pst` file is hostile input; the
//! reader checks every offset and size, and what can't be read of one item
//! is named in that item's notes while the rest is brought in.

use std::io::{Read, Seek};

use base64::Engine as _;
use mailcraft_pst::{
    AttachmentContent, MessageRef, Nid, PropName, PropValue, Properties, Pst, RecipientKind,
};

use crate::vcard::{ContactCard, escape, fold};

/// Most bytes of one body (text, HTML or rich text) brought in.
const MAX_BODY: u64 = 16 * 1024 * 1024;
/// Most bytes of the headers a message was received with.
const MAX_HEADERS: u64 = 1024 * 1024;
/// How deep attached messages are opened.
const MAX_DEPTH: usize = 4;

const PSETID_APPOINTMENT: [u8; 16] = guid(0x0006_2002);
const PSETID_ADDRESS: [u8; 16] = guid(0x0006_2004);
const PSETID_COMMON: [u8; 16] = guid(0x0006_2008);
/// `{6ED8DA90-450B-101B-98DA-00AA003F1305}` as a file keeps it.
const PSETID_MEETING: [u8; 16] = [
    0x90, 0xDA, 0xD8, 0x6E, 0x0B, 0x45, 0x1B, 0x10, 0x98, 0xDA, 0x00, 0xAA, 0x00, 0x3F, 0x13, 0x05,
];

/// `{000620xx-0000-0000-C000-000000000046}` as a file keeps it (the first
/// three fields little-endian).
const fn guid(first: u32) -> [u8; 16] {
    let b = first.to_le_bytes();
    [
        b[0], b[1], b[2], b[3], 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46,
    ]
}

/// Why a file can't be read at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PstError {
    /// It isn't a `.pst` file, or it is damaged past reading.
    #[error("this isn't an Outlook data file that can be read ({0})")]
    Unreadable(String),
    /// It has a password.
    #[error(
        "this Outlook data file has a password. Remove it in Outlook (File > Account Settings > Data Files > Settings > Change Password), then import it again"
    )]
    Password,
}

fn unreadable(error: &mailcraft_pst::Error) -> PstError {
    PstError::Unreadable(error.to_string())
}

/// What a folder holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderKind {
    /// Mail (and anything not named otherwise).
    Mail,
    /// Contacts.
    Contacts,
    /// Calendar items.
    Calendar,
    /// Tasks.
    Tasks,
    /// Sticky notes.
    Notes,
    /// Journal entries.
    Journal,
}

/// A folder of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PstFolder {
    /// The file's id for it.
    pub id: u32,
    /// Its names from the top of the owner's folders down (empty for the
    /// top itself).
    pub path: Vec<String>,
    /// What it holds.
    pub kind: FolderKind,
    /// How many items it says it holds.
    pub count: u32,
}

/// What an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// A message (mail, meeting request, report, post).
    Mail,
    /// A contact.
    Contact,
    /// A contact group, not brought in.
    ContactGroup,
    /// A calendar item.
    Event,
    /// A task.
    Task,
    /// A sticky note.
    Note,
    /// A journal entry.
    Journal,
}

/// An item of a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PstItem {
    /// The file's id for it.
    pub id: u32,
    /// What it is.
    pub kind: ItemKind,
}

/// A message built from an item.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct BuiltMessage {
    /// RFC 5322, with `\r\n` line ends.
    pub raw: Vec<u8>,
    /// When it arrived (Unix seconds), for the server's `INTERNALDATE`.
    pub received: Option<i64>,
    /// Its Message-ID, without angle brackets.
    pub message_id: Option<String>,
    /// Read.
    pub seen: bool,
    /// Flagged.
    pub flagged: bool,
    /// Replied to.
    pub answered: bool,
    /// What of it couldn't be brought in.
    pub notes: Vec<String>,
}

/// A contact built from an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltContact {
    /// What the app shows.
    pub card: ContactCard,
    /// The whole card (vCard 3.0), with addresses and birthday.
    pub vcard: String,
}

/// A calendar item built from an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltEvent {
    /// Its `UID`.
    pub uid: String,
    /// The resource (`VCALENDAR`).
    pub ics: String,
    /// What of it couldn't be brought in.
    pub notes: Vec<String>,
}

/// The ids this file gives the named properties the import reads.
#[derive(Debug, Clone, Copy, Default)]
struct Named {
    start: Option<u16>,
    end: Option<u16>,
    location: Option<u16>,
    all_day: Option<u16>,
    recur: Option<u16>,
    recurring: Option<u16>,
    zone: Option<u16>,
    reminder_minutes: Option<u16>,
    reminder_set: Option<u16>,
    clean_id: Option<u16>,
    global_id: Option<u16>,
    emails: [Option<u16>; 3],
    work_address: [Option<u16>; 5],
}

/// An Outlook data file opened for importing.
pub struct PstFile<R> {
    pst: Pst<R>,
    named: Named,
    /// The file's own name for itself, for the ids of what it holds.
    key: String,
    counter: u32,
}

impl<R> std::fmt::Debug for PstFile<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PstFile")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// FILETIME (100 ns since 1601) to Unix seconds, for a date a message can
/// have.
fn unix(filetime: u64) -> Option<i64> {
    let seconds = i64::try_from(filetime / 10_000_000)
        .ok()?
        .checked_sub(11_644_473_600)?;
    (seconds > 0 && seconds < 253_402_300_800).then_some(seconds)
}

/// How the 8-bit text of an item is read: its own code page, else
/// Windows-1252.
#[derive(Debug, Clone, Copy)]
struct Text {
    charset: &'static str,
}

impl Text {
    fn of(p: &Properties) -> Self {
        let own = [0x3FFD, 0x3FDE]
            .into_iter()
            .filter_map(|id| p.int(id))
            .find_map(mailcraft_pst::code_page_label);
        Self {
            charset: own.unwrap_or("windows-1252"),
        }
    }

    fn decode(self, bytes: &[u8]) -> String {
        let end = bytes
            .iter()
            .rposition(|b| *b != 0)
            .map_or(0, |i| i.saturating_add(1));
        mailcraft_charset::decode(bytes.get(..end).unwrap_or_default(), self.charset)
    }

    fn value(self, v: &PropValue) -> Option<String> {
        match v {
            PropValue::String(s) => Some(s.trim_end_matches('\0').to_owned()),
            PropValue::String8(b) => Some(self.decode(b)),
            _ => None,
        }
    }

    /// A short text (a name, an address, a subject): one line, trimmed.
    fn short(self, p: &Properties, id: u16) -> Option<String> {
        let s = self.value(p.get(id)?)?;
        let s: String = s.chars().filter(|c| !c.is_control()).take(4096).collect();
        let s = s.trim();
        (!s.is_empty()).then(|| s.to_owned())
    }

    fn lines(self, p: &Properties, id: u16) -> Option<String> {
        let s = self.value(p.get(id)?)?;
        let s = s.replace("\r\n", "\n");
        let s = s.trim();
        (!s.is_empty()).then(|| s.to_owned())
    }
}

/// Header text for any characters: as it is when it is plain ASCII, else
/// RFC 2047 encoded words.
fn header_text(text: &str) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    if clean.is_ascii() {
        return clean;
    }
    let mut words = Vec::new();
    let mut chunk = String::new();
    for c in clean.chars() {
        if chunk.len().saturating_add(c.len_utf8()) > 45 {
            words.push(std::mem::take(&mut chunk));
        }
        chunk.push(c);
    }
    if !chunk.is_empty() {
        words.push(chunk);
    }
    words
        .iter()
        .map(|w| {
            format!(
                "=?UTF-8?B?{}?=",
                base64::engine::general_purpose::STANDARD.encode(w.as_bytes())
            )
        })
        .collect::<Vec<_>>()
        .join("\r\n ")
}

/// A mailbox for an address header.
fn mailbox(name: &str, address: &str) -> String {
    let address: String = address
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control() && !matches!(c, '<' | '>' | ','))
        .collect();
    let name = name.trim();
    if name.is_empty() || name == address {
        return format!("<{address}>");
    }
    if name.is_ascii() {
        let quoted = name.replace(['"', '\\'], "");
        format!("\"{quoted}\" <{address}>")
    } else {
        format!("{} <{address}>", header_text(name))
    }
}

/// A person from their name and addresses: the SMTP address, else the
/// address when it is one (an Exchange address is a directory name).
fn person(text: Text, p: &Properties, [name, smtp, address]: [u16; 3]) -> Option<(String, String)> {
    let name = text.short(p, name).unwrap_or_default();
    let email = text
        .short(p, smtp)
        .filter(|e| e.contains('@'))
        .or_else(|| text.short(p, address).filter(|e| e.contains('@')))?;
    Some((name, email))
}

/// The message ids in a `References`-like value, each with its brackets.
fn ids(value: &str) -> Vec<String> {
    value
        .split(|c: char| c.is_whitespace() || c == ',')
        .map(|id| id.trim().trim_start_matches('<').trim_end_matches('>'))
        .filter(|id| id.contains('@') && !id.contains(['<', '>']))
        .map(|id| format!("<{id}>"))
        .collect()
}

/// RFC 5322 date.
fn rfc_date(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0)
        .unwrap_or_default()
        .format("%a, %d %b %Y %H:%M:%S +0000")
        .to_string()
}

fn base64_lines(data: &[u8], out: &mut Vec<u8>) {
    let encoded = base64::engine::general_purpose::STANDARD.encode(data);
    for chunk in encoded.as_bytes().chunks(76) {
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\r\n");
    }
}

/// A file name for a header parameter: quoted ASCII, or RFC 2231.
fn file_param(key: &str, name: &str) -> String {
    use std::fmt::Write as _;
    let clean = crate::message::safe_file_name(name);
    if clean.is_ascii() {
        format!("{key}=\"{}\"", clean.replace(['"', '\\'], "_"))
    } else {
        let mut encoded = String::new();
        for b in clean.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_') {
                encoded.push(char::from(b));
            } else {
                let _ = write!(encoded, "%{b:02X}");
            }
        }
        format!("{key}*=UTF-8''{encoded}")
    }
}

/// One part of the message being built.
enum Part {
    Text {
        subtype: &'static str,
        text: String,
    },
    File {
        name: String,
        mime: String,
        content_id: Option<String>,
        data: Vec<u8>,
    },
    Message {
        name: String,
        raw: Vec<u8>,
    },
}

fn write_part(part: &Part, out: &mut Vec<u8>) {
    match part {
        Part::Text { subtype, text } => {
            out.extend_from_slice(
                format!("Content-Type: text/{subtype}; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n").as_bytes(),
            );
            base64_lines(text.as_bytes(), out);
        }
        Part::File {
            name,
            mime,
            content_id,
            data,
        } => {
            out.extend_from_slice(
                format!("Content-Type: {mime}; {}\r\n", file_param("name", name)).as_bytes(),
            );
            let disposition = if content_id.is_some() {
                "inline"
            } else {
                "attachment"
            };
            out.extend_from_slice(
                format!(
                    "Content-Disposition: {disposition}; {}\r\n",
                    file_param("filename", name)
                )
                .as_bytes(),
            );
            if let Some(id) = content_id {
                out.extend_from_slice(format!("Content-ID: <{id}>\r\n").as_bytes());
            }
            out.extend_from_slice(b"Content-Transfer-Encoding: base64\r\n\r\n");
            base64_lines(data, out);
        }
        Part::Message { name, raw } => {
            out.extend_from_slice(b"Content-Type: message/rfc822\r\n");
            out.extend_from_slice(
                format!(
                    "Content-Disposition: attachment; {}\r\n\r\n",
                    file_param("filename", name)
                )
                .as_bytes(),
            );
            out.extend_from_slice(raw);
            if !raw.ends_with(b"\r\n") {
                out.extend_from_slice(b"\r\n");
            }
        }
    }
}

/// The body parts as one entity: one part, or `multipart/<kind>`.
fn entity(parts: &[Part], kind: &str, boundary: &str, out: &mut Vec<u8>) {
    if let [one] = parts {
        write_part(one, out);
    } else {
        out.extend_from_slice(
            format!("Content-Type: multipart/{kind}; boundary=\"{boundary}\"\r\n\r\n").as_bytes(),
        );
        for part in parts {
            out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            write_part(part, out);
        }
        out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    }
}

/// Header lines received with the message, without those that describe a
/// body (this one is built anew).
fn kept_headers(block: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut skipping = false;
    for line in block.replace("\r\n", "\n").split('\n') {
        if line.is_empty() {
            break;
        }
        if line.starts_with([' ', '\t']) {
            if !skipping && let Some(last) = out.last_mut() {
                last.push_str("\r\n");
                last.push_str(line);
            }
            continue;
        }
        let name = line
            .split(':')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        skipping = name.starts_with("content-") || name == "mime-version" || !line.contains(':');
        if !skipping {
            out.push(line.to_owned());
        }
    }
    out
}

fn has_header(lines: &[String], name: &str) -> bool {
    lines.iter().any(|l| {
        l.split(':')
            .next()
            .is_some_and(|n| n.trim().eq_ignore_ascii_case(name))
    })
}

/// The HTML of a body kept as bytes: in the code page it says, else
/// guessed.
fn html_text(bytes: &[u8], p: &Properties) -> String {
    let label = p.int(0x3FDE).and_then(mailcraft_pst::code_page_label);
    match label {
        Some(label) => mailcraft_charset::decode(bytes, label),
        None => mailcraft_charset::guess(bytes),
    }
}

impl<R: Read + Seek> PstFile<R> {
    /// Open a file.
    ///
    /// # Errors
    /// [`PstError::Unreadable`] for what isn't a `.pst` file (or one too
    /// damaged), [`PstError::Password`] for one with a password.
    pub fn open(reader: R) -> Result<Self, PstError> {
        let mut pst = Pst::open(reader).map_err(|e| unreadable(&e))?;
        if pst.password_required() {
            return Err(PstError::Password);
        }
        let store = pst.store().map_err(|e| unreadable(&e))?;
        let key = store.record_key.as_deref().map(hex).unwrap_or_default();
        let named = match pst.named_properties() {
            Ok(map) => {
                let find = |set: &[u8; 16], lid: u32| map.find(set, &PropName::Id(lid));
                Named {
                    start: find(&PSETID_APPOINTMENT, 0x820D),
                    end: find(&PSETID_APPOINTMENT, 0x820E),
                    location: find(&PSETID_APPOINTMENT, 0x8208),
                    all_day: find(&PSETID_APPOINTMENT, 0x8215),
                    recur: find(&PSETID_APPOINTMENT, 0x8216),
                    recurring: find(&PSETID_APPOINTMENT, 0x8223),
                    zone: find(&PSETID_APPOINTMENT, 0x8260),
                    reminder_minutes: find(&PSETID_COMMON, 0x8501),
                    reminder_set: find(&PSETID_COMMON, 0x8503),
                    clean_id: find(&PSETID_MEETING, 0x0023),
                    global_id: find(&PSETID_MEETING, 0x0003),
                    emails: [0x8083, 0x8093, 0x80A3].map(|lid| find(&PSETID_ADDRESS, lid)),
                    work_address: [0x8045, 0x8046, 0x8047, 0x8048, 0x8049]
                        .map(|lid| find(&PSETID_ADDRESS, lid)),
                }
            }
            Err(_) => Named::default(),
        };
        Ok(Self {
            pst,
            named,
            key,
            counter: 0,
        })
    }

    /// The owner's folders (under the top of the file's folders), parents
    /// before children. Search folders are left out.
    ///
    /// # Errors
    /// When the file's folders can't be read at all.
    pub fn folders(&mut self) -> Result<Vec<PstFolder>, PstError> {
        let tree = self.pst.walk_folders().map_err(|e| unreadable(&e))?;
        let ipm = self.pst.store().ok().and_then(|s| s.ipm_subtree);
        let mut paths: std::collections::HashMap<u32, Vec<String>> =
            std::collections::HashMap::new();
        let mut out = Vec::new();
        for entry in tree.folders {
            let folder = entry.folder;
            if !entry.in_ipm_subtree {
                continue;
            }
            let kind = match folder.kind {
                mailcraft_pst::FolderKind::Search => continue,
                mailcraft_pst::FolderKind::Calendar => FolderKind::Calendar,
                mailcraft_pst::FolderKind::Contacts => FolderKind::Contacts,
                mailcraft_pst::FolderKind::Tasks => FolderKind::Tasks,
                mailcraft_pst::FolderKind::Notes => FolderKind::Notes,
                mailcraft_pst::FolderKind::Journal => FolderKind::Journal,
                _ => FolderKind::Mail,
            };
            let path = if Some(folder.id) == ipm {
                Vec::new()
            } else {
                let mut path = folder
                    .parent
                    .and_then(|p| paths.get(&p.0))
                    .cloned()
                    .unwrap_or_default();
                let name: String = folder.name.chars().filter(|c| !c.is_control()).collect();
                let name = name.trim();
                path.push(if name.is_empty() {
                    "Folder".to_owned()
                } else {
                    name.to_owned()
                });
                path
            };
            paths.insert(folder.id.0, path.clone());
            out.push(PstFolder {
                id: folder.id.0,
                path,
                kind,
                count: folder.content_count,
            });
        }
        Ok(out)
    }

    /// The items of a folder, with what each is (by its class, else by
    /// what the folder holds).
    ///
    /// # Errors
    /// When the folder's list can't be read at all.
    pub fn items(&mut self, folder: &PstFolder) -> Result<(Vec<PstItem>, usize), PstError> {
        let list = self.pst.items(Nid(folder.id)).map_err(|e| unreadable(&e))?;
        let items = list
            .items
            .iter()
            .map(|item| PstItem {
                id: item.id.0,
                kind: item_kind(item.class.as_deref(), folder.kind),
            })
            .collect();
        Ok((items, list.past_cap))
    }

    fn read(&mut self, id: &MessageRef) -> Result<mailcraft_pst::Message, String> {
        self.pst.message(id).map_err(|e| e.to_string())
    }

    /// A property whole, read on its own when it was too large to come with
    /// the rest.
    fn whole(&mut self, m: &mailcraft_pst::Message, id: u16, max: u64) -> Option<PropValue> {
        match m.properties.get(id)? {
            PropValue::Large { size } if *size <= max => {
                self.pst.read_property(&m.id, id, max).ok().flatten()
            }
            PropValue::Large { .. } | PropValue::Damaged { .. } => None,
            other => Some(other.clone()),
        }
    }

    /// The body: HTML (kept as HTML, or from rich text that holds HTML) and
    /// text (its own, else the rich text's).
    fn body(
        &mut self,
        m: &mailcraft_pst::Message,
        text: Text,
        notes: &mut Vec<String>,
    ) -> (Option<String>, Option<String>) {
        let html = match self.whole(m, 0x1013, MAX_BODY) {
            Some(PropValue::Binary(b)) => Some(html_text(&b, &m.properties)),
            Some(v) => text.value(&v),
            None => None,
        }
        .filter(|h| !h.trim().is_empty());
        let plain = self
            .whole(m, 0x1000, MAX_BODY)
            .and_then(|v| text.value(&v))
            .map(|t| t.replace("\r\n", "\n"))
            .filter(|t| !t.trim().is_empty());
        if html.is_some() || plain.is_some() {
            return (html, plain);
        }
        let max = usize::try_from(MAX_BODY).unwrap_or(usize::MAX);
        match self.whole(m, 0x1009, MAX_BODY) {
            Some(PropValue::Binary(stored)) => match mailcraft_pst::decompress_rtf(&stored, max) {
                Ok(rtf) => {
                    if let Ok(Some((html, _))) = mailcraft_pst::rtf_html(&rtf, max)
                        && !html.trim().is_empty()
                    {
                        return (Some(html), None);
                    }
                    match mailcraft_pst::rtf_text(&rtf, max) {
                        Ok((text, _)) => (None, Some(text).filter(|t| !t.trim().is_empty())),
                        Err(e) => {
                            notes.push(format!("its rich text body couldn't be read ({e})"));
                            (None, None)
                        }
                    }
                }
                Err(e) => {
                    notes.push(format!("its rich text body couldn't be read ({e})"));
                    (None, None)
                }
            },
            _ => (None, None),
        }
    }

    fn boundary(&mut self, depth: usize) -> String {
        self.counter = self.counter.wrapping_add(1);
        format!("=_mocky_{depth:02}_{:08}_=", self.counter)
    }

    /// A mail item as a message.
    ///
    /// # Errors
    /// The reason when the item can't be read at all.
    pub fn message(&mut self, item: &PstItem) -> Result<BuiltMessage, String> {
        let mut notes = Vec::new();
        let m = self.read(&MessageRef::item(Nid(item.id)))?;
        let built = self.build(&m, 0, &mut notes);
        let p = &m.properties;
        let flags = p.int(0x0E07).unwrap_or(0);
        let received = p.time(0x0E06).or_else(|| p.time(0x0039)).and_then(unix);
        let (_, message_id) = crate::mbox::date_and_id(&built);
        Ok(BuiltMessage {
            raw: built,
            received,
            message_id,
            seen: flags & 0x01 != 0,
            flagged: p.int(0x1090) == Some(2),
            answered: matches!(p.int(0x1081), Some(102 | 103)),
            notes,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn build(
        &mut self,
        m: &mailcraft_pst::Message,
        depth: usize,
        notes: &mut Vec<String>,
    ) -> Vec<u8> {
        let p = m.properties.clone();
        let text = Text::of(&p);
        for part in &m.left_out {
            notes.push(format!("part of it couldn't be read ({})", part.reason));
        }
        let received = match p.get(0x007D) {
            Some(PropValue::Large { .. }) => self
                .pst
                .read_property(&m.id, 0x007D, MAX_HEADERS)
                .ok()
                .flatten()
                .and_then(|v| text.value(&v)),
            Some(v) => text.value(v),
            None => None,
        }
        .filter(|h| h.contains(':'));
        let mut headers = received.as_deref().map(kept_headers).unwrap_or_default();
        let from_headers = has_header(&headers, "From");
        if !from_headers {
            let from = person(text, &p, [0x0042, 0x5D02, 0x0065])
                .or_else(|| person(text, &p, [0x0C1A, 0x5D01, 0x0C1F]));
            if let Some((name, email)) = from {
                headers.push(format!("From: {}", mailbox(&name, &email)));
            }
        }
        if !has_header(&headers, "To") && !has_header(&headers, "Cc") {
            let mut lists: [Vec<String>; 3] = [Vec::new(), Vec::new(), Vec::new()];
            for r in &m.recipients {
                let Some((name, email)) = person(text, &r.properties, [0x3001, 0x39FE, 0x3003])
                else {
                    continue;
                };
                let slot = match r.kind() {
                    RecipientKind::To => 0,
                    RecipientKind::Cc => 1,
                    RecipientKind::Bcc if received.is_none() => 2,
                    _ => continue,
                };
                if let Some(list) = lists.get_mut(slot) {
                    list.push(mailbox(&name, &email));
                }
            }
            for (name, list) in ["To", "Cc", "Bcc"].iter().zip(lists) {
                if !list.is_empty() {
                    headers.push(format!("{name}: {}", list.join(",\r\n ")));
                }
            }
        }
        if !has_header(&headers, "Subject")
            && let Some(subject) = text.short(&p, 0x0037)
        {
            headers.push(format!("Subject: {}", header_text(&subject)));
        }
        if !has_header(&headers, "Date") {
            let date = p.time(0x0039).or_else(|| p.time(0x0E06)).and_then(unix);
            if let Some(date) = date {
                headers.push(format!("Date: {}", rfc_date(date)));
            }
        }
        if !has_header(&headers, "Message-ID")
            && let Some(id) = text
                .short(&p, 0x1035)
                .and_then(|v| ids(&v).into_iter().next())
        {
            headers.push(format!("Message-ID: {id}"));
        }
        if !has_header(&headers, "In-Reply-To")
            && let Some(id) = text
                .short(&p, 0x1042)
                .and_then(|v| ids(&v).into_iter().next())
        {
            headers.push(format!("In-Reply-To: {id}"));
        }
        if !has_header(&headers, "References")
            && let Some(refs) = text
                .lines(&p, 0x1039)
                .map(|v| ids(&v))
                .filter(|r| !r.is_empty())
        {
            headers.push(format!("References: {}", refs.join("\r\n ")));
        }
        if !has_header(&headers, "Importance") {
            match p.int(0x0017) {
                Some(0) => headers.push("Importance: low".to_owned()),
                Some(2) => headers.push("Importance: high".to_owned()),
                _ => {}
            }
        }

        let (html, plain) = self.body(m, text, notes);
        let mut body: Vec<Part> = Vec::new();
        if let Some(plain) = plain {
            body.push(Part::Text {
                subtype: "plain",
                text: plain.replace('\n', "\r\n"),
            });
        }
        if let Some(html) = html {
            body.push(Part::Text {
                subtype: "html",
                text: html,
            });
        }
        if body.is_empty() {
            body.push(Part::Text {
                subtype: "plain",
                text: String::new(),
            });
        }
        let mut files: Vec<Part> = Vec::new();
        for a in &m.attachments {
            let ap = &a.properties;
            let name = text
                .short(ap, 0x3707)
                .or_else(|| text.short(ap, 0x3704))
                .or_else(|| text.short(ap, 0x3001))
                .unwrap_or_default();
            match &a.content {
                AttachmentContent::Message(inner) if depth < MAX_DEPTH => {
                    match self.pst.message(inner) {
                        Ok(attached) => {
                            let raw = self.build(&attached, depth.saturating_add(1), notes);
                            let subject = text
                                .short(&attached.properties, 0x0037)
                                .unwrap_or_else(|| name.clone());
                            let stem = if subject.is_empty() {
                                "attached message".to_owned()
                            } else {
                                subject
                            };
                            files.push(Part::Message {
                                name: format!("{stem}.eml"),
                                raw,
                            });
                        }
                        Err(e) => notes.push(format!("an attached message couldn't be read ({e})")),
                    }
                }
                AttachmentContent::Message(_) | AttachmentContent::MessageTooDeep => {
                    notes.push(
                        "an attached message is nested too deeply and was left out".to_owned(),
                    );
                }
                AttachmentContent::Data { .. } | AttachmentContent::Object { .. } => match self
                    .pst
                    .attachment_bytes(&a.id)
                {
                    Ok(data) => {
                        let ole = matches!(a.content, AttachmentContent::Object { .. })
                            || a.method == Some(6);
                        let name = if ole {
                            "embedded object.bin".to_owned()
                        } else if name.is_empty() {
                            "attachment".to_owned()
                        } else {
                            name
                        };
                        let mime = text
                            .short(ap, 0x370E)
                            .map(|m| m.to_ascii_lowercase())
                            .filter(|m| {
                                m.contains('/') && m.len() < 128 && !m.contains([' ', ';', '"'])
                            })
                            .unwrap_or_else(|| "application/octet-stream".to_owned());
                        let content_id = text
                            .short(ap, 0x3712)
                            .map(|c| c.trim_matches(['<', '>']).to_owned())
                            .filter(|c| {
                                !c.is_empty() && c.is_ascii() && !c.contains([' ', '<', '>'])
                            });
                        files.push(Part::File {
                            name,
                            mime,
                            content_id,
                            data,
                        });
                    }
                    Err(e) => notes.push(format!("the attachment {name} couldn't be read ({e})")),
                },
                AttachmentContent::None => {
                    if matches!(a.method, Some(2..=4 | 7)) {
                        notes.push(format!(
                            "{name} is a link to a file elsewhere and wasn't brought in"
                        ));
                    }
                }
            }
        }

        let mut out = Vec::new();
        for line in &headers {
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"MIME-Version: 1.0\r\n");
        let alternative = self.boundary(depth);
        if files.is_empty() {
            entity(&body, "alternative", &alternative, &mut out);
        } else {
            let mixed = self.boundary(depth);
            out.extend_from_slice(
                format!("Content-Type: multipart/mixed; boundary=\"{mixed}\"\r\n\r\n").as_bytes(),
            );
            out.extend_from_slice(format!("--{mixed}\r\n").as_bytes());
            entity(&body, "alternative", &alternative, &mut out);
            for file in &files {
                out.extend_from_slice(format!("--{mixed}\r\n").as_bytes());
                write_part(file, &mut out);
            }
            out.extend_from_slice(format!("--{mixed}--\r\n").as_bytes());
        }
        out
    }

    /// The id a contact or event without one of its own is given, the same
    /// every time this file is imported.
    fn own_uid(&self, item: PstItem) -> String {
        format!("pst-{}-{:x}@mocky.mail", self.key, item.id)
    }

    /// A contact item as a card.
    ///
    /// # Errors
    /// The reason when the item can't be read at all.
    #[allow(clippy::too_many_lines)]
    pub fn contact(&mut self, item: &PstItem) -> Result<Option<BuiltContact>, String> {
        let m = self.read(&MessageRef::item(Nid(item.id)))?;
        let p = &m.properties;
        let text = Text::of(p);
        let named = self.named;
        let lid = |id: Option<u16>| id.and_then(|id| text.short(p, id));
        let mut lines: Vec<String> = vec![
            "BEGIN:VCARD".to_owned(),
            "VERSION:3.0".to_owned(),
            format!("UID:{}", escape(&self.own_uid(*item))),
        ];
        let given = text.short(p, 0x3A06).unwrap_or_default();
        let family = text.short(p, 0x3A11).unwrap_or_default();
        let middle = text.short(p, 0x3A44).unwrap_or_default();
        let display = text
            .short(p, 0x3001)
            .or_else(|| {
                let joined = [given.as_str(), middle.as_str(), family.as_str()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                (!joined.is_empty()).then_some(joined)
            })
            .unwrap_or_default();
        lines.push(format!("FN:{}", escape(&display)));
        lines.push(format!(
            "N:{};{};{};;",
            escape(&family),
            escape(&given),
            escape(&middle)
        ));
        for email in named
            .emails
            .iter()
            .filter_map(|id| lid(*id))
            .filter(|e| e.contains('@') && !e.contains(' '))
        {
            lines.push(format!("EMAIL;TYPE=INTERNET:{}", escape(&email)));
        }
        let phones: [(u16, &str); 8] = [
            (0x3A1C, "CELL"),
            (0x3A08, "WORK,VOICE"),
            (0x3A1B, "WORK,VOICE"),
            (0x3A09, "HOME,VOICE"),
            (0x3A2F, "HOME,VOICE"),
            (0x3A1A, "VOICE"),
            (0x3A24, "WORK,FAX"),
            (0x3A1F, "VOICE"),
        ];
        for (id, kind) in phones {
            if let Some(number) = text.short(p, id) {
                lines.push(format!("TEL;TYPE={kind}:{}", escape(&number)));
            }
        }
        let company = text.short(p, 0x3A16).unwrap_or_default();
        let department = text.short(p, 0x3A18).unwrap_or_default();
        if !company.is_empty() || !department.is_empty() {
            lines.push(format!("ORG:{};{}", escape(&company), escape(&department)));
        }
        if let Some(title) = text.short(p, 0x3A17) {
            lines.push(format!("TITLE:{}", escape(&title)));
        }
        let address = |kind: &str, parts: [Option<String>; 5]| -> Option<String> {
            parts.iter().any(Option::is_some).then(|| {
                let [street, city, region, postal, country] =
                    parts.map(|x| escape(&x.unwrap_or_default()));
                format!("ADR;TYPE={kind}:;;{street};{city};{region};{postal};{country}")
            })
        };
        let one_line = |id: u16| text.lines(p, id).map(|s| s.replace('\n', ", "));
        lines.extend(address(
            "WORK",
            named.work_address.map(|id| id.and_then(one_line)),
        ));
        if named.work_address.iter().all(Option::is_none) {
            lines.extend(address(
                "WORK",
                [0x3A29, 0x3A27, 0x3A28, 0x3A2A, 0x3A26].map(one_line),
            ));
        }
        lines.extend(address(
            "HOME",
            [0x3A5D, 0x3A59, 0x3A5C, 0x3A5B, 0x3A5A].map(one_line),
        ));
        if let Some(day) = p
            .time(0x3A42)
            .and_then(unix)
            .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        {
            // Outlook keeps a birthday as the start of that day in some
            // zone; noon UTC lands on the right day for every zone.
            let day = day
                .checked_add_signed(chrono::TimeDelta::hours(12))
                .unwrap_or(day);
            lines.push(format!("BDAY:{}", day.format("%Y-%m-%d")));
        }
        for id in [0x3A51u16, 0x3A50] {
            if let Some(url) = text.short(p, id) {
                lines.push(format!("URL:{}", escape(&url)));
            }
        }
        if let Some(notes) = self
            .whole(&m, 0x1000, MAX_BODY)
            .and_then(|v| text.value(&v))
        {
            let notes = notes.replace("\r\n", "\n");
            if !notes.trim().is_empty() {
                lines.push(format!("NOTE:{}", escape(notes.trim())));
            }
        }
        lines.push("END:VCARD".to_owned());
        let mut vcard = String::new();
        for line in &lines {
            fold(line, &mut vcard);
        }
        Ok(crate::vcard::parse(&vcard).map(|card| BuiltContact { card, vcard }))
    }

    /// A calendar item as an iCalendar resource. `None` for one with no
    /// date.
    ///
    /// # Errors
    /// The reason when the item can't be read at all.
    #[allow(clippy::too_many_lines)]
    pub fn event(&mut self, item: &PstItem) -> Result<Option<BuiltEvent>, String> {
        let m = self.read(&MessageRef::item(Nid(item.id)))?;
        let p = m.properties.clone();
        let text = Text::of(&p);
        let named = self.named;
        let mut notes = Vec::new();
        let time = |id: Option<u16>, fallback: u16| {
            id.and_then(|id| p.time(id))
                .or_else(|| p.time(fallback))
                .and_then(unix)
        };
        let Some(start) = time(named.start, 0x0060) else {
            return Ok(None);
        };
        let end = time(named.end, 0x0061)
            .filter(|e| *e >= start)
            .unwrap_or(start);
        let all_day = named.all_day.and_then(|id| p.bool(id)).unwrap_or(false);
        let zone_key = named
            .zone
            .and_then(|id| p.binary(id))
            .and_then(|b| mailcraft_pst::time_zone_key(b).ok());
        let zone = zone_key.as_deref().and_then(crate::ical::zone_for);
        let uid = [named.clean_id, named.global_id]
            .into_iter()
            .flatten()
            .find_map(|id| p.binary(id).filter(|b| !b.is_empty()).map(hex))
            .map_or_else(|| self.own_uid(*item), |h| format!("{h}@outlook"));

        let local = |seconds: i64| -> Option<chrono::NaiveDateTime> {
            let utc = chrono::DateTime::from_timestamp(seconds, 0)?;
            Some(match zone {
                Some(zone) => utc.with_timezone(&zone).naive_local(),
                None => utc.naive_utc(),
            })
        };
        let stamp = |name: &str, seconds: i64| -> Option<String> {
            if all_day {
                let day = local(seconds)?;
                // Midnight in the zone; with no zone, the nearest midnight.
                let day = if zone.is_none() {
                    day.checked_add_signed(chrono::TimeDelta::hours(12))?.date()
                } else {
                    day.date()
                };
                return Some(format!("{name};VALUE=DATE:{}", day.format("%Y%m%d")));
            }
            Some(match zone {
                Some(zone) => format!(
                    "{name};TZID={}:{}",
                    zone.name(),
                    local(seconds)?.format("%Y%m%dT%H%M%S")
                ),
                None => format!(
                    "{name}:{}",
                    chrono::DateTime::from_timestamp(seconds, 0)?.format("%Y%m%dT%H%M%SZ")
                ),
            })
        };

        let mut event: Vec<String> =
            vec!["BEGIN:VEVENT".to_owned(), format!("UID:{}", escape(&uid))];
        if let Some(created) = p
            .time(0x3007)
            .and_then(unix)
            .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        {
            event.push(format!("DTSTAMP:{}", created.format("%Y%m%dT%H%M%SZ")));
        } else {
            event.push("DTSTAMP:19700101T000000Z".to_owned());
        }
        event.extend(stamp("DTSTART", start));
        event.extend(stamp("DTEND", end));
        if let Some(subject) = text.short(&p, 0x0037) {
            event.push(format!("SUMMARY:{}", escape(&subject)));
        }
        if let Some(place) = named.location.and_then(|id| text.short(&p, id)) {
            event.push(format!("LOCATION:{}", escape(&place)));
        }
        let (html, plain) = self.body(&m, text, &mut notes);
        let description = plain.or_else(|| html.map(|h| crate::rich::html_to_text(&h)));
        if let Some(description) = description.filter(|d| !d.trim().is_empty()) {
            event.push(format!("DESCRIPTION:{}", escape(description.trim())));
        }
        let recurring = named.recurring.and_then(|id| p.bool(id)).unwrap_or(false);
        let mut overrides: Vec<Vec<String>> = Vec::new();
        if recurring {
            match named
                .recur
                .and_then(|id| p.binary(id))
                .map(mailcraft_pst::appointment_recurrence)
            {
                Some(Ok(recur)) => {
                    match rrule(&recur, start, zone) {
                        Some(rule) => event.push(rule),
                        None => notes.push("it repeats in a way that couldn't be read; only its first date was brought in".to_owned()),
                    }
                    let start_offset = i64::from(recur.start_offset);
                    for minutes in recur.deleted.iter().filter(|d| !recur.modified.contains(d)) {
                        let at = minutes_local(*minutes, start_offset);
                        if let Some(line) = at.map(|at| local_stamp("EXDATE", at, all_day, zone)) {
                            event.push(line);
                        }
                    }
                    for exception in &recur.exceptions {
                        let original = minutes_local(exception.original, 0);
                        let (Some(original), Some(new_start), Some(new_end)) = (
                            original,
                            minutes_local(exception.start, 0),
                            minutes_local(exception.end, 0),
                        ) else {
                            continue;
                        };
                        let mut block =
                            vec!["BEGIN:VEVENT".to_owned(), format!("UID:{}", escape(&uid))];
                        block.push(local_stamp("RECURRENCE-ID", original, all_day, zone));
                        block.push(local_stamp("DTSTART", new_start, all_day, zone));
                        block.push(local_stamp("DTEND", new_end, all_day, zone));
                        let subject = exception
                            .subject
                            .clone()
                            .or_else(|| exception.subject_8bit.as_deref().map(|b| text.decode(b)))
                            .or_else(|| text.short(&p, 0x0037));
                        if let Some(subject) = subject {
                            block.push(format!("SUMMARY:{}", escape(&subject)));
                        }
                        if let Some(place) = exception
                            .location
                            .clone()
                            .or_else(|| exception.location_8bit.as_deref().map(|b| text.decode(b)))
                        {
                            block.push(format!("LOCATION:{}", escape(&place)));
                        }
                        block.push("END:VEVENT".to_owned());
                        overrides.push(block);
                    }
                }
                Some(Err(e)) => notes.push(format!(
                    "how it repeats couldn't be read ({e}); only its first date was brought in"
                )),
                None => notes.push(
                    "it repeats, but how isn't in the file; only its first date was brought in"
                        .to_owned(),
                ),
            }
        }
        let reminder = named
            .reminder_set
            .and_then(|id| p.bool(id))
            .unwrap_or(false);
        if reminder
            && let Some(minutes) = named
                .reminder_minutes
                .and_then(|id| p.int(id))
                .filter(|m| (0..=40_320).contains(m))
        {
            event.extend([
                "BEGIN:VALARM".to_owned(),
                "ACTION:DISPLAY".to_owned(),
                format!("TRIGGER:-PT{minutes}M"),
                "DESCRIPTION:Reminder".to_owned(),
                "END:VALARM".to_owned(),
            ]);
        }
        event.push("END:VEVENT".to_owned());

        let mut lines = vec![
            "BEGIN:VCALENDAR".to_owned(),
            "VERSION:2.0".to_owned(),
            "PRODID:-//Mocky Digital//Mocky Mail//EN".to_owned(),
            "CALSCALE:GREGORIAN".to_owned(),
        ];
        if let Some(zone) = zone.filter(|_| !all_day) {
            let year = chrono::DateTime::from_timestamp(start, 0).map_or(2026, |d| {
                use chrono::Datelike as _;
                d.year()
            });
            lines.extend(crate::ical::vtimezone(zone, year));
        }
        lines.extend(event);
        for block in overrides {
            lines.extend(block);
        }
        lines.push("END:VCALENDAR".to_owned());
        let mut ics = String::new();
        for line in &lines {
            fold(line, &mut ics);
        }
        Ok(Some(BuiltEvent { uid, ics, notes }))
    }

    /// Problems the reader met in the file's own structure, in words.
    #[must_use]
    pub fn damage(&self) -> Vec<String> {
        self.pst.damage().iter().map(|d| format!("{d:?}")).collect()
    }
}

/// A local time from minutes since 1601 (by the series' clock) plus
/// `offset` minutes.
fn minutes_local(minutes: u32, offset: i64) -> Option<chrono::NaiveDateTime> {
    let base = chrono::NaiveDate::from_ymd_opt(1601, 1, 1)?.and_hms_opt(0, 0, 0)?;
    let total = i64::from(minutes).checked_add(offset)?;
    base.checked_add_signed(chrono::TimeDelta::try_minutes(total)?)
}

/// A date line from a local time of the series.
fn local_stamp(
    name: &str,
    at: chrono::NaiveDateTime,
    all_day: bool,
    zone: Option<chrono_tz::Tz>,
) -> String {
    if all_day {
        return format!("{name};VALUE=DATE:{}", at.format("%Y%m%d"));
    }
    match zone {
        Some(zone) => format!("{name};TZID={}:{}", zone.name(), at.format("%Y%m%dT%H%M%S")),
        // Without the zone the series' clock is taken as UTC.
        None => format!("{name}:{}", at.format("%Y%m%dT%H%M%SZ")),
    }
}

/// The `RRULE` of an Outlook series.
fn rrule(recur: &mailcraft_pst::Recur, start: i64, zone: Option<chrono_tz::Tz>) -> Option<String> {
    use mailcraft_pst::recur::{
        END_COUNT, END_DATE, FREQ_DAILY, FREQ_MONTHLY, FREQ_WEEKLY, FREQ_YEARLY, PATTERN_DAY,
        PATTERN_MONTH, PATTERN_MONTH_END, PATTERN_MONTH_NTH, PATTERN_WEEK,
    };
    use std::fmt::Write as _;
    const DAYS: [&str; 7] = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];
    let days = || -> String {
        DAYS.iter()
            .enumerate()
            .filter(|(i, _)| recur.weekdays & (1u32 << i) != 0)
            .map(|(_, d)| *d)
            .collect::<Vec<_>>()
            .join(",")
    };
    let nth = if recur.day_or_nth == 5 {
        -1
    } else {
        i64::from(recur.day_or_nth)
    };
    let mut rule = match (recur.frequency, recur.pattern) {
        (FREQ_DAILY, PATTERN_DAY) => {
            format!("FREQ=DAILY;INTERVAL={}", (recur.period / 1440).max(1))
        }
        (FREQ_DAILY | FREQ_WEEKLY, PATTERN_WEEK) if recur.weekdays != 0 => {
            let interval = if recur.frequency == FREQ_DAILY {
                1
            } else {
                recur.period.max(1)
            };
            format!("FREQ=WEEKLY;INTERVAL={interval};BYDAY={}", days())
        }
        (FREQ_MONTHLY | FREQ_YEARLY, PATTERN_MONTH | PATTERN_MONTH_END | PATTERN_MONTH_NTH) => {
            let months = recur.period.max(1);
            let (freq, interval) = if months.is_multiple_of(12) {
                ("YEARLY", months / 12)
            } else {
                ("MONTHLY", months)
            };
            let mut rule = format!("FREQ={freq};INTERVAL={interval}");
            if freq == "YEARLY" {
                use chrono::Datelike as _;
                let month = chrono::DateTime::from_timestamp(start, 0).map(|d| match zone {
                    Some(z) => d.with_timezone(&z).month(),
                    None => d.month(),
                })?;
                let _ = write!(rule, ";BYMONTH={month}");
            }
            match recur.pattern {
                PATTERN_MONTH if recur.day_or_nth == 31 => rule.push_str(";BYMONTHDAY=-1"),
                PATTERN_MONTH => {
                    let _ = write!(rule, ";BYMONTHDAY={}", recur.day_or_nth);
                }
                PATTERN_MONTH_END => rule.push_str(";BYMONTHDAY=-1"),
                _ if recur.weekdays != 0 => {
                    let _ = write!(rule, ";BYDAY={nth}{}", days());
                }
                _ => return None,
            }
            if recur.pattern == PATTERN_MONTH_NTH && days().contains(',') {
                // "The last weekday": the days, then which of them.
                rule = rule.replace(
                    &format!(";BYDAY={nth}{}", days()),
                    &format!(";BYDAY={};BYSETPOS={nth}", days()),
                );
            }
            rule
        }
        _ => return None,
    };
    match recur.end_type {
        END_COUNT if recur.count > 0 => {
            let _ = write!(rule, ";COUNT={}", recur.count);
        }
        END_DATE => {
            let end = minutes_local(recur.end_date, 24 * 60 - 1)?;
            let _ = write!(rule, ";UNTIL={}", end.format("%Y%m%dT%H%M%SZ"));
        }
        _ => {}
    }
    Some(format!("RRULE:{rule}"))
}

/// What an item is, from its class (`IPM.Note`, `IPM.Contact`…), else from
/// its folder.
fn item_kind(class: Option<&str>, folder: FolderKind) -> ItemKind {
    let class = class.unwrap_or_default().to_ascii_lowercase();
    let family = |prefix: &str| class == prefix || class.starts_with(&format!("{prefix}."));
    if family("ipm.contact") {
        ItemKind::Contact
    } else if family("ipm.distlist") {
        ItemKind::ContactGroup
    } else if family("ipm.appointment") {
        ItemKind::Event
    } else if family("ipm.task") || family("ipm.taskrequest") {
        ItemKind::Task
    } else if family("ipm.stickynote") {
        ItemKind::Note
    } else if family("ipm.activity") {
        ItemKind::Journal
    } else if class.starts_with("ipm") || class.starts_with("report") {
        ItemKind::Mail
    } else {
        match folder {
            FolderKind::Contacts => ItemKind::Contact,
            FolderKind::Calendar => ItemKind::Event,
            FolderKind::Tasks => ItemKind::Task,
            FolderKind::Notes => ItemKind::Note,
            FolderKind::Journal => ItemKind::Journal,
            FolderKind::Mail => ItemKind::Mail,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::too_many_lines)]
mod tests {
    use super::*;
    use mailcraft_pst::test_writer::{
        AttachSpec, FileSpec, FolderSpec, Layout, MessageSpec, binary, boolean, int32, time,
        unicode,
    };
    use mailcraft_pst::{Encryption, Format};
    use std::io::Cursor;

    /// 2026-11-04 09:30 UTC as a FILETIME.
    const DELIVERED: u64 = 134_067_498_000_000_000;
    const HOUR: u64 = 36_000_000_000;

    fn note(subject: &str) -> MessageSpec {
        MessageSpec {
            class: "IPM.Note".into(),
            subject: subject.into(),
            body: format!("{subject}\r\n\r\nWritten for the tests."),
            read: false,
            delivered: DELIVERED,
            ..Default::default()
        }
    }

    fn spec(format: Format) -> FileSpec {
        let mut report = note("Quarterly figures");
        report.read = true;
        report.recipients = vec![
            ("Ada Fernsby".into(), "ada@example.org".into(), 1),
            ("B\u{f6}ram Okonjo".into(), "bram@example.org".into(), 2),
        ];
        let mut forwarded = note("Minutes");
        forwarded.attachments = vec![AttachSpec::File {
            name: "minutes.txt".into(),
            mime: "text/plain".into(),
            data: b"Present: everyone.".to_vec(),
        }];
        report.attachments = vec![
            AttachSpec::File {
                name: "figures.csv".into(),
                mime: "text/csv".into(),
                data: b"a,b\r\n1,2\r\n".to_vec(),
            },
            AttachSpec::Message(Box::new(forwarded)),
        ];
        report.extra = vec![
            (0x0042, unicode("Wanjiru Kamau")),
            (0x5D02, unicode("wanjiru@shop.example")),
            (0x1035, unicode("<q-1@shop.example>")),
            (0x0039, time(DELIVERED)),
        ];
        let mut received = note("With headers");
        received.extra = vec![(
            0x007D,
            unicode(
                "Received: from mx.example\r\n by me\r\nFrom: Peter <peter@client.example>\r\nTo: me@x.example\r\nSubject: With headers\r\nDate: Wed, 04 Nov 2026 09:30:00 +0000\r\nMessage-ID: <h-2@client.example>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n boundary=\"old\"\r\n\r\n",
            ),
        )];
        // Named properties: ids from 0x8000 in this order.
        let mut contact = MessageSpec {
            class: "IPM.Contact".into(),
            subject: "Amina Hassan".into(),
            ..Default::default()
        };
        contact.extra = vec![
            (0x3001, unicode("Amina Hassan")),
            (0x3A06, unicode("Amina")),
            (0x3A11, unicode("Hassan")),
            (0x3A16, unicode("Studio Ltd")),
            (0x3A1C, unicode("+254 700 000 003")),
            (0x8000, unicode("amina@studio.example")),
        ];
        let mut meeting = MessageSpec {
            class: "IPM.Appointment".into(),
            subject: "Site visit".into(),
            body: "Bring the plans.".into(),
            ..Default::default()
        };
        meeting.extra = vec![
            (0x8001, time(DELIVERED)),
            (0x8002, time(DELIVERED + HOUR)),
            (0x8003, unicode("Kilimani")),
            (0x8004, boolean(false)),
            (0x8005, boolean(true)),
            (0x8006, int32(15)),
            (0x8007, binary(&[1, 2, 3, 4])),
        ];
        let task = MessageSpec {
            class: "IPM.Task".into(),
            subject: "Paint".into(),
            ..Default::default()
        };
        let layout = Layout::new(format, Encryption::Permute);
        FileSpec {
            layout,
            password: None,
            password_ansi: false,
            name: "Personal Folders".into(),
            folders: vec![
                FolderSpec {
                    name: "Inbox".into(),
                    class: Some("IPF.Note".into()),
                    messages: vec![report, received],
                    children: vec![FolderSpec {
                        name: "Projects / 2026".into(),
                        class: Some("IPF.Note".into()),
                        messages: vec![note("Shed roof")],
                        children: vec![],
                    }],
                },
                FolderSpec {
                    name: "Contacts".into(),
                    class: Some("IPF.Contact".into()),
                    messages: vec![contact],
                    children: vec![],
                },
                FolderSpec {
                    name: "Calendar".into(),
                    class: Some("IPF.Appointment".into()),
                    messages: vec![meeting],
                    children: vec![],
                },
                FolderSpec {
                    name: "Tasks".into(),
                    class: Some("IPF.Task".into()),
                    messages: vec![task],
                    children: vec![],
                },
            ],
            named: vec![
                (PSETID_ADDRESS, 0x8083),
                (PSETID_APPOINTMENT, 0x820D),
                (PSETID_APPOINTMENT, 0x820E),
                (PSETID_APPOINTMENT, 0x8208),
                (PSETID_APPOINTMENT, 0x8215),
                (PSETID_COMMON, 0x8503),
                (PSETID_COMMON, 0x8501),
                (PSETID_MEETING, 0x0003),
            ],
            named_strings: vec![],
        }
    }

    fn open(format: Format) -> PstFile<Cursor<Vec<u8>>> {
        let bytes = mailcraft_pst::test_writer::build(&spec(format));
        PstFile::open(Cursor::new(bytes)).unwrap()
    }

    #[test]
    fn walks_the_folders_with_their_paths() {
        for format in [Format::Unicode, Format::Ansi] {
            let mut file = open(format);
            let folders = file.folders().unwrap();
            let named: Vec<(Vec<String>, FolderKind)> = folders
                .iter()
                .filter(|f| !f.path.is_empty())
                .map(|f| (f.path.clone(), f.kind))
                .collect();
            assert_eq!(
                named,
                [
                    (vec!["Inbox".to_owned()], FolderKind::Mail),
                    (
                        vec!["Inbox".to_owned(), "Projects / 2026".to_owned()],
                        FolderKind::Mail
                    ),
                    (vec!["Contacts".to_owned()], FolderKind::Contacts),
                    (vec!["Calendar".to_owned()], FolderKind::Calendar),
                    (vec!["Tasks".to_owned()], FolderKind::Tasks),
                ],
                "{format:?}"
            );
        }
    }

    fn folder(file: &mut PstFile<Cursor<Vec<u8>>>, name: &str) -> PstFolder {
        file.folders()
            .unwrap()
            .into_iter()
            .find(|f| f.path.last().is_some_and(|n| n == name))
            .unwrap()
    }

    #[test]
    fn mail_becomes_messages_with_attachments() {
        let mut file = open(Format::Unicode);
        let inbox = folder(&mut file, "Inbox");
        let (items, past_cap) = file.items(&inbox).unwrap();
        assert_eq!(past_cap, 0);
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| i.kind == ItemKind::Mail));
        let built = file.message(&items[0]).unwrap();
        assert!(built.seen && !built.flagged);
        assert_eq!(built.message_id.as_deref(), Some("q-1@shop.example"));
        assert_eq!(built.received, unix(DELIVERED));
        let summary = crate::message::parse_summary(&built.raw).unwrap();
        assert_eq!(summary.subject, "Quarterly figures");
        assert_eq!(summary.from.unwrap().address, "wanjiru@shop.example");
        assert_eq!(summary.to[0].address, "ada@example.org");
        assert_eq!(summary.cc[0].name.as_deref(), Some("B\u{f6}ram Okonjo"));
        assert_eq!(summary.date, unix(DELIVERED));
        let body = crate::message::parse_body(&built.raw).unwrap();
        assert!(body.text.unwrap().contains("Written for the tests."));
        let names: Vec<&str> = body
            .attachments
            .iter()
            .map(|a| a.file_name.as_str())
            .collect();
        assert_eq!(names, ["figures.csv", "Minutes.eml"]);
        let (_, csv) = crate::message::attachment_bytes(&built.raw, 0)
            .unwrap()
            .unwrap();
        assert_eq!(csv, b"a,b\r\n1,2\r\n");
        let (info, inner) = crate::message::attachment_bytes(&built.raw, 1)
            .unwrap()
            .unwrap();
        assert_eq!(info.mime_type, "message/rfc822");
        let inner_body = crate::message::parse_body(&inner).unwrap();
        assert_eq!(inner_body.attachments[0].file_name, "minutes.txt");
        assert!(built.raw.windows(2).all(|w| w != b"\n\n"));
    }

    #[test]
    fn received_headers_are_kept_and_the_body_is_new() {
        let mut file = open(Format::Ansi);
        let inbox = folder(&mut file, "Inbox");
        let (items, _) = file.items(&inbox).unwrap();
        let built = file.message(&items[1]).unwrap();
        let text = String::from_utf8_lossy(&built.raw);
        assert!(text.starts_with("Received: from mx.example\r\n by me\r\nFrom: Peter"));
        assert!(!text.contains("boundary=\"old\""));
        assert_eq!(text.matches("MIME-Version").count(), 1);
        assert_eq!(built.message_id.as_deref(), Some("h-2@client.example"));
        let summary = crate::message::parse_summary(&built.raw).unwrap();
        assert_eq!(summary.from.unwrap().address, "peter@client.example");
        assert!(!built.seen);
    }

    #[test]
    fn contacts_become_cards() {
        let mut file = open(Format::Unicode);
        let contacts = folder(&mut file, "Contacts");
        let (items, _) = file.items(&contacts).unwrap();
        assert_eq!(items[0].kind, ItemKind::Contact);
        let built = file.contact(&items[0]).unwrap().unwrap();
        assert_eq!(built.card.name, "Amina Hassan");
        assert_eq!(built.card.emails, ["amina@studio.example"]);
        assert_eq!(built.card.phones, ["+254 700 000 003"]);
        assert_eq!(built.card.organization, "Studio Ltd");
        assert!(built.vcard.contains("N:Hassan;Amina;;;"));
        // The same id each time the file is read.
        let again = file.contact(&items[0]).unwrap().unwrap();
        assert_eq!(again.card.uid, built.card.uid);
    }

    #[test]
    fn appointments_become_events() {
        let mut file = open(Format::Unicode);
        let calendar = folder(&mut file, "Calendar");
        let (items, _) = file.items(&calendar).unwrap();
        assert_eq!(items[0].kind, ItemKind::Event);
        let built = file.event(&items[0]).unwrap().unwrap();
        assert_eq!(built.uid, "01020304@outlook");
        let event = crate::ical::master(&built.ics).unwrap();
        assert_eq!(event.title, "Site visit");
        assert_eq!(event.location, "Kilimani");
        assert_eq!(event.notes, "Bring the plans.");
        assert_eq!(Some(event.start), unix(DELIVERED));
        assert_eq!(event.end, event.start + 3600);
        assert_eq!(event.reminders, [15]);
        assert!(!event.all_day);
    }

    #[test]
    fn tasks_are_named() {
        let mut file = open(Format::Unicode);
        let tasks = folder(&mut file, "Tasks");
        let (items, _) = file.items(&tasks).unwrap();
        assert_eq!(items[0].kind, ItemKind::Task);
    }

    #[test]
    fn a_password_or_junk_is_refused() {
        let mut with_password = spec(Format::Unicode);
        with_password.password = Some("secret".into());
        let bytes = mailcraft_pst::test_writer::build(&with_password);
        assert_eq!(
            PstFile::open(Cursor::new(bytes)).err(),
            Some(PstError::Password)
        );
        assert!(matches!(
            PstFile::open(Cursor::new(b"not a pst".to_vec())),
            Err(PstError::Unreadable(_))
        ));
    }

    #[test]
    fn the_fixtures_of_the_reader_open() {
        // MailCraft's own sample files (its fuzz seeds), read when present.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../mailcraft/fuzz/seeds/pst");
        for name in ["ansi-cyclic.pst", "unicode-permute.pst"] {
            let Ok(bytes) = std::fs::read(dir.join(name)) else {
                continue;
            };
            let mut file = PstFile::open(Cursor::new(bytes)).unwrap();
            for f in file.folders().unwrap() {
                let (items, _) = file.items(&f).unwrap();
                for item in items {
                    match item.kind {
                        ItemKind::Mail => assert!(file.message(&item).is_ok()),
                        ItemKind::Contact => drop(file.contact(&item)),
                        ItemKind::Event => drop(file.event(&item)),
                        _ => {}
                    }
                }
            }
        }
    }

    #[test]
    fn recurrence_rules() {
        use mailcraft_pst::Recur;
        use mailcraft_pst::recur::{
            END_COUNT, FREQ_DAILY, FREQ_MONTHLY, FREQ_WEEKLY, FREQ_YEARLY, PATTERN_DAY,
            PATTERN_MONTH, PATTERN_MONTH_NTH, PATTERN_WEEK,
        };
        let weekly = Recur {
            frequency: FREQ_WEEKLY,
            pattern: PATTERN_WEEK,
            period: 2,
            weekdays: 0b0010_1010,
            end_type: END_COUNT,
            count: 10,
            ..Recur::default()
        };
        assert_eq!(
            rrule(&weekly, 0, None).unwrap(),
            "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE,FR;COUNT=10"
        );
        let daily = Recur {
            frequency: FREQ_DAILY,
            pattern: PATTERN_DAY,
            period: 1440 * 3,
            end_type: 0x2023,
            ..Recur::default()
        };
        assert_eq!(
            rrule(&daily, 0, None).unwrap(),
            "RRULE:FREQ=DAILY;INTERVAL=3"
        );
        let monthly = Recur {
            frequency: FREQ_MONTHLY,
            pattern: PATTERN_MONTH_NTH,
            period: 1,
            weekdays: 0b10,
            day_or_nth: 5,
            end_type: 0x2023,
            ..Recur::default()
        };
        assert_eq!(
            rrule(&monthly, 0, None).unwrap(),
            "RRULE:FREQ=MONTHLY;INTERVAL=1;BYDAY=-1MO"
        );
        let yearly = Recur {
            frequency: FREQ_YEARLY,
            pattern: PATTERN_MONTH,
            period: 12,
            day_or_nth: 4,
            end_type: 0x2023,
            ..Recur::default()
        };
        assert_eq!(
            rrule(&yearly, 1_793_871_000, None).unwrap(),
            "RRULE:FREQ=YEARLY;INTERVAL=1;BYMONTH=11;BYMONTHDAY=4"
        );
        assert!(rrule(&Recur::default(), 0, None).is_none());
    }

    #[test]
    fn headers_are_encoded() {
        assert_eq!(header_text("Plain"), "Plain");
        assert!(header_text("Gr\u{fc}\u{df}e").starts_with("=?UTF-8?B?"));
        assert_eq!(mailbox("Ada", "ada@x.example"), "\"Ada\" <ada@x.example>");
        assert_eq!(mailbox("", "ada@x.example"), "<ada@x.example>");
        assert_eq!(ids("<a@x> <b@y>"), ["<a@x>", "<b@y>"]);
        assert_eq!(
            kept_headers("A: 1\r\nContent-Type: x;\r\n b=1\r\nB: 2\r\n\r\nbody"),
            ["A: 1", "B: 2"]
        );
        assert_eq!(
            file_param("name", "r\u{e9}sum\u{e9}.pdf"),
            "name*=UTF-8''r%C3%A9sum%C3%A9.pdf"
        );
    }
}
