//! No UI. The document (Ropey's rope, whose clone is a cheap snapshot another thread may read), its undo, the
//! carets, and every editing command as a pure function to an [`EditPlan`]. Offsets are UTF-8 bytes throughout.

mod carets;
mod commands;
mod diff;
mod document;
mod movement;
mod rows;
mod search;

pub use carets::*;
pub use commands::*;
pub use diff::*;
pub use document::*;
pub use movement::*;
pub use ropey::Rope;
pub use rows::*;
pub use search::*;

use std::ops::Range;

/// A caret, with the selection from `anchor` to `head` (the end the caret is at).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(offset: usize) -> Self {
        Self { anchor: offset, head: offset }
    }

    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn range(&self) -> Range<usize> {
        self.start()..self.end()
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// Puts `primary` back at the end of carets put in the text's order, where the editor keeps the primary.
fn primary_last(carets: &mut Vec<Selection>, primary: Option<Selection>) {
    if let Some(at) = primary.and_then(|p| carets.iter().position(|c| *c == p)) {
        let primary = carets.remove(at);
        carets.push(primary);
    }
}

/// One replacement, against the text before the command.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
}

/// What an edit was, so a run of one kind undoes as one step.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum EditKind {
    #[default]
    Other,
    Typing,
    DeleteBackward,
    DeleteForward,
}

/// What a command does: its edits, sorted and disjoint, against the text before it, and the carets after it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct EditPlan {
    pub edits: Vec<Edit>,
    pub selections: Vec<Selection>,
    pub kind: EditKind,
}
