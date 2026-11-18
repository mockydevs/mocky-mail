//! Which received attachments the app may show inside its own preview.
//!
//! The preview handles three kinds: photos (PNG, JPEG, GIF, WebP), PDFs and
//! plain text. A file is offered for preview by its name and stated type,
//! then its first bytes must agree before anything is shown, so a program
//! renamed `photo.jpg` is never handed to the viewer. HTML and SVG are never
//! previewed: both can carry scripts. Everything else is saved or opened in
//! another app instead.

use serde::{Deserialize, Serialize};

/// Largest attachment the preview will load, in bytes. Bigger files are
/// opened in another app or saved instead.
pub const MAX_PREVIEW_BYTES: u64 = 30 * 1024 * 1024;

/// How the preview shows a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreviewKind {
    /// PNG, JPEG, GIF or WebP, shown as a picture.
    Image,
    /// Shown page by page.
    Pdf,
    /// Plain text, shown as text (never as HTML).
    Text,
}

const IMAGE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/jpg",
    "image/pjpeg",
    "image/gif",
    "image/webp",
];
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "jfif", "gif", "webp"];
const TEXT_TYPES: &[&str] = &["text/plain", "text/csv"];
const TEXT_EXTENSIONS: &[&str] = &["txt", "text", "csv", "log", "md"];

fn extension(file_name: &str) -> Option<String> {
    let name = file_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(file_name)
        .trim_end_matches(['.', ' '])
        .to_ascii_lowercase();
    name.rsplit_once('.').map(|(_, ext)| ext.to_owned())
}

/// The kind of preview offered for an attachment, judged from its name and
/// stated type only. `None`: no preview; open or save it instead.
#[must_use]
pub fn preview_kind(file_name: &str, mime_type: &str, size: u64) -> Option<PreviewKind> {
    if size == 0 || size > MAX_PREVIEW_BYTES {
        return None;
    }
    let mime = mime_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let ext = extension(file_name).unwrap_or_default();
    // A script-capable name wins over whatever type the sender claimed.
    if matches!(
        ext.as_str(),
        "svg" | "svgz" | "htm" | "html" | "xhtml" | "xml" | "mht" | "mhtml"
    ) {
        return None;
    }
    if IMAGE_TYPES.contains(&mime.as_str()) || IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        return Some(PreviewKind::Image);
    }
    if mime == "application/pdf" || ext == "pdf" {
        return Some(PreviewKind::Pdf);
    }
    if TEXT_TYPES.contains(&mime.as_str()) || TEXT_EXTENSIONS.contains(&ext.as_str()) {
        return Some(PreviewKind::Text);
    }
    None
}

/// Whether the file's own bytes are of `kind`. Checked before a preview is
/// shown, whatever the name and stated type said.
#[must_use]
pub fn content_is(kind: PreviewKind, content: &[u8]) -> bool {
    if content.is_empty() || content.len() as u64 > MAX_PREVIEW_BYTES {
        return false;
    }
    match kind {
        PreviewKind::Image => image_magic(content),
        // Readers accept a %PDF- header within the first 1024 bytes.
        PreviewKind::Pdf => content
            .get(..content.len().min(1024))
            .unwrap_or(content)
            .windows(5)
            .any(|w| w == b"%PDF-"),
        PreviewKind::Text => {
            let head = content.get(..content.len().min(8192)).unwrap_or(content);
            !head.contains(&0) && !crate::attachment_policy::is_executable_content(content)
        }
    }
}

fn image_magic(content: &[u8]) -> bool {
    content.starts_with(b"\x89PNG\r\n\x1a\n")
        || content.starts_with(b"\xff\xd8\xff")
        || content.starts_with(b"GIF87a")
        || content.starts_with(b"GIF89a")
        || (content.starts_with(b"RIFF") && content.get(8..12) == Some(b"WEBP"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    #[test]
    fn photos_pdfs_and_text_are_offered() {
        assert_eq!(
            preview_kind("IMG_0042.JPG", "image/jpeg", 10),
            Some(PreviewKind::Image)
        );
        assert_eq!(
            preview_kind("scan", "image/png", 10),
            Some(PreviewKind::Image)
        );
        assert_eq!(
            preview_kind("photo.webp", "application/octet-stream", 10),
            Some(PreviewKind::Image)
        );
        assert_eq!(
            preview_kind("INV-0144.pdf", "application/octet-stream", 10),
            Some(PreviewKind::Pdf)
        );
        assert_eq!(
            preview_kind("invoice", "application/pdf; name=x", 10),
            Some(PreviewKind::Pdf)
        );
        assert_eq!(
            preview_kind("notes.txt", "text/plain", 10),
            Some(PreviewKind::Text)
        );
        assert_eq!(
            preview_kind("export.csv", "text/csv", 10),
            Some(PreviewKind::Text)
        );
    }

    #[test]
    fn script_capable_and_other_files_are_not() {
        for (name, mime) in [
            ("logo.svg", "image/svg+xml"),
            ("logo.svg", "image/png"),
            ("page.html", "text/plain"),
            ("page.htm", "image/jpeg"),
            (
                "letter.docx",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            ),
            ("archive.zip", "application/zip"),
            ("noname", "application/octet-stream"),
            ("vector", "image/svg+xml"),
            ("page", "text/html"),
        ] {
            assert_eq!(preview_kind(name, mime, 10), None, "{name} {mime}");
        }
    }

    #[test]
    fn empty_and_huge_files_are_not_previewed() {
        assert_eq!(preview_kind("a.png", "image/png", 0), None);
        assert_eq!(
            preview_kind("a.png", "image/png", MAX_PREVIEW_BYTES + 1),
            None
        );
        assert_eq!(
            preview_kind("a.png", "image/png", MAX_PREVIEW_BYTES),
            Some(PreviewKind::Image)
        );
    }

    #[test]
    fn content_must_match_the_kind() {
        assert!(content_is(PreviewKind::Image, PNG));
        assert!(content_is(PreviewKind::Image, b"\xff\xd8\xff\xe0JFIF"));
        assert!(content_is(PreviewKind::Image, b"GIF89a...."));
        assert!(content_is(PreviewKind::Image, b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(!content_is(
            PreviewKind::Image,
            b"<svg xmlns='http://www.w3.org/2000/svg'>"
        ));
        assert!(!content_is(PreviewKind::Image, b"MZ\x90\0renamed.exe"));
        assert!(!content_is(PreviewKind::Image, b"RIFF\0\0\0\0WAVEfmt "));

        assert!(content_is(PreviewKind::Pdf, b"%PDF-1.7\n"));
        assert!(content_is(PreviewKind::Pdf, b"\r\n%PDF-1.4"));
        assert!(!content_is(PreviewKind::Pdf, PNG));

        assert!(content_is(
            PreviewKind::Text,
            "Habari, invoice attached.\n".as_bytes()
        ));
        assert!(!content_is(PreviewKind::Text, PNG));
        assert!(!content_is(PreviewKind::Text, b"#!/bin/sh\nrm -rf ~"));
        assert!(!content_is(PreviewKind::Text, b""));
    }
}
