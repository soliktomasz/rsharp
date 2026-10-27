//! Every editing command, as a pure function of the text and carets to an [`EditPlan`].

use crate::{Edit, EditKind, EditPlan, Rope, Selection, line_indent, next_char, previous_char};
use std::ops::Range;

/// How a file indents: a tab, or so many spaces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Indent {
    pub tabs: bool,
    pub size: usize,
}

impl Default for Indent {
    /// C#'s, as Rider's defaults have it.
    fn default() -> Self {
        Self { tabs: false, size: 4 }
    }
}

impl Indent {
    /// One level of it.
    pub fn unit(&self) -> String {
        if self.tabs { "\t".into() } else { " ".repeat(self.size) }
    }

    /// The status bar's words for it: "Tab", "4 spaces".
    pub fn name(&self) -> String {
        match (self.tabs, self.size) {
            (true, _) => "Tab".into(),
            (false, 1) => "1 space".into(),
            (false, n) => format!("{n} spaces"),
        }
    }
}

/// How a file's own lines indent, as Rider's Detect and use existing file indents reads them: tabs where more lines
/// start with one than with spaces, else the step most often seen between a line and a deeper one after it. None for
/// a file with no indented line.
pub fn detect_indent(text: &Rope) -> Option<Indent> {
    let (mut tabs, mut spaces) = (0, 0);
    let mut steps = [0usize; 9];
    let mut previous = 0;
    // A file's first thousand lines say as much as the rest.
    for line in text.lines().take(1000) {
        let leading: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        if line.chars().all(char::is_whitespace) {
            continue;
        }
        if leading.starts_with('\t') {
            tabs += 1;
        } else if !leading.is_empty() {
            spaces += 1;
            let width = leading.len();
            if width > previous && width - previous < steps.len() {
                steps[width - previous] += 1;
            }
        }
        previous = if leading.starts_with('\t') { previous } else { leading.len() };
    }
    if tabs + spaces == 0 {
        return None;
    }
    if tabs > spaces {
        return Some(Indent { tabs: true, size: 4 });
    }
    let (size, seen) =
        steps.iter().enumerate().skip(1).max_by_key(|(size, seen)| (**seen, std::cmp::Reverse(*size)))?;
    (*seen > 0).then_some(Indent { tabs: false, size })
}

/// Replaces each selection with `text` (typing, paste).
pub fn insert(_text: &Rope, selections: &[Selection], typed: &str) -> EditPlan {
    plan(selections, |s| Some(Edit { range: s.range(), text: typed.to_string() }))
}

/// Backspace: the selection, else the character before the caret.
pub fn backspace(text: &Rope, selections: &[Selection]) -> EditPlan {
    let plan = plan(selections, |s| {
        if !s.is_empty() {
            return Some(Edit { range: s.range(), text: String::new() });
        }
        (s.head > 0).then(|| Edit { range: previous_char(text, s.head)..s.head, text: String::new() })
    });
    EditPlan { kind: EditKind::DeleteBackward, ..plan }
}

/// A quote typed in markup, as Passenger types one: in front of the same quote it steps over it; where no word and no
/// quote touches the caret it comes with its closing quote, the caret between them; anything else goes in as typed.
pub fn type_quote(text: &Rope, selections: &[Selection], quote: char) -> EditPlan {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut plan = plan_back(selections, |s| {
        let before = (s.start() > 0).then(|| text.byte_slice(previous_char(text, s.start())..s.start()).chars().next());
        let after = (s.end() < text.len_bytes()).then(|| text.byte_slice(s.end()..).chars().next());
        let (before, after) = (before.flatten(), after.flatten());
        let edit = |range: Range<usize>, typed: String| Edit { range, text: typed };
        Some(if s.is_empty() && after == Some(quote) {
            (edit(s.head..s.head + quote.len_utf8(), quote.to_string()), 0)
        } else if s.is_empty() && !word(before) && !word(after) && before != Some(quote) {
            (edit(s.range(), format!("{quote}{quote}")), quote.len_utf8())
        } else {
            (edit(s.range(), quote.to_string()), 0)
        })
    });
    // Keep the shifted carets, but stepping over an existing quote must not change the text or undo history.
    plan.edits.retain(|edit| text.byte_slice(edit.range.clone()) != edit.text.as_str());
    EditPlan { kind: EditKind::Typing, ..plan }
}

/// Backspace in markup: between a pair of quotes, both go, as the pair came in together.
pub fn backspace_quotes(text: &Rope, selections: &[Selection]) -> EditPlan {
    let paired = |s: &Selection| {
        s.is_empty()
            && s.head > 0
            && s.head < text.len_bytes()
            && matches!(text.byte(s.head - 1), b'"' | b'\'')
            && text.byte(s.head - 1) == text.byte(s.head)
    };
    if !selections.iter().any(paired) {
        return backspace(text, selections);
    }
    let plan = plan(selections, |s| {
        if paired(s) {
            return Some(Edit { range: s.head - 1..s.head + 1, text: String::new() });
        }
        if !s.is_empty() {
            return Some(Edit { range: s.range(), text: String::new() });
        }
        (s.head > 0).then(|| Edit { range: previous_char(text, s.head)..s.head, text: String::new() })
    });
    EditPlan { kind: EditKind::DeleteBackward, ..plan }
}

/// Delete: the selection, else the character after the caret.
pub fn delete(text: &Rope, selections: &[Selection]) -> EditPlan {
    let plan = plan(selections, |s| {
        if !s.is_empty() {
            return Some(Edit { range: s.range(), text: String::new() });
        }
        (s.head < text.len_bytes()).then(|| Edit { range: s.head..next_char(text, s.head), text: String::new() })
    });
    EditPlan { kind: EditKind::DeleteForward, ..plan }
}

/// Paste of as many lines as there are carets: one line to each caret, in the order they stand in the text.
pub fn insert_each(selections: &[Selection], pieces: &[&str]) -> EditPlan {
    // The plan takes the carets in the text's order, so the lines are dealt out in it.
    let mut pieces = pieces.iter();
    plan(selections, |s| Some(Edit { range: s.range(), text: pieces.next().map_or("", |p| p).to_string() }))
}

/// Enter: a new line at the line's indent, one deeper after an opening brace.
pub fn newline(text: &Rope, selections: &[Selection], unit: &Indent) -> EditPlan {
    plan(selections, |s| {
        let line = text.byte_to_line(s.start());
        let mut indent = line_indent(text, line);
        let line_start = text.line_to_byte(line);
        let before = text.byte_slice(line_start..s.start()).to_string();
        if before.trim_end().ends_with('{') {
            indent.push_str(&unit.unit());
        }
        Some(Edit { range: s.range(), text: format!("\n{indent}") })
    })
}

/// Tab: a tab, or an indent's worth of spaces to the next tab stop.
pub fn tab(text: &Rope, selections: &[Selection], unit: &Indent) -> EditPlan {
    plan(selections, |s| {
        if unit.tabs {
            return Some(Edit { range: s.range(), text: "\t".into() });
        }
        let column = crate::column_of(text, s.start());
        let spaces = unit.size.max(1) - column % unit.size.max(1);
        Some(Edit { range: s.range(), text: " ".repeat(spaces) })
    })
}

/// The plan of one edit (or none) per caret, the carets after each edit's text. Carets whose edits would touch
/// are kept apart by the order of their starts.
fn plan(selections: &[Selection], mut edit: impl FnMut(&Selection) -> Option<Edit>) -> EditPlan {
    plan_back(selections, |s| edit(s).map(|e| (e, 0)))
}

/// [`plan`], each edit's caret that many bytes back from the end of its text.
fn plan_back(selections: &[Selection], mut edit: impl FnMut(&Selection) -> Option<(Edit, usize)>) -> EditPlan {
    let mut ordered: Vec<&Selection> = selections.iter().collect();
    ordered.sort_by_key(|s| s.start());
    let primary = selections.last();
    let mut primary_caret = None;
    let mut edits: Vec<Edit> = Vec::new();
    let mut carets = Vec::with_capacity(ordered.len());
    let mut shift: isize = 0;
    for selection in ordered {
        match edit(selection) {
            Some((e, back)) if edits.last().is_none_or(|last| last.range.end <= e.range.start) => {
                let caret = (e.range.start as isize + shift) as usize + e.text.len() - back;
                shift += e.text.len() as isize - e.range.len() as isize;
                carets.push(Selection::caret(caret));
                edits.push(e);
            }
            _ => carets.push(Selection::caret((selection.head as isize + shift) as usize)),
        }
        if primary.is_some_and(|p| std::ptr::eq(p, selection)) {
            primary_caret = carets.last().copied();
        }
    }
    carets.dedup();
    crate::primary_last(&mut carets, primary_caret);
    EditPlan { edits, selections: carets, kind: EditKind::Other }
}
