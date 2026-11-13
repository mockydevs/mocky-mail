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
use serde::Serialize;

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
    let lower = url.trim_start().to_ascii_lowercase();
    lower.starts_with("http:") || lower.starts_with("https:")
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
        .add_generic_attributes(["style"])
        .filter_style_properties(ALLOWED_STYLE_PROPERTIES.iter().copied().collect())
        .rm_tag_attributes("img", ["srcset"])
        .attribute_filter(move |element, attribute, value| {
            if remote == RemoteContent::Block
                && element == "img"
                && attribute == "src"
                && is_remote(value)
            {
                counter.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            Some(Cow::Borrowed(value))
        });

    let html = builder.clean(html).to_string();
    SanitizedHtml {
        html,
        blocked_remote_images: blocked.load(Ordering::Relaxed),
    }
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
    fn keeps_ordinary_email_formatting() {
        let out = clean(
            r#"<table><tr><td style="padding: 8px"><b>Total</b></td><td>KES 4,500</td></tr></table>"#,
        );
        assert!(out.contains("<table>"));
        assert!(out.contains("<b>Total</b>"));
        assert!(out.contains("padding"));
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
