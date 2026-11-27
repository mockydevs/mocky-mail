//! Rich-text (HTML) messages from the compose editor.
//!
//! The editor's HTML is cleaned for sending with its own allow-list, which
//! is stricter than the one for reading mail: the formatting an email needs
//! (paragraphs, bold, italic, underline, strike, fonts and colours, lists,
//! quotes, links, simple tables, rules, headings) and nothing that runs,
//! loads from the internet or asks for input. Pictures stay only when they
//! travel inside the message (`cid:` and `data:` images); `data:` pictures
//! are turned into parts of the message, since many mail apps don't show
//! them. A plain-text version is made from the result for mail apps that
//! read text only.
//!
//! Compose marks two places with attributes the backend acts on:
//!
//! - `data-mocky-quote` on the element holding the quoted original of a
//!   reply or forward (best with its "On …, … wrote:" line inside it). The
//!   signature goes just before it.
//! - `data-mocky-signature="<signature id>"` on an element standing for
//!   the signature. It is replaced by that signature in full. Messages
//!   sent or saved from here carry the mark on their signature, so a
//!   continued draft gets the signature as it is now.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use ammonia::{Builder, UrlRelative};
use base64::Engine as _;

use crate::message::{InlineImage, MAX_INLINE_IMAGE_BYTES};
use crate::sanitize::{Piece, attribute, escape_attribute, pieces, unescape};
use crate::signature::{SEPARATOR, Signature, signature_html};

/// Marks the quoted original in compose HTML (an attribute, no value).
pub const QUOTE_MARKER: &str = "data-mocky-quote";
/// Marks where the signature goes in compose HTML; its value is the id of
/// one of the account's signatures.
pub const SIGNATURE_MARKER: &str = "data-mocky-signature";
/// In the head of every rich-text message sent or saved, so the app can
/// tell it from HTML it made out of plain text.
pub const RICH_MARKER: &str = "<meta name=\"mocky-compose\" content=\"rich\">";
/// Lists nested deeper than this are flattened into the one above.
pub const MAX_LIST_DEPTH: usize = 8;

/// Elements kept.
const TAGS: &[&str] = &[
    "p",
    "br",
    "div",
    "span",
    "b",
    "strong",
    "i",
    "em",
    "u",
    "s",
    "strike",
    "del",
    "ins",
    "sub",
    "sup",
    "small",
    "big",
    "font",
    "code",
    "pre",
    "ol",
    "ul",
    "li",
    "blockquote",
    "a",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "td",
    "th",
    "caption",
    "colgroup",
    "col",
    "img",
    "hr",
    "h1",
    "h2",
    "h3",
];

/// Elements removed together with everything in them.
const DROPPED_WITH_CONTENT: &[&str] = &[
    "title", "form", "button", "select", "textarea", "iframe", "frame", "frameset", "object",
    "embed", "applet", "noscript", "template", "svg", "math", "audio", "video", "canvas", "map",
];

/// CSS properties kept in `style`: how text and simple tables look.
const STYLE_PROPERTIES: &[&str] = &[
    "color",
    "background-color",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "text-decoration",
    "text-decoration-line",
    "text-align",
    "text-indent",
    "line-height",
    "vertical-align",
    "white-space",
    "direction",
    "list-style-type",
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
    "border-collapse",
    "border-spacing",
    "border-radius",
    "width",
    "height",
    "max-width",
];

/// Styles that keep quotes and code readable in every mail app.
const STYLE: &str = "blockquote{margin:0 0 0 8px;padding-left:10px;border-left:3px solid #ccc;color:#555}code,pre{font-family:Consolas,Menlo,monospace;background:#f4f4f4}pre{padding:8px;white-space:pre-wrap}";

/// Why a rich-text message can't be sent as written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RichError {
    /// A picture pasted into the message is too big to go inside it.
    #[error(
        "A picture in this message is {size_kb} KB; pictures inside a message can be up to 5 MB. Attach it as a file instead."
    )]
    PictureTooLarge {
        /// Its size in KB.
        size_kb: usize,
    },
}

/// A rich-text message ready to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichBody {
    /// The whole HTML document.
    pub html: String,
    /// The plain-text version.
    pub text: String,
    /// Pictures the HTML shows by `cid:` (from `data:` pictures).
    pub inline_images: Vec<InlineImage>,
}

/// Compose HTML cleaned for sending (a fragment, no document around it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanHtml {
    html: String,
}

/// `(closing, lower-case name)` of a serialized tag.
fn tag_name(tag: &str) -> (bool, String) {
    let inner = tag.trim_start_matches('<');
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, inner),
    };
    let name = inner
        .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    (closing, name)
}

fn is_void(name: &str) -> bool {
    matches!(name, "br" | "img" | "hr" | "col")
}

/// A size: digits, maybe a percentage.
fn is_size(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 10
        && value
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '%' | '.' | '+' | '-'))
}

/// `#hex` or a colour name.
fn is_colour(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 32
        && value
            .strip_prefix('#')
            .unwrap_or(value)
            .chars()
            .all(|c| c.is_ascii_alphanumeric())
}

/// A `data:` picture as pasted: PNG, JPEG, GIF or WebP, base64.
fn is_data_picture(value: &str) -> bool {
    let lower: String = value.trim_start().chars().take(32).collect::<String>();
    let lower = lower.to_ascii_lowercase();
    ["png", "jpeg", "jpg", "gif", "webp"]
        .iter()
        .any(|kind| lower.starts_with(&format!("data:image/{kind};base64,")))
}

/// A declaration list with anything that could load or run removed.
fn safe_style(value: &str) -> String {
    value
        .split(';')
        .filter(|declaration| {
            let lower = declaration.to_ascii_lowercase();
            ![
                "url(",
                "image",
                "expression",
                "javascript:",
                "var(",
                "attr(",
                "\\",
                "/*",
                "@",
                "<",
                "{",
            ]
            .iter()
            .any(|bad| lower.contains(bad))
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn attribute_value<'a>(element: &str, attribute: &str, value: &'a str) -> Option<Cow<'a, str>> {
    let keep = Some(Cow::Borrowed(value));
    match (element, attribute) {
        (_, QUOTE_MARKER) => Some(Cow::Borrowed("")),
        (_, SIGNATURE_MARKER) => {
            let id = value.trim();
            let ok = id.len() <= 64
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            Some(if ok {
                Cow::Borrowed(id)
            } else {
                Cow::Borrowed("")
            })
        }
        ("a", "href") => {
            let lower = value.trim_start().to_ascii_lowercase();
            ["http://", "https://", "mailto:"]
                .iter()
                .any(|scheme| lower.starts_with(scheme))
                .then_some(Cow::Borrowed(value))
        }
        ("img", "src") => {
            let lower = value.trim_start().to_ascii_lowercase();
            (lower.starts_with("cid:") || is_data_picture(value)).then_some(Cow::Borrowed(value))
        }
        (_, "style") => Some(Cow::Owned(safe_style(value))),
        (_, "color" | "bgcolor") => is_colour(value).then_some(Cow::Borrowed(value)),
        (
            _,
            "width" | "height" | "border" | "cellpadding" | "cellspacing" | "colspan" | "rowspan"
            | "span" | "size" | "start" | "value",
        ) => is_size(value).then_some(Cow::Borrowed(value)),
        (_, "face") => value
            .chars()
            .all(|c| c.is_alphanumeric() || " ,-_'\"".contains(c))
            .then_some(Cow::Borrowed(value)),
        (_, "align") => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "left" | "right" | "center" | "justify"
        )
        .then_some(Cow::Borrowed(value)),
        (_, "valign") => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "top" | "middle" | "bottom" | "baseline"
        )
        .then_some(Cow::Borrowed(value)),
        (_, "dir") => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "ltr" | "rtl" | "auto"
        )
        .then_some(Cow::Borrowed(value)),
        (_, "type") => matches!(
            value.trim(),
            "1" | "a" | "A" | "i" | "I" | "disc" | "circle" | "square"
        )
        .then_some(Cow::Borrowed(value)),
        _ => keep,
    }
}

fn builder() -> Builder<'static> {
    let mut tag_attributes: HashMap<&'static str, HashSet<&'static str>> = HashMap::new();
    for (tag, attributes) in [
        ("a", &["href"][..]),
        ("img", &["src", "alt", "width", "height"][..]),
        ("font", &["face", "size", "color"][..]),
        (
            "table",
            &["width", "border", "cellpadding", "cellspacing", "bgcolor"][..],
        ),
        ("tr", &["bgcolor", "valign"][..]),
        (
            "td",
            &[
                "width", "height", "colspan", "rowspan", "bgcolor", "valign", "nowrap",
            ][..],
        ),
        (
            "th",
            &[
                "width", "height", "colspan", "rowspan", "bgcolor", "valign", "nowrap",
            ][..],
        ),
        ("col", &["width", "span"][..]),
        ("colgroup", &["width", "span"][..]),
        ("ol", &["start", "type"][..]),
        ("ul", &["type"][..]),
        ("li", &["value"][..]),
    ] {
        tag_attributes.insert(tag, attributes.iter().copied().collect());
    }
    let mut builder = Builder::empty();
    builder
        .tags(TAGS.iter().copied().collect())
        .add_clean_content_tags(DROPPED_WITH_CONTENT.iter().copied())
        .generic_attributes(HashSet::from([
            "style",
            "dir",
            "align",
            QUOTE_MARKER,
            SIGNATURE_MARKER,
        ]))
        .tag_attributes(tag_attributes)
        .url_schemes(HashSet::from(["http", "https", "mailto", "cid", "data"]))
        .url_relative(UrlRelative::Deny)
        .link_rel(None)
        .strip_comments(true)
        .filter_style_properties(STYLE_PROPERTIES.iter().copied().collect())
        .attribute_filter(attribute_value);
    builder
}

/// Lists deeper than [`MAX_LIST_DEPTH`] are flattened into their parent
/// item, and pictures left without a source are dropped.
fn tidy(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    // Open lists and items: whether each was kept.
    let mut lists: Vec<bool> = Vec::new();
    let mut items: Vec<bool> = Vec::new();
    for piece in pieces(html) {
        let tag = match piece {
            Piece::Text(text) => {
                out.push_str(text);
                continue;
            }
            Piece::Tag(tag) => tag,
        };
        let (closing, name) = tag_name(tag);
        let keep = match (closing, name.as_str()) {
            (false, "ol" | "ul") => {
                let kept = lists.iter().filter(|kept| **kept).count() < MAX_LIST_DEPTH;
                lists.push(kept);
                kept
            }
            (true, "ol" | "ul") => lists.pop().unwrap_or(true),
            (false, "li") => {
                let kept = lists.last().copied().unwrap_or(true);
                items.push(kept);
                kept
            }
            (true, "li") => items.pop().unwrap_or(true),
            (false, "img") => attribute(tag, "src").is_some_and(|src| !src.is_empty()),
            _ => true,
        };
        if keep {
            out.push_str(tag);
        }
    }
    out
}

/// Clean compose HTML for sending.
#[must_use]
pub fn clean(html: &str) -> CleanHtml {
    CleanHtml {
        html: tidy(&builder().clean(html).to_string()),
    }
}

impl CleanHtml {
    /// The cleaned fragment.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.html
    }

    /// The signature id on the element marked `data-mocky-signature`, if
    /// the HTML has one (`""` when it names none).
    #[must_use]
    pub fn signature_placeholder(&self) -> Option<String> {
        pieces(&self.html)
            .into_iter()
            .find_map(|piece| match piece {
                Piece::Tag(tag) if !tag.starts_with("</") => {
                    attribute(tag, SIGNATURE_MARKER).map(unescape)
                }
                _ => None,
            })
    }

    /// The plain-text version of the cleaned HTML.
    #[must_use]
    pub fn text(&self) -> String {
        html_to_text(&self.html)
    }
}

/// Where the signature goes: `(start, end)` of what it replaces.
fn signature_place(html: &str) -> (usize, usize) {
    let mut offset = 0usize;
    let mut quote: Option<usize> = None;
    let mut placeholder: Option<(usize, String, usize)> = None;
    for piece in pieces(html) {
        let (text, is_tag) = match piece {
            Piece::Text(text) => (text, false),
            Piece::Tag(tag) => (tag, true),
        };
        let start = offset;
        offset = offset.saturating_add(text.len());
        if !is_tag {
            continue;
        }
        let (closing, name) = tag_name(text);
        match &mut placeholder {
            None if !closing && attribute(text, SIGNATURE_MARKER).is_some() => {
                if is_void(&name) {
                    return (start, offset);
                }
                placeholder = Some((start, name, 1));
            }
            None => {
                if quote.is_none() && !closing && attribute(text, QUOTE_MARKER).is_some() {
                    quote = Some(start);
                }
            }
            Some((from, open, depth)) => {
                if *open == name && !is_void(&name) {
                    if closing {
                        *depth = depth.saturating_sub(1);
                        if *depth == 0 {
                            return (*from, offset);
                        }
                    } else {
                        *depth = depth.saturating_add(1);
                    }
                }
            }
        }
    }
    match (placeholder, quote) {
        // Never closed: to the end.
        (Some((from, _, _)), _) => (from, html.len()),
        (None, Some(at)) => (at, at),
        (None, None) => (html.len(), html.len()),
    }
}

/// 64-bit FNV-1a, to name a picture after its bytes.
fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The picture's type from its own bytes.
fn picture_type(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if data.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

/// `data:` pictures become parts of the message, shown by `cid:`. Ones
/// that aren't really pictures are dropped.
fn embed_pictures(html: &str) -> Result<(String, Vec<InlineImage>), RichError> {
    let mut out = String::with_capacity(html.len());
    let mut images: Vec<InlineImage> = Vec::new();
    for piece in pieces(html) {
        let tag = match piece {
            Piece::Text(text) => {
                out.push_str(text);
                continue;
            }
            Piece::Tag(tag) => tag,
        };
        let src = attribute(tag, "src").filter(|_| tag_name(tag) == (false, "img".to_owned()));
        let Some(src) =
            src.filter(|src| src.trim_start().to_ascii_lowercase().starts_with("data:"))
        else {
            out.push_str(tag);
            continue;
        };
        let value = unescape(src);
        let data = value
            .split_once(',')
            .map(|(_, b64)| {
                b64.chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
            })
            .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok());
        let Some((data, mime)) = data.and_then(|d| picture_type(&d).map(|mime| (d, mime))) else {
            continue;
        };
        if data.len() > MAX_INLINE_IMAGE_BYTES {
            return Err(RichError::PictureTooLarge {
                size_kb: data.len().div_ceil(1024),
            });
        }
        let content_id = if let Some(same) = images.iter().find(|image| image.data == data) {
            same.content_id.clone()
        } else {
            let id = format!(
                "image{}.{:016x}@mocky.mail",
                images.len().saturating_add(1),
                fnv(&data)
            );
            images.push(InlineImage {
                content_id: id.clone(),
                mime_type: mime.to_owned(),
                data,
            });
            id
        };
        out.push_str(&tag.replacen(
            &format!("src=\"{src}\""),
            &format!("src=\"cid:{}\"", escape_attribute(&content_id)),
            1,
        ));
    }
    Ok((out, images))
}

/// The message to send: the cleaned HTML with the signature in place (see
/// the module notes), as a document, its plain text, and its pictures.
/// `signature` is the signature and the id to mark it with.
///
/// # Errors
/// [`RichError::PictureTooLarge`] for a pasted picture over 5 MB.
pub fn finish(
    clean: &CleanHtml,
    signature: Option<(&Signature, Option<&str>)>,
) -> Result<RichBody, RichError> {
    let html = clean.as_str();
    let (fragment, text) = match signature {
        Some((signature, id)) => {
            let (start, end) = signature_place(html);
            let before = html.get(..start).unwrap_or_default();
            let after = html.get(end..).unwrap_or_default();
            let mut text = html_to_text(before);
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            let _ = write!(text, "{SEPARATOR}\n{}", signature.text);
            let rest = html_to_text(after);
            if !rest.is_empty() {
                text.push_str("\n\n");
                text.push_str(&rest);
            }
            (
                format!("{before}{}{after}", signature_html(signature, id)),
                text,
            )
        }
        None => (html.to_owned(), html_to_text(html)),
    };
    let (fragment, inline_images) = embed_pictures(&fragment)?;
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">{RICH_MARKER}<style>{STYLE}</style></head><body style=\"font-family:Arial,Helvetica,sans-serif;font-size:14px\">{fragment}</body></html>"
    );
    Ok(RichBody {
        html,
        text,
        inline_images,
    })
}

/// Builds the plain-text version while walking the HTML.
#[derive(Default)]
struct TextWriter {
    out: String,
    /// Line breaks owed before the next text.
    pending: usize,
    /// Quote depth of the blank lines owed.
    pending_depth: usize,
    /// Current quote depth.
    depth: usize,
    /// A space is owed before the next word.
    space: bool,
    line_start: bool,
    /// Inside `<pre>`: spacing kept.
    pre: usize,
    /// The last thing written ends in its own space (a list marker, a
    /// table cell break), so the next word needs none.
    glued: bool,
}

impl TextWriter {
    fn owe(&mut self, breaks: usize, add: bool) {
        self.pending_depth = if self.pending == 0 {
            self.depth
        } else {
            self.pending_depth.min(self.depth)
        };
        self.pending = if add {
            self.pending.saturating_add(breaks)
        } else {
            self.pending.max(breaks)
        };
    }

    fn block(&mut self, breaks: usize) {
        self.owe(breaks, false);
    }

    fn line_break(&mut self) {
        self.owe(1, true);
    }

    fn flush(&mut self) {
        if self.out.is_empty() {
            self.pending = 0;
        }
        for i in 0..self.pending {
            if i > 0 {
                let depth = self.pending_depth.min(self.depth);
                self.out.push_str(&">".repeat(depth));
            }
            self.out.push('\n');
            self.line_start = true;
            self.space = false;
        }
        self.pending = 0;
        if self.out.is_empty() {
            self.line_start = true;
        }
    }

    fn start_line(&mut self) {
        if self.line_start {
            if self.depth > 0 {
                self.out.push_str(&">".repeat(self.depth));
                self.out.push(' ');
            }
            self.line_start = false;
            self.space = false;
        }
    }

    /// Text as written, spacing included.
    fn raw(&mut self, text: &str) {
        let space = self.space;
        self.flush();
        let at_start = self.line_start || self.out.is_empty();
        self.start_line();
        if space && !at_start && !self.glued && !text.starts_with(char::is_whitespace) {
            self.out.push(' ');
        }
        self.space = false;
        self.out.push_str(text);
        self.glued = text.ends_with(char::is_whitespace);
    }

    /// Text from the HTML: runs of spaces are one space.
    fn text(&mut self, text: &str) {
        for c in text.chars() {
            if self.pre > 0 {
                if c == '\n' {
                    self.line_break();
                } else {
                    self.flush();
                    self.start_line();
                    self.out.push(c);
                }
            } else if c.is_whitespace() {
                self.space = true;
            } else {
                let space = self.space;
                self.flush();
                let at_start = self.line_start || self.out.is_empty();
                self.start_line();
                if space && !at_start && !self.glued {
                    self.out.push(' ');
                }
                self.space = false;
                self.glued = false;
                self.out.push(c);
            }
        }
    }
}

impl TextWriter {
    /// A list item's marker, `- ` or the next number, indented by level.
    fn list_item(&mut self, lists: &mut [Option<u64>]) {
        self.block(1);
        let indent = "  ".repeat(lists.len().saturating_sub(1));
        let marker = match lists.last_mut() {
            Some(Some(next)) => {
                let marker = format!("{indent}{next}. ");
                *next = next.saturating_add(1);
                marker
            }
            _ => format!("{indent}- "),
        };
        self.raw(&marker);
    }

    /// After a link's text (written from `start`): its address in
    /// brackets, unless the text is the address.
    fn end_link(&mut self, href: &str, start: usize) {
        let shown = self.out.get(start..).unwrap_or_default().trim().to_owned();
        let address = href
            .strip_prefix("mailto:")
            .map_or(href, |rest| rest.split('?').next().unwrap_or(rest))
            .trim()
            .to_owned();
        if shown.is_empty() {
            self.text(&address);
        } else if bare_url(&shown) != bare_url(&address) {
            self.space = true;
            self.raw(&format!("({address})"));
        }
    }
}

/// Lines without trailing spaces, at most one blank line in a row, none at
/// either end.
fn tidy_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = false;
    for line in text.lines().map(str::trim_end) {
        if line.is_empty() {
            blank = !out.is_empty();
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank { "\n\n" } else { "\n" });
        }
        blank = false;
        out.push_str(line);
    }
    out
}

/// A link address as it would be written out, for comparing with its text.
fn bare_url(url: &str) -> String {
    let lower = url.trim().to_lowercase();
    let lower = lower.strip_prefix("mailto:").unwrap_or(&lower);
    let lower = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(lower);
    lower.trim_end_matches('/').to_owned()
}

/// A readable plain-text version of cleaned HTML: paragraphs and line
/// breaks kept, list items as `- ` and `1. `, links as `text (address)`,
/// quotes with `> `.
#[must_use]
pub fn html_to_text(html: &str) -> String {
    let mut w = TextWriter {
        line_start: true,
        ..TextWriter::default()
    };
    // Open lists: the next number of each numbered one.
    let mut lists: Vec<Option<u64>> = Vec::new();
    // Open links: the address and where their text starts.
    let mut links: Vec<(Option<String>, usize)> = Vec::new();
    let mut first_cell = true;
    for piece in pieces(html) {
        let tag = match piece {
            Piece::Text(text) => {
                w.text(&unescape(text));
                continue;
            }
            Piece::Tag(tag) => tag,
        };
        let (closing, name) = tag_name(tag);
        match (closing, name.as_str()) {
            (_, "p" | "h1" | "h2" | "h3" | "table") => w.block(2),
            (false, "pre") => {
                w.block(2);
                w.pre = w.pre.saturating_add(1);
            }
            (true, "pre") => {
                w.pre = w.pre.saturating_sub(1);
                w.block(2);
            }
            (_, "div" | "caption") => w.block(1),
            (false, "br") => w.line_break(),
            (false, "hr") => {
                w.block(1);
                w.raw("----------");
                w.block(1);
            }
            (false, "blockquote") => {
                w.block(2);
                w.depth = w.depth.saturating_add(1);
            }
            (true, "blockquote") => {
                w.depth = w.depth.saturating_sub(1);
                w.block(2);
            }
            (false, "ul" | "ol") => {
                w.block(if lists.is_empty() { 2 } else { 1 });
                let start = attribute(tag, "start")
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .unwrap_or(1);
                lists.push((name == "ol").then_some(start));
            }
            (true, "ul" | "ol") => {
                lists.pop();
                w.block(if lists.is_empty() { 2 } else { 1 });
            }
            (false, "li") => w.list_item(&mut lists),
            (false, "tr") => {
                w.block(1);
                first_cell = true;
            }
            (false, "td" | "th") => {
                if !first_cell {
                    w.raw("\t");
                }
                first_cell = false;
            }
            (false, "img") => {
                if let Some(alt) = attribute(tag, "alt").map(unescape)
                    && !alt.trim().is_empty()
                {
                    w.text(&format!(" [{}] ", alt.trim()));
                }
            }
            (false, "a") => links.push((attribute(tag, "href").map(unescape), w.out.len())),
            (true, "a") => {
                if let Some((Some(href), start)) = links.pop() {
                    w.end_link(&href, start);
                }
            }
            _ => {}
        }
    }
    tidy_text(&w.out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn cleaned(html: &str) -> String {
        clean(html).as_str().to_owned()
    }

    #[test]
    fn keeps_the_formatting_an_email_needs() {
        let html = cleaned(concat!(
            r#"<p style="text-align:center">Hi <b>Peter</b>, <strong>please</strong> <i>note</i> <em>this</em> "#,
            r##"<u>now</u> <s>not</s> <font face="Georgia" size="4" color="#c00000">red</font> "##,
            r#"<span style="color:#1f497d;background-color:yellow;font-size:18px;font-family:Calibri">blue</span></p>"#,
            r#"<h1>Title</h1><h2>Sub</h2><h3>Small</h3><hr>"#,
            r#"<ol><li>One<ul><li>Nested</li></ul></li></ol>"#,
            r#"<blockquote data-mocky-quote>Earlier</blockquote>"#,
            r#"<a href="https://mocky.co.ke/q">quote</a> <a href="mailto:info@mocky.co.ke">mail</a>"#,
            r#"<table border="1" cellpadding="4" style="border-collapse:collapse"><tr><th>Item</th><td colspan="2" style="padding:4px">Paint</td></tr></table>"#,
            r#"<img src="cid:logo@x" alt="Logo" width="120"><img src="data:image/png;base64,iVBORw0KGgo=">"#,
        ));
        for kept in [
            "text-align:center",
            "<b>Peter</b>",
            "<strong>please</strong>",
            "<i>note</i>",
            "<em>this</em>",
            "<u>now</u>",
            "<s>not</s>",
            r#"face="Georgia""#,
            r#"size="4""#,
            r##"color="#c00000""##,
            "color:#1f497d",
            "background-color:yellow",
            "font-size:18px",
            "font-family:Calibri",
            "<h1>Title</h1>",
            "<h2>Sub</h2>",
            "<h3>Small</h3>",
            "<hr>",
            "<ol><li>One<ul><li>Nested</li></ul></li></ol>",
            r#"<blockquote data-mocky-quote="">Earlier</blockquote>"#,
            r#"<a href="https://mocky.co.ke/q">quote</a>"#,
            r#"<a href="mailto:info@mocky.co.ke">mail</a>"#,
            r#"<table border="1" cellpadding="4""#,
            r#"colspan="2""#,
            "<th>Item</th>",
            r#"src="cid:logo@x""#,
            r#"alt="Logo""#,
            r#"src="data:image/png;base64,iVBORw0KGgo=""#,
        ] {
            assert!(html.contains(kept), "{kept} missing from {html}");
        }
        assert!(!html.contains("rel="), "{html}");
    }

    #[test]
    fn strips_what_could_run_load_or_ask() {
        let html = cleaned(concat!(
            r#"<script>steal()</script><p onclick="steal()" onmouseover="x()">Hi</p>"#,
            r#"<a href="javascript:alert(1)">bad</a><a href="data:text/html,x">bad2</a><a href="ftp://x.example/f">ftp</a>"#,
            r#"<img src="https://tracker.example/p.gif"><img src="http://x.example/a.png" alt="gone">"#,
            r#"<img src="data:image/svg+xml;base64,PHN2Zz4=">"#,
            r#"<iframe src="https://evil.example">inside</iframe><object data="x">obj</object>"#,
            r#"<form action="https://evil.example"><input name="pw"><button>Send</button></form>"#,
            r#"<style>body{color:red}</style><link rel="stylesheet" href="https://x.example/s.css">"#,
            r#"<div style="background-image:url(https://x.example/b.png);position:fixed;color:red;width:expression(alert(1))">styled</div>"#,
            r#"<p style="background:url(https://x.example/b.png)">bg</p><video src="x.mp4">v</video>"#,
            r#"<p class="x" id="y" title="t">attrs</p><!-- note -->"#,
        ));
        for gone in [
            "script",
            "steal",
            "onclick",
            "onmouseover",
            "javascript:",
            "data:text",
            "ftp:",
            "tracker.example",
            "x.example",
            "svg",
            "iframe",
            "inside",
            "object",
            "form",
            "input",
            "button",
            "Send",
            "<style",
            "<link",
            "url(",
            "position",
            "expression",
            "class=",
            "id=",
            "title=",
            "note",
            "<video",
        ] {
            assert!(!html.contains(gone), "{gone} left in {html}");
        }
        for kept in ["Hi", "bad", "ftp", "color:red", "styled", "attrs"] {
            assert!(html.contains(kept), "{kept} missing from {html}");
        }
        // An image left without a source is dropped entirely.
        assert!(!html.contains("<img"), "{html}");
    }

    #[test]
    fn lists_nest_eight_deep_at_most() {
        let depth = 12;
        let html = format!(
            "{}deep{}",
            "<ul><li>x".repeat(depth),
            "</li></ul>".repeat(depth)
        );
        let out = cleaned(&html);
        assert_eq!(out.matches("<ul>").count(), MAX_LIST_DEPTH);
        assert_eq!(out.matches("</ul>").count(), MAX_LIST_DEPTH);
        assert_eq!(out.matches("<li>").count(), out.matches("</li>").count());
        assert!(out.contains("deep"));
    }

    #[test]
    fn the_text_part_reads_naturally() {
        let text = clean(concat!(
            "<div>Hi Peter,</div><div><br></div><div>Please <b>confirm</b>:</div>",
            "<ul><li>Paint</li><li>Tiles<ol><li>White</li><li>Grey</li></ol></li></ul>",
            r#"<ol start="3"><li>Third</li></ol>"#,
            r#"<p>See <a href="https://mocky.co.ke/q">the quote</a>, <a href="https://mocky.co.ke">mocky.co.ke</a> "#,
            r#"or write to <a href="mailto:info@mocky.co.ke">info@mocky.co.ke</a> / <a href="mailto:don@mocky.co.ke?subject=Hi">Don</a>.</p>"#,
            "<p>Line one<br>Line two</p><hr>",
            "<table><tr><th>Item</th><th>Price</th></tr><tr><td>Paint</td><td>2,500</td></tr></table>",
            "<p>Tom &amp; Jerry &lt;3</p>",
        ))
        .text();
        let expected = "Hi Peter,\n\nPlease confirm:\n\n- Paint\n- Tiles\n  1. White\n  2. Grey\n\n3. Third\n\nSee the quote (https://mocky.co.ke/q), mocky.co.ke or write to info@mocky.co.ke / Don (don@mocky.co.ke).\n\nLine one\nLine two\n\n----------\n\nItem\tPrice\nPaint\t2,500\n\nTom & Jerry <3";
        assert_eq!(text, expected);
    }

    #[test]
    fn quotes_get_angle_marks() {
        let text = clean(
            "<p>Sure.</p><div data-mocky-quote><p>On Tue, Amina wrote:</p><blockquote><p>Lunch?</p><p>Friday</p><blockquote>Older</blockquote></blockquote></div><p>After</p>",
        )
        .text();
        assert_eq!(
            text,
            "Sure.\n\nOn Tue, Amina wrote:\n\n> Lunch?\n>\n> Friday\n>\n>> Older\n\nAfter"
        );
    }

    fn signature() -> Signature {
        Signature {
            html: "<b>Don</b><br>Mocky Digital<br><img src=\"https://mocky.co.ke/logo.png\">"
                .into(),
            text: "Don\nMocky Digital".into(),
            image: None,
        }
    }

    #[test]
    fn the_signature_goes_before_the_quoted_original() {
        let c = clean(
            "<p>See you Friday.</p><div data-mocky-quote><p>On Tue, Amina wrote:</p><blockquote>Lunch?</blockquote></div>",
        );
        let body = finish(&c, Some((&signature(), Some("s1")))).unwrap();
        let sig = body.html.find("<b>Don</b>").unwrap();
        assert!(body.html.find("See you Friday").unwrap() < sig);
        assert!(sig < body.html.find("Amina wrote").unwrap());
        assert!(body.html.contains("data-mocky-signature=\"s1\""));
        // The signature's own logo is sent as it was saved.
        assert!(body.html.contains("https://mocky.co.ke/logo.png"));
        assert!(body.html.contains(RICH_MARKER));
        assert_eq!(
            body.text,
            "See you Friday.\n\n-- \nDon\nMocky Digital\n\nOn Tue, Amina wrote:\n\n> Lunch?"
        );
    }

    #[test]
    fn a_marked_place_is_replaced_by_the_signature() {
        let c = clean(
            "<p>Hi</p><div data-mocky-signature=\"s1\"><div>old <b>copy</b></div><img src=\"https://x.example/l.png\"></div><p>PS</p>",
        );
        assert_eq!(c.signature_placeholder().as_deref(), Some("s1"));
        let body = finish(&c, Some((&signature(), Some("s1")))).unwrap();
        assert!(!body.html.contains("old"), "{}", body.html);
        assert!(body.html.find("<b>Don</b>").unwrap() < body.html.find("PS").unwrap());
        assert_eq!(body.text, "Hi\n\n-- \nDon\nMocky Digital\n\nPS");
        // Without a signature to put there, it stays as written.
        let none = finish(&c, None).unwrap();
        assert!(none.html.contains("old <b>copy</b>"));
        assert_eq!(clean("<p>Hi</p>").signature_placeholder(), None);
    }

    #[test]
    fn without_a_quote_the_signature_goes_last() {
        let body = finish(&clean("<p>Hi</p>"), Some((&signature(), None))).unwrap();
        assert!(body.html.find("Hi").unwrap() < body.html.find("<b>Don</b>").unwrap());
        assert!(!body.html.contains("data-mocky-signature"));
        assert_eq!(body.text, "Hi\n\n-- \nDon\nMocky Digital");
        let plain = finish(&clean("<p>Hi <i>there</i></p>"), None).unwrap();
        assert_eq!(plain.text, "Hi there");
        assert!(plain.html.starts_with("<!doctype html>"));
        assert!(plain.html.contains("<i>there</i>"));
    }

    #[test]
    fn pasted_pictures_travel_inside_the_message() {
        use base64::Engine as _;
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDRfake".to_vec();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        let c = clean(&format!(
            "<p>Look</p><img src=\"data:image/png;base64,{b64}\" alt=\"plan\"><img src=\"data:image/png;base64,{b64}\"><img src=\"data:image/gif;base64,bm90IGFuIGltYWdl\">"
        ));
        let body = finish(&c, None).unwrap();
        assert_eq!(body.inline_images.len(), 1);
        let image = &body.inline_images[0];
        assert_eq!(image.mime_type, "image/png");
        assert_eq!(image.data, png);
        assert_eq!(
            body.html
                .matches(&format!("src=\"cid:{}\"", image.content_id))
                .count(),
            2
        );
        assert!(!body.html.contains("data:image"));
        // Not really a picture: gone.
        assert_eq!(body.html.matches("<img").count(), 2);
        assert_eq!(body.text, "Look\n\n[plan]");

        let mut huge = png.clone();
        huge.resize(MAX_INLINE_IMAGE_BYTES + 1, 0);
        let big = clean(&format!(
            "<img src=\"data:image/png;base64,{}\">",
            base64::engine::general_purpose::STANDARD.encode(&huge)
        ));
        assert!(matches!(
            finish(&big, None),
            Err(RichError::PictureTooLarge { .. })
        ));
    }

    #[test]
    fn saved_drafts_keep_the_marks_when_shown_again() {
        let shown = crate::sanitize::sanitize_html(
            "<p>Hi</p><div class=\"signature\" data-mocky-signature=\"s1\">Don</div><div data-mocky-quote=\"\"><blockquote>Q</blockquote></div>",
            crate::sanitize::RemoteContent::Block,
        )
        .html;
        assert!(shown.contains("data-mocky-signature=\"s1\""), "{shown}");
        assert!(shown.contains("data-mocky-quote"), "{shown}");
    }
}
