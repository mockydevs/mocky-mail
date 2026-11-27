//! The search box query language.
//!
//! Plain words must all match (each as a prefix, so results come while
//! typing); `"quoted phrases"` match as written; a leading `-` negates a
//! word, phrase or operator; `OR` (in capitals) between two terms matches
//! either. Operators: `from:` `to:` `cc:` `subject:` `body:` (words),
//! `has:attachment`, `is:unread|read|flagged|pinned|answered`,
//! `category:` (quote names with spaces), `before:` / `after:`
//! (`YYYY-MM-DD`, `today`, `yesterday`), `in:` (a folder role or name),
//! `larger:` / `smaller:` (`500k`, `2m`). A `word:` that isn't an operator
//! is searched as plain text.
//!
//! [`parse`] never fails: what it can't use is reported in plain words in
//! [`ParsedSearch::errors`] and left out of the search. The store turns
//! [`ParsedSearch::groups`] into SQL; this module does no I/O.

use chrono::{Duration, NaiveDate};
use serde::Serialize;

use crate::folders::FolderRole;

/// Most terms used from one search, so a pasted essay stays cheap.
pub const MAX_TERMS: usize = 16;

/// Longest value kept from one term, in characters.
const MAX_VALUE: usize = 200;

/// Where a search looks (the interface's `SearchScope`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchScope {
    /// The open view only (its folder, role or unified view).
    #[default]
    Folder,
    /// The open folder and the folders below it.
    Subfolders,
    /// The open folder's whole account, without Trash and Junk.
    Account,
    /// Every account, without Trash and Junk.
    All,
}

/// What a term looks at (the interface's `SearchTerm.field`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Field {
    /// Plain words and phrases.
    Text,
    /// The sender.
    From,
    /// To recipients.
    To,
    /// Cc recipients.
    Cc,
    /// The subject.
    Subject,
    /// The message text.
    Body,
    /// `has:attachment`.
    Has,
    /// `is:` a state.
    Is,
    /// A colour category.
    Category,
    /// Sent before a day.
    Before,
    /// Sent on or after a day.
    After,
    /// In a folder.
    In,
    /// Bigger than a size.
    Larger,
    /// Smaller than a size.
    Smaller,
}

impl Field {
    fn from_operator(name: &str) -> Option<Self> {
        Some(match name {
            "from" => Self::From,
            "to" => Self::To,
            "cc" => Self::Cc,
            "subject" => Self::Subject,
            "body" => Self::Body,
            "has" => Self::Has,
            "is" => Self::Is,
            "category" => Self::Category,
            "before" => Self::Before,
            "after" => Self::After,
            "in" => Self::In,
            "larger" => Self::Larger,
            "smaller" => Self::Smaller,
            _ => return None,
        })
    }

    const fn operator(self) -> &'static str {
        match self {
            Self::Text => "",
            Self::From => "from:",
            Self::To => "to:",
            Self::Cc => "cc:",
            Self::Subject => "subject:",
            Self::Body => "body:",
            Self::Has => "has:",
            Self::Is => "is:",
            Self::Category => "category:",
            Self::Before => "before:",
            Self::After => "after:",
            Self::In => "in:",
            Self::Larger => "larger:",
            Self::Smaller => "smaller:",
        }
    }
}

/// One piece of a parsed search, for showing the query back as chips.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchTerm {
    /// What it looks at.
    pub field: Field,
    /// The value as typed (without quotes).
    pub value: String,
    /// Led by `-`: messages that match are left out.
    pub negated: bool,
    /// Joined to the next term by `OR` (either may match). Left out of the
    /// JSON when false.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub or_with_next: bool,
}

/// A message state for `is:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Not yet read.
    Unread,
    /// Read.
    Read,
    /// Flagged.
    Flagged,
    /// Pinned.
    Pinned,
    /// Replied to.
    Answered,
}

/// Where `in:` looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// Folders with this role.
    Role(FolderRole),
    /// Folders with this name or path (any case).
    Name(String),
}

/// What one term matches, ready for the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    /// Anywhere in the subject, people, preview or text. `phrase`: as
    /// written; otherwise each word may be the start of a longer one.
    Text {
        /// The words.
        text: String,
        /// Quoted.
        phrase: bool,
    },
    /// Sender name or address.
    From(String),
    /// To names or addresses.
    To(String),
    /// Cc names or addresses.
    Cc(String),
    /// Subject words.
    Subject(String),
    /// Text words (and the preview, for mail whose text isn't downloaded).
    Body(String),
    /// Has attachments.
    HasAttachment,
    /// In this state.
    Is(State),
    /// Has this category (any case).
    Category(String),
    /// Dated before this Unix time.
    Before(i64),
    /// Dated at or after this Unix time.
    After(i64),
    /// In these folders.
    In(Place),
    /// Bigger than this many bytes.
    Larger(u64),
    /// Smaller than this many bytes.
    Smaller(u64),
}

/// A filter, or its opposite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    /// What to match.
    pub filter: Filter,
    /// Leave out the messages it matches instead.
    pub negated: bool,
}

/// A parsed search box query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ParsedSearch {
    /// The terms that are used, in order.
    pub terms: Vec<SearchTerm>,
    /// Plain-language problems, e.g. "after: needs a date like 2026-10-01".
    pub errors: Vec<String>,
    /// What to match: every group must match, and within a group any
    /// condition (terms joined by `OR`).
    #[serde(skip)]
    pub groups: Vec<Vec<Condition>>,
}

impl ParsedSearch {
    /// Whether any `in:` term (not negated) picks folders: the search then
    /// looks across the account instead of only the open folder, Trash and
    /// Junk included.
    #[must_use]
    pub fn picks_folders(&self) -> bool {
        self.groups
            .iter()
            .flatten()
            .any(|c| !c.negated && matches!(c.filter, Filter::In(_)))
    }
}

/// "Today" for `today`, `yesterday` and dates, which are local days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchContext {
    /// The local date now.
    pub today: NaiveDate,
    /// Seconds the local time zone is ahead of UTC.
    pub utc_offset_secs: i32,
}

impl SearchContext {
    /// From a Unix time and the local offset.
    #[must_use]
    pub fn at(unix_secs: i64, utc_offset_secs: i32) -> Self {
        let local = unix_secs.saturating_add(i64::from(utc_offset_secs));
        let today =
            chrono::DateTime::from_timestamp(local, 0).map_or(NaiveDate::MIN, |t| t.date_naive());
        Self {
            today,
            utc_offset_secs,
        }
    }

    /// Unix time of the local midnight starting `day`.
    fn start_of(self, day: NaiveDate) -> i64 {
        day.and_hms_opt(0, 0, 0)
            .map_or(0, |t| t.and_utc().timestamp())
            .saturating_sub(i64::from(self.utc_offset_secs))
    }
}

/// A raw piece of the query before its value is checked.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    /// `None` for plain text.
    field: Option<Field>,
    value: String,
    /// Typed in double quotes.
    quoted: bool,
    negated: bool,
    /// A bare `OR`.
    or: bool,
}

/// Characters kept in values: no control characters (FTS5 stops at NUL).
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_VALUE)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn read_quoted(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut out = String::new();
    for c in chars.by_ref() {
        if c == '"' {
            break;
        }
        out.push(c);
    }
    out
}

fn tokens(input: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&first) = chars.peek() else { break };
        let mut negated = false;
        if first == '-' {
            chars.next();
            match chars.peek() {
                Some(c) if !c.is_whitespace() => negated = true,
                // A lone "-": nothing to search.
                _ => continue,
            }
        }
        if chars.next_if_eq(&'"').is_some() {
            let value = read_quoted(&mut chars);
            out.push(Token {
                field: None,
                value,
                quoted: true,
                negated,
                or: false,
            });
            continue;
        }
        let mut word = String::new();
        let mut token = None;
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                break;
            }
            chars.next();
            if c == '"'
                && let Some(name) = word.strip_suffix(':')
                && let Some(field) = Field::from_operator(&name.to_lowercase())
            {
                token = Some(Token {
                    field: Some(field),
                    value: read_quoted(&mut chars),
                    quoted: true,
                    negated,
                    or: false,
                });
                break;
            }
            word.push(c);
        }
        if let Some(token) = token {
            out.push(token);
            continue;
        }
        if word == "OR" && !negated {
            out.push(Token {
                field: None,
                value: word,
                quoted: false,
                negated: false,
                or: true,
            });
            continue;
        }
        let operator = word
            .split_once(':')
            .and_then(|(name, value)| Some((Field::from_operator(&name.to_lowercase())?, value)));
        out.push(match operator {
            Some((field, value)) => Token {
                field: Some(field),
                value: value.to_owned(),
                quoted: false,
                negated,
                or: false,
            },
            None => Token {
                field: None,
                value: word,
                quoted: false,
                negated,
                or: false,
            },
        });
    }
    out
}

fn has_words(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}

/// A local day: `YYYY-MM-DD` (or with `/`), `today`, `yesterday`.
fn day(value: &str, ctx: SearchContext) -> Option<NaiveDate> {
    match value.to_lowercase().as_str() {
        "today" => Some(ctx.today),
        "yesterday" => ctx.today.checked_sub_signed(Duration::days(1)),
        other => NaiveDate::parse_from_str(&other.replace('/', "-"), "%Y-%m-%d").ok(),
    }
}

/// Bytes from `500`, `500k`, `1.5m`, `2gb` (powers of 1024).
fn size(value: &str) -> Option<u64> {
    let lower = value.to_lowercase();
    let lower = lower.strip_suffix('b').unwrap_or(&lower);
    let (number, unit) = match lower.char_indices().last() {
        Some((i, 'k')) => (lower.get(..i)?, 1024.0),
        Some((i, 'm')) => (lower.get(..i)?, 1024.0 * 1024.0),
        Some((i, 'g')) => (lower.get(..i)?, 1024.0 * 1024.0 * 1024.0),
        _ => (lower, 1.0),
    };
    if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let n: f64 = number.parse().ok()?;
    let bytes = (n * unit).round();
    // Up to 1 TB: anything bigger is a typo.
    if !(0.0..=1_099_511_627_776.0).contains(&bytes) {
        return None;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked to be a whole number from 0 to 1 TB just above"
    )]
    Some(bytes as u64)
}

fn place(value: &str) -> Place {
    let role = match value.to_lowercase().as_str() {
        "inbox" => Some(FolderRole::Inbox),
        "sent" => Some(FolderRole::Sent),
        "drafts" | "draft" => Some(FolderRole::Drafts),
        "archive" => Some(FolderRole::Archive),
        "junk" | "spam" => Some(FolderRole::Junk),
        "trash" | "bin" | "deleted" => Some(FolderRole::Trash),
        _ => None,
    };
    role.map_or_else(|| Place::Name(value.to_owned()), Place::Role)
}

/// The filter for one token, or what is wrong with it.
fn filter(token: &Token, ctx: SearchContext) -> Result<Option<Filter>, String> {
    let value = clean(&token.value);
    let Some(field) = token.field else {
        // Punctuation alone has nothing to search; skip it quietly.
        return Ok(has_words(&value).then_some(Filter::Text {
            text: value,
            phrase: token.quoted,
        }));
    };
    let op = field.operator();
    if value.is_empty() {
        return Err(format!("Type something after {op}"));
    }
    let words = |make: fn(String) -> Filter| {
        if has_words(&value) {
            Ok(Some(make(value.clone())))
        } else {
            Err(format!("{op} needs a word or address to look for."))
        }
    };
    match field {
        Field::Text => Ok(None),
        Field::From => words(Filter::From),
        Field::To => words(Filter::To),
        Field::Cc => words(Filter::Cc),
        Field::Subject => words(Filter::Subject),
        Field::Body => words(Filter::Body),
        Field::Has => match value.to_lowercase().as_str() {
            "attachment" | "attachments" | "attach" | "file" | "files" => {
                Ok(Some(Filter::HasAttachment))
            }
            _ => Err("has: only understands has:attachment.".to_owned()),
        },
        Field::Is => {
            let state = match value.to_lowercase().as_str() {
                "unread" | "new" => State::Unread,
                "read" | "seen" => State::Read,
                "flagged" | "starred" => State::Flagged,
                "pinned" => State::Pinned,
                "answered" | "replied" => State::Answered,
                _ => {
                    return Err("is: can be unread, read, flagged, pinned or answered.".to_owned());
                }
            };
            Ok(Some(Filter::Is(state)))
        }
        Field::Category => Ok(Some(Filter::Category(value))),
        Field::Before | Field::After => {
            let Some(day) = day(&value, ctx) else {
                return Err(format!(
                    "{op} needs a date like 2026-10-01 (or today, yesterday)."
                ));
            };
            let at = ctx.start_of(day);
            Ok(Some(if field == Field::Before {
                Filter::Before(at)
            } else {
                Filter::After(at)
            }))
        }
        Field::In => Ok(Some(Filter::In(place(&value)))),
        Field::Larger | Field::Smaller => {
            let Some(bytes) = size(&value) else {
                return Err(format!("{op} needs a size like 500k or 2m."));
            };
            Ok(Some(if field == Field::Larger {
                Filter::Larger(bytes)
            } else {
                Filter::Smaller(bytes)
            }))
        }
    }
}

/// Parse what was typed in the search box. Never fails.
#[must_use]
pub fn parse(input: &str, ctx: SearchContext) -> ParsedSearch {
    let mut out = ParsedSearch::default();
    let tokens = tokens(input);
    // Whether the last term pushed was followed by a usable OR.
    let mut join_next = false;
    // Whether the token before was a term that couldn't be used.
    let mut failed = false;
    for (i, token) in tokens.iter().enumerate() {
        if out.terms.len() >= MAX_TERMS {
            break;
        }
        if token.or && failed {
            // "is:nope OR b": the OR has nothing to join.
            failed = false;
            continue;
        }
        failed = false;
        if token.or {
            let follows = tokens.get(i.saturating_add(1)).is_some_and(|t| !t.or);
            if !out.terms.is_empty() && follows && !join_next {
                join_next = true;
                continue;
            }
            // Not between two terms: the word itself.
        }
        let made = if token.or {
            Ok(Some(Filter::Text {
                text: token.value.clone(),
                phrase: false,
            }))
        } else {
            filter(token, ctx)
        };
        match made {
            Ok(Some(filter)) => {
                let condition = Condition {
                    filter,
                    negated: token.negated,
                };
                let term = SearchTerm {
                    field: token.field.unwrap_or(Field::Text),
                    value: clean(&token.value),
                    negated: token.negated,
                    or_with_next: false,
                };
                match (join_next, out.groups.last_mut(), out.terms.last_mut()) {
                    (true, Some(group), Some(previous)) => {
                        previous.or_with_next = true;
                        group.push(condition);
                    }
                    _ => out.groups.push(vec![condition]),
                }
                out.terms.push(term);
            }
            Ok(None) => {}
            Err(error) => {
                failed = true;
                if !out.errors.contains(&error) {
                    out.errors.push(error);
                }
            }
        }
        join_next = false;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn ctx() -> SearchContext {
        // 2026-11-27 12:00 in Nairobi (UTC+3).
        SearchContext::at(1_795_770_000, 3 * 3600)
    }

    fn conditions(input: &str) -> Vec<Vec<Condition>> {
        parse(input, ctx()).groups
    }

    fn one(input: &str) -> Condition {
        let mut groups = conditions(input);
        assert_eq!(groups.len(), 1, "{input}: {groups:?}");
        let mut group = groups.remove(0);
        assert_eq!(group.len(), 1, "{input}");
        group.remove(0)
    }

    fn text(t: &str, phrase: bool) -> Filter {
        Filter::Text {
            text: t.into(),
            phrase,
        }
    }

    #[test]
    fn the_context_knows_the_local_day() {
        assert_eq!(ctx().today, NaiveDate::from_ymd_opt(2026, 11, 27).unwrap());
        // Just after midnight in Nairobi is still the day before in UTC.
        let early = SearchContext::at(1_795_726_800 + 60, 3 * 3600);
        assert_eq!(early.today, NaiveDate::from_ymd_opt(2026, 11, 27).unwrap());
    }

    #[test]
    fn plain_words_and_phrases() {
        let parsed = parse("invoice  \"march report\" q3", ctx());
        assert!(parsed.errors.is_empty());
        assert_eq!(
            parsed.groups,
            vec![
                vec![Condition {
                    filter: text("invoice", false),
                    negated: false
                }],
                vec![Condition {
                    filter: text("march report", true),
                    negated: false
                }],
                vec![Condition {
                    filter: text("q3", false),
                    negated: false
                }],
            ]
        );
        assert_eq!(parsed.terms[1].field, Field::Text);
        assert_eq!(parsed.terms[1].value, "march report");
        // An unclosed quote runs to the end.
        assert_eq!(one("\"half open").filter, text("half open", true));
    }

    #[test]
    fn negation() {
        let c = one("-newsletter");
        assert!(c.negated);
        assert_eq!(c.filter, text("newsletter", false));
        let c = one("-from:boss@x.co");
        assert!(c.negated);
        assert_eq!(c.filter, Filter::From("boss@x.co".into()));
        let c = one("-\"weekly digest\"");
        assert!(c.negated && c.filter == text("weekly digest", true));
        // A lone dash and a hyphenated word.
        assert!(conditions("- ").is_empty());
        assert_eq!(one("e-mail").filter, text("e-mail", false));
        let parsed = parse("-from:x", ctx());
        assert_eq!(
            serde_json::to_value(&parsed.terms[0]).unwrap(),
            serde_json::json!({ "field": "from", "value": "x", "negated": true })
        );
    }

    #[test]
    fn word_operators() {
        assert_eq!(one("from:jane").filter, Filter::From("jane".into()));
        assert_eq!(one("FROM:Jane").filter, Filter::From("Jane".into()));
        assert_eq!(
            one("to:peter@client.example").filter,
            Filter::To("peter@client.example".into())
        );
        assert_eq!(one("cc:carol").filter, Filter::Cc("carol".into()));
        assert_eq!(
            one("subject:invoice").filter,
            Filter::Subject("invoice".into())
        );
        assert_eq!(one("body:mpesa").filter, Filter::Body("mpesa".into()));
        assert_eq!(
            one("subject:\"quarterly report\"").filter,
            Filter::Subject("quarterly report".into())
        );
        let parsed = parse("from:", ctx());
        assert!(parsed.groups.is_empty());
        assert_eq!(parsed.errors, ["Type something after from:"]);
        let parsed = parse("from:@@", ctx());
        assert_eq!(
            parsed.errors,
            ["from: needs a word or address to look for."]
        );
    }

    #[test]
    fn unknown_operators_are_plain_text() {
        assert_eq!(one("foo:bar").filter, text("foo:bar", false));
        assert_eq!(
            one("https://example.com/x").filter,
            text("https://example.com/x", false)
        );
        assert_eq!(one("10:30").filter, text("10:30", false));
        assert!(parse("re: hello", ctx()).errors.is_empty());
    }

    #[test]
    fn states_and_attachments() {
        assert_eq!(one("has:attachment").filter, Filter::HasAttachment);
        assert_eq!(one("has:Attachments").filter, Filter::HasAttachment);
        assert_eq!(one("is:unread").filter, Filter::Is(State::Unread));
        assert_eq!(one("is:read").filter, Filter::Is(State::Read));
        assert_eq!(one("is:flagged").filter, Filter::Is(State::Flagged));
        assert_eq!(one("is:starred").filter, Filter::Is(State::Flagged));
        assert_eq!(one("is:pinned").filter, Filter::Is(State::Pinned));
        assert_eq!(one("is:answered").filter, Filter::Is(State::Answered));
        let c = one("-is:read");
        assert!(c.negated && c.filter == Filter::Is(State::Read));
        let parsed = parse("is:sleepy has:cats", ctx());
        assert!(parsed.groups.is_empty());
        assert_eq!(
            parsed.errors,
            [
                "is: can be unread, read, flagged, pinned or answered.",
                "has: only understands has:attachment."
            ]
        );
    }

    #[test]
    fn categories_keep_their_spaces_when_quoted() {
        assert_eq!(
            one("category:Clients").filter,
            Filter::Category("Clients".into())
        );
        assert_eq!(
            one("category:\"Red team\"").filter,
            Filter::Category("Red team".into())
        );
        let parsed = parse("category:\"Red team\" urgent", ctx());
        assert_eq!(parsed.groups.len(), 2);
        assert_eq!(parsed.terms[0].value, "Red team");
    }

    #[test]
    fn dates_are_local_days() {
        // Midnight 2026-10-01 in Nairobi is 21:00 UTC the day before.
        let oct1 = 1_790_802_000;
        assert_eq!(one("after:2026-10-01").filter, Filter::After(oct1));
        assert_eq!(one("before:2026/10/01").filter, Filter::Before(oct1));
        let today = 1_795_726_800;
        assert_eq!(one("after:today").filter, Filter::After(today));
        assert_eq!(one("before:Today").filter, Filter::Before(today));
        assert_eq!(one("after:yesterday").filter, Filter::After(today - 86_400));
        for bad in ["after:2026-13-01", "after:soon", "before:01-10-2026"] {
            let parsed = parse(bad, ctx());
            assert!(parsed.groups.is_empty(), "{bad}");
            assert_eq!(parsed.errors.len(), 1, "{bad}");
            assert!(
                parsed.errors[0].contains("needs a date like 2026-10-01"),
                "{bad}"
            );
        }
        assert!(parse("before:x", ctx()).errors[0].starts_with("before:"));
        let utc = SearchContext::at(1_795_770_000, 0);
        assert_eq!(
            parse("after:2026-10-01", utc).groups[0][0].filter,
            Filter::After(1_790_812_800)
        );
    }

    #[test]
    fn folders_by_role_or_name() {
        assert_eq!(
            one("in:inbox").filter,
            Filter::In(Place::Role(FolderRole::Inbox))
        );
        assert_eq!(
            one("in:Sent").filter,
            Filter::In(Place::Role(FolderRole::Sent))
        );
        assert_eq!(
            one("in:drafts").filter,
            Filter::In(Place::Role(FolderRole::Drafts))
        );
        assert_eq!(
            one("in:archive").filter,
            Filter::In(Place::Role(FolderRole::Archive))
        );
        assert_eq!(
            one("in:spam").filter,
            Filter::In(Place::Role(FolderRole::Junk))
        );
        assert_eq!(
            one("in:junk").filter,
            Filter::In(Place::Role(FolderRole::Junk))
        );
        assert_eq!(
            one("in:trash").filter,
            Filter::In(Place::Role(FolderRole::Trash))
        );
        assert_eq!(
            one("in:Clients").filter,
            Filter::In(Place::Name("Clients".into()))
        );
        assert_eq!(
            one("in:\"Big Projects\"").filter,
            Filter::In(Place::Name("Big Projects".into()))
        );
        assert!(parse("in:trash invoice", ctx()).picks_folders());
        assert!(parse("in:Clients", ctx()).picks_folders());
        assert!(!parse("-in:trash", ctx()).picks_folders());
        assert!(!parse("invoice", ctx()).picks_folders());
    }

    #[test]
    fn sizes() {
        assert_eq!(one("larger:500k").filter, Filter::Larger(512_000));
        assert_eq!(one("larger:2m").filter, Filter::Larger(2_097_152));
        assert_eq!(one("larger:2MB").filter, Filter::Larger(2_097_152));
        assert_eq!(one("smaller:1.5m").filter, Filter::Smaller(1_572_864));
        assert_eq!(one("smaller:1g").filter, Filter::Smaller(1_073_741_824));
        assert_eq!(one("smaller:800").filter, Filter::Smaller(800));
        assert_eq!(one("larger:10kb").filter, Filter::Larger(10_240));
        for bad in [
            "larger:big",
            "larger:k",
            "smaller:-5",
            "larger:1.2.3m",
            "larger:9999999g",
        ] {
            let parsed = parse(bad, ctx());
            assert!(parsed.groups.is_empty(), "{bad}");
            assert_eq!(parsed.errors.len(), 1, "{bad}");
        }
    }

    #[test]
    fn or_joins_two_terms() {
        let parsed = parse("from:jane OR from:peter invoice", ctx());
        assert_eq!(
            parsed.groups,
            vec![
                vec![
                    Condition {
                        filter: Filter::From("jane".into()),
                        negated: false
                    },
                    Condition {
                        filter: Filter::From("peter".into()),
                        negated: false
                    },
                ],
                vec![Condition {
                    filter: text("invoice", false),
                    negated: false
                }],
            ]
        );
        assert!(parsed.terms[0].or_with_next);
        assert!(!parsed.terms[1].or_with_next);
        assert_eq!(
            serde_json::to_value(&parsed.terms[0]).unwrap()["orWithNext"],
            serde_json::json!(true)
        );
        // Chains.
        assert_eq!(conditions("a OR b OR c")[0].len(), 3);
        // Not between two terms, or lower case: just a word.
        assert_eq!(
            conditions("a OR"),
            vec![
                vec![Condition {
                    filter: text("a", false),
                    negated: false
                }],
                vec![Condition {
                    filter: text("OR", false),
                    negated: false
                }],
            ]
        );
        assert_eq!(conditions("OR a").len(), 2);
        assert_eq!(conditions("a or b").len(), 3);
        assert_eq!(conditions("OR").len(), 1);
        // A failed term doesn't swallow the next.
        let parsed = parse("is:nope OR b", ctx());
        assert_eq!(parsed.groups.len(), 1);
        assert_eq!(parsed.errors.len(), 1);
    }

    #[test]
    fn hostile_text_is_kept_as_words() {
        assert_eq!(one("a\u{0}b").filter, text("ab", false));
        assert!(conditions("*** ( ) ^").is_empty());
        assert!(parse("", ctx()).groups.is_empty());
        assert!(parse("   ", ctx()).terms.is_empty());
        let long = "word ".repeat(100);
        assert_eq!(parse(&long, ctx()).terms.len(), MAX_TERMS);
        let huge = "x".repeat(10_000);
        match &one(&huge).filter {
            Filter::Text { text, .. } => assert_eq!(text.chars().count(), MAX_VALUE),
            other => panic!("{other:?}"),
        }
    }

    proptest! {
        #[test]
        fn never_panics_and_terms_match_groups(input in ".{0,80}") {
            let parsed = parse(&input, ctx());
            let in_groups: usize = parsed.groups.iter().map(Vec::len).sum();
            prop_assert_eq!(in_groups, parsed.terms.len());
            prop_assert!(parsed.terms.len() <= MAX_TERMS);
            for term in &parsed.terms {
                prop_assert!(!term.value.chars().any(char::is_control));
            }
        }
    }
}
