//! Turning what the user typed into a safe FTS5 query.
//!
//! User text is never handed to `MATCH` as is: `"`, `*`, `NEAR(`, `OR`,
//! column filters and the like would be FTS5 syntax. Instead each word is
//! stripped of double quotes and control characters (FTS5 stops reading a
//! string at NUL), wrapped in quotes as a literal phrase, and
//! marked as a prefix so results appear while typing. All words must match.

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
