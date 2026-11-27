//! Mail rules: conditions, exceptions and actions, checked against a
//! message, and written as a Sieve script (RFC 5228) for servers that run
//! them while every device is off.
//!
//! A rule matches when its conditions do (`all` of them, or `any`; no
//! conditions: every message) and none of its exceptions does. Text
//! conditions hold one or more comma-separated words or addresses; any of
//! them found anywhere in the field (ignoring case) is a match. Rules run
//! in their order; a matching rule with `stop_processing` ends the run for
//! that message. What all the matching rules do is gathered into one
//! [`Plan`], so a message is moved at most once (the first move or delete
//! wins) and later flag or importance actions override earlier ones.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::folders::FolderRole;
use crate::importance::Importance;
use crate::message::Mailbox;

/// The app's rules script on the server.
pub const RULES_SCRIPT: &str = "mocky-mail-rules";
const PREVIOUS_MARK: &str = "# mocky-mail-rules-previous: ";
const MAX_NAME: usize = 100;
const MAX_VALUE: usize = 500;
const MAX_CONDITIONS: usize = 30;
const MAX_ACTIONS: usize = 20;
/// Most header bytes read for `header` conditions.
const MAX_HEADER_BYTES: usize = 64 * 1024;

/// What a condition looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConditionKind {
    /// Sender name or address.
    From,
    /// To names or addresses.
    To,
    /// Cc names or addresses.
    Cc,
    /// To or Cc.
    ToOrCc,
    /// Subject.
    Subject,
    /// Message text.
    Body,
    /// Subject or text.
    SubjectOrBody,
    /// A header, written `Name: words`.
    Header,
    /// Has attachments.
    HasAttachment,
    /// `high`, `normal` or `low`.
    Importance,
    /// Bigger than this many kilobytes.
    SizeOver,
    /// Smaller than this many kilobytes.
    SizeUnder,
    /// Has this category.
    Category,
    /// The account's address is in To or Cc.
    ToMe,
    /// Sent to the account's address alone (To, no Cc).
    OnlyToMe,
}

impl ConditionKind {
    /// What the editor calls it, for messages.
    const fn label(self) -> &'static str {
        match self {
            Self::From => "From",
            Self::To => "To",
            Self::Cc => "Cc",
            Self::ToOrCc => "To or Cc",
            Self::Subject => "Subject",
            Self::Body => "Message text",
            Self::SubjectOrBody => "Subject or text",
            Self::Header => "Header",
            Self::HasAttachment => "Has attachment",
            Self::Importance => "Importance",
            Self::SizeOver => "Larger than",
            Self::SizeUnder => "Smaller than",
            Self::Category => "Category",
            Self::ToMe => "Sent to me",
            Self::OnlyToMe => "Sent only to me",
        }
    }

    const fn is_text(self) -> bool {
        matches!(
            self,
            Self::From
                | Self::To
                | Self::Cc
                | Self::ToOrCc
                | Self::Subject
                | Self::Body
                | Self::SubjectOrBody
        )
    }
}

/// One condition (or exception).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleCondition {
    /// What it looks at.
    pub kind: ConditionKind,
    /// What it looks for (see `docs/ipc.md`, Rules).
    #[serde(default)]
    pub value: String,
}

/// What an action does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionKind {
    /// Move to a folder.
    Move,
    /// Copy to a folder.
    Copy,
    /// Add a category.
    Categorize,
    /// Flag.
    Flag,
    /// Remove the flag.
    ClearFlag,
    /// Remove every category.
    ClearCategories,
    /// Mark as read.
    MarkRead,
    /// Set the importance (this device).
    SetImportance,
    /// Move to Trash.
    Delete,
    /// Show a desktop notification.
    Notify,
    /// Pin (this device).
    Pin,
}

/// One action.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleAction {
    /// What it does.
    pub kind: ActionKind,
    /// For move and copy: a folder of the rule's account.
    #[serde(default)]
    pub folder_id: Option<String>,
    /// For move and copy: the folder with this role (in each account).
    #[serde(default)]
    pub role: Option<FolderRole>,
    /// For categorize.
    #[serde(default)]
    pub category: Option<String>,
    /// For set importance.
    #[serde(default)]
    pub importance: Option<Importance>,
}

/// How conditions combine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Match {
    /// Every condition.
    #[default]
    All,
    /// At least one.
    Any,
}

/// What the user sets in a rule (the interface's `RuleInput` without `id`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleSpec {
    /// Shown in the list.
    pub name: String,
    /// Off rules don't run.
    pub enabled: bool,
    /// `None`: every account.
    pub account_id: Option<String>,
    /// All or any condition.
    #[serde(rename = "match", default)]
    pub match_kind: Match,
    /// What it looks for.
    #[serde(default)]
    pub conditions: Vec<RuleCondition>,
    /// A message matching any of these is skipped.
    #[serde(default)]
    pub exceptions: Vec<RuleCondition>,
    /// What it does.
    #[serde(default)]
    pub actions: Vec<RuleAction>,
    /// Later rules skip a message this one matched.
    #[serde(default)]
    pub stop_processing: bool,
    /// Also kept on the mail server (Sieve).
    #[serde(default)]
    pub keep_on_server: bool,
}

/// A rule sent by the editor (`RuleInput`): `id` is `None` for a new one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleInput {
    /// The rule to change, or `None` to add one.
    pub id: Option<String>,
    /// The rule.
    #[serde(flatten)]
    pub spec: RuleSpec,
}

/// Where a rule runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerStatus {
    /// On this device only.
    #[default]
    Local,
    /// Also on the mail server.
    Server,
    /// The server can't run it (see the note); it runs on this device.
    Unsupported,
}

impl ServerStatus {
    /// The stored and serialized name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Server => "server",
            Self::Unsupported => "unsupported",
        }
    }

    /// From [`ServerStatus::as_str`]; anything else is `Local`.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "server" => Self::Server,
            "unsupported" => Self::Unsupported,
            _ => Self::Local,
        }
    }
}

/// A saved rule (the interface's `Rule`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    /// Opaque id.
    pub id: String,
    /// What the user set.
    #[serde(flatten)]
    pub spec: RuleSpec,
    /// Where it runs.
    pub server_status: ServerStatus,
    /// Why, in plain words.
    pub server_note: Option<String>,
    /// Lower runs first.
    pub order: i64,
}

/// What a rule can see of one message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageFacts {
    /// Sender.
    pub from: Option<Mailbox>,
    /// To.
    pub to: Vec<Mailbox>,
    /// Cc.
    pub cc: Vec<Mailbox>,
    /// Subject.
    pub subject: String,
    /// The message text as downloaded (or the preview); `None` if neither.
    pub text: Option<String>,
    /// Header lines, when the message is downloaded (`header` conditions).
    pub headers: Option<Vec<(String, String)>>,
    /// Has attachments.
    pub has_attachments: bool,
    /// Importance.
    pub importance: Importance,
    /// Bytes.
    pub size: u64,
    /// Category names.
    pub categories: Vec<String>,
    /// The account's own addresses, lower case.
    pub own_addresses: Vec<String>,
}

/// Where to move or copy to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    /// This folder.
    Folder(String),
    /// The account's folder with this role.
    Role(FolderRole),
}

/// What the matching rules do to one message, gathered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Plan {
    /// Read state to set.
    pub seen: Option<bool>,
    /// Flag to set.
    pub flagged: Option<bool>,
    /// Remove every category first.
    pub clear_categories: bool,
    /// Categories to add.
    pub add_categories: Vec<String>,
    /// Importance to set.
    pub importance: Option<Importance>,
    /// Pin it.
    pub pin: bool,
    /// Show a notification.
    pub notify: bool,
    /// Copies to make.
    pub copies: Vec<Target>,
    /// Where it ends up.
    pub destination: Option<Target>,
}

/// The words of a text condition: comma-separated, lower case.
fn needles(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|n| n.trim().to_lowercase())
        .filter(|n| !n.is_empty())
        .collect()
}

fn contains_any(haystack: &str, needles: &[String]) -> bool {
    let haystack = haystack.to_lowercase();
    needles.iter().any(|n| haystack.contains(n.as_str()))
}

fn mailbox_text(m: &Mailbox) -> String {
    format!("{} <{}>", m.name.as_deref().unwrap_or(""), m.address)
}

fn any_mailbox(list: &[Mailbox], needles: &[String]) -> bool {
    list.iter().any(|m| contains_any(&mailbox_text(m), needles))
}

/// `Name: words` of a header condition: the name and the words (none:
/// the header just has to be there).
fn header_parts(value: &str) -> Option<(String, Vec<String>)> {
    let (name, words) = value.split_once(':').unwrap_or((value, ""));
    let name = name.trim();
    let ok = !name.is_empty()
        && name.len() <= 76
        && name.bytes().all(|b| b.is_ascii_graphic() && b != b':');
    ok.then(|| (name.to_owned(), needles(words)))
}

/// Kilobytes of a size condition.
fn kilobytes(value: &str) -> Option<u64> {
    let n: u64 = value.trim().parse().ok()?;
    (1..=10_000_000).contains(&n).then_some(n)
}

fn is_me(m: &Mailbox, own: &[String]) -> bool {
    let address = m.address.trim().to_lowercase();
    own.contains(&address)
}

/// Whether one condition holds for a message.
#[must_use]
pub fn condition_holds(c: &RuleCondition, m: &MessageFacts) -> bool {
    let words = || needles(&c.value);
    match c.kind {
        ConditionKind::From => m
            .from
            .as_ref()
            .is_some_and(|f| contains_any(&mailbox_text(f), &words())),
        ConditionKind::To => any_mailbox(&m.to, &words()),
        ConditionKind::Cc => any_mailbox(&m.cc, &words()),
        ConditionKind::ToOrCc => {
            let words = words();
            any_mailbox(&m.to, &words) || any_mailbox(&m.cc, &words)
        }
        ConditionKind::Subject => contains_any(&m.subject, &words()),
        ConditionKind::Body => m.text.as_deref().is_some_and(|t| contains_any(t, &words())),
        ConditionKind::SubjectOrBody => {
            let words = words();
            contains_any(&m.subject, &words)
                || m.text.as_deref().is_some_and(|t| contains_any(t, &words))
        }
        ConditionKind::Header => {
            let (Some((name, words)), Some(headers)) = (header_parts(&c.value), &m.headers) else {
                return false;
            };
            headers.iter().any(|(n, v)| {
                n.eq_ignore_ascii_case(&name) && (words.is_empty() || contains_any(v, &words))
            })
        }
        ConditionKind::HasAttachment => m.has_attachments,
        ConditionKind::Importance => {
            Importance::parse(c.value.trim()).is_some_and(|i| i == m.importance)
        }
        ConditionKind::SizeOver => {
            kilobytes(&c.value).is_some_and(|kb| m.size > kb.saturating_mul(1024))
        }
        ConditionKind::SizeUnder => {
            kilobytes(&c.value).is_some_and(|kb| m.size < kb.saturating_mul(1024))
        }
        ConditionKind::Category => {
            let wanted = c.value.trim();
            !wanted.is_empty()
                && m.categories
                    .iter()
                    .any(|have| crate::categories::same_name(have, wanted))
        }
        ConditionKind::ToMe => m.to.iter().chain(&m.cc).any(|r| is_me(r, &m.own_addresses)),
        ConditionKind::OnlyToMe => {
            m.cc.is_empty() && !m.to.is_empty() && m.to.iter().all(|r| is_me(r, &m.own_addresses))
        }
    }
}

/// Whether a rule matches a message (ignoring whether it is enabled).
#[must_use]
pub fn matches(rule: &RuleSpec, m: &MessageFacts) -> bool {
    let conditions = rule.conditions.is_empty()
        || match rule.match_kind {
            Match::All => rule.conditions.iter().all(|c| condition_holds(c, m)),
            Match::Any => rule.conditions.iter().any(|c| condition_holds(c, m)),
        };
    conditions && !rule.exceptions.iter().any(|c| condition_holds(c, m))
}

fn uses(rule: &RuleSpec, kinds: &[ConditionKind]) -> bool {
    rule.conditions
        .iter()
        .chain(&rule.exceptions)
        .any(|c| kinds.contains(&c.kind))
}

/// Whether a rule looks at the message text.
#[must_use]
pub fn needs_text(rule: &RuleSpec) -> bool {
    uses(rule, &[ConditionKind::Body, ConditionKind::SubjectOrBody])
}

/// Whether a rule looks at headers (needs the downloaded message).
#[must_use]
pub fn needs_headers(rule: &RuleSpec) -> bool {
    uses(rule, &[ConditionKind::Header])
}

/// The header lines of a raw message, unfolded (values not decoded).
#[must_use]
pub fn header_lines(raw: &[u8]) -> Vec<(String, String)> {
    let head = raw.get(..raw.len().min(MAX_HEADER_BYTES)).unwrap_or(raw);
    let text = String::from_utf8_lossy(head);
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            break;
        }
        if line.starts_with([' ', '\t']) {
            if let Some((_, value)) = out.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            out.push((name.trim().to_owned(), value.trim().to_owned()));
        }
    }
    out
}

/// Gather what the matching rules do. `rules` are the enabled rules that
/// apply to the message's account, in order. `None` when none matched.
#[must_use]
pub fn plan(rules: &[&Rule], m: &MessageFacts) -> Option<Plan> {
    let mut plan = Plan::default();
    let mut any = false;
    for rule in rules {
        if !matches(&rule.spec, m) {
            continue;
        }
        any = true;
        for action in &rule.spec.actions {
            apply_action(&mut plan, action);
        }
        if rule.spec.stop_processing {
            break;
        }
    }
    any.then_some(plan)
}

fn target(action: &RuleAction) -> Option<Target> {
    action
        .folder_id
        .clone()
        .map(Target::Folder)
        .or_else(|| action.role.map(Target::Role))
}

fn apply_action(plan: &mut Plan, action: &RuleAction) {
    match action.kind {
        ActionKind::Move => {
            if plan.destination.is_none() {
                plan.destination = target(action);
            }
        }
        ActionKind::Delete => {
            if plan.destination.is_none() {
                plan.destination = Some(Target::Role(FolderRole::Trash));
            }
        }
        ActionKind::Copy => {
            if let Some(t) = target(action)
                && !plan.copies.contains(&t)
            {
                plan.copies.push(t);
            }
        }
        ActionKind::Categorize => {
            if let Some(name) = action.category.as_deref().map(str::trim)
                && !name.is_empty()
                && !plan
                    .add_categories
                    .iter()
                    .any(|n| crate::categories::same_name(n, name))
            {
                plan.add_categories.push(name.to_owned());
            }
        }
        ActionKind::ClearCategories => {
            plan.clear_categories = true;
            plan.add_categories.clear();
        }
        ActionKind::Flag => plan.flagged = Some(true),
        ActionKind::ClearFlag => plan.flagged = Some(false),
        ActionKind::MarkRead => plan.seen = Some(true),
        ActionKind::SetImportance => {
            if action.importance.is_some() {
                plan.importance = action.importance;
            }
        }
        ActionKind::Notify => plan.notify = true,
        ActionKind::Pin => plan.pin = true,
    }
}

fn tidy(text: &str, max: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn check_condition(c: &RuleCondition) -> Result<RuleCondition, String> {
    let value = tidy(&c.value, MAX_VALUE);
    let label = c.kind.label();
    let value = match c.kind {
        k if k.is_text() => {
            if needles(&value).is_empty() {
                return Err(format!("Type what the {label} condition looks for."));
            }
            value
        }
        ConditionKind::Header => {
            if header_parts(&value).is_none() {
                return Err(
                    "Write a header condition as Name: words, for example List-Id: newsletter."
                        .to_owned(),
                );
            }
            value
        }
        ConditionKind::Importance => {
            let lower = value.to_lowercase();
            if Importance::parse(&lower).is_none() {
                return Err("Choose high, normal or low importance.".to_owned());
            }
            lower
        }
        ConditionKind::SizeOver | ConditionKind::SizeUnder => {
            if kilobytes(&value).is_none() {
                return Err(format!(
                    "The {label} condition needs a size in kilobytes, like 500."
                ));
            }
            value
        }
        ConditionKind::Category => {
            if value.is_empty() {
                return Err("Choose the category the condition looks for.".to_owned());
            }
            value
        }
        _ => String::new(),
    };
    Ok(RuleCondition {
        kind: c.kind,
        value,
    })
}

fn check_action(a: &RuleAction, every_account: bool) -> Result<RuleAction, String> {
    let mut out = RuleAction {
        kind: a.kind,
        folder_id: None,
        role: None,
        category: None,
        importance: None,
    };
    match a.kind {
        ActionKind::Move | ActionKind::Copy => {
            let verb = if a.kind == ActionKind::Move {
                "move"
            } else {
                "copy"
            };
            let folder = a
                .folder_id
                .as_deref()
                .map(str::trim)
                .filter(|f| !f.is_empty());
            if let Some(role) = a.role {
                if matches!(
                    role,
                    FolderRole::All | FolderRole::Flagged | FolderRole::Other
                ) {
                    return Err(format!("Choose a folder to {verb} the mail to."));
                }
                out.role = Some(role);
            } else if every_account {
                return Err(format!(
                    "A rule for every account can only {verb} mail to a kind of folder, such as Archive."
                ));
            } else if let Some(folder) = folder {
                out.folder_id = Some(folder.to_owned());
            } else {
                return Err(format!("Choose a folder to {verb} the mail to."));
            }
        }
        ActionKind::Categorize => {
            let name = a
                .category
                .as_deref()
                .map(|c| tidy(c, MAX_NAME))
                .unwrap_or_default();
            if name.is_empty() {
                return Err("Choose the category to add.".to_owned());
            }
            out.category = Some(name);
        }
        ActionKind::SetImportance => {
            if a.importance.is_none() {
                return Err("Choose the importance to set.".to_owned());
            }
            out.importance = a.importance;
        }
        _ => {}
    }
    Ok(out)
}

/// Tidy and check a rule from the editor: what can be checked without the
/// database (the folder's account and the category's existence are
/// checked by the caller).
///
/// # Errors
/// A message for the user.
pub fn checked(spec: &RuleSpec) -> Result<RuleSpec, String> {
    let name = tidy(&spec.name, MAX_NAME);
    if name.is_empty() {
        return Err("Give the rule a name.".to_owned());
    }
    if spec.actions.is_empty() {
        return Err("Choose at least one thing for the rule to do.".to_owned());
    }
    if spec.conditions.len() > MAX_CONDITIONS || spec.exceptions.len() > MAX_CONDITIONS {
        return Err(format!(
            "A rule can have up to {MAX_CONDITIONS} conditions and {MAX_CONDITIONS} exceptions."
        ));
    }
    if spec.actions.len() > MAX_ACTIONS {
        return Err(format!("A rule can do up to {MAX_ACTIONS} things."));
    }
    let account_id = spec
        .account_id
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_owned);
    let every_account = account_id.is_none();
    Ok(RuleSpec {
        name,
        enabled: spec.enabled,
        account_id,
        match_kind: spec.match_kind,
        conditions: spec
            .conditions
            .iter()
            .map(check_condition)
            .collect::<Result<_, _>>()?,
        exceptions: spec
            .exceptions
            .iter()
            .map(check_condition)
            .collect::<Result<_, _>>()?,
        actions: spec
            .actions
            .iter()
            .map(|a| check_action(a, every_account))
            .collect::<Result<_, _>>()?,
        stop_processing: spec.stop_processing,
        keep_on_server: spec.keep_on_server,
    })
}

// ---------------------------------------------------------------- Sieve

/// Why the mail server can't run this rule, or `None` if it can.
/// `extensions` are the server's Sieve extensions.
#[must_use]
pub fn sieve_unsupported(rule: &RuleSpec, extensions: &[String]) -> Option<String> {
    let has = |e: &str| extensions.iter().any(|x| x.eq_ignore_ascii_case(e));
    for c in rule.conditions.iter().chain(&rule.exceptions) {
        let why = match c.kind {
            ConditionKind::Body | ConditionKind::SubjectOrBody => {
                "The mail server can't look in message text"
            }
            ConditionKind::HasAttachment => "The mail server can't check for attachments",
            ConditionKind::Category => "Categories are kept in Mocky Mail, not on the mail server",
            ConditionKind::OnlyToMe => "The mail server can't check \"sent only to me\"",
            _ => continue,
        };
        return Some(format!("{why}, so this rule runs on this device."));
    }
    for a in &rule.actions {
        let why = match a.kind {
            ActionKind::Categorize | ActionKind::ClearCategories => {
                "Categories are set by Mocky Mail"
            }
            ActionKind::Pin => "Pinning is kept on this device",
            ActionKind::Notify => "Notifications come from this device",
            ActionKind::SetImportance => "Importance is set on this device",
            ActionKind::ClearFlag => "The mail server can't remove flags from new mail",
            ActionKind::Move | ActionKind::Delete if !has("fileinto") => {
                "The mail server can't file mail into folders"
            }
            ActionKind::Copy if !has("fileinto") || !has("copy") => {
                "The mail server can't copy mail into folders"
            }
            ActionKind::Flag | ActionKind::MarkRead if !has("imap4flags") => {
                "The mail server can't flag or mark mail read"
            }
            _ => continue,
        };
        return Some(format!("{why}, so this rule runs on this device."));
    }
    None
}

/// A rule ready for the script: the folder (UTF-8 name, as Sieve wants
/// it) each move, copy or delete action files into, `None` for the others.
#[derive(Debug, Clone)]
pub struct SieveRule<'a> {
    /// The rule.
    pub spec: &'a RuleSpec,
    /// One per action.
    pub mailboxes: Vec<Option<String>>,
}

/// A Sieve quoted string (control characters dropped).
fn quoted(text: &str) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    format!("\"{}\"", clean.replace('\\', "\\\\").replace('"', "\\\""))
}

fn string_list(items: &[String]) -> String {
    format!(
        "[{}]",
        items
            .iter()
            .map(|i| quoted(i))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Tests for the three importance levels, as the app reads them.
fn importance_test(level: Importance) -> String {
    let high = "anyof (header :contains \"importance\" \"high\", header :matches \"x-priority\" [\"1*\", \"2*\"], header :contains \"x-msmail-priority\" \"high\")";
    let low = "anyof (header :contains \"importance\" \"low\", header :matches \"x-priority\" [\"4*\", \"5*\"], header :contains \"x-msmail-priority\" \"low\")";
    match level {
        Importance::High => high.to_owned(),
        Importance::Low => low.to_owned(),
        Importance::Normal => format!("not anyof ({high}, {low})"),
    }
}

/// The Sieve test for a condition the server supports.
fn sieve_test(c: &RuleCondition, own: &[String]) -> String {
    let words = || string_list(&needles(&c.value));
    match c.kind {
        ConditionKind::From => format!("header :contains \"from\" {}", words()),
        ConditionKind::To => format!("header :contains \"to\" {}", words()),
        ConditionKind::Cc => format!("header :contains \"cc\" {}", words()),
        ConditionKind::ToOrCc => format!("header :contains [\"to\", \"cc\"] {}", words()),
        ConditionKind::Subject => format!("header :contains \"subject\" {}", words()),
        ConditionKind::Header => match header_parts(&c.value) {
            Some((name, words)) if words.is_empty() => format!("exists {}", quoted(&name)),
            Some((name, words)) => {
                format!("header :contains {} {}", quoted(&name), string_list(&words))
            }
            None => "false".to_owned(),
        },
        ConditionKind::SizeOver => {
            format!(
                "size :over {}K",
                kilobytes(&c.value).unwrap_or(u64::MAX / 2048)
            )
        }
        ConditionKind::SizeUnder => format!("size :under {}K", kilobytes(&c.value).unwrap_or(0)),
        ConditionKind::Importance => {
            Importance::parse(c.value.trim()).map_or_else(|| "false".to_owned(), importance_test)
        }
        ConditionKind::ToMe => format!("address :is [\"to\", \"cc\"] {}", string_list(own)),
        // Never written: `sieve_unsupported` keeps these rules local.
        ConditionKind::Body
        | ConditionKind::SubjectOrBody
        | ConditionKind::HasAttachment
        | ConditionKind::Category
        | ConditionKind::OnlyToMe => "false".to_owned(),
    }
}

fn rule_test(rule: &RuleSpec, own: &[String]) -> String {
    let tests: Vec<String> = rule.conditions.iter().map(|c| sieve_test(c, own)).collect();
    let conditions = match (tests.len(), rule.match_kind) {
        (0, _) => None,
        (1, _) => tests.into_iter().next(),
        (_, Match::All) => Some(format!("allof ({})", tests.join(", "))),
        (_, Match::Any) => Some(format!("anyof ({})", tests.join(", "))),
    };
    let exceptions: Vec<String> = rule.exceptions.iter().map(|c| sieve_test(c, own)).collect();
    let exceptions =
        (!exceptions.is_empty()).then(|| format!("not anyof ({})", exceptions.join(", ")));
    match (conditions, exceptions) {
        (None, None) => "true".to_owned(),
        (Some(c), None) => c,
        (None, Some(e)) => e,
        (Some(c), Some(e)) => format!("allof ({c}, {e})"),
    }
}

/// The body of one rule's `if` block.
fn rule_actions(rule: &SieveRule<'_>) -> Vec<String> {
    let mut flags = Vec::new();
    let mut copies = Vec::new();
    let mut destination = None;
    for (action, mailbox) in rule.spec.actions.iter().zip(&rule.mailboxes) {
        match action.kind {
            ActionKind::Flag => flags.push("\\Flagged"),
            ActionKind::MarkRead => flags.push("\\Seen"),
            ActionKind::Copy => {
                if let Some(m) = mailbox
                    && !copies.contains(m)
                {
                    copies.push(m.clone());
                }
            }
            ActionKind::Move | ActionKind::Delete if destination.is_none() => {
                destination.clone_from(mailbox);
            }
            _ => {}
        }
    }
    flags.dedup();
    let mut lines = Vec::new();
    // Flags first: they go with the message wherever it is filed.
    if !flags.is_empty() {
        lines.push(format!(
            "addflag {};",
            string_list(&flags.iter().map(|f| (*f).to_owned()).collect::<Vec<_>>())
        ));
    }
    for copy in copies {
        lines.push(format!("fileinto :copy {};", quoted(&copy)));
    }
    if let Some(destination) = destination {
        lines.push(format!("fileinto {};", quoted(&destination)));
    }
    if rule.spec.stop_processing {
        lines.push("stop;".to_owned());
    }
    lines
}

/// The rules script: `rules` (supported, in order) for an account whose
/// own addresses are `own`; `previous` is a script that ran before, to
/// keep running after them (through `include`).
#[must_use]
pub fn sieve_script(rules: &[SieveRule<'_>], own: &[String], previous: Option<&str>) -> String {
    let mut out =
        String::from("# Mocky Mail rules. Change them in Mocky Mail: edits here are replaced.\n");
    if let Some(previous) = previous {
        out.push_str(PREVIOUS_MARK);
        out.push_str(&serde_json::to_string(previous).unwrap_or_default());
        out.push('\n');
    }
    let blocks: Vec<(String, String, Vec<String>)> = rules
        .iter()
        .map(|r| (r.spec.name.clone(), rule_test(r.spec, own), rule_actions(r)))
        .filter(|(_, _, actions)| !actions.is_empty())
        .collect();
    let text = blocks
        .iter()
        .flat_map(|(_, _, a)| a.iter())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    let mut requires = Vec::new();
    if text.contains("fileinto ") {
        requires.push("fileinto");
    }
    if text.contains("fileinto :copy") {
        requires.push("copy");
    }
    if text.contains("addflag ") {
        requires.push("imap4flags");
    }
    if previous.is_some() {
        requires.push("include");
    }
    if !requires.is_empty() {
        let _ = writeln!(
            out,
            "require {};",
            string_list(&requires.iter().map(|r| (*r).to_owned()).collect::<Vec<_>>())
        );
    }
    for (name, test, actions) in blocks {
        let name: String = name.chars().filter(|c| !c.is_control()).collect();
        let _ = writeln!(out, "\n# {name}\nif {test} {{");
        for action in actions {
            let _ = writeln!(out, "    {action}");
        }
        out.push_str("}\n");
    }
    if let Some(previous) = previous {
        let _ = writeln!(out, "\ninclude :personal {};", quoted(previous));
    }
    out
}

/// The script the rules script keeps running, read back from it.
#[must_use]
pub fn sieve_previous(script: &str) -> Option<String> {
    script
        .lines()
        .find_map(|l| l.strip_prefix(PREVIOUS_MARK))
        .and_then(|json| serde_json::from_str::<String>(json).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mb(name: Option<&str>, address: &str) -> Mailbox {
        Mailbox {
            name: name.map(str::to_owned),
            address: address.to_owned(),
        }
    }

    fn facts() -> MessageFacts {
        MessageFacts {
            from: Some(mb(Some("Safaricom M-PESA"), "mpesa@safaricom.co.ke")),
            to: vec![mb(Some("Info"), "info@mocky.co.ke")],
            cc: vec![mb(None, "accounts@mocky.co.ke")],
            subject: "Payment received: KES 8,500".into(),
            text: Some("Confirmed. You have received Ksh8,500.00 from JANE".into()),
            headers: Some(vec![("List-Id".into(), "<alerts.safaricom.co.ke>".into())]),
            has_attachments: false,
            importance: Importance::High,
            size: 30 * 1024,
            categories: vec!["Payments".into()],
            own_addresses: vec!["info@mocky.co.ke".into()],
        }
    }

    fn cond(kind: ConditionKind, value: &str) -> RuleCondition {
        RuleCondition {
            kind,
            value: value.into(),
        }
    }

    fn action(kind: ActionKind) -> RuleAction {
        RuleAction {
            kind,
            folder_id: None,
            role: None,
            category: None,
            importance: None,
        }
    }

    fn spec(conditions: Vec<RuleCondition>, actions: Vec<RuleAction>) -> RuleSpec {
        RuleSpec {
            name: "Payments".into(),
            enabled: true,
            account_id: Some("a".into()),
            match_kind: Match::All,
            conditions,
            exceptions: vec![],
            actions,
            stop_processing: false,
            keep_on_server: false,
        }
    }

    fn rule(id: &str, spec: RuleSpec) -> Rule {
        Rule {
            id: id.into(),
            spec,
            server_status: ServerStatus::Local,
            server_note: None,
            order: 0,
        }
    }

    fn holds(kind: ConditionKind, value: &str) -> bool {
        condition_holds(&cond(kind, value), &facts())
    }

    #[test]
    fn text_conditions_look_for_any_word_ignoring_case() {
        use ConditionKind as K;
        assert!(holds(K::From, "SAFARICOM"));
        assert!(holds(K::From, "nobody, mpesa@"));
        assert!(holds(K::From, "M-PESA"));
        assert!(!holds(K::From, "kcb"));
        assert!(holds(K::To, "info@mocky"));
        assert!(!holds(K::To, "accounts@"));
        assert!(holds(K::Cc, "accounts@"));
        assert!(holds(K::ToOrCc, "accounts@"));
        assert!(holds(K::Subject, "payment"));
        assert!(!holds(K::Subject, "jane"));
        assert!(holds(K::Body, "jane"));
        assert!(holds(K::SubjectOrBody, "jane"));
        assert!(holds(K::SubjectOrBody, "kes 8,500"));
        // An empty list never matches.
        assert!(!holds(K::Subject, " , "));
        // No text downloaded: text conditions don't match.
        let mut bare = facts();
        bare.text = None;
        assert!(!condition_holds(&cond(K::Body, "jane"), &bare));
        assert!(condition_holds(&cond(K::SubjectOrBody, "payment"), &bare));
    }

    #[test]
    fn other_conditions() {
        use ConditionKind as K;
        assert!(holds(K::Header, "List-Id: safaricom"));
        assert!(holds(K::Header, "list-id"));
        assert!(!holds(K::Header, "List-Id: kcb"));
        assert!(!holds(K::Header, "X-Spam: yes"));
        assert!(!holds(K::Header, ": x"));
        assert!(!holds(K::HasAttachment, ""));
        assert!(holds(K::Importance, "high"));
        assert!(!holds(K::Importance, "low"));
        assert!(holds(K::SizeOver, "29"));
        assert!(!holds(K::SizeOver, "30"));
        assert!(holds(K::SizeUnder, "31"));
        assert!(!holds(K::SizeUnder, "abc"));
        assert!(holds(K::Category, "payments"));
        assert!(!holds(K::Category, "clients"));
        assert!(holds(K::ToMe, ""));
        // Cc'd to someone else too: not only to me.
        assert!(!holds(K::OnlyToMe, ""));
        let mut alone = facts();
        alone.cc.clear();
        assert!(condition_holds(&cond(K::OnlyToMe, ""), &alone));
        alone.to.push(mb(None, "boss@mocky.co.ke"));
        assert!(!condition_holds(&cond(K::OnlyToMe, ""), &alone));
        // Not downloaded: header conditions don't match.
        let mut bare = facts();
        bare.headers = None;
        assert!(!condition_holds(&cond(K::Header, "List-Id"), &bare));
    }

    #[test]
    fn all_any_none_and_exceptions() {
        use ConditionKind as K;
        let mut r = spec(
            vec![cond(K::From, "safaricom"), cond(K::Subject, "invoice")],
            vec![action(ActionKind::Flag)],
        );
        assert!(!matches(&r, &facts()));
        r.match_kind = Match::Any;
        assert!(matches(&r, &facts()));
        r.exceptions = vec![cond(K::Body, "nothing"), cond(K::Body, "jane")];
        assert!(!matches(&r, &facts()));
        // No conditions: every message, still minus exceptions.
        r.conditions.clear();
        r.exceptions.clear();
        assert!(matches(&r, &facts()));
        r.exceptions = vec![cond(K::ToMe, "")];
        assert!(!matches(&r, &facts()));
        assert!(!needs_text(&spec(vec![], vec![])));
        r.exceptions = vec![cond(K::SubjectOrBody, "x")];
        assert!(needs_text(&r));
        assert!(!needs_headers(&r));
        r.conditions = vec![cond(K::Header, "List-Id")];
        assert!(needs_headers(&r));
    }

    #[test]
    fn plans_gather_actions_in_order_and_stop() {
        use ConditionKind as K;
        let mut first = spec(
            vec![cond(K::From, "safaricom")],
            vec![
                RuleAction {
                    folder_id: Some("payments".into()),
                    ..action(ActionKind::Move)
                },
                RuleAction {
                    category: Some("M-Pesa".into()),
                    ..action(ActionKind::Categorize)
                },
                action(ActionKind::Flag),
            ],
        );
        let second = spec(
            vec![],
            vec![
                action(ActionKind::Delete),
                action(ActionKind::ClearFlag),
                action(ActionKind::MarkRead),
                RuleAction {
                    role: Some(FolderRole::Archive),
                    ..action(ActionKind::Copy)
                },
                RuleAction {
                    category: Some("m-pesa".into()),
                    ..action(ActionKind::Categorize)
                },
                RuleAction {
                    importance: Some(Importance::Low),
                    ..action(ActionKind::SetImportance)
                },
                action(ActionKind::Notify),
                action(ActionKind::Pin),
            ],
        );
        let not_matching = spec(vec![cond(K::From, "kcb")], vec![action(ActionKind::Delete)]);
        let rules = [
            rule("0", not_matching.clone()),
            rule("1", first.clone()),
            rule("2", second.clone()),
        ];
        let refs: Vec<&Rule> = rules.iter().collect();
        let p = plan(&refs, &facts()).unwrap();
        assert_eq!(p.destination, Some(Target::Folder("payments".into())));
        assert_eq!(p.flagged, Some(false));
        assert_eq!(p.seen, Some(true));
        assert_eq!(p.add_categories, ["M-Pesa"]);
        assert_eq!(p.copies, [Target::Role(FolderRole::Archive)]);
        assert_eq!(p.importance, Some(Importance::Low));
        assert!(p.notify && p.pin && !p.clear_categories);
        // Stop: the second rule doesn't run.
        first.stop_processing = true;
        let rules = [rule("1", first), rule("2", second)];
        let refs: Vec<&Rule> = rules.iter().collect();
        let p = plan(&refs, &facts()).unwrap();
        assert_eq!(p.flagged, Some(true));
        assert_eq!(p.seen, None);
        // Nothing matched.
        let rules = [rule("0", not_matching)];
        let refs: Vec<&Rule> = rules.iter().collect();
        assert_eq!(plan(&refs, &facts()), None);
        assert_eq!(plan(&[], &facts()), None);
    }

    #[test]
    fn clearing_categories_drops_earlier_ones() {
        let r = spec(
            vec![],
            vec![
                RuleAction {
                    category: Some("A".into()),
                    ..action(ActionKind::Categorize)
                },
                action(ActionKind::ClearCategories),
                RuleAction {
                    category: Some("B".into()),
                    ..action(ActionKind::Categorize)
                },
            ],
        );
        let rules = [rule("1", r)];
        let refs: Vec<&Rule> = rules.iter().collect();
        let p = plan(&refs, &facts()).unwrap();
        assert!(p.clear_categories);
        assert_eq!(p.add_categories, ["B"]);
    }

    #[test]
    fn rules_are_checked_and_tidied() {
        use ConditionKind as K;
        let ok = spec(
            vec![
                cond(K::From, "  boss@x.co \u{7}"),
                cond(K::Importance, "HIGH"),
            ],
            vec![RuleAction {
                folder_id: Some("f1".into()),
                category: Some("ignored".into()),
                ..action(ActionKind::Move)
            }],
        );
        let tidy = checked(&ok).unwrap();
        assert_eq!(tidy.conditions[0].value, "boss@x.co");
        assert_eq!(tidy.conditions[1].value, "high");
        assert_eq!(tidy.actions[0].category, None);
        let bad = |f: fn(&mut RuleSpec)| {
            let mut r = ok.clone();
            f(&mut r);
            checked(&r).unwrap_err()
        };
        assert_eq!(bad(|r| r.name = "  ".into()), "Give the rule a name.");
        assert_eq!(
            bad(|r| r.actions.clear()),
            "Choose at least one thing for the rule to do."
        );
        assert_eq!(
            bad(|r| r.conditions[0].value = ",".into()),
            "Type what the From condition looks for."
        );
        assert!(bad(|r| r.conditions[1].value = "urgent".into()).contains("high, normal or low"));
        assert!(bad(|r| r.conditions.push(cond(K::SizeOver, "big"))).contains("kilobytes"));
        assert!(bad(|r| r.conditions.push(cond(K::Header, "Bad Name: x"))).contains("Name: words"));
        assert!(bad(|r| r.conditions.push(cond(K::Category, " "))).contains("category"));
        assert_eq!(
            bad(|r| r.actions[0].folder_id = None),
            "Choose a folder to move the mail to."
        );
        assert!(bad(|r| r.account_id = None).contains("every account"));
        assert!(
            bad(|r| r.actions = vec![RuleAction {
                role: Some(FolderRole::Other),
                ..action(ActionKind::Copy)
            }])
            .contains("copy")
        );
        assert!(bad(|r| r.actions = vec![action(ActionKind::Categorize)]).contains("category"));
        assert!(
            bad(|r| r.actions = vec![action(ActionKind::SetImportance)]).contains("importance")
        );
        assert!(bad(|r| r.conditions = vec![cond(K::ToMe, ""); 31]).contains("up to 30"));
        // Every account by role is fine; blank account id means every account.
        let mut every = ok.clone();
        every.account_id = Some(" ".into());
        every.actions = vec![RuleAction {
            role: Some(FolderRole::Archive),
            ..action(ActionKind::Move)
        }];
        let every = checked(&every).unwrap();
        assert_eq!(every.account_id, None);
        // Flag-only conditions keep no value.
        let c = checked(&spec(
            vec![cond(K::HasAttachment, "x")],
            vec![action(ActionKind::Pin)],
        ))
        .unwrap();
        assert_eq!(c.conditions[0].value, "");
    }

    #[test]
    fn reads_header_lines() {
        let raw = b"From: a@b.c\r\nList-Id: Weekly\r\n  <weekly.example>\r\nX-Empty:\r\n\r\nList-Id: body\r\n";
        assert_eq!(
            header_lines(raw),
            vec![
                ("From".to_owned(), "a@b.c".to_owned()),
                ("List-Id".to_owned(), "Weekly <weekly.example>".to_owned()),
                ("X-Empty".to_owned(), String::new()),
            ]
        );
        assert!(header_lines(b"").is_empty());
    }

    #[test]
    fn rules_round_trip_as_the_interface_sends_them() {
        let json = serde_json::json!({
            "id": null, "name": "News", "enabled": true, "accountId": null, "match": "any",
            "conditions": [{ "kind": "toOrCc", "value": "news" }, { "kind": "hasAttachment", "value": "" }],
            "exceptions": [],
            "actions": [{ "kind": "move", "role": "archive" }, { "kind": "setImportance", "importance": "low" }],
            "stopProcessing": true, "keepOnServer": false
        });
        let input: RuleInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.id, None);
        assert_eq!(input.spec.match_kind, Match::Any);
        assert_eq!(input.spec.conditions[0].kind, ConditionKind::ToOrCc);
        assert_eq!(input.spec.actions[0].role, Some(FolderRole::Archive));
        let saved = Rule {
            id: "r1".into(),
            spec: input.spec,
            server_status: ServerStatus::Unsupported,
            server_note: Some("why".into()),
            order: 2,
        };
        let out = serde_json::to_value(&saved).unwrap();
        assert_eq!(out["match"], "any");
        assert_eq!(out["serverStatus"], "unsupported");
        assert_eq!(out["stopProcessing"], true);
        assert_eq!(out["accountId"], serde_json::Value::Null);
        assert_eq!(out["actions"][1]["importance"], "low");
        assert_eq!(out["order"], 2);
        assert_eq!(ServerStatus::parse("server"), ServerStatus::Server);
        assert_eq!(ServerStatus::parse("?"), ServerStatus::Local);
    }

    fn all_extensions() -> Vec<String> {
        ["fileinto", "copy", "imap4flags", "include", "vacation"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    }

    #[test]
    fn says_why_the_server_cant_run_a_rule() {
        use ConditionKind as K;
        let ext = all_extensions();
        let fine = spec(
            vec![
                cond(K::From, "x"),
                cond(K::Header, "List-Id: y"),
                cond(K::SizeOver, "10"),
            ],
            vec![action(ActionKind::Flag), action(ActionKind::Delete)],
        );
        assert_eq!(sieve_unsupported(&fine, &ext), None);
        for (c, a) in [
            (Some(K::Body), None),
            (Some(K::SubjectOrBody), None),
            (Some(K::HasAttachment), None),
            (Some(K::Category), None),
            (Some(K::OnlyToMe), None),
            (None, Some(ActionKind::Categorize)),
            (None, Some(ActionKind::Pin)),
            (None, Some(ActionKind::Notify)),
            (None, Some(ActionKind::SetImportance)),
        ] {
            let mut r = fine.clone();
            if let Some(k) = c {
                r.exceptions.push(cond(k, "x"));
            }
            if let Some(k) = a {
                r.actions.push(action(k));
            }
            let why = sieve_unsupported(&r, &ext).unwrap();
            assert!(why.ends_with("so this rule runs on this device."), "{why}");
        }
        // A server without imap4flags can still file mail.
        let only_fileinto = vec!["fileinto".to_owned()];
        assert!(
            sieve_unsupported(&fine, &only_fileinto)
                .unwrap()
                .contains("flag")
        );
        let mut filing = fine.clone();
        filing.actions = vec![action(ActionKind::Delete)];
        assert_eq!(sieve_unsupported(&filing, &only_fileinto), None);
        filing.actions = vec![action(ActionKind::Copy)];
        assert!(
            sieve_unsupported(&filing, &only_fileinto)
                .unwrap()
                .contains("copy")
        );
        assert!(sieve_unsupported(&filing, &[]).is_some());
    }

    #[test]
    fn writes_a_sieve_script() {
        use ConditionKind as K;
        let mut payments = spec(
            vec![
                cond(K::From, "mpesa@, \"kcb\""),
                cond(K::Subject, "payment"),
            ],
            vec![
                RuleAction {
                    folder_id: Some("f".into()),
                    ..action(ActionKind::Move)
                },
                action(ActionKind::MarkRead),
                action(ActionKind::Flag),
                RuleAction {
                    folder_id: Some("c".into()),
                    ..action(ActionKind::Copy)
                },
            ],
        );
        payments.stop_processing = true;
        payments.exceptions = vec![cond(K::ToMe, ""), cond(K::Importance, "low")];
        let mut big = spec(
            vec![cond(K::SizeOver, "5000"), cond(K::Header, "X-Spam-Flag")],
            vec![action(ActionKind::Delete)],
        );
        big.name = "Big\nones".into();
        big.match_kind = Match::Any;
        let everything = spec(vec![], vec![action(ActionKind::Flag)]);
        let own = vec!["info@mocky.co.ke".to_owned()];
        let rules = [
            SieveRule {
                spec: &payments,
                mailboxes: vec![
                    Some("Payments/M-Pesa".into()),
                    None,
                    None,
                    Some("Copies".into()),
                ],
            },
            SieveRule {
                spec: &big,
                mailboxes: vec![Some("Trash".into())],
            },
            SieveRule {
                spec: &everything,
                mailboxes: vec![None],
            },
        ];
        let text = sieve_script(&rules, &own, Some("roundcube"));
        assert!(text.starts_with("# Mocky Mail rules."), "{text}");
        assert!(
            text.contains("require [\"fileinto\", \"copy\", \"imap4flags\", \"include\"];\n"),
            "{text}"
        );
        assert!(text.contains(
            "if allof (allof (header :contains \"from\" [\"mpesa@\", \"\\\"kcb\\\"\"], header :contains \"subject\" [\"payment\"]), not anyof (address :is [\"to\", \"cc\"] [\"info@mocky.co.ke\"], anyof (header :contains \"importance\" \"low\""
        ), "{text}");
        // Flags first, then copies, then the move, then stop.
        assert!(text.contains(
            "    addflag [\"\\\\Seen\", \"\\\\Flagged\"];\n    fileinto :copy \"Copies\";\n    fileinto \"Payments/M-Pesa\";\n    stop;\n}"
        ), "{text}");
        assert!(text.contains("# Bigones\nif anyof (size :over 5000K, exists \"X-Spam-Flag\") {\n    fileinto \"Trash\";\n}"), "{text}");
        assert!(
            text.contains("if true {\n    addflag [\"\\\\Flagged\"];\n}"),
            "{text}"
        );
        assert!(
            text.ends_with("include :personal \"roundcube\";\n"),
            "{text}"
        );
        assert_eq!(sieve_previous(&text).as_deref(), Some("roundcube"));

        // Nothing to require, nothing kept.
        let flag_only = sieve_script(&rules[2..], &own, None);
        assert!(flag_only.contains("require [\"imap4flags\"];"));
        assert!(!flag_only.contains("include"));
        assert_eq!(sieve_previous(&flag_only), None);
        let empty = sieve_script(&[], &own, None);
        assert!(!empty.contains("require") && !empty.contains("if "));
        assert_eq!(
            importance_test(Importance::Normal).matches("anyof").count(),
            3
        );
    }
}
