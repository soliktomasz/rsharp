//! Source discovery and test runs for one workspace; blocking work and protocols stay off the UI thread.

use super::*;
use crate::test_finder::{TestFinder, holds_tests};
use crate::test_view::{TestAction, TestView};
use futures::{FutureExt, SinkExt};
use rsharp_core::project::EvaluatedProject;
use rsharp_core::testing::{RunSummary, TestEvent, TestItem, TestRunners, TestState, resolve_assemblies};
use rsharp_languages::testing::FileTests;
use std::collections::HashMap;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

#[derive(Default)]
struct Sources {
    finder: TestFinder,
    open: HashMap<PathBuf, (gpui::EntityId, u64, FileTests)>,
    evaluated: HashMap<PathBuf, EvaluatedProject>,
}

pub(super) struct Tests {
    pub view: Entity<TestView>,
    sources: Arc<Mutex<Sources>>,
    scan_generation: u64,
    scan: Option<Task<()>>,
    disk_pending: bool,
    pub(super) projects: Arc<Vec<EvaluatedProject>>,
    available: bool,
    cancel: Option<Arc<AtomicBool>>,
    generation: u64,
    task: Option<Task<()>>,
    after_build: Option<Scope>,
}

impl Tests {
    pub fn new(view: Entity<TestView>) -> Self {
        Self {
            view,
            sources: Default::default(),
            scan_generation: 0,
            scan: None,
            disk_pending: true,
            projects: Default::default(),
            available: false,
            cancel: None,
            generation: 0,
            task: None,
            after_build: None,
        }
    }
}

enum Event {
    Runner(TestEvent),
    Started(Vec<TestItem>),
    Ended(Result<RunSummary, String>),
}

enum Scope {
    All,
    Projects(Vec<PathBuf>),
    Tests(Vec<TestItem>),
}

impl Scope {
    fn includes_project(&self, project: &Path) -> bool {
        match self {
            Self::All => true,
            Self::Projects(projects) => projects.iter().any(|p| p == project),
            Self::Tests(tests) => tests.iter().any(|t| t.project == project),
        }
    }
    fn includes_test(&self, test: &TestItem) -> bool {
        match self {
            Self::Tests(tests) => tests.iter().any(|choice| selects(choice, test)),
            _ => self.includes_project(&test.project),
        }
    }
}

impl MainWindow {
    pub fn test_view(&self) -> &Entity<TestView> {
        &self.tests.view
    }

    pub fn is_testing(&self) -> bool {
        self.tests.cancel.is_some()
    }

    pub(super) fn sync_tests(&mut self, cx: &mut Context<Self>) {
        self.tests.projects = Arc::new(self.run_projects.clone());
        let available = self.tests.projects.iter().any(holds_tests);
        self.tests.available = available;
        self.tests.view.update(cx, |view, cx| view.set_available(available, cx));
        self.refresh_tests(cx);
    }

    /// Rescans changed closed files and reads open files from their shared syntax after typing pauses.
    pub fn refresh_tests(&mut self, cx: &mut Context<Self>) {
        self.schedule_test_scan(true, cx);
    }

    pub(super) fn refresh_open_tests(&mut self, cx: &mut Context<Self>) {
        self.schedule_test_scan(false, cx);
    }

    fn schedule_test_scan(&mut self, disk: bool, cx: &mut Context<Self>) {
        if self.workspace.is_none() {
            return;
        }
        self.tests.disk_pending |= disk;
        self.tests.scan_generation += 1;
        let generation = self.tests.scan_generation;
        self.tests.scan = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(150)).await;
            let request = this
                .update(cx, |m, cx| {
                    m.workspace.as_ref()?;
                    let projects = m.tests.projects.clone();
                    // gpui may retain a closed view in its previous frame; actual tabs define open sources.
                    let mut buffers: Vec<_> = m
                        .area
                        .groups
                        .iter()
                        .flat_map(|g| &g.tabs)
                        .map(|tab| tab.editor.read(cx).buffer.clone())
                        .collect();
                    let me = cx.entity_id();
                    for window in cx.windows() {
                        if window.downcast::<MainWindow>().is_some() {
                            if let Ok(other) = window.read(cx, |other: Entity<MainWindow>, cx| {
                                if other.entity_id() == me {
                                    Vec::new()
                                } else {
                                    other
                                        .read(cx)
                                        .area
                                        .groups
                                        .iter()
                                        .flat_map(|g| &g.tabs)
                                        .map(|tab| tab.editor.read(cx).buffer.clone())
                                        .collect()
                                }
                            }) {
                                buffers.extend(other);
                            }
                        } else if let Some(window) = window.downcast::<crate::editor_window::EditorWindow>()
                            && let Ok(other) = window.read(cx)
                        {
                            buffers.extend(other.buffers(cx));
                        }
                    }
                    let mut seen = HashSet::new();
                    let open: Vec<_> = buffers
                        .into_iter()
                        .filter(|buffer| seen.insert(buffer.entity_id()))
                        .filter_map(|buffer| {
                            let b = buffer.read(cx);
                            if !b.path.extension().is_some_and(|ext| ext == "cs") {
                                return None;
                            }
                            let syntax = (b.tree_version == b.document.version())
                                .then(|| b.tree.clone())
                                .flatten()
                                .map(|tree| (buffer.entity_id(), tree, b.document.text().clone(), b.tree_version));
                            Some((b.path.clone(), syntax))
                        })
                        .collect();
                    let host = m.tree.is_none().then(|| m.host.clone()).flatten();
                    Some((projects, open, m.tests.sources.clone(), std::mem::take(&mut m.tests.disk_pending), host))
                })
                .ok()
                .flatten();
            let Some((projects, open, sources, disk, host)) = request else { return };
            let (code, projects, available) = cx
                .background_executor()
                .spawn(async move {
                    let mut sources = sources.lock().unwrap();
                    let projects = if let Some(host) = host {
                        let mut projects = (*projects).clone();
                        for project in &mut projects {
                            let stale = sources.evaluated.get(&project.path).is_none_or(EvaluatedProject::is_stale);
                            if disk && stale {
                                match host.evaluate(&project.path, None) {
                                    Ok(evaluated) => {
                                        sources.evaluated.insert(project.path.clone(), evaluated);
                                    }
                                    Err(error) => {
                                        log::warn!("Could not evaluate tests in {}: {error}", project.path.display())
                                    }
                                }
                            }
                            if let Some(evaluated) = sources.evaluated.get(&project.path) {
                                *project = evaluated.clone();
                            }
                        }
                        Arc::new(projects)
                    } else {
                        projects
                    };
                    let open: Vec<_> = open
                        .into_iter()
                        .filter(|(path, _)| {
                            projects.iter().any(|project| {
                                project.path.parent().is_some_and(|folder| path.starts_with(folder))
                                    || project.items.iter().any(|item| item.path == *path)
                            })
                        })
                        .collect();
                    sources.open.retain(|path, _| open.iter().any(|(p, _)| p == path));
                    for (path, syntax) in open {
                        if let Some((entity, tree, text, version)) = syntax
                            && sources
                                .open
                                .get(&path)
                                .is_none_or(|(was_entity, was, _)| *was_entity != entity || *was != version)
                        {
                            sources.open.insert(path, (entity, version, rsharp_languages::testing::read(&tree, &text)));
                        }
                    }
                    let open = sources.open.iter().map(|(p, (_, _, tests))| (p.clone(), tests.clone())).collect();
                    let code = if disk {
                        sources.finder.scan(&projects, &open)
                    } else {
                        sources.finder.update_open(&projects, &open)
                    };
                    let available = projects.iter().any(holds_tests);
                    (code, projects, available)
                })
                .await;
            let _ = this.update(cx, |m, cx| {
                if m.tests.scan_generation == generation {
                    m.tests.projects = projects;
                    m.tests.available = available;
                    m.tests.view.update(cx, |view, cx| view.set_available(available, cx));
                    m.tests.view.update(cx, |view, cx| view.set_code(code, false, cx));
                }
            });
        }));
    }

    pub(super) fn test_action(&mut self, action: &TestAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            TestAction::RunAll => self.run_tests(None, cx),
            TestAction::RunSelected => self.run_tests(Some(self.tests.view.read(cx).selected_tests()), cx),
            TestAction::RunFailed => self.run_tests(Some(self.tests.view.read(cx).failed_tests()), cx),
            TestAction::Stop => self.stop_tests(cx),
            TestAction::Navigate { path, line } => self.open_file(path, Some((line.saturating_sub(1), 0)), window, cx),
            TestAction::Changed => {
                if let Some(workspace) = &self.workspace {
                    store(cx).write_for(&workspace.folder, &self.tests.view.read(cx).state());
                }
            }
        }
    }

    /// A selection from code names a method; a runner's selection can name one data row and framework.
    pub fn run_tests(&mut self, chosen: Option<Vec<TestItem>>, cx: &mut Context<Self>) {
        self.start_tests(chosen.map(Scope::Tests).unwrap_or(Scope::All), cx);
    }

    fn start_tests(&mut self, chosen: Scope, cx: &mut Context<Self>) {
        if self.is_testing() {
            return;
        }
        let refusal = if self.is_building() {
            Some("Tests did not start: another build is running")
        } else if self.workspace.is_none() {
            Some("No workspace is open")
        } else if matches!(&chosen, Scope::Tests(tests) if tests.is_empty()) {
            Some("No tests selected")
        } else if !self.tests.available {
            Some("No test projects found")
        } else {
            None
        };
        if let Some(message) = refusal {
            self.test_status(message.into(), cx);
            return;
        }
        let projects: Vec<_> = self
            .tests
            .projects
            .iter()
            .filter(|p| holds_tests(p))
            .filter(|p| chosen.includes_project(&p.path))
            .map(|p| p.path.clone())
            .collect();
        if projects.is_empty() {
            self.test_status("No test projects selected".into(), cx);
            return;
        }
        self.tests.cancel = Some(Arc::new(AtomicBool::new(false)));
        self.tests.generation += 1;
        self.tests.after_build = Some(chosen);
        self.dock.show(Tool::UnitTests);
        self.save_state(cx);
        self.tests.view.update(cx, |view, cx| view.set_running(true, cx));
        self.test_status("Building test projects…".into(), cx);
        self.implicit_build = true;
        self.begin_build(rsharp_core::build::Kind::Build, Some(projects), cx);
        if !self.is_building() {
            self.tests.after_build = None;
            self.finish_tests(
                Err(self
                    .build_outcome
                    .as_ref()
                    .map(|o| o.message.clone())
                    .unwrap_or_else(|| "The test build could not start".into())),
                cx,
            );
        }
    }

    pub(super) fn after_tests_build(&mut self, status: BuildStatus, cx: &mut Context<Self>) {
        let Some(chosen) = self.tests.after_build.take() else { return };
        let Some(cancel) = self.tests.cancel.clone() else { return };
        if cancel.load(Ordering::Acquire) || status != BuildStatus::Succeeded {
            let message = if cancel.load(Ordering::Acquire) || status == BuildStatus::Stopped {
                "Tests stopped".to_string()
            } else {
                "Tests did not start: their build failed".into()
            };
            self.finish_tests(Err(message), cx);
            return;
        }
        let Some(folder) = self.workspace.as_ref().map(|w| w.folder.clone()) else { return };
        let Some(dotnet) = rsharp_core::programs::dotnet() else {
            self.finish_tests(Err("The .NET SDK is not installed".into()), cx);
            return;
        };
        let projects: Vec<_> = self
            .tests
            .projects
            .iter()
            .filter(|p| holds_tests(p))
            .filter(|p| chosen.includes_project(&p.path))
            .cloned()
            .collect();
        let generation = self.tests.generation;
        let paths: Vec<_> = projects.iter().map(|p| p.path.clone()).collect();
        self.tests.view.update(cx, |view, cx| view.clear_discovered_in(&paths, cx));
        self.test_status("Discovering tests…".into(), cx);
        let (mut sender, mut receiver) = mpsc::channel(32);
        std::thread::spawn(move || {
            let result = (|| {
                let assemblies = resolve_assemblies(&dotnet, &folder, &projects, &cancel)?;
                if cancel.load(Ordering::Acquire) {
                    return Err("Tests stopped".into());
                }
                if assemblies.is_empty() {
                    return Err("MSBuild found no test assemblies".into());
                }
                let mut runners = TestRunners::new(dotnet, folder);
                let mut emit = |event| {
                    let _ = futures::executor::block_on(sender.send(Event::Runner(event)));
                };
                let tests = runners.discover(&assemblies, &cancel, &mut emit)?;
                let selected: Vec<_> = tests.into_iter().filter(|test| chosen.includes_test(test)).collect();
                if cancel.load(Ordering::Acquire) {
                    return Err("Tests stopped".into());
                }
                if selected.is_empty() {
                    return Err("No runnable tests found for this selection".into());
                }
                let _ = futures::executor::block_on(sender.send(Event::Started(selected.clone())));
                runners.run(&selected, &cancel, &mut |event| {
                    let _ = futures::executor::block_on(sender.send(Event::Runner(event)));
                })
            })();
            let _ = futures::executor::block_on(sender.send(Event::Ended(result)));
        });
        self.tests.task = Some(cx.spawn(async move |this, cx| {
            while let Some(first) = receiver.next().await {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let mut events = vec![first];
                while events.len() < 64 {
                    let Some(event) = receiver.next().now_or_never().flatten() else { break };
                    events.push(event);
                }
                let ended = events.iter().any(|event| matches!(event, Event::Ended(_)));
                let current = this.update(cx, |m, cx| {
                    if m.tests.generation != generation {
                        return false;
                    }
                    for event in events {
                        match event {
                            Event::Runner(TestEvent::Log(text)) => {
                                m.tests.view.update(cx, |v, cx| v.append_log(text, cx))
                            }
                            Event::Runner(TestEvent::Discovered(tests)) => {
                                m.tests.view.update(cx, |v, cx| v.discovered(tests, cx))
                            }
                            Event::Runner(TestEvent::Outcomes(outcomes)) => {
                                m.tests.view.update(cx, |v, cx| v.outcomes(outcomes, cx))
                            }
                            Event::Started(tests) => {
                                m.tests.view.update(cx, |v, cx| v.begin(&tests, cx));
                                m.test_status(format!("Running {} tests…", tests.len()), cx);
                            }
                            Event::Ended(result) => m.finish_tests(result, cx),
                        }
                    }
                    true
                });
                if ended || !current.unwrap_or(false) {
                    break;
                }
            }
        }));
    }

    fn test_status(&mut self, message: String, cx: &mut Context<Self>) {
        self.said = Some(message.clone().into());
        self.tests.view.update(cx, |view, cx| view.set_status(message, cx));
        cx.notify();
    }

    fn finish_tests(&mut self, result: Result<RunSummary, String>, cx: &mut Context<Self>) {
        let stopped = self.tests.cancel.take().is_some_and(|c| c.load(Ordering::Acquire));
        let message = if stopped {
            "Tests stopped".into()
        } else {
            match result {
                Err(error) => error,
                Ok(summary) => {
                    self.tests.view.update(cx, |view, cx| view.outcomes(summary.outcomes.clone(), cx));
                    if let Some(error) = summary.error {
                        format!("Tests aborted: {error}")
                    } else if summary.canceled {
                        "Tests stopped".into()
                    } else if summary.aborted {
                        "Tests aborted".into()
                    } else {
                        let count = |state| summary.outcomes.iter().filter(|o| o.state == state).count();
                        format!(
                            "{} passed, {} failed, {} skipped in {:.2} s",
                            count(TestState::Passed),
                            count(TestState::Failed),
                            count(TestState::Skipped),
                            summary.elapsed.as_secs_f64()
                        )
                    }
                }
            }
        };
        self.tests.view.update(cx, |view, cx| view.set_running(false, cx));
        self.test_status(message, cx);
    }

    pub(super) fn cancel_tests(&self) {
        if let Some(cancel) = &self.tests.cancel {
            cancel.store(true, Ordering::Release);
        }
    }

    pub(super) fn stop_tests(&mut self, cx: &mut Context<Self>) {
        if !self.is_testing() {
            return;
        }
        self.cancel_tests();
        if self.tests.after_build.is_some() {
            self.cancel_build();
        }
        self.test_status("Stopping tests…".into(), cx);
    }

    pub(super) fn leave_tests(&mut self, cx: &mut Context<Self>) {
        self.cancel_tests();
        self.tests.cancel = None;
        self.tests.generation += 1;
        self.tests.scan_generation += 1;
        self.tests.task = None;
        self.tests.scan = None;
        self.tests.after_build = None;
        self.tests.projects = Default::default();
        self.tests.available = false;
        self.tests.sources = Default::default();
        self.tests.view.update(cx, |view, cx| view.clear(cx));
    }

    fn run_tests_in_context(&mut self, window: &Window, cx: &mut Context<Self>) {
        let selected = self.tests.view.read(cx).selected_tests();
        if self.tests.view.focus_handle(cx).contains_focused(window, cx) {
            self.run_tests(Some(selected), cx);
            return;
        }
        if self.explorer_focus.contains_focused(window, cx) {
            if let Some(projects) = self.selected_build_targets() {
                self.start_tests(Scope::Projects(projects), cx);
            } else {
                self.run_tests(None, cx);
            }
            return;
        }
        let context = self.active_editor().and_then(|e| {
            let e = e.read(cx);
            let b = e.buffer.read(cx);
            if b.tree_version != b.document.version() || b.grammar != Some(rsharp_languages::Grammar::CSharp) {
                return None;
            }
            let (owner, method) =
                rsharp_languages::testing::at(b.tree.as_ref()?, b.document.text(), e.caret().line + 1)?;
            Some((b.path.clone(), owner, method))
        });
        let tests = self.tests.view.read(cx).tests();
        if let Some((path, type_name, method)) = context {
            let owner = type_name.replace('+', ".");
            let chosen = tests
                .into_iter()
                .filter(|t| {
                    if t.path.as_ref() != Some(&path) {
                        return false;
                    }
                    let name = t.method().replace('+', ".");
                    method
                        .as_ref()
                        .map_or_else(|| name.starts_with(&format!("{owner}.")), |m| name == format!("{owner}.{m}"))
                })
                .collect();
            self.run_tests(Some(chosen), cx);
        } else {
            self.run_tests(None, cx);
        }
    }

    pub(super) fn test_actions<T: InteractiveElement + FluentBuilder>(&self, d: T, cx: &mut Context<Self>) -> T {
        d.when(self.workspace.is_some(), |d| {
            d.on_action(cx.listener(|m, _: &ToggleUnitTests, w, cx| m.toggle_tool(Tool::UnitTests, w, cx)))
                .on_action(cx.listener(|m, _: &RefreshTests, _, cx| m.refresh_tests(cx)))
                .when(!self.is_testing() && !self.is_building() && self.tests.available, |d| {
                    d.on_action(cx.listener(|m, _: &RunAllTests, _, cx| m.run_tests(None, cx)))
                        .on_action(cx.listener(|m, _: &RunTests, w, cx| m.run_tests_in_context(w, cx)))
                        .when(!self.tests.view.read(cx).failed_tests().is_empty(), |d| {
                            d.on_action(cx.listener(|m, _: &RerunFailedTests, _, cx| {
                                m.run_tests(Some(m.tests.view.read(cx).failed_tests()), cx)
                            }))
                        })
                })
        })
    }
}

fn selects(choice: &TestItem, test: &TestItem) -> bool {
    choice.project == test.project
        && choice.method().replace('+', ".") == test.method().replace('+', ".")
        && choice.framework.as_ref().is_none_or(|f| test.framework.as_ref() == Some(f))
        // Rows generated only at execution have no standalone discovery identity; their method can be rerun.
        && choice.row.as_ref().is_none_or(|row| test.row.as_ref().is_none_or(|found| found == row))
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    #[test]
    fn a_runtime_only_failed_data_row_reruns_its_discoverable_method() {
        let discovered = TestItem {
            project: "Tests.csproj".into(),
            fully_qualified_name: "Shop.Tests.Adds".into(),
            framework: Some("net10.0".into()),
            ..Default::default()
        };
        let failed = TestItem { row: Some("Adds(Shape { Edge = 3 })".into()), ..discovered.clone() };
        assert!(selects(&failed, &discovered), "Run Failed could not rerun a row the adapter only makes at execution");
        let other = TestItem { row: Some("Adds(Shape { Edge = 4 })".into()), ..discovered };
        assert!(!selects(&failed, &other), "a discoverable unrelated data row was selected");
    }
}
