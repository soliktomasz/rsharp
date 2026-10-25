//! One workspace's debugging: the folder's breakpoints, the plan a Debug works out, the session in `rsharp-dap`, and what
//! the Debug window shows of it. Every request waits off the UI thread; what the adapter and the program say lands here.

use super::*;
use crate::debug_view::{DebugAction, FrameRow, Phase, VariableRow};
use crate::state::store;
use futures::channel::mpsc;
use futures::{FutureExt, StreamExt};
use rsharp_core::run::LaunchPlan;
use rsharp_dap::{Event as Said, Session, SourceBreakpoint, Start};

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
        self.debug_view.update(cx, |view, cx| view.starting(&configuration.name, cx));
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
                self.debug.session = Some(session);
                self.debug.phase = Phase::Running;
                self.debug_view.update(cx, |view, cx| view.running(cx));
                self.said = Some(format!("Debugging {}…", self.debug_name()).into());
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
        self.debug.plan.as_ref().map(|plan| plan.name.clone()).unwrap_or_default()
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
        let breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)> = self
            .debug
            .breakpoints
            .resolved(&folder)
            .enabled()
            .into_iter()
            .map(|(path, lines)| {
                let lines = lines
                    .into_iter()
                    .map(|line| SourceBreakpoint {
                        line: line.line,
                        condition: (!line.condition.trim().is_empty()).then(|| line.condition.clone()),
                        log_message: (!line.log_message.trim().is_empty()).then(|| line.log_message.clone()),
                    })
                    .collect();
                (path, lines)
            })
            .collect();
        let symbols = store(cx).root().join("cache").join("symbols");
        let (sender, mut receiver) = mpsc::unbounded();
        let generation = self.debug.generation;
        std::thread::spawn(move || {
            let reports = sender.clone();
            let result = Session::start(&adapter, Start::Launch(launch), &symbols, breakpoints, move |said| {
                let _ = reports.unbounded_send(Event::Said(said));
            });
            let event = match result {
                Ok(session) => Event::Started(Arc::new(session)),
                Err(why) => Event::Failed(why),
            };
            let _ = sender.unbounded_send(event);
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
        self.said = Some(format!("Starting {} under the debugger…", plan.name).into());
        cx.notify();
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
        self.ask_session(cx, move |session| session.resume(thread), |_, (), _| {});
    }

    pub(super) fn pause_debug(&mut self, cx: &mut Context<Self>) {
        if self.debug.phase != Phase::Running {
            return;
        }
        let thread = self.debug.thread.unwrap_or(1);
        self.ask_session(cx, move |session| session.pause(thread), |_, (), _| {});
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
            |_, (), _| {},
        );
    }

    /// Runs a session request off the UI thread, and hands its answer back here when it arrives: a session that has ended
    /// meanwhile leaves the answer nowhere.
    fn ask_session<T, F, G>(&mut self, cx: &mut Context<Self>, work: F, then: G)
    where
        T: Send + 'static,
        F: FnOnce(Arc<Session>) -> Result<T, String> + Send + 'static,
        G: FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    {
        let Some(session) = self.debug.session.clone() else { return };
        let generation = self.debug.generation;
        let handle = self.handle;
        cx.spawn(async move |_, cx| {
            let answer = cx.background_executor().spawn(async move { work(session) }).await;
            let _ = handle.update(cx, |m, _, cx| {
                if m.debug.generation != generation {
                    return;
                }
                match answer {
                    Ok(value) => then(m, value, cx),
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
        self.debug.session = None;
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
        self.debug.generation += 1;
        self.debug._task = None;
        self.debug.after_build = None;
        if let Some(session) = self.debug.session.take() {
            std::thread::spawn(move || session.disconnect(true));
        }
        self.debug.phase = Phase::Idle;
        self.debug.frames.clear();
        self.debug.thread = None;
        self.debug.pending.clear();
        self.debug.breakpoints = rsharp_core::debug::Breakpoints::default();
        self.debug.shown_at = None;
        self.debug_view.update(cx, |view, cx| view.finished("Not debugging", cx));
        self.debug_console.update(cx, |view, cx| view.clear(cx));
    }

    fn refuse_debug(&mut self, message: &str, cx: &mut Context<Self>) {
        self.debug.generation += 1;
        self.debug._task = None;
        self.debug.session = None;
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
        if let Some(session) = self.debug.session.clone() {
            let lines = self.debug.breakpoints.lines(path);
            let sent: Vec<SourceBreakpoint> =
                lines.iter().map(|line| SourceBreakpoint { line: *line, ..SourceBreakpoint::default() }).collect();
            let path = path.to_path_buf();
            std::thread::spawn(move || {
                if let Err(why) = session.set_breakpoints(&path, &sent) {
                    log::warn!("Could not send {path:?}'s breakpoints: {why}");
                }
            });
        }
        cx.notify();
    }

    fn save_breakpoints(&self, cx: &App) {
        let Some(folder) = self.workspace.as_ref().map(|workspace| workspace.folder.clone()) else { return };
        store(cx).write_for(&folder, &self.debug.breakpoints.for_store(&folder));
    }

    /// A file's buffer shows its breakpoints, as its tab is opened.
    pub(super) fn show_breakpoints(&self, path: &Path, cx: &mut App) {
        let Some(buffer) = crate::buffer::find(&path.to_path_buf(), cx) else { return };
        let lines = self.debug.breakpoints.lines(path);
        buffer.update(cx, |buffer, cx| buffer.set_breakpoints(lines, cx));
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
        }
    }

    /// The actions debugging offers, each only while it can act, as every action is.
    pub(super) fn debug_actions<T: InteractiveElement + FluentBuilder>(&self, d: T, cx: &mut Context<Self>) -> T {
        d.when(self.workspace.is_some(), |d| {
            d.on_action(cx.listener(|m, _: &ToggleDebug, window, cx| m.toggle_tool(Tool::Debug, window, cx)))
                .on_action(cx.listener(|m, _: &ToggleBreakpoint, _, cx| m.toggle_breakpoint(cx)))
                .when(!self.is_debugging(), |d| {
                    d.on_action(cx.listener(|m, _: &DebugProgram, _, cx| m.debug_program(cx)))
                })
                .when(self.debug.phase == Phase::Paused, |d| {
                    d.on_action(cx.listener(|m, _: &ResumeDebug, _, cx| m.resume_debug(cx)))
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
