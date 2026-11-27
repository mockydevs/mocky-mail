//! Files the system asks the app to open: `.eml`, `.ics` and `.vcf` files
//! double-clicked or opened with Mocky Mail (file associations), dropped
//! on it, or shared to it on Android.
//!
//! They come on the command line when the app starts (Windows, Linux), in
//! a second launch while it runs (the single-instance plugin), as an
//! "open" event (macOS), or from Android's intent (copied into the cache by
//! `MainActivity`, handed over by `FilesPlugin`). Until the page has asked
//! for them (`take_opened_files`) they wait; after that each one is sent as
//! `mail://open-file`.
//!
//! Only files handed over this way may be read through the commands that
//! take a path (`read_message_file`, `import_message_file`,
//! `import_opened_file`): a page can't name any other file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use crate::error::{AppError, AppResult};
use crate::state::lock;

/// A file the app was asked to open (`OpenedFile` in `docs/ipc.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenedFile {
    /// `eml`, `ics` or `vcf`.
    pub kind: &'static str,
    /// Where it is.
    pub path: String,
    /// Its name, to show.
    pub name: String,
}

#[derive(Default)]
struct Opened {
    /// Waiting for the page.
    waiting: Vec<OpenedFile>,
    /// Whether the page has asked (from then on files are sent as events).
    taken: bool,
    /// Every file handed over this run, by the id its message is shown
    /// under (`file:<hash>`).
    known: HashMap<String, PathBuf>,
}

static OPENED: Mutex<Option<Opened>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Opened) -> T) -> T {
    let mut guard = lock(&OPENED);
    f(guard.get_or_insert_with(Opened::default))
}

/// What kind of file a name is, by its extension.
pub fn kind_of(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "eml" => Some("eml"),
        "ics" | "ical" | "icalendar" | "ifb" => Some("ics"),
        "vcf" | "vcard" => Some("vcf"),
        _ => None,
    }
}

/// The id an opened message is shown under: the same for the same file.
pub fn id_for(path: &Path) -> String {
    let digest = Sha256::digest(path.to_string_lossy().as_bytes());
    let hex = digest.iter().take(12).fold(String::new(), |mut out, b| {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
        out
    });
    format!("file:{hex}")
}

/// Whether a message id is that of an opened file.
pub fn is_file_id(id: &str) -> bool {
    id.starts_with("file:")
}

/// The files among command-line arguments (relative ones against `cwd`).
pub fn from_args<'a>(args: impl IntoIterator<Item = &'a str>, cwd: &Path) -> Vec<PathBuf> {
    args.into_iter()
        .filter(|a| {
            !a.starts_with('-')
                && !a.contains("://")
                && !a.to_ascii_lowercase().starts_with("mailto:")
        })
        .map(|a| {
            let path = PathBuf::from(a);
            if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            }
        })
        .filter(|p| kind_of(p).is_some() && p.is_file())
        .collect()
}

fn entry(path: &Path, name: Option<&str>, kind: &'static str) -> OpenedFile {
    let name = name
        .map(str::to_owned)
        .or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    OpenedFile {
        kind,
        path: path.display().to_string(),
        name: mail_core::message::safe_file_name(&name),
    }
}

/// Hand a file over: it waits for the page, or is sent to it.
pub fn deliver(
    app: Option<&AppHandle>,
    path: &Path,
    name: Option<&str>,
    kind: Option<&'static str>,
) {
    let Some(kind) = kind.or_else(|| kind_of(path)) else {
        return;
    };
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let file = entry(&path, name, kind);
    let send = with(|o| {
        o.known.insert(id_for(&path), path.clone());
        if o.taken {
            true
        } else {
            if !o.waiting.iter().any(|w| w.path == file.path) {
                o.waiting.push(file.clone());
            }
            false
        }
    });
    if send && let Some(app) = app {
        let _ = app.emit("mail://open-file", serde_json::json!({ "file": file }));
    }
}

/// Remember the files this process was started with.
pub fn remember_launch_files() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_default();
    for path in from_args(args.iter().map(String::as_str), &cwd) {
        deliver(None, &path, None, None);
    }
}

/// The file behind a path the page names, if it was handed over.
pub fn checked(path: &str) -> AppResult<PathBuf> {
    let wanted = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    let id = id_for(&wanted);
    with(|o| o.known.get(&id).cloned())
        .ok_or_else(|| AppError::invalid("That file wasn't opened with Mocky Mail."))
}

/// The bytes of an opened message, for an id from [`id_for`]. `None` for
/// any other id.
///
/// # Errors
/// `not_found` for an opened file that is gone.
pub fn raw_for_id(id: &str) -> AppResult<Option<Vec<u8>>> {
    if !is_file_id(id) {
        return Ok(None);
    }
    let path = with(|o| o.known.get(id).cloned())
        .ok_or_else(|| AppError::new("not_found", "That file is no longer open."))?;
    read_limited(&path, mail_core::message::MAX_MESSAGE_BYTES).map(Some)
}

/// A file read whole, refused past `max` bytes.
///
/// # Errors
/// `too_large`, or the file can't be read.
pub fn read_limited(path: &Path, max: usize) -> AppResult<Vec<u8>> {
    let size = std::fs::metadata(path)
        .map_err(|_| AppError::new("not_found", "That file is no longer there."))?
        .len();
    if size > u64::try_from(max).unwrap_or(u64::MAX) {
        return Err(AppError::new(
            "too_large",
            "That file is too large to open here.",
        ));
    }
    Ok(std::fs::read(path)?)
}

/// `take_opened_files`: the files the app was started with (or shared to
/// it before the page was ready), once each.
#[tauri::command]
pub fn take_opened_files() -> Vec<OpenedFile> {
    with(|o| {
        o.taken = true;
        std::mem::take(&mut o.waiting)
    })
}

/// Android: wait for files shared to the app, for as long as it runs.
#[cfg(target_os = "android")]
pub fn watch_shared(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            match crate::files_android::wait_shared_files().await {
                Ok(files) => {
                    for file in files {
                        let kind = match file.kind.as_str() {
                            "eml" => Some("eml"),
                            "ics" => Some("ics"),
                            "vcf" => Some("vcf"),
                            _ => None,
                        };
                        deliver(Some(&app), &file.path, Some(&file.name), kind);
                    }
                }
                Err(error) => {
                    eprintln!("shared files: {error}");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind_of(Path::new("a/Quote.EML")), Some("eml"));
        assert_eq!(kind_of(Path::new("x.ics")), Some("ics"));
        assert_eq!(kind_of(Path::new("x.vcard")), Some("vcf"));
        assert_eq!(kind_of(Path::new("x.pdf")), None);
        assert_eq!(kind_of(Path::new("noext")), None);
    }

    #[test]
    fn ids_are_stable_and_marked() {
        let a = id_for(Path::new("/tmp/a.eml"));
        assert_eq!(a, id_for(Path::new("/tmp/a.eml")));
        assert_ne!(a, id_for(Path::new("/tmp/b.eml")));
        assert!(is_file_id(&a));
        assert!(!is_file_id("0123abcd"));
    }

    #[test]
    fn only_existing_mail_files_are_taken_from_arguments() {
        let dir = std::env::temp_dir().join(format!("mocky-opened-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let eml = dir.join("m.eml");
        let _ = std::fs::write(&eml, b"Subject: x\r\n\r\n");
        let found = from_args(
            [
                "--minimized",
                "m.eml",
                "missing.eml",
                "mailto:a@b.co",
                "notes.txt",
            ],
            &dir,
        );
        assert_eq!(found, std::slice::from_ref(&eml));
        // Waiting until the page asks, then handed over once.
        deliver(None, &eml, None, None);
        let taken = take_opened_files();
        assert!(taken.iter().any(|f| f.kind == "eml" && f.name == "m.eml"));
        assert!(take_opened_files().is_empty());
        assert!(checked(&eml.display().to_string()).is_ok());
        assert!(checked("/etc/passwd").is_err());
        let id = id_for(&std::fs::canonicalize(&eml).unwrap_or(eml.clone()));
        assert!(raw_for_id(&id).ok().flatten().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
