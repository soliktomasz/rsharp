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

/// The brackets the editor pairs, each opener with its closer, as Passenger's editor pairs them.
const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

/// A character typed with the editor's help, as Passenger types one: a bracket or quote typed over a selection wraps
/// it, still selected; a closer or quote typed in front of the same one steps over it; an opener before a space, a
/// closer, `;` or `,` comes with its closer, and a quote no word or quote touches with its own, the caret between them.
/// Anything else goes in as typed.
pub fn type_char(text: &Rope, selections: &[Selection], c: char) -> EditPlan {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let closer = PAIRS.iter().find(|(open, _)| *open == c).map(|(_, close)| *close);
    let quote = matches!(c, '"' | '\'');
    let closes = PAIRS.iter().any(|(_, close)| *close == c);
    let mut plan = plan_back(selections, |s| {
        let before = (s.start() > 0).then(|| text.byte_slice(previous_char(text, s.start())..s.start()).chars().next());
        let after = (s.end() < text.len_bytes()).then(|| text.byte_slice(s.end()..).chars().next());
        let (before, after) = (before.flatten(), after.flatten());
        let typed = |range: Range<usize>, typed: String| Edit { range, text: typed };
        Some(match closer.or(quote.then_some(c)) {
            Some(close) if !s.is_empty() => {
                let inner = text.byte_slice(s.range()).to_string();
                let length = inner.len();
                (typed(s.range(), format!("{c}{inner}{close}")), close.len_utf8(), length)
            }
            _ if s.is_empty() && (closes || quote) && after == Some(c) => {
                (typed(s.head..s.head + c.len_utf8(), c.to_string()), 0, 0)
            }
            Some(close)
                if closer.is_some()
                    && after.is_none_or(|a| {
                        a.is_whitespace() || PAIRS.iter().any(|(_, p)| *p == a) || a == ';' || a == ','
                    }) =>
            {
                (typed(s.range(), format!("{c}{close}")), close.len_utf8(), 0)
            }
            _ if quote && !word(before) && !word(after) && before != Some(c) => {
                (typed(s.range(), format!("{c}{c}")), c.len_utf8(), 0)
            }
            _ => (typed(s.range(), c.to_string()), 0, 0),
        })
    });
    // Stepping over a closer leaves the text, and so the undo history and the saved state, as they were.
    plan.edits.retain(|edit| text.byte_slice(edit.range.clone()) != edit.text.as_str());
    EditPlan { kind: EditKind::Typing, ..plan }
}

/// Backspace with the editor's help: between an opener and its closer, or a pair of quotes, both go, as they came in
/// together.
pub fn backspace_pairs(text: &Rope, selections: &[Selection]) -> EditPlan {
    let paired = |s: &Selection| {
        s.is_empty() && s.head > 0 && s.head < text.len_bytes() && {
            let (before, after) = (text.byte(s.head - 1) as char, text.byte(s.head) as char);
            PAIRS.contains(&(before, after)) || (matches!(before, '"' | '\'') && before == after)
        }
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
    plan_back(selections, |s| {
        let line = text.byte_to_line(s.start());
        let indent = line_indent(text, line);
        let line_start = text.line_to_byte(line);
        let before = text.byte_slice(line_start..s.start()).to_string();
        let opener = before.trim_end().chars().last();
        let deeper = if opener == Some('{') { format!("{indent}{}", unit.unit()) } else { indent.clone() };
        // Between an opener and its closer, as a typed opener leaves them, the closer goes to a line of its own.
        let line_end = text.line_to_byte(line + 1).min(text.len_bytes());
        let rest = text.byte_slice(s.end()..line_end).to_string();
        let next = rest.trim_start_matches([' ', '\t']).chars().next();
        if let Some(pair) = PAIRS.iter().find(|(open, close)| opener == Some(*open) && next == Some(*close)) {
            let deeper = if pair.0 == '{' { deeper } else { format!("{indent}{}", unit.unit()) };
            let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
            let closing = format!("\n{indent}");
            return Some((
                Edit { range: s.start()..s.end() + spaces, text: format!("\n{deeper}{closing}") },
                closing.len(),
                0,
            ));
        }
        Some((Edit { range: s.range(), text: format!("\n{deeper}") }, 0, 0))
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
    plan_back(selections, |s| edit(s).map(|e| (e, 0, 0)))
}

/// [`plan`], each edit's caret that many bytes back from the end of its text, selecting so many bytes before it.
fn plan_back(selections: &[Selection], mut edit: impl FnMut(&Selection) -> Option<(Edit, usize, usize)>) -> EditPlan {
    let mut ordered: Vec<&Selection> = selections.iter().collect();
    ordered.sort_by_key(|s| s.start());
    let primary = selections.last();
    let mut primary_caret = None;
    let mut edits: Vec<Edit> = Vec::new();
    let mut carets = Vec::with_capacity(ordered.len());
    let mut shift: isize = 0;
    for selection in ordered {
        match edit(selection) {
            Some((e, back, selected)) if edits.last().is_none_or(|last| last.range.end <= e.range.start) => {
                let caret = (e.range.start as isize + shift) as usize + e.text.len() - back;
                shift += e.text.len() as isize - e.range.len() as isize;
                carets.push(Selection { anchor: caret - selected, head: caret });
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
