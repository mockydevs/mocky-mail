//! The `<style>` blocks of an HTML email, cleaned.
//!
//! Most designed email (receipts, newsletters, notifications) puts its
//! phone layout in `@media` rules and much of its look in class rules, the
//! way Gmail and Outlook expect. Dropping those blocks leaves the email
//! with only its desktop layout, squeezed onto a phone.
//!
//! The HTML sanitizer can only filter `style` attributes, so stylesheets are
//! cleaned here with the same property allow-list: plain rules and `@media`
//! rules are kept; every other at-rule (`@import`, `@font-face`...) is
//! dropped, and so is any rule or value that could load a URL, run code or
//! hide a trick behind an escape. What is left can only change how the
//! email's own content looks.

use super::{ALLOWED_STYLE_PROPERTIES, background_colour};

/// The text of every `<style>` element in `html`. Read the way a browser
/// reads it: the content runs to the next `</style`, whatever it contains.
pub(super) fn style_blocks(html: &str) -> Vec<&str> {
    let lower = html.to_ascii_lowercase();
    let mut blocks = Vec::new();
    let mut from = 0;
    while let Some(open) = lower.get(from..).and_then(|rest| rest.find("<style")) {
        let start_tag = from.saturating_add(open);
        // `<styles>` or `<style-x>` is another element.
        let after_name = start_tag.saturating_add("<style".len());
        if !lower
            .as_bytes()
            .get(after_name)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/')
        {
            from = after_name;
            continue;
        }
        let Some(content) = lower
            .get(after_name..)
            .and_then(|rest| rest.find('>'))
            .map(|end| after_name.saturating_add(end).saturating_add(1))
        else {
            break;
        };
        let end = lower
            .get(content..)
            .and_then(|rest| rest.find("</style"))
            .map_or(lower.len(), |end| content.saturating_add(end));
        if let Some(css) = html.get(content..end) {
            blocks.push(css);
        }
        from = end;
    }
    blocks
}

/// Text that has no business in a stylesheet an email may use: loading
/// something, running something, or an escape that could spell either.
fn is_unsafe(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains(['<', '\\', '@'])
        || [
            "url(",
            "image(",
            "image-set(",
            "expression",
            "javascript:",
            "behavior",
            "-moz-binding",
            "attr(",
            "var(",
        ]
        .iter()
        .any(|bad| lower.contains(bad))
}

/// Remove `/* comments */` and the `<!-- -->` markers old email wraps its
/// style blocks in.
fn without_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(rest.get(..start).unwrap_or_default());
        rest = rest
            .get(start..)
            .and_then(|tail| tail.find("*/").map(|end| tail.get(end.saturating_add(2)..)))
            .flatten()
            .unwrap_or_default();
    }
    out.push_str(rest);
    out.replace("<!--", " ").replace("-->", " ")
}

/// The index just past the `}` closing the block that starts after `open`
/// (the index of its `{`), counting nested blocks.
fn block_end(css: &str, open: usize) -> usize {
    let mut depth = 0usize;
    for (i, c) in css.char_indices().skip_while(|(i, _)| *i < open) {
        match c {
            '{' => depth = depth.saturating_add(1),
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i.saturating_add(1);
                }
            }
            _ => {}
        }
    }
    css.len()
}

/// One declaration kept as `property:value`, or nothing.
fn clean_declaration(declaration: &str) -> Option<String> {
    let (property, value) = declaration.split_once(':')?;
    let property = property.trim().to_ascii_lowercase();
    let value = value.trim();
    if value.is_empty() || is_unsafe(value) || value.contains(['{', '}']) {
        return None;
    }
    if property == "background" {
        // Only its colour: anything else could load a picture.
        return background_colour(value).map(|colour| format!("background-color:{colour}"));
    }
    ALLOWED_STYLE_PROPERTIES
        .contains(&property.as_str())
        .then(|| format!("{property}:{value}"))
}

/// A selector list an email may use: no way out of the stylesheet.
fn is_plain_selector(selector: &str) -> bool {
    let selector = selector.trim();
    !selector.is_empty() && !is_unsafe(selector) && !selector.contains(['{', '}', ';'])
}

/// A media query made of plain words, numbers and the usual punctuation.
fn is_plain_media_query(query: &str) -> bool {
    !query.trim().is_empty()
        && query
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " ():,.-/".contains(c))
}

/// Clean a run of rules (a whole stylesheet, or the inside of `@media`).
fn clean_rules(css: &str, nested: bool) -> String {
    let mut out = String::new();
    let mut at = 0;
    while at < css.len() {
        let rest = css.get(at..).unwrap_or_default();
        // The rule's prelude runs to its block, or to `;` for `@import x;`.
        let Some(stop) = rest.find(['{', ';', '}']) else {
            break;
        };
        let prelude = rest.get(..stop).unwrap_or_default().trim();
        let stop = at.saturating_add(stop);
        if css.as_bytes().get(stop) != Some(&b'{') {
            // A statement at-rule, or a stray character: skip it.
            at = stop.saturating_add(1);
            continue;
        }
        let end = block_end(css, stop);
        let inner = css
            .get(stop.saturating_add(1)..end.saturating_sub(1))
            .unwrap_or_default();
        at = end;
        if let Some(query) = prelude
            .strip_prefix('@')
            .and_then(|rule| {
                rule.get(..5)
                    .filter(|name| name.eq_ignore_ascii_case("media"))
                    .map(|_| rule)
            })
            .and_then(|rule| rule.get(5..))
        {
            // Responsive layout lives here. One level deep is all email needs.
            if nested || !is_plain_media_query(query) {
                continue;
            }
            let rules = clean_rules(inner, true);
            if !rules.is_empty() {
                out.push_str("@media ");
                out.push_str(query.trim());
                out.push('{');
                out.push_str(&rules);
                out.push('}');
            }
            continue;
        }
        if prelude.starts_with('@') || !is_plain_selector(prelude) || inner.contains('{') {
            continue;
        }
        let declarations: Vec<String> = inner.split(';').filter_map(clean_declaration).collect();
        if !declarations.is_empty() {
            out.push_str(prelude);
            out.push('{');
            out.push_str(&declarations.join(";"));
            out.push('}');
        }
    }
    out
}

/// Clean an email's stylesheet: only what changes how its content looks.
pub(super) fn clean_stylesheet(css: &str) -> String {
    clean_rules(&without_comments(css), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_every_style_block() {
        let html = "<html><head><STYLE type=\"text/css\">a{color:red}</STYLE></head>\
                    <body><styles>no</styles><style>p{margin:0}</style><p>x</p></body></html>";
        assert_eq!(style_blocks(html), ["a{color:red}", "p{margin:0}"]);
        // An unclosed block runs to the end, as in a browser.
        assert_eq!(style_blocks("<style>b{color:red}"), ["b{color:red}"]);
        assert!(style_blocks("<p>no styles</p>").is_empty());
    }

    #[test]
    fn keeps_layout_and_responsive_rules() {
        let css = "<!-- body{margin:0;padding:0} .wrap{width:100%;max-width:600px} \
                   /* phone */ @media only screen and (max-width: 600px) { .col{display:block !important;width:100% !important} } -->";
        assert_eq!(
            clean_stylesheet(css),
            "body{margin:0;padding:0}.wrap{width:100%;max-width:600px}\
             @media only screen and (max-width: 600px){.col{display:block !important;width:100% !important}}"
        );
    }

    #[test]
    fn drops_anything_that_loads_or_runs() {
        let css = "@import url(https://evil.example/x.css); \
                   @font-face{font-family:x;src:url(https://evil.example/f.woff)} \
                   .a{background:url(https://evil.example/p.gif);color:red} \
                   .b{background-image:url(https://evil.example/p.gif)} \
                   .c{width:expression(alert(1))} \
                   .d{color:r\\65 d} \
                   .e{-moz-binding:url(x)} \
                   .f{position:fixed;top:0} \
                   .g</style><script>{color:red}";
        let out = clean_stylesheet(css);
        assert_eq!(out, ".a{color:red}");
    }

    #[test]
    fn background_keeps_only_its_colour() {
        assert_eq!(
            clean_stylesheet(".btn{background:#FF5400 none;color:#fff}"),
            ".btn{background-color:#ff5400;color:#fff}"
        );
    }

    #[test]
    fn media_rules_must_be_plain_and_not_nested() {
        assert_eq!(
            clean_stylesheet("@media (max-width:600px){@media print{p{color:red}}}"),
            ""
        );
        assert_eq!(clean_stylesheet("@media x<y{p{color:red}}"), "");
        assert_eq!(
            clean_stylesheet("@supports (display:grid){p{color:red}}"),
            ""
        );
    }
}
