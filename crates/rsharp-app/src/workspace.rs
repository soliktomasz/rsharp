//! The open folder: its solution and the files the Solution view lists. No UI.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Folders the Solution view leaves out: build output and tools' own state.
const LEFT_OUT: &[&str] = &["bin", "obj", ".git", ".vs", ".idea", "node_modules", "target", "artifacts", ".impeccable"];

pub struct Workspace {
    pub folder: PathBuf,
    pub solution: Option<PathBuf>,
    listings: HashMap<PathBuf, Vec<Entry>>,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_folder: bool,
}

impl Workspace {
    /// Opens a folder, or the folder of a solution or project file, as Rider opens either.
    pub fn open(path: &Path) -> Self {
        let path = rsharp_core::paths::canonical(path);
        let (folder, named) = if path.is_file() {
            (path.parent().map(Path::to_path_buf).unwrap_or_default(), Some(path))
        } else {
            (path, None)
        };
        let solution = named.or_else(|| find_solution(&folder));
        Self { folder, solution, listings: HashMap::new() }
    }

    /// The name the window and the Solution view's root take: the solution's, else the folder's.
    pub fn name(&self) -> String {
        let named = self.solution.as_deref().unwrap_or(&self.folder);
        named.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// A folder's entries, folders first, each by name as a person sorts them.
    // shortcut: read once and kept; a file added on disk shows after the folder is collapsed and opened again
    // only once a watcher (Passenger's FolderWatcher) drops the listing.
    pub fn entries(&mut self, folder: &Path) -> &[Entry] {
        self.listings.entry(folder.to_path_buf()).or_insert_with(|| {
            let mut entries: Vec<Entry> = std::fs::read_dir(folder)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let is_folder = e.file_type().ok()?.is_dir();
                    (!(is_folder && LEFT_OUT.contains(&name.as_str())) && name != ".DS_Store").then(|| Entry {
                        path: e.path(),
                        name,
                        is_folder,
                    })
                })
                .collect();
            entries.sort_by(|a, b| {
                b.is_folder.cmp(&a.is_folder).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            entries
        })
    }

    pub fn forget(&mut self, folder: &Path) {
        self.listings.remove(folder);
    }
}

/// The folder's solution: a .slnx before a .sln, as `dotnet` picks; one of several by name.
fn find_solution(folder: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("slnx" | "sln")))
        .collect();
    found.sort_by_key(|p| (p.extension().and_then(|e| e.to_str()) != Some("slnx"), p.clone()));
    found.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_opens_on_its_solution_and_lists_its_folders_first_without_build_output() {
        let folder = std::env::temp_dir().join(format!("rsharp-ws-{}", std::process::id()));
        for dir in ["src", "bin", "obj"] {
            std::fs::create_dir_all(folder.join(dir)).unwrap();
        }
        for file in ["App.sln", "App.slnx", "b.cs", "A.cs"] {
            std::fs::write(folder.join(file), "").unwrap();
        }
        let mut workspace = Workspace::open(&folder);
        assert_eq!(workspace.solution.as_deref().and_then(|s| s.file_name()), Some("App.slnx".as_ref()));
        assert_eq!(workspace.name(), "App");
        let names: Vec<String> = workspace.entries(&workspace.folder.clone()).iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, ["src", "A.cs", "App.sln", "App.slnx", "b.cs"]);
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
