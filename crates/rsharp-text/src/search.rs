//! Finding every match of a query, as Rider's find bar does, and what each one is replaced with. A regex that does
//! not parse finds nothing and says why.

use crate::{Edit, Rope};
use regex::{Regex, RegexBuilder};
use std::ops::Range;

/// The most matches a search reports, so a one-letter query in a huge file stays quick; past it the count says so.
pub const MAX_MATCHES: usize = 1000;

/// What a search looks for: the text, and Rider's three toggles.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SearchQuery {
    pub text: String,
    pub match_case: bool,
    pub whole_words: bool,
    pub regex: bool,
}

/// Every match, sorted, whether there were more than were kept, and what is wrong with the query if anything is.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SearchResult {
    pub matches: Vec<Range<usize>>,
    pub truncated: bool,
    pub problem: Option<String>,
}

impl SearchResult {
    /// The first match at or after `offset`, wrapping to the first.
    pub fn next_from(&self, offset: usize) -> Option<usize> {
        let at = self.matches.iter().position(|m| m.start >= offset);
        at.or((!self.matches.is_empty()).then_some(0))
    }

    /// The last match before `offset`, wrapping to the last.
    pub fn previous_from(&self, offset: usize) -> Option<usize> {
        self.matches.iter().rposition(|m| m.start < offset).or(self.matches.len().checked_sub(1))
    }
}

fn build(query: &SearchQuery) -> Result<Regex, String> {
    let pattern = if query.regex { query.text.clone() } else { regex::escape(&query.text) };
    RegexBuilder::new(&pattern).case_insensitive(!query.match_case).multi_line(true).build().map_err(|e| {
        // The crate's message draws the pattern with a pointer under it; its last line is the reason.
        let reason = e.to_string().lines().last().unwrap_or_default().trim_start_matches("error: ").to_string();
        let mut chars = reason.chars();
        chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or(reason)
    })
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether a match stands as a whole word: no word character just before it or just after it.
fn whole_word(haystack: &str, range: &Range<usize>) -> bool {
    !haystack[..range.start].chars().next_back().is_some_and(is_word)
        && !haystack[range.end..].chars().next().is_some_and(is_word)
}

/// Every match of `query`, the first `limit` of them.
pub fn find_all(text: &Rope, query: &SearchQuery, limit: usize) -> SearchResult {
    if query.text.is_empty() {
        return SearchResult::default();
    }
    let pattern = match build(query) {
        Ok(pattern) => pattern,
        Err(problem) => return SearchResult { problem: Some(problem), ..Default::default() },
    };
    // shortcut: the text is copied out and searched whole on each change while the bar is open, as Passenger
    // does; search the rope's chunks off the UI thread once a large file makes typing with the bar open felt.
    let haystack = text.to_string();
    let mut result = SearchResult::default();
    for found in matches(&pattern, &haystack, query) {
        if result.matches.len() == limit {
            result.truncated = true;
            break;
        }
        result.matches.push(found);
    }
    result
}

fn matches<'a>(
    pattern: &'a Regex,
    haystack: &'a str,
    query: &'a SearchQuery,
) -> impl Iterator<Item = Range<usize>> + 'a {
    // Rider's `^` (Java's) is every line's start but the empty end after the last break, so prefixing every
    // line leaves a file's last newline as it was.
    // shortcut: any empty match there is dropped when the pattern has a `^`, which also drops a `$` alternative's;
    // read the pattern's anchors once a person meets it.
    let caret_at_end = |m: &Range<usize>| {
        m.is_empty() && m.start == haystack.len() && haystack.ends_with('\n') && query.regex && query.text.contains('^')
    };
    pattern
        .find_iter(haystack)
        .map(|m| m.range())
        .filter(move |m| !caret_at_end(m))
        // shortcut: whole words are checked after matching, so a match that is not a whole word can hide one
        // overlapping it (Rider's lookarounds would not); rare outside regex alternations.
        .filter(move |m| !query.whole_words || whole_word(haystack, m))
}

/// What the match at `range` is replaced with: `with` as typed, or for a regex with its groups ($1, ${name})
/// filled in from that match where it stood.
pub fn replacement(text: &Rope, query: &SearchQuery, range: &Range<usize>, with: &str) -> String {
    if !query.regex {
        return with.to_string();
    }
    let Ok(pattern) = build(query) else { return with.to_string() };
    let haystack = text.to_string();
    let mut out = String::new();
    match pattern.captures_at(&haystack, range.start) {
        Some(captures) if captures.get(0).is_some_and(|m| m.range() == *range) => {
            captures.expand(with, &mut out);
            out
        }
        _ => with.to_string(),
    }
}

/// Replace All: every match's replacement, as edits of the text searched.
pub fn replace_all(text: &Rope, query: &SearchQuery, with: &str) -> Vec<Edit> {
    let Ok(pattern) = build(query) else { return Vec::new() };
    if query.text.is_empty() {
        return Vec::new();
    }
    let haystack = text.to_string();
    let found: Vec<Range<usize>> = matches(&pattern, &haystack, query).collect();
    found
        .into_iter()
        .map(|range| {
            let mut replaced = String::new();
            match pattern.captures_at(&haystack, range.start) {
                Some(captures) if query.regex => captures.expand(with, &mut replaced),
                _ => replaced.push_str(with),
            }
            Edit { range, text: replaced }
        })
        .collect()
}
