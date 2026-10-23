//! A project as MSBuild evaluates it, in Passenger's Host (its `EvaluatedProject`, read as the Host writes it), and
//! where the Solution view shows each file it holds.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EvaluatedProject {
    pub path: PathBuf,
    /// False for a multi-targeted project known so far by its outer evaluation alone; its frameworks' follow.
    #[serde(default = "yes")]
    pub complete: bool,
    /// Why it did not evaluate: MSBuild's message, with the file and line it names.
    pub problem: Option<String>,
    pub assembly_name: Option<String>,
    pub output_type: Option<String>,
    pub target_frameworks: Vec<String>,
    pub is_test_project: bool,
    pub dependencies: Vec<FrameworkDependencies>,
    /// Every file the project holds, each once; the Host leaves out those it hides (`Visible="false"`).
    pub items: Vec<EvaluatedItem>,
    pub inputs: Vec<Stamp>,
    pub folders: Vec<Stamp>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FrameworkDependencies {
    pub framework: Option<String>,
    pub packages: Vec<PackageDependency>,
    pub projects: Vec<PathBuf>,
    pub framework_references: Vec<String>,
    pub windows_targeting: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PackageDependency {
    pub name: String,
    pub version: Option<String>,
    /// Added by the SDK itself, which Rider's Dependencies leaves out.
    pub implicit: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EvaluatedItem {
    pub r#type: String,
    pub path: PathBuf,
    /// Where it shows when it lives elsewhere, relative to the project's folder.
    pub link: Option<String>,
    /// The file it depends on, relative to its own folder, as MSBuild's DependentUpon is.
    pub dependent_upon: Option<String>,
}

/// What says an evaluation still stands: a file's last write (.NET ticks) and length, or a folder's names hashed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Stamp {
    pub path: PathBuf,
    pub ticks: i64,
    pub length: i64,
    pub folder: bool,
}

/// The Host leaves `complete` out when it is true.
fn yes() -> bool {
    true
}

/// The files no project means to hold: the system's folder metadata.
const HIDDEN: &[&str] = &[".ds_store", "thumbs.db"];

/// An evaluated project's files by where the Solution view shows them, as Rider's does: each in its folder under the
/// project's, one from elsewhere at its Link or else at the project's top, and its Folder items. Places are relative
/// to the project's folder with '/' between folders ("" its top), matched without regard to letter case.
pub struct ProjectFiles {
    root: PathBuf,
    files: HashMap<String, Vec<(String, PathBuf)>>,
    folders: HashMap<String, Vec<String>>,
}

impl ProjectFiles {
    pub fn new(project: &EvaluatedProject) -> Self {
        let root = project.path.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut placed = Self { root, files: HashMap::new(), folders: HashMap::new() };
        let mut seen = HashSet::new();
        for item in &project.items {
            let logical = placed.logical(item);
            if item.r#type == "Folder" {
                placed.add_folder(&logical);
                continue;
            }
            let (folder, name) = logical.rsplit_once('/').unwrap_or(("", &logical));
            if name.is_empty() || HIDDEN.contains(&name.to_lowercase().as_str()) || !seen.insert(item.path.clone()) {
                continue;
            }
            placed.add_folder(folder);
            placed.files.entry(folder.to_lowercase()).or_default().push((name.to_string(), item.path.clone()));
        }
        placed
    }

    fn logical(&self, item: &EvaluatedItem) -> String {
        if let Some(link) = item.link.as_deref().filter(|l| !l.is_empty()) {
            return link.replace('\\', "/").trim_matches('/').to_string();
        }
        match item.path.strip_prefix(&self.root) {
            Ok(inside) => inside.to_string_lossy().replace('\\', "/").trim_matches('/').to_string(),
            Err(_) => item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        }
    }

    fn add_folder(&mut self, relative: &str) {
        let mut relative = relative.to_string();
        while !relative.is_empty() {
            let (parent, name) = relative.rsplit_once('/').unwrap_or(("", &relative));
            let names = self.folders.entry(parent.to_lowercase()).or_default();
            if names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                return;
            }
            names.push(name.to_string());
            relative = parent.to_string();
        }
    }

    /// A place in the project as a path on disk, whether or not a folder is there.
    pub fn path_of(&self, relative: &str) -> PathBuf {
        if relative.is_empty() { self.root.clone() } else { self.root.join(relative) }
    }

    /// The folders at a place, by name: those the project holds something in, and those on disk with nothing in them,
    /// since MSBuild has nothing to say of one and one just made would otherwise vanish.
    pub fn folders_in(&self, relative: &str) -> Vec<String> {
        let mut names = self.folders.get(&relative.to_lowercase()).cloned().unwrap_or_default();
        for entry in std::fs::read_dir(self.path_of(relative)).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if path.is_dir()
                && !name.starts_with('.')
                && !matches!(name.as_str(), "bin" | "obj")
                && !names.iter().any(|n| n.eq_ignore_ascii_case(&name))
                && !holds_a_file(&path)
            {
                names.push(name);
            }
        }
        names.sort_by_key(|n| n.to_lowercase());
        names
    }

    /// The files at a place, each with the name it shows by and its path, by name.
    pub fn files_in(&self, relative: &str) -> Vec<(String, PathBuf)> {
        let mut files = self.files.get(&relative.to_lowercase()).cloned().unwrap_or_default();
        files.sort_by_key(|(n, _)| n.to_lowercase());
        files
    }
}

fn holds_a_file(folder: &Path) -> bool {
    std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.path().is_file() || (e.path().is_dir() && holds_a_file(&e.path())))
}

/// .NET's ticks (100 ns since 0001-01-01) at the Unix epoch.
const UNIX_EPOCH_TICKS: i64 = 621_355_968_000_000_000;

/// Names in a folder that say nothing of what a project holds, as Passenger.Host's `Stamps` passes over them.
const PASSED: &[&str] = &["bin", "obj", "node_modules", ".ds_store", "thumbs.db"];

impl Stamp {
    /// A file's last write and length as Passenger.Host's `Stamps.File` takes them; one not there has length -1.
    pub fn file(path: &Path) -> Self {
        let Ok(meta) = std::fs::metadata(path) else {
            return Self { path: path.into(), ticks: 0, length: -1, folder: false };
        };
        let since = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).unwrap_or_default();
        let ticks = UNIX_EPOCH_TICKS + (since.as_nanos() / 100) as i64;
        Self { path: path.into(), ticks, length: meta.len() as i64, folder: false }
    }

    /// A folder's names hashed as Passenger.Host's `Stamps.Folder` hashes them (FNV-1a over each name's UTF-16, a
    /// folder's with '/', and a newline), with how many there are; its own last write would change with every build.
    pub fn folder(path: &Path) -> Self {
        let Ok(entries) = std::fs::read_dir(path) else {
            return Self { path: path.into(), ticks: 0, length: -1, folder: true };
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let folder = e.path().is_dir();
                let counts = !PASSED.contains(&name.to_lowercase().as_str()) && !(folder && name.starts_with('.'));
                counts.then(|| if folder { format!("{name}/") } else { name })
            })
            .collect();
        // shortcut: sorted by UTF-8 bytes where .NET sorts by UTF-16 units, which differ only for names with characters
        // past U+FFFF beside ones in U+E000–U+FFFF; such a folder reads as changed each start, until it is worth a sort by UTF-16.
        names.sort();
        let mut hash: u64 = 14695981039346656037;
        for name in &names {
            for unit in name.encode_utf16().chain(['\n' as u16]) {
                hash = (hash ^ unit as u64).wrapping_mul(1099511628211);
            }
        }
        Self { path: path.into(), ticks: hash as i64, length: names.len() as i64, folder: true }
    }

    /// Whether what was stamped is as it was.
    pub fn holds(&self) -> bool {
        *self == if self.folder { Self::folder(&self.path) } else { Self::file(&self.path) }
    }
}

impl EvaluatedProject {
    /// Whether anything it was evaluated from changed since: its project file, an import, or a folder its wildcards walked.
    pub fn is_stale(&self) -> bool {
        !self.inputs.iter().chain(&self.folders).all(Stamp::holds)
    }

    /// The file the project says `file` depends on, as a path.
    pub fn dependent_upon(&self, file: &Path) -> Option<PathBuf> {
        let parent = self.items.iter().find(|i| i.path == file)?.dependent_upon.as_deref()?;
        Some(file.parent()?.join(parent.replace('\\', "/")))
    }
}

/// The folder's last evaluations, so a window opened again shows its projects before any is evaluated again.
#[derive(Default, Serialize, Deserialize)]
pub struct Evaluations {
    pub projects: Vec<EvaluatedProject>,
}

impl crate::state::StateDocument for Evaluations {
    const FILE: &'static str = "projectModels.json";
}
