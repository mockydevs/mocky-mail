//! `.ics` files: a whole calendar in one file, for importing (one calendar
//! resource per event, as a `CalDAV` server keeps them) and exporting (one
//! `VCALENDAR` holding every event and the time zones they use).
//!
//! The events' own lines are kept as written: only the calendar around
//! them is new. An event and the changed occurrences of its series share a
//! `UID`, so they stay together in one resource.

use std::collections::BTreeMap;

use crate::vcard::{fold, split_line, unescape, unfold};

/// Largest calendar resource made from a file (the reader's own limit).
const MAX_RESOURCE_BYTES: usize = 4 * 1024 * 1024;
const PRODID: &str = "PRODID:-//Mocky Digital//Mocky Mail//EN";

/// One component inside a `VCALENDAR` (an event, a time zone, a to-do),
/// with its lines unfolded, from `BEGIN` to `END`.
#[derive(Debug, Clone)]
struct Block {
    name: String,
    lines: Vec<String>,
}

impl Block {
    fn value(&self, name: &str) -> Option<String> {
        self.lines.iter().find_map(|line| {
            let split = split_line(line)?;
            (split.name == name).then(|| unescape(split.value).trim().to_owned())
        })
    }

    /// The `TZID`s its dates are given in, and the first year each is used.
    fn zones_used(&self) -> Vec<(String, i32)> {
        let mut out: Vec<(String, i32)> = Vec::new();
        for line in &self.lines {
            let Some(split) = split_line(line) else {
                continue;
            };
            for param in split.params.split(';') {
                let Some((key, value)) = param.split_once('=') else {
                    continue;
                };
                if !key.trim().eq_ignore_ascii_case("TZID") {
                    continue;
                }
                let zone = value.trim().trim_matches('"').to_owned();
                let year = split
                    .value
                    .get(..4)
                    .and_then(|y| y.parse::<i32>().ok())
                    .unwrap_or(2026);
                if !zone.is_empty() && !out.iter().any(|(z, _)| *z == zone) {
                    out.push((zone, year));
                }
            }
        }
        out
    }
}

/// The components of every `VCALENDAR` in a text, in order.
fn blocks(text: &str) -> Vec<Block> {
    let unfolded = unfold(text);
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut current: Option<Block> = None;
    for line in unfolded.lines() {
        let Some(split) = split_line(line) else {
            continue;
        };
        let value = split.value.trim().to_ascii_uppercase();
        match split.name.as_str() {
            "BEGIN" => {
                depth = depth.saturating_add(1);
                if depth == 2 {
                    current = Some(Block {
                        name: value,
                        lines: Vec::new(),
                    });
                }
                if let Some(block) = current.as_mut() {
                    block.lines.push(line.to_owned());
                }
            }
            "END" => {
                if let Some(block) = current.as_mut() {
                    block.lines.push(line.to_owned());
                }
                if depth == 2
                    && let Some(block) = current.take()
                {
                    out.push(block);
                }
                depth = depth.saturating_sub(1);
            }
            _ => {
                if depth >= 2
                    && let Some(block) = current.as_mut()
                {
                    block.lines.push(line.to_owned());
                }
            }
        }
    }
    out
}

fn calendar_text(zones: &[&Block], items: &[&Block], name: Option<&str>) -> String {
    let mut out = String::new();
    fold("BEGIN:VCALENDAR", &mut out);
    fold("VERSION:2.0", &mut out);
    fold(PRODID, &mut out);
    fold("CALSCALE:GREGORIAN", &mut out);
    if let Some(name) = name.filter(|n| !n.trim().is_empty()) {
        fold(
            &format!("X-WR-CALNAME:{}", crate::vcard::escape(name.trim())),
            &mut out,
        );
    }
    for block in zones.iter().chain(items) {
        for line in &block.lines {
            fold(line, &mut out);
        }
    }
    fold("END:VCALENDAR", &mut out);
    out
}

/// One calendar resource made from a file: an event (with its changed
/// occurrences), ready to be stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarObject {
    /// Its `UID`.
    pub uid: String,
    /// The resource's text (`VCALENDAR` with the time zones it uses).
    pub ics: String,
}

/// What a file held.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IcsFile {
    /// One resource per event.
    pub objects: Vec<CalendarObject>,
    /// Events left out (too large).
    pub skipped: usize,
    /// To-dos and journal entries, which aren't brought in.
    pub other: usize,
}

/// Generated time zones, from the IANA (or Windows) name a date uses, for
/// one that the file names but doesn't define.
fn generated_zone(id: &str, year: i32) -> Option<Block> {
    let zone = crate::ical::zone_for(id)?;
    let lines = crate::ical::vtimezone(zone, year);
    // Written for the IANA name; the events name it as they do.
    let lines = lines
        .into_iter()
        .map(|line| {
            if line.starts_with("TZID:") {
                format!("TZID:{id}")
            } else {
                line
            }
        })
        .collect();
    Some(Block {
        name: "VTIMEZONE".to_owned(),
        lines,
    })
}

fn zones_for<'a>(
    items: &[&Block],
    defined: &'a BTreeMap<String, Block>,
    generated: &'a mut BTreeMap<String, Option<Block>>,
) -> Vec<&'a Block> {
    let mut wanted: Vec<(String, i32)> = Vec::new();
    for item in items {
        for (zone, year) in item.zones_used() {
            if !wanted.iter().any(|(z, _)| *z == zone) {
                wanted.push((zone, year));
            }
        }
    }
    for (zone, year) in &wanted {
        if !defined.contains_key(zone) && !generated.contains_key(zone) {
            generated.insert(zone.clone(), generated_zone(zone, *year));
        }
    }
    let generated: &'a BTreeMap<String, Option<Block>> = generated;
    wanted
        .iter()
        .filter_map(|(zone, _)| {
            defined
                .get(zone)
                .or_else(|| generated.get(zone).and_then(Option::as_ref))
        })
        .collect()
}

/// A text file's bytes as text: UTF-8 (a byte order mark dropped), UTF-16
/// by its byte order mark, else Windows-1252.
#[must_use]
pub fn file_text(bytes: &[u8]) -> String {
    if bytes.starts_with(b"\xFF\xFE") {
        return mailcraft_charset::decode(bytes, "utf-16le");
    }
    if bytes.starts_with(b"\xFE\xFF") {
        return mailcraft_charset::decode(bytes, "utf-16be");
    }
    mailcraft_charset::guess(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes))
}

/// Read an `.ics` file into one resource per event. An event without a
/// `UID` gets one from `new_uid`.
pub fn split_calendar(text: &str, new_uid: &mut dyn FnMut() -> String) -> IcsFile {
    let mut file = IcsFile::default();
    let mut defined: BTreeMap<String, Block> = BTreeMap::new();
    let mut events: Vec<(String, Vec<Block>)> = Vec::new();
    for mut block in blocks(text) {
        match block.name.as_str() {
            "VTIMEZONE" => {
                if let Some(id) = block.value("TZID") {
                    defined.entry(id).or_insert(block);
                }
            }
            "VEVENT" => {
                let uid = if let Some(uid) = block.value("UID").filter(|u| !u.is_empty()) {
                    uid
                } else {
                    let uid = new_uid();
                    let at = usize::from(!block.lines.is_empty());
                    block
                        .lines
                        .insert(at, format!("UID:{}", crate::vcard::escape(&uid)));
                    uid
                };
                match events.iter_mut().find(|(u, _)| *u == uid) {
                    Some((_, list)) => list.push(block),
                    None => events.push((uid, vec![block])),
                }
            }
            "VTODO" | "VJOURNAL" => file.other = file.other.saturating_add(1),
            _ => {}
        }
    }
    let mut generated = BTreeMap::new();
    for (uid, list) in events {
        let items: Vec<&Block> = list.iter().collect();
        let zones = zones_for(&items, &defined, &mut generated);
        let ics = calendar_text(&zones, &items, None);
        if ics.len() > MAX_RESOURCE_BYTES {
            file.skipped = file.skipped.saturating_add(1);
            continue;
        }
        file.objects.push(CalendarObject { uid, ics });
    }
    file
}

/// Every stored resource of a calendar as one `.ics` file: their events
/// (and to-dos) as stored, and each time zone they use once, defined (by
/// the resource's own definition, else made from its name).
#[must_use]
pub fn join_calendars<'a>(
    resources: impl IntoIterator<Item = &'a str>,
    name: Option<&str>,
) -> String {
    let mut defined: BTreeMap<String, Block> = BTreeMap::new();
    let mut items: Vec<Block> = Vec::new();
    for resource in resources {
        for block in blocks(resource) {
            if block.name == "VTIMEZONE" {
                if let Some(id) = block.value("TZID") {
                    defined.entry(id).or_insert(block);
                }
            } else {
                items.push(block);
            }
        }
    }
    let refs: Vec<&Block> = items.iter().collect();
    let mut generated = BTreeMap::new();
    let zones = zones_for(&refs, &defined, &mut generated);
    calendar_text(&zones, &refs, name)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    const FILE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//EN\r\nMETHOD:PUBLISH\r\nX-WR-CALNAME:Work\r\nBEGIN:VTIMEZONE\r\nTZID:Africa/Nairobi\r\nBEGIN:STANDARD\r\nDTSTART:19700101T000000\r\nTZOFFSETFROM:+0300\r\nTZOFFSETTO:+0300\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VTIMEZONE\r\nTZID:Unused\r\nBEGIN:STANDARD\r\nDTSTART:19700101T000000\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:weekly@example\r\nDTSTART;TZID=Africa/Nairobi:20261124T140000\r\nDTEND;TZID=Africa/Nairobi:20261124T150000\r\nRRULE:FREQ=WEEKLY\r\nSUMMARY:Site visit\\, Kilimani\r\nDESCRIPTION:A long description that goes on and on so that it is folded over more than one line here\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:weekly@example\r\nRECURRENCE-ID;TZID=Africa/Nairobi:20261201T140000\r\nDTSTART;TZID=Africa/Nairobi:20261201T160000\r\nDTEND;TZID=Africa/Nairobi:20261201T170000\r\nSUMMARY:Site visit (later)\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART;VALUE=DATE:20261225\r\nDTEND;VALUE=DATE:20261226\r\nSUMMARY:Holiday\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:berlin@example\r\nDTSTART;TZID=Europe/Berlin:20260704T100000\r\nDTEND;TZID=Europe/Berlin:20260704T110000\r\nSUMMARY:Call\r\nEND:VEVENT\r\nBEGIN:VTODO\r\nUID:todo@example\r\nSUMMARY:Buy paint\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";

    fn split(text: &str) -> IcsFile {
        let mut n = 0u32;
        split_calendar(text, &mut || {
            n = n.saturating_add(1);
            format!("new-{n}@mocky.mail")
        })
    }

    #[test]
    fn one_resource_per_event_with_its_zones() {
        let file = split(FILE);
        assert_eq!(file.objects.len(), 3);
        assert_eq!(file.other, 1);
        assert_eq!(file.skipped, 0);
        let weekly = &file.objects[0];
        assert_eq!(weekly.uid, "weekly@example");
        assert_eq!(weekly.ics.matches("BEGIN:VEVENT").count(), 2);
        assert_eq!(weekly.ics.matches("BEGIN:VTIMEZONE").count(), 1);
        assert!(weekly.ics.contains("TZID:Africa/Nairobi"));
        assert!(!weekly.ics.contains("Unused") && !weekly.ics.contains("METHOD"));
        assert_eq!(
            crate::ical::uid_of(&weekly.ics).as_deref(),
            Some("weekly@example")
        );
        assert_eq!(
            crate::ical::master(&weekly.ics).unwrap().title,
            "Site visit, Kilimani"
        );
        // No UID: one is given.
        let holiday = &file.objects[1];
        assert_eq!(holiday.uid, "new-1@mocky.mail");
        assert_eq!(
            crate::ical::uid_of(&holiday.ics).as_deref(),
            Some("new-1@mocky.mail")
        );
        assert!(!holiday.ics.contains("VTIMEZONE"));
        // A zone named but not defined is made from its name.
        let berlin = &file.objects[2];
        assert!(berlin.ics.contains("TZID:Europe/Berlin"));
        assert!(berlin.ics.contains("BEGIN:DAYLIGHT"));
        assert!(berlin.ics.split("\r\n").all(|l| l.len() <= 75));
    }

    #[test]
    fn export_and_import_round_trip() {
        let first = split(FILE);
        let joined = join_calendars(first.objects.iter().map(|o| o.ics.as_str()), Some("Work"));
        assert!(joined.starts_with("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n"));
        assert!(joined.contains("X-WR-CALNAME:Work"));
        assert_eq!(joined.matches("BEGIN:VCALENDAR").count(), 1);
        assert_eq!(joined.matches("BEGIN:VEVENT").count(), 4);
        // Each zone once.
        assert_eq!(joined.matches("TZID:Africa/Nairobi\r\n").count(), 1);
        assert_eq!(joined.matches("BEGIN:VTIMEZONE").count(), 2);
        let second = split(&joined);
        assert_eq!(second, IcsFile { other: 0, ..first });
    }

    #[test]
    fn odd_input_gives_nothing() {
        assert_eq!(split(""), IcsFile::default());
        assert_eq!(split("BEGIN:VEVENT\nUID:x\nEND:VEVENT"), IcsFile::default());
        assert_eq!(
            split("BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x"),
            IcsFile::default()
        );
        let _ = join_calendars(["", "garbage", "BEGIN:VCALENDAR\nEND"], None);
    }
}
