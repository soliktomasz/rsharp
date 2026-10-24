//! What the window keeps between runs, in the IDE's state store: the recent solutions, and each folder's tabs,
//! carets and Solution view.

use rsharp_core::state::{StateDocument, StateStore};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The process's store, set once as the app starts.
pub struct Ide {
    pub store: StateStore,
    /// Whether a folder starts its language servers; a test that checks none turns them off.
    pub servers: bool,
}

impl gpui::Global for Ide {}

pub fn store(cx: &gpui::App) -> StateStore {
    cx.global::<Ide>().store.clone()
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RecentSolutions {
    pub entries: Vec<RecentSolution>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RecentSolution {
    /// What was opened: the solution or project file, or the folder.
    pub path: PathBuf,
    pub name: String,
}

impl StateDocument for RecentSolutions {
    const FILE: &'static str = "recentSolutions.json";
}

impl RecentSolutions {
    const KEPT: usize = 30;

    pub fn opened(&mut self, path: PathBuf, name: String) {
        self.entries.retain(|e| e.path != path);
        self.entries.insert(0, RecentSolution { path, name });
        self.entries.truncate(Self::KEPT);
    }

    /// The ones still on disk, newest first.
    pub fn existing(&self) -> impl Iterator<Item = &RecentSolution> {
        self.entries.iter().filter(|e| e.path.exists())
    }
}

/// A folder's editors and Solution view as they were left, paths relative to the folder.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct FolderState {
    pub tabs: Vec<KeptTab>,
    pub active: Option<usize>,
    pub expanded: Vec<PathBuf>,
    /// The Solution view's open nodes, by their keys, for a folder with a solution.
    pub open: Vec<String>,
    /// The folder's tool windows, else the last layout the person left.
    pub dock: Option<crate::dock::DockLayout>,
    /// Whether Roslyn analyses every file, not only the open ones, for Problems' Solution tab.
    pub solution_wide: bool,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct KeptTab {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub pinned: bool,
}

impl StateDocument for FolderState {
    const FILE: &'static str = "editors.json";
}
