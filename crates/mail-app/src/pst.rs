//! Importing an Outlook data file (`.pst`) into an account.
//!
//! Mail goes into a new folder tree on the account's server, "Imported from
//! Outlook", with the file's own folders under it (uploaded like any
//! other import, so a second import of the same file brings nothing
//! twice). Contacts go to the account's address book when it has one on
//! the server, else to this device's; calendar items to the account's
//! first calendar (there is no calendar on the device). Tasks, notes and
//! journal entries are counted and left out until the app can keep them.
//!
//! The file is read with `mail_core::pst_import`, a block at a time.

use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;

use mail_core::folders::encode_folder_name;
use mail_core::ical;
use mail_core::mbox::FileFlags;
use mail_core::pst_import::{ItemKind, PstFile, PstFolder};
use mail_store::{Book, Calendar, person_key};

use crate::Ctx;
use crate::error::{AppError, AppResult};
use crate::files::{Counts, ImportReport, Outcome, Progress, Uploader, pick_files, require_online};
use crate::state::{AppState, now};
use crate::{events, folders, sync};

/// The folder imported mail goes under.
const TOP: &str = "Imported from Outlook";

/// A folder name the server will take: no level separator or characters
/// IMAP treats specially, not empty.
fn safe_name(name: &str, delimiter: Option<&str>) -> String {
    let mut clean: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '*' | '%' | '"' | '\\') {
                '-'
            } else {
                c
            }
        })
        .collect();
    if let Some(d) = delimiter.filter(|d| !d.is_empty()) {
        clean = clean.replace(d, "-");
    }
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean: String = clean.chars().take(200).collect();
    if clean.trim().is_empty() {
        "Folder".to_owned()
    } else {
        clean
    }
}

/// The server paths of the folder tree: the top, and each folder's.
struct Paths {
    top: String,
    delimiter: Option<String>,
}

impl Paths {
    fn of(&self, path: &[String]) -> String {
        match &self.delimiter {
            Some(d) => {
                let mut out = self.top.clone();
                for name in path {
                    out.push_str(d);
                    out.push_str(&encode_folder_name(&safe_name(name, Some(d))));
                }
                out
            }
            // A server without levels: one folder per path, named in full.
            None if path.is_empty() => self.top.clone(),
            None => encode_folder_name(&safe_name(&format!("{TOP} - {}", path.join(" - ")), None)),
        }
    }

    /// The path and each one above it below the top, top first.
    fn chain(&self, path: &[String]) -> Vec<String> {
        if self.delimiter.is_none() {
            return vec![self.top.clone(), self.of(path)];
        }
        (0..=path.len())
            .map(|n| self.of(path.get(..n).unwrap_or_default()))
            .collect()
    }
}

/// Where contacts and events of the file go.
struct Places {
    contacts: Option<String>,
    calendar: Option<Calendar>,
    contact_keys: (HashSet<String>, HashSet<(String, String)>),
    event_uids: HashSet<String>,
}

/// Everything one import keeps track of.
struct Import<'a> {
    state: &'a Arc<AppState>,
    up: Uploader<'a>,
    paths: Paths,
    created: HashSet<String>,
    places: Places,
    calendar_client: Option<(mail_sync::dav::DavClient, reqwest::Url)>,
    report: ImportReport,
    counts: Counts,
    left_out: (u32, u32, u32),
}

impl Import<'_> {
    async fn ensure_folder(&mut self, path: &[String]) -> AppResult<String> {
        let target = self.paths.of(path);
        for step in self.paths.chain(path) {
            if self.created.insert(step.clone()) {
                self.up.create_folder(&step).await?;
            }
        }
        Ok(target)
    }

    async fn mail(
        &mut self,
        file: &mut PstFile<BufReader<File>>,
        folder: &PstFolder,
        item: &mail_core::pst_import::PstItem,
    ) -> AppResult<()> {
        let built = match file.message(item) {
            Ok(built) => built,
            Err(why) => {
                self.report.skip();
                self.report.problem(format!(
                    "A message in {} couldn't be read: {why}",
                    folder.path.join("/")
                ));
                return Ok(());
            }
        };
        for note in &built.notes {
            self.report
                .problem(format!("A message in {}: {note}", folder.path.join("/")));
        }
        let path = self.ensure_folder(&folder.path).await?;
        let flags = FileFlags {
            seen: built.seen,
            flagged: built.flagged,
            answered: built.answered,
            deleted: false,
        };
        match self
            .up
            .upload(
                &path,
                &built.raw,
                flags,
                built.received,
                built.message_id.as_deref(),
            )
            .await?
        {
            Outcome::Imported => {
                self.report.add();
                self.counts.messages = self.counts.messages.saturating_add(1);
            }
            Outcome::Duplicate => {
                self.report.skip();
                self.report
                    .problem("Messages already in the folder were left out.");
            }
            Outcome::Refused(why) => {
                self.report.skip();
                self.report.problem(format!(
                    "The server didn't accept a message in {}: {why}",
                    folder.path.join("/")
                ));
            }
        }
        Ok(())
    }

    fn contact(
        &mut self,
        file: &mut PstFile<BufReader<File>>,
        item: mail_core::pst_import::PstItem,
    ) -> AppResult<()> {
        let built = match file.contact(&item) {
            Ok(Some(built)) => built,
            Ok(None) => {
                self.report.skip();
                self.report
                    .problem("Contacts with no name, address or number were left out.");
                return Ok(());
            }
            Err(why) => {
                self.report.skip();
                self.report
                    .problem(format!("A contact couldn't be read: {why}"));
                return Ok(());
            }
        };
        let uid = built.card.uid.clone();
        let person = person_key(&built.card);
        let (uids, people) = &mut self.places.contact_keys;
        if uids.contains(&uid) || people.contains(&person) {
            self.report.skip();
            self.report
                .problem("Contacts already in the address book were left out.");
            return Ok(());
        }
        self.state.store.import_contact(
            self.places.contacts.as_deref(),
            built.card,
            &built.vcard,
            now(),
        )?;
        uids.insert(uid);
        people.insert(person);
        self.report.add();
        self.counts.contacts = self.counts.contacts.saturating_add(1);
        Ok(())
    }

    async fn event(
        &mut self,
        file: &mut PstFile<BufReader<File>>,
        item: &mail_core::pst_import::PstItem,
    ) -> AppResult<()> {
        let Some(calendar) = self.places.calendar.clone() else {
            self.report.skip();
            self.report
                .problem("This account has no calendar here, so calendar items were left out.");
            return Ok(());
        };
        let built = match file.event(item) {
            Ok(Some(built)) => built,
            Ok(None) => {
                self.report.skip();
                self.report
                    .problem("Calendar items without a date were left out.");
                return Ok(());
            }
            Err(why) => {
                self.report.skip();
                self.report
                    .problem(format!("A calendar item couldn't be read: {why}"));
                return Ok(());
            }
        };
        for note in &built.notes {
            self.report.problem(format!("A calendar item: {note}"));
        }
        if self.places.event_uids.contains(&built.uid) {
            self.report.skip();
            self.report
                .problem("Events already in the calendar were left out.");
            return Ok(());
        }
        if self.calendar_client.is_none() {
            self.calendar_client = Some(crate::calendar::client_for(self.state, &calendar).await?);
        }
        let Some((client, url)) = &self.calendar_client else {
            return Ok(());
        };
        match client
            .put_event(url, None, &built.uid, &built.ics, None)
            .await
        {
            Ok((href, etag)) => {
                self.state.store.put_calendar_event(
                    &calendar.id,
                    &href,
                    etag.as_deref(),
                    &built.ics,
                )?;
                self.places.event_uids.insert(built.uid);
                self.report.add();
                self.counts.events = self.counts.events.saturating_add(1);
            }
            Err(error) => {
                self.report.skip();
                self.report.problem(format!(
                    "The calendar server didn't accept an event: {}",
                    AppError::from(error).message
                ));
            }
        }
        Ok(())
    }

    fn later(&mut self, kind: ItemKind) {
        self.report.skip();
        match kind {
            ItemKind::Task => self.left_out.0 = self.left_out.0.saturating_add(1),
            ItemKind::Note => self.left_out.1 = self.left_out.1.saturating_add(1),
            _ => self.left_out.2 = self.left_out.2.saturating_add(1),
        }
        self.report
            .problem("Tasks and notes come in a later update.");
    }
}

/// The account's address book on its server, if it has one; else this
/// device's.
fn contacts_place(state: &AppState, account_id: &str) -> Option<String> {
    crate::contacts::has_address_book(state, account_id).then(|| account_id.to_owned())
}

/// `import_pst`: pick an Outlook `.pst` file and bring its mail, contacts
/// and calendar into an account. `None` when cancelled.
#[allow(clippy::too_many_lines)]
pub async fn import_pst(ctx: &Ctx, account_id: String) -> AppResult<Option<ImportReport>> {
    let state = &ctx.state;
    let account = state.store.get_account(&account_id)?;
    require_online(state)?;
    let Some(path) = pick_files(ctx, "Outlook data files", &["pst"], false)
        .await?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let reader = BufReader::with_capacity(64 * 1024, File::open(&path)?);
    let mut file = PstFile::open(reader).map_err(|e| AppError::invalid(format!("{e}.")))?;
    let pst_folders = file
        .folders()
        .map_err(|e| AppError::invalid(format!("{e}.")))?;
    let mut progress = Progress::new(ctx);
    progress.report("Reading the Outlook file", 0, 0);

    let state_ref: &Arc<AppState> = state;
    let known_folders = state.store.list_folders(&account.id)?;
    let delimiter = folders::delimiter_of(&known_folders);
    let prefix = folders::top_level_prefix(&known_folders, delimiter.as_deref());
    let paths = Paths {
        top: format!("{prefix}{}", encode_folder_name(TOP)),
        delimiter,
    };
    let contacts = contacts_place(state, &account.id);
    let book = match contacts.as_deref() {
        Some(id) => Book::Account(id),
        None => Book::Device,
    };
    let calendar = state
        .store
        .list_calendars()?
        .into_iter()
        .find(|c| c.account_id == account.id);
    let event_uids = match &calendar {
        Some(c) => state
            .store
            .calendar_events_in(&c.id)?
            .iter()
            .filter_map(|e| ical::uid_of(&e.ics))
            .collect(),
        None => HashSet::new(),
    };
    let places = Places {
        contact_keys: state.store.contact_keys(book)?,
        contacts,
        calendar,
        event_uids,
    };
    let up = Uploader::new(state_ref, account.clone()).await?;
    let mut import = Import {
        state: state_ref,
        up,
        paths,
        created: HashSet::new(),
        places,
        calendar_client: None,
        report: ImportReport::default(),
        counts: Counts::default(),
        left_out: (0, 0, 0),
    };
    let result = run(&mut file, &pst_folders, &mut import, &mut progress).await;

    // The new folders show at once; their mail arrives with the sync.
    let top = import.paths.top.clone();
    let made_folders = !import.created.is_empty();
    let Import {
        up,
        mut report,
        counts,
        ..
    } = import;
    let mut folder_id = None;
    let mut up = up;
    if made_folders
        && let Ok(conn) = up.conn().await
        && sync::refresh_folders(state, &account, conn).await.is_ok()
    {
        folder_id = state
            .store
            .list_folders(&account.id)?
            .into_iter()
            .find(|f| f.path == top)
            .map(|f| f.id);
    }
    up.finish(ctx).await;
    crate::contacts::soon(ctx, state);
    crate::calendar::changed(ctx, &[]);
    events::changed(ctx, &account.id, &[]);
    if let Err(error) = result {
        if report.imported == 0 {
            return Err(error);
        }
        report.problem(format!("The import stopped early: {}", error.message));
    }
    for damage in file.damage().into_iter().take(3) {
        report.problem(format!(
            "Part of the file is damaged and couldn't be read ({damage})."
        ));
    }
    progress.report("Done", 1, 1);
    report.counts = Some(counts);
    report.folder_id = folder_id;
    Ok(Some(report))
}

async fn run(
    file: &mut PstFile<BufReader<File>>,
    pst_folders: &[PstFolder],
    import: &mut Import<'_>,
    progress: &mut Progress,
) -> AppResult<()> {
    for folder in pst_folders {
        let (items, past_cap) = match file.items(folder) {
            Ok(list) => list,
            Err(why) => {
                import.report.problem(format!(
                    "The folder {} couldn't be read: {why}",
                    folder.path.join("/")
                ));
                continue;
            }
        };
        for _ in 0..past_cap {
            import.report.skip();
        }
        if past_cap > 0 {
            import.report.problem(format!(
                "{} holds more items than can be read at once; {past_cap} were left out.",
                folder.path.join("/")
            ));
        }
        let name = folder
            .path
            .last()
            .cloned()
            .unwrap_or_else(|| "Outlook".to_owned());
        let total = u64::try_from(items.len()).unwrap_or(u64::MAX);
        for (i, item) in items.iter().enumerate() {
            progress.report(
                &format!("Importing {name}"),
                u64::try_from(i).unwrap_or(u64::MAX),
                total,
            );
            match item.kind {
                ItemKind::Mail => import.mail(file, folder, item).await?,
                ItemKind::Contact => import.contact(file, *item)?,
                ItemKind::ContactGroup => {
                    import.report.skip();
                    import.report.problem("Contact groups aren't brought in.");
                }
                ItemKind::Event => import.event(file, item).await?,
                ItemKind::Task | ItemKind::Note | ItemKind::Journal => import.later(item.kind),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_and_paths() {
        assert_eq!(safe_name("Projects / 2026", Some("/")), "Projects - 2026");
        assert_eq!(safe_name("  ", Some(".")), "Folder");
        assert_eq!(safe_name("a*b\"c", None), "a-b-c");
        let paths = Paths {
            top: "Imported from Outlook".into(),
            delimiter: Some("/".into()),
        };
        let path = vec!["Inbox".to_owned(), "Kl\u{e4}r".to_owned()];
        assert_eq!(paths.of(&path), "Imported from Outlook/Inbox/Kl&AOQ-r");
        assert_eq!(
            paths.chain(&path),
            [
                "Imported from Outlook",
                "Imported from Outlook/Inbox",
                "Imported from Outlook/Inbox/Kl&AOQ-r"
            ]
        );
        let flat = Paths {
            top: "Imported from Outlook".into(),
            delimiter: None,
        };
        assert_eq!(flat.of(&path), "Imported from Outlook - Inbox - Kl&AOQ-r");
    }
}
