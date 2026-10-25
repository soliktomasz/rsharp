//! A folder's breakpoints, kept with it as Rider keeps them: the lines of each file and what each asks of the debugger
//! (its condition, its hit count, its log message, whether it is on), the exception breakpoints, and Mute Breakpoints.
//! Data alone; a session in `rsharp-dap` sends them.

use crate::state::StateDocument;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Rider's "Suspend if" for an exception breakpoint, as the protocol names each case.
pub const SCENARIOS: [&str; 5] =
    ["thrownInUserCode", "thrownInExternalCode", "handledInUserCode", "handledInExternalCode", "unhandled"];

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
    /// Rider's "Hit count": a number, `> 5`, `% 3` and the like, as the debugger's own condition reads it.
    pub hit_count: String,
    /// A message logged where the program would stop, in `{expression}` braces; empty for one that stops.
    pub log_message: String,
}

impl Default for LineBreakpoint {
    fn default() -> Self {
        Self { line: 0, enabled: true, condition: String::new(), hit_count: String::new(), log_message: String::new() }
    }
}

/// Rider's CLR exception breakpoint: the exception types it is about (none for "Any exception"), when it stops, and
/// what a hit asks besides.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExceptionBreakpoint {
    /// The exception's full names, as CLR spells them; none for every exception, Rider's "Any exception".
    pub types: Vec<String>,
    pub enabled: bool,
    /// Which of `SCENARIOS` hold; one of each half must, as Rider's own "Suspend if" says.
    pub scenarios: Vec<String>,
    pub condition: String,
    pub hit_count: String,
    pub log_message: String,
}

impl Default for ExceptionBreakpoint {
    fn default() -> Self {
        Self {
            types: Vec::new(),
            enabled: true,
            scenarios: vec!["unhandled".into()],
            condition: String::new(),
            hit_count: String::new(),
            log_message: String::new(),
        }
    }
}

impl ExceptionBreakpoint {
    /// Rider's "Any exception": every exception the program's own code leaves unhandled.
    pub fn any() -> Self {
        Self::default()
    }

    pub fn is_any(&self) -> bool {
        self.types.is_empty()
    }

    /// The breakpoint as the protocol names it: its type path, and which of Rider's cases stop it.
    pub fn path(&self) -> Vec<String> {
        let mut path = vec!["CLR".to_string()];
        path.extend(self.types.iter().filter(|kind| !kind.trim().is_empty()).cloned());
        path
    }
}

/// The folder's breakpoints: each file's lines, the exception breakpoints ("Any exception" never removed) and whether
/// every breakpoint is muted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Breakpoints {
    files: BTreeMap<PathBuf, Vec<LineBreakpoint>>,
    /// Rider's "Any exception", which every folder has and none removes.
    any_exception: ExceptionBreakpoint,
    /// The other CLR exception breakpoints, by type.
    exceptions: Vec<ExceptionBreakpoint>,
    muted: bool,
}

impl Breakpoints {
    /// The enabled lines of a file, from 0, sorted.
    pub fn lines(&self, path: &Path) -> Vec<usize> {
        if self.muted {
            return Vec::new();
        }
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
        if self.has(path, line) {
            self.remove(path, line);
            return;
        }
        let key = self.files.keys().find(|file| crate::paths::same(file, path)).cloned();
        let breakpoints = match key {
            Some(key) => self.files.get_mut(&key).expect("the key came from the map"),
            None => self.files.entry(path.to_path_buf()).or_default(),
        };
        breakpoints.push(LineBreakpoint { line, ..LineBreakpoint::default() });
    }

    pub fn remove(&mut self, path: &Path, line: usize) {
        let key = self.files.keys().find(|file| crate::paths::same(file, path)).cloned();
        if let Some(key) = key
            && let Some(breakpoints) = self.files.get_mut(&key)
        {
            breakpoints.retain(|breakpoint| breakpoint.line != line);
            if breakpoints.is_empty() {
                self.files.remove(&key);
            }
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

    /// Edits a line's breakpoint as the dialog's fields do.
    pub fn edit(&mut self, path: &Path, line: usize, change: impl FnOnce(&mut LineBreakpoint)) {
        let key = self.files.keys().find(|file| crate::paths::same(file, path)).cloned();
        if let Some(key) = key
            && let Some(breakpoints) = self.files.get_mut(&key)
            && let Some(breakpoint) = breakpoints.iter_mut().find(|breakpoint| breakpoint.line == line)
        {
            change(breakpoint);
        }
    }

    /// Every enabled line breakpoint there is, by file, as a session is told of them. Muted, there are none.
    pub fn enabled(&self) -> Vec<(PathBuf, Vec<LineBreakpoint>)> {
        if self.muted {
            return Vec::new();
        }
        self.files
            .iter()
            .filter_map(|(file, breakpoints)| {
                let enabled: Vec<LineBreakpoint> =
                    breakpoints.iter().filter(|breakpoint| breakpoint.enabled).cloned().collect();
                (!enabled.is_empty()).then(|| (file.clone(), enabled))
            })
            .collect()
    }

    /// Every file with a breakpoint, on or off, as a session must be told of even an emptied one.
    pub fn files(&self) -> Vec<PathBuf> {
        self.files.keys().cloned().collect()
    }

    /// How many breakpoints there are, on or off: what the window says.
    pub fn count(&self) -> usize {
        self.files.values().map(Vec::len).sum::<usize>() + self.exceptions.len() + 1
    }

    pub fn any_exception(&self) -> &ExceptionBreakpoint {
        &self.any_exception
    }

    pub fn any_exception_mut(&mut self) -> &mut ExceptionBreakpoint {
        &mut self.any_exception
    }

    pub fn exceptions(&self) -> &[ExceptionBreakpoint] {
        &self.exceptions
    }

    pub fn exceptions_mut(&mut self) -> &mut Vec<ExceptionBreakpoint> {
        &mut self.exceptions
    }

    /// Adds a CLR exception breakpoint, as the dialog's Add does: a type to be edited, stopping as Rider's new ones do.
    pub fn add_exception(&mut self) {
        self.exceptions.push(ExceptionBreakpoint {
            types: vec!["System.Exception".into()],
            scenarios: vec!["thrownInUserCode".into(), "unhandled".into()],
            ..ExceptionBreakpoint::default()
        });
    }

    pub fn remove_exception(&mut self, index: usize) {
        if index < self.exceptions.len() {
            self.exceptions.remove(index);
        }
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
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
            ..self.clone()
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
            ..self.clone()
        }
    }
}

impl StateDocument for Breakpoints {
    const FILE: &'static str = "breakpoints.json";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("rsharp-breakpoints-{name}-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("App")).unwrap();
        crate::paths::canonical(&folder)
    }

    #[test]
    fn a_line_is_toggled_and_kept_relative_to_the_folder() {
        let folder = folder("lines");
        let file = folder.join("App").join("Program.cs");
        std::fs::write(&file, "class Program {}\n").unwrap();
        let mut breakpoints = Breakpoints::default();
        breakpoints.toggle(&file, 4);
        breakpoints.toggle(&file, 2);
        assert_eq!(breakpoints.lines(&file), vec![2, 4], "the lines a session is told of are in order");
        assert!(breakpoints.has(&file, 4) && !breakpoints.has(&file, 3));
        assert_eq!(breakpoints.count(), 3, "two lines and Any exception");

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
        assert_eq!(breakpoints.count(), 2, "but it is still there");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_breakpoint_keeps_its_condition_hit_count_and_log_message() {
        let folder = folder("details");
        let file = folder.join("App").join("Program.cs");
        std::fs::write(&file, "class Program {}\n").unwrap();
        let mut breakpoints = Breakpoints::default();
        breakpoints.toggle(&file, 6);
        breakpoints.edit(&file, 6, |breakpoint| {
            breakpoint.condition = "a > 3".into();
            breakpoint.hit_count = "> 2".into();
            breakpoint.log_message = "a is {a}".into();
        });
        let kept = breakpoints.of(&file);
        assert_eq!(kept[0].condition, "a > 3");
        assert_eq!(kept[0].hit_count, "> 2");
        assert_eq!(kept[0].log_message, "a is {a}");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn any_exception_is_always_there_and_mute_silences_every_breakpoint() {
        let folder = folder("exceptions");
        let file = folder.join("App").join("Program.cs");
        std::fs::write(&file, "class Program {}\n").unwrap();
        let mut breakpoints = Breakpoints::default();
        assert!(breakpoints.any_exception().is_any(), "a folder begins with Rider's Any exception");
        assert_eq!(breakpoints.any_exception().path(), vec!["CLR"], "it is every exception");
        breakpoints.toggle(&file, 4);
        breakpoints.set_muted(true);
        assert!(breakpoints.lines(&file).is_empty(), "Mute Breakpoints sends none");
        assert!(breakpoints.enabled().is_empty());
        breakpoints.set_muted(false);
        assert_eq!(breakpoints.lines(&file), vec![4]);

        breakpoints.add_exception();
        assert_eq!(breakpoints.exceptions().len(), 1);
        assert_eq!(breakpoints.exceptions()[0].path(), vec!["CLR", "System.Exception"]);
        breakpoints.exceptions_mut()[0].types = vec!["System.InvalidOperationException".into()];
        assert_eq!(
            breakpoints.exceptions()[0].path(),
            vec!["CLR", "System.InvalidOperationException"],
            "a typed exception keeps its full name"
        );
        breakpoints.remove_exception(0);
        assert!(breakpoints.exceptions().is_empty());
        let _ = std::fs::remove_dir_all(&folder);
    }
}
