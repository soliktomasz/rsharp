//! Lines as the editor shows them: folded lines left out, and with soft wrap a long line going on in rows below.
//! While nothing is folded or wrapped every line is a row and nothing is built, so a key costs what it did.

use crate::{Rope, line_end, line_indent, line_len_chars};
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

pub enum Rows {
    /// Every line one row.
    Lines,
    Built(Vec<Row>),
}

impl Rows {
    /// The rows of `text` with `hidden` (sorted, disjoint line ranges) left out and, given `wrap`, lines longer than
    /// that many characters broken after a space where there is one, else at the edge.
    // shortcut: every line is wrapped again after each edit while wrap or a fold is on; wrap only the lines an edit
    // touched once a large file with soft wrap on makes typing felt. Wide characters count as one column.
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
            let start = text.line_to_byte(line);
            let end = line_end(text, line);
            match wrap.filter(|w| *w > 0 && line_len_chars(text, line) > *w) {
                None => rows.push(Row { line, start, end, first: true, last: true, indent: 0 }),
                Some(width) => wrap_line(text, line, start, end, width, &mut rows),
            }
            line += 1;
        }
        Rows::Built(rows)
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
