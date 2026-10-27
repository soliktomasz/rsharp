//! One workspace's builds, held against the Host, with bounded output crossing to the UI.

use super::*;
use crate::console::ConsoleView;
use crate::settings::{BuildResults, settings};
use futures::{FutureExt, channel::mpsc};
use rsharp_core::build::{Kind, Parser, Problem, Severity};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildStatus {
    Succeeded,
    Failed,
    Stopped,
    Refused,
    NotStarted,
}

#[derive(Clone, Debug)]
pub struct BuildOutcome {
    pub status: BuildStatus,
    pub message: String,
    pub problems: Vec<Problem>,
    pub output: Vec<String>,
}

enum Event {
    Output(String, bool),
    Status(String),
    Ended(BuildOutcome),
}

impl MainWindow {
    pub fn is_building(&self) -> bool {
        self.build_cancel.is_some()
    }

    pub fn build_output(&self, cx: &App) -> String {
        self.build.read(cx).text()
    }

    pub fn build_view(&self) -> &Entity<ConsoleView> {
        &self.build
    }

    pub fn build_problems(&self) -> &[Problem] {
        &self.build_problems
    }

    pub fn build_outcome(&self) -> Option<&BuildOutcome> {
        self.build_outcome.as_ref()
    }

    pub(super) fn cancel_build(&mut self) {
        if let Some(cancel) = &self.build_cancel {
            cancel.store(true, Ordering::Release);
        }
    }

    pub(super) fn leave_build(&mut self, cx: &mut Context<Self>) {
        self.cancel_build();
        self.implicit_build = false;
        self.build_cancel = None;
        self.build_generation += 1;
        self._build_task = None;
        self.last_build = None;
        self.build_problems.clear();
        self.build_outcome = None;
        self.build.update(cx, |view, cx| {
            view.running = false;
            view.can_rerun = false;
            view.clear(cx);
        });
    }

    /// The solution's open buffers, including sibling projects, linked files and detached editors.
    pub(super) fn workspace_buffers(&self, cx: &App) -> Vec<Entity<Buffer>> {
        let Some(folder) = self.workspace.as_ref().map(|workspace| workspace.folder.clone()) else {
            return Vec::new();
        };
        let mut roots = vec![folder];
        if let Some(tree) = &self.tree {
            roots.extend(
                tree.solution().projects().iter().filter_map(|project| project.path.parent().map(Path::to_path_buf)),
            );
        }
        let mut buffers = crate::buffer::in_folders(&roots, cx);
        for item in self.run_projects.iter().chain(self.tests.projects.iter()).flat_map(|p| &p.items) {
            if let Some(buffer) = crate::buffer::find(&item.path, cx)
                && !buffers.iter().any(|known| known.entity_id() == buffer.entity_id())
            {
                buffers.push(buffer);
            }
        }
        buffers
    }

    /// Saves every open buffer in the workspace before a build or launch.
    pub(super) fn save_all(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        for buffer in self.workspace_buffers(cx) {
            if buffer.read(cx).is_modified()
                && let Err(error) = buffer.update(cx, |buffer, cx| buffer.save(cx))
            {
                return Err(format!("Could not save {}: {error}", buffer.read(cx).path.display()));
            }
        }
        Ok(())
    }

    pub(super) fn selected_build_targets(&self) -> Option<Vec<PathBuf>> {
        let row = self.rows.get(self.selected?)?;
        match row.kind {
            NodeKind::Project => Some(vec![row.path.clone()?]),
            NodeKind::SolutionFolder => {
                fn under(entries: &[rsharp_core::solution::SolutionEntry], folder: &str) -> Vec<PathBuf> {
                    let mut projects = Vec::new();
                    for entry in entries {
                        match entry {
                            rsharp_core::solution::SolutionEntry::Folder { path, children, .. } => {
                                if path == folder {
                                    collect_projects(children, &mut projects);
                                } else {
                                    projects.extend(under(children, folder));
                                }
                            }
                            _ => {}
                        }
                    }
                    projects
                }
                fn collect_projects(entries: &[rsharp_core::solution::SolutionEntry], into: &mut Vec<PathBuf>) {
                    for entry in entries {
                        match entry {
                            rsharp_core::solution::SolutionEntry::Project(project) if project.exists => {
                                into.push(project.path.clone())
                            }
                            rsharp_core::solution::SolutionEntry::Folder { children, .. } => {
                                collect_projects(children, into)
                            }
                            _ => {}
                        }
                    }
                }
                let projects = under(&self.tree.as_ref()?.solution().entries, row.key.strip_prefix("sf:")?);
                (!projects.is_empty()).then_some(projects)
            }
            _ => None,
        }
    }

    pub(super) fn begin_build(&mut self, kind: Kind, selected: Option<Vec<PathBuf>>, cx: &mut Context<Self>) {
        // A run's build is implicit: it still reports to the Build window and Problems, but only a failure there
        // brings the Build window forward, since the person is looking at the Run window it is starting.
        let implicit = std::mem::take(&mut self.implicit_build);
        let refused = if self.workspace.is_none() {
            Some("Nothing to build: no folder is open")
        } else if self.is_building() {
            Some("Another build is running")
        } else if selected.as_ref().is_some_and(Vec::is_empty) {
            Some("Nothing to build: no projects selected")
        } else {
            None
        };
        if let Some(message) = refused {
            self.said = Some(message.into());
            cx.notify();
            return;
        }
        let workspace = self.workspace.as_ref().unwrap();
        let (folder, solution) = (workspace.folder.clone(), workspace.solution.clone());
        let evaluated: Vec<_> = self
            .tree
            .as_ref()
            .into_iter()
            .flat_map(|tree| {
                tree.solution().projects().into_iter().filter_map(|project| tree.evaluated(&project.path).cloned())
            })
            .collect();
        if let Err(message) = self.save_all(cx) {
            self.said = Some(message.clone().into());
            self.build_outcome = Some(BuildOutcome {
                status: BuildStatus::NotStarted,
                message,
                problems: Vec::new(),
                output: Vec::new(),
            });
            cx.notify();
            return;
        }
        self.implicit_build = implicit;
        self.last_build = Some((kind, selected.clone()));
        self.build_problems.clear();
        self.build_outcome = None;
        self.build_generation += 1;
        let generation = self.build_generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.build_cancel = Some(cancel.clone());
        self.build.update(cx, |view, cx| view.start(folder.clone(), cx));
        if !implicit && settings(cx).open_build_window {
            self.dock.show(Tool::Build);
            self.save_state(cx);
        }
        self.said = Some(
            format!(
                "{} {}…",
                verb(kind),
                solution.as_deref().unwrap_or(&folder).file_name().unwrap_or_default().to_string_lossy()
            )
            .into(),
        );
        let host = self.host.clone();
        let (mut sender, mut receiver) = mpsc::channel(32);
        // Pipe reads can block; a worker also lets gpui's deterministic test executor keep driving Stop.
        std::thread::spawn(move || {
            let outcome = execute(&folder, solution, selected, evaluated, kind, host, &cancel, &mut sender);
            send(&mut sender, Event::Ended(outcome), &cancel, true);
        });
        self._build_task = Some(cx.spawn(async move |this, cx| {
            while let Some(first) = receiver.next().await {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let mut events = vec![first];
                let mut bytes = 0;
                while bytes < 256 * 1024 {
                    let Some(event) = receiver.next().now_or_never().flatten() else { break };
                    bytes += match &event {
                        Event::Output(text, _) => text.len(),
                        _ => 1,
                    };
                    events.push(event);
                }
                let ended = events.iter().any(|event| matches!(event, Event::Ended(_)));
                if this
                    .update(cx, |m, cx| {
                        if m.build_generation != generation {
                            return;
                        }
                        let mut chunks = Vec::new();
                        for event in events {
                            match event {
                                Event::Output(text, error) => chunks.push((text, error)),
                                Event::Status(status) => m.said = Some(status.into()),
                                Event::Ended(outcome) => m.finish_build(outcome, cx),
                            }
                        }
                        if !chunks.is_empty() {
                            m.build.update(cx, |view, cx| view.append(&chunks, cx));
                        }
                        cx.notify();
                    })
                    .is_err()
                    || ended
                {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn finish_build(&mut self, outcome: BuildOutcome, cx: &mut Context<Self>) {
        let implicit = std::mem::take(&mut self.implicit_build);
        self.build_cancel = None;
        self.build_problems = outcome.problems.clone();
        self.xaml_built(cx);
        self.restart_preview(cx);
        let failed = matches!(outcome.status, BuildStatus::Failed | BuildStatus::NotStarted);
        let warned = outcome.problems.iter().any(|problem| problem.severity == Severity::Warning);
        let show = match settings(cx).build_results {
            BuildResults::Always => true,
            BuildResults::WarningsOrErrors => failed || (warned && outcome.status != BuildStatus::Stopped),
            BuildResults::Errors => failed,
            BuildResults::Never => false,
        };
        let show = if implicit { failed } else { show };
        self.build.update(cx, |view, cx| {
            view.running = false;
            cx.notify();
        });
        self.said = Some(outcome.message.clone().into());
        let ended = outcome.status;
        self.build_outcome = Some(outcome);
        if show {
            self.dock.show(Tool::Build);
            self.save_state(cx);
        }
        self.after_run_build(ended, cx);
        self.after_debug_build(ended, cx);
        self.after_tests_build(ended, cx);
    }

    pub(super) fn build_problems_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div().id("build-problems").flex_1().min_h_0().overflow_y_scroll().px(px(6.)).children(
            self.build_problems.iter().enumerate().map(|(i, problem)| {
                let target = problem.path.clone().map(|path| (path, problem.line, problem.column));
                let place = target
                    .as_ref()
                    .map(|(path, line, _)| {
                        format!("{}:{}", path.file_name().unwrap_or_default().to_string_lossy(), line + 1)
                    })
                    .unwrap_or_default();
                self.problem_row(
                    ("build-problem", i),
                    problem.severity == Severity::Error,
                    format!("{}: {}", problem.code, problem.message.lines().next().unwrap_or_default()),
                    place,
                    move |m, window, cx| {
                        if let Some((path, line, column)) = &target {
                            m.open_file(path, Some((*line, *column)), window, cx);
                        }
                    },
                    cx,
                )
                .debug_selector(move || format!("build problem: {i}"))
                .tooltip({
                    let message = problem.message.clone();
                    move |_, cx| cx.new(|_| Tip(message.clone().into())).into()
                })
            }),
        )
    }

    pub(super) fn build_actions<T: InteractiveElement + FluentBuilder>(&self, d: T, cx: &mut Context<Self>) -> T {
        d.when(self.workspace.is_some(), |d| {
            d.on_action(cx.listener(|m, _: &ToggleBuild, window, cx| m.toggle_tool(Tool::Build, window, cx)))
                .when(!self.is_building(), |d| {
                    d.on_action(cx.listener(|m, _: &BuildSolution, _, cx| m.begin_build(Kind::Build, None, cx)))
                        .on_action(cx.listener(|m, _: &RebuildSolution, _, cx| m.begin_build(Kind::Rebuild, None, cx)))
                        .on_action(cx.listener(|m, _: &CleanSolution, _, cx| m.begin_build(Kind::Clean, None, cx)))
                        .when(self.selected_build_targets().is_some(), |d| {
                            d.on_action(cx.listener(|m, _: &BuildSelected, _, cx| {
                                m.begin_build(Kind::Build, m.selected_build_targets(), cx)
                            }))
                            .on_action(cx.listener(|m, _: &RebuildSelected, _, cx| {
                                m.begin_build(Kind::Rebuild, m.selected_build_targets(), cx)
                            }))
                            .on_action(cx.listener(|m, _: &CleanSelected, _, cx| {
                                m.begin_build(Kind::Clean, m.selected_build_targets(), cx)
                            }))
                        })
                })
                .when(self.is_building(), |d| d.on_action(cx.listener(|m, _: &Stop, _, cx| m.stop_everything(cx))))
        })
    }
}

fn send(sender: &mut mpsc::Sender<Event>, mut event: Event, cancel: &AtomicBool, terminal: bool) -> bool {
    loop {
        match sender.try_send(event) {
            Ok(()) => return true,
            Err(error) if error.is_full() => {
                if !terminal && cancel.load(Ordering::Acquire) {
                    return false;
                }
                event = error.into_inner();
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => {
                cancel.store(true, Ordering::Release);
                return false;
            }
        }
    }
}

fn verb(kind: Kind) -> &'static str {
    match kind {
        Kind::Build => "Building",
        Kind::Rebuild => "Rebuilding",
        Kind::Clean => "Cleaning",
    }
}

fn windows_targeting(target: &Path, projects: &[rsharp_core::project::EvaluatedProject]) -> bool {
    let by_path: HashMap<_, _> = projects.iter().map(|project| (&project.path, project)).collect();
    let mut pending = if by_path.contains_key(&target.to_path_buf()) {
        vec![target.to_path_buf()]
    } else {
        projects.iter().map(|project| project.path.clone()).collect()
    };
    let mut seen = HashSet::new();
    while let Some(path) = pending.pop() {
        if seen.insert(path.clone())
            && let Some(project) = by_path.get(&path)
        {
            if project.dependencies.iter().any(|dependency| dependency.windows_targeting) {
                return true;
            }
            pending.extend(project.dependencies.iter().flat_map(|dependency| dependency.projects.iter().cloned()));
        }
    }
    false
}

/// The models a build decides `EnableWindowsTargeting` from: the window's own, plus the Host's answer for each project
/// this build could touch that the window had not evaluated yet — which is every project of a solution just created.
fn completed_models(
    solution: Option<&Path>,
    targets: &[PathBuf],
    known: Vec<rsharp_core::project::EvaluatedProject>,
    host: Option<&rsharp_lsp::SharedHost>,
    cancel: &AtomicBool,
) -> Vec<rsharp_core::project::EvaluatedProject> {
    let Some(host) = host else { return known };
    let mut seen: HashSet<_> = known.iter().map(|project| project.path.clone()).collect();
    let mut candidates = Vec::new();
    for target in targets {
        let is_project = target
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| matches!(extension, "csproj" | "fsproj" | "vbproj"));
        if is_project {
            candidates.push(target.clone());
        } else if let Some(read) = solution.and_then(rsharp_core::solution::read) {
            candidates.extend(read.projects().into_iter().filter(|project| project.exists).map(|p| p.path.clone()));
        }
    }
    let mut models = known;
    for project in candidates {
        if cancel.load(Ordering::Acquire) {
            return models;
        }
        // A project the Host cannot evaluate leaves the decision as it was: a build is not refused for its flag.
        if seen.insert(project.clone())
            && let Ok(evaluated) = host.evaluate(&project, solution)
        {
            models.push(evaluated);
        }
    }
    models
}

fn execute(
    folder: &Path,
    solution: Option<PathBuf>,
    selected: Option<Vec<PathBuf>>,
    evaluated: Vec<rsharp_core::project::EvaluatedProject>,
    kind: Kind,
    host: Option<rsharp_lsp::SharedHost>,
    cancel: &AtomicBool,
    sender: &mut mpsc::Sender<Event>,
) -> BuildOutcome {
    let mut parser = Parser::new(folder.to_path_buf());
    let mut lines: VecDeque<String> = VecDeque::new();
    let mut partials = [String::new(), String::new()];
    let mut status = BuildStatus::Succeeded;
    let mut failure = None;
    let targets = selected.unwrap_or_else(|| {
        solution.clone().map(|solution| vec![solution]).unwrap_or_else(|| {
            let mut projects: Vec<_> = crate::workspace::all_files(folder, usize::MAX)
                .into_iter()
                .filter(|path| {
                    path.extension()
                        .and_then(|s| s.to_str())
                        .is_some_and(|extension| matches!(extension, "csproj" | "fsproj" | "vbproj"))
                })
                .collect();
            projects.sort();
            if projects.is_empty() {
                projects.push(folder.to_path_buf());
            }
            projects
        })
    });
    // The window's models are the ones it had when Build was pressed: a New Solution builds before its projects are
    // evaluated, and a Windows-targeting project would then be given no `EnableWindowsTargeting` and fail with
    // NETSDK1100. Completing them before the hold is taken keeps requesting them out of the hold's wait.
    let evaluated = completed_models(solution.as_deref(), &targets, evaluated, host.as_ref(), cancel);
    let dotnet = rsharp_core::programs::dotnet();
    let held = host
        .as_ref()
        .map(|host| {
            host.hold(cancel, |running| {
                let names = running
                    .iter()
                    .map(|path| path.file_stem().unwrap_or_default().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(", ");
                let message = format!("Waiting for the design-time build of {names} to end\n");
                send(sender, Event::Output(message, false), cancel, false);
            })
        })
        .transpose();
    let hold = match held {
        Ok(hold) => hold,
        Err(error) => {
            status = if cancel.load(Ordering::Acquire) { BuildStatus::Stopped } else { BuildStatus::NotStarted };
            failure = Some(error.to_string());
            None
        }
    };
    if status == BuildStatus::Succeeded {
        if let Some(dotnet) = dotnet {
            for target in targets {
                if cancel.load(Ordering::Acquire) {
                    status = BuildStatus::Stopped;
                    break;
                }
                send(
                    sender,
                    Event::Status(format!(
                        "{} {}…",
                        verb(kind),
                        target.file_name().unwrap_or_default().to_string_lossy()
                    )),
                    cancel,
                    false,
                );
                let args = rsharp_core::build::arguments(&target, kind, windows_targeting(&target, &evaluated));
                let command = rsharp_core::dotnet::command(&dotnet, args, folder);
                let result = rsharp_core::build::run_while(
                    command,
                    cancel,
                    || hold.as_ref().is_none_or(|hold| hold.failed().is_none()),
                    |text, error| {
                        let partial = &mut partials[usize::from(error)];
                        partial.push_str(text);
                        while let Some(end) = partial.find('\n') {
                            let line: String = partial.drain(..=end).collect();
                            parser.push(&line);
                            lines.push_back(line.trim_end_matches(['\n', '\r']).to_string());
                            if lines.len() > 20 {
                                lines.pop_front();
                            }
                        }
                        send(sender, Event::Output(text.to_string(), error), cancel, false);
                    },
                );
                for partial in &mut partials {
                    if !partial.is_empty() {
                        parser.push(partial);
                        lines.push_back(std::mem::take(partial));
                        if lines.len() > 20 {
                            lines.pop_front();
                        }
                    }
                }
                if let Some(error) = hold.as_ref().and_then(|hold| hold.failed()) {
                    status = BuildStatus::Failed;
                    failure = Some(format!("The project Host stopped: {error}"));
                    break;
                }
                match result {
                    Ok(Some(0)) => {}
                    Ok(Some(code)) => {
                        status = BuildStatus::Failed;
                        failure = Some(format!("exit code {code}"));
                        break;
                    }
                    Ok(None) => {
                        status = BuildStatus::Stopped;
                        break;
                    }
                    Err(error) => {
                        status = BuildStatus::NotStarted;
                        failure = Some(error.to_string());
                        break;
                    }
                }
            }
        } else {
            status = BuildStatus::NotStarted;
            failure = Some("No .NET command line found: install the .NET SDK, or set DOTNET_ROOT".into());
        }
    }
    drop(hold);
    let problems = parser.finish();
    let errors = problems.iter().filter(|problem| problem.severity == Severity::Error).count();
    let warnings = problems.len() - errors;
    if status == BuildStatus::Succeeded && errors > 0 {
        status = BuildStatus::Failed;
    }
    let noun = match kind {
        Kind::Build => "Build",
        Kind::Rebuild => "Rebuild",
        Kind::Clean => "Clean",
    };
    let message = match status {
        BuildStatus::Succeeded if warnings == 0 => format!("{noun} succeeded"),
        BuildStatus::Succeeded => format!("{noun} succeeded with {warnings} warning(s)"),
        BuildStatus::Stopped => format!("{noun} stopped"),
        _ if errors + warnings > 0 => {
            format!("{noun} failed: {errors} error(s), {warnings} warning(s); Problems lists them")
        }
        _ => format!("{noun} failed: {}", failure.unwrap_or_default()),
    };
    send(
        sender,
        Event::Output(format!("\n{message}\n"), status == BuildStatus::Failed || status == BuildStatus::NotStarted),
        cancel,
        true,
    );
    BuildOutcome { status, message, problems, output: lines.into_iter().collect() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsharp_core::project::{EvaluatedProject, FrameworkDependencies};

    fn project(path: &str, references: &[&str], windows: bool) -> EvaluatedProject {
        EvaluatedProject {
            path: PathBuf::from(path),
            dependencies: vec![FrameworkDependencies {
                projects: references.iter().map(PathBuf::from).collect(),
                windows_targeting: windows,
                ..FrameworkDependencies::default()
            }],
            ..EvaluatedProject::default()
        }
    }

    #[test]
    fn a_build_is_given_windows_targeting_through_references_without_spinning_on_a_cycle() {
        let models = [
            project("/work/App/App.csproj", &["/work/Core/Core.csproj"], false),
            project("/work/Core/Core.csproj", &["/work/App/App.csproj", "/work/Win/Win.csproj"], false),
            project("/work/Win/Win.csproj", &["/work/Core/Core.csproj"], true),
            project("/work/Plain/Plain.csproj", &[], false),
        ];
        assert!(
            windows_targeting(Path::new("/work/App/App.csproj"), &models),
            "a project whose reference needs Windows targeting was built without it"
        );
        assert!(
            windows_targeting(Path::new("/work/Shop.slnx"), &models),
            "a solution build ignored a project that needs Windows targeting"
        );
        assert!(!windows_targeting(Path::new("/work/Plain/Plain.csproj"), &models));
    }
}
