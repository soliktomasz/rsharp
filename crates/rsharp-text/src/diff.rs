//! Two texts' lines side by side in one list, as a diff shows them: those both have, and those only one has.

use imara_diff::intern::InternedInput;
use imara_diff::{Algorithm, diff};
use std::ops::Range;

/// A replacement of zero-based lines, including their own line endings.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DiffHunk {
    pub before: Range<usize>,
    pub after: Range<usize>,
}

/// A replacement inside changed lines, with UTF-8 byte offsets on each side.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InlineDiff {
    pub before: Range<usize>,
    pub after: Range<usize>,
}

/// Changed stretches of lines. A missing final newline differs from a present one.
pub fn diff_hunks(before: &str, after: &str) -> Vec<DiffHunk> {
    // imara's default string tokenizer omits line endings; Git must also show a missing final newline.
    token_hunks(&before.split_inclusive('\n').collect::<Vec<_>>(), &after.split_inclusive('\n').collect::<Vec<_>>())
}

fn collect_hunks<T>(input: &InternedInput<T>) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    diff(Algorithm::Histogram, input, |before: Range<u32>, after: Range<u32>| {
        hunks.push(DiffHunk {
            before: before.start as usize..before.end as usize,
            after: after.start as usize..after.end as usize,
        });
    });
    hunks
}

pub(crate) fn token_hunks<'a>(before: &[&'a str], after: &[&'a str]) -> Vec<DiffHunk> {
    let mut input = InternedInput::default();
    input.update_before(before.iter().copied());
    input.update_after(after.iter().copied());
    collect_hunks(&input)
}

/// Restores one hunk in `after` from `before`, suitable for one undoable document edit.
pub fn rollback_hunk(before: &str, after: &str, hunk: &DiffHunk) -> crate::Edit {
    let old = line_starts(before);
    let new = line_starts(after);
    crate::Edit {
        range: new[hunk.after.start]..new[hunk.after.end],
        text: before[old[hunk.before.start]..old[hunk.before.end]].to_owned(),
    }
}

pub(crate) fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for line in text.split_inclusive('\n') {
        starts.push(starts.last().unwrap() + line.len());
    }
    starts
}

/// Marks changed words and punctuation with byte ranges; huge or whole-fragment rewrites need line tint alone.
pub fn word_diff(before: &str, after: &str) -> Vec<InlineDiff> {
    let old = word_chunks(before);
    let new = word_chunks(after);
    if old.len() + new.len() > 20_000 {
        return Vec::new();
    }
    let old_words: Vec<_> = old.iter().map(|r| &before[r.clone()]).collect();
    let new_words: Vec<_> = new.iter().map(|r| &after[r.clone()]).collect();
    let hunks = token_hunks(&old_words, &new_words);
    let mut found = Vec::new();
    let (mut i, mut j, mut old_from, mut new_from) = (0, 0, 0, 0);
    for hunk in hunks {
        while i < hunk.before.start {
            word_gap(before, after, old_from..old[i].start, new_from..new[j].start, &mut found);
            old_from = old[i].end;
            new_from = new[j].end;
            i += 1;
            j += 1;
        }
        i = hunk.before.end;
        j = hunk.after.end;
    }
    while i < old.len() && j < new.len() {
        word_gap(before, after, old_from..old[i].start, new_from..new[j].start, &mut found);
        old_from = old[i].end;
        new_from = new[j].end;
        i += 1;
        j += 1;
    }
    word_gap(before, after, old_from..before.len(), new_from..after.len(), &mut found);
    if found.len() == 1 && found[0].before == (0..before.len()) && found[0].after == (0..after.len()) {
        found.clear();
    }
    found
}

fn word_gap(before: &str, after: &str, mut old: Range<usize>, mut new: Range<usize>, found: &mut Vec<InlineDiff>) {
    while old.start < old.end && new.start < new.end {
        let a = before[old.clone()].chars().next().unwrap();
        let b = after[new.clone()].chars().next().unwrap();
        if a != b || word_part(a) {
            break;
        }
        old.start += a.len_utf8();
        new.start += b.len_utf8();
    }
    while old.start < old.end && new.start < new.end {
        let a = before[old.clone()].chars().next_back().unwrap();
        let b = after[new.clone()].chars().next_back().unwrap();
        if a != b || word_part(a) {
            break;
        }
        old.end -= a.len_utf8();
        new.end -= b.len_utf8();
    }
    if !old.is_empty() || !new.is_empty() {
        found.push(InlineDiff { before: old, after: new });
    }
}

fn word_chunks(text: &str) -> Vec<Range<usize>> {
    let mut chunks = Vec::new();
    let mut word = None;
    for (at, c) in text.char_indices() {
        let continuous = matches!(c, '\u{0e00}'..='\u{0e7f}' | '\u{3040}'..='\u{30ff}'
            | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}');
        if word_part(c) && !continuous {
            word.get_or_insert(at);
        } else {
            if let Some(start) = word.take() {
                chunks.push(start..at);
            }
            if word_part(c) || c == '\n' {
                chunks.push(at..at + c.len_utf8());
            }
        }
    }
    if let Some(start) = word {
        chunks.push(start..text.len());
    }
    chunks
}

fn word_part(c: char) -> bool {
    !c.is_whitespace() && (!c.is_ascii_punctuation() || c == '_')
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineChange {
    Same,
    /// Only in the text before.
    Removed,
    /// Only in the text after.
    Added,
}

/// Every line of `before` and `after` in order, each with whether both have it, its break left off.
pub fn line_diff<'a>(before: &'a str, after: &'a str) -> Vec<(LineChange, &'a str)> {
    // imara-diff counts lines as split_inclusive does, so the indices agree.
    let old: Vec<&str> = before.split_inclusive('\n').collect();
    let new: Vec<&str> = after.split_inclusive('\n').collect();
    let input = InternedInput::new(before, after);
    let mut hunks: Vec<(Range<u32>, Range<u32>)> = Vec::new();
    diff(Algorithm::Histogram, &input, |b: Range<u32>, a: Range<u32>| hunks.push((b, a)));
    let line = |s: &'a str| s.strip_suffix('\n').unwrap_or(s);
    let mut out = Vec::with_capacity(old.len().max(new.len()));
    let mut at = 0;
    for (b, a) in hunks {
        out.extend(old[at..b.start as usize].iter().map(|l| (LineChange::Same, line(l))));
        out.extend(old[b.start as usize..b.end as usize].iter().map(|l| (LineChange::Removed, line(l))));
        out.extend(new[a.start as usize..a.end as usize].iter().map(|l| (LineChange::Added, line(l))));
        at = b.end as usize;
    }
    out.extend(old[at..].iter().map(|l| (LineChange::Same, line(l))));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_line_shows_as_removed_then_added_between_the_lines_both_have() {
        let lines = line_diff("a\nb\nc\n", "a\nB\nc\nd\n");
        assert_eq!(
            lines,
            [
                (LineChange::Same, "a"),
                (LineChange::Removed, "b"),
                (LineChange::Added, "B"),
                (LineChange::Same, "c"),
                (LineChange::Added, "d"),
            ]
        );
    }
}
