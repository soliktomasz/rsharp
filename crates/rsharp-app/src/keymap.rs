//! The keys, as Rider's: its macOS keymap on macOS and its Visual Studio keymap elsewhere.

use crate::editor::*;
use crate::find_action::{Choose, Dismiss, SelectNext, SelectPrevious};
use crate::window::*;
use gpui::{App, KeyBinding};

gpui::actions!(app, [Quit, OpenSettings]);

pub fn bind(cx: &mut App) {
    let editor = Some("CodeEditor");
    let explorer = Some("Explorer");
    let mac = cfg!(target_os = "macos");
    cx.bind_keys([
        KeyBinding::new("left", Left, editor),
        KeyBinding::new("right", Right, editor),
        KeyBinding::new("up", Up, editor),
        KeyBinding::new("down", Down, editor),
        KeyBinding::new("shift-left", SelectLeft, editor),
        KeyBinding::new("shift-right", SelectRight, editor),
        KeyBinding::new("shift-up", SelectUp, editor),
        KeyBinding::new("shift-down", SelectDown, editor),
        KeyBinding::new("home", LineStart, editor),
        KeyBinding::new("end", LineEnd, editor),
        KeyBinding::new("shift-home", SelectLineStart, editor),
        KeyBinding::new("shift-end", SelectLineEnd, editor),
        KeyBinding::new("pageup", PageUp, editor),
        KeyBinding::new("pagedown", PageDown, editor),
        KeyBinding::new("backspace", Backspace, editor),
        KeyBinding::new("delete", Delete, editor),
        KeyBinding::new("enter", Newline, editor),
        KeyBinding::new("tab", Tab, editor),
        KeyBinding::new("secondary-a", SelectAll, editor),
        KeyBinding::new("secondary-c", Copy, editor),
        KeyBinding::new("secondary-x", Cut, editor),
        KeyBinding::new("secondary-v", Paste, editor),
        KeyBinding::new("secondary-z", Undo, editor),
        KeyBinding::new("escape", Escape, editor),
        // Rider has no key for these on macOS (Option pressed twice, then held); Passenger's are taken on both.
        KeyBinding::new("alt-shift-up", CloneCaretAbove, editor),
        KeyBinding::new("alt-shift-down", CloneCaretBelow, editor),
        KeyBinding::new("alt-shift-g", CaretPerLine, editor),
        KeyBinding::new(if mac { "cmd-shift-8" } else { "alt-shift-insert" }, ToggleColumnMode, editor),
        KeyBinding::new("secondary-s", Save, None),
        KeyBinding::new("secondary-w", CloseTab, None),
        KeyBinding::new("secondary-o", OpenFolder, None),
        KeyBinding::new(if mac { "cmd-1" } else { "alt-1" }, ToggleSolution, None),
        KeyBinding::new(if mac { "cmd-6" } else { "alt-6" }, ToggleProblems, None),
        KeyBinding::new("shift-escape", HideToolWindow, None),
        KeyBinding::new("shift-f12", RestoreDefaultLayout, None),
        KeyBinding::new("up", ExplorerUp, explorer),
        KeyBinding::new("down", ExplorerDown, explorer),
        KeyBinding::new("right", ExplorerExpand, explorer),
        KeyBinding::new("left", ExplorerCollapse, explorer),
        KeyBinding::new("enter", ExplorerOpen, explorer),
        KeyBinding::new("secondary-shift-a", FindAction, None),
        KeyBinding::new("up", SelectPrevious, Some("FindAction")),
        KeyBinding::new("down", SelectNext, Some("FindAction")),
        KeyBinding::new("enter", Choose, Some("FindAction")),
        KeyBinding::new("escape", Dismiss, Some("FindAction")),
        KeyBinding::new("escape", Dismiss, Some("SettingsDialog")),
        KeyBinding::new("enter", Choose, Some("SettingsDialog")),
        // The field's editing keys are the editor's own.
        KeyBinding::new("left", Left, Some("TextField")),
        KeyBinding::new("right", Right, Some("TextField")),
        KeyBinding::new("shift-left", SelectLeft, Some("TextField")),
        KeyBinding::new("shift-right", SelectRight, Some("TextField")),
        KeyBinding::new("home", LineStart, Some("TextField")),
        KeyBinding::new("end", LineEnd, Some("TextField")),
        KeyBinding::new("backspace", Backspace, Some("TextField")),
        KeyBinding::new("delete", Delete, Some("TextField")),
        KeyBinding::new("secondary-a", SelectAll, Some("TextField")),
        KeyBinding::new("secondary-c", Copy, Some("TextField")),
        KeyBinding::new("secondary-x", Cut, Some("TextField")),
        KeyBinding::new("secondary-v", Paste, Some("TextField")),
    ]);
    if mac {
        cx.bind_keys([
            KeyBinding::new("alt-left", WordLeft, editor),
            KeyBinding::new("alt-right", WordRight, editor),
            KeyBinding::new("alt-shift-left", SelectWordLeft, editor),
            KeyBinding::new("alt-shift-right", SelectWordRight, editor),
            KeyBinding::new("cmd-left", LineStart, editor),
            KeyBinding::new("cmd-right", LineEnd, editor),
            KeyBinding::new("cmd-shift-left", SelectLineStart, editor),
            KeyBinding::new("cmd-shift-right", SelectLineEnd, editor),
            KeyBinding::new("cmd-up", DocumentStart, editor),
            KeyBinding::new("cmd-down", DocumentEnd, editor),
            KeyBinding::new("cmd-shift-up", SelectDocumentStart, editor),
            KeyBinding::new("cmd-shift-down", SelectDocumentEnd, editor),
            KeyBinding::new("cmd-shift-z", Redo, editor),
            KeyBinding::new("ctrl-g", SelectNextOccurrence, editor),
            KeyBinding::new("ctrl-shift-g", UnselectOccurrence, editor),
            KeyBinding::new("ctrl-cmd-g", SelectAllOccurrences, editor),
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-,", OpenSettings, None),
            KeyBinding::new("alt-left", WordLeft, Some("TextField")),
            KeyBinding::new("alt-right", WordRight, Some("TextField")),
            KeyBinding::new("cmd-left", LineStart, Some("TextField")),
            KeyBinding::new("cmd-right", LineEnd, Some("TextField")),
        ]);
    } else {
        cx.bind_keys([
            KeyBinding::new("ctrl-left", WordLeft, editor),
            KeyBinding::new("ctrl-right", WordRight, editor),
            KeyBinding::new("ctrl-shift-left", SelectWordLeft, editor),
            KeyBinding::new("ctrl-shift-right", SelectWordRight, editor),
            KeyBinding::new("ctrl-home", DocumentStart, editor),
            KeyBinding::new("ctrl-end", DocumentEnd, editor),
            KeyBinding::new("ctrl-shift-home", SelectDocumentStart, editor),
            KeyBinding::new("ctrl-shift-end", SelectDocumentEnd, editor),
            KeyBinding::new("ctrl-y", Redo, editor),
            KeyBinding::new("ctrl-shift-z", Redo, editor),
            KeyBinding::new("alt-shift-.", SelectNextOccurrence, editor),
            KeyBinding::new("alt-shift-,", UnselectOccurrence, editor),
            KeyBinding::new("alt-shift-;", SelectAllOccurrences, editor),
            KeyBinding::new("alt-f4", Quit, None),
            KeyBinding::new("ctrl-alt-s", OpenSettings, None),
            KeyBinding::new("ctrl-left", WordLeft, Some("TextField")),
            KeyBinding::new("ctrl-right", WordRight, Some("TextField")),
        ]);
    }
}
