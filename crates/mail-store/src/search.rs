//! Turning what the user typed into a safe FTS5 query.
//!
//! User text is never handed to `MATCH` as is: `"`, `*`, `NEAR(`, `OR`,
//! column filters and the like would be FTS5 syntax. Instead each word is
//! stripped of double quotes and control characters (FTS5 stops reading a
//! string at NUL), wrapped in quotes as a literal phrase, and
//! marked as a prefix so results appear while typing. All words must match.
//!
//! The query language (`mail_core::search`) adds operators; [`selection`]
//! turns them into conditions on bound parameters (`?9` on) next to the
//! message list's fixed selectors, and widens the view for the search
//! scope.

use std::fmt::Write as _;

use mail_core::search::{Condition, Filter, Place, SearchContext, SearchScope, State, parse};
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Result;
use crate::folders::role_str;
use crate::messages::ListQuery;

/// What a search box value means for the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SearchTerms {
    /// No search: list everything.
    None,
    /// Text was typed but has nothing searchable (only punctuation): match
    /// nothing.
    Nothing,
    /// An FTS5 `MATCH` expression built only from quoted literals.
    Match(String),
}

/// Most words used from one search, so a pasted essay stays cheap.
const MAX_TERMS: usize = 16;

/// Build a safe FTS5 query from user input.
pub(crate) fn fts_query(input: Option<&str>) -> SearchTerms {
    let Some(input) = input.filter(|s| !s.trim().is_empty()) else {
        return SearchTerms::None;
    };
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|c| *c != '"' && !c.is_control())
                .collect::<String>()
        })
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .take(MAX_TERMS)
        .map(|word| format!("\"{word}\"*"))
        .collect();
    if terms.is_empty() {
        SearchTerms::Nothing
    } else {
        SearchTerms::Match(terms.join(" "))
    }
}

/// One word or phrase as an FTS5 literal: double quotes and control
/// characters dropped, `*` after it for a prefix. `None` when nothing
/// searchable is left.
fn literal(text: &str, prefix: bool) -> Option<String> {
    let clean: String = text
        .chars()
        .filter(|c| *c != '"' && !c.is_control())
        .collect();
    if !clean.chars().any(char::is_alphanumeric) {
        return None;
    }
    Some(if prefix {
        format!("\"{clean}\"*")
    } else {
        format!("\"{clean}\"")
    })
}

/// The message list's selectors after the search box is read: the fixed
/// `?1`..`?8` of `list_where!` plus extra conditions on `?9` on.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Selection {
    /// `?1`.
    pub account: Option<String>,
    /// `?2`.
    pub folder: Option<String>,
    /// `?3`.
    pub role: Option<&'static str>,
    /// `?5`: the plain words, one FTS5 query.
    pub fts: Option<String>,
    /// `?8`.
    pub focus: Option<&'static str>,
    /// `AND (...)` conditions, built only from fixed pieces and `?N`.
    pub sql: String,
    /// Their values, `?9` on.
    pub values: Vec<Value>,
}

impl Selection {
    fn bind(&mut self, value: Value) -> String {
        self.values.push(value);
        format!("?{}", self.values.len().saturating_add(8))
    }

    /// Every value in order: `?1`..`?8` (`limit` and `offset` are `?6`
    /// and `?7`), then the extras.
    pub fn params(&self, filter: i64, limit: i64, offset: i64) -> Vec<Value> {
        let text = |v: Option<&str>| v.map_or(Value::Null, |s| Value::Text(s.to_owned()));
        let mut out = vec![
            text(self.account.as_deref()),
            text(self.folder.as_deref()),
            text(self.role),
            Value::Integer(filter),
            text(self.fts.as_deref()),
            Value::Integer(limit),
            Value::Integer(offset),
            text(self.focus),
        ];
        out.extend(self.values.iter().cloned());
        out
    }

    /// The FTS5 subquery for one literal on some columns (`None`: all).
    fn fts(&mut self, columns: Option<&str>, text: &str, prefix: bool) -> String {
        let Some(literal) = literal(text, prefix) else {
            return "0".to_owned();
        };
        let expr = match columns {
            Some(columns) => format!("{columns} : {literal}"),
            None => literal,
        };
        let p = self.bind(Value::Text(expr));
        format!("m.seq IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH {p})")
    }

    fn condition(&mut self, c: &Condition, categories: &[String]) -> String {
        let int = |n: i64| Value::Integer(n);
        let sql = match &c.filter {
            Filter::Text { text, phrase } => self.fts(None, text, !phrase),
            Filter::From(v) => self.fts(Some("sender"), v, true),
            Filter::To(v) => self.fts(Some("recipients"), v, true),
            Filter::Cc(v) => self.fts(Some("cc"), v, true),
            Filter::Subject(v) => self.fts(Some("subject"), v, true),
            Filter::Body(v) => self.fts(Some("{body preview}"), v, true),
            Filter::HasAttachment => "m.has_attachments = 1".to_owned(),
            Filter::Is(state) => match state {
                State::Unread => "m.seen = 0",
                State::Read => "m.seen = 1",
                State::Flagged => "m.flagged = 1",
                State::Pinned => "m.pinned = 1",
                State::Answered => "m.answered = 1",
            }
            .to_owned(),
            Filter::Category(name) => {
                // As the category is spelled, whatever case was typed.
                let spelled = categories
                    .iter()
                    .find(|c| mail_core::categories::same_name(c, name))
                    .unwrap_or(name);
                let p = self.bind(Value::Text(spelled.clone()));
                format!("EXISTS (SELECT 1 FROM json_each(m.categories) mc WHERE mc.value = {p})")
            }
            Filter::Before(at) => format!("m.date < {}", self.bind(int(*at))),
            Filter::After(at) => format!("m.date >= {}", self.bind(int(*at))),
            Filter::In(Place::Role(role)) => {
                format!(
                    "f.role = {}",
                    self.bind(Value::Text(role_str(*role).to_owned()))
                )
            }
            Filter::In(Place::Name(name)) => {
                let p = self.bind(Value::Text(name.clone()));
                format!("(f.name = {p} COLLATE NOCASE OR f.path = {p} COLLATE NOCASE)")
            }
            Filter::Larger(bytes) => {
                let n = i64::try_from(*bytes).unwrap_or(i64::MAX);
                format!("m.size > {}", self.bind(int(n)))
            }
            Filter::Smaller(bytes) => {
                let n = i64::try_from(*bytes).unwrap_or(i64::MAX);
                format!("m.size < {}", self.bind(int(n)))
            }
        };
        // NULL (an undated message) counts as no match either way.
        if c.negated {
            format!("NOT coalesce(({sql}), 0)")
        } else {
            format!("coalesce(({sql}), 0)")
        }
    }
}

/// Most folders a "this folder and below" search lists by id.
const MAX_SCOPE_FOLDERS: usize = 2000;

/// A folder: id, account, path and delimiter.
type ViewFolder = (String, String, String, Option<String>);

/// Folders matching the view's selectors.
fn view_folders(conn: &Connection, query: &ListQuery) -> Result<Vec<ViewFolder>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, account_id, path, delimiter FROM folders
         WHERE (?1 IS NULL OR account_id = ?1) AND (?2 IS NULL OR id = ?2)
           AND (?3 IS NULL OR role = ?3)",
    )?;
    let rows = stmt.query_map(
        params![query.account_id, query.folder_id, query.role.map(role_str)],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The view's folders and every folder below them, by path.
fn with_subfolders(conn: &Connection, query: &ListQuery) -> Result<Vec<String>> {
    let base = view_folders(conn, query)?;
    let mut ids: Vec<String> = Vec::new();
    let mut stmt = conn.prepare_cached(
        "SELECT id FROM folders WHERE account_id = ?1 AND substr(path, 1, length(?2)) = ?2",
    )?;
    for (id, account, path, delimiter) in base {
        ids.push(id);
        let Some(delimiter) = delimiter.filter(|d| !d.is_empty()) else {
            continue;
        };
        let prefix = format!("{path}{delimiter}");
        for child in stmt.query_map(params![account, prefix], |row| row.get::<_, String>(0))? {
            ids.push(child?);
        }
    }
    ids.sort();
    ids.dedup();
    ids.truncate(MAX_SCOPE_FOLDERS);
    Ok(ids)
}

/// Read the search box into the list's selectors. `None`: something was
/// typed but nothing in it can be searched, so nothing matches.
pub(crate) fn selection(conn: &Connection, query: &ListQuery) -> Result<Option<Selection>> {
    let mut sel = Selection {
        account: query.account_id.clone(),
        folder: query.folder_id.clone(),
        role: query.role.map(role_str),
        focus: query.focus.map(mail_core::focus::Focus::as_str),
        ..Selection::default()
    };
    let Some(input) = query.search.as_deref().filter(|s| !s.trim().is_empty()) else {
        return Ok(Some(sel));
    };
    let ctx = query.search_context.unwrap_or_else(|| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
        SearchContext::at(now, 0)
    });
    let parsed = parse(input, ctx);
    if parsed.groups.is_empty() {
        return Ok(None);
    }
    let picks = parsed.picks_folders();
    let scope = query.search_scope.unwrap_or_default();
    let wide = picks || matches!(scope, SearchScope::Account | SearchScope::All);
    if wide {
        // The view's account (or, in a unified view, every account).
        let account = match (scope, &query.account_id, &query.folder_id) {
            (SearchScope::All, _, _) | (_, None, None) => None,
            (_, Some(account), _) => Some(account.clone()),
            (_, None, Some(folder)) => conn
                .query_row(
                    "SELECT account_id FROM folders WHERE id = ?1",
                    params![folder],
                    |row| row.get(0),
                )
                .optional()?,
        };
        sel.account = account;
        sel.folder = None;
        sel.role = None;
        sel.focus = None;
        // Named folders are searched wherever they are; otherwise the bin
        // and spam stay out of a wide search.
        if !picks {
            sel.sql.push_str(" AND f.role NOT IN ('trash', 'junk')");
        }
    } else if scope == SearchScope::Subfolders
        && (query.folder_id.is_some() || query.role.is_some())
    {
        let ids = with_subfolders(conn, query)?;
        if ids.is_empty() {
            return Ok(None);
        }
        let marks: Vec<String> = ids
            .into_iter()
            .map(|id| sel.bind(Value::Text(id)))
            .collect();
        sel.folder = None;
        sel.role = None;
        let _ = write!(sel.sql, " AND m.folder_id IN ({})", marks.join(", "));
    }
    let categories: Vec<String> = conn
        .prepare_cached("SELECT name FROM categories")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut words: Vec<&str> = Vec::new();
    for group in &parsed.groups {
        if let [
            Condition {
                filter:
                    Filter::Text {
                        text,
                        phrase: false,
                    },
                negated: false,
            },
        ] = group.as_slice()
        {
            words.push(text);
            continue;
        }
        let parts: Vec<String> = group
            .iter()
            .map(|c| sel.condition(c, &categories))
            .collect();
        let _ = write!(sel.sql, " AND ({})", parts.join(" OR "));
    }
    match fts_query(Some(&words.join(" "))) {
        SearchTerms::Match(expr) => sel.fts = Some(expr),
        SearchTerms::None | SearchTerms::Nothing => {}
    }
    Ok(Some(sel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn blank_is_no_filter() {
        assert_eq!(fts_query(None), SearchTerms::None);
        assert_eq!(fts_query(Some("")), SearchTerms::None);
        assert_eq!(fts_query(Some("  \t\n")), SearchTerms::None);
    }

    #[test]
    fn words_become_quoted_prefixes() {
        assert_eq!(
            fts_query(Some("invoice  march")),
            SearchTerms::Match("\"invoice\"* \"march\"*".into())
        );
    }

    #[test]
    fn syntax_is_neutralised() {
        assert_eq!(
            fts_query(Some("a OR NEAR(b \"c\" subject:d")),
            SearchTerms::Match("\"a\"* \"OR\"* \"NEAR(b\"* \"c\"* \"subject:d\"*".into())
        );
        assert_eq!(fts_query(Some("\"")), SearchTerms::Nothing);
        assert_eq!(fts_query(Some("* - ( )")), SearchTerms::Nothing);
        assert_eq!(
            fts_query(Some("\0a\u{7}b")),
            SearchTerms::Match("\"ab\"*".into())
        );
    }

    #[test]
    fn keeps_unicode_words() {
        assert_eq!(
            fts_query(Some("Grüße 日本語")),
            SearchTerms::Match("\"Grüße\"* \"日本語\"*".into())
        );
    }

    #[test]
    fn caps_the_number_of_words() {
        let long = "word ".repeat(100);
        let SearchTerms::Match(query) = fts_query(Some(&long)) else {
            panic!("expected a match");
        };
        assert_eq!(query.matches('*').count(), MAX_TERMS);
    }

    proptest! {
        #[test]
        fn output_never_contains_a_stray_quote(input in ".{0,60}") {
            if let SearchTerms::Match(query) = fts_query(Some(&input)) {
                for term in query.split(' ') {
                    prop_assert!(term.starts_with('"') && term.ends_with("\"*"));
                    let inner = term.trim_start_matches('"').trim_end_matches("\"*");
                    prop_assert!(!inner.contains('"'));
                }
            }
        }
    }
}
