//! The keys, as Rider's: its macOS keymap on macOS and its Visual Studio keymap elsewhere.

use crate::editor::*;
use crate::window::*;
use gpui::{App, KeyBinding};

gpui::actions!(app, [Quit]);

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
        KeyBinding::new("secondary-s", Save, None),
        KeyBinding::new("secondary-w", CloseTab, None),
        KeyBinding::new("secondary-o", OpenFolder, None),
        KeyBinding::new(if mac { "cmd-1" } else { "alt-1" }, ToggleSolution, None),
        KeyBinding::new("up", ExplorerUp, explorer),
        KeyBinding::new("down", ExplorerDown, explorer),
        KeyBinding::new("right", ExplorerExpand, explorer),
        KeyBinding::new("left", ExplorerCollapse, explorer),
        KeyBinding::new("enter", ExplorerOpen, explorer),
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
            KeyBinding::new("cmd-q", Quit, None),
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
            KeyBinding::new("alt-f4", Quit, None),
        ]);
    }
}
