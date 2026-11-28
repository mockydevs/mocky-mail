//! Keeping rules on the mail server (`keepOnServer`), so they run while
//! every device is off: a Sieve script, `mocky-mail-rules`, written over
//! `ManageSieve` like the out-of-office reply (`out_of_office.rs`).
//!
//! A server runs one script. When the out-of-office script (`mocky-mail`)
//! is the one running, it runs the rules through `include`; a script that
//! was running before (webmail filters) keeps running after the rules,
//! through `include` too. Without `include`, rules don't go on a server
//! that already runs another script, and say so.
//!
//! Writes happen in the background, retried a few times, and never hold
//! up the editor. Each account's outcome is kept in a setting
//! (`rules_server:<account id>`); the rules' `serverStatus` and
//! `serverNote` are worked out from those, and `mail://rules-changed`
//! tells the interface.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mail_core::folders::{FolderRole, decode_folder_name};
use mail_core::rules::{
    ActionKind, RULES_SCRIPT, Rule, RuleSpec, ServerStatus, SieveRule, sieve_previous,
    sieve_script, sieve_unsupported,
};
use mail_core::vacation;
use mail_store::{Account, AuthKind, Folder};
use mail_sync::sieve::SieveClient;
use serde::{Deserialize, Serialize};

use crate::Ctx;
use crate::error::AppResult;
use crate::events;
use crate::out_of_office::{NoSieve, open_sieve};
use crate::rules::{applies_to, role_name};
use crate::state::{AppState, lock};

const GMAIL_NOTE: &str =
    "Gmail and Outlook keep their own filters, so this rule runs on this device.";
const PENDING_NOTE: &str = "Being saved to the mail server.";
const FAILED_NOTE: &str = "Couldn't save it to the mail server, so this rule runs on this device for now. Mocky Mail tries again later.";
const OFFLINE_NOTE: &str =
    "You're working offline. This rule is saved to the mail server when you're back online.";
const OFF_NOTE: &str = "Turned off, so it isn't on the mail server.";
const NO_PASSWORD_NOTE: &str = "This account has no password to sign in to the server's filters with, so this rule runs on this device.";
const NOT_OFFERED_NOTE: &str = "The mail server doesn't take filters from apps (ManageSieve), so this rule runs on this device.";
const VACATION_NOTE: &str = "Automatic replies are on, and the mail server can't run rules alongside them, so this rule runs on this device.";
const FOREIGN_NOTE: &str = "Other filters already run on the mail server, and it can't run these rules alongside them, so this rule runs on this device.";

/// What every Sieve server is assumed to have, to say early which rules
/// can't go on any server (body conditions, categories and so on).
const COMMON_EXTENSIONS: [&str; 3] = ["fileinto", "copy", "imap4flags"];

/// Waits before each attempt to write an account's rules.
const ATTEMPTS: [Duration; 4] = [
    Duration::ZERO,
    Duration::from_secs(20),
    Duration::from_mins(2),
    Duration::from_mins(10),
];

/// One write at a time, so two never interleave on a server.
static WRITING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// The newest write asked for per account; older ones give way.
static GENERATIONS: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());

/// Where the last write of an account's rules got to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Write {
    /// Written (or nothing to write).
    #[default]
    Done,
    /// A write is wanted.
    Pending,
    /// The last write gave up; it is tried again later.
    Failed,
    /// Working offline when it was tried.
    Offline,
}

/// How one account's rules stand on its server.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Record {
    /// The last write.
    write: Write,
    /// Our script runs on the server now (on its own or included).
    on_server: bool,
    /// Each rule's outcome on this server.
    rules: HashMap<String, (ServerStatus, Option<String>)>,
}

fn key(account_id: &str) -> String {
    format!("rules_server:{account_id}")
}

fn load(state: &AppState, account_id: &str) -> Record {
    state
        .store
        .get_setting(&key(account_id))
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn save(state: &AppState, account_id: &str, record: &Record) -> AppResult<()> {
    let json = serde_json::to_string(record).unwrap_or_default();
    state.store.set_setting(&key(account_id), &json)?;
    Ok(())
}

/// Accounts with a Sieve server to try (password sign-in, not Gmail).
fn uses_sieve(account: &Account) -> bool {
    account.auth == AuthKind::Password && !account.is_gmail()
}

fn common_extensions() -> Vec<String> {
    COMMON_EXTENSIONS.iter().map(|e| (*e).to_owned()).collect()
}

/// A rule's status from each account's outcome.
fn status_of(
    rule: &Rule,
    accounts: &[Account],
    records: &HashMap<String, Record>,
) -> (ServerStatus, Option<String>) {
    if !rule.spec.keep_on_server {
        return (ServerStatus::Local, None);
    }
    if !rule.spec.enabled {
        return (ServerStatus::Local, Some(OFF_NOTE.to_owned()));
    }
    if let Some(why) = sieve_unsupported(&rule.spec, &common_extensions()) {
        return (ServerStatus::Unsupported, Some(why));
    }
    let mut on_server: Vec<&str> = Vec::new();
    let mut other: Option<(ServerStatus, Option<String>)> = None;
    for account in accounts.iter().filter(|a| applies_to(&rule.spec, &a.id)) {
        let outcome = if uses_sieve(account) {
            let record = records.get(&account.id).cloned().unwrap_or_default();
            let note = |n: &str| (ServerStatus::Local, Some(n.to_owned()));
            match record.write {
                Write::Offline => note(OFFLINE_NOTE),
                Write::Failed => note(FAILED_NOTE),
                Write::Pending => note(PENDING_NOTE),
                Write::Done => record
                    .rules
                    .get(&rule.id)
                    .cloned()
                    .unwrap_or_else(|| note(PENDING_NOTE)),
            }
        } else {
            (ServerStatus::Unsupported, Some(GMAIL_NOTE.to_owned()))
        };
        if outcome.0 == ServerStatus::Server {
            on_server.push(&account.email);
        } else if other.is_none() {
            other = Some(outcome);
        }
    }
    match (on_server.is_empty(), other) {
        (true, None) => (ServerStatus::Local, None),
        (false, None) => (ServerStatus::Server, None),
        (true, Some(outcome)) => outcome,
        (false, Some((_, note))) => (
            ServerStatus::Server,
            Some(format!(
                "On the mail server for {}. Your other accounts: {}",
                on_server.join(", "),
                note.unwrap_or_default()
            )),
        ),
    }
}

/// Work out every rule's server status again. Returns whether any changed.
fn refresh_statuses(state: &AppState) -> AppResult<bool> {
    let accounts = state.store.list_accounts()?;
    let records: HashMap<String, Record> = accounts
        .iter()
        .filter(|a| uses_sieve(a))
        .map(|a| (a.id.clone(), load(state, &a.id)))
        .collect();
    let mut changed = false;
    for rule in state.store.list_rules()? {
        let (status, note) = status_of(&rule, &accounts, &records);
        if status != rule.server_status || note != rule.server_note {
            state
                .store
                .set_rule_server_status(&rule.id, status, note.as_deref())?;
            changed = true;
        }
    }
    Ok(changed)
}

fn app_state(ctx: &Ctx) -> Arc<AppState> {
    Arc::clone(&ctx.state)
}

/// Write these accounts' rules to their servers in the background, and
/// update every rule's status now.
pub fn schedule(ctx: &Ctx, account_ids: Vec<String>) {
    let state = app_state(ctx);
    let accounts = state.store.list_accounts().unwrap_or_default();
    let mut ids = account_ids;
    ids.sort();
    ids.dedup();
    for id in ids {
        let Some(account) = accounts.iter().find(|a| a.id == id) else {
            continue;
        };
        if !uses_sieve(account) {
            continue;
        }
        let mut record = load(&state, &id);
        record.write = Write::Pending;
        if let Err(error) = save(&state, &id, &record) {
            eprintln!("couldn't note a rules write: {}", error.message);
        }
        let generation = {
            let mut generations = lock(&GENERATIONS);
            let next = generations.get(&id).copied().unwrap_or(0).wrapping_add(1);
            generations.insert(id.clone(), next);
            next
        };
        ctx.spawn(run(ctx.clone(), Arc::clone(&state), id, generation));
    }
    match refresh_statuses(&state) {
        Ok(true) => events::rules_changed(ctx, &state),
        Ok(false) => {}
        Err(error) => eprintln!("couldn't update rule statuses: {}", error.message),
    }
}

/// Whether an account's rules still need writing: a write gave up, or
/// rules for it were never written (a new account, an earlier crash).
fn needs_write(state: &AppState, account: &Account, rules: &[Rule], starting: bool) -> bool {
    if !uses_sieve(account) {
        return false;
    }
    let record = load(state, &account.id);
    if matches!(record.write, Write::Failed | Write::Offline)
        || (starting && record.write == Write::Pending)
    {
        return true;
    }
    record.write == Write::Done
        && rules.iter().any(|r| {
            r.spec.keep_on_server
                && applies_to(&r.spec, &account.id)
                && !record.rules.contains_key(&r.id)
        })
}

/// At start-up: finish writes that didn't happen, and check the others.
pub fn start(ctx: &Ctx) {
    let state = app_state(ctx);
    let (Ok(accounts), Ok(rules)) = (state.store.list_accounts(), state.store.list_rules()) else {
        return;
    };
    let due: Vec<String> = accounts
        .iter()
        .filter(|a| {
            needs_write(&state, a, &rules, true)
                || (uses_sieve(a)
                    && rules
                        .iter()
                        .any(|r| r.spec.keep_on_server && applies_to(&r.spec, &a.id)))
        })
        .map(|a| a.id.clone())
        .collect();
    schedule(ctx, due);
}

/// After an account synced: try again a write that gave up (or was made
/// while working offline), and write rules for a newly added account.
pub fn retry_if_due(ctx: &Ctx, account: &Account) {
    let state = app_state(ctx);
    let Ok(rules) = state.store.list_rules() else {
        return;
    };
    if needs_write(&state, account, &rules, false) {
        schedule(ctx, vec![account.id.clone()]);
    }
}

fn current(account_id: &str, generation: u64) -> bool {
    lock(&GENERATIONS).get(account_id).copied() == Some(generation)
}

async fn run(ctx: Ctx, state: Arc<AppState>, account_id: String, generation: u64) {
    let mut gave_up = true;
    for wait in ATTEMPTS {
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        if !current(&account_id, generation) {
            return;
        }
        if crate::settings::working_offline(&state) {
            let mut record = load(&state, &account_id);
            record.write = Write::Offline;
            let _ = save(&state, &account_id, &record);
            gave_up = false;
            break;
        }
        let _writing = WRITING.lock().await;
        if !current(&account_id, generation) {
            return;
        }
        match write_account(&state, &account_id).await {
            Ok(Some((statuses, on_server))) => {
                let record = Record {
                    write: Write::Done,
                    on_server,
                    rules: statuses,
                };
                let _ = save(&state, &account_id, &record);
                gave_up = false;
                break;
            }
            // The account was removed.
            Ok(None) => return,
            Err(error) => eprintln!("couldn't write mail rules to the server: {}", error.message),
        }
    }
    if gave_up {
        let mut record = load(&state, &account_id);
        record.write = Write::Failed;
        let _ = save(&state, &account_id, &record);
    }
    match refresh_statuses(&state) {
        Ok(true) => events::rules_changed(&ctx, &state),
        Ok(false) => {}
        Err(error) => eprintln!("couldn't update rule statuses: {}", error.message),
    }
}

type Outcomes = HashMap<String, (ServerStatus, Option<String>)>;

/// Write one account's rules; `None` when the account is gone. Returns each
/// rule's outcome and whether our script now runs there.
async fn write_account(state: &AppState, account_id: &str) -> AppResult<Option<(Outcomes, bool)>> {
    let Ok(account) = state.store.get_account(account_id) else {
        return Ok(None);
    };
    let all = state.store.list_rules()?;
    let mine: Vec<&Rule> = all
        .iter()
        .filter(|r| r.spec.keep_on_server && applies_to(&r.spec, &account.id))
        .collect();
    let mut outcomes = Outcomes::new();
    // Nothing to put there and nothing there to take away.
    if mine.is_empty() && !load(state, account_id).on_server {
        return Ok(Some((outcomes, false)));
    }
    let mut client = match open_sieve(state, &account).await? {
        Ok(client) => client,
        Err(why) => {
            let note = match why {
                NoSieve::NotPassword => GMAIL_NOTE,
                NoSieve::NoPassword => NO_PASSWORD_NOTE,
                NoSieve::NotOffered => NOT_OFFERED_NOTE,
            };
            for rule in mine {
                outcomes.insert(
                    rule.id.clone(),
                    (ServerStatus::Unsupported, Some(note.to_owned())),
                );
            }
            return Ok(Some((outcomes, false)));
        }
    };
    let folders = state.store.list_folders(&account.id)?;
    let result = write_with(&account, &mine, &folders, &mut client, &mut outcomes).await;
    client.logout().await;
    Ok(Some((outcomes, result?)))
}

/// The UTF-8 name Sieve files into for a role or folder of the account.
fn mailbox(
    folders: &[Folder],
    folder_id: Option<&str>,
    role: Option<FolderRole>,
) -> Option<String> {
    folders
        .iter()
        .find(|f| match (folder_id, role) {
            (Some(id), _) => f.id == id,
            (None, Some(role)) => f.role == role,
            (None, None) => false,
        })
        .map(|f| decode_folder_name(&f.path))
}

/// Each action's mailbox, or why the account can't file there.
fn mailboxes(spec: &RuleSpec, folders: &[Folder]) -> Result<Vec<Option<String>>, String> {
    spec.actions
        .iter()
        .map(|a| match a.kind {
            ActionKind::Move | ActionKind::Copy => {
                mailbox(folders, a.folder_id.as_deref(), a.role)
                    .map(Some)
                    .ok_or_else(|| {
                        format!(
                            "This account has no {} folder on the server, so this rule runs on this device.",
                            a.role.map_or("such", role_name)
                        )
                    })
            }
            ActionKind::Delete => mailbox(folders, None, Some(FolderRole::Trash))
                .map(Some)
                .ok_or_else(|| {
                    "This account has no Trash folder on the server, so this rule runs on this device."
                        .to_owned()
                }),
            _ => Ok(None),
        })
        .collect()
}

/// The rules this server can run, with their mailboxes; the others get
/// their outcome (off, or why not) in `outcomes`.
fn usable_rules<'a>(
    mine: &[&'a Rule],
    folders: &[Folder],
    extensions: &[String],
    outcomes: &mut Outcomes,
) -> Vec<(&'a Rule, Vec<Option<String>>)> {
    let mut usable = Vec::new();
    for rule in mine {
        if !rule.spec.enabled {
            outcomes.insert(
                rule.id.clone(),
                (ServerStatus::Local, Some(OFF_NOTE.to_owned())),
            );
            continue;
        }
        if let Some(why) = sieve_unsupported(&rule.spec, extensions) {
            outcomes.insert(rule.id.clone(), (ServerStatus::Unsupported, Some(why)));
            continue;
        }
        match mailboxes(&rule.spec, folders) {
            Ok(boxes) => usable.push((*rule, boxes)),
            Err(why) => {
                outcomes.insert(rule.id.clone(), (ServerStatus::Unsupported, Some(why)));
            }
        }
    }
    usable
}

/// Put the script on the server and make it run, keeping what ran before.
async fn write_with(
    account: &Account,
    mine: &[&Rule],
    folders: &[Folder],
    client: &mut SieveClient,
    outcomes: &mut Outcomes,
) -> AppResult<bool> {
    let extensions = client.extensions.clone();
    let mut usable = usable_rules(mine, folders, &extensions, outcomes);
    let scripts = client.list().await?;
    let active = scripts.iter().find(|s| s.active).map(|s| s.name.clone());
    let ours = if scripts.iter().any(|s| s.name == RULES_SCRIPT) {
        Some(client.get(RULES_SCRIPT).await?)
    } else {
        None
    };
    let our_previous = ours.as_deref().and_then(sieve_previous);
    // What ran before ours, and the out-of-office reply if it is running.
    let mut away: Option<(vacation::Vacation, Option<String>)> = None;
    let foreign = match active.as_deref() {
        None => None,
        Some(RULES_SCRIPT) => our_previous,
        Some(vacation::SCRIPT_NAME) => {
            match vacation::read(&client.get(vacation::SCRIPT_NAME).await?) {
                Some((settings, previous)) => {
                    let foreign = if previous.as_deref() == Some(RULES_SCRIPT) {
                        our_previous
                    } else {
                        previous.clone()
                    };
                    away = Some((settings, previous));
                    foreign
                }
                None => Some(vacation::SCRIPT_NAME.to_owned()),
            }
        }
        Some(other) => Some(other.to_owned()),
    };
    // Never our own script (a damaged mark): it would include itself.
    let foreign = foreign.filter(|f| f != RULES_SCRIPT);
    let include = extensions.iter().any(|e| e == "include");
    if !usable.is_empty() && (foreign.is_some() || away.is_some()) && !include {
        let note = if away.is_some() {
            VACATION_NOTE
        } else {
            FOREIGN_NOTE
        };
        for (rule, _) in usable.drain(..) {
            outcomes.insert(
                rule.id.clone(),
                (ServerStatus::Unsupported, Some(note.to_owned())),
            );
        }
    }
    let addresses = std::slice::from_ref(&account.email);
    if usable.is_empty() {
        // Nothing of ours should run: put back what ran before.
        if active.as_deref() == Some(RULES_SCRIPT) {
            client.set_active(foreign.as_deref().unwrap_or("")).await?;
        }
        if let Some((settings, previous)) = &away
            && previous.as_deref() == Some(RULES_SCRIPT)
        {
            let text = vacation::script(settings, addresses, foreign.as_deref());
            client.put(vacation::SCRIPT_NAME, &text).await?;
        }
        return Ok(false);
    }
    let own = vec![account.email.trim().to_lowercase()];
    let sieve_rules: Vec<SieveRule<'_>> = usable
        .iter()
        .map(|(rule, boxes)| SieveRule {
            spec: &rule.spec,
            mailboxes: boxes.clone(),
        })
        .collect();
    let text = sieve_script(&sieve_rules, &own, foreign.as_deref());
    if ours.as_deref() != Some(text.as_str()) {
        client.put(RULES_SCRIPT, &text).await?;
    }
    match &away {
        Some((settings, previous)) => {
            if previous.as_deref() != Some(RULES_SCRIPT) {
                let text = vacation::script(settings, addresses, Some(RULES_SCRIPT));
                client.put(vacation::SCRIPT_NAME, &text).await?;
            }
        }
        None => {
            if active.as_deref() != Some(RULES_SCRIPT) {
                client.set_active(RULES_SCRIPT).await?;
            }
        }
    }
    for (rule, _) in usable {
        outcomes.insert(rule.id.clone(), (ServerStatus::Server, None));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::rules::{ConditionKind, Match, RuleAction, RuleCondition};
    use mail_store::{Security, ServerConfig};

    fn account(id: &str, auth: AuthKind, host: &str) -> Account {
        let server = ServerConfig {
            host: host.into(),
            port: 993,
            security: Security::Tls,
        };
        Account {
            id: id.into(),
            email: format!("{id}@example.com"),
            display_name: String::new(),
            auth,
            color: "#000000".into(),
            imap: server.clone(),
            smtp: server,
            size_limit_bytes: None,
            created_at: 0,
            sort_order: 0,
            sync_months: None,
        }
    }

    fn rule(account: Option<&str>) -> Rule {
        Rule {
            id: "r".into(),
            spec: RuleSpec {
                name: "r".into(),
                enabled: true,
                account_id: account.map(str::to_owned),
                match_kind: Match::All,
                conditions: vec![RuleCondition {
                    kind: ConditionKind::From,
                    value: "x".into(),
                }],
                exceptions: vec![],
                actions: vec![RuleAction {
                    kind: ActionKind::Flag,
                    folder_id: None,
                    role: None,
                    category: None,
                    importance: None,
                }],
                stop_processing: false,
                keep_on_server: true,
            },
            server_status: ServerStatus::Local,
            server_note: None,
            order: 0,
        }
    }

    fn done(status: ServerStatus) -> Record {
        Record {
            rules: [("r".to_owned(), (status, None))].into_iter().collect(),
            ..Record::default()
        }
    }

    #[test]
    fn statuses_come_from_each_accounts_outcome() {
        let own = account("own", AuthKind::Password, "mail.mocky.co.ke");
        let gmail = account("g", AuthKind::Google, "imap.gmail.com");
        let gmail_password = account("gp", AuthKind::Password, "imap.gmail.com");
        let accounts = vec![own.clone(), gmail.clone(), gmail_password];
        let mut records = HashMap::new();
        // Not kept on the server: local, no note.
        let mut local = rule(Some("own"));
        local.spec.keep_on_server = false;
        assert_eq!(
            status_of(&local, &accounts, &records),
            (ServerStatus::Local, None)
        );
        // Waiting for the write.
        let r = rule(Some("own"));
        assert_eq!(
            status_of(&r, &accounts, &records),
            (ServerStatus::Local, Some(PENDING_NOTE.to_owned()))
        );
        records.insert("own".to_owned(), done(ServerStatus::Server));
        assert_eq!(
            status_of(&r, &accounts, &records),
            (ServerStatus::Server, None)
        );
        // Gmail keeps its own filters, whichever way it signs in.
        let g = rule(Some("g"));
        assert_eq!(
            status_of(&g, &accounts, &records),
            (ServerStatus::Unsupported, Some(GMAIL_NOTE.to_owned()))
        );
        let gp = rule(Some("gp"));
        assert_eq!(
            status_of(&gp, &accounts, &records).0,
            ServerStatus::Unsupported
        );
        // Every account: on the server where it could go, with a note.
        let every = rule(None);
        let (status, note) = status_of(&every, &accounts, &records);
        assert_eq!(status, ServerStatus::Server);
        let note = note.unwrap_or_default();
        assert!(
            note.starts_with("On the mail server for own@example.com."),
            "{note}"
        );
        assert!(note.contains("Gmail"), "{note}");
        // Gave up: runs here, will try again.
        records.insert(
            "own".to_owned(),
            Record {
                write: Write::Failed,
                ..Record::default()
            },
        );
        assert_eq!(
            status_of(&r, &accounts, &records),
            (ServerStatus::Local, Some(FAILED_NOTE.to_owned()))
        );
        // Something no server can do, and a rule that is off.
        let mut body = rule(Some("own"));
        body.spec.conditions[0].kind = ConditionKind::Body;
        assert_eq!(
            status_of(&body, &accounts, &records).0,
            ServerStatus::Unsupported
        );
        let mut off = rule(Some("own"));
        off.spec.enabled = false;
        assert_eq!(
            status_of(&off, &accounts, &records),
            (ServerStatus::Local, Some(OFF_NOTE.to_owned()))
        );
        assert!(uses_sieve(&own));
        assert!(!uses_sieve(&gmail));
    }

    #[test]
    fn mailboxes_are_the_folders_utf8_names() {
        let folder = |id: &str, path: &str, role: FolderRole| Folder {
            id: id.into(),
            account_id: "a".into(),
            path: path.into(),
            name: path.into(),
            role,
            delimiter: Some(".".into()),
            uidvalidity: None,
            highest_uid: None,
            unread: 0,
            total: 0,
            favorite: false,
        };
        let folders = vec![
            folder("t", "INBOX.Trash", FolderRole::Trash),
            folder("e", "INBOX.Entw&APw-rfe", FolderRole::Other),
        ];
        let mut spec = rule(Some("a")).spec;
        spec.actions = vec![
            RuleAction {
                kind: ActionKind::Move,
                folder_id: Some("e".into()),
                role: None,
                category: None,
                importance: None,
            },
            RuleAction {
                kind: ActionKind::Delete,
                folder_id: None,
                role: None,
                category: None,
                importance: None,
            },
            RuleAction {
                kind: ActionKind::Flag,
                folder_id: None,
                role: None,
                category: None,
                importance: None,
            },
        ];
        assert_eq!(
            mailboxes(&spec, &folders),
            Ok(vec![
                Some("INBOX.Entwürfe".to_owned()),
                Some("INBOX.Trash".to_owned()),
                None
            ])
        );
        spec.actions[0].folder_id = None;
        spec.actions[0].role = Some(FolderRole::Archive);
        assert!(
            mailboxes(&spec, &folders)
                .unwrap_err()
                .contains("no Archive folder")
        );
    }
}
