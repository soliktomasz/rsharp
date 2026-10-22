//! Where each caret move goes, as offsets in the text.

use crate::Rope;

pub fn previous_char(text: &Rope, offset: usize) -> usize {
    if offset == 0 { 0 } else { text.char_to_byte(text.byte_to_char(offset) - 1) }
}

pub fn next_char(text: &Rope, offset: usize) -> usize {
    let char = text.byte_to_char(offset);
    if char >= text.len_chars() { offset } else { text.char_to_byte(char + 1) }
}

/// The line's length in characters, its line break left out.
pub fn line_len_chars(text: &Rope, line: usize) -> usize {
    let slice = text.line(line);
    let len = slice.len_chars();
    if len > 0 && slice.char(len - 1) == '\n' { len - 1 } else { len }
}

/// The end of a line's text, before its break.
pub fn line_end(text: &Rope, line: usize) -> usize {
    text.char_to_byte(text.line_to_char(line) + line_len_chars(text, line))
}

/// A caret's column in characters.
pub fn column_of(text: &Rope, offset: usize) -> usize {
    let char = text.byte_to_char(offset);
    char - text.line_to_char(text.char_to_line(char))
}

/// The offset at a line and character column, kept inside the line.
pub fn offset_at(text: &Rope, line: usize, column: usize) -> usize {
    let line = line.min(text.len_lines() - 1);
    text.char_to_byte(text.line_to_char(line) + column.min(line_len_chars(text, line)))
}

/// The spaces and tabs a line starts with.
pub fn line_indent(text: &Rope, line: usize) -> String {
    text.line(line).chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// Home, as Rider's: to the line's first non-blank character, or to its start from there.
pub fn smart_home(text: &Rope, offset: usize) -> usize {
    let line = text.byte_to_line(offset);
    let start = text.line_to_byte(line);
    let first = start + line_indent(text, line).len();
    if offset == first { start } else { first }
}

pub fn end_of_line(text: &Rope, offset: usize) -> usize {
    line_end(text, text.byte_to_line(offset))
}

/// The caret a line up or down from `offset` at `goal` column, or the text's start or end past its first or last line.
pub fn vertical(text: &Rope, offset: usize, goal: usize, lines: isize) -> usize {
    let line = text.byte_to_line(offset) as isize + lines;
    if line < 0 {
        0
    } else if line as usize >= text.len_lines() {
        text.len_bytes()
    } else {
        offset_at(text, line as usize, goal)
    }
}

/// Ctrl/Alt+Left: to the start of the word before the caret, across what is between.
pub fn word_left(text: &Rope, offset: usize) -> usize {
    let mut char = text.byte_to_char(offset);
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            2
        } else if c.is_whitespace() {
            0
        } else {
            1
        }
    };
    while char > 0 && class(text.char(char - 1)) == 0 {
        char -= 1;
    }
    if char > 0 {
        let kind = class(text.char(char - 1));
        while char > 0 && class(text.char(char - 1)) == kind {
            char -= 1;
        }
    }
    text.char_to_byte(char)
}

/// Ctrl/Alt+Right: to the end of the word after the caret, across what is between.
pub fn word_right(text: &Rope, offset: usize) -> usize {
    let mut char = text.byte_to_char(offset);
    let len = text.len_chars();
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            2
        } else if c.is_whitespace() {
            0
        } else {
            1
        }
    };
    while char < len && class(text.char(char)) == 0 {
        char += 1;
    }
    if char < len {
        let kind = class(text.char(char));
        while char < len && class(text.char(char)) == kind {
            char += 1;
        }
    }
    text.char_to_byte(char)
}
