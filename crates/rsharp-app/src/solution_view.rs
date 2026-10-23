//! The Solution view's tree, as Rider's: the solution's folders, the files they list and its projects; each project's
//! Dependencies and its files as MSBuild evaluates them (`ProjectFiles`), a file's dependents nested under it. Only
//! open nodes are walked, so a project nobody opened costs nothing; what is open is kept by each node's key.

use rsharp_core::nesting;
use rsharp_core::project::{EvaluatedProject, PackageDependency, ProjectFiles};
use rsharp_core::solution::{Project, Solution, SolutionEntry};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Folder,
    File,
    Solution,
    SolutionFolder,
    Project,
    Dependencies,
    Framework,
    Package,
    ProjectReference,
}

#[derive(Clone, Debug)]
pub struct TreeRow {
    pub kind: NodeKind,
    pub name: String,
    pub detail: Option<String>,
    pub path: Option<PathBuf>,
    pub depth: usize,
    pub expandable: bool,
    pub expanded: bool,
    pub dimmed: bool,
    pub tip: Option<String>,
    /// What keeps it open across a re-evaluation and a restart: unique in the tree, paths relative to the folder.
    pub key: String,
}

/// Never shown from disk: build output, tools' own folders and the system's folder metadata.
fn hidden_on_disk(name: &str, folder: bool) -> bool {
    (folder && (name.starts_with('.') || matches!(name, "bin" | "obj")))
        || name.eq_ignore_ascii_case(".ds_store")
        || name.eq_ignore_ascii_case("thumbs.db")
}

fn by_name(name: &str) -> String {
    name.to_lowercase()
}

pub struct SolutionTree {
    solution: Solution,
    root: PathBuf,
    evaluated: HashMap<PathBuf, EvaluatedProject>,
    files: HashMap<PathBuf, ProjectFiles>,
    open: HashSet<String>,
    rows: Vec<TreeRow>,
}

impl SolutionTree {
    pub fn new(solution: Solution, evaluated: HashMap<PathBuf, EvaluatedProject>, open: HashSet<String>) -> Self {
        let root = solution.path.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut tree = Self { solution, root, evaluated, files: HashMap::new(), open, rows: Vec::new() };
        tree.open.insert("solution".into());
        tree.rebuild();
        tree
    }

    pub fn solution(&self) -> &Solution {
        &self.solution
    }

    /// A project's evaluation, its rows following and what was open kept open.
    pub fn set_evaluated(&mut self, project: EvaluatedProject) {
        self.files.remove(&project.path);
        self.evaluated.insert(project.path.clone(), project);
        self.rebuild();
    }

    pub fn evaluated(&self, project: &Path) -> Option<&EvaluatedProject> {
        self.evaluated.get(project)
    }

    pub fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    /// The keys of the open nodes, to keep for the next time the folder opens.
    pub fn open_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.open.iter().cloned().collect();
        keys.sort();
        keys
    }

    pub fn toggle(&mut self, row: usize, open: Option<bool>) {
        let Some(row) = self.rows.get(row).filter(|r| r.expandable) else { return };
        let key = row.key.clone();
        if open.unwrap_or(!row.expanded) {
            self.open.insert(key);
        } else {
            self.open.remove(&key);
        }
        self.rebuild();
    }

    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root).unwrap_or(path).to_string_lossy().replace('\\', "/")
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        let key = "solution".to_string();
        let expanded = self.open.contains(&key);
        rows.push(TreeRow {
            kind: NodeKind::Solution,
            name: self.solution.name.clone(),
            detail: None,
            path: Some(self.solution.path.clone()),
            depth: 0,
            expandable: true,
            expanded,
            dimmed: false,
            tip: None,
            key,
        });
        if expanded {
            let entries = std::mem::take(&mut self.solution.entries);
            self.entries(&entries, 1, &mut rows);
            self.solution.entries = entries;
        }
        self.rows = rows;
    }

    /// A level of the solution as Rider's Solution view sorts it, whatever order the file declares: its folders, then
    /// its projects, then the files a folder lists, each by name.
    fn entries(&mut self, entries: &[SolutionEntry], depth: usize, rows: &mut Vec<TreeRow>) {
        let mut sorted: Vec<&SolutionEntry> = entries.iter().collect();
        sorted.sort_by_key(|e| match e {
            SolutionEntry::Folder { name, .. } => (0, by_name(name)),
            SolutionEntry::Project(project) => (1, by_name(&project.name)),
            SolutionEntry::Item { name, .. } => (2, by_name(name)),
        });
        for entry in sorted {
            match entry {
                SolutionEntry::Folder { name, path, children } => {
                    let key = format!("sf:{path}");
                    let expanded = self.open.contains(&key);
                    rows.push(self.row(
                        NodeKind::SolutionFolder,
                        name,
                        None,
                        depth,
                        !children.is_empty(),
                        expanded,
                        key,
                    ));
                    if expanded {
                        self.entries(children, depth + 1, rows);
                    }
                }
                SolutionEntry::Project(project) => self.project(project, depth, rows),
                SolutionEntry::Item { name, path, exists } => {
                    let mut row =
                        self.row(NodeKind::File, name, Some(path.clone()), depth, false, false, String::new());
                    row.dimmed = !exists;
                    row.key = format!("si:{}", self.relative(path));
                    rows.push(row);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        kind: NodeKind,
        name: &str,
        path: Option<PathBuf>,
        depth: usize,
        expandable: bool,
        expanded: bool,
        key: String,
    ) -> TreeRow {
        TreeRow {
            kind,
            name: name.to_string(),
            detail: None,
            path,
            depth,
            expandable,
            expanded: expandable && expanded,
            dimmed: false,
            tip: None,
            key,
        }
    }

    fn project(&mut self, project: &Project, depth: usize, rows: &mut Vec<TreeRow>) {
        let key = format!("p:{}", self.relative(&project.path));
        let expanded = self.open.contains(&key);
        let mut row = self.row(
            NodeKind::Project,
            &project.name,
            Some(project.path.clone()),
            depth,
            project.exists,
            expanded,
            key.clone(),
        );
        let evaluated = self.evaluated.get(&project.path).cloned();
        if !project.exists {
            row.dimmed = true;
            row.detail = Some("not found".into());
        } else if let Some(problem) = evaluated.as_ref().and_then(|e| e.problem.clone()) {
            row.detail = Some("load failed".into());
            row.tip = Some(problem);
        } else if let Some(evaluated) = evaluated.as_ref().filter(|e| !e.target_frameworks.is_empty()) {
            row.detail = Some(evaluated.target_frameworks.join(", "));
        }
        let expanded = row.expanded;
        rows.push(row);
        if !expanded {
            return;
        }
        let sound = evaluated.filter(|e| e.problem.is_none());
        self.dependencies(project, sound.as_ref(), depth + 1, rows);
        match sound {
            Some(evaluated) => {
                if !self.files.contains_key(&project.path) {
                    self.files.insert(project.path.clone(), ProjectFiles::new(&evaluated));
                }
                self.held(&project.path, &evaluated, "", depth + 1, rows);
            }
            None => {
                let folder = project.path.parent().map(Path::to_path_buf).unwrap_or_default();
                self.on_disk(&folder, Some(&project.path), depth + 1, rows);
            }
        }
    }

    /// A project's dependencies as MSBuild evaluates them: for one framework, it with the projects and packages it
    /// references; for a multi-targeted project, each framework with its own, as Rider shows them.
    fn dependencies(
        &self,
        project: &Project,
        evaluated: Option<&EvaluatedProject>,
        depth: usize,
        rows: &mut Vec<TreeRow>,
    ) {
        let key = format!("d:{}", self.relative(&project.path));
        let expanded = self.open.contains(&key);
        rows.push(self.row(
            NodeKind::Dependencies,
            "Dependencies",
            None,
            depth,
            evaluated.is_some(),
            expanded,
            key.clone(),
        ));
        let Some(evaluated) = evaluated.filter(|_| expanded) else { return };
        let frameworks = &evaluated.dependencies;
        if frameworks.first().is_some_and(|f| f.framework.is_some()) {
            for framework in frameworks {
                let name = framework.framework.clone().unwrap_or_default();
                let key = format!("{key}|{name}");
                let expanded = self.open.contains(&key);
                rows.push(self.row(NodeKind::Framework, &name, None, depth + 1, true, expanded, key));
                if expanded {
                    self.referenced(&framework.projects, &framework.packages, depth + 2, rows);
                }
            }
            return;
        }
        for framework in &evaluated.target_frameworks {
            rows.push(self.row(
                NodeKind::Framework,
                framework,
                None,
                depth + 1,
                false,
                false,
                format!("{key}|{framework}"),
            ));
        }
        let (projects, packages) = frameworks.first().map(|f| (&f.projects[..], &f.packages[..])).unwrap_or_default();
        self.referenced(projects, packages, depth + 1, rows);
    }

    /// The projects and packages referenced; packages the SDK adds of itself are left out, as Rider leaves them.
    fn referenced(&self, projects: &[PathBuf], packages: &[PackageDependency], depth: usize, rows: &mut Vec<TreeRow>) {
        for project in projects {
            let name = project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            rows.push(self.row(
                NodeKind::ProjectReference,
                &name,
                Some(project.clone()),
                depth,
                false,
                false,
                String::new(),
            ));
        }
        for package in packages.iter().filter(|p| !p.implicit) {
            let mut row = self.row(NodeKind::Package, &package.name, None, depth, false, false, String::new());
            row.detail = package.version.clone();
            rows.push(row);
        }
    }

    /// What an evaluated project holds at one place in it: its folders, then its files, a file's dependents under it.
    fn held(
        &self,
        project: &Path,
        evaluated: &EvaluatedProject,
        relative: &str,
        depth: usize,
        rows: &mut Vec<TreeRow>,
    ) {
        let files = &self.files[project];
        let base = self.relative(project);
        for name in files.folders_in(relative) {
            let inner = if relative.is_empty() { name.clone() } else { format!("{relative}/{name}") };
            let key = format!("pf:{base}|{inner}");
            let expanded = self.open.contains(&key);
            rows.push(self.row(NodeKind::Folder, &name, Some(files.path_of(&inner)), depth, true, expanded, key));
            if expanded {
                self.held(project, evaluated, &inner, depth + 1, rows);
            }
        }
        let held = files.files_in(relative);
        let paths: Vec<PathBuf> = held.iter().map(|(_, p)| p.clone()).collect();
        let parents = nesting::parents(&paths, |f| evaluated.dependent_upon(f), true);
        self.files_with_dependents(&held, &parents, depth, rows);
    }

    /// The files in a folder on disk: a project's until MSBuild has evaluated it, or when it could not.
    fn on_disk(&self, folder: &Path, project_file: Option<&Path>, depth: usize, rows: &mut Vec<TreeRow>) {
        let mut folders = Vec::new();
        let mut files = Vec::new();
        for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
            let (path, name) = (entry.path(), entry.file_name().to_string_lossy().into_owned());
            let is_folder = path.is_dir();
            if hidden_on_disk(&name, is_folder) || Some(path.as_path()) == project_file {
                continue;
            }
            if is_folder { folders.push((name, path)) } else { files.push((name, path)) }
        }
        folders.sort_by_key(|(n, _)| by_name(n));
        files.sort_by_key(|(n, _)| by_name(n));
        for (name, path) in folders {
            let key = format!("df:{}", self.relative(&path));
            let expanded = self.open.contains(&key);
            rows.push(self.row(NodeKind::Folder, &name, Some(path.clone()), depth, true, expanded, key));
            if expanded {
                self.on_disk(&path, None, depth + 1, rows);
            }
        }
        let paths: Vec<PathBuf> = files.iter().map(|(_, p)| p.clone()).collect();
        let parents = nesting::parents(&paths, |_| None, true);
        self.files_with_dependents(&files, &parents, depth, rows);
    }

    fn files_with_dependents(
        &self,
        files: &[(String, PathBuf)],
        parents: &HashMap<PathBuf, PathBuf>,
        depth: usize,
        rows: &mut Vec<TreeRow>,
    ) {
        for (name, path) in files.iter().filter(|(_, p)| !parents.contains_key(p)) {
            let nested: Vec<&(String, PathBuf)> = files.iter().filter(|(_, p)| parents.get(p) == Some(path)).collect();
            let key = format!("n:{}", self.relative(path));
            let expanded = self.open.contains(&key);
            rows.push(self.row(NodeKind::File, name, Some(path.clone()), depth, !nested.is_empty(), expanded, key));
            if expanded && !nested.is_empty() {
                for (name, path) in nested {
                    rows.push(self.row(
                        NodeKind::File,
                        name,
                        Some(path.clone()),
                        depth + 1,
                        false,
                        false,
                        String::new(),
                    ));
                }
            }
        }
    }
}
