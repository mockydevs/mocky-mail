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
        .link_rel(Some("noopener noreferrer"))
        // Links open as popups, which the app hands to the system browser;
        // the viewer itself never navigates.
        .set_tag_attribute_value("a", "target", "_blank")
        .add_generic_attributes(["style"])
        .filter_style_properties(ALLOWED_STYLE_PROPERTIES.iter().copied().collect())
        .rm_tag_attributes("img", ["srcset"])
        // Old-style cell colours, still common in email buttons and banners.
        .add_tag_attributes("table", ["bgcolor"])
        .add_tag_attributes("tr", ["bgcolor"])
        .add_tag_attributes("td", ["bgcolor"])
        .add_tag_attributes("th", ["bgcolor"])
        .attribute_filter(move |element, attribute, value| {
            if remote == RemoteContent::Block
                && element == "img"
                && attribute == "src"
                && is_remote(value)
            {
                counter.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            match attribute {
                "bgcolor" => is_plain_colour_value(value).then_some(Cow::Borrowed(value)),
                // Runs before the CSS property allow-list, which then
                // removes whatever is left of `background`.
                "style" => Some(keep_background_colour(value)),
                _ => Some(Cow::Borrowed(value)),
            }
        });

    let html = builder.clean(html).to_string();
    SanitizedHtml {
        html,
        blocked_remote_images: blocked.load(Ordering::Relaxed),
    }
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
    fn removes_forms_iframes_svg_and_style_blocks() {
        let out = clean(
            r#"<form action="https://evil.example"><input name="password"></form><iframe src="https://evil.example"></iframe><svg><script>x</script></svg><style>body{display:none}</style><p>ok</p>"#,
        );
        for banned in [
            "<form",
            "<input",
            "<iframe",
            "<svg",
            "<style",
            "evil.example",
        ] {
            assert!(!out.contains(banned), "{banned} survived: {out}");
        }
        assert!(out.contains("<p>ok</p>"));
    }

    #[test]
    fn links_open_without_referrer_or_opener() {
        let out = clean(r#"<a href="https://example.com">x</a>"#);
        assert!(out.contains(r#"rel="noopener noreferrer""#));
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
    }
}
