use crate::test_finder::CodeTests;
use rsharp_core::state::StateDocument;
use rsharp_core::testing::{TestItem, TestOutcome, TestState};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GroupLevel {
    TargetFramework,
    Project,
    FileSystem,
    Namespace,
    Category,
    Duration,
}

impl GroupLevel {
    pub const ALL: [Self; 6] =
        [Self::TargetFramework, Self::Project, Self::FileSystem, Self::Namespace, Self::Category, Self::Duration];
    pub fn title(self) -> &'static str {
        match self {
            Self::TargetFramework => "Target Framework",
            Self::Project => "Project",
            Self::FileSystem => "File System",
            Self::Namespace => "Namespace",
            Self::Category => "Category",
            Self::Duration => "Duration",
        }
    }
}

pub const PRESETS: &[(&str, &[GroupLevel])] = &[
    ("Test Hierarchy", &[]),
    ("Project Structure", &[GroupLevel::Project, GroupLevel::FileSystem]),
    ("Projects", &[GroupLevel::Project]),
    ("Namespaces", &[GroupLevel::Namespace]),
    ("Projects and Namespaces", &[GroupLevel::Project, GroupLevel::Namespace]),
    ("Categories", &[GroupLevel::Category]),
    ("Project and Categories", &[GroupLevel::Project, GroupLevel::Category]),
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UnitTestsState {
    #[serde(deserialize_with = "known_values")]
    pub grouping: Vec<GroupLevel>,
    #[serde(deserialize_with = "known_values")]
    pub outcomes: Vec<TestState>,
}
fn known_values<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(
    deserializer: D,
) -> Result<Vec<T>, D::Error> {
    let values = Option::<Vec<serde_json::Value>>::deserialize(deserializer)?.unwrap_or_default();
    Ok(values.into_iter().filter_map(|value| serde_json::from_value(value).ok()).collect())
}

impl Default for UnitTestsState {
    fn default() -> Self {
        Self { grouping: vec![GroupLevel::Project, GroupLevel::Namespace], outcomes: vec![] }
    }
}
impl StateDocument for UnitTestsState {
    const FILE: &'static str = "unitTests.json";
}
impl UnitTestsState {
    pub fn clamped(mut self) -> Self {
        self.grouping.sort();
        self.grouping.dedup();
        self.outcomes.sort_by_key(|state| match state {
            TestState::NotRun => 0,
            TestState::Running => 1,
            TestState::Passed => 2,
            TestState::Failed => 3,
            TestState::Skipped => 4,
        });
        let mut seen = Vec::new();
        self.outcomes.retain(|state| {
            if seen.contains(state) {
                false
            } else {
                seen.push(*state);
                true
            }
        });
        self
    }
}

pub fn categories(traits: &[(String, String)]) -> Vec<String> {
    let mut found = Vec::new();
    for (name, value) in traits {
        let category = name.eq_ignore_ascii_case("Category") || name.eq_ignore_ascii_case("TestCategory");
        for value in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let label = if category { value.to_string() } else { format!("{} [{value}]", name.trim()) };
            if !found.contains(&label) {
                found.push(label);
            }
        }
    }
    found
}
pub fn duration_grade(duration: Option<Duration>) -> (&'static str, usize) {
    let Some(duration) = duration else { return ("Not Run", 5) };
    for (rank, (under, name)) in
        [(100, "Fast (<100 ms)"), (1000, "Medium (>100 ms)"), (15000, "Slow (>1 sec)"), (30000, "Slower (>15 sec)")]
            .into_iter()
            .enumerate()
    {
        if duration.as_millis() < under {
            return (name, rank);
        }
    }
    ("Slowest (>30 sec)", 4)
}

pub fn state_label(state: TestState) -> &'static str {
    match state {
        TestState::NotRun => "Not Run",
        TestState::Running => "Running",
        TestState::Passed => "Passed",
        TestState::Failed => "Failed",
        TestState::Skipped => "Skipped",
    }
}
fn combine(states: impl Iterator<Item = TestState>) -> TestState {
    states
        .min_by_key(|s| match s {
            TestState::Failed => 0,
            TestState::Running => 1,
            TestState::NotRun => 2,
            TestState::Passed => 3,
            TestState::Skipped => 4,
        })
        .unwrap_or(TestState::NotRun)
}
fn identity(test: &TestItem) -> String {
    format!(
        "{}|{}|{}|{}",
        test.project.display(),
        test.method().replace('+', "."),
        test.framework.as_deref().unwrap_or(""),
        test.row.as_deref().unwrap_or("")
    )
}
fn method_identity(test: &TestItem) -> String {
    format!("{}|{}", test.project.display(), test.method().replace('+', "."))
}

/// One visible row; groups carry the unique tests below them for Run Selected.
#[derive(Clone, Debug)]
pub struct TestRow {
    pub key: String,
    pub label: String,
    pub depth: usize,
    pub state: TestState,
    pub duration: Duration,
    pub expandable: bool,
    pub expanded: bool,
    pub tests: Vec<TestItem>,
    pub results: Vec<TestOutcome>,
    pub path: Option<PathBuf>,
    pub line: usize,
}
#[derive(Default)]
struct SourceIndex {
    methods: BTreeMap<String, usize>,
    test_names_by_file: BTreeMap<PathBuf, BTreeSet<String>>,
}
impl SourceIndex {
    fn new(code: &CodeTests) -> Self {
        let mut index = Self::default();
        for (at, test) in code.tests.iter().enumerate() {
            index.methods.insert(normalized_method(test.method()).into_owned(), at);
            if let Some(path) = &test.path {
                index
                    .test_names_by_file
                    .entry(path.clone())
                    .or_default()
                    .insert(test.method().rsplit('.').next().unwrap_or(test.method()).to_string());
            }
        }
        index
    }
}
fn normalized_method(method: &str) -> Cow<'_, str> {
    if method.contains('+') { Cow::Owned(method.replace('+', ".")) } else { Cow::Borrowed(method) }
}
#[derive(Default)]
pub struct TestModel {
    code: BTreeMap<PathBuf, CodeTests>,
    source_index: BTreeMap<PathBuf, SourceIndex>,
    runtime: BTreeMap<String, TestItem>,
    results: BTreeMap<String, TestOutcome>,
    running: BTreeSet<String>,
}
impl TestModel {
    pub fn set_code(&mut self, code: BTreeMap<PathBuf, CodeTests>) -> bool {
        if self.code == code {
            return false;
        }
        // A source scan owns files it saw before; their deletion must also remove an old assembly's cases.
        let removed: Vec<_> = self
            .runtime
            .iter()
            .filter(|(_, test)| {
                self.code.get(&test.project).is_some_and(|previous| {
                    code.get(&test.project).is_none_or(|current| {
                        test.path.as_ref().is_some_and(|path| {
                            previous.methods_by_file.contains_key(path) && !current.methods_by_file.contains_key(path)
                        })
                    })
                })
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in removed {
            self.runtime.remove(&key);
            self.results.remove(&key);
            self.running.remove(&key);
        }
        self.source_index = code.iter().map(|(path, code)| (path.clone(), SourceIndex::new(code))).collect();
        self.code = code;
        self.prune();
        true
    }
    pub fn discovered(&mut self, tests: Vec<TestItem>) {
        for test in tests {
            self.runtime.insert(identity(&test), test);
        }
        self.prune();
    }
    pub fn clear_discovered(&mut self) {
        self.runtime.clear();
    }
    pub fn clear_discovered_in(&mut self, projects: &[PathBuf]) {
        self.runtime.retain(|_, test| !projects.contains(&test.project));
    }
    pub fn outcomes(&mut self, outcomes: Vec<TestOutcome>) {
        for outcome in outcomes {
            if self.stale(&outcome.test) {
                continue;
            }
            let key = identity(&outcome.test);
            self.running.remove(&key);
            self.runtime.entry(key.clone()).or_insert_with(|| outcome.test.clone());
            self.results.insert(key, outcome);
        }
    }
    pub fn begin(&mut self, tests: &[TestItem]) {
        for test in tests {
            let key = identity(test);
            self.results.remove(&key);
            self.running.insert(key);
        }
    }
    pub fn end(&mut self) {
        self.running.clear();
    }
    fn stale(&self, test: &TestItem) -> bool {
        let Some(project) = self.code.get(&test.project) else { return !self.code.is_empty() };
        test.path.as_ref().and_then(|p| project.methods_by_file.get(p)).is_some_and(|methods| {
            let short = test.method().rsplit('.').next().unwrap_or(test.method());
            let index = self.source_index.get(&test.project).expect("a source project has its method index");
            !methods.contains(short)
                || (!index.methods.contains_key(normalized_method(test.method()).as_ref())
                    && test
                        .path
                        .as_ref()
                        .and_then(|path| index.test_names_by_file.get(path))
                        .is_some_and(|names| names.contains(short)))
        })
    }
    fn prune(&mut self) {
        let gone: Vec<_> = self.runtime.iter().filter(|(_, t)| self.stale(t)).map(|(k, _)| k.clone()).collect();
        for key in gone {
            self.runtime.remove(&key);
            self.results.remove(&key);
            self.running.remove(&key);
        }
    }
    fn source_test(&self, test: &TestItem) -> Option<&TestItem> {
        let at = self.source_index.get(&test.project)?.methods.get(normalized_method(test.method()).as_ref())?;
        self.code.get(&test.project)?.tests.get(*at)
    }
    pub fn tests(&self) -> Vec<TestItem> {
        let mut tests: Vec<_> = self
            .runtime
            .values()
            .cloned()
            .map(|mut test| {
                if let Some(code) = self.source_test(&test) {
                    test.path = code.path.clone();
                    test.line = code.line;
                    if test.traits.is_empty() {
                        test.traits = code.traits.clone();
                    }
                }
                test
            })
            .collect();
        let runtime_methods: BTreeSet<_> = tests.iter().map(method_identity).collect();
        tests.extend(
            self.code
                .values()
                .flat_map(|code| &code.tests)
                .filter(|test| !runtime_methods.contains(&method_identity(test)))
                .cloned(),
        );
        tests.sort_by_key(identity);
        tests
    }
    pub fn failed_tests(&self) -> Vec<TestItem> {
        self.tests()
            .into_iter()
            .filter(|t| self.results.get(&identity(t)).is_some_and(|r| r.state == TestState::Failed))
            .collect()
    }
    pub fn counts(&self) -> (usize, usize) {
        let tests = self.tests();
        let failed = tests
            .iter()
            .filter(|test| self.results.get(&identity(test)).is_some_and(|result| result.state == TestState::Failed))
            .count();
        (tests.len(), failed)
    }
    pub fn rows(&self, state: &UnitTestsState, query: &str, collapsed: &BTreeSet<String>) -> Vec<TestRow> {
        let all = self.tests();

        let frameworks = all.iter().fold(BTreeMap::<PathBuf, BTreeSet<String>>::new(), |mut map, t| {
            if let Some(f) = &t.framework {
                map.entry(t.project.clone()).or_default().insert(f.clone());
            }
            map
        });
        let mut method_durations = BTreeMap::<String, Duration>::new();
        for result in self.results.values() {
            let method =
                format!("{}|{}", method_identity(&result.test), result.test.framework.as_deref().unwrap_or(""));
            *method_durations.entry(method).or_default() += result.duration;
        }
        let query = query.trim().to_lowercase();
        let mut roots: BTreeMap<(usize, String), Node> = BTreeMap::new();
        for test in all {
            let result = self.results.get(&identity(&test)).cloned();
            let status = if self.running.contains(&identity(&test)) {
                TestState::Running
            } else {
                result.as_ref().map_or(TestState::NotRun, |r| r.state)
            };
            if !state.outcomes.is_empty() && !state.outcomes.contains(&status) {
                continue;
            }
            if !query.is_empty()
                && !format!(
                    "{} {} {} {}",
                    test.fully_qualified_name,
                    test.display_name,
                    test.project.display(),
                    categories(&test.traits).join(" ")
                )
                .to_lowercase()
                .contains(&query)
            {
                continue;
            }
            let several = frameworks.get(&test.project).is_some_and(|f| f.len() > 1);
            let mut paths: Vec<Vec<Segment>> = vec![vec![]];
            let method = self.source_test(&test).map_or_else(|| test.method(), TestItem::method);
            let (owner, name) = method.rsplit_once('.').unwrap_or(("", method));
            let (space, class) = owner.rsplit_once('.').unwrap_or(("", owner));
            for level in &state.grouping {
                let segments = match level {
                    GroupLevel::TargetFramework => {
                        test.framework.as_ref().map(|f| vec![Segment::new(0, f, f)]).unwrap_or_default()
                    }
                    GroupLevel::Project => {
                        let project = test.project.to_string_lossy();
                        let mut s = vec![Segment::new(
                            1,
                            &project,
                            &test.project.file_stem().unwrap_or_default().to_string_lossy(),
                        )];
                        if several
                            && !state.grouping.contains(&GroupLevel::TargetFramework)
                            && let Some(f) = &test.framework
                        {
                            s.push(Segment::new(0, f, f));
                        }
                        s
                    }
                    GroupLevel::FileSystem => test
                        .path
                        .as_ref()
                        .and_then(|p| p.parent())
                        .and_then(|p| p.strip_prefix(test.project.parent()?).ok())
                        .map(|p| {
                            p.components()
                                .map(|c| {
                                    let text = c.as_os_str().to_string_lossy();
                                    Segment::new(2, &text, &text)
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    GroupLevel::Namespace => {
                        if space.is_empty() {
                            vec![]
                        } else {
                            vec![Segment::new(3, space, space)]
                        }
                    }
                    GroupLevel::Category => {
                        let labels = categories(&test.traits);
                        let labels = if labels.is_empty() { vec!["<Uncategorized>".into()] } else { labels };
                        paths = paths
                            .into_iter()
                            .flat_map(|p| {
                                labels.iter().map(move |label| {
                                    let mut p = p.clone();
                                    p.push(Segment::new(4, label, label));
                                    p
                                })
                            })
                            .collect();
                        vec![]
                    }
                    GroupLevel::Duration => {
                        let method = format!("{}|{}", method_identity(&test), test.framework.as_deref().unwrap_or(""));
                        let (label, rank) = duration_grade(method_durations.get(&method).copied());
                        vec![Segment { rank: rank + 10, id: label.into(), label: label.into() }]
                    }
                };
                for path in &mut paths {
                    path.extend(segments.clone());
                }
            }
            let class_label = if state.grouping.contains(&GroupLevel::Namespace) { class } else { owner };
            let class_label = if several
                && !state.grouping.contains(&GroupLevel::Project)
                && !state.grouping.contains(&GroupLevel::TargetFramework)
            {
                format!("{class_label} ({})", test.framework.as_deref().unwrap_or(""))
            } else {
                class_label.into()
            };
            for mut path in paths {
                if !class_label.is_empty() {
                    path.push(Segment::new(
                        20,
                        &format!("{}|{owner}|{}", test.project.display(), test.framework.as_deref().unwrap_or("")),
                        &class_label,
                    ));
                }
                path.push(Segment::new(
                    21,
                    &format!("{}|{method}|{}", test.project.display(), test.framework.as_deref().unwrap_or("")),
                    name,
                ));
                if let Some(row) = &test.row {
                    path.push(Segment::new(22, row, row));
                }
                insert(&mut roots, &path, &test, result.as_ref(), status);
            }
        }
        let mut rows = Vec::new();
        for node in roots.values() {
            node.flatten("", 0, collapsed, &mut rows);
        }
        rows
    }
}
#[derive(Clone)]
struct Segment {
    rank: usize,
    id: String,
    label: String,
}
impl Segment {
    fn new(rank: usize, id: &str, label: &str) -> Self {
        Self { rank, id: id.into(), label: label.into() }
    }
}
struct Node {
    segment: Segment,
    tests: BTreeMap<String, TestItem>,
    results: BTreeMap<String, TestOutcome>,
    states: BTreeMap<String, TestState>,
    children: BTreeMap<(usize, String), Node>,
}
fn insert(
    nodes: &mut BTreeMap<(usize, String), Node>,
    path: &[Segment],
    test: &TestItem,
    result: Option<&TestOutcome>,
    state: TestState,
) {
    let Some((segment, rest)) = path.split_first() else { return };
    let node = nodes.entry((segment.rank, segment.id.clone())).or_insert_with(|| Node {
        segment: segment.clone(),
        tests: BTreeMap::new(),
        results: BTreeMap::new(),
        states: BTreeMap::new(),
        children: BTreeMap::new(),
    });
    let id = identity(test);
    node.tests.insert(id.clone(), test.clone());
    node.states.insert(id.clone(), state);
    if let Some(result) = result {
        node.results.insert(id, result.clone());
    }
    insert(&mut node.children, rest, test, result, state);
}
impl Node {
    fn flatten(&self, parent: &str, depth: usize, collapsed: &BTreeSet<String>, out: &mut Vec<TestRow>) {
        let key = format!("{parent}/{}:{}", self.segment.rank, self.segment.id);
        let expanded = !collapsed.contains(&key);
        let located = self.tests.values().find(|t| t.path.is_some());
        out.push(TestRow {
            key: key.clone(),
            label: self.segment.label.clone(),
            depth,
            state: combine(self.states.values().copied()),
            duration: self.results.values().map(|r| r.duration).sum(),
            expandable: !self.children.is_empty(),
            expanded,
            tests: self.tests.values().cloned().collect(),
            results: self.results.values().cloned().collect(),
            path: located.and_then(|t| t.path.clone()),
            line: located.map_or(1, |t| t.line),
        });
        if expanded {
            for child in self.children.values() {
                child.flatten(&key, depth + 1, collapsed, out);
            }
        }
    }
}
