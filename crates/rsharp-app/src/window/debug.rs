//! One workspace's debugging: the folder's breakpoints, the plan a Debug works out, the session in `rsharp-dap`, and what
//! the Debug window shows of it. Every request waits off the UI thread; what the adapter and the program say lands here.

use super::*;
use crate::debug_view::{DebugAction, FrameRow, Phase, VariableRow};
use crate::state::store;
use futures::channel::mpsc;
use futures::{FutureExt, StreamExt};
use rsharp_core::run::LaunchPlan;
use rsharp_dap::{Event as Said, Session, SourceBreakpoint, Start};
use std::collections::{BTreeMap, BTreeSet};

/// The window's side of a debug session: its breakpoints, the session once the adapter is ready, and where it stands.
pub(super) struct DebugState {
    pub(super) breakpoints: rsharp_core::debug::Breakpoints,
    session: Option<Arc<Session>>,
    /// Counted on every start and stop, so a reply or an event of a session since stopped lands nowhere.
    generation: u64,
    phase: Phase,
    /// The plan being started, or waiting for its build.
    plan: Option<LaunchPlan>,
    after_build: Option<LaunchPlan>,
    /// The thread that stopped, while paused.
    thread: Option<i64>,
    /// The frames of the stopped thread, innermost first.
    frames: Vec<rsharp_dap::protocol::Frame>,
    /// The file the program is stopped in, whose buffer shows the execution line.
    shown_at: Option<PathBuf>,
    /// What the debugger made of each breakpoint: whether it will be hit, and why not.
    verification: BTreeMap<(PathBuf, usize), (bool, Option<String>)>,
    /// The program attached to, while one is; none for a launch.
    attached: Option<rsharp_core::debug::DotnetProcess>,
    /// The program attached to last in this folder, which Reattach asks for.
    recent_attached: Option<rsharp_core::debug::DotnetProcess>,
    /// What the adapter said while the session was still starting.
    pending: Vec<Said>,
    _task: Option<Task<()>>,
}

impl DebugState {
    pub(super) fn new() -> Self {
        Self {
            breakpoints: rsharp_core::debug::Breakpoints::default(),
            session: None,
            generation: 0,
            phase: Phase::Idle,
            plan: None,
            after_build: None,
            thread: None,
            frames: Vec::new(),
            shown_at: None,
            verification: BTreeMap::new(),
            attached: None,
            recent_attached: None,
            pending: Vec::new(),
            _task: None,
        }
    }
}

enum Event {
    /// The plan a Debug works out, or why it cannot run.
    Resolved(Result<Box<LaunchPlan>, String>),
    Started(Arc<Session>),
    Failed(String),
    Said(Said),
}

impl MainWindow {
    /// A folder's breakpoints as its document keeps them.
    pub fn breakpoints(&self) -> &rsharp_core::debug::Breakpoints {
        &self.debug.breakpoints
    }

    pub fn debug_phase(&self) -> Phase {
        self.debug.phase
    }

    /// The frames of the thread stopped at, innermost first.
    pub fn debug_frames(&self) -> &[rsharp_dap::protocol::Frame] {
        &self.debug.frames
    }

    pub fn debug_view(&self) -> &Entity<crate::debug_view::DebugView> {
        &self.debug_view
    }

    pub fn debug_console(&self) -> &Entity<crate::console::ConsoleView> {
        &self.debug_console
    }

    /// Whether a session is starting, running or paused.
    pub fn is_debugging(&self) -> bool {
        !matches!(self.debug.phase, Phase::Idle)
    }

    /// Debug: the chosen configuration's program is built, its plan worked out, and it is started under the debugger with
    /// the folder's breakpoints, as Rider's F5 starts it.
    pub(super) fn debug_program(&mut self, cx: &mut Context<Self>) {
        if self.is_debugging() {
            self.said = Some("A debugging session is already running".into());
            cx.notify();
            return;
        }
        let Some(configuration) = self.chosen_run_configuration().cloned() else {
            self.refuse_debug("Nothing to debug: this folder has no run configuration", cx);
            return;
        };
        if self.workspace.is_none() {
            self.refuse_debug("Nothing to debug: no folder is open", cx);
            return;
        }
        if let Some(why) = self.why_not_run(&configuration) {
            self.refuse_debug(&format!("{} cannot run here: {why}", configuration.name), cx);
            return;
        }
        if let Err(message) = self.save_all(cx) {
            self.refuse_debug(&message, cx);
            return;
        }
        let project = self
            .run_projects
            .iter()
            .find(|project| rsharp_core::paths::same(&project.path, &configuration.project))
            .cloned()
            .unwrap_or_else(|| rsharp_core::project::EvaluatedProject {
                path: configuration.project.clone(),
                ..rsharp_core::project::EvaluatedProject::default()
            });
        let folder = rsharp_core::run::configuration::project_folder(&configuration.project);
        self.debug.generation += 1;
        let generation = self.debug.generation;
        self.debug.phase = Phase::Starting;
        self.debug.plan = None;
        self.debug.after_build = None;
        self.debug.frames.clear();
        self.debug.thread = None;
        self.debug.pending.clear();
        self.debug.verification.clear();
        self.debug_view.update(cx, |view, cx| {
            view.set_muted(self.debug.breakpoints.muted(), cx);
            view.starting(&configuration.name, cx);
        });
        self.debug_console.update(cx, |view, cx| view.start(folder, cx));
        self.said = Some(format!("Building and debugging {}…", configuration.name).into());
        if !self.dock.is_open(Tool::Debug) {
            self.dock.show(Tool::Debug);
            self.save_state(cx);
        }
        let (sender, mut receiver) = mpsc::unbounded();
        let resolving = sender.clone();
        std::thread::spawn(move || {
            let needed = rsharp_core::run::plan::needed_properties(&configuration);
            let resolved =
                rsharp_core::run::plan::properties(&configuration.project, configuration.framework.as_deref(), &needed)
                    .and_then(|properties| LaunchPlan::resolve(&configuration, &project, &properties));
            let _ = resolving.unbounded_send(Event::Resolved(resolved.map(Box::new)));
        });
        let handle = self.handle;
        self.debug._task = Some(cx.spawn(async move |_, cx| {
            while let Some(first) = receiver.next().await {
                let mut events = vec![first];
                while let Some(event) = receiver.next().now_or_never().flatten() {
                    events.push(event);
                }
                let landed = handle
                    .update(cx, |m, window, cx| {
                        if m.debug.generation != generation {
                            return false;
                        }
                        for event in events {
                            m.debug_event(event, window, cx);
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !landed {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn debug_event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            Event::Resolved(Ok(plan)) => self.begin_debug(*plan, cx),
            Event::Resolved(Err(why)) => self.refuse_debug(&why, cx),
            Event::Started(session) => {
                let verified = session.verified();
                self.debug.session = Some(session);
                for (path, answers) in verified {
                    let lines: Vec<usize> =
                        answers.iter().filter_map(|answer| answer.line.map(|line| line.saturating_sub(1))).collect();
                    self.note_verification(&path, &lines, answers, cx);
                }
                self.debug.phase = Phase::Running;
                self.debug_view.update(cx, |view, cx| view.running(cx));
                self.said = Some(
                    match &self.debug.attached {
                        Some(process) => format!("Attached to {}…", process.program()),
                        None => format!("Debugging {}…", self.debug_name()),
                    }
                    .into(),
                );
                // A stop told before the session was ready stands, as one told before a build's end does.
                let pending = std::mem::take(&mut self.debug.pending);
                for said in pending {
                    self.session_said(said, window, cx);
                }
                cx.notify();
            }
            Event::Failed(why) => self.refuse_debug(&why, cx),
            Event::Said(said) => self.session_said(said, window, cx),
        }
    }

    fn debug_name(&self) -> String {
        self.debug
            .plan
            .as_ref()
            .map(|plan| plan.name.clone())
            .or_else(|| self.debug.attached.as_ref().map(|process| process.program()))
            .unwrap_or_default()
    }

    /// The plan is worked out: its project builds first when it says so, and the session starts after it.
    fn begin_debug(&mut self, plan: LaunchPlan, cx: &mut Context<Self>) {
        self.debug.plan = Some(plan.clone());
        if rsharp_core::programs::dotnet().is_none() {
            self.refuse_debug("No .NET command line found: install the .NET SDK, or set DOTNET_ROOT", cx);
            return;
        }
        if rsharp_dap::adapter().is_none() {
            self.refuse_debug("The debugger is not available: Passenger's debug adapter was not built", cx);
            return;
        }
        match plan.build.clone() {
            Some(step) => {
                self.debug.after_build = Some(plan);
                self.implicit_build = true;
                let generation = self.build_generation;
                self.begin_build(rsharp_core::build::Kind::Build, Some(vec![step.project]), cx);
                if self.build_generation == generation
                    && let Some(plan) = self.debug.after_build.take()
                {
                    self.implicit_build = false;
                    self.refuse_debug(&format!("{} did not start: its build did not start", plan.name), cx);
                }
            }
            None => self.start_session(cx),
        }
    }

    /// Evaluate Expression (Alt+F8): Rider's dialog over the paused program, the text chosen in the editor if any.
    pub(super) fn open_evaluate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.evaluate_dialog.is_some() {
            return;
        }
        if self.debug.phase != Phase::Paused {
            self.said = Some("The program must be paused to evaluate an expression".into());
            cx.notify();
            return;
        }
        let initial = self.selected_text(cx);
        let frame = self.debug.frames.first().map(|frame| frame.short_name());
        let dialog = cx.new(|cx| crate::evaluate_dialog::EvaluateDialog::new(&initial, frame, cx));
        cx.subscribe_in(&dialog, window, |m, dialog, ended: &crate::evaluate_dialog::Ended, window, cx| match ended {
            crate::evaluate_dialog::Ended::Evaluate(expression) => {
                m.evaluate_in(dialog.clone(), expression.clone(), cx)
            }
            crate::evaluate_dialog::Ended::Dismissed => {
                m.evaluate_dialog = None;
                m.focus_editor(window, cx);
            }
        })
        .detach();
        window.focus(&dialog.read(cx).expression_focus(cx));
        self.evaluate_dialog = Some(dialog);
        cx.notify();
    }

    /// The text chosen in the editor, which Evaluate Expression starts with.
    fn selected_text(&self, cx: &App) -> String {
        let Some(editor) = self.active_editor() else { return String::new() };
        let editor = editor.read(cx);
        let selection = editor.primary();
        if selection.is_empty() {
            return String::new();
        }
        let text = editor.text();
        let start = selection.start().min(text.len_bytes());
        let end = selection.end().min(text.len_bytes());
        text.byte_slice(start..end).to_string().trim().to_string()
    }

    fn evaluate_in(
        &mut self,
        dialog: Entity<crate::evaluate_dialog::EvaluateDialog>,
        expression: String,
        cx: &mut Context<Self>,
    ) {
        let Some(frame) = self.debug.frames.first().map(|frame| frame.id) else { return };
        let Some(session) = self.debug.session.clone() else { return };
        let generation = self.debug.generation;
        let handle = self.handle;
        cx.spawn(async move |_, cx| {
            let read = cx.background_executor().spawn(async move { session.evaluate(&expression, frame) }).await;
            let _ = handle.update(cx, |m, _, cx| {
                if m.debug.generation != generation || m.evaluate_dialog.as_ref() != Some(&dialog) {
                    return;
                }
                match read {
                    Ok(value) => {
                        let answer = match value.r#type {
                            Some(kind) => format!("{} ({kind})", value.result),
                            None => value.result,
                        };
                        dialog.update(cx, |dialog, cx| dialog.set_result(answer, false, cx));
                    }
                    Err(why) => dialog.update(cx, |dialog, cx| dialog.set_result(why, true, cx)),
                }
            });
        })
        .detach();
    }

    /// Set Next Statement: the paused thread goes on from the caret's line, where the debugger allows it.
    pub(super) fn set_next_statement(&mut self, cx: &mut Context<Self>) {
        if self.debug.phase != Phase::Paused {
            return;
        }
        let (Some(thread), Some(editor)) = (self.debug.thread, self.active_editor()) else { return };
        let (path, line) = {
            let editor = editor.read(cx);
            (editor.path.clone(), editor.caret().line)
        };
        self.ask_session(
            cx,
            move |session| {
                let targets = session.goto_targets(&path, line)?;
                let target = targets
                    .first()
                    .ok_or_else(|| format!("The debugger cannot go on from {}:{}", path.display(), line + 1))?;
                session.set_next_statement(thread, target.id)?;
                read_stop(&session, thread)
            },
            |m, (frames, variables), window, cx| {
                m.show_stopped("Paused where Set Next Statement put it".into(), None, frames, variables, window, cx);
            },
        );
    }

    /// The build before the launch has ended: a session starts when it succeeded.
    pub(super) fn after_debug_build(&mut self, ended: BuildStatus, cx: &mut Context<Self>) {
        let Some(plan) = self.debug.after_build.take() else { return };
        if ended == BuildStatus::Succeeded {
            self.start_session(cx);
        } else {
            self.refuse_debug(&format!("{} did not start: its build failed", plan.name), cx);
        }
    }

    /// Starts the adapter and the program under it, the folder's breakpoints sent as the session opens.
    fn start_session(&mut self, cx: &mut Context<Self>) {
        let Some(plan) = self.debug.plan.clone() else { return };
        let Some(adapter) = rsharp_dap::adapter() else {
            self.refuse_debug("The debugger is not available: Passenger's debug adapter was not built", cx);
            return;
        };
        let Some(folder) = self.workspace.as_ref().map(|workspace| workspace.folder.clone()) else { return };
        // The program is the project's own assembly, as the adapter starts it: the `dotnet` host's first argument is the
        // plan's, not the debugger's.
        let program = plan.built.clone().unwrap_or_else(|| plan.program.clone());
        let arguments = match &plan.built {
            Some(built) if plan.arguments.first().is_some_and(|first| Path::new(first) == built) => {
                plan.arguments[1..].to_vec()
            }
            _ => plan.arguments.clone(),
        };
        let launch = rsharp_dap::Launch {
            program,
            arguments,
            working_directory: plan.working_directory.clone(),
            environment: plan.environment.clone(),
            stop_at_entry: false,
        };
        let breakpoints = self.session_breakpoints(&folder);
        let exceptions = self.enabled_exceptions();
        let symbols = store(cx).root().join("cache").join("symbols");
        let (sender, receiver) = mpsc::unbounded();
        let generation = self.debug.generation;
        std::thread::spawn(move || {
            let reports = sender.clone();
            let result =
                Session::start(&adapter, Start::Launch(launch), &symbols, breakpoints, exceptions, move |said| {
                    let _ = reports.unbounded_send(Event::Said(said));
                });
            let event = match result {
                Ok(session) => Event::Started(Arc::new(session)),
                Err(why) => Event::Failed(why),
            };
            let _ = sender.unbounded_send(event);
        });
        self.watch_session(receiver, generation, cx);
        self.said = Some(format!("Starting {} under the debugger…", plan.name).into());
        cx.notify();
    }

    /// The folder's breakpoints as a session is told of them: only the enabled lines, each with what it asks.
    fn session_breakpoints(&self, folder: &Path) -> Vec<(PathBuf, Vec<SourceBreakpoint>)> {
        self.debug
            .breakpoints
            .resolved(folder)
            .enabled()
            .into_iter()
            .map(|(path, lines)| {
                let lines = lines
                    .into_iter()
                    .map(|line| SourceBreakpoint {
                        line: line.line,
                        condition: filled(&line.condition),
                        hit_condition: filled(&line.hit_count),
                        log_message: filled(&line.log_message),
                    })
                    .collect();
                (path, lines)
            })
            .collect()
    }

    /// Drains a session's events into the window until it ends, each event applied only while its generation stands.
    fn watch_session(&mut self, mut receiver: mpsc::UnboundedReceiver<Event>, generation: u64, cx: &mut Context<Self>) {
        let handle = self.handle;
        self.debug._task = Some(cx.spawn(async move |_, cx| {
            while let Some(first) = receiver.next().await {
                let mut events = vec![first];
                while let Some(event) = receiver.next().now_or_never().flatten() {
                    events.push(event);
                }
                let landed = handle
                    .update(cx, |m, window, cx| {
                        if m.debug.generation != generation {
                            return false;
                        }
                        for event in events {
                            m.debug_event(event, window, cx);
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !landed {
                    break;
                }
            }
        }));
    }

    /// Attach to Process: the program chosen in the dialog, with the folder's breakpoints, as Rider's Debug attaches.
    pub(super) fn attach_to(&mut self, process: rsharp_core::debug::DotnetProcess, cx: &mut Context<Self>) {
        if self.is_debugging() {
            self.said = Some("A debugging session is already running".into());
            cx.notify();
            return;
        }
        if let Err(message) = self.save_all(cx) {
            self.refuse_debug(&message, cx);
            return;
        }
        let Some(adapter) = rsharp_dap::adapter() else {
            self.refuse_debug("The debugger is not available: Passenger's debug adapter was not built", cx);
            return;
        };
        self.debug.generation += 1;
        let generation = self.debug.generation;
        self.debug.phase = Phase::Starting;
        self.debug.plan = None;
        self.debug.after_build = None;
        self.debug.frames.clear();
        self.debug.thread = None;
        self.debug.pending.clear();
        self.debug.verification.clear();
        self.debug.attached = Some(process.clone());
        self.debug.recent_attached = Some(process.clone());
        self.save_debug_sessions(cx);
        let folder = self
            .workspace
            .as_ref()
            .map(|workspace| workspace.folder.clone())
            .unwrap_or_else(rsharp_core::programs::home);
        self.debug_console.update(cx, |view, cx| view.start(folder.clone(), cx));
        self.debug_view.update(cx, |view, cx| view.starting(&process.program(), cx));
        if !self.dock.is_open(Tool::Debug) {
            self.dock.show(Tool::Debug);
            self.save_state(cx);
        }
        let breakpoints = self.session_breakpoints(&folder);
        let exceptions = self.enabled_exceptions();
        let symbols = store(cx).root().join("cache").join("symbols");
        let id = process.id;
        let (sender, receiver) = mpsc::unbounded();
        std::thread::spawn(move || {
            let reports = sender.clone();
            let result = Session::start(
                &adapter,
                Start::Attach { process_id: id },
                &symbols,
                breakpoints,
                exceptions,
                move |said| {
                    let _ = reports.unbounded_send(Event::Said(said));
                },
            );
            let event = match result {
                Ok(session) => Event::Started(Arc::new(session)),
                Err(why) => Event::Failed(why),
            };
            let _ = sender.unbounded_send(event);
        });
        self.watch_session(receiver, generation, cx);
        self.said = Some(format!("Attaching to {}…", process.program()).into());
        cx.notify();
    }

    /// The program attached to, while one is.
    pub fn attached_process(&self) -> Option<&rsharp_core::debug::DotnetProcess> {
        self.debug.attached.as_ref()
    }

    /// The program attached to last in this folder.
    pub fn recent_attached(&self) -> Option<&rsharp_core::debug::DotnetProcess> {
        self.debug.recent_attached.as_ref()
    }

    fn save_debug_sessions(&self, cx: &App) {
        let Some(folder) = self.workspace.as_ref().map(|workspace| workspace.folder.clone()) else { return };
        let sessions = rsharp_core::debug::DebugSessions { recent_attached: self.debug.recent_attached.clone() };
        store(cx).write_for(&folder, &sessions);
    }

    /// Detach: the program goes on without the debugger, which is what Stop does to an attached one, as Rider's Detach.
    pub(super) fn detach_debug(&mut self, cx: &mut Context<Self>) {
        if self.debug.attached.is_none() {
            return;
        }
        self.debug.generation += 1;
        self.debug._task = None;
        self.debug.after_build = None;
        if let Some(session) = self.debug.session.take() {
            std::thread::spawn(move || session.disconnect(false));
        }
        let name = self.debug_name();
        self.finish_debug(&format!("Detached from {name}"), cx);
    }

    /// Reattach to Process: the program attached to last in this folder, if it is running and alone.
    pub(super) fn reattach(&mut self, cx: &mut Context<Self>) {
        let Some(last) = self.debug.recent_attached.clone() else {
            self.said = Some("Nothing has been attached to in this folder: Attach to Process chooses a program".into());
            cx.notify();
            return;
        };
        let handle = self.handle;
        let generation = self.debug.generation;
        self.said = Some(format!("Looking for {}…", last.program()).into());
        cx.spawn(async move |_, cx| {
            let listed = cx.background_executor().spawn(async move { rsharp_core::debug::processes() }).await;
            let _ = handle.update(cx, |m, _, cx| {
                if m.debug.generation != generation {
                    return;
                }
                if let Some(same) =
                    listed.iter().find(|process| process.id == last.id && process.program() == last.program())
                {
                    m.attach_to(same.clone(), cx);
                    return;
                }
                let mut alike: Vec<_> =
                    listed.iter().filter(|process| process.program() == last.program()).cloned().collect();
                match alike.len() {
                    0 => {
                        m.said = Some(
                            format!("{} is not running: Attach to Process chooses another program", last.program())
                                .into(),
                        );
                        cx.notify();
                    }
                    1 => m.attach_to(alike.remove(0), cx),
                    _ => {
                        m.said = Some(
                            format!(
                                "Several programs named {} are running: Attach to Process chooses one",
                                last.program()
                            )
                            .into(),
                        );
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// What the adapter says while a session runs.
    fn session_said(&mut self, said: Said, window: &mut Window, cx: &mut Context<Self>) {
        match said {
            Said::Stopped(stop) => {
                if self.debug.session.is_none() {
                    self.debug.pending.push(Said::Stopped(stop));
                    return;
                }
                let at = stop.path.clone().zip(stop.line);
                self.debug.thread = Some(stop.thread);
                self.said = Some(stop.said().into());
                self.fetch_stopped(stop.said(), at, stop.thread, window, cx);
            }
            Said::Continued => {
                if self.debug.session.is_none() {
                    self.debug.pending.push(Said::Continued);
                    return;
                }
                self.debug.phase = Phase::Running;
                self.debug.thread = None;
                self.debug.frames.clear();
                self.clear_execution_line(cx);
                self.debug_view.update(cx, |view, cx| view.running(cx));
                self.said = Some("Running…".into());
            }
            Said::Output { text, error } => {
                self.debug_console.update(cx, |view, cx| view.append(&[(text, error)], cx));
            }
            Said::Breakpoint { path, line, verified, message } => {
                if let (Some(path), Some(line)) = (path, line) {
                    let key = rsharp_core::paths::canonical(&path);
                    self.debug.verification.insert((key, line), (verified, message));
                    self.show_breakpoints(&path, cx);
                }
            }
            Said::Exited(code) => {
                let name = self.debug_name();
                let message = match code {
                    Some(code) => format!("{name} exited with code {code}"),
                    None => format!("{name} exited"),
                };
                self.finish_debug(&message, cx);
            }
            Said::Terminated | Said::Ended => {
                let name = self.debug_name();
                self.finish_debug(&format!("Debugging {name} ended"), cx);
            }
        }
        cx.notify();
    }

    /// The stop's frames and the chosen frame's values, read off the UI thread, then shown and opened at the stop.
    fn fetch_stopped(
        &mut self,
        status: String,
        at: Option<(PathBuf, usize)>,
        thread: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.debug.session.clone() else { return };
        let generation = self.debug.generation;
        let handle = self.handle;
        cx.spawn(async move |_, cx| {
            let read = cx.background_executor().spawn(async move { read_stop(&session, thread) }).await;
            let _ = handle.update(cx, |m, window, cx| {
                if m.debug.generation != generation {
                    return;
                }
                match read {
                    Ok((frames, variables)) => m.show_stopped(status, at, frames, variables, window, cx),
                    Err(why) => m.fail_debug(&why, cx),
                }
            });
        })
        .detach();
        let _ = window;
    }

    fn show_stopped(
        &mut self,
        status: String,
        at: Option<(PathBuf, usize)>,
        frames: Vec<rsharp_dap::protocol::Frame>,
        variables: Vec<VariableRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rows: Vec<FrameRow> = frames
            .iter()
            .map(|frame| FrameRow {
                name: frame.short_name(),
                place: match (&frame.path(), frame.line) {
                    (Some(path), line) => {
                        format!("{}:{}", path.file_name().unwrap_or_default().to_string_lossy(), line + 1)
                    }
                    _ => "external code".into(),
                },
                path: frame.path(),
                line: frame.line,
            })
            .collect();
        let at = at.or_else(|| frames.first().and_then(|frame| frame.path().map(|path| (path, frame.line))));
        self.debug.frames = frames;
        self.debug.phase = Phase::Paused;
        self.debug_view.update(cx, |view, cx| {
            view.stopped(&status, rows, cx);
            view.set_variables(variables, cx);
        });
        if let Some((path, line)) = at {
            self.show_execution_line(&path, line, cx);
            self.open_file(&path, Some((line, 0)), window, cx);
        }
        self.said = Some(status.into());
        cx.notify();
    }

    /// Opens a frame's source at its place: what clicking it in the Frames list does.
    pub(super) fn select_debug_frame(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(frame) = self.debug.frames.get(index).cloned() else { return };
        if let Some(path) = frame.path() {
            self.open_file(&path, Some((frame.line, 0)), window, cx);
        }
        self.debug_view.update(cx, |view, cx| view.select(index, cx));
        let Some(session) = self.debug.session.clone() else { return };
        let generation = self.debug.generation;
        let handle = self.handle;
        cx.spawn(async move |_, cx| {
            let read = cx.background_executor().spawn(async move { read_variables(&session, frame.id) }).await;
            let _ = handle.update(cx, |m, _, cx| {
                if m.debug.generation != generation {
                    return;
                }
                match read {
                    Ok(variables) => m.debug_view.update(cx, |view, cx| view.set_variables(variables, cx)),
                    Err(why) => m.fail_debug(&why, cx),
                }
            });
        })
        .detach();
    }

    /// A value's members, as its row opens in the Variables tree.
    pub(super) fn open_debug_variable(&mut self, path: Vec<usize>, reference: i64, cx: &mut Context<Self>) {
        let Some(session) = self.debug.session.clone() else { return };
        let generation = self.debug.generation;
        let handle = self.handle;
        cx.spawn(async move |_, cx| {
            let read = cx.background_executor().spawn(async move { session.variables(reference) }).await;
            let _ = handle.update(cx, |m, _, cx| {
                if m.debug.generation != generation {
                    return;
                }
                match read {
                    Ok(variables) => {
                        let variables = variables.into_iter().map(variable_row).collect();
                        m.debug_view.update(cx, |view, cx| view.set_children(&path, variables, cx));
                    }
                    Err(why) => m.fail_debug(&why, cx),
                }
            });
        })
        .detach();
    }

    /// A watch, asked as it is typed: the expression in the frame the program is stopped in.
    pub(super) fn evaluate_watch(&mut self, expression: String, cx: &mut Context<Self>) {
        let Some(frame) = self.debug.frames.first().map(|frame| frame.id) else {
            self.debug_view.update(cx, |view, cx| view.set_watch(&expression, "Not paused", cx));
            return;
        };
        let Some(session) = self.debug.session.clone() else { return };
        let generation = self.debug.generation;
        let handle = self.handle;
        let asked = expression.clone();
        cx.spawn(async move |_, cx| {
            let read = cx.background_executor().spawn(async move { session.evaluate(&asked, frame) }).await;
            let _ = handle.update(cx, |m, _, cx| {
                if m.debug.generation != generation {
                    return;
                }
                match read {
                    Ok(value) => {
                        let answer = match value.r#type {
                            Some(kind) => format!("{} ({kind})", value.result),
                            None => value.result,
                        };
                        m.debug_view.update(cx, |view, cx| view.set_watch(&expression, &answer, cx));
                    }
                    Err(why) => m.debug_view.update(cx, |view, cx| view.set_watch(&expression, &why, cx)),
                }
            });
        })
        .detach();
    }

    /// Resume: the paused program goes on from where it stopped.
    pub(super) fn resume_debug(&mut self, cx: &mut Context<Self>) {
        if self.debug.phase != Phase::Paused {
            return;
        }
        let Some(thread) = self.debug.thread else { return };
        self.debug.phase = Phase::Running;
        self.debug.frames.clear();
        self.clear_execution_line(cx);
        self.debug_view.update(cx, |view, cx| view.running(cx));
        self.said = Some("Running…".into());
        self.ask_session(cx, move |session| session.resume(thread), |_, (), _, _| {});
    }

    pub(super) fn pause_debug(&mut self, cx: &mut Context<Self>) {
        if self.debug.phase != Phase::Running {
            return;
        }
        let thread = self.debug.thread.unwrap_or(1);
        self.ask_session(cx, move |session| session.pause(thread), |_, (), _, _| {});
    }

    /// Step Over, Step Into or Step Out, as Rider's keys do them.
    pub(super) fn step_debug(&mut self, how: Step, cx: &mut Context<Self>) {
        if self.debug.phase != Phase::Paused {
            return;
        }
        let Some(thread) = self.debug.thread else { return };
        self.debug.phase = Phase::Running;
        self.debug.frames.clear();
        self.clear_execution_line(cx);
        self.debug_view.update(cx, |view, cx| view.running(cx));
        self.ask_session(
            cx,
            move |session| match how {
                Step::Over => session.step_over(thread),
                Step::Into => session.step_into(thread),
                Step::Out => session.step_out(thread),
            },
            |_, (), _, _| {},
        );
    }

    /// Runs a session request off the UI thread, and hands its answer back here when it arrives: a session that has ended
    /// meanwhile leaves the answer nowhere.
    fn ask_session<T, F, G>(&mut self, cx: &mut Context<Self>, work: F, then: G)
    where
        T: Send + 'static,
        F: FnOnce(Arc<Session>) -> Result<T, String> + Send + 'static,
        G: FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    {
        let Some(session) = self.debug.session.clone() else { return };
        let generation = self.debug.generation;
        let handle = self.handle;
        cx.spawn(async move |_, cx| {
            let answer = cx.background_executor().spawn(async move { work(session) }).await;
            let _ = handle.update(cx, |m, window, cx| {
                if m.debug.generation != generation {
                    return;
                }
                match answer {
                    Ok(value) => then(m, value, window, cx),
                    Err(why) => m.fail_debug(&why, cx),
                }
            });
        })
        .detach();
    }

    /// Stop: the session ends, and a program it launched with it.
    pub(super) fn stop_debug(&mut self, cx: &mut Context<Self>) {
        if !self.is_debugging() {
            return;
        }
        // An attached program is the person's, not this window's: Stop lets it go rather than ending it.
        if self.debug.attached.is_some() {
            self.detach_debug(cx);
            return;
        }
        self.debug.generation += 1;
        self.debug._task = None;
        self.debug.after_build = None;
        if let Some(session) = self.debug.session.take() {
            // The disconnect answers when it may, and its answer is waited for nowhere: the adapter ends with it.
            std::thread::spawn(move || session.disconnect(true));
        }
        let name = self.debug_name();
        self.finish_debug(&format!("Debugging {name} stopped"), cx);
    }

    /// The session ended or was stopped: the lists go, and the status bar says how.
    fn finish_debug(&mut self, message: &str, cx: &mut Context<Self>) {
        self.evaluate_dialog = None;
        self.debug.session = None;
        self.debug.attached = None;
        self.debug.phase = Phase::Idle;
        self.debug.frames.clear();
        self.debug.thread = None;
        self.debug.pending.clear();
        self.clear_execution_line(cx);
        self.debug_view.update(cx, |view, cx| view.finished(message, cx));
        self.debug_console.update(cx, |view, cx| {
            view.running = false;
            cx.notify();
        });
        self.said = Some(message.to_string().into());
        cx.notify();
    }

    /// Leaving the folder: the session ends and everything it started with it.
    pub(super) fn leave_debug(&mut self, cx: &mut Context<Self>) {
        self.evaluate_dialog = None;
        self.debug.generation += 1;
        self.debug._task = None;
        self.debug.after_build = None;
        if let Some(session) = self.debug.session.take() {
            let terminate = self.debug.attached.is_none();
            std::thread::spawn(move || session.disconnect(terminate));
        }
        self.debug.phase = Phase::Idle;
        self.debug.attached = None;
        self.debug.frames.clear();
        self.debug.thread = None;
        self.debug.pending.clear();
        self.debug.breakpoints = rsharp_core::debug::Breakpoints::default();
        self.debug.shown_at = None;
        self.debug.verification.clear();
        self.debug_view.update(cx, |view, cx| view.finished("Not debugging", cx));
        self.debug_console.update(cx, |view, cx| view.clear(cx));
    }

    fn refuse_debug(&mut self, message: &str, cx: &mut Context<Self>) {
        self.evaluate_dialog = None;
        self.debug.generation += 1;
        self.debug._task = None;
        self.debug.session = None;
        self.debug.attached = None;
        self.debug.phase = Phase::Idle;
        self.debug.after_build = None;
        self.debug.pending.clear();
        self.debug_console.update(cx, |view, cx| view.append(&[(format!("{message}\n"), true)], cx));
        self.debug_view.update(cx, |view, cx| view.finished(message, cx));
        self.said = Some(message.to_string().into());
        cx.notify();
    }

    fn fail_debug(&mut self, why: &str, cx: &mut Context<Self>) {
        self.said = Some(why.to_string().into());
        self.debug_view.update(cx, |view, cx| view.set_status(why, cx));
        cx.notify();
    }

    /// The folder's breakpoints as its document has them.
    pub(super) fn load_breakpoints(&mut self, folder: &Path, cx: &App) {
        self.debug.breakpoints = store(cx).read_for::<rsharp_core::debug::Breakpoints>(folder).resolved(folder);
        self.debug.recent_attached = store(cx).read_for::<rsharp_core::debug::DebugSessions>(folder).recent_attached;
    }

    /// F9 at the caret: a breakpoint set there, or the one there cleared.
    pub(super) fn toggle_breakpoint(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else { return };
        let (path, line) = {
            let editor = editor.read(cx);
            (editor.path.clone(), editor.caret().line)
        };
        self.toggle_breakpoint_at(&path, line, cx);
    }

    /// Sets or clears a breakpoint on a line, and tells a running session at once, as Rider does.
    pub(super) fn toggle_breakpoint_at(&mut self, path: &Path, line: usize, cx: &mut Context<Self>) {
        self.debug.breakpoints.toggle(path, line);
        self.save_breakpoints(cx);
        self.show_breakpoints(path, cx);
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        self.said = Some(if self.debug.breakpoints.has(path, line) {
            format!("Breakpoint set at {name}:{}", line + 1).into()
        } else {
            format!("Breakpoint cleared at {name}:{}", line + 1).into()
        });
        self.push_breakpoints(path.to_path_buf(), cx);
        cx.notify();
    }

    /// Sends a file's breakpoints to the session that runs, and keeps what the debugger made of each: a line set while
    /// the program runs binds or says why not, as Rider's breakpoint marks do.
    fn push_breakpoints(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(session) = self.debug.session.clone() else { return };
        let breakpoints: Vec<rsharp_core::debug::LineBreakpoint> = if self.debug.breakpoints.muted() {
            Vec::new()
        } else {
            self.debug.breakpoints.of(&path).into_iter().filter(|breakpoint| breakpoint.enabled).collect()
        };
        let sent: Vec<SourceBreakpoint> = breakpoints
            .iter()
            .map(|breakpoint| SourceBreakpoint {
                line: breakpoint.line,
                condition: filled(&breakpoint.condition),
                hit_condition: filled(&breakpoint.hit_count),
                log_message: filled(&breakpoint.log_message),
            })
            .collect();
        let lines: Vec<usize> = breakpoints.iter().map(|breakpoint| breakpoint.line).collect();
        let key = rsharp_core::paths::canonical(&path);
        let generation = self.debug.generation;
        let handle = self.handle;
        cx.spawn(async move |_, cx| {
            let answer = cx.background_executor().spawn(async move { session.set_breakpoints(&path, &sent) }).await;
            let _ = handle.update(cx, |m, _, cx| {
                if m.debug.generation != generation {
                    return;
                }
                match answer {
                    Ok(answers) => m.note_verification(&key, &lines, answers, cx),
                    Err(why) => m.fail_debug(&why, cx),
                }
            });
        })
        .detach();
    }

    /// The exception breakpoints as the session is told of them, muted with every other breakpoint.
    fn push_exceptions(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.debug.session.clone() else { return };
        let filters = self.enabled_exceptions();
        let generation = self.debug.generation;
        let handle = self.handle;
        cx.spawn(async move |_, cx| {
            let answer = cx.background_executor().spawn(async move { session.set_exceptions(&filters) }).await;
            let _ = handle.update(cx, |m, _, cx| {
                if m.debug.generation != generation {
                    return;
                }
                if let Err(why) = answer {
                    m.fail_debug(&why, cx);
                }
            });
        })
        .detach();
    }

    /// Every file's breakpoints and the exceptions, as the dialog's changes or Mute Breakpoints ask.
    fn push_all_breakpoints(&mut self, cx: &mut Context<Self>) {
        if self.debug.session.is_none() {
            return;
        }
        let mut paths: BTreeSet<PathBuf> = self.debug.breakpoints.files().into_iter().collect();
        paths.extend(self.debug.verification.keys().map(|(path, _)| path.clone()));
        for path in paths {
            self.push_breakpoints(path, cx);
        }
        self.push_exceptions(cx);
    }

    /// The exception breakpoints that are on, as Rider's "Suspend if" asks of each.
    fn enabled_exceptions(&self) -> Vec<rsharp_dap::ExceptionFilter> {
        if self.debug.breakpoints.muted() {
            return Vec::new();
        }
        let any = self.debug.breakpoints.any_exception();
        std::iter::once(any)
            .chain(self.debug.breakpoints.exceptions().iter())
            .filter(|exception| exception.enabled)
            .map(|exception| rsharp_dap::ExceptionFilter {
                types: exception.types.clone(),
                scenarios: exception.scenarios.clone(),
                condition: filled(&exception.condition),
                hit_condition: filled(&exception.hit_count),
                log_message: filled(&exception.log_message),
            })
            .collect()
    }

    /// What the adapter answered for a file's breakpoints, by line: whether each will be hit, and why not.
    fn note_verification(
        &mut self,
        path: &Path,
        lines: &[usize],
        answers: Vec<rsharp_dap::protocol::Breakpoint>,
        cx: &mut Context<Self>,
    ) {
        let key = rsharp_core::paths::canonical(path);
        for (line, answer) in lines.iter().zip(answers) {
            self.debug.verification.insert((key.clone(), *line), (answer.verified, answer.message));
        }
        self.show_breakpoints(path, cx);
    }

    /// Whether the debugger will hit a line's breakpoint, once a session has said; none before one has.
    pub fn breakpoint_verified(&self, path: &Path, line: usize) -> Option<bool> {
        self.debug.verification.get(&(rsharp_core::paths::canonical(path), line)).map(|(verified, _)| *verified)
    }

    /// Why the debugger will not hit it, as its own words say.
    pub fn breakpoint_message(&self, path: &Path, line: usize) -> Option<String> {
        self.debug
            .verification
            .get(&(rsharp_core::paths::canonical(path), line))
            .and_then(|(_, message)| message.clone())
    }

    /// Whether every breakpoint is muted.
    pub fn breakpoints_muted(&self) -> bool {
        self.debug.breakpoints.muted()
    }

    /// Mute Breakpoints: every breakpoint stays where it is, and the debugger is told of none.
    pub(super) fn set_breakpoints_muted(&mut self, muted: bool, cx: &mut Context<Self>) {
        self.debug.breakpoints.set_muted(muted);
        self.save_breakpoints(cx);
        self.refresh_breakpoint_marks(cx);
        self.push_all_breakpoints(cx);
        self.said = Some(if muted { "Breakpoints muted".into() } else { "Breakpoints on".into() });
        cx.notify();
    }

    /// The dialog's changes: kept with the folder, drawn in every open file, and sent to the session that runs.
    pub(super) fn apply_breakpoints(&mut self, breakpoints: rsharp_core::debug::Breakpoints, cx: &mut Context<Self>) {
        self.debug.breakpoints = breakpoints;
        self.save_breakpoints(cx);
        self.refresh_breakpoint_marks(cx);
        self.debug_view.update(cx, |view, cx| view.set_muted(self.debug.breakpoints.muted(), cx));
        self.push_all_breakpoints(cx);
        cx.notify();
    }

    /// Every open editor's breakpoint dots, as a change makes them.
    fn refresh_breakpoint_marks(&mut self, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = self.area.editors().map(|editor| editor.read(cx).path.clone()).collect();
        for path in paths {
            self.show_breakpoints(&path, cx);
        }
    }

    fn save_breakpoints(&self, cx: &App) {
        let Some(folder) = self.workspace.as_ref().map(|workspace| workspace.folder.clone()) else { return };
        store(cx).write_for(&folder, &self.debug.breakpoints.for_store(&folder));
    }

    /// A file's buffer shows its breakpoints, as its tab is opened: red where the debugger will hit it, a warning where
    /// it will not, dim where it is off or every breakpoint is muted.
    pub(super) fn show_breakpoints(&self, path: &Path, cx: &mut App) {
        let Some(buffer) = crate::buffer::find(&path.to_path_buf(), cx) else { return };
        let key = rsharp_core::paths::canonical(path);
        let muted = self.debug.breakpoints.muted();
        let debugging = self.is_debugging();
        let marks = self
            .debug
            .breakpoints
            .of(path)
            .into_iter()
            .map(|breakpoint| crate::buffer::BreakpointMark {
                line: breakpoint.line,
                verified: !debugging
                    || self
                        .debug
                        .verification
                        .get(&(key.clone(), breakpoint.line))
                        .is_none_or(|(verified, _)| *verified),
                enabled: breakpoint.enabled && !muted,
            })
            .collect();
        buffer.update(cx, |buffer, cx| buffer.set_breakpoints(marks, cx));
    }

    /// Marks the line the program is stopped at, clearing the line it was at.
    fn show_execution_line(&mut self, path: &Path, line: usize, cx: &mut App) {
        self.clear_execution_line(cx);
        if let Some(buffer) = crate::buffer::find(&path.to_path_buf(), cx) {
            buffer.update(cx, |buffer, cx| buffer.set_execution_line(Some(line), cx));
            self.debug.shown_at = Some(path.to_path_buf());
        }
    }

    fn clear_execution_line(&mut self, cx: &mut App) {
        if let Some(path) = self.debug.shown_at.take()
            && let Some(buffer) = crate::buffer::find(&path, cx)
        {
            buffer.update(cx, |buffer, cx| buffer.set_execution_line(None, cx));
        }
    }

    /// The Debug window's tools and lists, as the window itself asks for them.
    pub(super) fn debug_action(&mut self, action: DebugAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            DebugAction::Resume => self.resume_debug(cx),
            DebugAction::Pause => self.pause_debug(cx),
            DebugAction::Stop => self.stop_debug(cx),
            DebugAction::StepOver => self.step_debug(Step::Over, cx),
            DebugAction::StepInto => self.step_debug(Step::Into, cx),
            DebugAction::StepOut => self.step_debug(Step::Out, cx),
            DebugAction::SelectFrame(index) => self.select_debug_frame(index, window, cx),
            DebugAction::OpenVariable { path, reference } => self.open_debug_variable(path, reference, cx),
            DebugAction::Evaluate(expression) => self.evaluate_watch(expression, cx),
            DebugAction::ToggleMute => self.set_breakpoints_muted(!self.debug.breakpoints.muted(), cx),
        }
    }

    /// Attach to Process: the machine's .NET programs listed off the UI thread, then Rider's dialog over the window.
    pub(super) fn open_attach_dialog(&mut self, cx: &mut Context<Self>) {
        if self.attach_dialog.is_some() {
            return;
        }
        self.said = Some("Looking for .NET programs…".into());
        let handle = self.handle;
        let generation = self.debug.generation;
        let recent = self.debug.recent_attached.clone();
        cx.spawn(async move |_, cx| {
            let listed = cx.background_executor().spawn(async move { rsharp_core::debug::processes() }).await;
            let _ = handle.update(cx, |m, window, cx| {
                if m.debug.generation != generation {
                    return;
                }
                m.show_attach_dialog(listed, recent, window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn show_attach_dialog(
        &mut self,
        processes: Vec<rsharp_core::debug::DotnetProcess>,
        recent: Option<rsharp_core::debug::DotnetProcess>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = processes.len();
        let dialog = cx.new(|cx| crate::attach_dialog::AttachDialog::new(processes, recent, cx));
        cx.subscribe_in(&dialog, window, |m, _, ended: &crate::attach_dialog::Ended, window, cx| match ended {
            crate::attach_dialog::Ended::Chosen(process) => {
                m.attach_dialog = None;
                m.attach_to(process.clone(), cx);
            }
            crate::attach_dialog::Ended::Refresh => {
                m.attach_dialog = None;
                m.open_attach_dialog(cx);
            }
            crate::attach_dialog::Ended::Dismissed => {
                m.attach_dialog = None;
                m.focus_editor(window, cx);
            }
        })
        .detach();
        window.focus(&dialog.focus_handle(cx));
        self.attach_dialog = Some(dialog);
        self.said = Some(
            if count == 0 {
                "No .NET program to attach to: start one, then Refresh".to_string()
            } else {
                format!("{count} .NET program(s) can be attached to")
            }
            .into(),
        );
        cx.notify();
    }

    /// The Evaluate Expression dialog while it is open, for a test to drive.
    pub fn evaluate_dialog(&self) -> Option<Entity<crate::evaluate_dialog::EvaluateDialog>> {
        self.evaluate_dialog.clone()
    }

    /// The Attach to Process dialog while it is open, for a test to drive.
    pub fn attach_dialog(&self) -> Option<Entity<crate::attach_dialog::AttachDialog>> {
        self.attach_dialog.clone()
    }

    /// View Breakpoints (Cmd+Shift+F8): Rider's dialog over the window, every change applied as it is made.
    pub(super) fn open_breakpoints(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.breakpoints_dialog.is_some() {
            return;
        }
        let verification: Vec<(PathBuf, usize, bool, Option<String>)> = self
            .debug
            .verification
            .iter()
            .map(|((path, line), (verified, message))| (path.clone(), *line, *verified, message.clone()))
            .collect();
        let dialog = cx.new(|cx| {
            crate::breakpoints_dialog::BreakpointsDialog::new(self.debug.breakpoints.clone(), verification, cx)
        });
        cx.subscribe_in(&dialog, window, |m, _, ended: &crate::breakpoints_dialog::Ended, window, cx| match ended {
            crate::breakpoints_dialog::Ended::Changed(breakpoints) => m.apply_breakpoints(breakpoints.clone(), cx),
            crate::breakpoints_dialog::Ended::Dismissed => {
                m.breakpoints_dialog = None;
                m.focus_editor(window, cx);
            }
        })
        .detach();
        window.focus(&dialog.focus_handle(cx));
        self.breakpoints_dialog = Some(dialog);
        self.debug_view.update(cx, |view, cx| view.set_muted(self.debug.breakpoints.muted(), cx));
        cx.notify();
    }

    /// The dialog open, for a test to drive.
    pub fn breakpoints_dialog(&self) -> Option<Entity<crate::breakpoints_dialog::BreakpointsDialog>> {
        self.breakpoints_dialog.clone()
    }

    /// The actions debugging offers, each only while it can act, as every action is.
    pub(super) fn debug_actions<T: InteractiveElement + FluentBuilder>(&self, d: T, cx: &mut Context<Self>) -> T {
        d.when(self.workspace.is_some(), |d| {
            d.on_action(cx.listener(|m, _: &ToggleDebug, window, cx| m.toggle_tool(Tool::Debug, window, cx)))
                .on_action(cx.listener(|m, _: &ToggleBreakpoint, _, cx| m.toggle_breakpoint(cx)))
                .on_action(cx.listener(|m, _: &ShowBreakpoints, window, cx| m.open_breakpoints(window, cx)))
                .on_action(cx.listener(|m, _: &AttachToProcess, _, cx| m.open_attach_dialog(cx)))
                .on_action(cx.listener(|m, _: &ReattachToProcess, _, cx| m.reattach(cx)))
                .when(self.debug.attached.is_some(), |d| {
                    d.on_action(cx.listener(|m, _: &DetachDebug, _, cx| m.detach_debug(cx)))
                })
                .on_action(cx.listener(|m, _: &MuteBreakpoints, _, cx| {
                    let muted = m.debug.breakpoints.muted();
                    m.set_breakpoints_muted(!muted, cx)
                }))
                .when(!self.is_debugging(), |d| {
                    d.on_action(cx.listener(|m, _: &DebugProgram, _, cx| m.debug_program(cx)))
                })
                .when(self.debug.phase == Phase::Paused, |d| {
                    d.on_action(cx.listener(|m, _: &ResumeDebug, _, cx| m.resume_debug(cx)))
                        .on_action(cx.listener(|m, _: &EvaluateExpression, window, cx| m.open_evaluate(window, cx)))
                        .on_action(cx.listener(|m, _: &SetNextStatement, _, cx| m.set_next_statement(cx)))
                        .on_action(cx.listener(|m, _: &StepOver, _, cx| m.step_debug(Step::Over, cx)))
                        .on_action(cx.listener(|m, _: &StepInto, _, cx| m.step_debug(Step::Into, cx)))
                        .on_action(cx.listener(|m, _: &StepOut, _, cx| m.step_debug(Step::Out, cx)))
                })
                .when(self.debug.phase == Phase::Running, |d| {
                    d.on_action(cx.listener(|m, _: &PauseDebug, _, cx| m.pause_debug(cx)))
                })
                .when(self.is_debugging(), |d| d.on_action(cx.listener(|m, _: &StopDebug, _, cx| m.stop_debug(cx))))
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Step {
    Over,
    Into,
    Out,
}

/// A breakpoint's field as the protocol takes it: none when the field is blank.
fn filled(text: &str) -> Option<String> {
    (!text.trim().is_empty()).then(|| text.to_string())
}

fn variable_row(variable: rsharp_dap::protocol::Variable) -> VariableRow {
    VariableRow {
        name: variable.name,
        value: variable.value,
        r#type: variable.r#type,
        reference: variable.variables_reference,
    }
}

/// A stop's frames and the innermost frame's values, read on a thread that may wait.
fn read_stop(session: &Session, thread: i64) -> Result<(Vec<rsharp_dap::protocol::Frame>, Vec<VariableRow>), String> {
    let frames = session.stack_trace(thread)?;
    let variables = match frames.first() {
        Some(frame) => read_variables(session, frame.id)?,
        None => Vec::new(),
    };
    Ok((frames, variables))
}

fn read_variables(session: &Session, frame: i64) -> Result<Vec<VariableRow>, String> {
    let scopes = session.scopes(frame)?;
    let Some(scope) = scopes.iter().find(|scope| scope.name == "Locals").or_else(|| scopes.first()) else {
        return Ok(Vec::new());
    };
    Ok(session.variables(scope.variables_reference)?.into_iter().map(variable_row).collect())
}
