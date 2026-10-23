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
