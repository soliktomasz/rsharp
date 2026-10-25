//! Source discovery on a worker; open files supply their shared syntax reads.

use rsharp_core::{
    editorconfig::EditorConfig,
    encodings,
    project::{EvaluatedProject, Stamp},
    testing::TestItem,
};
use rsharp_languages::testing::FileTests;
use rsharp_text::Rope;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeTests {
    pub tests: Vec<TestItem>,
    pub methods_by_file: BTreeMap<PathBuf, BTreeSet<String>>,
}

#[derive(Default)]
pub struct TestFinder {
    on_disk: HashMap<PathBuf, (Stamp, FileTests)>,
    listed: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
}

/// Framework references identify test projects even without an adapter or an explicit test flag.
pub fn holds_tests(project: &EvaluatedProject) -> bool {
    const FRAMEWORKS: &[&str] =
        &["xunit", "xunit.core", "xunit.v3", "xunit.v3.core", "NUnit", "MSTest", "MSTest.TestFramework"];
    project.path.is_file()
        && (project.is_test_project
            || project
                .dependencies
                .iter()
                .flat_map(|d| &d.packages)
                .any(|p| FRAMEWORKS.iter().any(|name| p.name.eq_ignore_ascii_case(name))))
}

impl TestFinder {
    /// Runs on a worker. The caller supplies open buffers' syntax reads, including unsaved new files.
    pub fn scan(
        &mut self,
        projects: &[EvaluatedProject],
        open: &HashMap<PathBuf, FileTests>,
    ) -> BTreeMap<PathBuf, CodeTests> {
        let project_folders: BTreeSet<PathBuf> =
            projects.iter().filter_map(|p| p.path.parent().map(Path::to_path_buf)).collect();
        let mut current = BTreeSet::new();
        let mut listed = BTreeMap::new();
        for project in projects.iter().filter(|p| holds_tests(p)) {
            let Some(folder) = project.path.parent() else { continue };
            let mut files = BTreeSet::new();
            list_files(folder, &project_folders, &mut files);
            for item in project.items.iter().filter(|i| i.r#type.eq_ignore_ascii_case("Compile")) {
                let path = if item.path.is_absolute() { item.path.clone() } else { folder.join(&item.path) };
                if is_csharp(&path) {
                    files.insert(rsharp_core::paths::canonical(&path));
                }
            }
            for path in &files {
                current.insert(path.clone());
                if !open.contains_key(path) {
                    self.read_file(path);
                }
            }
            listed.insert(project.path.clone(), files);
        }
        self.listed = listed;
        self.on_disk.retain(|path, _| current.contains(path));
        self.update_open(projects, open)
    }

    /// Refreshes typed declarations from the last scan, with no disk reads, stamps or directory walks.
    pub fn update_open(
        &self,
        projects: &[EvaluatedProject],
        open: &HashMap<PathBuf, FileTests>,
    ) -> BTreeMap<PathBuf, CodeTests> {
        let mut found = BTreeMap::new();
        for project in projects {
            let Some(listed) = self.listed.get(&project.path) else { continue };
            let mut files = listed.clone();
            for path in open.keys().filter(|p| is_csharp(p)) {
                if owner(projects, path).is_some_and(|p| p.path == project.path) {
                    files.insert(path.clone());
                }
            }
            let mut code = CodeTests::default();
            for path in files {
                let tests = open.get(&path).or_else(|| self.on_disk.get(&path).map(|(_, tests)| tests));
                let Some(tests) = tests else { continue };
                code.methods_by_file.insert(path.clone(), tests.methods.clone());
                for test in tests.tests.iter().filter(|t| !t.abstract_type) {
                    code.tests.push(TestItem {
                        project: project.path.clone(),
                        fully_qualified_name: format!("{}.{}", test.type_name, test.method),
                        display_name: test.method.clone(),
                        path: Some(path.clone()),
                        line: test.line,
                        traits: test.traits.clone(),
                        ..Default::default()
                    });
                }
            }
            found.insert(project.path.clone(), code);
        }
        found
    }

    fn read_file(&mut self, path: &Path) -> Option<&FileTests> {
        let stamp = Stamp::file(path);
        if stamp.length < 0 {
            self.on_disk.remove(path);
            return None;
        }
        if self.on_disk.get(path).is_none_or(|(previous, _)| *previous != stamp) {
            let bytes = std::fs::read(path).ok()?;
            let config = EditorConfig::for_file(path);
            let charset = config.get("charset").and_then(encodings::Charset::from_editorconfig);
            let text = Rope::from_str(&encodings::decode(&bytes, charset).0);
            let tree = rsharp_languages::parse(&text, None);
            self.on_disk.insert(path.into(), (stamp, rsharp_languages::testing::read(&tree, &text)));
        }
        self.on_disk.get(path).map(|(_, tests)| tests)
    }
}

fn is_csharp(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("cs"))
}

fn owner<'a>(projects: &'a [EvaluatedProject], path: &Path) -> Option<&'a EvaluatedProject> {
    projects
        .iter()
        .filter(|p| p.path.parent().is_some_and(|folder| path.starts_with(folder)))
        .max_by_key(|p| p.path.components().count())
}

fn list_files(folder: &Path, project_folders: &BTreeSet<PathBuf>, files: &mut BTreeSet<PathBuf>) {
    let mut folders = vec![folder.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(folder) else { continue };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_file() && is_csharp(&path) {
                files.insert(path);
            } else if kind.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with('.')
                    && !["bin", "obj", "node_modules"].iter().any(|ignored| name.eq_ignore_ascii_case(ignored))
                    && !project_folders.contains(&path)
                {
                    folders.push(path);
                }
            }
        }
    }
}
