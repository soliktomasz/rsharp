//! Every action a person can run by name: its title, the menu it sits in and the other words Find Action
//! finds it by. The main menu and Find Action both read this list, and keys run the same gpui actions,
//! so an action is one entry here, one binding in `keymap.rs` and one handler where it acts.

use crate::editor::{Copy, Cut, Paste, Redo, Save, SelectAll, Undo};
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
