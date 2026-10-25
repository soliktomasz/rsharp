//! Three-way text comparison and explicit resolution, independent of files and UI.

use crate::diff::{line_starts, token_hunks};
use std::ops::Range;

/// One changed stretch, in zero-based line coordinates of the three original texts.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MergeChunk {
    pub base: Range<usize>,
    pub left: Range<usize>,
    pub right: Range<usize>,
    pub left_changed: bool,
    pub right_changed: bool,
    pub is_conflict: bool,
    pub resolved: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MergeChoice {
    Left,
    Right,
    Both,
    Base,
}

/// Nonconflicting changes apply automatically; conflicts keep their base until explicitly resolved.
#[derive(Clone, Debug)]
pub struct ThreeWayMerge {
    pub chunks: Vec<MergeChunk>,
    base: String,
    left: String,
    right: String,
    base_starts: Vec<usize>,
    left_starts: Vec<usize>,
    right_starts: Vec<usize>,
    replacements: Vec<String>,
}

impl ThreeWayMerge {
    pub fn new(base: &str, left: &str, right: &str) -> Self {
        let chunks = merge_chunks(
            &base.split_inclusive('\n').collect::<Vec<_>>(),
            &left.split_inclusive('\n').collect::<Vec<_>>(),
            &right.split_inclusive('\n').collect::<Vec<_>>(),
        );
        let mut merge = Self {
            chunks,
            base: base.to_owned(),
            left: left.to_owned(),
            right: right.to_owned(),
            base_starts: line_starts(base),
            left_starts: line_starts(left),
            right_starts: line_starts(right),
            replacements: Vec::new(),
        };
        merge.replacements = merge
            .chunks
            .iter()
            .map(|chunk| {
                let choice = if chunk.is_conflict {
                    MergeChoice::Base
                } else if chunk.left_changed {
                    MergeChoice::Left
                } else {
                    MergeChoice::Right
                };
                merge.chosen(chunk, choice)
            })
            .collect();
        merge
    }

    pub fn result(&self) -> String {
        let mut result = String::new();
        let mut at = 0;
        for (chunk, replacement) in self.chunks.iter().zip(&self.replacements) {
            result.push_str(&self.base[at..self.base_starts[chunk.base.start]]);
            result.push_str(replacement);
            at = self.base_starts[chunk.base.end];
        }
        result.push_str(&self.base[at..]);
        result
    }

    pub fn unresolved_count(&self) -> usize {
        self.chunks.iter().filter(|c| c.is_conflict && !c.resolved).count()
    }

    /// Chooses original chunk content, even after another choice has already been applied.
    pub fn accept(&mut self, index: usize, choice: MergeChoice) -> bool {
        let Some(chunk) = self.chunks.get(index) else {
            return false;
        };
        self.replace_chunk(index, self.chosen(chunk, choice))
    }

    /// Acknowledges a person's edited chunk without resolving any other conflict.
    pub fn replace_chunk(&mut self, index: usize, text: String) -> bool {
        let Some(chunk) = self.chunks.get_mut(index) else {
            return false;
        };
        self.replacements[index] = text;
        chunk.resolved = true;
        true
    }

    pub fn resolve_simple(&mut self, index: usize) -> bool {
        let Some(chunk) = self.chunks.get(index) else {
            return false;
        };
        if !chunk.is_conflict || chunk.resolved {
            return false;
        }
        let base = &self.base[self.base_starts[chunk.base.start]..self.base_starts[chunk.base.end]];
        let left = &self.left[self.left_starts[chunk.left.start]..self.left_starts[chunk.left.end]];
        let right = &self.right[self.right_starts[chunk.right.start]..self.right_starts[chunk.right.end]];
        let Some(result) = resolve_words(base, left, right) else {
            return false;
        };
        self.replace_chunk(index, result)
    }

    pub fn resolve_simple_conflicts(&mut self) -> usize {
        (0..self.chunks.len()).filter(|&i| self.resolve_simple(i)).count()
    }

    fn chosen(&self, chunk: &MergeChunk, choice: MergeChoice) -> String {
        let base = &self.base[self.base_starts[chunk.base.start]..self.base_starts[chunk.base.end]];
        let left = &self.left[self.left_starts[chunk.left.start]..self.left_starts[chunk.left.end]];
        let right = &self.right[self.right_starts[chunk.right.start]..self.right_starts[chunk.right.end]];
        match choice {
            MergeChoice::Base => base.to_owned(),
            MergeChoice::Left => left.to_owned(),
            MergeChoice::Right => right.to_owned(),
            MergeChoice::Both => {
                let line_break = [&self.base, &self.left, &self.right]
                    .into_iter()
                    .find_map(|text| {
                        text.find('\n').map(|at| if at > 0 && text.as_bytes()[at - 1] == b'\r' { "\r\n" } else { "\n" })
                    })
                    .unwrap_or("\n");
                let separator =
                    if !left.is_empty() && !right.is_empty() && !left.ends_with('\n') { line_break } else { "" };
                format!("{left}{separator}{right}")
            }
        }
    }
}

fn merge_chunks<'a>(base: &[&'a str], left: &[&'a str], right: &[&'a str]) -> Vec<MergeChunk> {
    let lefts = token_hunks(base, left);
    let rights = token_hunks(base, right);
    let (mut i, mut j, mut left_at, mut right_at, mut base_at) = (0, 0, 0, 0, 0);
    let mut chunks = Vec::new();
    while i < lefts.len() || j < rights.len() {
        let start = lefts
            .get(i)
            .map_or(usize::MAX, |h| h.before.start)
            .min(rights.get(j).map_or(usize::MAX, |h| h.before.start));
        let mut end = start;
        let (first_left, first_right) = (i, j);
        let (mut left_removed, mut left_added, mut right_removed, mut right_added) = (0, 0, 0, 0);
        loop {
            if let Some(h) = lefts.get(i).filter(|h| h.before.start <= end) {
                end = end.max(h.before.end);
                left_removed += h.before.len();
                left_added += h.after.len();
                i += 1;
            } else if let Some(h) = rights.get(j).filter(|h| h.before.start <= end) {
                end = end.max(h.before.end);
                right_removed += h.before.len();
                right_added += h.after.len();
                j += 1;
            } else {
                break;
            }
        }
        let left_start = left_at + start - base_at;
        let right_start = right_at + start - base_at;
        let left_end = left_start + (end - start - left_removed) + left_added;
        let right_end = right_start + (end - start - right_removed) + right_added;
        let left_changed = i > first_left;
        let right_changed = j > first_right;
        let is_conflict = left_changed && right_changed && left[left_start..left_end] != right[right_start..right_end];
        chunks.push(MergeChunk {
            base: start..end,
            left: left_start..left_end,
            right: right_start..right_end,
            left_changed,
            right_changed,
            is_conflict,
            resolved: !is_conflict,
        });
        base_at = end;
        left_at = left_end;
        right_at = right_end;
    }
    chunks
}

fn resolve_words(base: &str, left: &str, right: &str) -> Option<String> {
    let base = merge_words(base);
    let left = merge_words(left);
    let right = merge_words(right);
    if base.len() + left.len() + right.len() > 20_000 {
        return None;
    }
    let mut result = String::new();
    let mut at = 0;
    for chunk in merge_chunks(&base, &left, &right) {
        if chunk.is_conflict {
            return None;
        }
        result.extend(base[at..chunk.base.start].iter().copied());
        result.extend(if chunk.left_changed { &left[chunk.left] } else { &right[chunk.right] }.iter().copied());
        at = chunk.base.end;
    }
    result.extend(base[at..].iter().copied());
    Some(result)
}

fn merge_words(text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let kind = |c: char| {
            if c.is_alphanumeric() || c == '_' {
                0
            } else if c == ' ' || c == '\t' {
                1
            } else {
                2
            }
        };
        let mut end = start + c.len_utf8();
        if kind(c) != 2 {
            while let Some(&(at, next)) = chars.peek() {
                if kind(next) != kind(c) {
                    break;
                }
                end = at + next.len_utf8();
                chars.next();
            }
        }
        words.push(&text[start..end]);
    }
    words
}
