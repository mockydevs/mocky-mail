//! Which files Mocky Mail will attach to outgoing mail.
//!
//! A mail client has to send ordinary files of any kind, so this is a
//! blocklist of program, script and installer types: the same families Gmail
//! and most providers refuse. A file is refused when
//!
//! - its extension is blocked (every dotted suffix counts, so
//!   `report.pdf.exe` is caught, and trailing dots or spaces are ignored);
//! - its content is a program or script whatever it is called (ELF, Windows
//!   PE, Mach-O, a `#!` script, a `.desktop` launcher); or
//! - it is a `.zip` holding such a file by name, one level deep, as Gmail does.
//!
//! The check runs when a file is attached and again when the message is
//! built, so forwarded attachments and restored drafts are covered too.
//!
//! Adapted from Overlook's `attachment_policy.rs` (MIT License, Copyright (c)
//! 2026 Last Refuge, <https://github.com/last-refuge/overlook>).

use std::fmt;

/// Gmail's published blocked file types, plus Linux and macOS executables
/// and packages.
pub const BLOCKED_EXTENSIONS: &[&str] = &[
    "ade",
    "adp",
    "apk",
    "appx",
    "appxbundle",
    "bat",
    "cab",
    "chm",
    "cmd",
    "com",
    "cpl",
    "diagcab",
    "diagcfg",
    "diagpkg",
    "dll",
    "dmg",
    "ex",
    "ex_",
    "exe",
    "hta",
    "img",
    "ins",
    "iso",
    "isp",
    "jar",
    "jnlp",
    "js",
    "jse",
    "lib",
    "lnk",
    "mde",
    "mjs",
    "msc",
    "msi",
    "msix",
    "msixbundle",
    "msp",
    "mst",
    "nsh",
    "pif",
    "ps1",
    "scr",
    "sct",
    "shb",
    "sys",
    "vb",
    "vbe",
    "vbs",
    "vhd",
    "vxd",
    "wsc",
    "wsf",
    "wsh",
    "xll", // Linux and macOS executables, launchers and packages.
    "sh",
    "bash",
    "zsh",
    "csh",
    "ksh",
    "run",
    "elf",
    "desktop",
    "appimage",
    "deb",
    "rpm",
    "flatpak",
    "flatpakref",
    "snap",
    "pkg",
    "command",
    "app",
];

/// Why a file can't be attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The file's own extension is blocked.
    BlockedType {
        /// The extension, lowercased.
        extension: String,
    },
    /// The content is a program or script, whatever the name says.
    ExecutableContent,
    /// A zip holds a blocked entry.
    BlockedInArchive {
        /// The entry's name inside the archive.
        entry: String,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BlockedType { extension } => write!(
                f,
                "Mocky Mail doesn't send .{extension} files, because they can run programs and most mail services block them."
            ),
            Self::ExecutableContent => write!(
                f,
                "Mocky Mail doesn't send programs or scripts, because most mail services block them."
            ),
            Self::BlockedInArchive { entry } => write!(
                f,
                "Mocky Mail doesn't send archives containing programs or scripts (\"{entry}\"), because most mail services block them."
            ),
        }
    }
}

/// Check a file about to be attached or sent.
///
/// # Errors
/// The [`Refusal`] explaining why it can't be sent.
pub fn check_outgoing(file_name: &str, content: &[u8]) -> Result<(), Refusal> {
    if let Some(extension) = blocked_extension(file_name) {
        return Err(Refusal::BlockedType { extension });
    }
    if is_executable_content(content) {
        return Err(Refusal::ExecutableContent);
    }
    if let Some(entry) = zip_entries(content)
        .into_iter()
        .find(|entry| blocked_extension(entry).is_some())
    {
        return Err(Refusal::BlockedInArchive { entry });
    }
    Ok(())
}

/// The blocked extension of a name, lowercased, if any.
#[must_use]
pub fn blocked_extension(file_name: &str) -> Option<String> {
    let name = file_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(file_name)
        .trim_end_matches(['.', ' '])
        .to_ascii_lowercase();
    let (_, extension) = name.rsplit_once('.')?;
    BLOCKED_EXTENSIONS
        .contains(&extension)
        .then(|| extension.to_owned())
}

/// Whether the first bytes are those of a program or script.
#[must_use]
pub fn is_executable_content(content: &[u8]) -> bool {
    const MAGICS: &[&[u8]] = &[
        b"\x7fELF",          // Linux/BSD executables and libraries
        b"MZ",               // Windows PE (exe, dll, scr, sys)
        b"\xfe\xed\xfa\xce", // Mach-O 32-bit
        b"\xfe\xed\xfa\xcf", // Mach-O 64-bit
        b"\xce\xfa\xed\xfe", // Mach-O 32-bit, little-endian
        b"\xcf\xfa\xed\xfe", // Mach-O 64-bit, little-endian
        b"\xca\xfe\xba\xbe", // Mach-O universal binary
        b"#!",               // interpreter scripts
    ];
    if MAGICS.iter().any(|magic| content.starts_with(magic)) {
        return true;
    }
    let head = content.get(..content.len().min(512)).unwrap_or(content);
    String::from_utf8_lossy(head)
        .trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with("[Desktop Entry]")
}

fn le16(bytes: &[u8], at: usize) -> Option<usize> {
    let end = at.checked_add(2)?;
    let b = bytes.get(at..end)?;
    Some(usize::from(u16::from_le_bytes([*b.first()?, *b.get(1)?])))
}

fn le32(bytes: &[u8], at: usize) -> Option<usize> {
    let end = at.checked_add(4)?;
    let b: [u8; 4] = bytes.get(at..end)?.try_into().ok()?;
    usize::try_from(u32::from_le_bytes(b)).ok()
}

/// File names in a zip's central directory; nothing is decompressed. Not a
/// zip, or a damaged one, gives no names (the other checks still apply).
fn zip_entries(content: &[u8]) -> Vec<String> {
    const END: &[u8] = b"PK\x05\x06";
    const ENTRY: &[u8] = b"PK\x01\x02";
    if !(content.starts_with(b"PK\x03\x04") || content.starts_with(END)) {
        return Vec::new();
    }
    // The end record sits in the last 22 bytes plus any comment (up to 64 KiB).
    let search_from = content
        .len()
        .saturating_sub(22_usize.saturating_add(65_535));
    let last = content.len().saturating_sub(21);
    let Some(end) = (search_from..last)
        .rev()
        .find(|&i| content.get(i..).is_some_and(|rest| rest.starts_with(END)))
    else {
        return Vec::new();
    };
    let count = end.checked_add(10).and_then(|at| le16(content, at));
    let start = end.checked_add(16).and_then(|at| le32(content, at));
    let (Some(count), Some(mut at)) = (count, start) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for _ in 0..count {
        if !content
            .get(at..)
            .is_some_and(|rest| rest.starts_with(ENTRY))
        {
            break;
        }
        let field = |offset: usize| at.checked_add(offset).and_then(|p| le16(content, p));
        let (Some(name_len), Some(extra_len), Some(comment_len)) =
            (field(28), field(30), field(32))
        else {
            break;
        };
        let Some(name_start) = at.checked_add(46) else {
            break;
        };
        let Some(name_end) = name_start.checked_add(name_len) else {
            break;
        };
        let Some(name) = content.get(name_start..name_end) else {
            break;
        };
        names.push(String::from_utf8_lossy(name).into_owned());
        let Some(next) = name_end
            .checked_add(extra_len)
            .and_then(|n| n.checked_add(comment_len))
        else {
            break;
        };
        at = next;
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal stored zip with the given entry names (no file data needed:
    /// only the central directory is read).
    fn zip_with(names: &[&str]) -> Vec<u8> {
        let mut out = b"PK\x03\x04".to_vec();
        out.extend_from_slice(&[0; 26]);
        let central_start = u32::try_from(out.len()).unwrap_or_default();
        for name in names {
            out.extend_from_slice(b"PK\x01\x02");
            out.extend_from_slice(&[0; 24]);
            out.extend_from_slice(&u16::try_from(name.len()).unwrap_or_default().to_le_bytes());
            out.extend_from_slice(&[0; 2]); // extra length
            out.extend_from_slice(&[0; 2]); // comment length
            out.extend_from_slice(&[0; 12]);
            out.extend_from_slice(name.as_bytes());
        }
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(&u16::try_from(names.len()).unwrap_or_default().to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&central_start.to_le_bytes());
        out.extend_from_slice(&[0; 2]);
        out
    }

    #[test]
    fn ordinary_files_are_fine() {
        assert_eq!(check_outgoing("Invoice INV-0144.pdf", b"%PDF-1.7"), Ok(()));
        assert_eq!(check_outgoing("photo.JPG", b"\xff\xd8\xff"), Ok(()));
        assert_eq!(check_outgoing("notes", b"hello"), Ok(()));
        assert_eq!(
            check_outgoing("photos.zip", &zip_with(&["a.jpg", "b.png"])),
            Ok(())
        );
    }

    #[test]
    fn programs_are_refused_by_name_even_when_disguised() {
        for name in [
            "setup.exe",
            "SETUP.EXE",
            "report.pdf.exe",
            "run.sh",
            "virus.exe. ",
            "app.apk",
        ] {
            assert!(
                matches!(
                    check_outgoing(name, b"data"),
                    Err(Refusal::BlockedType { .. })
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn programs_are_refused_by_content_whatever_the_name() {
        assert_eq!(
            check_outgoing("cat.jpg", b"MZ\x90\x00"),
            Err(Refusal::ExecutableContent)
        );
        assert_eq!(
            check_outgoing("notes.txt", b"#!/bin/sh\nrm -rf ~"),
            Err(Refusal::ExecutableContent)
        );
        assert_eq!(
            check_outgoing("doc.pdf", b"\x7fELF\x02"),
            Err(Refusal::ExecutableContent)
        );
        assert_eq!(
            check_outgoing("open-me.txt", b"\xef\xbb\xbf[Desktop Entry]\nExec=x"),
            Err(Refusal::ExecutableContent)
        );
    }

    #[test]
    fn zips_holding_programs_are_refused() {
        assert_eq!(
            check_outgoing(
                "invoices.zip",
                &zip_with(&["invoice.pdf", "invoice.pdf.exe"])
            ),
            Err(Refusal::BlockedInArchive {
                entry: "invoice.pdf.exe".into()
            })
        );
    }

    #[test]
    fn damaged_zips_do_not_crash() {
        let mut zip = zip_with(&["a.exe"]);
        zip.truncate(zip.len().saturating_sub(5));
        let _ = check_outgoing("broken.zip", &zip);
        let _ = check_outgoing("tiny.zip", b"PK\x05\x06");
    }

    #[test]
    fn refusals_explain_themselves() {
        let message = Refusal::BlockedType {
            extension: "exe".into(),
        }
        .to_string();
        assert!(message.contains(".exe"), "{message}");
    }
}
