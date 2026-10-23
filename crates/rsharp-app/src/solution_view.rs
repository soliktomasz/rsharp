//! The Solution view's tree, as Rider's: the solution's folders, the files they list and its projects; each project's
//! Dependencies and its files as MSBuild evaluates them (`ProjectFiles`), a file's dependents nested under it. Only
//! open nodes are walked, so a project nobody opened costs nothing; what is open is kept by each node's key.

use rsharp_core::nesting;
use rsharp_core::project::{EvaluatedProject, PackageDependency, ProjectFiles};
use rsharp_core::solution::{Project, Solution, SolutionEntry};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

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
    /// What keeps it open, and selected, across a re-evaluation and a restart: unique in the tree, paths relative to
    /// the folder.
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

/// One level of a project's files as shown, made once and kept until it is opened again or its project is evaluated
/// again: its folders (name, path, place in the project), then its files, each with those nested under it.
#[derive(Default)]
struct Level {
    folders: Vec<(String, PathBuf, String)>,
    files: Vec<(String, PathBuf, Vec<(String, PathBuf)>)>,
}

pub struct SolutionTree {
    solution: Solution,
    root: PathBuf,
    evaluated: HashMap<PathBuf, EvaluatedProject>,
    files: RefCell<HashMap<PathBuf, Rc<ProjectFiles>>>,
    levels: RefCell<HashMap<String, Rc<Level>>>,
    open: HashSet<String>,
    rows: Vec<TreeRow>,
    /// Why no project could be evaluated, said on the solution's row.
    problem: Option<String>,
}

impl SolutionTree {
    pub fn new(solution: Solution, evaluated: HashMap<PathBuf, EvaluatedProject>, open: HashSet<String>) -> Self {
        let root = solution.path.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut tree = Self {
            solution,
            root,
            evaluated,
            files: RefCell::default(),
            levels: RefCell::default(),
            open,
            rows: Vec::new(),
            problem: None,
        };
        tree.open.insert("solution".into());
        tree.rebuild();
        tree
    }

    pub fn solution(&self) -> &Solution {
        &self.solution
    }

    /// A project's evaluation, its rows following and what was open kept open.
    pub fn set_evaluated(&mut self, project: EvaluatedProject) {
        self.files.borrow_mut().remove(&project.path);
        let prefix = format!("pf:{}|", self.relative(&project.path));
        self.levels.borrow_mut().retain(|key, _| !key.starts_with(&prefix));
        self.evaluated.insert(project.path.clone(), project);
        // The Host answered, so whatever stopped it before is over.
        self.problem = None;
        self.rebuild();
    }

    /// Why the projects could not be evaluated, or none once they can.
    pub fn set_problem(&mut self, problem: Option<String>) {
        self.problem = problem;
        self.rebuild();
    }

    pub fn evaluated(&self, project: &Path) -> Option<&EvaluatedProject> {
        self.evaluated.get(project)
    }

    /// Every evaluation it holds, for the folder's cache.
    pub fn all_evaluated(&self) -> Vec<EvaluatedProject> {
        self.evaluated.values().cloned().collect()
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
            // What a folder holds on disk is read again as it opens, as the folder view's listings are.
            let mut levels = self.levels.borrow_mut();
            levels.remove(&key);
            if let Some(path) = row.path.as_ref().filter(|_| row.kind == NodeKind::Project) {
                levels.retain(|k, _| !k.starts_with(&format!("pf:{}|", self.relative(path))));
                levels.remove(&format!("disk:{}", path.parent().unwrap_or(path).display()));
            } else if let Some(path) = &row.path {
                levels.remove(&format!("disk:{}", path.display()));
            }
            drop(levels);
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
        let mut root =
            self.row(NodeKind::Solution, &self.solution.name, Some(self.solution.path.clone()), 0, true, expanded, key);
        if let Some(problem) = &self.problem {
            root.detail = Some("projects not evaluated".into());
            root.tip = Some(problem.clone());
        }
        rows.push(root);
        if expanded {
            self.entries(&self.solution.entries, 1, &mut rows);
        }
        self.rows = rows;
    }

    /// A level of the solution as Rider's Solution view sorts it, whatever order the file declares: its folders, then
    /// its projects, then the files a folder lists, each by name.
    fn entries(&self, entries: &[SolutionEntry], depth: usize, rows: &mut Vec<TreeRow>) {
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
                    let key = format!("si:{}", self.relative(path));
                    let mut row = self.row(NodeKind::File, name, Some(path.clone()), depth, false, false, key);
                    row.dimmed = !exists;
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

    fn project(&self, project: &Project, depth: usize, rows: &mut Vec<TreeRow>) {
        let base = self.relative(&project.path);
        let key = format!("p:{base}");
        let expanded = self.open.contains(&key);
        let mut row = self.row(
            NodeKind::Project,
            &project.name,
            Some(project.path.clone()),
            depth,
            project.exists,
            expanded,
            key,
        );
        let evaluated = self.evaluated.get(&project.path);
        if !project.exists {
            row.dimmed = true;
            row.detail = Some("not found".into());
        } else if let Some(problem) = evaluated.and_then(|e| e.problem.as_ref()) {
            row.detail = Some("load failed".into());
            row.tip = Some(problem.clone());
        } else if let Some(evaluated) = evaluated.filter(|e| !e.target_frameworks.is_empty()) {
            row.detail = Some(evaluated.target_frameworks.join(", "));
        }
        let expanded = row.expanded;
        rows.push(row);
        if !expanded {
            return;
        }
        let sound = evaluated.filter(|e| e.problem.is_none());
        self.dependencies(&base, sound, depth + 1, rows);
        match sound {
            Some(evaluated) => {
                let files = self
                    .files
                    .borrow_mut()
                    .entry(project.path.clone())
                    .or_insert_with(|| Rc::new(ProjectFiles::new(evaluated)))
                    .clone();
                self.held(&base, &files, "", depth + 1, rows);
            }
            None => {
                let folder = project.path.parent().map(Path::to_path_buf).unwrap_or_default();
                self.on_disk(&base, &folder, Some(&project.path), depth + 1, rows);
            }
        }
    }

    /// A project's dependencies as MSBuild evaluates them: for one framework, it with the projects and packages it
    /// references; for a multi-targeted project, each framework with its own, as Rider shows them.
    fn dependencies(&self, base: &str, evaluated: Option<&EvaluatedProject>, depth: usize, rows: &mut Vec<TreeRow>) {
        let key = format!("d:{base}");
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
                rows.push(self.row(NodeKind::Framework, &name, None, depth + 1, true, expanded, key.clone()));
                if expanded {
                    self.referenced(&key, &framework.projects, &framework.packages, depth + 2, rows);
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
        self.referenced(&key, projects, packages, depth + 1, rows);
    }

    /// The projects and packages referenced; packages the SDK adds of itself are left out, as Rider leaves them.
    fn referenced(
        &self,
        under: &str,
        projects: &[PathBuf],
        packages: &[PackageDependency],
        depth: usize,
        rows: &mut Vec<TreeRow>,
    ) {
        for project in projects {
            let name = project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let key = format!("ref:{under}|{}", self.relative(project));
            rows.push(self.row(NodeKind::ProjectReference, &name, Some(project.clone()), depth, false, false, key));
        }
        for package in packages.iter().filter(|p| !p.implicit) {
            let key = format!("pkg:{under}|{}", package.name);
            let mut row = self.row(NodeKind::Package, &package.name, None, depth, false, false, key);
            row.detail = package.version.clone();
            rows.push(row);
        }
    }

    /// The level once made, else made now and kept.
    fn level(&self, key: String, make: impl FnOnce() -> Level) -> Rc<Level> {
        if let Some(level) = self.levels.borrow().get(&key) {
            return level.clone();
        }
        let level = Rc::new(make());
        self.levels.borrow_mut().insert(key, level.clone());
        level
    }

    /// What an evaluated project holds at one place in it: its folders, then its files, a file's dependents under it.
    fn held(&self, base: &str, files: &ProjectFiles, relative: &str, depth: usize, rows: &mut Vec<TreeRow>) {
        let level = self.level(format!("pf:{base}|{relative}"), || {
            let folders = files
                .folders_in(relative)
                .into_iter()
                .map(|name| {
                    let inner = if relative.is_empty() { name.clone() } else { format!("{relative}/{name}") };
                    (name, files.path_of(&inner), inner)
                })
                .collect();
            let held = files.files_in(relative);
            let paths: Vec<PathBuf> = held.iter().map(|(_, p)| p.clone()).collect();
            let parents = nesting::parents(&paths, |f| files.dependent_upon(f), true);
            Level { folders, files: with_dependents(held, &parents) }
        });
        for (name, path, inner) in &level.folders {
            let key = format!("pf:{base}|{inner}");
            let expanded = self.open.contains(&key);
            rows.push(self.row(NodeKind::Folder, name, Some(path.clone()), depth, true, expanded, key));
            if expanded {
                self.held(base, files, inner, depth + 1, rows);
            }
        }
        self.file_rows(base, &level, depth, rows);
    }

    /// The files in a folder on disk: a project's until MSBuild has evaluated it, or when it could not.
    fn on_disk(&self, base: &str, folder: &Path, project_file: Option<&Path>, depth: usize, rows: &mut Vec<TreeRow>) {
        let level = self.level(format!("disk:{}", folder.display()), || {
            let mut folders = Vec::new();
            let mut files = Vec::new();
            for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
                let (path, name) = (entry.path(), entry.file_name().to_string_lossy().into_owned());
                let is_folder = path.is_dir();
                if hidden_on_disk(&name, is_folder) || Some(path.as_path()) == project_file {
                    continue;
                }
                if is_folder { folders.push((name, path, String::new())) } else { files.push((name, path)) }
            }
            folders.sort_by_key(|(n, _, _)| by_name(n));
            files.sort_by_key(|(n, _)| by_name(n));
            let paths: Vec<PathBuf> = files.iter().map(|(_, p)| p.clone()).collect();
            let parents = nesting::parents(&paths, |_| None, true);
            Level { folders, files: with_dependents(files, &parents) }
        });
        for (name, path, _) in &level.folders {
            let key = format!("df:{}", self.relative(path));
            let expanded = self.open.contains(&key);
            rows.push(self.row(NodeKind::Folder, name, Some(path.clone()), depth, true, expanded, key));
            if expanded {
                self.on_disk(base, path, None, depth + 1, rows);
            }
        }
        self.file_rows(base, &level, depth, rows);
    }

    /// A level's files, each opening to those nested under it; keyed by their project too, as one file may be linked
    /// into several.
    fn file_rows(&self, base: &str, level: &Level, depth: usize, rows: &mut Vec<TreeRow>) {
        for (name, path, nested) in &level.files {
            let key = format!("n:{base}|{}", self.relative(path));
            let expanded = self.open.contains(&key);
            rows.push(self.row(NodeKind::File, name, Some(path.clone()), depth, !nested.is_empty(), expanded, key));
            if expanded {
                for (name, path) in nested {
                    let key = format!("nc:{base}|{}", self.relative(path));
                    rows.push(self.row(NodeKind::File, name, Some(path.clone()), depth + 1, false, false, key));
                }
            }
        }
    }
}

/// Files with those nested under each, in one pass: the nested ones leave the level and go under their parent.
fn with_dependents(
    files: Vec<(String, PathBuf)>,
    parents: &HashMap<PathBuf, PathBuf>,
) -> Vec<(String, PathBuf, Vec<(String, PathBuf)>)> {
    let mut nested: HashMap<&PathBuf, Vec<(String, PathBuf)>> = HashMap::new();
    let mut top = Vec::new();
    for (name, path) in files {
        match parents.get(&path) {
            Some(parent) => nested.entry(parent).or_default().push((name, path)),
            None => top.push((name, path)),
        }
    }
    top.into_iter()
        .map(|(name, path)| {
            let under = nested.remove(&path).unwrap_or_default();
            (name, path, under)
        })
        .collect()
}
