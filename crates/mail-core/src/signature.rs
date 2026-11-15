//! Email signatures.
//!
//! A signature is kept as cleaned HTML (as pasted from webmail such as `SOGo`,
//! with its formatting, links and logo) plus a plain-text version. Compose
//! shows and edits the plain text under the standard `-- ` separator. When the
//! message is sent with that block intact, an HTML version is added in which
//! the block is replaced by the full signature; plain-text readers get the
//! text.

use std::fmt::Write as _;

use crate::sanitize::{RemoteContent, sanitize_html};

/// Largest signature accepted, as cleaned HTML.
pub const MAX_SIGNATURE_BYTES: usize = 64 * 1024;

/// The line that starts a signature in plain text (RFC 3676: dash dash space).
pub const SEPARATOR: &str = "-- ";

/// A cleaned signature.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Signature {
    /// Cleaned HTML, safe to show and to send.
    pub html: String,
    /// Plain-text version, as shown in compose.
    pub text: String,
}

/// Why a signature was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    /// Too big once cleaned.
    #[error(
        "This signature is too large ({size} bytes; the limit is {max}). Use a linked logo instead of a pasted image."
    )]
    TooLarge {
        /// Cleaned size in bytes.
        size: usize,
        /// The limit.
        max: usize,
    },
}

/// Clean a signature as pasted or typed. `Ok(None)` when nothing is left
/// (the signature was cleared).
///
/// The HTML goes through the same sanitizer as incoming mail; its pictures
/// stay as links (recipients load them as they would from any email).
///
/// # Errors
/// [`SignatureError::TooLarge`] when the cleaned HTML is over the limit.
pub fn clean_signature(html: &str) -> Result<Option<Signature>, SignatureError> {
    let html = sanitize_html(html, RemoteContent::Allow).html;
    let html = html.trim().to_owned();
    let text = plain_text(&html);
    if text.is_empty() && !html.contains("<img") {
        return Ok(None);
    }
    if html.len() > MAX_SIGNATURE_BYTES {
        return Err(SignatureError::TooLarge {
            size: html.len(),
            max: MAX_SIGNATURE_BYTES,
        });
    }
    Ok(Some(Signature { html, text }))
}

/// Readable text for the HTML: line breaks kept, runs of blank lines
/// collapsed, trailing spaces removed.
fn plain_text(html: &str) -> String {
    let text = mail_parser::decoders::html::html_to_text(html);
    let mut out = String::new();
    let mut blank = false;
    for line in text.lines().map(str::trim_end) {
        if line.trim().is_empty() {
            blank = true;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank { "\n\n" } else { "\n" });
        }
        blank = false;
        out.push_str(line.trim_start_matches('\u{a0}'));
    }
    out
}

/// The block compose adds for a signature, separator included.
#[must_use]
pub fn signature_block(text: &str) -> String {
    format!("{SEPARATOR}\n{text}")
}

/// Where the signature block sits in a body, if it is there unchanged and
/// starts a line.
fn find_block(body: &str, text: &str) -> Option<(usize, usize)> {
    let block = signature_block(text);
    body.match_indices(&block)
        .map(|(start, _)| start)
        .find(|&start| {
            start == 0
                || body
                    .get(..start)
                    .is_some_and(|before| before.ends_with('\n'))
        })
        .map(|start| (start, start.saturating_add(block.len())))
}

fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
}

/// Plain text as HTML that keeps its lines and spacing.
fn text_block(text: &str, out: &mut String) {
    if text.trim().is_empty() {
        return;
    }
    out.push_str("<div style=\"white-space:pre-wrap\">");
    escape(text, out);
    out.push_str("</div>");
}

/// The HTML version of a body whose signature block is intact: the text as
/// written, with the block replaced by the full signature. `None` when the
/// block was edited or removed; the message then goes as plain text only.
#[must_use]
pub fn html_with_signature(body: &str, signature: &Signature) -> Option<String> {
    let body = body.replace("\r\n", "\n");
    let (start, end) = find_block(&body, &signature.text)?;
    let mut html = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"></head><body style=\"font-family:Arial,Helvetica,sans-serif;font-size:14px\">",
    );
    text_block(body.get(..start)?.trim_end_matches('\n'), &mut html);
    let _ = write!(
        html,
        "<div class=\"signature\" style=\"margin-top:16px\">{}</div>",
        signature.html
    );
    text_block(body.get(end..)?.trim_start_matches('\n'), &mut html);
    html.push_str("</body></html>");
    Some(html)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const SOGO: &str = r#"<div><b>Don Onyango</b><br>Mocky Digital<br><a href="https://mocky.co.ke">mocky.co.ke</a><br><img src="https://mocky.co.ke/logo.png" alt="Mocky"></div><script>steal()</script>"#;

    #[test]
    fn a_pasted_signature_is_cleaned_and_keeps_its_parts() {
        let s = clean_signature(SOGO).unwrap().unwrap();
        assert!(!s.html.contains("script"));
        assert!(s.html.contains("https://mocky.co.ke/logo.png"));
        assert!(s.html.contains("<b>Don Onyango</b>"));
        assert_eq!(s.text, "Don Onyango\nMocky Digital\nmocky.co.ke");
    }

    #[test]
    fn an_empty_signature_clears_it() {
        assert_eq!(clean_signature("  <div><br></div> ").unwrap(), None);
    }

    #[test]
    fn a_huge_signature_is_refused() {
        let big = format!("<p>{}</p>", "x".repeat(MAX_SIGNATURE_BYTES));
        assert!(matches!(
            clean_signature(&big),
            Err(SignatureError::TooLarge { .. })
        ));
    }

    #[test]
    fn the_html_version_puts_the_full_signature_where_the_block_was() {
        let s = clean_signature(SOGO).unwrap().unwrap();
        let body = format!(
            "Hi <Amina>,\n\nSee you Friday.\n\n{}\n\nOn Tue, Amina wrote:\n> Lunch?",
            signature_block(&s.text)
        );
        let html = html_with_signature(&body, &s).unwrap();
        assert!(html.contains("Hi &lt;Amina&gt;,"));
        assert!(html.contains(&s.html));
        assert!(html.find(&s.html).unwrap() < html.find("Amina wrote").unwrap());
        // The plain separator isn't shown in the HTML version.
        assert!(!html.contains("-- \n"));
    }

    #[test]
    fn an_edited_signature_sends_as_plain_text() {
        let s = clean_signature(SOGO).unwrap().unwrap();
        let body = format!(
            "Hi\n\n{}",
            signature_block(&s.text).replace("Mocky Digital", "Mocky")
        );
        assert_eq!(html_with_signature(&body, &s), None);
        assert_eq!(html_with_signature("Hi, no signature here", &s), None);
    }
}
