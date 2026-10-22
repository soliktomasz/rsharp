//! Every editing command, as a pure function of the text and carets to an [`EditPlan`].

use crate::{Edit, EditPlan, Rope, Selection, line_indent, next_char, previous_char};

const INDENT: &str = "    ";

/// Replaces each selection with `text` (typing, paste).
pub fn insert(_text: &Rope, selections: &[Selection], typed: &str) -> EditPlan {
    plan(selections, |s| Some(Edit { range: s.range(), text: typed.to_string() }))
}

/// Backspace: the selection, else the character before the caret.
pub fn backspace(text: &Rope, selections: &[Selection]) -> EditPlan {
    plan(selections, |s| {
        if !s.is_empty() {
            return Some(Edit { range: s.range(), text: String::new() });
        }
        (s.head > 0).then(|| Edit { range: previous_char(text, s.head)..s.head, text: String::new() })
    })
}

/// Delete: the selection, else the character after the caret.
pub fn delete(text: &Rope, selections: &[Selection]) -> EditPlan {
    plan(selections, |s| {
        if !s.is_empty() {
            return Some(Edit { range: s.range(), text: String::new() });
        }
        (s.head < text.len_bytes()).then(|| Edit { range: s.head..next_char(text, s.head), text: String::new() })
    })
}

/// Enter: a new line at the line's indent, one deeper after an opening brace.
pub fn newline(text: &Rope, selections: &[Selection]) -> EditPlan {
    plan(selections, |s| {
        let line = text.byte_to_line(s.start());
        let mut indent = line_indent(text, line);
        let line_start = text.line_to_byte(line);
        let before = text.byte_slice(line_start..s.start()).to_string();
        if before.trim_end().ends_with('{') {
            indent.push_str(INDENT);
        }
        Some(Edit { range: s.range(), text: format!("\n{indent}") })
    })
}

/// Tab: an indent's worth of spaces to the next tab stop.
pub fn tab(text: &Rope, selections: &[Selection]) -> EditPlan {
    plan(selections, |s| {
        let column = crate::column_of(text, s.start());
        let spaces = INDENT.len() - column % INDENT.len();
        Some(Edit { range: s.range(), text: " ".repeat(spaces) })
    })
}

/// The plan of one edit (or none) per caret, the carets after each edit's text. Carets whose edits would touch
/// are kept apart by the order of their starts.
fn plan(selections: &[Selection], mut edit: impl FnMut(&Selection) -> Option<Edit>) -> EditPlan {
    let mut ordered: Vec<&Selection> = selections.iter().collect();
    ordered.sort_by_key(|s| s.start());
    let mut edits: Vec<Edit> = Vec::new();
    let mut carets = Vec::with_capacity(ordered.len());
    let mut shift: isize = 0;
    for selection in ordered {
        match edit(selection) {
            Some(e) if edits.last().is_none_or(|last| last.range.end <= e.range.start) => {
                let caret = (e.range.start as isize + shift) as usize + e.text.len();
                shift += e.text.len() as isize - e.range.len() as isize;
                carets.push(Selection::caret(caret));
                edits.push(e);
            }
            _ => carets.push(Selection::caret((selection.head as isize + shift) as usize)),
        }
    }
    carets.dedup();
    EditPlan { edits, selections: carets }
}
