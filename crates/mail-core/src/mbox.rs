//! Mailbox files (`.mbox`) and single messages (`.eml`): reading them for
//! an import, and writing a folder out as one file.
//!
//! An mbox file is messages one after another, each opened by a `From `
//! line. Reading takes mboxrd and the older mboxo: a body line that would
//! look like a separator was written with `>` before it, and one `>` is
//! taken off again. Writing escapes the mboxrd way (every `>*From ` line
//! gets one more `>`), so every line reads back as it was.
//!
//! The reader goes a line at a time over any [`BufRead`], so a file of many
//! gigabytes is never held in memory: only the message being read, and none
//! larger than [`MAX_FILE_MESSAGE_BYTES`].

use std::io::{self, BufRead, Write};

use mail_parser::MessageParser;

/// Largest message read from a file; a larger one is counted and skipped.
pub const MAX_FILE_MESSAGE_BYTES: usize = crate::message::MAX_MESSAGE_BYTES;

/// One message of an mbox file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MboxItem {
    /// The message, unescaped, as it was stored.
    Message(Vec<u8>),
    /// A message larger than [`MAX_FILE_MESSAGE_BYTES`], skipped: its size.
    TooLarge(u64),
}

/// Whether a file's first bytes are an mbox file's (a `From ` line), rather
/// than one message.
#[must_use]
pub fn is_mbox(start: &[u8]) -> bool {
    let start = start.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(start);
    let first = start
        .iter()
        .position(|b| !matches!(b, b'\r' | b'\n'))
        .and_then(|at| start.get(at..))
        .unwrap_or_default();
    first.starts_with(b"From ")
}

/// Whether a line looks like a separator's date part follows: a time
/// (`09:42`) and a four-digit year, as every mbox writer puts there.
fn looks_dated(line: &[u8]) -> bool {
    let text = String::from_utf8_lossy(line);
    let mut time = false;
    let mut year = false;
    for word in text.split_whitespace() {
        let digits_and_colons = word.chars().all(|c| c.is_ascii_digit() || c == ':');
        if digits_and_colons && word.matches(':').count() >= 1 && word.len() >= 4 {
            time = true;
        }
        if word.len() == 4 && word.chars().all(|c| c.is_ascii_digit()) {
            year = true;
        }
    }
    time && year
}

fn is_blank(line: &[u8]) -> bool {
    matches!(line, b"\n" | b"\r\n" | b"\r")
}

/// The reader of an mbox file, one message at a time.
#[derive(Debug)]
pub struct MboxReader<R> {
    reader: R,
    started: bool,
    done: bool,
    position: u64,
    line: Vec<u8>,
}

impl<R: BufRead> MboxReader<R> {
    /// A reader over a file (or anything else that reads).
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            started: false,
            done: false,
            position: 0,
            line: Vec::new(),
        }
    }

    /// How many bytes of the file have been read so far (for progress).
    #[must_use]
    pub fn position(&self) -> u64 {
        self.position
    }

    fn read_line(&mut self) -> io::Result<bool> {
        self.line.clear();
        let read = self.reader.read_until(b'\n', &mut self.line)?;
        self.position = self
            .position
            .saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        Ok(read > 0)
    }

    /// The next message, or `None` at the end of the file.
    ///
    /// # Errors
    /// When the file can't be read.
    pub fn next_message(&mut self) -> io::Result<Option<MboxItem>> {
        if self.done {
            return Ok(None);
        }
        if !self.started {
            // Anything before the first separator isn't a message.
            loop {
                if !self.read_line()? {
                    self.done = true;
                    return Ok(None);
                }
                if self.line.starts_with(b"From ") {
                    break;
                }
            }
            self.started = true;
        }
        let mut out: Vec<u8> = Vec::new();
        let mut size = 0u64;
        let mut too_large = false;
        // A blank line is held back: the one before a separator (or the end
        // of the file) closes the message and isn't part of it.
        let mut held_blank: Option<Vec<u8>> = None;
        loop {
            if !self.read_line()? {
                self.done = true;
                break;
            }
            let separator = self.line.starts_with(b"From ")
                && (held_blank.is_some() || looks_dated(&self.line));
            if separator {
                break;
            }
            let mut pending: Vec<Vec<u8>> = Vec::with_capacity(2);
            if let Some(blank) = held_blank.take() {
                pending.push(blank);
            }
            if is_blank(&self.line) {
                held_blank = Some(self.line.clone());
            } else {
                pending.push(unescape(&self.line).to_vec());
            }
            for line in pending {
                size = size.saturating_add(u64::try_from(line.len()).unwrap_or(u64::MAX));
                if size > u64::try_from(MAX_FILE_MESSAGE_BYTES).unwrap_or(u64::MAX) {
                    too_large = true;
                    out = Vec::new();
                }
                if !too_large {
                    out.extend_from_slice(&line);
                }
            }
        }
        if too_large {
            return Ok(Some(MboxItem::TooLarge(size)));
        }
        Ok(Some(MboxItem::Message(out)))
    }
}

impl<R: BufRead> Iterator for MboxReader<R> {
    type Item = io::Result<MboxItem>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_message().transpose()
    }
}

/// A body line with one `>` taken off when it is an escaped separator
/// (`>From `, `>>From `…).
fn unescape(line: &[u8]) -> &[u8] {
    let quotes = line.iter().take_while(|b| **b == b'>').count();
    if quotes > 0
        && line
            .get(quotes..)
            .is_some_and(|rest| rest.starts_with(b"From "))
    {
        line.get(1..).unwrap_or(line)
    } else {
        line
    }
}

/// The date of a separator line (`asctime` form, in UTC).
fn separator_date(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0)
        .unwrap_or_default()
        .format("%a %b %e %H:%M:%S %Y")
        .to_string()
}

/// Write one message to an mbox file: its `From ` line (with `sender`,
/// an address, and `date`), `Status` lines for its read state when it has
/// none, its lines with separators escaped (mboxrd) and the blank line that
/// ends it. Line ends are written as `\n`.
///
/// # Errors
/// When the file can't be written.
pub fn write_message<W: Write>(
    out: &mut W,
    raw: &[u8],
    sender: &str,
    date: i64,
    flags: Option<FileFlags>,
) -> io::Result<()> {
    let sender: String = sender
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect();
    let sender = if sender.is_empty() {
        "MAILER-DAEMON".to_owned()
    } else {
        sender
    };
    writeln!(out, "From {sender} {}", separator_date(date))?;
    if let Some(flags) = flags
        && flags_of(raw).is_none()
    {
        out.write_all(if flags.seen {
            b"Status: RO\n"
        } else {
            b"Status: O\n"
        })?;
        let mut x = String::new();
        if flags.answered {
            x.push('A');
        }
        if flags.flagged {
            x.push('F');
        }
        if !x.is_empty() {
            writeln!(out, "X-Status: {x}")?;
        }
    }
    for line in raw.split_inclusive(|b| *b == b'\n') {
        let body = line
            .strip_suffix(b"\n")
            .map_or(line, |l| l.strip_suffix(b"\r").unwrap_or(l));
        let quotes = body.iter().take_while(|b| **b == b'>').count();
        if body
            .get(quotes..)
            .is_some_and(|rest| rest.starts_with(b"From "))
        {
            out.write_all(b">")?;
        }
        out.write_all(body)?;
        out.write_all(b"\n")?;
    }
    out.write_all(b"\n")
}

/// A message's state as a mail file keeps it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct FileFlags {
    /// Read.
    pub seen: bool,
    /// Flagged for follow-up.
    pub flagged: bool,
    /// Replied to.
    pub answered: bool,
    /// Marked deleted (Thunderbird keeps these until compacting).
    pub deleted: bool,
}

impl FileFlags {
    /// The IMAP flags for an APPEND, such as `(\Seen \Flagged)`.
    #[must_use]
    pub fn imap(self) -> String {
        let mut flags = Vec::new();
        if self.seen {
            flags.push("\\Seen");
        }
        if self.flagged {
            flags.push("\\Flagged");
        }
        if self.answered {
            flags.push("\\Answered");
        }
        format!("({})", flags.join(" "))
    }
}

/// The header block of a raw message (up to its first blank line).
fn header_block(raw: &[u8]) -> &[u8] {
    let end = raw
        .windows(2)
        .position(|w| w == b"\n\n")
        .or_else(|| raw.windows(4).position(|w| w == b"\r\n\r\n"))
        .unwrap_or(raw.len());
    raw.get(..end).unwrap_or(raw)
}

/// The read, flagged and replied state written into a message by the
/// mail app that saved it (`Status`, `X-Status` as mutt and Unix mail
/// write them, `X-Mozilla-Status` as Thunderbird does). `None` when it has
/// none of them.
#[must_use]
pub fn flags_of(raw: &[u8]) -> Option<FileFlags> {
    let headers = String::from_utf8_lossy(header_block(raw));
    let mut flags = FileFlags::default();
    let mut found = false;
    for line in headers.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "status" => {
                found = true;
                flags.seen |= value.contains('R');
            }
            "x-status" => {
                found = true;
                flags.flagged |= value.contains('F');
                flags.answered |= value.contains('A');
                flags.deleted |= value.contains('D');
            }
            "x-mozilla-status" => {
                if let Ok(bits) = u32::from_str_radix(value, 16) {
                    found = true;
                    flags.seen |= bits & 0x0001 != 0;
                    flags.answered |= bits & 0x0002 != 0;
                    flags.flagged |= bits & 0x0004 != 0;
                    flags.deleted |= bits & 0x0008 != 0;
                }
            }
            _ => {}
        }
    }
    found.then_some(flags)
}

/// The Date and Message-ID headers of a message (the ID without angle
/// brackets).
#[must_use]
pub fn date_and_id(raw: &[u8]) -> (Option<i64>, Option<String>) {
    let Some(message) = MessageParser::default().parse_headers(raw) else {
        return (None, None);
    };
    let date = message.date().map(mail_parser::DateTime::to_timestamp);
    let id = message
        .message_id()
        .map(|id| {
            id.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_owned()
        })
        .filter(|id| !id.is_empty());
    (date, id)
}

/// The address an mbox `From ` line names for a message: its
/// `Return-Path`, else its sender.
#[must_use]
pub fn envelope_sender(raw: &[u8]) -> String {
    let Some(message) = MessageParser::default().parse_headers(raw) else {
        return String::new();
    };
    let return_path = message
        .header("Return-Path")
        .and_then(|value| value.as_text().map(str::to_owned))
        .map(|text| {
            text.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_owned()
        })
        .filter(|text| text.contains('@'));
    return_path
        .or_else(|| {
            message
                .from()
                .and_then(|from| from.first())
                .and_then(|from| from.address())
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A time as IMAP's `date-time` (RFC 3501), quoted, in UTC: the
/// `INTERNALDATE` given with an APPEND.
#[must_use]
pub fn imap_date(seconds: i64) -> String {
    use chrono::Datelike as _;
    let at = chrono::DateTime::from_timestamp(seconds, 0).unwrap_or_default();
    let month = usize::try_from(at.month0())
        .ok()
        .and_then(|m| MONTHS.get(m))
        .copied()
        .unwrap_or("Jan");
    format!(
        "\"{:02}-{month}-{} {} +0000\"",
        at.day(),
        at.format("%Y"),
        at.format("%H:%M:%S")
    )
}

/// A message with every line ending `\r\n`, as IMAP wants it.
#[must_use]
pub fn to_crlf(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len().saturating_add(raw.len() / 32));
    let mut previous = 0u8;
    for &byte in raw {
        if byte == b'\n' && previous != b'\r' {
            out.push(b'\r');
        }
        out.push(byte);
        previous = byte;
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn read_all(file: &[u8]) -> Vec<MboxItem> {
        MboxReader::new(file).map(Result::unwrap).collect()
    }

    fn messages(file: &[u8]) -> Vec<Vec<u8>> {
        read_all(file)
            .into_iter()
            .map(|item| match item {
                MboxItem::Message(raw) => raw,
                MboxItem::TooLarge(_) => Vec::new(),
            })
            .collect()
    }

    const FIRST: &[u8] = b"From: Wanjiru <wanjiru@shop.example>\nSubject: One\nMessage-ID: <one@shop.example>\nDate: Thu, 13 Nov 2026 09:42:00 +0300\n\nFrom the shop, hello.\n>From the archive.\n>>From deeper.\nFrom here on\n";
    const SECOND: &[u8] =
        b"From: Peter <peter@client.example>\nSubject: Two\n\nLast line, no blank after\n";

    #[test]
    fn mboxrd_round_trip_keeps_every_line() {
        let mut file = Vec::new();
        write_message(
            &mut file,
            FIRST,
            "wanjiru@shop.example",
            1_794_552_120,
            None,
        )
        .unwrap();
        write_message(&mut file, SECOND, "", 1_794_552_180, None).unwrap();
        let text = String::from_utf8_lossy(&file);
        assert!(text.starts_with("From wanjiru@shop.example Fri Nov 13 06:42:00 2026\n"));
        assert!(text.contains(
            "\n>From the shop, hello.\n>>From the archive.\n>>>From deeper.\n>From here on\n"
        ));
        assert!(text.contains("\n\nFrom MAILER-DAEMON "));
        assert!(is_mbox(&file));
        assert_eq!(messages(&file), vec![FIRST.to_vec(), SECOND.to_vec()]);
    }

    #[test]
    fn reads_mboxo_files_and_crlf() {
        // mboxo: only "From " lines escaped; a separator after a blank line.
        let file = b"From someone@x.example Mon Jan  5 10:00:00 2026\r\nSubject: a\r\n\r\n>From me\r\nline\r\n\r\nFrom other@x.example Mon Jan  5 11:00:00 2026\r\nSubject: b\r\n\r\nbody\r\n";
        let got = messages(file);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], b"Subject: a\r\n\r\nFrom me\r\nline\r\n".to_vec());
        assert_eq!(got[1], b"Subject: b\r\n\r\nbody\r\n".to_vec());
    }

    #[test]
    fn a_from_line_in_a_paragraph_is_not_a_separator() {
        let file = b"From a@x.example Mon Jan  5 10:00:00 2026\nSubject: a\n\nWe met.\nFrom then on it rained\n\nbye\n";
        let got = messages(file);
        assert_eq!(got.len(), 1);
        assert!(String::from_utf8_lossy(&got[0]).contains("From then on it rained"));
    }

    #[test]
    fn separators_without_a_blank_line_are_found_by_their_date() {
        let file = b"From a@x.example Mon Jan  5 10:00:00 2026\nSubject: a\n\nx\nFrom b@x.example Tue Jan  6 10:00:00 2026\nSubject: b\n\ny\n";
        assert_eq!(messages(file).len(), 2);
    }

    #[test]
    fn empty_and_junk_files_have_no_messages() {
        assert!(read_all(b"").is_empty());
        assert!(read_all(b"not a mailbox\n").is_empty());
        assert!(!is_mbox(b"Subject: x\r\n\r\n"));
        assert!(is_mbox(b"\xEF\xBB\xBF\r\nFrom x"));
    }

    #[test]
    fn progress_counts_bytes() {
        let file = b"From a@x.example Mon Jan  5 10:00:00 2026\nSubject: a\n\nx\n";
        let mut reader = MboxReader::new(&file[..]);
        reader.next_message().unwrap();
        assert_eq!(reader.position(), file.len() as u64);
    }

    #[test]
    fn status_headers_become_flags() {
        assert_eq!(flags_of(b"Subject: a\n\nStatus: RO\n"), None);
        let f = flags_of(b"Status: RO\nX-Status: AF\nSubject: a\n\nb").unwrap();
        assert!(f.seen && f.flagged && f.answered && !f.deleted);
        assert_eq!(f.imap(), "(\\Seen \\Flagged \\Answered)");
        let f = flags_of(b"X-Mozilla-Status: 0001\r\n\r\n").unwrap();
        assert!(f.seen && !f.flagged);
        assert_eq!(FileFlags::default().imap(), "()");
        // Written on export, read back on import.
        let mut file = Vec::new();
        let flags = FileFlags {
            seen: true,
            flagged: true,
            ..FileFlags::default()
        };
        write_message(&mut file, SECOND, "p@x", 0, Some(flags)).unwrap();
        let back = messages(&file).remove(0);
        assert_eq!(flags_of(&back), Some(flags));
    }

    #[test]
    fn dates_ids_and_senders() {
        let (date, id) = date_and_id(FIRST);
        assert_eq!(date, Some(1_794_552_120));
        assert_eq!(id.as_deref(), Some("one@shop.example"));
        assert_eq!(imap_date(1_794_552_120), "\"13-Nov-2026 06:42:00 +0000\"");
        assert_eq!(imap_date(0), "\"01-Jan-1970 00:00:00 +0000\"");
        assert_eq!(envelope_sender(FIRST), "wanjiru@shop.example");
        assert_eq!(
            envelope_sender(b"Return-Path: <bounce@list.example>\nFrom: a@b.example\n\nx"),
            "bounce@list.example"
        );
        assert_eq!(date_and_id(b""), (None, None));
    }

    #[test]
    fn line_ends_become_crlf_once() {
        assert_eq!(to_crlf(b"a\nb\r\nc"), b"a\r\nb\r\nc".to_vec());
        assert_eq!(to_crlf(b""), Vec::<u8>::new());
    }

    proptest::proptest! {
        #[test]
        fn any_body_survives_a_round_trip(lines in proptest::collection::vec("(>{0,3}From )?[a-zA-Z >]{0,20}", 0..20)) {
            let mut raw = b"Subject: p\n\n".to_vec();
            for line in &lines {
                raw.extend_from_slice(line.as_bytes());
                raw.push(b'\n');
            }
            // Trailing blank lines are the separator's, by the format.
            while raw.ends_with(b"\n\n") {
                raw.pop();
            }
            let mut file = Vec::new();
            write_message(&mut file, &raw, "a@b", 0, None).unwrap();
            write_message(&mut file, &raw, "a@b", 0, None).unwrap();
            let back = messages(&file);
            proptest::prop_assert_eq!(back, vec![raw.clone(), raw]);
        }

        #[test]
        fn any_bytes_never_panic(file in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..2048)) {
            let mut with_header = b"From x\n".to_vec();
            with_header.extend_from_slice(&file);
            let _ = read_all(&with_header);
        }
    }
}
