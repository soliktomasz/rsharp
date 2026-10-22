use crate::{Edit, EditPlan, Selection};
use ropey::Rope;

/// How a file's lines end on disk; the document holds `\n` alone and writes them back as they were.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineEnding {
    Lf,
    CrLf,
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
    /// Edits that take the text back to before the step, against the text after it.
    inverse: Vec<Edit>,
    before: Vec<Selection>,
    after: Vec<Selection>,
}

pub struct Document {
    text: Rope,
    selections: Vec<Selection>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    version: u64,
    saved_version: u64,
    pub line_ending: LineEnding,
}

impl Document {
    pub fn new(text: &str) -> Self {
        let line_ending = if text.contains("\r\n") { LineEnding::CrLf } else { LineEnding::Lf };
        Self {
            text: Rope::from_str(&text.replace("\r\n", "\n")),
            selections: vec![Selection::default()],
            undo: Vec::new(),
            redo: Vec::new(),
            version: 0,
            saved_version: 0,
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

    pub fn is_modified(&self) -> bool {
        self.version != self.saved_version
    }

    /// The text as the file should hold it, its line endings put back.
    pub fn contents(&self) -> String {
        let text = self.text.to_string();
        match self.line_ending {
            LineEnding::Lf => text,
            LineEnding::CrLf => text.replace('\n', "\r\n"),
        }
    }

    pub fn mark_saved(&mut self) {
        self.saved_version = self.version;
    }

    /// Applies a command's plan as one undo step and returns its changes in the order they were made.
    pub fn apply(&mut self, plan: EditPlan) -> Vec<Change> {
        if plan.edits.is_empty() {
            self.set_selections(plan.selections);
            return Vec::new();
        }
        let before = std::mem::take(&mut self.selections);
        let (changes, inverse) = self.replace(plan.edits);
        // shortcut: each command is its own undo step; Rider merges a run of typing into one.
        self.undo.push(Step { inverse, before, after: plan.selections.clone() });
        self.redo.clear();
        self.set_selections(plan.selections);
        changes
    }

    pub fn undo(&mut self) -> Option<Vec<Change>> {
        let step = self.undo.pop()?;
        let (changes, inverse) = self.replace(step.inverse);
        self.set_selections(step.before.clone());
        self.redo.push(Step { inverse, before: step.before, after: step.after });
        Some(changes)
    }

    pub fn redo(&mut self) -> Option<Vec<Change>> {
        let step = self.redo.pop()?;
        let (changes, inverse) = self.replace(step.inverse);
        self.set_selections(step.after.clone());
        self.undo.push(Step { inverse, before: step.before, after: step.after });
        Some(changes)
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
        let line = self.text.byte_to_line(offset);
        let line_start = self.text.line_to_byte(line);
        let utf16_column = self.text.char_to_utf16_cu(self.text.byte_to_char(offset))
            - self.text.char_to_utf16_cu(self.text.byte_to_char(line_start));
        Position { offset, line, column: offset - line_start, utf16_column }
    }

    /// The offset of a line and UTF-16 column, as the language server names a place.
    pub fn offset_of_utf16(&self, line: usize, utf16_column: usize) -> usize {
        let line = line.min(self.text.len_lines().saturating_sub(1));
        let line_start_char = self.text.line_to_char(line);
        let line_end_char = line_start_char + crate::line_len_chars(&self.text, line);
        let target =
            (self.text.char_to_utf16_cu(line_start_char) + utf16_column).min(self.text.char_to_utf16_cu(line_end_char));
        let char = self.text.utf16_cu_to_char(target);
        self.text.char_to_byte(char)
    }
}
