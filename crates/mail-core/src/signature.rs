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

/// Largest signature image accepted, in bytes.
pub const MAX_SIGNATURE_IMAGE_BYTES: usize = 1024 * 1024;

/// Display widths offered for a signature image, in CSS pixels.
pub const SIGNATURE_IMAGE_WIDTHS: &[u32] = &[160, 240, 320, 400, 480, 600];

/// Content-ID the signature image is sent under (`cid:` in the HTML).
pub const SIGNATURE_IMAGE_CID: &str = "signature-image@mocky.mail";

/// A cleaned signature.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Signature {
    /// Cleaned HTML, safe to show and to send.
    pub html: String,
    /// Plain-text version, as shown in compose.
    pub text: String,
    /// A picture made elsewhere (for example in Illustrator), shown under
    /// the text and sent inside the message so it appears without
    /// "load images".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<SignatureImage>,
}

/// A signature picture: PNG, JPEG or GIF, checked by its bytes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureImage {
    /// `image/png`, `image/jpeg` or `image/gif`.
    pub mime_type: String,
    /// The image, base64.
    pub data: String,
    /// Width shown in messages, CSS pixels.
    pub width: u32,
    /// The picture's own size in pixels.
    pub natural_width: u32,
    /// The picture's own size in pixels.
    pub natural_height: u32,
}

impl SignatureImage {
    /// The picture's bytes, or `None` if the stored data is damaged.
    #[must_use]
    pub fn bytes(&self) -> Option<Vec<u8>> {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(&self.data)
            .ok()
    }
}

/// Why a signature image was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignatureImageError {
    /// Not PNG, JPEG or GIF (SVG and WebP don't show in many mail apps).
    #[error(
        "Use a PNG, JPG or GIF picture. From Illustrator, use File > Export > Export As > PNG."
    )]
    NotAnImage,
    /// Over [`MAX_SIGNATURE_IMAGE_BYTES`].
    #[error(
        "This picture is {size_kb} KB; the limit is 1 MB. Export it smaller (about 600 pixels wide is plenty)."
    )]
    TooLarge {
        /// Size in KB.
        size_kb: usize,
    },
}

/// Check a picture chosen for a signature and work out how wide to show it.
///
/// # Errors
/// [`SignatureImageError`] when it isn't a PNG, JPEG or GIF, or is too big.
pub fn signature_image(bytes: &[u8]) -> Result<SignatureImage, SignatureImageError> {
    use base64::Engine as _;
    if bytes.len() > MAX_SIGNATURE_IMAGE_BYTES {
        return Err(SignatureImageError::TooLarge {
            size_kb: bytes.len().div_ceil(1024),
        });
    }
    let (mime_type, (natural_width, natural_height)) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ("image/png", png_size(bytes))
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        ("image/jpeg", jpeg_size(bytes))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        ("image/gif", gif_size(bytes))
    } else {
        return Err(SignatureImageError::NotAnImage);
    };
    if natural_width == 0 || natural_height == 0 {
        return Err(SignatureImageError::NotAnImage);
    }
    Ok(SignatureImage {
        mime_type: mime_type.to_owned(),
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
        width: default_width(natural_width),
        natural_width,
        natural_height,
    })
}

/// Pictures exported at 2x for sharp screens are shown at half size; others
/// at their own size, never wider than 480 px by default.
fn default_width(natural: u32) -> u32 {
    let width = if natural > 960 {
        natural.div_ceil(2)
    } else {
        natural
    };
    width.clamp(1, 480)
}

/// The nearest offered width, never wider than the picture itself.
#[must_use]
pub fn allowed_width(requested: u32, natural: u32) -> u32 {
    requested.clamp(40, 600).min(natural.max(1))
}

/// Two bytes at `at`, big-endian; 0 when out of range.
fn be16(b: &[u8], at: usize) -> u32 {
    b.get(at..)
        .and_then(|rest| rest.get(..2))
        .and_then(|two| <[u8; 2]>::try_from(two).ok())
        .map_or(0, |[hi, lo]| u32::from_be_bytes([0, 0, hi, lo]))
}

/// Two bytes at `at`, little-endian; 0 when out of range.
fn le16(b: &[u8], at: usize) -> u32 {
    b.get(at..)
        .and_then(|rest| rest.get(..2))
        .and_then(|two| <[u8; 2]>::try_from(two).ok())
        .map_or(0, |[lo, hi]| u32::from_le_bytes([lo, hi, 0, 0]))
}

fn be32(b: &[u8], at: usize) -> u32 {
    b.get(at..)
        .and_then(|rest| rest.get(..4))
        .and_then(|four| <[u8; 4]>::try_from(four).ok())
        .map_or(0, u32::from_be_bytes)
}

fn png_size(b: &[u8]) -> (u32, u32) {
    // IHDR is always first: width and height at bytes 16 and 20.
    if b.get(12..16) != Some(b"IHDR") {
        return (0, 0);
    }
    (be32(b, 16), be32(b, 20))
}

fn gif_size(b: &[u8]) -> (u32, u32) {
    (le16(b, 6), le16(b, 8))
}

fn jpeg_size(b: &[u8]) -> (u32, u32) {
    // Walk the segments to the first start-of-frame marker.
    let mut at: usize = 2;
    while let Some(&[0xff, marker]) = b.get(at..).and_then(|rest| rest.get(..2)) {
        if marker == 0xff {
            at = at.saturating_add(1);
            continue;
        }
        let is_frame = matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
        if is_frame {
            return (be16(b, at.saturating_add(7)), be16(b, at.saturating_add(5)));
        }
        let length = usize::try_from(be16(b, at.saturating_add(2))).unwrap_or(0);
        if length < 2 {
            return (0, 0);
        }
        at = at.saturating_add(2).saturating_add(length);
    }
    (0, 0)
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
    Ok(Some(Signature {
        html,
        text,
        image: None,
    }))
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
        "<div class=\"signature\" style=\"margin-top:16px\">{}",
        signature.html
    );
    if let Some(image) = &signature.image {
        let mut alt = String::new();
        escape(&signature.text.replace('\n', " "), &mut alt);
        let _ = write!(
            html,
            "<img src=\"cid:{SIGNATURE_IMAGE_CID}\" width=\"{}\" alt=\"{alt}\" style=\"display:block;border:0;margin-top:8px;max-width:100%;height:auto\">",
            image.width
        );
    }
    html.push_str("</div>");
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

    /// A 2x1 PNG header, enough to read its size.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend_from_slice(&width.to_be_bytes());
        b.extend_from_slice(&height.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0]);
        b
    }

    #[test]
    fn pictures_from_design_tools_are_accepted_and_sized() {
        let img = signature_image(&png(1200, 300)).unwrap();
        assert_eq!(img.mime_type, "image/png");
        assert_eq!((img.natural_width, img.natural_height), (1200, 300));
        // A 2x export shows at half size.
        assert_eq!(img.width, 480);
        assert_eq!(signature_image(&png(300, 80)).unwrap().width, 300);

        let gif = b"GIF89a\x40\x01\x50\x00rest";
        let g = signature_image(gif).unwrap();
        assert_eq!((g.natural_width, g.natural_height), (320, 80));

        // JPEG: SOI, an APP0 segment, then SOF0 with height 90 and width 400.
        let jpeg = [
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00,
            0x5a, 0x01, 0x90, 0x03, 0, 0, 0, 0,
        ];
        let j = signature_image(&jpeg).unwrap();
        assert_eq!(
            (j.mime_type.as_str(), j.natural_width, j.natural_height),
            ("image/jpeg", 400, 90)
        );
    }

    #[test]
    fn other_files_are_refused() {
        for bad in [
            b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec(),
            b"RIFF\0\0\0\0WEBPVP8 ".to_vec(),
            b"%PDF-1.7".to_vec(),
            b"MZ\x90\0".to_vec(),
        ] {
            assert_eq!(signature_image(&bad), Err(SignatureImageError::NotAnImage));
        }
        let mut huge = png(100, 100);
        huge.resize(MAX_SIGNATURE_IMAGE_BYTES + 1, 0);
        assert!(matches!(
            signature_image(&huge),
            Err(SignatureImageError::TooLarge { .. })
        ));
    }

    #[test]
    fn the_picture_goes_in_the_html_by_content_id() {
        let mut s = clean_signature("<b>Don</b>").unwrap().unwrap();
        s.image = Some(signature_image(&png(300, 80)).unwrap());
        let html = html_with_signature(&format!("Hi\n\n{}", signature_block(&s.text)), &s).unwrap();
        assert!(html.contains(&format!(
            "src=\"cid:{SIGNATURE_IMAGE_CID}\" width=\"300\" alt=\"Don\""
        )));
        assert!(!html.contains("base64"));
    }

    #[test]
    fn widths_stay_within_the_picture() {
        assert_eq!(allowed_width(480, 300), 300);
        assert_eq!(allowed_width(5000, 2000), 600);
        assert_eq!(allowed_width(240, 1200), 240);
    }
}
