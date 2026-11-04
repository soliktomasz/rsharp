//! The TODO window's scan: Rider's default patterns, `\btodo\b` and `\bfixme\b` without regard to case, matched
//! inside a file's comments only.

use std::ops::Range;

/// A TODO found: where its word starts (a byte of the text), which pattern, and the comment from it to its end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Todo {
    pub at: usize,
    pub kind: &'static str,
    pub text: String,
}

const PATTERNS: [(&str, &str); 2] = [("todo", "TODO"), ("fixme", "FIXME")];

/// Every TODO in `comments`, the byte ranges of the text's comments, in order.
pub fn scan(text: &str, comments: &[Range<usize>]) -> Vec<Todo> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut found = Vec::new();
    for range in comments {
        let Some(comment) = text.get(range.clone()) else { continue };
        let lowered = comment.to_ascii_lowercase();
        let hit = PATTERNS
            .iter()
            .flat_map(|(pattern, kind)| lowered.match_indices(pattern).map(move |(at, _)| (at, pattern.len(), *kind)))
            .filter(|(at, len, _)| {
                !comment[..*at].chars().next_back().is_some_and(word)
                    && !comment[at + len..].chars().next().is_some_and(word)
            })
            .min_by_key(|(at, ..)| *at);
        // One entry per comment, from its first pattern: the rest of the comment is that TODO's text.
        if let Some((at, _, kind)) = hit {
            let rest = comment[at..].trim_end();
            let rest = rest.strip_suffix("*/").unwrap_or(rest).trim_end();
            found.push(Todo { at: range.start + at, kind, text: rest.to_string() });
        }
    }
    found
}
