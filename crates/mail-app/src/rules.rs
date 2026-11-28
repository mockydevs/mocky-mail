//! Mail rules (`docs/ipc.md`, "Rules"): the commands, running rules on new
//! mail and on a folder, the search box's query language, and the senders
//! whose pictures always load.

use std::collections::{HashMap, HashSet};

use mail_core::folders::FolderRole;
use mail_core::rules::{
    self, ActionKind, ConditionKind, Plan, Rule, RuleAction, RuleInput, RuleSpec, ServerStatus,
    Target,
};
use mail_core::search::{ParsedSearch, SearchContext};
use mail_store::{Account, ImageSender, RuleSubject, image_sender_key};
use serde::Serialize;

use crate::Ctx;
use crate::error::{AppError, AppResult};
use crate::mail::{MoveTarget, move_groups, notify_changed, set_seen_flagged};
use crate::organize::{change_categories, copy_groups};
use crate::state::AppState;
use crate::{events, server_rules, sync};

/// Most messages "run now" and the editor's preview look at: the folder's
/// newest.
pub const MAX_RUN: u32 = 5000;
/// Most messages one action touches at a time.
const BATCH: usize = 1000;
/// Most notifications one run shows; beyond that, one that counts them.
const MAX_NOTIFICATIONS: usize = 3;
/// Longest search box text read.
const MAX_QUERY_CHARS: usize = 2000;

/// The local day, for `before:`, `after:`, `today` and `yesterday`.
#[must_use]
pub fn search_context() -> SearchContext {
    let now = chrono::Local::now();
    SearchContext {
        today: now.date_naive(),
        utc_offset_secs: now.offset().local_minus_utc(),
    }
}

/// `parse_search`: what a search box query means, for chips and help.
#[must_use]
pub fn parse_search(query: String) -> ParsedSearch {
    let query: String = query.chars().take(MAX_QUERY_CHARS).collect();
    mail_core::search::parse(&query, search_context())
}

/// The result of running rules now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleRunResult {
    /// Messages looked at.
    checked: u32,
    /// Messages at least one rule matched.
    matched: u32,
}

/// Whether a rule is for this account (or every account).
pub(crate) fn applies_to(spec: &RuleSpec, account_id: &str) -> bool {
    spec.account_id.as_deref().is_none_or(|a| a == account_id)
}

/// The enabled rules for an account, in order.
fn enabled_for<'a>(all: &'a [Rule], account_id: &str) -> Vec<&'a Rule> {
    all.iter()
        .filter(|r| r.spec.enabled && applies_to(&r.spec, account_id))
        .collect()
}

fn rule_gone() -> AppError {
    AppError::new("not_found", "That rule no longer exists.")
}

/// Check what only the database knows: the account and folders exist and
/// belong together, and categories exist (spelled as they are stored).
fn check_against_store(state: &AppState, mut spec: RuleSpec) -> AppResult<RuleSpec> {
    let account = spec.account_id.clone();
    if let Some(account) = &account {
        state
            .store
            .get_account(account)
            .map_err(|_| AppError::invalid("That account no longer exists."))?;
    }
    let categories = state.store.list_categories()?;
    let spelled = |name: &str| -> AppResult<String> {
        categories
            .iter()
            .find(|c| mail_core::categories::same_name(&c.name, name))
            .map(|c| c.name.clone())
            .ok_or_else(|| {
                AppError::invalid(format!(
                    "There's no category called {name}. Add it first, then choose it."
                ))
            })
    };
    for action in &mut spec.actions {
        if let Some(folder_id) = &action.folder_id {
            let folder = state
                .store
                .get_folder(folder_id)
                .map_err(|_| AppError::invalid("That folder no longer exists. Choose another."))?;
            if account.as_deref() != Some(folder.account_id.as_str()) {
                return Err(AppError::invalid(
                    "Choose a folder in the account the rule is for.",
                ));
            }
        }
        if let Some(name) = &action.category {
            action.category = Some(spelled(name)?);
        }
    }
    for condition in spec.conditions.iter_mut().chain(spec.exceptions.iter_mut()) {
        if condition.kind == ConditionKind::Category {
            condition.value = spelled(&condition.value)?;
        }
    }
    Ok(spec)
}

/// Accounts whose server script a rule is (or was) in.
fn server_accounts(state: &AppState, spec: &RuleSpec) -> AppResult<Vec<String>> {
    if !spec.keep_on_server {
        return Ok(Vec::new());
    }
    Ok(match &spec.account_id {
        Some(account) => vec![account.clone()],
        None => state
            .store
            .list_accounts()?
            .into_iter()
            .map(|a| a.id)
            .collect(),
    })
}

/// `list_rules`.
pub fn list_rules(ctx: &Ctx) -> AppResult<Vec<Rule>> {
    let state = &ctx.state;
    Ok(state.store.list_rules()?)
}

/// `save_rule`: add (id null) or change a rule. Writing it to the mail
/// server happens in the background (`mail://rules-changed` when done).
pub fn save_rule(ctx: &Ctx, rule: RuleInput) -> AppResult<Vec<Rule>> {
    let state = &ctx.state;
    let spec = rules::checked(&rule.spec).map_err(AppError::invalid)?;
    let spec = check_against_store(state, spec)?;
    let previous = match rule.id.as_deref() {
        Some(id) => Some(state.store.get_rule(id).map_err(|_| rule_gone())?),
        None => None,
    };
    let mut accounts = server_accounts(state, &spec)?;
    if let Some(previous) = &previous {
        accounts.extend(server_accounts(state, &previous.spec)?);
    }
    state
        .store
        .save_rule(rule.id.as_deref(), &spec, ServerStatus::Local, None)
        .map_err(|error| match error {
            mail_store::StoreError::NotFound("rule") => rule_gone(),
            other => other.into(),
        })?;
    server_rules::schedule(ctx, accounts);
    Ok(state.store.list_rules()?)
}

/// `delete_rule`.
pub fn delete_rule(ctx: &Ctx, id: String) -> AppResult<Vec<Rule>> {
    let state = &ctx.state;
    let rule = state.store.get_rule(&id).map_err(|_| rule_gone())?;
    state.store.delete_rule(&id)?;
    server_rules::schedule(ctx, server_accounts(state, &rule.spec)?);
    Ok(state.store.list_rules()?)
}

/// `reorder_rules`: every rule id, in the new order.
pub fn reorder_rules(ctx: &Ctx, ids: Vec<String>) -> AppResult<Vec<Rule>> {
    let state = &ctx.state;
    state
        .store
        .reorder_rules(&ids)
        .map_err(|error| match error {
            mail_store::StoreError::Invalid(_) => {
                AppError::invalid("List every rule exactly once to reorder them.")
            }
            other => other.into(),
        })?;
    let rules = state.store.list_rules()?;
    let mut accounts = Vec::new();
    for rule in &rules {
        accounts.extend(server_accounts(state, &rule.spec)?);
    }
    server_rules::schedule(ctx, accounts);
    Ok(state.store.list_rules()?)
}

/// `run_rules`: run rules now on a folder's newest [`MAX_RUN`] messages.
/// `rule_ids` `None`: every enabled rule of the folder's account.
pub fn run_rules(
    ctx: &Ctx,
    rule_ids: Option<Vec<String>>,
    folder_id: String,
) -> AppResult<RuleRunResult> {
    let state = &ctx.state;
    let folder = state.store.get_folder(&folder_id)?;
    let all = state.store.list_rules()?;
    let chosen: Vec<&Rule> = match &rule_ids {
        None => enabled_for(&all, &folder.account_id),
        Some(ids) => {
            if ids.iter().any(|id| !all.iter().any(|r| r.id == *id)) {
                return Err(rule_gone());
            }
            all.iter()
                .filter(|r| ids.contains(&r.id) && applies_to(&r.spec, &folder.account_id))
                .collect()
        }
    };
    if chosen.is_empty() {
        return Ok(RuleRunResult::default());
    }
    let subjects = state.store.folder_rule_subjects(
        &folder.id,
        MAX_RUN,
        chosen.iter().any(|r| rules::needs_text(&r.spec)),
        chosen.iter().any(|r| rules::needs_headers(&r.spec)),
    )?;
    let checked = u32::try_from(subjects.len()).unwrap_or(u32::MAX);
    let applied = apply(ctx, state, &chosen, subjects)?;
    Ok(RuleRunResult {
        checked,
        matched: applied.matched,
    })
}

/// `preview_rule`: how many of a folder's newest [`MAX_RUN`] messages an
/// unsaved rule's conditions and exceptions match (its actions, name and
/// account are not needed yet).
pub fn preview_rule(ctx: &Ctx, rule: RuleInput, folder_id: String) -> AppResult<u32> {
    let state = &ctx.state;
    let probe = RuleSpec {
        name: "Preview".to_owned(),
        account_id: None,
        actions: vec![RuleAction {
            kind: ActionKind::Flag,
            folder_id: None,
            role: None,
            category: None,
            importance: None,
        }],
        ..rule.spec
    };
    let spec = rules::checked(&probe).map_err(AppError::invalid)?;
    let folder = state.store.get_folder(&folder_id)?;
    let subjects = state.store.folder_rule_subjects(
        &folder.id,
        MAX_RUN,
        rules::needs_text(&spec),
        rules::needs_headers(&spec),
    )?;
    let n = subjects
        .iter()
        .filter(|s| rules::matches(&spec, &s.facts))
        .count();
    Ok(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Rules on mail that just arrived in an account's inbox: `new` is each
/// message id and whether it is unread. Returns how many of the unread
/// ones the rules moved out of the inbox or marked read (so the new-mail
/// notification leaves them out). Each message is looked at once, and
/// copies the rules make don't set them off again.
///
/// # Errors
/// Database errors.
pub fn on_new_mail(
    ctx: &Ctx,
    state: &AppState,
    account: &Account,
    new: &[(String, bool)],
) -> AppResult<u32> {
    let all = state.store.list_rules()?;
    let chosen = enabled_for(&all, &account.id);
    if chosen.is_empty() || new.is_empty() {
        return Ok(0);
    }
    let ids: Vec<&String> = new.iter().map(|(id, _)| id).collect();
    let claimed = state.store.claim_for_rules(&ids)?;
    let subjects = state.store.rule_subjects(
        &claimed,
        chosen.iter().any(|r| rules::needs_text(&r.spec)),
        chosen.iter().any(|r| rules::needs_headers(&r.spec)),
    )?;
    let applied = apply(ctx, state, &chosen, subjects)?;
    let handled = new
        .iter()
        .filter(|(id, unread)| *unread && applied.handled.contains(id))
        .count();
    Ok(u32::try_from(handled).unwrap_or(u32::MAX))
}

/// What a run did.
struct Applied {
    matched: u32,
    /// Messages moved away or marked read.
    handled: HashSet<String>,
}

fn move_target(target: &Target) -> MoveTarget {
    match target {
        Target::Folder(folder_id) => MoveTarget::Folder {
            folder_id: folder_id.clone(),
        },
        Target::Role(role) => MoveTarget::Role { role: *role },
    }
}

fn notification_line(s: &RuleSubject) -> (String, String) {
    let sender = s.facts.from.as_ref().map_or_else(
        || "Mail rules".to_owned(),
        |from| {
            from.name
                .clone()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| from.address.clone())
        },
    );
    let subject = if s.facts.subject.trim().is_empty() {
        "(no subject)".to_owned()
    } else {
        s.facts.subject.clone()
    };
    (sender, subject)
}

/// Work out and carry out what the rules do to these messages.
fn apply(
    ctx: &Ctx,
    state: &AppState,
    chosen: &[&Rule],
    subjects: Vec<RuleSubject>,
) -> AppResult<Applied> {
    let mut plans: HashMap<Plan, Vec<String>> = HashMap::new();
    let mut clears: Vec<(String, Vec<String>)> = Vec::new();
    let mut notes: Vec<(String, String)> = Vec::new();
    let mut handled = HashSet::new();
    let mut matched = 0u32;
    for subject in subjects {
        let rules: Vec<&Rule> = chosen
            .iter()
            .copied()
            .filter(|r| applies_to(&r.spec, &subject.account_id))
            .collect();
        let Some(plan) = rules::plan(&rules, &subject.facts) else {
            continue;
        };
        matched = matched.saturating_add(1);
        if plan.notify {
            notes.push(notification_line(&subject));
        }
        if plan.clear_categories && !subject.facts.categories.is_empty() {
            clears.push((subject.id.clone(), subject.facts.categories.clone()));
        }
        if plan.destination.is_some() || plan.seen == Some(true) {
            handled.insert(subject.id.clone());
        }
        plans.entry(plan).or_default().push(subject.id);
    }
    let mut touched: Vec<(String, String)> = Vec::new();
    // Each message has its own categories to take off.
    for (id, have) in clears {
        touched.extend(change_categories(state, &[id], &[], &have)?);
    }
    for (plan, ids) in &plans {
        for batch in ids.chunks(BATCH) {
            touched.extend(carry_out(state, plan, batch)?);
        }
    }
    touched.sort();
    touched.dedup();
    notify_changed(ctx, &touched);
    if notes.len() > MAX_NOTIFICATIONS {
        events::notify(
            ctx,
            "Mail rules",
            &format!("{} messages matched your rules.", notes.len()),
        );
    } else {
        for (title, body) in notes {
            events::notify(ctx, &title, &body);
        }
    }
    Ok(Applied { matched, handled })
}

/// One plan on a batch of messages: flags and marks first, then
/// categories, copies, and the move last (the changes before it go with
/// the message). Returns the (account, folder) pairs that changed.
fn carry_out(state: &AppState, plan: &Plan, ids: &[String]) -> AppResult<Vec<(String, String)>> {
    let ids = ids.to_vec();
    let mut touched: Vec<(String, String)> =
        sync::group_by_folder(state, &ids)?.into_keys().collect();
    if plan.seen.is_some() || plan.flagged.is_some() {
        set_seen_flagged(state, &ids, plan.seen, plan.flagged)?;
    }
    if plan.pin || plan.importance.is_some() {
        state
            .store
            .set_marks(&ids, plan.pin.then_some(true), plan.importance, None)?;
    }
    if !plan.add_categories.is_empty() {
        change_categories(state, &ids, &plan.add_categories, &[])?;
    }
    // A copy or move the account can't do (a folder gone since) must not
    // stop the rest.
    for copy in &plan.copies {
        if let Err(error) = copy_groups(state, &ids, &move_target(copy)) {
            eprintln!("a mail rule couldn't copy: {}", error.message);
        }
    }
    if let Some(destination) = &plan.destination {
        let groups = sync::group_by_folder(state, &ids)?;
        match move_groups(state, groups, &move_target(destination)) {
            Ok(moved) => touched.extend(moved),
            Err(error) => eprintln!("a mail rule couldn't move: {}", error.message),
        }
    }
    Ok(touched)
}

/// `list_image_senders`.
pub fn list_image_senders(ctx: &Ctx) -> AppResult<Vec<ImageSender>> {
    let state = &ctx.state;
    Ok(state.store.list_image_senders()?)
}

/// `trust_image_sender`: always load pictures from this address, or with
/// `wholeDomain` from anyone at its domain (and its subdomains).
pub fn trust_image_sender(
    ctx: &Ctx,
    sender: String,
    whole_domain: bool,
) -> AppResult<Vec<ImageSender>> {
    let state = &ctx.state;
    let key = image_sender_key(&sender, whole_domain).ok_or_else(|| {
        AppError::invalid(if whole_domain {
            "That isn't an email address or domain."
        } else {
            "That isn't an email address."
        })
    })?;
    state.store.trust_image_sender(&key, whole_domain)?;
    Ok(state.store.list_image_senders()?)
}

/// `untrust_image_sender`: as listed (address or domain).
pub fn untrust_image_sender(ctx: &Ctx, sender: String) -> AppResult<Vec<ImageSender>> {
    let state = &ctx.state;
    state.store.untrust_image_sender(&sender)?;
    Ok(state.store.list_image_senders()?)
}

/// The role a rule for every account files into, for messages.
pub(crate) const fn role_name(role: FolderRole) -> &'static str {
    match role {
        FolderRole::Inbox => "Inbox",
        FolderRole::Sent => "Sent",
        FolderRole::Drafts => "Drafts",
        FolderRole::Archive => "Archive",
        FolderRole::Junk => "Junk",
        FolderRole::Trash => "Trash",
        FolderRole::All => "All Mail",
        FolderRole::Flagged => "Flagged",
        FolderRole::Other => "folder",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(account: Option<&str>, enabled: bool) -> RuleSpec {
        RuleSpec {
            name: "r".into(),
            enabled,
            account_id: account.map(str::to_owned),
            match_kind: rules::Match::All,
            conditions: vec![],
            exceptions: vec![],
            actions: vec![],
            stop_processing: false,
            keep_on_server: false,
        }
    }

    #[test]
    fn rules_apply_to_their_account_or_every_account() {
        let all = [
            Rule {
                id: "1".into(),
                spec: spec(Some("a"), true),
                server_status: ServerStatus::Local,
                server_note: None,
                order: 0,
            },
            Rule {
                id: "2".into(),
                spec: spec(None, true),
                server_status: ServerStatus::Local,
                server_note: None,
                order: 1,
            },
            Rule {
                id: "3".into(),
                spec: spec(Some("a"), false),
                server_status: ServerStatus::Local,
                server_note: None,
                order: 2,
            },
        ];
        let ids = |account: &str| -> Vec<String> {
            enabled_for(&all, account)
                .into_iter()
                .map(|r| r.id.clone())
                .collect()
        };
        assert_eq!(ids("a"), ["1", "2"]);
        assert_eq!(ids("b"), ["2"]);
        assert_eq!(role_name(FolderRole::Archive), "Archive");
    }

    #[test]
    fn the_search_context_is_today_here() {
        let ctx = search_context();
        assert_eq!(ctx.today, chrono::Local::now().date_naive());
        assert_eq!(parse_search("from:x OR is:bogus".into()).errors.len(), 1);
    }
}
