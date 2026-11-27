//! Cleaning HTML email before it is shown.
//!
//! Received HTML is hostile until proven otherwise. We keep only a fixed
//! allow-list of tags, attributes, URL schemes and CSS properties (via
//! `ammonia`, an html5ever-based sanitizer), and by default we drop remote
//! images so opening a message does not tell the sender you read it.
//!
//! The UI also renders the result inside a sandboxed iframe with a strict
//! Content-Security-Policy. This module is the first of those two layers,
//! not the only one.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ammonia::{Builder, UrlRelative};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::Serialize;

use crate::message::InlineImage;

mod stylesheet;

/// Whether images and other content from the internet may load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RemoteContent {
    /// Drop remote images (the default; stops tracking pixels).
    #[default]
    Block,
    /// Keep remote images (the user trusts this sender).
    Allow,
}

/// Sanitized HTML ready for the sandboxed viewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SanitizedHtml {
    /// The cleaned markup.
    pub html: String,
    /// How many remote images were removed. The UI offers "Show images"
    /// when this is above zero.
    pub blocked_remote_images: usize,
}

/// CSS properties that only affect appearance. Anything that can position
/// content over the app, load a URL or run code is left out.
const ALLOWED_STYLE_PROPERTIES: &[&str] = &[
    "color",
    "background-color",
    "font",
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
    "letter-spacing",
    "line-height",
    "text-align",
    "text-decoration",
    "text-transform",
    "vertical-align",
    "white-space",
    "width",
    "max-width",
    "min-width",
    "height",
    "max-height",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "border",
    "border-top",
    "border-right",
    "border-bottom",
    "border-left",
    "border-color",
    "border-style",
    "border-width",
    "border-radius",
    "border-collapse",
    "border-spacing",
    "list-style-type",
    "display",
    // Table and text layout email relies on.
    "table-layout",
    "word-break",
    "word-wrap",
    "overflow-wrap",
    "word-spacing",
    "text-indent",
    "text-overflow",
    "direction",
    "float",
    "clear",
    "min-height",
    "box-sizing",
    "font-variant",
    "list-style",
    "list-style-position",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
    "border-top-style",
    "border-right-style",
    "border-bottom-style",
    "border-left-style",
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    // Rounded cards clip their corners with this; preheaders hide with it.
    "overflow",
    "overflow-x",
    "overflow-y",
    "opacity",
    "visibility",
];

fn is_remote(url: &str) -> bool {
    // Read the URL the way a browser does: it drops leading control
    // characters and spaces, and every tab and newline anywhere, so
    // `ht\ttps://` still loads over https.
    let lower: String = url
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .take(6)
        .collect::<String>()
        .to_ascii_lowercase();
    lower.starts_with("http:") || lower.starts_with("https:")
}

/// A `bgcolor` value: a hex or named colour, nothing that could load or run.
fn is_plain_colour_value(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 32
        && value
            .strip_prefix('#')
            .unwrap_or(value)
            .chars()
            .all(|c| c.is_ascii_alphanumeric())
}

/// `background` keywords that are not colours.
const BACKGROUND_KEYWORDS: &[&str] = &[
    "none", "repeat", "space", "round", "center", "top", "bottom", "left", "right", "fixed",
    "scroll", "local", "auto", "cover", "contain",
];

/// Whether one `background` token is a colour: `#hex`, `rgb()`/`hsl()`
/// with plain numbers, or a colour name.
fn is_colour_token(token: &str) -> bool {
    if let Some(hex) = token.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    for function in ["rgb(", "rgba(", "hsl(", "hsla("] {
        if let Some(inner) = token
            .strip_prefix(function)
            .and_then(|rest| rest.strip_suffix(')'))
        {
            return inner.chars().all(|c| {
                c.is_ascii_digit() || " .,%/-".contains(c) || c == 'd' || c == 'e' || c == 'g'
            });
        }
    }
    (3..=20).contains(&token.len())
        && token.chars().all(|c| c.is_ascii_alphabetic())
        && !BACKGROUND_KEYWORDS.contains(&token)
}

/// The colour in a `background` shorthand value, if it has one and nothing
/// that could load a URL or run code.
fn background_colour(value: &str) -> Option<String> {
    let lower = value.trim().to_ascii_lowercase();
    let lower = lower.strip_suffix("!important").unwrap_or(&lower).trim();
    if [
        "url(",
        "image",
        "expression",
        "var(",
        "attr(",
        "\\",
        "\"",
        "'",
        "/*",
        "<",
        "{",
    ]
    .iter()
    .any(|bad| lower.contains(bad))
    {
        return None;
    }
    // Split on spaces outside brackets, so `rgb(1, 2, 3)` stays whole.
    let mut tokens = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (i, c) in lower.char_indices() {
        match c {
            '(' => depth = depth.saturating_add(1),
            ')' => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && depth == 0 => {
                tokens.push(lower.get(start..i).unwrap_or_default());
                start = i.saturating_add(1);
            }
            _ => {}
        }
    }
    tokens.push(lower.get(start..).unwrap_or_default());
    tokens
        .into_iter()
        .find(|token| is_colour_token(token))
        .map(str::to_owned)
}

/// Keep the colour from a `background:` shorthand as `background-color`.
/// Email buttons are often `<td style="background:#FF5400">` with white
/// text; dropping the colour leaves the text invisible.
fn keep_background_colour(style: &str) -> Cow<'_, str> {
    if !style.to_ascii_lowercase().contains("background") {
        return Cow::Borrowed(style);
    }
    let declarations: Vec<String> = style
        .split(';')
        .map(|declaration| {
            let Some((property, value)) = declaration.split_once(':') else {
                return declaration.to_owned();
            };
            if !property.trim().eq_ignore_ascii_case("background") {
                return declaration.to_owned();
            }
            // No colour: leave it for the allow-list to remove.
            background_colour(value).map_or_else(
                || declaration.to_owned(),
                |colour| format!("background-color: {colour}"),
            )
        })
        .collect();
    Cow::Owned(declarations.join(";"))
}

/// Clean an HTML email body.
#[must_use]
pub fn sanitize_html(html: &str, remote: RemoteContent) -> SanitizedHtml {
    let blocked = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&blocked);

    let mut builder = Builder::default();
    builder
        .url_schemes(HashSet::from(["http", "https", "mailto", "tel", "cid"]))
        .url_relative(UrlRelative::Deny)
        // A <title> is the document's name, not part of the message: without
        // this its text shows up as stray words at the top of the email.
        .add_clean_content_tags(["title"])
        .link_rel(Some("noopener noreferrer"))
        // Links open as popups, which the app hands to the system browser;
        // the viewer itself never navigates.
        .set_tag_attribute_value("a", "target", "_blank")
        // `class` lets the email's own stylesheet (cleaned separately)
        // reach its elements; the rest is how email has always been laid out.
        .add_generic_attributes(["style", "class", "align", "valign", "dir", "role"])
        .filter_style_properties(ALLOWED_STYLE_PROPERTIES.iter().copied().collect())
        .rm_tag_attributes("img", ["srcset"])
        .add_tag_attributes("img", ["border"])
        // Table layout: without these a full-width email shrinks to its
        // content and sits against the left edge, as in no other client.
        .add_tag_attributes(
            "table",
            [
                "width",
                "height",
                "border",
                "cellpadding",
                "cellspacing",
                "bgcolor",
            ],
        )
        .add_tag_attributes("tr", ["height", "bgcolor"])
        .add_tag_attributes("td", ["width", "height", "nowrap", "bgcolor"])
        .add_tag_attributes("th", ["width", "height", "nowrap", "bgcolor"])
        .add_tag_attributes("col", ["width"])
        .add_tag_attributes("colgroup", ["width"])
        // Old email still sets its text with <font>.
        .add_tags(["font", "big"])
        .add_tag_attributes("font", ["color", "size", "face"])
        .attribute_filter(move |element, attribute, value| {
            if remote == RemoteContent::Block
                && element == "img"
                && attribute == "src"
                && is_remote(value)
            {
                counter.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            // A link's title says where it really goes (added below); the
            // sender's own could claim anything.
            if element == "a" && attribute == "title" {
                return None;
            }
            match attribute {
                "bgcolor" | "color" => is_plain_colour_value(value).then_some(Cow::Borrowed(value)),
                // Sizes are numbers or percentages, nothing else.
                "width" | "height" | "border" | "cellpadding" | "cellspacing" | "size" => value
                    .trim()
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, '%' | '.' | '+' | '-'))
                    .then_some(Cow::Borrowed(value)),
                // Runs before the CSS property allow-list, which then
                // removes whatever is left of `background`.
                "style" => Some(keep_background_colour(value)),
                _ => Some(Cow::Borrowed(value)),
            }
        });

    // The email's stylesheets, cleaned on their own and put back in front
    // of the body: the HTML sanitizer drops <style> blocks entirely.
    let sheet: Vec<String> = stylesheet::style_blocks(html)
        .into_iter()
        .map(stylesheet::clean_stylesheet)
        .filter(|css| !css.is_empty())
        .collect();
    let body = title_links(&builder.clean(html).to_string());
    let html = if sheet.is_empty() {
        body
    } else {
        format!("<style>{}</style>{body}", sheet.join(""))
    };
    SanitizedHtml {
        html,
        blocked_remote_images: blocked.load(Ordering::Relaxed),
    }
}

/// Most characters of a link's address shown in its title.
const MAX_TITLE_URL: usize = 300;

/// File types whose names look like domains (`invoice.pdf`).
const FILE_ENDINGS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "zip", "rar", "jpg", "jpeg", "png", "gif",
    "txt", "csv", "htm", "html", "php", "aspx", "ics", "eml", "mp3", "mp4",
];

/// A piece of serialized HTML: text, or a whole tag.
enum Piece<'a> {
    Text(&'a str),
    Tag(&'a str),
}

/// Split the sanitizer's own output into text and tags. It writes every
/// attribute value in double quotes (a `"` inside becomes `&quot;`) and
/// escapes `<` in text, and the elements that hold raw text (scripts,
/// styles) and comments are already gone, so a `<` outside quotes always
/// starts a tag and a `>` outside quotes always ends one.
fn pieces(html: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut in_tag = false;
    let mut in_quote = false;
    for (i, c) in html.char_indices() {
        match (in_tag, c) {
            (false, '<') => {
                if i > start {
                    out.push(Piece::Text(html.get(start..i).unwrap_or_default()));
                }
                start = i;
                in_tag = true;
                in_quote = false;
            }
            (true, '"') => in_quote = !in_quote,
            (true, '>') if !in_quote => {
                let end = i.saturating_add(1);
                out.push(Piece::Tag(html.get(start..end).unwrap_or_default()));
                start = end;
                in_tag = false;
            }
            _ => {}
        }
    }
    if start < html.len() {
        let rest = html.get(start..).unwrap_or_default();
        out.push(if in_tag {
            Piece::Tag(rest)
        } else {
            Piece::Text(rest)
        });
    }
    out
}

/// The value of an attribute in a serialized tag (still escaped).
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag.strip_prefix('<')?;
    // The tag name.
    rest = rest.trim_start_matches(|c: char| !c.is_whitespace() && c != '>');
    loop {
        rest = rest.trim_start();
        if rest.is_empty() || rest.starts_with('>') || rest.starts_with("/>") {
            return None;
        }
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '=' || c == '>')
            .unwrap_or(rest.len());
        let (key, after) = rest.split_at(end);
        rest = after;
        let value = if let Some(after_eq) = rest.strip_prefix("=\"") {
            let close = after_eq.find('"')?;
            let (value, after) = after_eq.split_at(close);
            rest = after.get(1..).unwrap_or_default();
            Some(value)
        } else {
            None
        };
        if key.eq_ignore_ascii_case(name) {
            return value;
        }
    }
}

/// Undo the escaping the serializer writes.
fn unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

fn escape_attribute(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The host after `scheme://`: no user name, port or trailing dot.
fn host_of(rest: &str) -> Option<String> {
    let authority = rest.split(['/', '?', '#', '\\']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = if host.starts_with('[') {
        host.split_inclusive(']').next()?
    } else {
        host.split(':').next()?
    };
    let host = host.trim_end_matches('.').to_lowercase();
    (!host.is_empty()).then_some(host)
}

/// The host a web link really goes to.
fn link_host(href: &str) -> Option<String> {
    let lower = href.trim().to_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))?;
    host_of(rest)
}

/// The host a link's visible text shows, when the text is a web address
/// or a domain (`www.bank.example`, `https://bank.example/login`).
fn shown_host(text: &str) -> Option<String> {
    let text = text
        .trim()
        .trim_matches(|c| matches!(c, '<' | '>' | '(' | ')' | '[' | ']' | '"' | '\''));
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = text.to_lowercase();
    let without = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    let rest = without.unwrap_or(&lower);
    let authority = rest.split(['/', '?', '#']).next()?;
    // An email address, not a web address.
    if without.is_none() && authority.contains('@') {
        return None;
    }
    let host = host_of(rest)?;
    let labels: Vec<&str> = host.split('.').collect();
    let tld = labels.last()?;
    let domain_like = labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_alphanumeric() || c == '-'))
        && tld.chars().count() >= 2
        && tld.chars().all(char::is_alphabetic);
    // `invoice.pdf` is a file name, unless it was written as an address.
    let file_name = without.is_none()
        && !host.starts_with("www.")
        && !rest.contains('/')
        && FILE_ENDINGS.contains(tld);
    (domain_like && !file_name).then_some(host)
}

/// The same site: equal ignoring `www.`, or one inside the other
/// (`example.com` shown, `click.example.com` real).
fn same_site(shown: &str, real: &str) -> bool {
    let a = shown.strip_prefix("www.").unwrap_or(shown);
    let b = real.strip_prefix("www.").unwrap_or(real);
    a == b || b.ends_with(&format!(".{a}")) || a.ends_with(&format!(".{b}"))
}

/// The title for a link: where it goes, with a warning first when the
/// text shows a different web address.
fn link_title(href: &str, visible: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.to_ascii_lowercase().starts_with("cid:") {
        return None;
    }
    let mut shown: String = href.chars().take(MAX_TITLE_URL).collect();
    if href.chars().count() > MAX_TITLE_URL {
        shown.push('…');
    }
    match (link_host(href), shown_host(visible)) {
        (Some(real), Some(claimed)) if !same_site(&claimed, &real) => Some(format!(
            "Link goes to {real}, not the address shown.\n{shown}"
        )),
        _ => Some(shown),
    }
}

/// Give every link in sanitized HTML a `title` with its real destination,
/// so pointing at it shows where it goes, and a warning when its text
/// looks like a different address.
fn title_links(html: &str) -> String {
    let pieces = pieces(html);
    let mut out = String::with_capacity(html.len().saturating_add(html.len() / 8));
    for (i, piece) in pieces.iter().enumerate() {
        let tag = match piece {
            Piece::Text(text) => {
                out.push_str(text);
                continue;
            }
            Piece::Tag(tag) => *tag,
        };
        let Some(after_name) = tag
            .strip_prefix("<a")
            .filter(|rest| rest.starts_with([' ', '>', '\t', '\n']))
        else {
            out.push_str(tag);
            continue;
        };
        let visible: String = pieces
            .iter()
            .skip(i.saturating_add(1))
            .take_while(|p| !matches!(p, Piece::Tag(t) if t.eq_ignore_ascii_case("</a>")))
            .filter_map(|p| match p {
                Piece::Text(t) => Some(unescape(t)),
                Piece::Tag(_) => None,
            })
            .collect();
        match attribute(tag, "href").and_then(|href| link_title(&unescape(href), &visible)) {
            Some(title) => {
                out.push_str("<a title=\"");
                out.push_str(&escape_attribute(&title));
                out.push('"');
                out.push_str(after_name);
            }
            None => out.push_str(tag),
        }
    }
    out
}

/// Replace `cid:` image references in *already sanitized* HTML with the
/// matching embedded images as `data:` URLs.
///
/// Only images parsed by `message::parse_body` (raster types, size-capped)
/// are used, and the replacement happens after sanitizing, so a sender cannot
/// smuggle a `data:` URL of their own choosing past the sanitizer.
#[must_use]
pub fn embed_inline_images(sanitized_html: &str, images: &[InlineImage]) -> String {
    let mut html = sanitized_html.to_owned();
    for image in images {
        if image.content_id.contains(['"', '<', '>', '&', '\'']) {
            continue;
        }
        let needle = format!("src=\"cid:{}\"", image.content_id);
        if html.contains(&needle) {
            let data_url = format!(
                "src=\"data:{};base64,{}\"",
                image.mime_type,
                STANDARD.encode(&image.data)
            );
            html = html.replace(&needle, &data_url);
        }
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn clean(html: &str) -> String {
        sanitize_html(html, RemoteContent::Block).html
    }

    #[test]
    fn drops_the_document_title_text() {
        let out = clean(
            "<html><head><title>Live Rating Notice</title></head><body><p>Body</p></body></html>",
        );
        assert!(!out.contains("Live Rating Notice"), "{out}");
        assert!(out.contains("<p>Body</p>"));
    }

    #[test]
    fn removes_scripts_and_event_handlers() {
        let out = clean(
            r#"<p onclick="steal()">Hi</p><script>alert(1)</script><img src="cid:a" onerror="x()">"#,
        );
        assert!(!out.contains("script"));
        assert!(!out.contains("onclick"));
        assert!(!out.contains("onerror"));
        assert!(out.contains("<p>Hi</p>"));
    }

    #[test]
    fn removes_javascript_and_data_links() {
        let out = clean(
            r#"<a href="javascript:alert(1)">a</a><a href=" JaVaScRiPt:x">b</a><a href="data:text/html,x">c</a>"#,
        );
        assert!(!out.to_ascii_lowercase().contains("javascript"));
        assert!(!out.contains("data:"));
    }

    #[test]
    fn removes_forms_iframes_svg_and_unsafe_styles() {
        let out = clean(
            r#"<form action="https://evil.example"><input name="password"></form><iframe src="https://evil.example"></iframe><svg><script>x</script></svg><style>@import url(https://evil.example/a.css); p{color:red;background:url(https://evil.example/t.gif)}</style><p>ok</p>"#,
        );
        for banned in [
            "<form",
            "<input",
            "<iframe",
            "<svg",
            "<script",
            "evil.example",
            "@import",
        ] {
            assert!(!out.contains(banned), "{banned} survived: {out}");
        }
        assert!(out.contains("<p>ok</p>"));
        // What is safe in the stylesheet stays, so the email keeps its look.
        assert!(out.starts_with("<style>p{color:red}</style>"), "{out}");
    }

    #[test]
    fn keeps_the_table_layout_email_is_built_with() {
        // The shape of a typical receipt: a full-width wrapper centring a card.
        let out = clean(
            r##"<table role="presentation" cellpadding="0" cellspacing="0" width="100%"><tr><td align="center" valign="top" class="outer"><table width="100%" style="max-width:640px" bgcolor="#ffffff"><tr><td width="60%" nowrap>Amount</td></tr></table></td></tr></table>"##,
        );
        for kept in [
            r#"width="100%""#,
            r#"cellpadding="0""#,
            r#"cellspacing="0""#,
            r#"align="center""#,
            r#"valign="top""#,
            r#"class="outer""#,
            r#"role="presentation""#,
            r#"width="60%""#,
            "nowrap",
            "max-width:640px",
        ] {
            assert!(out.contains(kept), "{kept} was dropped: {out}");
        }
        // Only numbers go in a size.
        assert!(
            !clean(r#"<table width="expression(1)"><tr><td>x</td></tr></table>"#)
                .contains("expression")
        );
    }

    #[test]
    fn links_open_without_referrer_or_opener() {
        let out = clean(r#"<a href="https://example.com">x</a>"#);
        assert!(out.contains(r#"rel="noopener noreferrer""#));
    }

    #[test]
    fn links_show_where_they_really_go() {
        let out = clean(
            r#"<a href="https://shop.example/order?id=1&amp;x=2" title="Your bank">Track your order</a>"#,
        );
        assert!(
            out.contains(r#"<a title="https://shop.example/order?id=1&amp;x=2" href="#),
            "{out}"
        );
        // The sender's own title is gone.
        assert!(!out.contains("Your bank"), "{out}");
        let mail = clean(r#"<a href="mailto:help@shop.example">Write to us</a>"#);
        assert!(
            mail.contains(r#"title="mailto:help@shop.example""#),
            "{mail}"
        );
        // No address, no title; inline references aren't links.
        assert!(!clean("<a>plain</a>").contains("title"));
        assert!(!clean(r#"<a href="cid:x">i</a>"#).contains("title"));
        // Long addresses are cut.
        let long = format!("https://t.example/{}", "a".repeat(1000));
        let out = clean(&format!(r#"<a href="{long}">x</a>"#));
        let title = attribute(&out, "title").unwrap();
        assert!(title.chars().count() <= MAX_TITLE_URL + 1, "{title}");
        assert!(title.ends_with('…'));
    }

    #[test]
    fn links_whose_text_shows_another_address_say_so() {
        let warned = |html: &str| {
            let out = clean(html);
            attribute(&out, "title")
                .unwrap_or_default()
                .starts_with("Link goes to")
        };
        let out =
            clean(r#"<a href="https://evil.example/login">https://www.mybank.co.ke/login</a>"#);
        assert!(
            out.contains("title=\"Link goes to evil.example, not the address shown.\nhttps://evil.example/login\""),
            "{out}"
        );
        assert!(warned(
            r#"<a href="https://paypal.com.evil.example/">paypal.com</a>"#
        ));
        assert!(warned(
            r#"<a href="https://www.paypal.com@evil.example/">www.paypal.com</a>"#
        ));
        assert!(warned(
            r#"<a href="http://evil.example:8080/"><b>www.</b>safaricom.co.ke</a>"#
        ));
        // The same site, a part of it, or words that aren't an address.
        assert!(!warned(
            r#"<a href="https://www.mocky.co.ke/x">mocky.co.ke</a>"#
        ));
        assert!(!warned(
            r#"<a href="https://click.mocky.co.ke/x">mocky.co.ke</a>"#
        ));
        assert!(!warned(
            r#"<a href="https://mocky.co.ke/">www.mocky.co.ke</a>"#
        ));
        assert!(!warned(
            r#"<a href="https://MOCKY.co.ke/">https://mocky.co.ke</a>"#
        ));
        assert!(!warned(
            r#"<a href="https://cdn.example/f">invoice.pdf</a>"#
        ));
        assert!(!warned(r#"<a href="https://t.example/">Pay now</a>"#));
        assert!(!warned(
            r#"<a href="https://t.example/">jane@mocky.co.ke</a>"#
        ));
        assert!(!warned(r#"<a href="https://t.example/">version 2.0</a>"#));
        assert!(!warned(
            r#"<a href="mailto:x@evil.example">www.bank.example</a>"#
        ));
    }

    #[test]
    fn titles_cannot_break_out_of_attributes() {
        // Markup inside an attribute value isn't a link, and quotes in the
        // address or the text stay inside the title.
        for html in [
            r#"<img src="cid:a" alt="<a onclick=x() href=https://e.example>">"#,
            r#"<p class="<a href=&quot;https://e.example&quot; onmouseover=x()>">t</p>"#,
            r#"<a href="https://e.example/&quot;onmouseover=&quot;x()">"www.bank.example"</a>"#,
            r#"<a href="https://e.example/'><img src=x onerror=y>">a<img alt="</a>" src="cid:q">b</a>"#,
        ] {
            let out = clean(html);
            let lower = out.to_ascii_lowercase();
            for (i, _) in lower
                .match_indices("onmouseover")
                .chain(lower.match_indices("onclick"))
                .chain(lower.match_indices("onerror"))
            {
                // Only ever inside a quoted value: an even number of quotes
                // in the tag before it.
                let tag_start = lower.get(..i).unwrap().rfind('<').unwrap();
                let quotes = lower.get(tag_start..i).unwrap().matches('"').count();
                assert!(quotes % 2 == 1, "{html} -> {out}");
            }
            // Every tag still parses with balanced quotes.
            for piece in pieces(&out) {
                if let Piece::Tag(tag) = piece {
                    assert!(tag.matches('"').count() % 2 == 0, "{html} -> {out}");
                }
            }
        }
    }

    #[test]
    fn links_open_in_a_new_window() {
        let out = clean(r#"<a href="https://example.com" target="_self">x</a>"#);
        assert!(out.contains(r#"target="_blank""#));
        assert!(!out.contains("_self"));
    }

    #[test]
    fn relative_urls_are_removed() {
        let out = clean(r#"<a href="/account/delete">x</a>"#);
        assert!(!out.contains("/account/delete"));
    }

    #[test]
    fn blocks_remote_images_and_counts_them() {
        let result = sanitize_html(
            r#"<img src="https://t.example/pixel.gif"><img src="//t.example/p.gif"><img src="cid:logo@x">"#,
            RemoteContent::Block,
        );
        // The https image is blocked and counted. The protocol-relative one
        // is dropped earlier as a relative URL, so it never loads at all,
        // even when remote content is allowed.
        assert_eq!(result.blocked_remote_images, 1);
        assert!(!result.html.contains("t.example"));
        assert!(result.html.contains("cid:logo@x"));
    }

    #[test]
    fn tabs_and_newlines_in_the_scheme_do_not_unblock_images() {
        // Browsers strip tabs and newlines from URLs, so these load over https.
        for html in [
            "<img src=\"ht\ttps://t.example/p.gif\">",
            "<img src=\"ht&#10;tps://t.example/p.gif\">",
            "<img src=\"h&#13;ttp://t.example/p.gif\">",
            "<img src=\" \t https://t.example/p.gif\">",
        ] {
            let result = sanitize_html(html, RemoteContent::Block);
            assert!(
                !result.html.contains("t.example"),
                "{html} kept: {}",
                result.html
            );
            assert_eq!(result.blocked_remote_images, 1, "{html}");
        }
    }

    #[test]
    fn allows_remote_images_when_trusted() {
        let result = sanitize_html(
            r#"<img src="https://cdn.example/a.png"><img src="//cdn.example/b.png">"#,
            RemoteContent::Allow,
        );
        assert_eq!(result.blocked_remote_images, 0);
        assert!(result.html.contains("https://cdn.example/a.png"));
        assert!(!result.html.contains("//cdn.example/b.png"));
    }

    #[test]
    fn srcset_cannot_sneak_remote_images_in() {
        let out = clean(r#"<img src="cid:a" srcset="https://t.example/p.gif 1x">"#);
        assert!(!out.contains("t.example"));
    }

    #[test]
    fn keeps_safe_styles_and_drops_dangerous_ones() {
        let out = clean(
            r#"<p style="color: red; position: fixed; top: 0; background-image: url(https://t.example/p.gif); behavior: url(x.htc)">x</p>"#,
        );
        assert!(out.contains("color"));
        for banned in ["position", "background-image", "behavior", "t.example"] {
            assert!(!out.contains(banned), "{banned} survived: {out}");
        }
    }

    #[test]
    fn button_colours_survive() {
        // A real M-Pesa payment button: white text on an orange cell.
        let out = clean(
            r##"<table><tr><td style="background:#FF5400;border-radius:4px;"><a href="https://pay.example/x" style="color:#ffffff;">Pay KES 8,500 with M-Pesa</a></td><td bgcolor="#FF5400">x</td><th bgcolor="orange">y</th></tr></table>"##,
        );
        assert!(
            out.contains("background-color:#ff5400") || out.contains("background-color: #ff5400"),
            "{out}"
        );
        assert!(out.contains("border-radius"), "{out}");
        assert!(out.contains(r##"bgcolor="#FF5400""##), "{out}");
        assert!(out.contains(r#"bgcolor="orange""#), "{out}");
        assert!(
            out.contains("color:#ffffff") || out.contains("color: #ffffff"),
            "{out}"
        );
    }

    #[test]
    fn background_keeps_only_a_plain_colour() {
        for (style, colour) in [
            (
                "background: #fff url(https://t.example/p.gif) no-repeat",
                None,
            ),
            ("background: red", Some("red")),
            (
                "background: rgb(255, 84, 0) !important",
                Some("rgb(255, 84, 0)"),
            ),
            ("background: none", None),
            ("background: image-set('x.png' 1x)", None),
            ("background: expression(alert(1))", None),
            ("background: \\75 rl(x)", None),
            ("background: var(--x)", None),
        ] {
            let out = clean(&format!(r#"<p style="{style}">x</p>"#));
            assert!(
                !out.contains("t.example") && !out.contains("url") && !out.contains("expression"),
                "{style}: {out}"
            );
            assert!(!out.contains("background:"), "{style}: {out}");
            match colour {
                Some(c) => assert!(
                    out.contains(&format!("background-color:{c}"))
                        || out.contains(&format!("background-color: {c}")),
                    "{style}: {out}"
                ),
                None => assert!(!out.contains("background"), "{style}: {out}"),
            }
        }
    }

    #[test]
    fn bgcolor_must_be_a_plain_colour() {
        let out = clean(
            r##"<table bgcolor="#123456"><tr bgcolor="red"><td bgcolor="url(https://t.example/x)">a</td><td bgcolor="red;x:y">b</td></tr></table><p bgcolor="red">c</p>"##,
        );
        assert!(out.contains(r##"bgcolor="#123456""##));
        assert!(out.contains(r#"<tr bgcolor="red">"#));
        assert!(!out.contains("t.example"));
        assert!(!out.contains("red;x"));
        assert!(
            out.contains("<p>c</p>"),
            "bgcolor only on table cells: {out}"
        );
    }

    #[test]
    fn keeps_ordinary_email_formatting() {
        let out = clean(
            r#"<table><tr><td style="padding: 8px"><b>Total</b></td><td>KES 4,500</td></tr></table>"#,
        );
        assert!(out.contains("<table>"));
        assert!(out.contains("<b>Total</b>"));
        assert!(out.contains("padding"));
    }

    #[test]
    fn embeds_known_inline_images_only() {
        let images = vec![InlineImage {
            content_id: "logo@x".into(),
            mime_type: "image/png".into(),
            data: vec![1, 2, 3],
        }];
        let html = clean(r#"<img src="cid:logo@x"><img src="cid:other@x">"#);
        let out = embed_inline_images(&html, &images);
        assert!(out.contains(r#"src="data:image/png;base64,AQID""#));
        assert!(out.contains("cid:other@x"));
    }

    #[test]
    fn content_ids_with_markup_are_ignored() {
        let images = vec![InlineImage {
            content_id: "x\" onerror=\"y".into(),
            mime_type: "image/png".into(),
            data: vec![1],
        }];
        let html = r#"<img src="cid:x" onerror="y">"#;
        assert_eq!(embed_inline_images(html, &images), html);
    }

    proptest! {
        #[test]
        fn arbitrary_input_never_yields_script_or_handlers(input in ".{0,400}") {
            let wrapped = format!("<div>{input}<script>x</script><img src=x onerror=y></div>");
            let out = clean(&wrapped).to_ascii_lowercase();
            prop_assert!(!out.contains("<script"));
            prop_assert!(!out.contains("onerror"));
        }

        #[test]
        fn titled_links_keep_tags_well_formed(text in ".{0,60}", href in "[ -~]{0,60}") {
            let html = format!(
                "<p>{text}<a href=\"https://{href}\">{text}</a><img alt=\"<a href='x'>{href}\" src=\"cid:a\"></p>"
            );
            let out = clean(&html);
            for piece in pieces(&out) {
                if let Piece::Tag(tag) = piece {
                    prop_assert!(tag.matches('"').count() % 2 == 0, "{}", out);
                }
            }
        }
    }
}
