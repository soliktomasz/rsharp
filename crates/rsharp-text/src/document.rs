use crate::{Edit, EditKind, EditPlan, Selection};
use ropey::Rope;
use std::time::{Duration, Instant};

/// How long a pause still lets typing join the step before it, as Passenger's.
const MERGE_WINDOW: Duration = Duration::from_secs(2);

/// How a file's lines end on disk; the document holds `\n` alone and writes them back as they were.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineEnding {
    Lf,
    CrLf,
    /// Classic Mac OS's lone `\r`.
    Cr,
}

impl LineEnding {
    /// Rider's Line Separators menu, in its order.
    pub const ALL: [LineEnding; 3] = [LineEnding::CrLf, LineEnding::Lf, LineEnding::Cr];

    /// The status bar's name for it.
    pub fn name(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
            LineEnding::Cr => "CR",
        }
    }

    /// Rider's menu item for it.
    pub fn title(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF - Unix and macOS (\\n)",
            LineEnding::CrLf => "CRLF - Windows (\\r\\n)",
            LineEnding::Cr => "CR - Classic Mac OS (\\r)",
        }
    }

    pub fn written(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::Cr => "\r",
        }
    }
}

/// A place in the text as each consumer counts it: tree-sitter by bytes in the line, the language server by
/// UTF-16 code units.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Position {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
    pub utf16_column: usize,
}

/// One edit as it was applied, in order, with the places before it (`start`, `old_end`) and after (`new_end`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    pub start: Position,
    pub old_end: Position,
    pub new_end: Position,
    pub text: String,
}

struct Step {
    /// Batches of edits that take the text back to before the step, applied in order, the first against the text
    /// after it: a merged run of typing is one batch per key.
    inverse: Vec<Vec<Edit>>,
    before: Vec<Selection>,
    after: Vec<Selection>,
    kind: EditKind,
    /// The last edit's text, and when it was made, for whether the next one joins it.
    last_text: String,
    at: Instant,
}

/// A file's text held as a buffer holds it: its own line separator made `\n`, the others kept as they are. A text
/// already separated by `\n` — nearly every file — is given back as it came, without a copy, so a large revision is
/// read for the gutter or a comparison without being copied twice.
// shortcut: a file with mixed breaks is held by its first kind found; Rider makes them one on save.
pub fn hold_newlines(text: String) -> String {
    if text.contains("\r\n") {
        text.replace("\r\n", "\n")
    } else if text.contains('\r') {
        text.replace('\r', "\n")
    } else {
        text
    }
}

pub struct Document {
    text: Rope,
    selections: Vec<Selection>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    version: u64,
    saved_version: u64,
    saved_line_ending: LineEnding,
    /// Set by undo and redo, after which nothing joins the step before.
    merge_broken: bool,
    pub line_ending: LineEnding,
}

impl Document {
    pub fn new(text: &str) -> Self {
        let line_ending = if text.contains("\r\n") {
            LineEnding::CrLf
        } else if text.contains('\r') {
            LineEnding::Cr
        } else {
            LineEnding::Lf
        };
        Self {
            text: Rope::from_str(&hold_newlines(text.to_string())),
            selections: vec![Selection::default()],
            undo: Vec::new(),
            redo: Vec::new(),
            version: 0,
            saved_version: 0,
            saved_line_ending: line_ending,
            merge_broken: true,
            line_ending,
        }
    }

    pub fn text(&self) -> &Rope {
        &self.text
    }

    /// Bumped by every change, undo and redo included.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn selections(&self) -> &[Selection] {
        &self.selections
    }

    pub fn set_selections(&mut self, selections: Vec<Selection>) {
        let len = self.text.len_bytes();
        self.selections =
            selections.into_iter().map(|s| Selection { anchor: s.anchor.min(len), head: s.head.min(len) }).collect();
    }

    /// Whether the file on disk differs: its text changed, or how its lines end.
    pub fn is_modified(&self) -> bool {
        self.version != self.saved_version || self.line_ending != self.saved_line_ending
    }

    /// The text as the file should hold it, its line endings put back.
    pub fn contents(&self) -> String {
        let text = self.text.to_string();
        match self.line_ending {
            LineEnding::Lf => text,
            LineEnding::CrLf => text.replace('\n', "\r\n"),
            LineEnding::Cr => text.replace('\n', "\r"),
        }
    }

    pub fn mark_saved(&mut self) {
        self.saved_version = self.version;
        self.saved_line_ending = self.line_ending;
    }

    /// Applies a command's plan as one undo step, or as part of the step before when it goes on with the same run
    /// of typing or deleting, and returns its changes in the order they were made.
    pub fn apply(&mut self, plan: EditPlan) -> Vec<Change> {
        if plan.edits.is_empty() {
            self.set_selections(plan.selections);
            return Vec::new();
        }
        let before = std::mem::take(&mut self.selections);
        let now = Instant::now();
        let joins = self.joins(&plan, &before, now);
        let last_text = plan.edits[0].text.clone();
        let (changes, inverse) = self.replace(plan.edits);
        match self.undo.last_mut() {
            Some(step) if joins => {
                step.inverse.insert(0, inverse);
                step.after = plan.selections.clone();
                step.last_text = last_text;
                step.at = now;
            }
            _ => self.undo.push(Step {
                inverse: vec![inverse],
                before,
                after: plan.selections.clone(),
                kind: plan.kind,
                last_text,
                at: now,
            }),
        }
        self.redo.clear();
        self.merge_broken = false;
        self.set_selections(plan.selections);
        changes
    }

    /// Whether a plan goes on with the last step: the same kind of edit, from where its carets were left, soon after,
    /// and typing a word at a time (a space after other text, or a line break, starts a step of its own).
    fn joins(&self, plan: &EditPlan, before: &[Selection], now: Instant) -> bool {
        let Some(last) = self.undo.last() else { return false };
        if self.merge_broken || plan.kind == EditKind::Other || last.kind != plan.kind || last.after != before {
            return false;
        }
        if now.duration_since(last.at) > MERGE_WINDOW {
            return false;
        }
        if plan.kind == EditKind::Typing {
            let typed = &plan.edits[0].text;
            let starts_gap = typed.starts_with(char::is_whitespace);
            let ended_gap = last.last_text.ends_with(char::is_whitespace);
            if (starts_gap && !ended_gap) || typed.contains('\n') {
                return false;
            }
        }
        true
    }

    pub fn undo(&mut self) -> Option<Vec<Change>> {
        let step = self.undo.pop()?;
        let (changes, inverse) = self.replay(step.inverse);
        self.set_selections(step.before.clone());
        self.redo.push(Step { inverse, ..step });
        self.merge_broken = true;
        Some(changes)
    }

    pub fn redo(&mut self) -> Option<Vec<Change>> {
        let step = self.redo.pop()?;
        let (changes, inverse) = self.replay(step.inverse);
        self.set_selections(step.after.clone());
        self.undo.push(Step { inverse, ..step });
        self.merge_broken = true;
        Some(changes)
    }

    /// Applies a step's batches in order, and returns their changes and the batches that take them back.
    fn replay(&mut self, batches: Vec<Vec<Edit>>) -> (Vec<Change>, Vec<Vec<Edit>>) {
        let mut changes = Vec::new();
        let mut inverse = Vec::with_capacity(batches.len());
        for batch in batches {
            let (applied, back) = self.replace(batch);
            changes.extend(applied);
            inverse.push(back);
        }
        inverse.reverse();
        (changes, inverse)
    }

    /// Applies sorted, disjoint edits from the last, so each one's offsets still hold as it is made, and returns
    /// them as changes and as the edits that undo them (against the text after).
    fn replace(&mut self, edits: Vec<Edit>) -> (Vec<Change>, Vec<Edit>) {
        let mut changes = Vec::with_capacity(edits.len());
        let mut removed = Vec::with_capacity(edits.len());
        for edit in edits.iter().rev() {
            let start = self.position(edit.range.start);
            let old_end = self.position(edit.range.end);
            let start_char = self.text.byte_to_char(edit.range.start);
            let end_char = self.text.byte_to_char(edit.range.end);
            removed.push(self.text.slice(start_char..end_char).to_string());
            self.text.remove(start_char..end_char);
            self.text.insert(start_char, &edit.text);
            let new_end = self.position(edit.range.start + edit.text.len());
            changes.push(Change { start, old_end, new_end, text: edit.text.clone() });
        }
        removed.reverse();
        let mut shift: isize = 0;
        let inverse = edits
            .iter()
            .zip(removed)
            .map(|(edit, old)| {
                let start = (edit.range.start as isize + shift) as usize;
                shift += edit.text.len() as isize - edit.range.len() as isize;
                Edit { range: start..start + edit.text.len(), text: old }
            })
            .collect();
        self.version += 1;
        (changes, inverse)
    }

    pub fn position(&self, offset: usize) -> Position {
        position(&self.text, offset)
    }

    /// The offset of a line and UTF-16 column, as the language server names a place.
    pub fn offset_of_utf16(&self, line: usize, utf16_column: usize) -> usize {
        offset_of_utf16(&self.text, line, utf16_column)
    }
}

/// The offset of a line and UTF-16 column, kept on the line; a line past the end is the last.
pub fn offset_of_utf16(text: &Rope, line: usize, utf16_column: usize) -> usize {
    let line = line.min(text.len_lines().saturating_sub(1));
    let line_start_char = text.line_to_char(line);
    let line_end_char = line_start_char + crate::line_len_chars(text, line);
    let target = (text.char_to_utf16_cu(line_start_char) + utf16_column).min(text.char_to_utf16_cu(line_end_char));
    text.char_to_byte(text.utf16_cu_to_char(target))
}

/// A place in `text`, as each consumer counts it.
pub fn position(text: &Rope, offset: usize) -> Position {
    let line = text.byte_to_line(offset);
    let line_start = text.line_to_byte(line);
    let utf16_column =
        text.char_to_utf16_cu(text.byte_to_char(offset)) - text.char_to_utf16_cu(text.byte_to_char(line_start));
    Position { offset, line, column: offset - line_start, utf16_column }
}
