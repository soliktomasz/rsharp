//! Every action a person can run by name: its title, the menu it sits in and the other words Find Action
//! finds it by. The main menu and Find Action both read this list, and keys run the same gpui actions,
//! so an action is one entry here, one binding in `keymap.rs` and one handler where it acts.

use crate::editor::{
    CaretPerLine, CloneCaretAbove, CloneCaretBelow, Copy, Cut, Paste, Redo, Save, SelectAll, SelectAllOccurrences,
    SelectNextOccurrence, ToggleColumnMode, Undo, UnselectOccurrence,
};
use crate::editor::{
    CollapseAll, CollapseRegion, DecreaseFontSize, ExpandAll, ExpandRegion, IncreaseFontSize, MatchBrace,
    ResetFontSize, ToggleFold, ToggleSoftWrap, ToggleWhitespace,
};
use crate::find_bar::{Find, FindNext, FindPrevious, Replace};
use crate::keymap::{OpenSettings, Quit};
use crate::window::{
    CloseSolution, CloseTab, FindAction, FloatToolWindow, HideToolWindow, OpenFolder, RestoreDefaultLayout,
    ToggleProblems, ToggleSolution,
};
use gpui::{Action, Menu, MenuItem};

pub struct IdeAction {
    pub title: &'static str,
    /// The main menu it sits in, by Rider's name.
    pub menu: &'static str,
    /// Other names Find Action finds it by, as Rider's older or plainer names.
    pub words: &'static [&'static str],
    pub action: fn() -> Box<dyn Action>,
}

macro_rules! ide {
    ($title:literal, $menu:literal, [$($word:literal),*], $action:expr) => {
        IdeAction { title: $title, menu: $menu, words: &[$($word),*], action: || Box::new($action) }
    };
}

pub static ALL: &[IdeAction] = &[
    ide!("Open…", "File", ["open folder", "open solution", "open project"], OpenFolder),
    ide!("Close Solution", "File", ["close project"], CloseSolution),
    ide!("Save", "File", ["save all"], Save),
    ide!("Close Tab", "Window", ["close editor"], CloseTab),
    ide!("Settings…", "File", ["preferences", "options"], OpenSettings),
    ide!("Undo", "Edit", [], Undo),
    ide!("Redo", "Edit", [], Redo),
    ide!("Cut", "Edit", [], Cut),
    ide!("Copy", "Edit", [], Copy),
    ide!("Paste", "Edit", [], Paste),
    ide!("Select All", "Edit", [], SelectAll),
    ide!("Find…", "Edit", ["search", "find in file"], Find),
    ide!("Replace…", "Edit", ["find and replace", "replace in file"], Replace),
    ide!("Find Next", "Edit", ["next occurrence", "search"], FindNext),
    ide!("Find Previous", "Edit", ["previous occurrence", "search"], FindPrevious),
    ide!(
        "Add Selection for Next Occurrence",
        "Edit",
        ["select next occurrence", "multiple carets"],
        SelectNextOccurrence
    ),
    ide!("Unselect Occurrence", "Edit", ["multiple carets"], UnselectOccurrence),
    ide!("Select All Occurrences", "Edit", ["multiple carets"], SelectAllOccurrences),
    ide!("Clone Caret Above", "Edit", ["multiple carets", "add caret"], CloneCaretAbove),
    ide!("Clone Caret Below", "Edit", ["multiple carets", "add caret"], CloneCaretBelow),
    ide!("Add Carets to Ends of Selected Lines", "Edit", ["caret per line", "multiple carets"], CaretPerLine),
    ide!("Column Selection Mode", "Edit", ["block selection", "rectangular selection"], ToggleColumnMode),
    ide!("Move Caret to Matching Brace", "Navigate", ["brace", "bracket"], MatchBrace),
    ide!("Expand", "Code", ["unfold", "folding"], ExpandRegion),
    ide!("Collapse", "Code", ["fold", "folding"], CollapseRegion),
    ide!("Expand All", "Code", ["unfold all", "folding"], ExpandAll),
    ide!("Collapse All", "Code", ["fold all", "folding"], CollapseAll),
    ide!("Toggle Folding", "Code", ["expand collapse", "fold", "unfold"], ToggleFold),
    ide!("Soft-Wrap", "View", ["soft wrap", "word wrap", "active editor"], ToggleSoftWrap),
    ide!("Show Whitespaces", "View", ["whitespace", "spaces", "active editor"], ToggleWhitespace),
    ide!("Increase Font Size in All Editors", "View", ["zoom in", "bigger"], IncreaseFontSize),
    ide!("Decrease Font Size in All Editors", "View", ["zoom out", "smaller"], DecreaseFontSize),
    ide!("Reset Font Size", "View", ["zoom", "default size"], ResetFontSize),
    ide!("Solution", "View", ["project", "explorer", "tool window"], ToggleSolution),
    ide!("Problems", "View", ["errors", "warnings", "tool window"], ToggleProblems),
    ide!("Hide Active Tool Window", "Window", ["close tool window"], HideToolWindow),
    ide!("Float Active Tool Window", "Window", ["floating mode", "view mode", "dock", "undock"], FloatToolWindow),
    ide!("Restore Default Layout", "Window", ["reset layout", "tool windows"], RestoreDefaultLayout),
    ide!("Find Action…", "Help", ["search actions", "commands"], FindAction),
    ide!("Quit RSharp", "File", ["exit"], Quit),
];

/// Rider's main menus in its order.
pub const MENUS: &[&str] = &["File", "Edit", "View", "Navigate", "Code", "Window", "Help"];

/// The main menu: macOS's menu bar, the app menu first with Settings and Quit where a Mac keeps them.
// shortcut: Windows and Linux get no in-window menu yet; Rider's is the toolbar's main menu button.
pub fn menus() -> Vec<Menu> {
    let item = |a: &IdeAction| MenuItem::Action { name: a.title.into(), action: (a.action)(), os_action: None };
    let on_mac_app_menu = |a: &IdeAction| cfg!(target_os = "macos") && matches!(a.title, "Settings…" | "Quit RSharp");
    let mut menus = Vec::new();
    if cfg!(target_os = "macos") {
        menus
            .push(Menu { name: "RSharp".into(), items: ALL.iter().filter(|a| on_mac_app_menu(a)).map(item).collect() });
    }
    for name in MENUS {
        let items: Vec<MenuItem> = ALL.iter().filter(|a| a.menu == *name && !on_mac_app_menu(a)).map(item).collect();
        if !items.is_empty() {
            menus.push(Menu { name: (*name).into(), items });
        }
    }
    menus
}
