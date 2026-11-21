//! Formatting while writing. Compose edits Markdown-flavoured text:
//! `**bold**`, `*italic*`, `~~strike~~`, `` `code` ``, `[text](url)`, `- `
//! and `1. ` lists, `> ` quotes and `# ` headings. The plain-text part of a
//! message keeps it as typed (it reads naturally); [`render_markdown`] makes
//! the HTML part. Raw HTML typed by the user is escaped, never passed
//! through, so a message can't carry markup its author didn't see.
//!
//! Adapted from Overlook's `compose.rs` (MIT License, Copyright (c) 2026
//! Last Refuge, <https://github.com/last-refuge/overlook>).

use std::fmt::Write as _;

/// Whether the text uses any formatting marks, so it should also be sent
/// as HTML. Only what the author typed counts: quoted lines (`>`) are
/// someone else's text, and their marks would restyle the whole message.
/// Emphasis marks count when they wrap text (`**bold**`, `_word_`), not
/// inside names such as `my_quote_v2.pdf` or `__init__`.
#[must_use]
pub fn has_formatting(text: &str) -> bool {
    text.lines()
        .map(str::trim_start)
        .filter(|line| !line.starts_with('>'))
        .any(|line| {
            ["~~", "`", "]("].iter().any(|m| line.contains(m))
                || line.starts_with("- ")
                || line.starts_with("* ")
                || line.starts_with("# ")
                || line.starts_with("## ")
                || numbered(line)
                || ["**", "__", "*", "_"].iter().any(|m| wraps(line, m))
        })
}

/// `1. ` to `99. ` at the start of a line.
fn numbered(line: &str) -> bool {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    (1..=2).contains(&digits)
        && line
            .get(digits..)
            .is_some_and(|rest| rest.starts_with(". "))
}

/// Whether `marker` wraps some text on this line, as in `*word*` or
/// `**two words**`: the opening mark not straight after a letter, digit
/// or another mark, the closing one not straight before one, and no
/// space just inside either. A double underscore around a single name
/// (`__init__`, `__FILE__`) is code, not bold.
fn wraps(line: &str, marker: &str) -> bool {
    let mark = marker.chars().next().unwrap_or_default();
    let outside = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric() && c != mark);
    let inside = |c: Option<char>| c.is_some_and(|c| !c.is_whitespace() && c != mark);
    let mut from = 0usize;
    while let Some(start) = line
        .get(from..)
        .and_then(|rest| rest.find(marker))
        .map(|at| from.saturating_add(at))
    {
        let open = start.saturating_add(marker.len());
        let before = line.get(..start).unwrap_or_default();
        let after = line.get(open..).unwrap_or_default();
        from = open;
        if !outside(before.chars().next_back()) || !inside(after.chars().next()) {
            continue;
        }
        // Only its nearest closing mark: that is the pair Markdown makes.
        let Some(end) = after.find(marker) else {
            break;
        };
        let inner = after.get(..end).unwrap_or_default();
        let rest = after
            .get(end.saturating_add(marker.len())..)
            .unwrap_or_default();
        let name = inner.chars().all(|c| c.is_alphanumeric() || c == '_');
        if inside(inner.chars().next_back())
            && outside(rest.chars().next())
            && !(marker == "__" && name)
        {
            return true;
        }
        from = open.saturating_add(end).saturating_add(marker.len());
    }
    false
}

/// The text as HTML. A single line break stays a line break, the way mail
/// is typed.
#[must_use]
pub fn render_markdown(src: &str) -> String {
    let mut options = markdown::Options::gfm();
    options.compile.allow_dangerous_html = false;
    options.compile.allow_dangerous_protocol = false;
    match markdown::to_html_with_options(src, &options) {
        Ok(html) => soft_breaks_to_br(&html),
        Err(_) => plain_to_html(src),
    }
}

/// Escaped text that keeps its lines, for when formatting can't be read.
fn plain_to_html(src: &str) -> String {
    let mut out = String::from("<div style=\"white-space:pre-wrap\">");
    for c in src.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out.push_str("</div>");
    out
}

/// A soft line break inside a paragraph becomes `<br>`; mail readers would
/// otherwise run the lines together.
fn soft_breaks_to_br(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_pre = false;
    let mut in_tag = false;
    for (i, c) in html.char_indices() {
        let rest = html.get(i..).unwrap_or_default();
        if c == '<' {
            in_tag = true;
            if rest.starts_with("<pre") {
                in_pre = true;
            } else if rest.starts_with("</pre") {
                in_pre = false;
            }
        } else if c == '>' {
            in_tag = false;
        }
        if c == '\n' && !in_tag && !in_pre {
            let before = html.get(..i).unwrap_or_default().trim_end_matches(' ');
            let after = html
                .get(i.saturating_add(1)..)
                .unwrap_or_default()
                .trim_start_matches(' ');
            if ends_at_block_boundary(before) || starts_at_block_boundary(after) {
                out.push('\n');
            } else {
                out.push_str("<br>\n");
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn boundary_tag(tag: &str) -> bool {
    let name = tag
        .trim_start_matches(['<', '/'])
        .split(['>', ' ', '\t', '\n'])
        .next()
        .unwrap_or_default();
    matches!(
        name,
        "p" | "div"
            | "br"
            | "hr"
            | "pre"
            | "blockquote"
            | "ul"
            | "ol"
            | "li"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "table"
            | "thead"
            | "tbody"
            | "tr"
            | "td"
            | "th"
    )
}

fn ends_at_block_boundary(text: &str) -> bool {
    text.ends_with('>')
        && text
            .rfind('<')
            .is_some_and(|start| text.get(start..).is_some_and(boundary_tag))
}

fn starts_at_block_boundary(text: &str) -> bool {
    text.starts_with('<') && boundary_tag(text)
}

/// Styles that keep quotes and code readable in every mail app.
const STYLE: &str = "blockquote{margin:0 0 0 8px;padding-left:10px;border-left:3px solid #ccc;color:#555}code,pre{font-family:Consolas,Menlo,monospace;background:#f4f4f4}pre{padding:8px;white-space:pre-wrap}";

/// A whole HTML document for a formatted message without a signature.
#[must_use]
pub fn html_document(src: &str) -> String {
    let mut html = String::from("<!doctype html><html><head><meta charset=\"utf-8\"><style>");
    html.push_str(STYLE);
    html.push_str(
        "</style></head><body style=\"font-family:Arial,Helvetica,sans-serif;font-size:14px\">",
    );
    let _ = write!(html, "{}", render_markdown(src));
    html.push_str("</body></html>");
    html
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn notices_formatting_but_not_plain_mail() {
        for formatted in [
            "Please **confirm** by Friday",
            "Items:\n- Paint\n- Tiles",
            "1. Measure\n2. Quote",
            "See [the quote](https://mocky.co.ke/q)",
            "Use `INV-0142`",
            "This is *really* urgent",
        ] {
            assert!(has_formatting(formatted), "{formatted}");
        }
        for plain in [
            "Hi Peter,\n\nThanks. 5 * 3 = 15 and the file is my_quote_v2.pdf.\n\nDon",
            "On Tue, Amina wrote:\n> Lunch?\n> Friday",
            "Price: KES 2,500",
        ] {
            assert!(!has_formatting(plain), "{plain}");
        }
    }

    #[test]
    fn quoted_text_and_code_names_are_not_formatting() {
        for plain in [
            // Marks in the quoted message are the other person's.
            "Sure.\n\nOn Tue, Amina wrote:\n> Please **confirm** the `INV-0142` total\n> 1. Paint\n> - Tiles\n>> see [the quote](https://mocky.co.ke/q)",
            "Call __init__ first, then set my_var_name in __main__.py",
            "2 ** 3 is 8, and 4 * 5 * 6 is 120",
            "Use snake_case for file_names and *args",
        ] {
            assert!(!has_formatting(plain), "{plain}");
        }
        for formatted in [
            "This is __very important__",
            "This is _really_ urgent",
            "**Agreed.**\n\n> Lunch?",
            "Fixed in `__init__`",
        ] {
            assert!(has_formatting(formatted), "{formatted}");
        }
    }

    #[test]
    fn renders_mail_style_html() {
        let html =
            render_markdown("Hi Peter,\nPlease **confirm**:\n\n- Paint\n- Tiles\n\n> quoted");
        assert!(html.contains("Hi Peter,<br>"));
        assert!(html.contains("<strong>confirm</strong>"));
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li>Paint</li>"));
        assert!(html.contains("<blockquote>"));
    }

    #[test]
    fn typed_html_and_scripts_stay_text() {
        let html =
            render_markdown("<script>alert(1)</script> and <b>bold</b> [x](javascript:alert(1))");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<b>"));
        assert!(!html.contains("href=\"javascript:"));
    }

    #[test]
    fn documents_wrap_the_body() {
        let doc = html_document("**Hi**");
        assert!(doc.starts_with("<!doctype html>"));
        assert!(doc.contains("<strong>Hi</strong>"));
        assert!(doc.ends_with("</body></html>"));
    }
}
