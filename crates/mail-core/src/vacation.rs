//! Out-of-office replies as a Sieve script (RFC 5228, vacation RFC 5230),
//! run by the mail server as mail arrives, so replies go out while every
//! device is off.
//!
//! The app keeps its own script, [`SCRIPT_NAME`]. Its settings ride along
//! in a comment line, so they read back exactly. If another script was
//! running (webmail filters), the app's script includes it, so those
//! filters keep working, and it runs again when the reply is switched off.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// The app's script on the server.
pub const SCRIPT_NAME: &str = "mocky-mail";
const SETTINGS_MARK: &str = "# mocky-mail-settings: ";
const PREVIOUS_MARK: &str = "# mocky-mail-previous: ";
const MAX_SUBJECT: usize = 200;
const MAX_MESSAGE: usize = 8000;

/// Out-of-office settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Vacation {
    /// Replies are on.
    pub enabled: bool,
    /// Reply subject.
    pub subject: String,
    /// Reply text.
    pub message: String,
    /// First day (`YYYY-MM-DD`), or `None` from now.
    pub start: Option<String>,
    /// Last day (`YYYY-MM-DD`), or `None` until switched off.
    pub end: Option<String>,
    /// Days before the same sender gets the reply again.
    pub days: u32,
    /// Microsoft accounts: the reply for people outside the organisation
    /// (`None`: the same as `message`). Other servers ignore it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_message: Option<String>,
    /// Microsoft accounts: who outside the organisation gets a reply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_audience: Option<ExternalAudience>,
}

/// Who outside the organisation gets an automatic reply (Microsoft).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExternalAudience {
    /// Nobody outside.
    None,
    /// Only people in your contacts.
    Contacts,
    /// Everyone.
    All,
}

impl Default for Vacation {
    fn default() -> Self {
        Self {
            enabled: false,
            subject: "Out of office".to_owned(),
            message: String::new(),
            start: None,
            end: None,
            days: 1,
            external_message: None,
            external_audience: None,
        }
    }
}

fn is_date(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
        && chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()
}

/// Tidy and check settings from the editor.
///
/// # Errors
/// A message for the user.
pub fn checked(mut v: Vacation) -> Result<Vacation, &'static str> {
    let tidy = |s: &str, max: usize, lines: bool| -> String {
        s.chars()
            .filter(|c| !c.is_control() || (lines && *c == '\n'))
            .take(max)
            .collect::<String>()
            .trim()
            .to_owned()
    };
    v.subject = tidy(&v.subject, MAX_SUBJECT, false);
    v.message = tidy(&v.message.replace("\r\n", "\n"), MAX_MESSAGE, true);
    v.external_message = v
        .external_message
        .map(|m| tidy(&m.replace("\r\n", "\n"), MAX_MESSAGE, true))
        .filter(|m| !m.is_empty());
    v.start = v
        .start
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    v.end = v.end.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    v.days = v.days.clamp(1, 30);
    if v.subject.is_empty() {
        "Out of office".clone_into(&mut v.subject);
    }
    if [&v.start, &v.end]
        .into_iter()
        .flatten()
        .any(|d| !is_date(d))
    {
        return Err("Choose the dates again.");
    }
    if let (Some(a), Some(b)) = (&v.start, &v.end)
        && b < a
    {
        return Err("The last day is before the first.");
    }
    if v.enabled && v.message.is_empty() {
        return Err("Write the reply people will get.");
    }
    Ok(v)
}

/// A Sieve quoted string.
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The script for these settings. `addresses` are the account's own
/// addresses (replies only go to mail sent to them); `previous` is the
/// script that ran before, to keep running inside this one.
#[must_use]
pub fn script(v: &Vacation, addresses: &[String], previous: Option<&str>) -> String {
    let settings = serde_json::to_string(v).unwrap_or_default();
    let mut out = String::from(
        "# Mocky Mail out of office. Change it in Mocky Mail: edits here are replaced.\n",
    );
    out.push_str(SETTINGS_MARK);
    out.push_str(&settings);
    out.push('\n');
    if let Some(previous) = previous {
        out.push_str(PREVIOUS_MARK);
        out.push_str(&serde_json::to_string(previous).unwrap_or_default());
        out.push('\n');
    }
    let dated = v.start.is_some() || v.end.is_some();
    let mut requires = vec!["vacation"];
    if dated {
        requires.extend(["date", "relational"]);
    }
    if previous.is_some() {
        requires.push("include");
    }
    let _ = writeln!(
        out,
        "require [{}];",
        requires
            .iter()
            .map(|r| quoted(r))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let addresses = addresses
        .iter()
        .map(|a| quoted(a))
        .collect::<Vec<_>>()
        .join(", ");
    let action = format!(
        "vacation :days {} :subject {} :addresses [{addresses}] {};\n",
        v.days.max(1),
        quoted(&v.subject),
        quoted(&v.message)
    );
    let mut tests = Vec::new();
    if let Some(start) = &v.start {
        tests.push(format!(
            "currentdate :value \"ge\" \"date\" {}",
            quoted(start)
        ));
    }
    if let Some(end) = &v.end {
        tests.push(format!(
            "currentdate :value \"le\" \"date\" {}",
            quoted(end)
        ));
    }
    if tests.is_empty() {
        out.push_str(&action);
    } else {
        let _ = write!(out, "if allof ({}) {{\n    {action}}}\n", tests.join(", "));
    }
    if let Some(previous) = previous {
        let _ = writeln!(out, "include :personal {};", quoted(previous));
    }
    out
}

/// The settings and the included script, read back from the app's script.
#[must_use]
pub fn read(script: &str) -> Option<(Vacation, Option<String>)> {
    let settings = script
        .lines()
        .find_map(|l| l.strip_prefix(SETTINGS_MARK))
        .and_then(|json| serde_json::from_str::<Vacation>(json).ok())?;
    let previous = script
        .lines()
        .find_map(|l| l.strip_prefix(PREVIOUS_MARK))
        .and_then(|json| serde_json::from_str::<String>(json).ok());
    Some((settings, previous))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn away() -> Vacation {
        Vacation {
            enabled: true,
            subject: "Away \"until\" Monday".into(),
            message: "Hi,\nI'm away. Call +254 700 000 001 if it's urgent.\\".into(),
            start: Some("2026-12-20".into()),
            end: Some("2027-01-04".into()),
            days: 2,
            ..Vacation::default()
        }
    }

    #[test]
    fn writes_a_dated_reply_that_keeps_other_filters() {
        let text = script(&away(), &["info@mocky.co.ke".into()], Some("sogo"));
        assert!(text.contains("require [\"vacation\", \"date\", \"relational\", \"include\"];\n"));
        assert!(text.contains(
            "if allof (currentdate :value \"ge\" \"date\" \"2026-12-20\", currentdate :value \"le\" \"date\" \"2027-01-04\") {\n"
        ));
        assert!(text.contains(
            ":days 2 :subject \"Away \\\"until\\\" Monday\" :addresses [\"info@mocky.co.ke\"]"
        ));
        assert!(text.contains("urgent.\\\\\";"));
        assert!(text.ends_with("include :personal \"sogo\";\n"));
        let (back, previous) = read(&text).unwrap();
        assert_eq!(back, away());
        assert_eq!(previous.as_deref(), Some("sogo"));
    }

    #[test]
    fn an_open_ended_reply_has_no_dates() {
        let v = Vacation {
            start: None,
            end: None,
            ..away()
        };
        let text = script(&v, &["a@b.example".into(), "c@b.example".into()], None);
        assert!(text.contains("require [\"vacation\"];\n"));
        assert!(!text.contains("currentdate"));
        assert!(!text.contains("include"));
        assert!(text.contains(":addresses [\"a@b.example\", \"c@b.example\"]"));
        assert_eq!(read(&text).unwrap().1, None);
        assert!(read("require \"fileinto\";").is_none());
    }

    #[test]
    fn settings_are_checked() {
        assert!(checked(away()).is_ok());
        let bad = |f: fn(&mut Vacation)| {
            let mut v = away();
            f(&mut v);
            checked(v)
        };
        assert_eq!(
            bad(|v| v.end = Some("2026-12-01".into())),
            Err("The last day is before the first.")
        );
        assert_eq!(
            bad(|v| v.start = Some("2026-13-40".into())),
            Err("Choose the dates again.")
        );
        assert_eq!(
            bad(|v| v.message = "  ".into()),
            Err("Write the reply people will get.")
        );
        let tidy = checked(Vacation {
            subject: " ".into(),
            days: 0,
            start: Some(String::new()),
            message: "a\r\nb\u{0}".into(),
            ..away()
        })
        .unwrap();
        assert_eq!(tidy.subject, "Out of office");
        assert_eq!(tidy.days, 1);
        assert_eq!(tidy.start, None);
        assert_eq!(tidy.message, "a\nb");
        // Off, nothing written yet: fine.
        assert!(checked(Vacation::default()).is_ok());
    }
}
