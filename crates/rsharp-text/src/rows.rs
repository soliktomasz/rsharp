//! Lines as the editor shows them: folded lines left out, and with soft wrap a long line going on in rows below.
//! While nothing is folded or wrapped every line is a row and nothing is built, so a key costs what it did.

use crate::{Change, Rope, line_end, line_indent, line_len_chars};
use std::ops::Range;

/// One row on screen: the bytes of its line it shows, its break left out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Row {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    /// Whether it starts its line, so it carries the line's number.
    pub first: bool,
    /// Whether it ends its line, so a selection past its end shows the line break and a fold's placeholder follows.
    pub last: bool,
    /// How many columns in it is drawn: a wrapped line's later rows start at the line's own indent, as Rider's.
    pub indent: usize,
}

#[derive(PartialEq, Debug)]
pub enum Rows {
    /// Every line one row.
    Lines,
    Built(Vec<Row>),
}

impl Rows {
    /// The rows of `text` with `hidden` (sorted, disjoint line ranges) left out and, given `wrap`, lines longer than
    /// that many characters broken after a space where there is one, else at the edge.
    // shortcut: wide characters count as one column; measure them once a person wraps CJK text.
    pub fn build(text: &Rope, hidden: &[Range<usize>], wrap: Option<usize>) -> Self {
        if hidden.is_empty() && wrap.is_none() {
            return Rows::Lines;
        }
        let mut rows = Vec::with_capacity(text.len_lines());
        let mut hidden = hidden.iter().peekable();
        let mut line = 0;
        while line < text.len_lines() {
            if let Some(h) = hidden.peek()
                && h.contains(&line)
            {
                line = h.end;
                hidden.next();
                continue;
            }
            push_line(text, line, wrap, &mut rows);
            line += 1;
        }
        Rows::Built(rows)
    }

    /// After one change to the text the rows were made for, the rows of the lines it touched made again and those
    /// after it moved along, so a key costs the lines it changed. False where the rows must be made whole: a folded
    /// region touches the lines that changed.
    pub fn apply_change(&mut self, text: &Rope, change: &Change, hidden: &[Range<usize>], wrap: Option<usize>) -> bool {
        let Rows::Built(rows) = self else { return true };
        let (first, old_last, new_last) = (change.start.line, change.old_end.line, change.new_end.line);
        if hidden.iter().any(|h| h.start <= new_last + 1 && h.end >= first) {
            return false;
        }
        let from = rows.partition_point(|r| r.line < first);
        let to = rows.partition_point(|r| r.line <= old_last);
        let mut fresh = Vec::with_capacity(new_last - first + 1);
        for line in first..=new_last {
            push_line(text, line, wrap, &mut fresh);
        }
        let lines = new_last as isize - old_last as isize;
        let bytes = change.new_end.offset as isize - change.old_end.offset as isize;
        for row in &mut rows[to..] {
            row.line = (row.line as isize + lines) as usize;
            row.start = (row.start as isize + bytes) as usize;
            row.end = (row.end as isize + bytes) as usize;
        }
        rows.splice(from..to, fresh);
        true
    }

    pub fn len(&self, text: &Rope) -> usize {
        match self {
            Rows::Lines => text.len_lines(),
            Rows::Built(rows) => rows.len(),
        }
    }

    pub fn is_empty(&self, text: &Rope) -> bool {
        self.len(text) == 0
    }

    /// Row `index`, kept inside the rows there are.
    pub fn row(&self, text: &Rope, index: usize) -> Row {
        match self {
            Rows::Lines => {
                let line = index.min(text.len_lines() - 1);
                Row {
                    line,
                    start: text.line_to_byte(line),
                    end: line_end(text, line),
                    first: true,
                    last: true,
                    indent: 0,
                }
            }
            Rows::Built(rows) => rows[index.min(rows.len() - 1)],
        }
    }

    /// The row an offset shows on: where a line wraps, the row it starts; in folded lines, the fold's first row.
    pub fn row_of(&self, text: &Rope, offset: usize) -> usize {
        match self {
            Rows::Lines => text.byte_to_line(offset),
            Rows::Built(rows) => rows.partition_point(|r| r.start <= offset).saturating_sub(1),
        }
    }
}

/// A line's rows: one, or as many as wrapping it at `wrap` makes.
fn push_line(text: &Rope, line: usize, wrap: Option<usize>, rows: &mut Vec<Row>) {
    let start = text.line_to_byte(line);
    let end = line_end(text, line);
    match wrap.filter(|w| *w > 0 && line_len_chars(text, line) > *w) {
        None => rows.push(Row { line, start, end, first: true, last: true, indent: 0 }),
        Some(width) => wrap_line(text, line, start, end, width, rows),
    }
}

fn wrap_line(text: &Rope, line: usize, start: usize, end: usize, width: usize, rows: &mut Vec<Row>) {
    // A deep indent would leave the later rows no room, so past half the width they start at the edge.
    let indent = line_indent(text, line).chars().count();
    let indent = if indent * 2 < width { indent } else { 0 };
    let mut row_start = start;
    let mut limit = width;
    let mut count = 0;
    let mut after_space = None;
    let mut at = start;
    for c in text.byte_slice(start..end).chars() {
        if count == limit {
            let cut = after_space.filter(|s| *s > row_start).unwrap_or(at);
            let first = row_start == start;
            rows.push(Row {
                line,
                start: row_start,
                end: cut,
                first,
                last: false,
                indent: if first { 0 } else { indent },
            });
            count = text.byte_slice(cut..at).chars().count();
            row_start = cut;
            limit = width - indent;
            after_space = None;
        }
        at += c.len_utf8();
        count += 1;
        if c == ' ' {
            after_space = Some(at);
        }
    }
    let first = row_start == start;
    rows.push(Row { line, start: row_start, end, first, last: true, indent: if first { 0 } else { indent } });
}
