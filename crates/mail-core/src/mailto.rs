//! `mailto:` links (RFC 6068), as other apps and web pages hand them to the
//! app. They only fill in a new message; nothing is sent without the user.
//!
//! Adapted from Overlook's `mailto.rs` (MIT License, Copyright (c) 2026
//! Last Refuge, <https://github.com/last-refuge/overlook>).

use serde::Serialize;

/// Longest subject kept, characters.
const MAX_SUBJECT: usize = 500;
/// Longest body kept, characters.
const MAX_BODY: usize = 100_000;
/// Longest address list kept, characters.
const MAX_ADDRESSES: usize = 2_000;

/// What a `mailto:` link asks to fill in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mailto {
    /// Recipients, comma separated, as compose's To field takes them.
    pub to: String,
    /// Cc, comma separated.
    pub cc: String,
    /// Bcc, comma separated.
    pub bcc: String,
    /// Subject.
    pub subject: String,
    /// Body text.
    pub body: String,
}

fn add(list: &mut String, more: &str) {
    let more = more.trim();
    if more.is_empty() {
        return;
    }
    if !list.is_empty() {
        list.push_str(", ");
    }
    list.push_str(more);
}

fn cap(text: String, max: usize) -> String {
    if text.chars().count() > max {
        text.chars().take(max).collect()
    } else {
        text
    }
}

/// One line: control characters (header injection) dropped.
fn one_line(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Read `mailto:a@b.c,d@e.f?subject=Hi&body=...&cc=...`. `None` for
/// anything else.
#[must_use]
pub fn parse(url: &str) -> Option<Mailto> {
    let url = url.trim();
    let rest = url
        .get(..7)
        .filter(|p| p.eq_ignore_ascii_case("mailto:"))
        .and_then(|_| url.get(7..))?;
    let (addresses, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut m = Mailto::default();
    add(&mut m.to, &one_line(&percent_decode(addresses)));
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode(value);
        match key.to_ascii_lowercase().as_str() {
            "to" => add(&mut m.to, &one_line(&value)),
            "cc" => add(&mut m.cc, &one_line(&value)),
            "bcc" => add(&mut m.bcc, &one_line(&value)),
            "subject" => m.subject = one_line(&value),
            "body" => m.body = value.replace("\r\n", "\n").replace('\r', "\n"),
            _ => {}
        }
    }
    Some(Mailto {
        to: cap(m.to, MAX_ADDRESSES),
        cc: cap(m.cc, MAX_ADDRESSES),
        bcc: cap(m.bcc, MAX_ADDRESSES),
        subject: cap(m.subject, MAX_SUBJECT),
        body: cap(m.body, MAX_BODY),
    })
}

/// `%XX` escapes undone; invalid ones are kept as typed.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        if b == b'%'
            && let (Some(h), Some(l)) = (
                hex(bytes.get(i.saturating_add(1)).copied()),
                hex(bytes.get(i.saturating_add(2)).copied()),
            )
        {
            out.push(h.saturating_mul(16).saturating_add(l));
            i = i.saturating_add(3);
            continue;
        }
        out.push(b);
        i = i.saturating_add(1);
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: Option<u8>) -> Option<u8> {
    match b? {
        c @ b'0'..=b'9' => c.checked_sub(b'0'),
        c @ b'a'..=b'f' => c.checked_sub(b'a')?.checked_add(10),
        c @ b'A'..=b'F' => c.checked_sub(b'A')?.checked_add(10),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn reads_addresses_and_fields() {
        let m = parse("mailto:info@mocky.co.ke?subject=Quote%20request&cc=sales@mocky.co.ke&body=Hi%20there,%0D%0A%0D%0AI%20need%20a%20website.").unwrap();
        assert_eq!(m.to, "info@mocky.co.ke");
        assert_eq!(m.cc, "sales@mocky.co.ke");
        assert_eq!(m.subject, "Quote request");
        assert_eq!(m.body, "Hi there,\n\nI need a website.");
        let several =
            parse("MAILTO:a@x.example,b@x.example?to=c@x.example&bcc=d@x.example").unwrap();
        assert_eq!(several.to, "a@x.example,b@x.example, c@x.example");
        assert_eq!(several.bcc, "d@x.example");
    }

    #[test]
    fn rejects_other_links_and_cleans_tricks() {
        assert!(parse("https://mocky.co.ke").is_none());
        assert!(parse("mail").is_none());
        assert_eq!(parse("mailto:").unwrap(), Mailto::default());
        // A newline can't sneak an extra header into the subject.
        let m = parse("mailto:a@x.example?subject=Hi%0D%0ABcc:%20evil@x.example").unwrap();
        assert_eq!(m.subject, "HiBcc: evil@x.example");
        // Broken escapes stay as typed; plus signs stay plus signs (RFC 6068).
        assert_eq!(
            parse("mailto:a+b@x.example?subject=100%25%2")
                .unwrap()
                .subject,
            "100%%2"
        );
        assert_eq!(parse("mailto:a+b@x.example").unwrap().to, "a+b@x.example");
        assert!(
            parse(&format!("mailto:a@x.example?body={}", "x".repeat(200_000)))
                .unwrap()
                .body
                .len()
                <= 100_000
        );
    }
}
