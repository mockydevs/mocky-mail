//! Importing and exporting files: mail (`.eml`, `.mbox`), calendars
//! (`.ics`) and contacts (`.vcf`). Outlook `.pst` files are in `pst`.
//!
//! Mail goes to the server: every message is uploaded (IMAP APPEND, with
//! its read state and date) and arrives here with the next sync, so an
//! import needs a connection. A message whose Message-ID is already in the
//! folder is left out, so importing the same file twice brings nothing
//! twice. Files are read a message at a time, never whole.
//!
//! File pickers and save dialogs are the system's: on desktop the dialog
//! plugin's, on Android `files_android` (which copies a picked file into
//! the cache, and copies a saved one out of it).

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mail_core::mbox::{
    FileFlags, MboxItem, MboxReader, date_and_id, envelope_sender, flags_of, imap_date, is_mbox,
    to_crlf, write_message,
};
use mail_core::message::{MAX_MESSAGE_BYTES, parse_summary, safe_file_name};
use mail_core::{ical, ics_file, vcf};
use mail_store::{Account, Book, Folder, message_id_key, person_key};
use mail_sync::imap::{ImapConnection, uid_set};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::convert::to_sync;
use crate::error::{AppError, AppResult};
use crate::state::{AppState, now};
use crate::{credentials, events, opened, settings, sync};

/// Most problems listed in a report.
const MAX_PROBLEMS: usize = 50;
/// Largest `.ics` or `.vcf` file read.
const MAX_TEXT_FILE: usize = 64 * 1024 * 1024;
/// Largest file picked on Android, where it is copied into the cache first.
const MAX_PICKED: u64 = 64 * 1024 * 1024 * 1024;

/// How many of each kind a `.pst` file brought in.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Counts {
    /// Messages.
    pub messages: u32,
    /// Contacts.
    pub contacts: u32,
    /// Calendar items.
    pub events: u32,
    /// Tasks.
    pub tasks: u32,
    /// Notes.
    pub notes: u32,
}

/// `ImportReport` in `docs/ipc.md`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    /// Items brought in.
    pub imported: u32,
    /// Items left out.
    pub skipped: u32,
    /// Why, in words (at most 50).
    pub problems: Vec<String>,
    /// For a `.pst` file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts: Option<Counts>,
    /// The folder imported mail went to, when one was made for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
}

impl ImportReport {
    /// Note a problem (the first 50 are kept).
    pub fn problem(&mut self, text: impl Into<String>) {
        if self.problems.len() < MAX_PROBLEMS {
            let text = text.into();
            if !self.problems.contains(&text) {
                self.problems.push(text);
            }
        }
    }

    /// One more item left out.
    pub fn skip(&mut self) {
        self.skipped = self.skipped.saturating_add(1);
    }

    /// One more item brought in.
    pub fn add(&mut self) {
        self.imported = self.imported.saturating_add(1);
    }
}

/// `mail://import-progress`, sent at most a few times a second.
pub struct Progress {
    app: AppHandle,
    last: Option<Instant>,
}

impl Progress {
    /// Progress for one long import or export.
    pub fn new(app: &AppHandle) -> Self {
        Self {
            app: app.clone(),
            last: None,
        }
    }

    /// Say how far it is (`total` 0 while unknown).
    pub fn report(&mut self, phase: &str, done: u64, total: u64) {
        let due = self
            .last
            .is_none_or(|at| at.elapsed() >= Duration::from_millis(200));
        if due || (total > 0 && done >= total) {
            self.last = Some(Instant::now());
            let _ = self.app.emit(
                "mail://import-progress",
                serde_json::json!({ "phase": phase, "done": done, "total": total }),
            );
        }
    }
}

fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Refused while Work Offline is on: an import uploads.
pub fn require_online(state: &AppState) -> AppResult<()> {
    if settings::working_offline(state) {
        return Err(AppError::new(
            "connect",
            "You're working offline. Turn off Work Offline to import or export mail.",
        ));
    }
    Ok(())
}

/// A connection to an account's mail server.
pub async fn connect(state: &AppState, account: &Account) -> AppResult<ImapConnection> {
    let credentials = credentials::for_account(state, account).await?;
    Ok(ImapConnection::connect(&to_sync(&account.imap), &credentials).await?)
}

// ---------------------------------------------------------------- pickers

#[cfg(not(target_os = "android"))]
async fn pick(app: &AppHandle, filter: (&str, &[&str]), many: bool) -> AppResult<Vec<PathBuf>> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app.dialog().file().add_filter(filter.0, filter.1);
    let picked = tauri::async_runtime::spawn_blocking(move || {
        if many {
            dialog.blocking_pick_files().unwrap_or_default()
        } else {
            dialog.blocking_pick_file().into_iter().collect()
        }
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    picked
        .into_iter()
        .map(|f| {
            f.into_path()
                .map_err(|_| AppError::invalid("Pick a file on this computer."))
        })
        .collect()
}

#[cfg(target_os = "android")]
async fn pick(_app: &AppHandle, _filter: (&str, &[&str]), many: bool) -> AppResult<Vec<PathBuf>> {
    let picked = crate::files_android::pick_files(MAX_PICKED).await?;
    let mut paths: Vec<PathBuf> = picked.into_iter().map(|p| p.path).collect();
    if !many {
        paths.truncate(1);
    }
    Ok(paths)
}

/// Pick files (`many`) or one file of a kind. Empty when cancelled.
pub async fn pick_files(
    app: &AppHandle,
    name: &str,
    extensions: &[&str],
    many: bool,
) -> AppResult<Vec<PathBuf>> {
    let _ = MAX_PICKED;
    pick(app, (name, extensions), many).await
}

/// Where an export is written before it is saved.
pub struct SaveTarget {
    /// The file to write.
    pub path: PathBuf,
    name: String,
    mime: &'static str,
}

/// Ask where to save (desktop); on Android the file is written to the
/// cache and the phone's "Save to" screen comes after ([`finish_save`]).
/// `None` when cancelled.
#[cfg(not(target_os = "android"))]
pub async fn save_target(
    app: &AppHandle,
    _state: &AppState,
    name: &str,
    mime: &'static str,
) -> AppResult<Option<SaveTarget>> {
    use tauri_plugin_dialog::DialogExt;
    let name = safe_file_name(name);
    let dialog = app.dialog().file().set_file_name(&name);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    let Some(path) = picked else { return Ok(None) };
    let path = path.into_path().map_err(|_| {
        AppError::invalid("That place can't be saved to. Pick a folder on this computer.")
    })?;
    Ok(Some(SaveTarget { path, name, mime }))
}

#[cfg(target_os = "android")]
pub async fn save_target(
    _app: &AppHandle,
    state: &AppState,
    name: &str,
    mime: &'static str,
) -> AppResult<Option<SaveTarget>> {
    let name = safe_file_name(name);
    let dir = state
        .cache_dir
        .join("opened")
        .join(format!("export-{}", now()));
    tokio::fs::create_dir_all(&dir).await?;
    Ok(Some(SaveTarget {
        path: dir.join(&name),
        name,
        mime,
    }))
}

/// After writing: where it was saved, or `None` when the user cancelled
/// (Android's screen comes now).
#[cfg(not(target_os = "android"))]
#[allow(clippy::unused_async)] // async to match the Android version
pub async fn finish_save(target: SaveTarget) -> AppResult<Option<String>> {
    let _ = (&target.name, target.mime);
    Ok(Some(target.path.display().to_string()))
}

#[cfg(target_os = "android")]
pub async fn finish_save(target: SaveTarget) -> AppResult<Option<String>> {
    let saved = crate::files_android::save_file(&target.path, &target.name, target.mime).await;
    let _ = tokio::fs::remove_file(&target.path).await;
    saved
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// ---------------------------------------------------------------- mail upload

/// What became of one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Uploaded.
    Imported,
    /// Its Message-ID is already in the folder.
    Duplicate,
    /// The server refused it (why).
    Refused(String),
}

/// Uploads messages to an account's folders over one connection.
pub struct Uploader<'a> {
    state: &'a AppState,
    account: Account,
    conn: Option<ImapConnection>,
    /// Message-IDs per folder path, read once per folder.
    known: HashMap<String, HashSet<String>>,
}

fn connection_lost(error: &AppError) -> bool {
    matches!(error.kind, "connect" | "tls")
}

impl<'a> Uploader<'a> {
    /// Connect to the account's server (refused when working offline).
    pub async fn new(state: &'a AppState, account: Account) -> AppResult<Self> {
        require_online(state)?;
        let conn = connect(state, &account).await?;
        Ok(Self {
            state,
            account,
            conn: Some(conn),
            known: HashMap::new(),
        })
    }

    /// The connection, made again if it was lost.
    pub async fn conn(&mut self) -> AppResult<&mut ImapConnection> {
        if self.conn.is_none() {
            self.conn = Some(connect(self.state, &self.account).await?);
        }
        self.conn
            .as_mut()
            .ok_or_else(|| AppError::internal("no connection"))
    }

    async fn known_ids(&mut self, path: &str) -> AppResult<&mut HashSet<String>> {
        if !self.known.contains_key(path) {
            // A folder made just now, or one the server can't list: only what
            // this import brings is known.
            let mut ids: HashSet<String> = self
                .conn()
                .await?
                .message_ids(path)
                .await
                .unwrap_or_default();
            let local = self
                .state
                .store
                .list_folders(&self.account.id)?
                .into_iter()
                .find(|f| f.path == path);
            if let Some(folder) = local {
                ids.extend(self.state.store.message_ids_in_folder(&folder.id)?);
            }
            self.known.insert(path.to_owned(), ids);
        }
        self.known
            .get_mut(path)
            .ok_or_else(|| AppError::internal("no message list"))
    }

    /// Upload one message to the folder at `path`, unless its Message-ID
    /// is there already.
    ///
    /// # Errors
    /// When the server can't be reached (a refused message is an
    /// [`Outcome`]).
    pub async fn upload(
        &mut self,
        path: &str,
        raw: &[u8],
        flags: FileFlags,
        date: Option<i64>,
        message_id: Option<&str>,
    ) -> AppResult<Outcome> {
        let key = message_id.map(message_id_key).filter(|k| !k.is_empty());
        if let Some(key) = &key
            && self.known_ids(path).await?.contains(key)
        {
            return Ok(Outcome::Duplicate);
        }
        let raw = to_crlf(raw);
        let date = date.filter(|d| *d > 0).map(imap_date);
        let flags = flags.imap();
        let mut tries = 0u8;
        loop {
            let sent = self
                .conn()
                .await?
                .append_dated(path, &raw, &flags, date.as_deref())
                .await
                .map_err(AppError::from);
            match sent {
                Ok(()) => break,
                Err(error) if connection_lost(&error) && tries == 0 => {
                    tries = 1;
                    self.conn = None;
                }
                Err(error) if connection_lost(&error) => return Err(error),
                Err(error) => return Ok(Outcome::Refused(error.message)),
            }
        }
        if let Some(key) = key {
            self.known_ids(path).await?.insert(key);
        }
        Ok(Outcome::Imported)
    }

    /// Upload a message as read from a file (its own state, else read).
    ///
    /// # Errors
    /// When the server can't be reached.
    pub async fn upload_file_message(
        &mut self,
        path: &str,
        raw: &[u8],
        report: &mut ImportReport,
    ) -> AppResult<()> {
        let flags = flags_of(raw).unwrap_or(FileFlags {
            seen: true,
            ..FileFlags::default()
        });
        if flags.deleted {
            report.skip();
            report.problem("Messages marked as deleted in the file were left out.");
            return Ok(());
        }
        let (date, id) = date_and_id(raw);
        match self.upload(path, raw, flags, date, id.as_deref()).await? {
            Outcome::Imported => report.add(),
            Outcome::Duplicate => {
                report.skip();
                report.problem("Messages already in the folder were left out.");
            }
            Outcome::Refused(why) => {
                report.skip();
                let subject = parse_summary(raw).map(|s| s.subject).unwrap_or_default();
                report.problem(format!("The server didn't accept \"{subject}\": {why}"));
            }
        }
        Ok(())
    }

    /// Done: log out, and sync the account so the messages show.
    pub async fn finish(mut self, app: &AppHandle) {
        if let Some(conn) = self.conn.take() {
            conn.logout().await;
        }
        self.state.wake_sync(Some(&self.account.id));
        events::changed(app, &self.account.id, &[]);
    }
}

/// Upload every message of one file (`.eml`, or `.mbox` read a message at
/// a time).
async fn import_one_file(
    up: &mut Uploader<'_>,
    folder: &Folder,
    path: &Path,
    report: &mut ImportReport,
    progress: &mut Progress,
    (index, files): (usize, usize),
) -> AppResult<()> {
    let name = file_name(path);
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut head = [0u8; 1024];
    let read = file.read(&mut head)?;
    file.seek(SeekFrom::Start(0))?;
    if is_mbox(head.get(..read).unwrap_or_default()) {
        let mut reader = MboxReader::new(BufReader::with_capacity(256 * 1024, file));
        let mut done = 0u64;
        while let Some(item) = reader.next_message()? {
            match item {
                MboxItem::Message(raw) => {
                    up.upload_file_message(&folder.path, &raw, report).await?;
                }
                MboxItem::TooLarge(_) => {
                    report.skip();
                    report.problem(format!(
                        "A message in {name} is larger than 100 MB and was left out."
                    ));
                }
            }
            done = done.saturating_add(1);
            progress.report(&format!("Uploading {name}: {done} messages"), done, 0);
        }
        return Ok(());
    }
    if size > count(MAX_MESSAGE_BYTES) {
        report.skip();
        report.problem(format!("{name} is larger than 100 MB and was left out."));
        return Ok(());
    }
    let mut raw = Vec::new();
    file.read_to_end(&mut raw)?;
    if raw.is_empty() || parse_summary(&raw).is_err() || !looks_like_mail(&raw) {
        report.skip();
        report.problem(format!("{name} isn't an email message or a mailbox file."));
        return Ok(());
    }
    up.upload_file_message(&folder.path, &raw, report).await?;
    progress.report(
        "Uploading messages",
        count(index.saturating_add(1)),
        count(files),
    );
    Ok(())
}

/// Whether bytes start with mail headers (`Name: value`).
fn looks_like_mail(raw: &[u8]) -> bool {
    let start = raw.get(..raw.len().min(2048)).unwrap_or_default();
    let text = String::from_utf8_lossy(start);
    text.lines().next().is_some_and(|line| {
        line.split_once(':').is_some_and(|(name, _)| {
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
    })
}

/// `import_mail_files`: pick `.eml` and `.mbox` files and upload their
/// messages into a folder. `None` when cancelled.
#[tauri::command]
pub async fn import_mail_files(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    folder_id: String,
) -> AppResult<Option<ImportReport>> {
    let folder = state.store.get_folder(&folder_id)?;
    let account = state.store.get_account(&folder.account_id)?;
    require_online(&state)?;
    let paths = pick_files(&app, "Mail files", &["eml", "mbox", "mbx", "mbs"], true).await?;
    if paths.is_empty() {
        return Ok(None);
    }
    let mut up = Uploader::new(&state, account).await?;
    let mut report = ImportReport::default();
    let mut progress = Progress::new(&app);
    progress.report("Reading the files", 0, count(paths.len()));
    let files = paths.len();
    let mut result = Ok(());
    for (index, path) in paths.iter().enumerate() {
        result = import_one_file(
            &mut up,
            &folder,
            path,
            &mut report,
            &mut progress,
            (index, files),
        )
        .await;
        if result.is_err() {
            break;
        }
    }
    up.finish(&app).await;
    // A lost connection ends the import; what came in so far stays.
    if let Err(error) = result {
        if report.imported == 0 {
            return Err(error);
        }
        report.problem(format!("The import stopped early: {}", error.message));
    }
    Ok(Some(report))
}

/// `import_message_file`: add an opened `.eml` file to a folder.
#[tauri::command]
pub async fn import_message_file(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    path: String,
    folder_id: String,
) -> AppResult<()> {
    let path = opened::checked(&path)?;
    let raw = opened::read_limited(&path, MAX_MESSAGE_BYTES)?;
    let folder = state.store.get_folder(&folder_id)?;
    let account = state.store.get_account(&folder.account_id)?;
    let mut up = Uploader::new(&state, account).await?;
    let mut report = ImportReport::default();
    let result = up
        .upload_file_message(&folder.path, &raw, &mut report)
        .await;
    up.finish(&app).await;
    result?;
    if report.imported == 0 {
        return Err(AppError::invalid(
            report.problems.first().cloned().map_or_else(
                || format!("This message couldn't be added to {}.", folder.name),
                |why| {
                    why.replace(
                        "Messages already in the folder were left out.",
                        &format!("This message is already in {}.", folder.name),
                    )
                },
            ),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------- mail export

/// What to export (`exportMail`'s `target`).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ExportTarget {
    /// A whole folder.
    #[serde(rename_all = "camelCase")]
    Folder {
        /// The folder.
        folder_id: String,
    },
    /// Some messages.
    Ids {
        /// The messages.
        ids: Vec<String>,
    },
}

/// One message to write.
enum Pending {
    /// A message known here.
    Local { id: String, flags: FileFlags },
    /// One only on the server, at this UID.
    Server { uid: u32, flags: FileFlags },
}

/// Writes exported messages: one `.mbox` file, or `.eml` files in a `.zip`.
enum Sink {
    Mbox(BufWriter<File>),
    Zip(Box<zip::ZipWriter<File>>, HashSet<String>),
}

impl Sink {
    fn add(&mut self, raw: &[u8], flags: FileFlags) -> AppResult<()> {
        match self {
            Self::Mbox(out) => {
                let (date, _) = date_and_id(raw);
                let date = date.unwrap_or_else(|| i64::try_from(now()).unwrap_or(0));
                write_message(out, raw, &envelope_sender(raw), date, Some(flags))?;
            }
            Self::Zip(zip, names) => {
                let subject = parse_summary(raw).map(|s| s.subject).unwrap_or_default();
                let base = crate::message_tools::eml_name(&subject);
                let stem = base.trim_end_matches(".eml").to_owned();
                let mut name = base;
                let mut n = 1u32;
                while names.contains(&name.to_lowercase()) {
                    n = n.saturating_add(1);
                    name = format!("{stem} ({n}).eml");
                }
                names.insert(name.to_lowercase());
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .large_file(raw.len() > 0xFFFF_0000);
                zip.start_file(name, options)
                    .map_err(|e| AppError::internal(e.to_string()))?;
                zip.write_all(raw)?;
            }
        }
        Ok(())
    }

    fn finish(self) -> AppResult<()> {
        match self {
            Self::Mbox(mut out) => out.flush()?,
            Self::Zip(zip, _) => {
                zip.finish()
                    .map_err(|e| AppError::internal(e.to_string()))?;
            }
        }
        Ok(())
    }
}

fn local_flags(state: &AppState, id: &str) -> FileFlags {
    state
        .store
        .get_message(id)
        .map(|m| FileFlags {
            seen: !m.item.unread,
            flagged: m.item.flagged,
            answered: m.item.answered,
            deleted: false,
        })
        .unwrap_or_default()
}

/// What a folder export writes: every message on the server (or, working
/// offline, every one here), oldest first.
async fn folder_messages(
    state: &AppState,
    folder: &Folder,
    conn: Option<&mut ImapConnection>,
) -> AppResult<Vec<Pending>> {
    let rows = state.store.export_rows(&folder.id)?;
    let Some(conn) = conn else {
        let missing = rows.iter().filter(|r| !r.has_raw).count();
        if missing > 0 {
            return Err(AppError::new(
                "connect",
                format!(
                    "{missing} messages in this folder aren't on this device yet. Turn off Work Offline to export the whole folder."
                ),
            ));
        }
        return Ok(rows
            .into_iter()
            .map(|r| Pending::Local {
                flags: FileFlags {
                    seen: r.seen,
                    flagged: r.flagged,
                    answered: r.answered,
                    deleted: false,
                },
                id: r.id,
            })
            .collect());
    };
    let mut uids: Vec<u32> = conn.all_uids(&folder.path).await?.into_iter().collect();
    uids.sort_unstable();
    let local: HashMap<u32, &mail_store::ExportRow> =
        rows.iter().filter_map(|r| Some((r.uid?, r))).collect();
    let unknown: Vec<u32> = uids
        .iter()
        .copied()
        .filter(|u| !local.contains_key(u))
        .collect();
    let mut server_flags: HashMap<u32, FileFlags> = HashMap::new();
    for chunk in unknown.chunks(500) {
        if let Some(set) = uid_set(chunk) {
            for (uid, f) in conn.fetch_flags(&folder.path, &set).await? {
                server_flags.insert(
                    uid,
                    FileFlags {
                        seen: f.seen,
                        flagged: f.flagged,
                        answered: f.answered,
                        deleted: f.deleted,
                    },
                );
            }
        }
    }
    Ok(uids
        .into_iter()
        .filter_map(|uid| {
            if let Some(row) = local.get(&uid) {
                return Some(Pending::Local {
                    id: row.id.clone(),
                    flags: FileFlags {
                        seen: row.seen,
                        flagged: row.flagged,
                        answered: row.answered,
                        deleted: false,
                    },
                });
            }
            let flags = server_flags.get(&uid).copied().unwrap_or_default();
            (!flags.deleted).then_some(Pending::Server {
                uid,
                flags: FileFlags {
                    deleted: false,
                    ..flags
                },
            })
        })
        .collect())
}

/// Largest message fetched for an export.
const MAX_EXPORT_FETCH: u64 = 100 * 1024 * 1024;

/// `export_mail`: a folder as one `.mbox` file, or messages as `.eml`
/// files in a `.zip`. Where it was saved; `None` when cancelled.
#[tauri::command]
pub async fn export_mail(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    target: ExportTarget,
    format: String,
) -> AppResult<Option<String>> {
    let zip = match format.as_str() {
        "mbox" => false,
        "eml" => true,
        _ => return Err(AppError::invalid("Choose .mbox or .eml.")),
    };
    let (stem, folder) = match &target {
        ExportTarget::Folder { folder_id } => {
            let folder = state.store.get_folder(folder_id)?;
            (folder.name.clone(), Some(folder))
        }
        ExportTarget::Ids { ids } => {
            crate::mail::check_batch(ids)?;
            ("Messages".to_owned(), None)
        }
    };
    let (name, mime) = if zip {
        (format!("{stem}.zip"), "application/zip")
    } else {
        (format!("{stem}.mbox"), "application/mbox")
    };
    let Some(save) = save_target(&app, &state, &name, mime).await? else {
        return Ok(None);
    };
    let file = File::create(&save.path)?;
    let mut sink = if zip {
        Sink::Zip(Box::new(zip::ZipWriter::new(file)), HashSet::new())
    } else {
        Sink::Mbox(BufWriter::new(file))
    };
    let mut progress = Progress::new(&app);
    let written = match (&target, folder) {
        (ExportTarget::Folder { .. }, Some(folder)) => {
            Box::pin(export_folder(&state, &folder, &mut sink, &mut progress)).await
        }
        (ExportTarget::Ids { ids }, _) => {
            let mut result = Ok(());
            for (i, id) in ids.iter().enumerate() {
                progress.report("Saving messages", count(i), count(ids.len()));
                match sync::body_now(&state, id).await {
                    Ok(raw) => {
                        if let Err(e) = sink.add(&raw, local_flags(&state, id)) {
                            result = Err(e);
                            break;
                        }
                    }
                    Err(e) => {
                        result = Err(e);
                        break;
                    }
                }
            }
            result
        }
        _ => Err(AppError::internal("no folder")),
    };
    let finished = written.and_then(|()| sink.finish());
    if let Err(error) = finished {
        let _ = std::fs::remove_file(&save.path);
        return Err(error);
    }
    progress.report("Saved", 1, 1);
    finish_save(save).await
}

async fn export_folder(
    state: &AppState,
    folder: &Folder,
    sink: &mut Sink,
    progress: &mut Progress,
) -> AppResult<()> {
    let account = state.store.get_account(&folder.account_id)?;
    let mut conn = if settings::working_offline(state) {
        None
    } else {
        Some(connect(state, &account).await?)
    };
    let pending = folder_messages(state, folder, conn.as_mut()).await;
    let pending = match pending {
        Ok(p) => p,
        Err(e) => {
            if let Some(conn) = conn {
                conn.logout().await;
            }
            return Err(e);
        }
    };
    let total = count(pending.len());
    let mut result = Ok(());
    for (i, item) in pending.into_iter().enumerate() {
        progress.report(&format!("Saving {}", folder.name), count(i), total);
        let step = async {
            match item {
                Pending::Local { id, flags } => {
                    let raw = match state.store.get_raw(&id)? {
                        Some(raw) => raw,
                        None => sync::body_now(state, &id).await?,
                    };
                    sink.add(&raw, flags)
                }
                Pending::Server { uid, flags } => {
                    let conn = conn
                        .as_mut()
                        .ok_or_else(|| AppError::internal("no connection"))?;
                    let raw = match state.store.raw_by_uid(&folder.id, uid)? {
                        Some(raw) => Some(raw),
                        None => conn.fetch_raw(&folder.path, uid, MAX_EXPORT_FETCH).await?,
                    };
                    match raw {
                        Some(raw) => sink.add(&raw, flags),
                        // Deleted on the server meanwhile.
                        None => Ok(()),
                    }
                }
            }
        };
        if let Err(e) = step.await {
            result = Err(e);
            break;
        }
    }
    if let Some(conn) = conn {
        conn.logout().await;
    }
    result
}

// ---------------------------------------------------------------- calendars

/// Add an `.ics` file's events to a calendar (on its server, then here).
/// Events whose `UID` the calendar has are left out.
pub async fn import_ics(
    app: &AppHandle,
    state: &AppState,
    calendar_id: &str,
    bytes: &[u8],
) -> AppResult<ImportReport> {
    let calendar = state.store.get_calendar(calendar_id)?;
    if !calendar.writable {
        return Err(AppError::invalid(
            "Events can't be added to this calendar. Choose another one.",
        ));
    }
    if settings::working_offline(state) {
        return Err(AppError::new(
            "connect",
            "You're working offline. Turn off Work Offline to add events to the calendar.",
        ));
    }
    let text = ics_file::file_text(bytes);
    let file = ics_file::split_calendar(&text, &mut || {
        mail_store::new_event_uid().unwrap_or_else(|_| format!("{}-import@mocky.mail", now()))
    });
    let mut report = ImportReport::default();
    if file.objects.is_empty() && file.other == 0 {
        return Err(AppError::invalid("This file has no calendar events in it."));
    }
    let (client, url) = crate::calendar::client_for(state, &calendar).await?;
    let mut known: HashSet<String> = state
        .store
        .calendar_events_in(&calendar.id)?
        .iter()
        .filter_map(|e| ical::uid_of(&e.ics))
        .collect();
    let mut progress = Progress::new(app);
    let total = count(file.objects.len());
    for (i, object) in file.objects.iter().enumerate() {
        progress.report(
            &format!("Adding events to {}", calendar.name),
            count(i),
            total,
        );
        if known.contains(&object.uid) {
            report.skip();
            report.problem("Events already in the calendar were left out.");
            continue;
        }
        match client
            .put_event(&url, None, &object.uid, &object.ics, None)
            .await
        {
            Ok((href, etag)) => {
                state.store.put_calendar_event(
                    &calendar.id,
                    &href,
                    etag.as_deref(),
                    &object.ics,
                )?;
                known.insert(object.uid.clone());
                report.add();
            }
            Err(error) => {
                let error = AppError::from(error);
                if connection_lost(&error) {
                    return Err(error);
                }
                report.skip();
                let title = ical::master(&object.ics)
                    .map(|o| o.title)
                    .unwrap_or_default();
                report.problem(format!(
                    "The calendar server didn't accept \"{title}\": {}",
                    error.message
                ));
            }
        }
    }
    for _ in 0..file.skipped {
        report.skip();
    }
    if file.skipped > 0 {
        report.problem("Events too large to keep were left out.");
    }
    for _ in 0..file.other {
        report.skip();
    }
    if file.other > 0 {
        report.problem("Tasks and journal entries in the file can't be added to a calendar yet.");
    }
    crate::calendar::changed(app, &[]);
    Ok(report)
}

/// `import_calendar_file`: pick an `.ics` file and add its events to a
/// calendar. `None` when cancelled.
#[tauri::command]
pub async fn import_calendar_file(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    calendar_id: String,
) -> AppResult<Option<ImportReport>> {
    state.store.get_calendar(&calendar_id)?;
    let Some(path) = pick_files(&app, "Calendar files", &["ics", "ical", "ifb"], false)
        .await?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let bytes = opened::read_limited(&path, MAX_TEXT_FILE)?;
    import_ics(&app, &state, &calendar_id, &bytes)
        .await
        .map(Some)
}

/// `export_calendar`: a calendar as one `.ics` file.
#[tauri::command]
pub async fn export_calendar(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    calendar_id: String,
) -> AppResult<Option<String>> {
    let calendar = state.store.get_calendar(&calendar_id)?;
    let events = state.store.calendar_events_in(&calendar.id)?;
    let text =
        ics_file::join_calendars(events.iter().map(|e| e.ics.as_str()), Some(&calendar.name));
    let Some(save) = save_target(
        &app,
        &state,
        &format!("{}.ics", calendar.name),
        "text/calendar",
    )
    .await?
    else {
        return Ok(None);
    };
    tokio::fs::write(&save.path, text).await?;
    finish_save(save).await
}

// ---------------------------------------------------------------- contacts

fn book_of(account_id: Option<&str>) -> Book<'_> {
    match account_id {
        None => Book::Device,
        Some("*") => Book::All,
        Some(id) => Book::Account(id),
    }
}

/// Add a `.vcf` file's contacts to an address book (`None`: this
/// device's). A card whose `UID`, or name and address, is there already is
/// left out.
pub fn import_vcf(
    app: &AppHandle,
    state: &Arc<AppState>,
    account_id: Option<&str>,
    bytes: &[u8],
) -> AppResult<ImportReport> {
    if let Some(id) = account_id {
        state.store.get_account(id)?;
    }
    let file = vcf::read_cards(bytes);
    if file.cards.is_empty() && file.skipped == 0 {
        return Err(AppError::invalid("This file has no contacts in it."));
    }
    let mut report = ImportReport::default();
    let (mut uids, mut people) = state.store.contact_keys(book_of(account_id))?;
    let mut progress = Progress::new(app);
    let total = count(file.cards.len());
    for (i, card) in file.cards.into_iter().enumerate() {
        progress.report("Adding contacts", count(i), total);
        let uid = card.card.uid.trim().to_owned();
        let person = person_key(&card.card);
        if (!uid.is_empty() && uids.contains(&uid)) || people.contains(&person) {
            report.skip();
            report.problem("Contacts already in the address book were left out.");
            continue;
        }
        state
            .store
            .import_contact(account_id, card.card, &card.vcard, now())?;
        uids.insert(uid);
        people.insert(person);
        report.add();
    }
    for _ in 0..file.skipped {
        report.skip();
    }
    if file.skipped > 0 {
        report.problem("Cards with no name, address or number (or too large) were left out.");
    }
    crate::contacts::soon(app, state);
    Ok(report)
}

/// `import_contacts_file`: pick a `.vcf` file and add its contacts
/// (`accountId` null: this device). `None` when cancelled.
#[tauri::command]
pub async fn import_contacts_file(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    account_id: Option<String>,
) -> AppResult<Option<ImportReport>> {
    let Some(path) = pick_files(&app, "Contact cards", &["vcf", "vcard"], false)
        .await?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let bytes = opened::read_limited(&path, MAX_TEXT_FILE)?;
    import_vcf(&app, &state, account_id.as_deref(), &bytes).map(Some)
}

/// `export_contacts`: contacts as one `.vcf` file (vCard 3.0): an
/// account's, this device's (null) or everyone's (`*`).
#[tauri::command]
pub async fn export_contacts(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    account_id: Option<String>,
) -> AppResult<Option<String>> {
    let contacts = state
        .store
        .contacts_in_book(book_of(account_id.as_deref()))?;
    let text = vcf::write_cards(
        contacts
            .iter()
            .map(|c| (&c.contact.card, c.vcard.as_deref())),
    );
    let Some(save) = save_target(&app, &state, "Contacts.vcf", "text/vcard").await? else {
        return Ok(None);
    };
    tokio::fs::write(&save.path, text).await?;
    finish_save(save).await
}

// ---------------------------------------------------------------- opened files

/// Where an opened file's items go (`importOpenedFile`'s `target`).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum OpenedTarget {
    /// A calendar, for an `.ics` file.
    #[serde(rename_all = "camelCase")]
    Calendar {
        /// The calendar.
        calendar_id: String,
    },
    /// An address book, for a `.vcf` file (null: this device).
    #[serde(rename_all = "camelCase")]
    Contacts {
        /// The account.
        account_id: Option<String>,
    },
}

/// `import_opened_file`: add an opened `.ics` or `.vcf` file's items.
#[tauri::command]
pub async fn import_opened_file(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    path: String,
    target: OpenedTarget,
) -> AppResult<ImportReport> {
    let path = opened::checked(&path)?;
    let bytes = opened::read_limited(&path, MAX_TEXT_FILE)?;
    match (opened::kind_of(&path), target) {
        (Some("ics") | None, OpenedTarget::Calendar { calendar_id }) => {
            import_ics(&app, &state, &calendar_id, &bytes).await
        }
        (Some("vcf") | None, OpenedTarget::Contacts { account_id }) => {
            import_vcf(&app, &state, account_id.as_deref(), &bytes)
        }
        _ => Err(AppError::invalid(
            "Choose a calendar for an .ics file, or an address book for a .vcf file.",
        )),
    }
}

/// `read_message_file`: an opened `.eml` file, read for showing.
#[tauri::command]
pub fn read_message_file(
    state: State<'_, Arc<AppState>>,
    path: String,
) -> AppResult<crate::mail::MessageDetail> {
    let path = opened::checked(&path)?;
    let raw = opened::read_limited(&path, MAX_MESSAGE_BYTES)?;
    crate::mail::file_detail(&state, &opened::id_for(&path), &raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_keep_fifty_different_problems() {
        let mut report = ImportReport::default();
        for i in 0..80 {
            report.problem(format!("problem {i}"));
            report.problem(format!("problem {i}"));
        }
        assert_eq!(report.problems.len(), MAX_PROBLEMS);
        report.add();
        report.skip();
        let json = serde_json::to_value(&report).unwrap_or_default();
        assert_eq!(json["imported"], 1);
        assert_eq!(json["skipped"], 1);
        assert!(json.get("counts").is_none() && json.get("folderId").is_none());
    }

    #[test]
    fn targets_read_as_the_interface_sends_them() {
        let folder: ExportTarget = serde_json::from_str(r#"{"folderId":"f1"}"#)
            .unwrap_or(ExportTarget::Ids { ids: vec![] });
        assert!(matches!(folder, ExportTarget::Folder { folder_id } if folder_id == "f1"));
        let ids: ExportTarget =
            serde_json::from_str(r#"{"ids":["a","b"]}"#).unwrap_or(ExportTarget::Folder {
                folder_id: String::new(),
            });
        assert!(matches!(ids, ExportTarget::Ids { ids } if ids.len() == 2));
        let cal: Option<OpenedTarget> = serde_json::from_str(r#"{"calendarId":"c"}"#).ok();
        assert!(matches!(cal, Some(OpenedTarget::Calendar { .. })));
        let device: Option<OpenedTarget> = serde_json::from_str(r#"{"accountId":null}"#).ok();
        assert!(matches!(
            device,
            Some(OpenedTarget::Contacts { account_id: None })
        ));
    }

    #[test]
    fn mail_files_start_with_headers() {
        assert!(looks_like_mail(b"Return-Path: <a@b>\r\nSubject: x\r\n\r\n"));
        assert!(!looks_like_mail(b"%PDF-1.4"));
        assert!(!looks_like_mail(b"hello world: no"));
    }
}
