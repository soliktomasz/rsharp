//! A folder's breakpoints, kept with it as Rider keeps them: the lines of each file, and what each one asks of the
//! debugger (its condition, its log message, whether it is on). Data alone; a session in `rsharp-dap` sends them.

use crate::state::StateDocument;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One line breakpoint: where it is, and what it asks of the debugger at a hit.
// shortcut: a breakpoint follows its line by number, so an edit above it moves it off its statement; carry it through
// the buffer's changes, as Rider does, once that is missed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LineBreakpoint {
    /// From 0.
    pub line: usize,
    pub enabled: bool,
    /// A C# expression the debugger evaluates where it stopped: it stops only when this is true.
    pub condition: String,
    /// A message logged where the program would stop, in `{expression}` braces; empty for one that stops.
    pub log_message: String,
}

impl Default for LineBreakpoint {
    fn default() -> Self {
        Self { line: 0, enabled: true, condition: String::new(), log_message: String::new() }
    }
}

/// The folder's breakpoints, by file. Kept with the folder, each file's path relative to it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Breakpoints {
    files: BTreeMap<PathBuf, Vec<LineBreakpoint>>,
}

impl Breakpoints {
    /// The enabled lines of a file, from 0, sorted.
    pub fn lines(&self, path: &Path) -> Vec<usize> {
        let mut lines: Vec<usize> = self
            .files
            .iter()
            .filter(|(file, _)| crate::paths::same(file, path))
            .flat_map(|(_, breakpoints)| breakpoints.iter())
            .filter(|breakpoint| breakpoint.enabled)
            .map(|breakpoint| breakpoint.line)
            .collect();
        lines.sort_unstable();
        lines.dedup();
        lines
    }

    /// Every breakpoint of a file, on or not, in line order.
    pub fn of(&self, path: &Path) -> Vec<LineBreakpoint> {
        let mut breakpoints = self
            .files
            .iter()
            .filter(|(file, _)| crate::paths::same(file, path))
            .flat_map(|(_, breakpoints)| breakpoints.clone())
            .collect::<Vec<_>>();
        breakpoints.sort_by_key(|breakpoint| breakpoint.line);
        breakpoints
    }

    /// Sets or clears a line, as F9 does: the one there goes, or a new one stands in its place.
    pub fn toggle(&mut self, path: &Path, line: usize) {
        let key = self.files.keys().find(|file| crate::paths::same(file, path)).cloned();
        let breakpoints = match key {
            Some(key) => self.files.get_mut(&key).expect("the key came from the map"),
            None => self.files.entry(path.to_path_buf()).or_default(),
        };
        match breakpoints.iter().position(|breakpoint| breakpoint.line == line) {
            Some(at) => {
                breakpoints.remove(at);
            }
            None => breakpoints.push(LineBreakpoint { line, ..LineBreakpoint::default() }),
        }
        if breakpoints.is_empty() {
            self.files.retain(|_, breakpoints| !breakpoints.is_empty());
        }
    }

    /// Whether a line holds a breakpoint, on or not.
    pub fn has(&self, path: &Path, line: usize) -> bool {
        self.files
            .iter()
            .filter(|(file, _)| crate::paths::same(file, path))
            .any(|(_, breakpoints)| breakpoints.iter().any(|breakpoint| breakpoint.line == line))
    }

    pub fn breakpoint(&self, path: &Path, line: usize) -> Option<&LineBreakpoint> {
        self.files
            .iter()
            .filter(|(file, _)| crate::paths::same(file, path))
            .flat_map(|(_, breakpoints)| breakpoints)
            .find(|breakpoint| breakpoint.line == line)
    }

    /// Every enabled breakpoint there is, by file, as a session is told of them.
    pub fn enabled(&self) -> Vec<(PathBuf, Vec<LineBreakpoint>)> {
        self.files
            .iter()
            .filter_map(|(file, breakpoints)| {
                let enabled: Vec<LineBreakpoint> =
                    breakpoints.iter().filter(|breakpoint| breakpoint.enabled).cloned().collect();
                (!enabled.is_empty()).then(|| (file.clone(), enabled))
            })
            .collect()
    }

    /// How many breakpoints there are, on or off: what the window says.
    pub fn count(&self) -> usize {
        self.files.values().map(Vec::len).sum()
    }

    /// Sets whether every breakpoint of a file is on (Mute Breakpoints asks for all of them).
    pub fn set_enabled(&mut self, path: &Path, line: usize, enabled: bool) {
        if let Some(breakpoints) =
            self.files.iter_mut().find(|(file, _)| crate::paths::same(file, path)).map(|(_, breakpoints)| breakpoints)
            && let Some(breakpoint) = breakpoints.iter_mut().find(|breakpoint| breakpoint.line == line)
        {
            breakpoint.enabled = enabled;
        }
    }

    /// The document with each file relative to the folder, so it survives the folder being moved.
    pub fn for_store(&self, folder: &Path) -> Self {
        let folder = crate::paths::canonical(folder);
        Self {
            files: self
                .files
                .iter()
                .map(|(file, breakpoints)| {
                    let relative = file.strip_prefix(&folder).map(Path::to_path_buf).unwrap_or_else(|_| file.clone());
                    (relative, breakpoints.clone())
                })
                .collect(),
        }
    }

    /// The document with each file absolute again, as a session needs it.
    pub fn resolved(&self, folder: &Path) -> Self {
        let folder = crate::paths::canonical(folder);
        Self {
            files: self
                .files
                .iter()
                .map(|(file, breakpoints)| {
                    let full = if file.is_absolute() { file.clone() } else { folder.join(file) };
                    (crate::paths::canonical(&full), breakpoints.clone())
                })
                .collect(),
        }
    }
}

impl StateDocument for Breakpoints {
    const FILE: &'static str = "breakpoints.json";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_toggled_and_kept_relative_to_the_folder() {
        let folder = std::env::temp_dir().join(format!("rsharp-breakpoints-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("App")).unwrap();
        let folder = crate::paths::canonical(&folder);
        let file = folder.join("App").join("Program.cs");
        std::fs::write(&file, "class Program {}\n").unwrap();
        let mut breakpoints = Breakpoints::default();
        breakpoints.toggle(&file, 4);
        breakpoints.toggle(&file, 2);
        assert_eq!(breakpoints.lines(&file), vec![2, 4], "the lines a session is told of are in order");
        assert!(breakpoints.has(&file, 4) && !breakpoints.has(&file, 3));
        assert_eq!(breakpoints.count(), 2);

        let stored = breakpoints.for_store(&folder);
        assert_eq!(
            stored.files.keys().collect::<Vec<_>>(),
            vec![&PathBuf::from("App/Program.cs")],
            "a breakpoint inside the folder is kept relative"
        );
        assert_eq!(stored.clone().resolved(&folder), breakpoints, "and read back absolute");

        breakpoints.toggle(&file, 4);
        assert_eq!(breakpoints.lines(&file), vec![2], "the second F9 clears it");
        breakpoints.set_enabled(&file, 2, false);
        assert!(breakpoints.lines(&file).is_empty(), "a muted breakpoint is not sent");
        assert!(breakpoints.enabled().is_empty(), "nothing is sent at all");
        assert_eq!(breakpoints.count(), 1, "but it is still there");
        let _ = std::fs::remove_dir_all(&folder);
    }
}
