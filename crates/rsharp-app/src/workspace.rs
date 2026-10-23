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
        let solution = named.or_else(|| rsharp_core::solution::find(&folder));
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

/// Every file under `folder` a search looks in, at most `limit`: build output, tools' state and hidden folders
/// (worktrees, caches) left out.
// shortcut: .gitignore is not read; read it once a project's ignored folder that is not hidden is searched.
pub fn all_files(folder: &Path, limit: usize) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            match entry.file_type() {
                Ok(t) if t.is_dir() && !LEFT_OUT.contains(&name.as_ref()) && !name.starts_with('.') => {
                    folders.push(entry.path())
                }
                Ok(t) if t.is_file() && name != ".DS_Store" => files.push(entry.path()),
                _ => {}
            }
            if files.len() >= limit {
                return files;
            }
        }
    }
    files.sort();
    files
}

/// A line Find in Files found: its file, line and UTF-16 column (from 0), and the line's text.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub text: String,
}

/// Every place `query` is in `text`, as byte ranges, ASCII case ignored as Rider's default (a non-ASCII letter matches
/// as written, so every range is exact in the text).
pub fn matches(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let (text, wanted) = (text.as_bytes(), query.as_bytes());
    let mut found = Vec::new();
    if wanted.is_empty() {
        return found;
    }
    let mut at = 0;
    while at + wanted.len() <= text.len() {
        if text[at..at + wanted.len()].eq_ignore_ascii_case(wanted) {
            found.push(at..at + wanted.len());
            at += wanted.len();
        } else {
            at += 1;
        }
    }
    found
}

/// A file's text as a search reads it: the open text (`open`, unsaved edits included), else the disk's; none for a
/// file over 4 MB or not text.
pub fn searched_text(path: &Path, open: &HashMap<PathBuf, String>) -> Option<String> {
    if let Some(text) = open.get(path) {
        return Some(text.clone());
    }
    if std::fs::metadata(path).map_or(true, |m| m.len() > 4 << 20) {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    (!bytes.contains(&0)).then(|| String::from_utf8_lossy(&bytes).into_owned())
}

/// Find in Files: each line holding `query`, at its first match, in the files' text as `searched_text` reads it. At
/// most `limit`.
pub fn find_in_files(files: &[PathBuf], open: &HashMap<PathBuf, String>, query: &str, limit: usize) -> Vec<Hit> {
    let mut hits = Vec::new();
    if query.is_empty() {
        return hits;
    }
    for path in files {
        let Some(text) = searched_text(path, open) else { continue };
        for (line, content) in text.lines().enumerate() {
            if let Some(found) = matches(content, query).first() {
                let column = content[..found.start].encode_utf16().count();
                hits.push(Hit { path: path.clone(), line, column, text: content.trim().to_string() });
                if hits.len() >= limit {
                    return hits;
                }
            }
        }
    }
    hits
}

/// Whether a file's name fits a mask: patterns apart by commas, each with `*` for any run and `?` for one character
/// (`*.cs, *.xaml`), case ignored; an empty mask fits every file.
pub fn fits_mask(name: &str, mask: &str) -> bool {
    fn glob(name: &[char], pattern: &[char]) -> bool {
        match pattern.split_first() {
            None => name.is_empty(),
            Some(('*', rest)) => (0..=name.len()).any(|skip| glob(&name[skip..], rest)),
            Some((c, rest)) => {
                name.first().is_some_and(|n| *c == '?' || n.eq_ignore_ascii_case(c)) && glob(&name[1..], rest)
            }
        }
    }
    let name: Vec<char> = name.chars().collect();
    let mut patterns = mask.split(',').map(str::trim).filter(|p| !p.is_empty()).peekable();
    patterns.peek().is_none() || patterns.any(|p| glob(&name, &p.chars().collect::<Vec<_>>()))
}

/// The folder's solution: a .slnx before a .sln, as `dotnet` picks; one of several by name.
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

    #[test]
    fn find_in_files_finds_each_line_case_ignored_in_the_open_text_before_the_disks() {
        let folder = std::env::temp_dir().join(format!("rsharp-find-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("obj")).unwrap();
        std::fs::write(folder.join("A.cs"), "class Greeter {}\n// greeter here\n").unwrap();
        std::fs::write(folder.join("B.cs"), "nothing\n").unwrap();
        std::fs::write(folder.join("obj/C.cs"), "Greeter in build output\n").unwrap();
        let files = all_files(&folder, 100);
        assert_eq!(files.len(), 2, "build output was searched: {files:?}");
        let open = HashMap::from([(folder.join("B.cs"), "  var g = new Greeter();".to_string())]);
        let hits = find_in_files(&files, &open, "GREETER", 100);
        let found: Vec<_> =
            hits.iter().map(|h| (h.path.file_name().unwrap().to_str().unwrap(), h.line, h.column)).collect();
        assert_eq!(found, [("A.cs", 0, 6), ("A.cs", 1, 3), ("B.cs", 0, 14)]);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_match_is_exact_bytes_with_ascii_case_ignored_and_a_mask_takes_globs() {
        assert_eq!(matches("Straße GREETer greeter", "greeter"), [8..15, 16..23], "ß is two bytes");
        assert_eq!(matches("aaaa", "aa"), [0..2, 2..4], "matches overlapped");
        assert!(fits_mask("Program.CS", "*.xaml, *.cs"));
        assert!(fits_mask("A.cs", ""));
        assert!(!fits_mask("A.csproj", "*.cs"));
        assert!(fits_mask("A1.cs", "A?.cs"));
    }
}
