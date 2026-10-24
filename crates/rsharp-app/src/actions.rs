//! Every action a person can run by name: its title, the menu it sits in and the other words Find Action
//! finds it by. The main menu and Find Action both read this list, and keys run the same gpui actions,
//! so an action is one entry here, one binding in `keymap.rs` and one handler where it acts.

use crate::editor::{BaseSymbols, DerivedSymbols};
use crate::editor::{
    CaretPerLine, CloneCaretAbove, CloneCaretBelow, Copy, Cut, Paste, Redo, Save, SelectAll, SelectAllOccurrences,
    SelectNextOccurrence, ToggleColumnMode, Undo, UnselectOccurrence,
};
use crate::editor::{ChangeSignature, ExtractInterface, MoveType, PullMembersUp, PushMembersDown};
use crate::editor::{
    CollapseAll, CollapseRegion, DecreaseFontSize, ExpandAll, ExpandRegion, IncreaseFontSize, MatchBrace,
    ResetFontSize, ToggleFold, ToggleSoftWrap, ToggleWhitespace,
};
use crate::editor::{
    Complete, FindUsages, GoToDeclaration, GoToImplementation, GoToTypeDeclaration, ParameterInfo, QuickDocumentation,
    Reformat, Rename, ShowContextActions,
};
use crate::editor::{
    CompleteStatement, EncapsulateField, ExtractMethod, Inline, IntroduceField, IntroduceParameter, IntroduceVariable,
    OptimizeUsings, SafeDelete, SurroundWith,
};
use crate::find_bar::{Find, FindNext, FindPrevious, Replace};
use crate::keymap::{OpenSettings, Quit};
use crate::window::{Back, FindInFiles, Forward, GoToFile, GoToSymbol, GoToType, LastEditLocation, RecentFiles};
use crate::window::{
    CloseAllButPinned, CloseAllTabs, CloseOtherTabs, CloseTabsToLeft, CloseTabsToRight, FileEncoding, LineSeparators,
    OpenTabInNewWindow, PinTab, ReopenClosedTab, ShowHistory, SplitDown, SplitRight, Unsplit, UnsplitAll,
};
use crate::window::{
    CloseSolution, CloseTab, FindAction, FloatToolWindow, HideToolWindow, OpenFolder, RestoreDefaultLayout,
    ToggleNuGet, ToggleProblems, ToggleSolution,
};
use crate::window::{NewProject, NewSolution};
use crate::window::{RecentLocations, ReplaceInFiles, SearchEverywhere};
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
    ide!("New Solution…", "File", ["create solution"], NewSolution),
    ide!("New Project…", "File", ["add project", "create project"], NewProject),
    ide!("Open…", "File", ["open folder", "open solution", "open project"], OpenFolder),
    ide!("Close Solution", "File", ["close project"], CloseSolution),
    ide!("Save", "File", ["save all"], Save),
    ide!("Show Local History", "File", ["local history", "show history", "revisions"], ShowHistory),
    ide!("File Encoding", "File", ["encoding", "charset", "file properties"], FileEncoding),
    ide!("Line Separators", "File", ["line endings", "crlf", "lf", "file properties"], LineSeparators),
    ide!("Close Tab", "Window", ["close editor"], CloseTab),
    ide!("Close Other Tabs", "Window", ["close others", "editor tabs"], CloseOtherTabs),
    ide!("Close All Tabs", "Window", ["close all", "editor tabs"], CloseAllTabs),
    ide!("Close All but Pinned", "Window", ["close unpinned", "editor tabs"], CloseAllButPinned),
    ide!("Close Tabs to the Left", "Window", ["close left", "editor tabs"], CloseTabsToLeft),
    ide!("Close Tabs to the Right", "Window", ["close right", "editor tabs"], CloseTabsToRight),
    ide!("Reopen Closed Tab", "Window", ["undo close", "editor tabs"], ReopenClosedTab),
    ide!("Pin Tab", "Window", ["unpin tab", "pin active tab", "editor tabs"], PinTab),
    ide!("Split Right", "Window", ["split vertically", "editor tabs"], SplitRight),
    ide!("Split Down", "Window", ["split horizontally", "editor tabs"], SplitDown),
    ide!("Unsplit", "Window", ["editor tabs"], Unsplit),
    ide!("Unsplit All", "Window", ["editor tabs"], UnsplitAll),
    ide!("Open Tab in New Window", "Window", ["move to new window", "editor tabs"], OpenTabInNewWindow),
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
    ide!("Back", "Navigate", ["navigate back", "previous place"], Back),
    ide!("Forward", "Navigate", ["navigate forward", "next place"], Forward),
    ide!("Last Edit Location", "Navigate", ["last change", "jump to last change"], LastEditLocation),
    ide!("Recent Files", "View", ["recently opened", "switcher"], RecentFiles),
    ide!("File…", "Navigate", ["go to file", "open file", "find file"], GoToFile),
    ide!("Class…", "Navigate", ["go to type", "go to class", "find type"], GoToType),
    ide!("Symbol…", "Navigate", ["go to symbol", "go to member", "find symbol"], GoToSymbol),
    ide!("Find in Files…", "Edit", ["search in files", "find in path", "grep"], FindInFiles),
    ide!("Replace in Files…", "Edit", ["replace in path", "find and replace in files"], ReplaceInFiles),
    ide!("Search Everywhere", "Navigate", ["go to everything", "search all"], SearchEverywhere),
    ide!("Recent Locations", "View", ["recent places", "recently visited"], RecentLocations),
    ide!("Declaration or Usages", "Navigate", ["go to declaration", "go to definition"], GoToDeclaration),
    ide!("Implementation(s)", "Navigate", ["go to implementation", "derived symbols"], GoToImplementation),
    ide!("Base Symbols", "Navigate", ["super method", "go to base", "base class"], BaseSymbols),
    ide!("Derived Symbols", "Navigate", ["go to derived", "inheritors", "subclasses"], DerivedSymbols),
    ide!("Type Declaration", "Navigate", ["go to type declaration", "go to type definition"], GoToTypeDeclaration),
    ide!("Find Usages", "Edit", ["references", "find references", "usages"], FindUsages),
    ide!("Basic Completion", "Code", ["code completion", "intellisense", "complete"], Complete),
    ide!("Parameter Info", "View", ["signature help", "parameters"], ParameterInfo),
    ide!("Quick Documentation", "View", ["hover", "documentation", "quick info"], QuickDocumentation),
    ide!("Show Context Actions", "Code", ["quick fix", "intentions", "alt enter", "code actions"], ShowContextActions),
    ide!("Reformat Code", "Code", ["format", "format document", "reformat"], Reformat),
    ide!("Rename…", "Refactor", ["rename symbol", "refactor"], Rename),
    ide!("Safe Delete…", "Refactor", ["delete symbol", "remove unused"], SafeDelete),
    ide!("Change Signature…", "Refactor", ["change parameters", "reorder parameters"], ChangeSignature),
    ide!("Extract Interface…", "Refactor", ["interface from class"], ExtractInterface),
    ide!("Move…", "Refactor", ["move type", "move to file", "move to namespace"], MoveType),
    ide!("Pull Members Up…", "Refactor", ["move to base class"], PullMembersUp),
    ide!("Push Members Down…", "Refactor", ["move to derived classes"], PushMembersDown),
    ide!("Introduce Variable…", "Refactor", ["extract variable", "introduce local"], IntroduceVariable),
    ide!("Introduce Field…", "Refactor", ["extract field", "introduce constant"], IntroduceField),
    ide!("Introduce Parameter…", "Refactor", ["extract parameter"], IntroduceParameter),
    ide!("Encapsulate Field…", "Refactor", ["property from field", "encapsulate"], EncapsulateField),
    ide!("Extract Method…", "Refactor", ["extract function"], ExtractMethod),
    ide!("Inline…", "Refactor", ["inline variable", "inline method"], Inline),
    ide!("Optimize Usings", "Code", ["optimize imports", "remove unused usings", "sort usings"], OptimizeUsings),
    ide!("Surround With…", "Code", ["wrap with", "surround"], SurroundWith),
    ide!("Complete Current Statement", "Code", ["complete statement", "smart enter"], CompleteStatement),
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
    ide!("NuGet", "Tools", ["packages", "package manager", "tool window"], ToggleNuGet),
    ide!("Hide Active Tool Window", "Window", ["close tool window"], HideToolWindow),
    ide!("Float Active Tool Window", "Window", ["floating mode", "view mode", "dock", "undock"], FloatToolWindow),
    ide!("Restore Default Layout", "Window", ["reset layout", "tool windows"], RestoreDefaultLayout),
    ide!("Find Action…", "Help", ["search actions", "commands"], FindAction),
    ide!("Quit RSharp", "File", ["exit"], Quit),
];

/// Rider's main menus in its order.
pub const MENUS: &[&str] = &["File", "Edit", "View", "Navigate", "Code", "Refactor", "Tools", "Window", "Help"];

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
