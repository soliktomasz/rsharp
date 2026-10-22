//! Commands that change only the carets: occurrences, clones and columns. The last caret is the primary, the one
//! added last; none makes more than [`MAX_CARETS`].

use crate::{Rope, Selection, column_of, line_end, offset_at};

/// As Rider's: past this many carets an editor stops adding them.
pub const MAX_CARETS: usize = 1000;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The identifier at or touching `offset`, empty where there is none.
pub fn word_at(text: &Rope, offset: usize) -> Selection {
    let at = text.byte_to_char(offset);
    let mut start = at;
    while start > 0 && is_word(text.char(start - 1)) {
        start -= 1;
    }
    let mut end = at;
    while end < text.len_chars() && is_word(text.char(end)) {
        end += 1;
    }
    Selection { anchor: text.char_to_byte(start), head: text.char_to_byte(end) }
}

/// Select Next Occurrence: first the word under each empty caret, then the next copy of the primary selection
/// after the last caret, wrapping, as the new primary.
pub fn select_next_occurrence(text: &Rope, selections: &[Selection]) -> Vec<Selection> {
    if selections.iter().any(Selection::is_empty) {
        return selections.iter().map(|s| if s.is_empty() { word_at(text, s.head) } else { *s }).collect();
    }
    let primary = selections.last().expect("a document always has a caret");
    if selections.len() >= MAX_CARETS {
        return selections.to_vec();
    }
    // shortcut: the text is copied out to search it, a whole-file cost per press; search the rope's chunks once
    // a file this large makes the press felt.
    let haystack = text.to_string();
    let needle = &haystack[primary.range()];
    let from = selections.iter().map(Selection::end).max().unwrap_or(0);
    let taken = |at: &usize| selections.iter().all(|s| s.start() != *at);
    let found = haystack[from..]
        .match_indices(needle)
        .map(|(at, _)| from + at)
        .find(&taken)
        .or_else(|| haystack[..from].match_indices(needle).map(|(at, _)| at).find(&taken));
    let mut next = selections.to_vec();
    if let Some(at) = found {
        next.push(Selection { anchor: at, head: at + needle.len() });
    }
    next
}

/// Select All Occurrences: every copy of the primary selection, or of the word under the caret, the first
/// [`MAX_CARETS`] of them, the one the caret was on last, as the primary.
pub fn select_all_occurrences(text: &Rope, selections: &[Selection]) -> Vec<Selection> {
    let primary = *selections.last().expect("a document always has a caret");
    let range = if primary.is_empty() { word_at(text, primary.head).range() } else { primary.range() };
    if range.is_empty() {
        return selections.to_vec();
    }
    let haystack = text.to_string();
    let needle = &haystack[range.clone()];
    let mut all: Vec<Selection> = haystack
        .match_indices(needle)
        .map(|(at, _)| Selection { anchor: at, head: at + needle.len() })
        .filter(|s| s.start() != range.start)
        .take(MAX_CARETS - 1)
        .collect();
    all.push(Selection { anchor: range.start, head: range.end });
    all
}

/// Unselect Occurrence: drops the primary, the caret added last, while others remain.
pub fn unselect_last(selections: &[Selection]) -> Vec<Selection> {
    let keep = selections.len().saturating_sub(1).max(1);
    selections[..keep].to_vec()
}

/// Clone Caret Above or Below: a caret a line beyond the topmost (or bottommost) one, at its column.
pub fn clone_caret(text: &Rope, selections: &[Selection], lines: isize) -> Vec<Selection> {
    let edge =
        if lines < 0 { selections.iter().min_by_key(|s| s.head) } else { selections.iter().max_by_key(|s| s.head) };
    let edge = edge.expect("a document always has a caret");
    let line = text.byte_to_line(edge.head) as isize + lines;
    let mut next = selections.to_vec();
    if line >= 0 && (line as usize) < text.len_lines() && selections.len() < MAX_CARETS {
        let caret = Selection::caret(offset_at(text, line as usize, column_of(text, edge.head)));
        if !next.contains(&caret) {
            next.push(caret);
        }
    }
    next
}

/// Alt+drag: on every line from the press to the pointer, a selection across the same character columns, as many
/// lines as the editor keeps carets for, those toward the pointer; the line under the pointer is the primary.
pub fn column_selection(text: &Rope, from: (usize, usize), to: (usize, usize)) -> Vec<Selection> {
    let last = text.len_lines() - 1;
    let (from_line, to_line) = (from.0.min(last), to.0.min(last));
    let lines: Vec<usize> = if from_line <= to_line {
        (from_line.max(to_line.saturating_sub(MAX_CARETS - 1))..=to_line).collect()
    } else {
        (to_line..=from_line.min(to_line + MAX_CARETS - 1)).rev().collect()
    };
    // shortcut: a line shorter than the columns is selected to its end; Rider can put carets past it (virtual
    // space) once that is turned on.
    lines
        .into_iter()
        .map(|line| Selection { anchor: offset_at(text, line, from.1), head: offset_at(text, line, to.1) })
        .collect()
}

/// Carets after a move: those that came to touch or overlap are one, in the order of the text.
pub fn merge_overlapping(mut selections: Vec<Selection>) -> Vec<Selection> {
    if selections.len() < 2 {
        return selections;
    }
    selections.sort_by_key(|s| (s.start(), s.end()));
    let mut merged: Vec<Selection> = Vec::with_capacity(selections.len());
    for s in selections {
        let touches = merged.last().is_some_and(|last| {
            s.start() < last.end()
                || s == *last
                || (s.is_empty() && s.head == last.end())
                || (last.is_empty() && s.start() == last.head)
        });
        if !touches {
            merged.push(s);
            continue;
        }
        let last = merged.last_mut().expect("a caret it touches");
        let (start, end) = (last.start(), last.end().max(s.end()));
        let forward = last.head >= last.anchor;
        *last = if forward { Selection { anchor: start, head: end } } else { Selection { anchor: end, head: start } };
    }
    merged
}

/// Caret Per Line: a caret at the end of every line each selection covers, a selection's last line left out
/// where it ends at that line's start, on the first [`MAX_CARETS`] lines.
pub fn caret_per_line(text: &Rope, selections: &[Selection]) -> Vec<Selection> {
    let mut lines: Vec<usize> = selections
        .iter()
        .flat_map(|s| {
            let first = text.byte_to_line(s.start());
            let mut last = text.byte_to_line(s.end());
            if last > first && text.line_to_byte(last) == s.end() {
                last -= 1;
            }
            first..=last
        })
        .collect();
    lines.sort_unstable();
    lines.dedup();
    lines.into_iter().take(MAX_CARETS).map(|line| Selection::caret(line_end(text, line))).collect()
}
