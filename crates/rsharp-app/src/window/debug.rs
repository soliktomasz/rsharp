//! One workspace's debugging: the folder's breakpoints, the plan a Debug works out, the session in `rsharp-dap`, and what
//! the Debug window shows of it. Every request waits off the UI thread; what the adapter and the program say lands here.

use super::*;
use crate::debug_view::{DebugAction, FrameRow, Phase, VariableRow};
use crate::state::store;
use futures::channel::mpsc;
use futures::{FutureExt, StreamExt};
use gpui::Focusable;
use rsharp_core::run::LaunchPlan;
use rsharp_dap::{Event as Said, Session, SourceBreakpoint, Start};
use rsharp_lsp::extension;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
mod tests;

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
    /// The paused program's threads, as last read.
    threads: Vec<rsharp_dap::protocol::Thread>,
    /// Counted at every stop's end, so an answer asked of one stop is told from the next's, whose frame ids may repeat.
    stop: u64,
    /// The file the program is stopped in, whose buffer shows the execution line.
    shown_at: Option<PathBuf>,
    /// What the debugger made of each breakpoint: whether it will be hit, and why not.
    verification: BTreeMap<(PathBuf, usize), (bool, Option<String>)>,
    /// The calls Smart Step Into offered for the frame the program is stopped in.
    step_targets: Vec<rsharp_dap::protocol::StepInTarget>,
    /// The person said Never to restarting for a rude edit in this session: it is not asked again until the next one.
    never_restart: bool,
    /// The program attached to, while one is; none for a launch.
    attached: Option<rsharp_core::debug::DotnetProcess>,
    /// The program attached to last in this folder, which Reattach asks for.
    recent_attached: Option<rsharp_core::debug::DotnetProcess>,
    /// The Terminal tab the program runs in, when the configuration starts it there.
    terminal: Option<usize>,
    /// Waits for that tab's program to start, to answer the adapter with its process.
    terminal_wait: Option<gpui::Subscription>,
    /// What the adapter said while the session was still starting.
    pending: Vec<Said>,
    /// The session while it waits for its program's runtime, so Stop can end it from its first moment, as Rider's can.
    starting: Option<Arc<rsharp_dap::Starting>>,
    /// Hot Reload's Edit and Continue session, for a program this window launched.
    hot_reload: Option<HotReload>,
    /// The step or resume waiting for the edits made while paused to reach the program.
    after_apply: Option<AfterApply>,
    /// The session's data breakpoints, watched for a write that changes the value: never kept with the folder, as
    /// the objects they watch live in the program alone.
    data_breakpoints: Vec<rsharp_dap::DataBreakpoint>,
    /// What the debugger made of each data breakpoint, in their order: whether it watches, and why not.
    data_bound: Vec<(bool, Option<String>)>,
    /// Console input reaches the adapter in the order it was entered.
    input_task: Option<Task<()>>,
    _task: Option<Task<()>>,
}

/// Hot Reload's Edit and Continue session for the program the debugger launched: Roslyn's session number, each C# file's
/// text from before its first edit (what the program was built from), and why the runtime stopped taking changes.
#[derive(Default)]
struct HotReload {
    /// Roslyn's session, once it has started: most sessions never edit a file, so it starts at the first save.
    session: Option<Arc<HotReloadSession>>,
    /// Each C# file's text before its first edit since the launch, as it was built.
    originals: BTreeMap<PathBuf, String>,
    /// The files whose original texts Roslyn has been given.
    given: BTreeSet<PathBuf>,
    /// Each file's version whose text the program runs: a later save is an edit to apply, an earlier one's undo too.
    applied: BTreeMap<PathBuf, u64>,
    /// A save's update is on its way to Roslyn and the debugger: a save meanwhile waits for the next one.
    applying: bool,
    /// A save or resume during the current apply needs another pass before the program goes on.
    pending: bool,
    /// Why the runtime refused an update, which stops Hot Reload for the rest of the session.
    stopped: Option<String>,
}

/// The window and an in-flight update share ownership; the last one ends Roslyn's session, even after Stop.
struct HotReloadSession {
    number: i64,
    server: Arc<LanguageServer>,
}

impl Drop for HotReloadSession {
    fn drop(&mut self) {
        let number = self.number;
        let server = self.server.clone();
        std::thread::spawn(move || {
            let (method, params) =
                extension::workspace_request(extension::END_HOT_RELOAD, &json!({ "session": number }));
            let _ = server.request(method, params);
        });
    }
}

impl HotReload {
    fn has_changes(&self, buffers: &[(PathBuf, u64, String)]) -> bool {
        self.pending
            || buffers.iter().any(|(path, version, text)| match (self.applied.get(path), self.originals.get(path)) {
                (Some(applied), _) => applied != version,
                (None, Some(original)) => original != text,
                (None, None) => false,
            })
    }

    /// The original texts Roslyn has not been given yet, in a stable order, as the extension takes them.
    fn ungiven(&self) -> Vec<(PathBuf, String)> {
        self.originals
            .iter()
            .filter(|(path, _)| !self.given.contains(*path))
            .map(|(path, text)| (path.clone(), text.clone()))
            .collect()
    }
}

/// What a save's Hot Reload came to, for the window to show and keep.
struct HotReloadOutcome {
    /// The session and the server it lives on, once it has started.
    session: Option<Arc<HotReloadSession>>,
    /// The original texts Roslyn now has.
    given: Vec<PathBuf>,
    /// What the Debug console says, each line with whether it is a failure.
    lines: Vec<(String, bool)>,
    /// The status bar's word, and the Debug window's.
    said: String,
    /// Whether the program's text is now what Roslyn committed, or nothing needed applying.
    committed: bool,
    /// The breakpoints sent again in the code the update replaced, with their lines and the debugger's answers.
    verified: Vec<(PathBuf, Vec<usize>, Vec<rsharp_dap::protocol::Breakpoint>)>,
    /// A paused stop read again after the update, as Passenger does: the threads, frames and values where it is now.
    refreshed: Option<(Vec<rsharp_dap::protocol::Thread>, Vec<rsharp_dap::protocol::Frame>, Vec<VariableRow>)>,
    /// Whether restarting the program would run the edits: Roslyn's rude edits, with nothing to compile.
    restart: bool,
    /// Why Hot Reload stops for the session: the runtime refused an update part way.
    stopped: Option<String>,
}

impl HotReloadOutcome {
    fn failed(said: String) -> Self {
        Self {
            session: None,
            given: Vec::new(),
            lines: vec![(format!("{said}\n"), true)],
            said,
            committed: false,
            verified: Vec::new(),
            refreshed: None,
            restart: false,
            stopped: None,
        }
    }

    /// The session it started on, kept even when the update that followed failed.
    fn session(mut self, session: Arc<HotReloadSession>) -> Self {
        self.session = Some(session);
        self
    }

    /// The original texts Roslyn has taken.
    fn given(mut self, given: Vec<PathBuf>) -> Self {
        self.given = given;
        self
    }
}

impl Drop for DebugState {
    fn drop(&mut self) {
        // The window is going: a program it launched ends with it, one it attached to is let go, as Stop does.
        if let Some(session) = self.session.take() {
            let terminate = self.attached.is_none();
            std::thread::spawn(move || session.disconnect(terminate));
        } else if let Some(starting) = self.starting.take() {
            let attached = self.attached.is_some();
            std::thread::spawn(move || if attached { starting.let_go() } else { starting.stop() });
        }
    }
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
            threads: Vec::new(),
            stop: 0,
            shown_at: None,
            verification: BTreeMap::new(),
            step_targets: Vec::new(),
            never_restart: false,
            attached: None,
            recent_attached: None,
            terminal: None,
            terminal_wait: None,
            pending: Vec::new(),
            starting: None,
            hot_reload: None,
            after_apply: None,
            data_breakpoints: Vec::new(),
            data_bound: Vec::new(),
            input_task: None,
            _task: None,
        }
    }
}

enum Event {
    /// The plan a Debug works out, or why it cannot run.
    Resolved(Result<Box<LaunchPlan>, String>),
    /// The adapter is up and the program is being started: its handle until the runtime answers.
    Starting(Arc<rsharp_dap::Starting>),
    Started(Arc<Session>),
    Failed(String),
    Said(Said),
    /// The adapter asks for its program in a terminal of the window's own; the answer is its process id.
    RunInTerminal {
        request: rsharp_dap::RunInTerminal,
        reply: std::sync::mpsc::Sender<Result<rsharp_dap::RunInTerminalAnswer, String>>,
    },
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

    /// The adapter's own process, for a test that ends it as a crash would.
    pub fn debug_adapter_process(&self) -> Option<u32> {
        match &self.debug.session {
            Some(session) => session.adapter_process_id(),
            None => self.debug.starting.as_ref().and_then(|starting| starting.adapter_process_id()),
        }
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
        self.debug_configuration(configuration, cx);
    }

    /// Debug: this configuration's program is built, its plan worked out, and it is started under the debugger with the
    /// folder's breakpoints, as Rider's F5 starts it.
    fn debug_configuration(&mut self, configuration: rsharp_core::run::RunConfiguration, cx: &mut Context<Self>) {
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
        // Hot Reload's baseline: every open C# file's text as the launch's save left it, since the build is about to read
        // the same text from disk.
        self.debug.hot_reload = Some(HotReload::default());
        self.debug.after_apply = None;
        self.debug.never_restart = false;
        self.choice_dialog = None;
        self.capture_hot_reload_originals(cx);
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
            Event::Starting(starting) => self.debug.starting = Some(starting),
            Event::Started(session) => {
                self.debug.starting = None;
                let verified = session.verified();
                self.debug.session = Some(session);
                // The adapter answers the breakpoints it was sent, in their order: the lines that were sent say where
                // each answer belongs, as an answer need not name one. Muted, none was sent, so none is verified.
                let sent: BTreeMap<PathBuf, Vec<usize>> = match self.workspace.as_ref() {
                    Some(workspace) if !self.debug.breakpoints.muted() => self
                        .session_breakpoints(&workspace.folder)
                        .into_iter()
                        .map(|(path, breakpoints)| {
                            let lines = breakpoints.into_iter().map(|breakpoint| breakpoint.line).collect();
                            (rsharp_core::paths::canonical(&path), lines)
                        })
                        .collect(),
                    _ => BTreeMap::new(),
                };
                for (path, answers) in verified {
                    let lines = sent.get(&rsharp_core::paths::canonical(&path)).cloned().unwrap_or_default();
                    self.note_verification(&path, &lines, answers, cx);
                }
                self.debug.phase = Phase::Running;
                self.debug_view.update(cx, |view, cx| view.running(cx));
                self.debug_console.update(cx, |view, cx| view.set_input(true, cx));
                self.push_data_breakpoints(cx);
                self.show_data_breakpoints(cx);
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
            Event::Failed(why) => {
                let name = self.debug_name();
                self.refuse_debug(&format!("Could not debug {name}: {why}"), cx)
            }
            Event::Said(said) => self.session_said(said, window, cx),
            Event::RunInTerminal { request, reply } => self.run_in_terminal(request, reply, cx),
        }
    }

    /// The adapter's runInTerminal: the program it asks for starts in a Terminal tab of its own, and the id of that
    /// program answers it. The session waits for the answer off the UI thread; Stop ends the program with the session.
    fn run_in_terminal(
        &mut self,
        request: rsharp_dap::RunInTerminal,
        reply: std::sync::mpsc::Sender<Result<rsharp_dap::RunInTerminalAnswer, String>>,
        cx: &mut Context<Self>,
    ) {
        let Some((program, arguments)) = request.args.split_first() else {
            let _ = reply.send(Err("The debugger asked for a terminal with no program to start".into()));
            return;
        };
        let folder = request
            .cwd
            .clone()
            .or_else(|| self.workspace.as_ref().map(|workspace| workspace.folder.clone()))
            .unwrap_or_else(rsharp_core::programs::home);
        let mut command = rsharp_terminal::CommandBuilder::new(program);
        command.args(arguments);
        command.cwd(&folder);
        if let Some(environment) = &request.env {
            for (key, value) in environment {
                command.env(key.as_str(), value.as_str());
            }
        }
        let title = if request.title.is_empty() { self.debug_name() } else { request.title.clone() };
        let name = if title.is_empty() { "Debug".to_string() } else { title };
        let id = self.terminal.update(cx, |panel, cx| panel.start(name, command, folder, true, cx));
        self.dock.show(Tool::Terminal);
        self.debug.terminal = Some(id);
        self.said = Some("The program runs in the terminal".into());
        // Answered the moment the tab has its program: a timer polling for it fell minutes behind on a busy Linux runner,
        // past the adapter's 30 s. That limit still ends the wait for a tab that never starts.
        let mut reply = Some(reply);
        self.debug.terminal_wait = Some(cx.observe(&self.terminal.clone(), move |_, panel, cx| {
            let answer = match panel.read(cx).problem(id) {
                Some(problem) => Err(problem.to_string()),
                None => match panel.read(cx).session(id) {
                    Some(session) => Ok(rsharp_dap::RunInTerminalAnswer { process_id: session.pid }),
                    None => return,
                },
            };
            if let Some(reply) = reply.take() {
                let _ = reply.send(answer);
            }
        }));
        cx.notify();
    }

    /// Stops the program a debug session started in a terminal, leaving its tab and what it wrote.
    fn stop_debug_terminal(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.debug.terminal.take() {
            self.terminal.update(cx, |panel, _| {
                if let Some(session) = panel.session(id) {
                    session.stop();
                }
            });
        }
    }

    /// Whether the program runs in a Terminal tab, and which, for a test.
    pub fn debug_terminal(&self) -> Option<usize> {
        self.debug.terminal
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
            let read = cx
                .background_executor()
                .spawn(async move {
                    let value = session.evaluate(&expression, frame)?;
                    let members = if value.variables_reference == 0 {
                        Vec::new()
                    } else {
                        session
                            .variables(value.variables_reference)
                            .unwrap_or_default()
                            .into_iter()
                            .map(|variable| (variable.name, variable.value, variable.r#type))
                            .collect()
                    };
                    Ok((value, members))
                })
                .await;
            let _ = handle.update(cx, |m, _, cx| {
                if m.debug.generation != generation || m.evaluate_dialog.as_ref() != Some(&dialog) {
                    return;
                }
                match read {
                    Ok((value, members)) => {
                        let result = crate::debug_view::shown(&value.result).into_owned();
                        let answer = match value.r#type {
                            Some(kind) => format!("{result} ({kind})"),
                            None => result,
                        };
                        dialog.update(cx, |dialog, cx| {
                            dialog.set_result(answer, false, cx);
                            dialog.set_members(members, cx);
                        });
                    }
                    Err(why) => dialog.update(cx, |dialog, cx| dialog.set_result(why, true, cx)),
                }
            });
        })
        .detach();
    }

    /// Set Next Statement: the paused thread goes on from the caret's line, where the debugger allows it.
    pub(super) fn set_next_statement(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else { return };
        let (path, line) = {
            let editor = editor.read(cx);
            (editor.path.clone(), editor.caret().line)
        };
        self.set_next_statement_at(path, line, cx);
    }

    /// The paused thread goes on from a line, as Set Next Statement and a drag of the execution arrow ask.
    pub(super) fn set_next_statement_at(&mut self, path: PathBuf, line: usize, cx: &mut Context<Self>) {
        if self.debug.phase != Phase::Paused {
            return;
        }
        let Some(thread) = self.debug.thread else { return };
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
            |m, (threads, frames, variables), window, cx| {
                m.show_stopped(
                    "Paused where Set Next Statement put it".into(),
                    None,
                    None,
                    threads,
                    frames,
                    variables,
                    window,
                    cx,
                );
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
            hot_reload: true,
            just_my_code: true,
        };
        let breakpoints = self.session_breakpoints(&folder);
        let functions = self.session_function_breakpoints();
        let exceptions = self.enabled_exceptions();
        let symbols = store(cx).root().join("cache").join("symbols");
        let (sender, receiver) = mpsc::unbounded();
        // A configuration that runs its program in a terminal: the adapter asks for it, and this window starts it.
        let terminal = plan.use_terminal.then(|| {
            let events = sender.clone();
            std::sync::Arc::new(move |request: rsharp_dap::RunInTerminal| {
                let (reply, answer) = std::sync::mpsc::channel();
                if events.unbounded_send(Event::RunInTerminal { request, reply }).is_err() {
                    return Err("The debug session has ended".to_string());
                }
                answer
                    .recv_timeout(std::time::Duration::from_secs(30))
                    .unwrap_or_else(|_| Err("The terminal did not start within 30 seconds".into()))
            }) as rsharp_dap::TerminalRun
        });
        let generation = self.debug.generation;
        std::thread::spawn(move || {
            let reports = sender.clone();
            let watching = sender.clone();
            let result = Session::start_watched(
                &adapter,
                Start::Launch(launch),
                &symbols,
                breakpoints,
                functions,
                exceptions,
                Vec::new(),
                terminal,
                move |said| {
                    let _ = reports.unbounded_send(Event::Said(said));
                },
                move |starting| {
                    let _ = watching.unbounded_send(Event::Starting(Arc::new(starting)));
                },
            );
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
                        column: line.column,
                        condition: filled(&line.condition),
                        hit_condition: filled(&line.hit_count),
                        log_message: filled(&line.log_message),
                    })
                    .collect();
                (path, lines)
            })
            .collect()
    }

    /// The folder's function breakpoints as a session is told of them: only the named ones that are on.
    fn session_function_breakpoints(&self) -> Vec<rsharp_dap::FunctionBreakpoint> {
        self.debug
            .breakpoints
            .enabled_functions()
            .into_iter()
            .map(|breakpoint| rsharp_dap::FunctionBreakpoint {
                name: breakpoint.name.trim().to_string(),
                condition: filled(&breakpoint.condition),
                hit_condition: filled(&breakpoint.hit_count),
                log_message: filled(&breakpoint.log_message),
            })
            .collect()
    }

    /// Sends the function breakpoints to the session that runs, as the dialog's changes ask.
    fn push_function_breakpoints(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.debug.session.clone() else { return };
        let functions = self.session_function_breakpoints();
        let generation = self.debug.generation;
        let handle = self.handle;
        cx.spawn(async move |_, cx| {
            let answer =
                cx.background_executor().spawn(async move { session.set_function_breakpoints(&functions) }).await;
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
        // An attached program was not built by this window's edits: Hot Reload applies to one it launched.
        self.end_hot_reload();
        self.debug.after_apply = None;
        self.debug.never_restart = false;
        self.choice_dialog = None;
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
        let functions = self.session_function_breakpoints();
        let exceptions = self.enabled_exceptions();
        let symbols = store(cx).root().join("cache").join("symbols");
        let id = process.id;
        let (sender, receiver) = mpsc::unbounded();
        std::thread::spawn(move || {
            let reports = sender.clone();
            let watching = sender.clone();
            let result = Session::start_watched(
                &adapter,
                Start::Attach { process_id: id, just_my_code: true },
                &symbols,
                breakpoints,
                functions,
                exceptions,
                Vec::new(),
                None,
                move |said| {
                    let _ = reports.unbounded_send(Event::Said(said));
                },
                move |starting| {
                    let _ = watching.unbounded_send(Event::Starting(Arc::new(starting)));
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

    /// Each open C# file's text as the launch's save left it: what Roslyn needs as a file's original once an edit makes
    /// its text no longer match the build's PDB.
    fn capture_hot_reload_originals(&mut self, cx: &mut Context<Self>) {
        let texts: Vec<(PathBuf, String)> = self
            .workspace_buffers(cx)
            .into_iter()
            .filter_map(|buffer| {
                let buffer = buffer.read(cx);
                is_csharp(&buffer.path)
                    .then(|| (rsharp_core::paths::canonical(&buffer.path), buffer.document.contents()))
            })
            .collect();
        if let Some(hot) = &mut self.debug.hot_reload {
            hot.originals.extend(texts);
        }
    }

    /// A file shown while debugging: its text as the disk has it is what the program was built from, until it is edited.
    pub(super) fn note_hot_reload_original(&mut self, buffer: &Entity<Buffer>, cx: &App) {
        let Some(hot) = &mut self.debug.hot_reload else { return };
        let buffer = buffer.read(cx);
        if !is_csharp(&buffer.path) {
            return;
        }
        hot.originals.entry(rsharp_core::paths::canonical(&buffer.path)).or_insert_with(|| buffer.document.contents());
    }

    /// The debugging session is ending: Roslyn's Edit and Continue session ends with it, as Passenger's does.
    fn end_hot_reload(&mut self) {
        self.debug.hot_reload = None;
    }

    /// A save made while the program is debugged: the file's edits go to the program, as Rider's Apply Hot Reload
    /// Changes does, unless nothing it runs has changed.
    pub(super) fn hot_reload_saved(&mut self, editor: &Entity<CodeEditor>, cx: &mut Context<Self>) {
        if !is_csharp(&editor.read(cx).path) {
            return;
        }
        self.hot_reload_apply(None, cx);
    }

    /// Rider's Apply Hot Reload Changes for the debugged program: the text the editors hold, saved or not, goes to
    /// Roslyn's Edit and Continue. `after` is the step or resume that goes on once the update lands. What came of it:
    /// no update when nothing changed, the program cannot take one, or only a restart takes the changes.
    pub(super) fn hot_reload_apply(&mut self, after: Option<AfterApply>, cx: &mut Context<Self>) -> HotReloadApply {
        if self.debug.attached.is_some() || !matches!(self.debug.phase, Phase::Running | Phase::Paused) {
            return HotReloadApply::Unavailable;
        }
        let applying = self.debug.hot_reload.as_ref().is_some_and(|hot| hot.applying);
        let stopped = self.debug.hot_reload.as_ref().is_some_and(|hot| hot.stopped.is_some());
        if stopped {
            return HotReloadApply::Unavailable;
        }
        // Coalesce saves and keep a waiting step behind the newest text, rather than only the update already on its way.
        if applying {
            if let Some(hot) = &mut self.debug.hot_reload {
                hot.pending = true;
            }
            if after.is_some() {
                self.debug.after_apply = after;
            }
            return HotReloadApply::Started;
        }
        let Some(session) = self.debug.session.clone() else { return HotReloadApply::Unavailable };
        let Some(server) = self.server.clone() else { return HotReloadApply::Unavailable };
        let Some(folder) = self.workspace.as_ref().map(|workspace| workspace.folder.clone()) else {
            return HotReloadApply::Unavailable;
        };
        let buffers: Vec<(PathBuf, u64, String)> = self
            .workspace_buffers(cx)
            .into_iter()
            .filter_map(|buffer| {
                let buffer = buffer.read(cx);
                is_csharp(&buffer.path).then(|| {
                    (rsharp_core::paths::canonical(&buffer.path), buffer.document.version(), buffer.document.contents())
                })
            })
            .collect();
        let breakpoints = self.session_breakpoints(&folder);
        let Some(hot) = &mut self.debug.hot_reload else { return HotReloadApply::Unavailable };
        if let Some(started_on) = &hot.session
            && !Arc::ptr_eq(&started_on.server, &server)
        {
            let said =
                "Hot Reload: the C# language server restarted since the program was launched: restart to apply changes";
            self.debug_console.update(cx, |view, cx| view.append(&[(format!("{said}\n"), true)], cx));
            self.said = Some(said.into());
            cx.notify();
            return HotReloadApply::Restart;
        }
        // A file whose text the program already runs is no edit; one never given an original was not open when the
        // program was built, and its text is no edit of that build.
        if !hot.has_changes(&buffers) {
            return HotReloadApply::Nothing;
        }
        let record: Vec<(PathBuf, u64)> = buffers.iter().map(|(path, version, _)| (path.clone(), *version)).collect();
        let reload = hot.session.clone();
        let originals = hot.ungiven();
        hot.pending = false;
        hot.applying = true;
        let paused = self.debug.phase == Phase::Paused;
        // A stop is read again after the update unless the step or resume waiting for it is about to go on.
        let refresh = (paused && after.is_none()).then_some(self.debug.thread).flatten();
        if after.is_some() {
            self.debug.after_apply = after;
        }
        let generation = self.debug.generation;
        let handle = self.handle;
        self.said = Some("Hot Reload: applying changes…".into());
        cx.notify();
        cx.spawn(async move |_, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    hot_reload_work(session, server, reload, originals, paused, refresh, breakpoints).await
                })
                .await;
            let _ = handle.update(cx, |m, window, cx| {
                if m.debug.generation != generation {
                    return;
                }
                m.hot_reload_finished(outcome, record, window, cx);
            });
        })
        .detach();
        HotReloadApply::Started
    }

    /// What the update came to: the session's number kept, the originals Roslyn now has marked given, the texts the
    /// program runs recorded, and what the console and status bar say. A paused stop is read again where the update left
    /// it, and a step or resume that waited for the edits goes on now.
    fn hot_reload_finished(
        &mut self,
        outcome: HotReloadOutcome,
        record: Vec<(PathBuf, u64)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let HotReloadOutcome { session, given, lines, said, committed, verified, refreshed, restart, stopped } =
            outcome;
        let pending = if let Some(hot) = &mut self.debug.hot_reload {
            hot.applying = false;
            if let Some(session) = session {
                hot.session = Some(session);
            }
            hot.given.extend(given);
            if committed {
                hot.applied.extend(record);
            }
            if stopped.is_some() {
                hot.stopped = stopped;
            }
            hot.pending
        } else {
            false
        };
        for (file, lines, answers) in verified {
            self.note_verification(&file, &lines, answers, cx);
        }
        self.debug_console.update(cx, |view, cx| view.append(&lines, cx));
        let reason = said.clone();
        self.said = Some(said.into());
        if pending {
            if self.hot_reload_apply(None, cx) != HotReloadApply::Started {
                self.continue_after_hot_reload(cx);
            }
            return;
        }
        // The stop stands where the update left it, even while a question is asked over it.
        if self.debug.after_apply.is_none()
            && let Some((threads, frames, variables)) = refreshed
            && self.debug.phase == Phase::Paused
        {
            self.show_stopped("Paused".into(), None, None, threads, frames, variables, window, cx);
        }
        // An edit the runtime cannot take runs only in a new program: restart it when Settings says so, else ask as
        // dotnet watch asks, unless this session was told Never.
        if restart {
            if crate::settings::settings(cx).hot_reload_restart_on_rude_edit {
                self.restart_for_hot_reload(cx);
                return;
            }
            if !self.debug.never_restart && self.choice_dialog.is_none() {
                self.ask_restart(&reason, window, cx);
                return;
            }
        }
        self.continue_after_hot_reload(cx);
    }

    /// Asks whether to restart for an edit the runtime cannot take, as dotnet watch asks; Settings' "Always restart"
    /// skips the question, and Never skips it for the rest of the session.
    fn ask_restart(&mut self, reason: &str, window: &mut Window, cx: &mut Context<Self>) {
        let message = format!("{reason}\n\nDo you want to restart your app?");
        let before = window.focused(cx);
        let dialog = cx.new(|cx| {
            crate::choice_dialog::ChoiceDialog::new(
                "Hot Reload",
                &message,
                &RESTART_ANSWERS,
                RESTART_ANSWERS.len() - 1,
                cx,
            )
        });
        cx.subscribe_in(&dialog, window, move |m, _, ended: &crate::choice_dialog::Ended, window, cx| {
            m.choice_dialog = None;
            // The question closes where it opened: the keys go back to what had them, whether the answer restarts the
            // program or not.
            if let Some(before) = &before {
                window.focus(before);
            }
            match ended {
                crate::choice_dialog::Ended::Chosen("Never") => {
                    m.debug.never_restart = true;
                    m.continue_after_hot_reload(cx);
                }
                crate::choice_dialog::Ended::Chosen("Always") => {
                    let mut settings = crate::settings::settings(cx).clone();
                    settings.hot_reload_restart_on_rude_edit = true;
                    crate::settings::apply(settings, cx);
                    m.restart_for_hot_reload(cx);
                }
                crate::choice_dialog::Ended::Chosen("Yes") => m.restart_for_hot_reload(cx),
                _ => m.continue_after_hot_reload(cx),
            }
        })
        .detach();
        window.focus(&dialog.focus_handle(cx));
        self.choice_dialog = Some(dialog);
        cx.notify();
    }

    /// Restarts the program Debug launched, so the edits it could not take run: stopped, built and launched again with
    /// its configuration and the folder's breakpoints. The configuration is the launched plan's, not the toolbar's
    /// choice if that changed while the program ran.
    fn restart_for_hot_reload(&mut self, cx: &mut Context<Self>) {
        let configuration = self
            .debug
            .plan
            .as_ref()
            .and_then(|plan| self.run_configs.iter().find(|configuration| configuration.name == plan.name).cloned())
            .or_else(|| self.chosen_run_configuration().cloned());
        self.debug.after_apply = None;
        self.stop_debug(cx);
        let Some(configuration) = configuration else {
            self.said = Some("Hot Reload: the restarted program's configuration is gone".into());
            cx.notify();
            return;
        };
        self.debug_configuration(configuration, cx);
        if self.is_debugging() {
            self.debug_console.update(cx, |view, cx| {
                view.append(&[("Hot Reload: restarting the program to apply the changes\n".into(), false)], cx)
            });
        }
    }

    /// Goes on with the step or resume that waited for the update, in the code the program has.
    fn continue_after_hot_reload(&mut self, cx: &mut Context<Self>) {
        if let Some(after) = self.debug.after_apply.take() {
            if self.debug.phase == Phase::Paused {
                match after {
                    AfterApply::Resume => self.resume_now(cx),
                    AfterApply::Step(how) => self.step_now(how, cx),
                }
            }
        }
        cx.notify();
    }

    /// The choice dialog open, for a test to answer it.
    pub fn choice_dialog(&self) -> Option<Entity<crate::choice_dialog::ChoiceDialog>> {
        self.choice_dialog.clone()
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
                let exception = (stop.reason == "exception").then(|| stop.description.clone()).flatten();
                let described = stop.reason == "exception";
                self.fetch_stopped(stop.said(), at, exception, described, stop.thread, window, cx);
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
                // A program in a Terminal tab is the tab's: the adapter's exit there is of what it started, which on
                // Windows is not the program itself.
                if let Some(id) = self.debug.terminal {
                    self.finish_with_terminal(id, name, cx);
                    return;
                }
                let message = match code {
                    Some(code) => format!("{name} exited with code {code}"),
                    None => format!("{name} exited"),
                };
                self.finish_debug(&message, cx);
            }
            Said::Terminated | Said::Ended => {
                // The exit status Said::Exited gave stands: a termination after it is the same session ending again.
                if self.debug.phase == Phase::Idle {
                    return;
                }
                // An adapter that dies as it starts may close its output before its start fails: the failure says why.
                if self.debug.session.is_none() {
                    self.debug.pending.push(said);
                    return;
                }
                let name = self.debug_name();
                match self.debug.terminal {
                    Some(id) => self.finish_with_terminal(id, name, cx),
                    None => self.finish_debug(&format!("Debugging {name} ended"), cx),
                }
            }
        }
        cx.notify();
    }

    /// A program the adapter started in a Terminal tab is the tab's to tell the end of: its exit code is waited for a
    /// moment, as the tab drains its last output, before the session says how it ended.
    fn finish_with_terminal(&mut self, id: usize, name: String, cx: &mut Context<Self>) {
        let generation = self.debug.generation;
        let handle = self.handle;
        let panel = self.terminal.clone();
        cx.spawn(async move |_, cx| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let code = loop {
                let code = panel
                    .read_with(cx, |panel, _| panel.session(id).and_then(|session| session.exit_code()))
                    .unwrap_or(None);
                if code.is_some() || std::time::Instant::now() > deadline {
                    break code;
                }
                cx.background_executor().timer(std::time::Duration::from_millis(20)).await;
            };
            let _ = handle.update(cx, |m, _, cx| {
                if m.debug.generation != generation || m.debug.phase == Phase::Idle {
                    return;
                }
                match code {
                    Some(code) => {
                        // Ended on its own: its tab keeps what it wrote and how it ended, with nothing left to stop.
                        m.debug.terminal = None;
                        m.finish_debug(&format!("Debugging {name} ended with exit code {code}"), cx);
                    }
                    None => m.finish_debug(&format!("Debugging {name} ended"), cx),
                }
            });
        })
        .detach();
    }

    /// The stop's frames and the chosen frame's values, read off the UI thread, then shown and opened at the stop.
    fn fetch_stopped(
        &mut self,
        status: String,
        at: Option<(PathBuf, usize)>,
        exception: Option<String>,
        described: bool,
        thread: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.debug.session.clone() else { return };
        let generation = self.debug.generation;
        let handle = self.handle;
        // A stop at an exception is told of it by the exception's own request, the stop naming only its reason.
        let fallback = exception.clone();
        cx.spawn(async move |_, cx| {
            let read = cx
                .background_executor()
                .spawn(async move {
                    let stop = read_stop(&session, thread);
                    // An exception whose description runs code that does not return in time still shows where it stopped.
                    let description = described.then(|| {
                        session
                            .exception_info(thread)
                            .map(|info| describe_exception(&info))
                            .map_err(|why| format!("Paused on an exception the debugger could not describe: {why}"))
                    });
                    stop.map(|(threads, frames, variables)| (threads, frames, variables, description))
                })
                .await;
            let _ = handle.update(cx, |m, window, cx| {
                if m.debug.generation != generation {
                    return;
                }
                match read {
                    Ok((threads, frames, variables, Some(Err(undescribed)))) => {
                        m.show_stopped(undescribed, at, fallback, threads, frames, variables, window, cx)
                    }
                    Ok((threads, frames, variables, description)) => {
                        let description = description.and_then(Result::ok).or(fallback);
                        m.show_stopped(status, at, description, threads, frames, variables, window, cx)
                    }
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
        exception: Option<String>,
        threads: Vec<rsharp_dap::protocol::Thread>,
        frames: Vec<rsharp_dap::protocol::Frame>,
        variables: Vec<VariableRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // An exception's stop names it once the exception's own request answers, as Rider's status bar does.
        let status = match (&exception, status.as_str()) {
            (Some(said), "Paused on an exception") => format!("Paused on an exception: {said}"),
            _ => status,
        };
        let rows: Vec<FrameRow> = frames.iter().map(frame_row).collect();
        // An exception stop names no place of its own: the frames' first with source is where the program stopped.
        let at = at.or_else(|| frames.iter().find_map(|frame| frame.path().map(|path| (path, frame.line))));
        // A stop with no source to show: the top frame's assembly is decompiled so its code shows where it stopped.
        let resolve_top = frames.first().is_some_and(|frame| {
            frame.decompiled_source_info.is_some() || (at.is_none() && frame.is_resolved == Some(false))
        });
        // The stopped frame's values, set in at the end of its line as Rider's inline values are: simple ones only,
        // four at most, each cut short.
        let inline = frames.iter().find(|frame| frame.path().is_some()).and_then(|_frame| {
            let parts: Vec<String> = variables
                .iter()
                .filter(|row| {
                    row.reference == 0
                        && !row.name.is_empty()
                        && !row.name.starts_with("$ReturnValue")
                        && row.evaluate_name.as_deref() != Some("this")
                })
                .take(4)
                .map(|row| {
                    let mut value: String = row.value.chars().take(40).collect();
                    if row.value.chars().count() > 40 {
                        value.push('…');
                    }
                    format!("{} = {value}", row.name)
                })
                .collect();
            (!parts.is_empty()).then(|| format!("  {}", parts.join("  ·  ")))
        });
        self.debug.frames = frames;
        self.debug.threads = threads;
        self.debug.phase = Phase::Paused;
        let threads: Vec<crate::debug_view::ThreadRow> = self
            .debug
            .threads
            .iter()
            .map(|thread| crate::debug_view::ThreadRow {
                id: thread.id,
                name: if thread.name.is_empty() { format!("Thread {}", thread.id) } else { thread.name.clone() },
            })
            .collect();
        let chosen = self.debug.thread;
        self.debug_view.update(cx, |view, cx| {
            view.stopped(&status, rows, cx);
            view.set_threads(threads, chosen, cx);
            view.set_variables(variables, cx);
        });
        self.clear_execution_line(cx);
        self.refresh_watches(cx);
        if let Some((path, line)) = at.filter(|_| !resolve_top) {
            self.show_stop_marks(&path, line, inline, exception, cx);
            self.open_file(&path, Some((line, 0)), window, cx);
        }
        self.said = Some(status.into());
        if resolve_top {
            self.resolve_debug_frame(0, cx);
        }
        cx.notify();
    }

    /// A frame with no source: its assembly is decompiled by the adapter, and its embedded source opened read-only at
    /// the stop. The first decompile of an assembly takes as long as it takes, off the UI thread.
    fn resolve_debug_frame(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(session) = self.debug.session.clone() else { return };
        let Some(frame) = self.debug.frames.get(index).cloned() else { return };
        let frame_id = frame.id;
        let generation = self.debug.generation;
        let handle = self.handle;
        self.said = Some("Decompiling the frame's assembly…".into());
        cx.notify();
        cx.spawn(async move |_, cx| {
            let read = cx
                .background_executor()
                .spawn(async move {
                    let resolved = if frame.decompiled_source_info.is_some() {
                        frame
                    } else {
                        session.resolve_stack_frame(frame.id)?
                    };
                    let (path, text) = session.decompiled_source(&resolved)?;
                    Ok::<_, String>((resolved, path, text))
                })
                .await;
            let _ = handle.update(cx, |m, window, cx| {
                if m.debug.generation != generation
                    || m.debug.phase != Phase::Paused
                    || m.debug.frames.get(index).is_none_or(|frame| frame.id != frame_id)
                {
                    return;
                }
                match read {
                    Ok((mut frame, path, _)) => {
                        let line = frame.line;
                        frame.source = Some(rsharp_dap::protocol::Source {
                            name: path.file_name().map(|name| name.to_string_lossy().into_owned()),
                            path: Some(path.clone()),
                        });
                        let row = frame_row(&frame);
                        if let Some(slot) = m.debug.frames.get_mut(index) {
                            *slot = frame;
                        }
                        m.debug_view.update(cx, |view, cx| view.resolved(index, row, cx));
                        if index == 0 {
                            m.show_stop_marks(&path, line, None, None, cx);
                        }
                        m.open_file(&path, Some((line, 0)), window, cx);
                        m.said = Some(
                            "Decompiled code: the debug adapter made this from the assembly, so it is read-only".into(),
                        );
                        cx.notify();
                    }
                    Err(why) => m.fail_debug(&why, cx),
                }
            });
        })
        .detach();
    }

    /// Opens a frame's source at its place: what clicking it in the Frames list does.
    pub(super) fn select_debug_frame(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(frame) = self.debug.frames.get(index).cloned() else { return };
        if frame.decompiled_source_info.is_some() || frame.is_resolved == Some(false) {
            self.resolve_debug_frame(index, cx);
        } else if let Some(path) = frame.path() {
            self.open_file(&path, Some((frame.line, 0)), window, cx);
        }
        self.debug_view.update(cx, |view, cx| view.select(index, cx));
        self.refresh_watches(cx);
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
    pub(super) fn open_debug_variable(&mut self, path: Vec<usize>, reference: i64, tree: u64, cx: &mut Context<Self>) {
        let Some(session) = self.debug.session.clone() else { return };
        // The debugger numbers values anew at each stop: a reference from an earlier one names another value now.
        if tree != self.debug_view.read(cx).tree() {
            self.debug_view.update(cx, |view, cx| {
                view.set_status("That value is from an earlier stop: open it again where the program is now", cx)
            });
            return;
        }
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
                        m.debug_view.update(cx, |view, cx| view.set_children(&path, tree, variables, cx));
                    }
                    Err(why) => m.fail_debug(&why, cx),
                }
            });
        })
        .detach();
    }

    /// The value of the word the pointer rests on in the editor, while the program is paused: its card, as Rider's
    /// debugger hover shows it, cut short as a row is.
    pub(super) fn hover_value(
        &mut self,
        editor: Entity<CodeEditor>,
        expression: String,
        at: usize,
        cx: &mut Context<Self>,
    ) {
        if self.debug.phase != Phase::Paused {
            return;
        }
        // The frame chosen in the Frames list, whose values Variables shows: a caller's word is the caller's value.
        let Some(chosen) = self.chosen_frame(cx) else { return };
        let frame = chosen.1;
        let Some(session) = self.debug.session.clone() else { return };
        let generation = self.debug.generation;
        let handle = self.handle;
        cx.spawn(async move |_, cx| {
            let asked = expression.clone();
            let read = cx.background_executor().spawn(async move { session.evaluate(&asked, frame) }).await;
            let Ok(value) = read else { return };
            let card = match &value.r#type {
                Some(kind) => format!("{expression} = {{{kind}}} {}", crate::debug_view::shown(&value.result)),
                None => format!("{expression} = {}", crate::debug_view::shown(&value.result)),
            };
            let _ = handle.update(cx, |m, _, cx| {
                // A stop, a step or another frame chosen since makes a new tree: the answer is then another frame's.
                if m.debug.generation == generation
                    && m.debug.phase == Phase::Paused
                    && m.chosen_frame(cx) == Some(chosen)
                {
                    editor.update(cx, |editor, cx| editor.show_value(at, card, cx));
                }
            });
        })
        .detach();
    }

    /// A watch, asked as it is typed: the expression in the frame chosen in the Frames list, as Rider's watches are.
    pub(super) fn evaluate_watch(&mut self, expression: String, cx: &mut Context<Self>) {
        let Some(chosen) = self.chosen_frame(cx) else {
            self.debug_view.update(cx, |view, cx| view.set_watch(&expression, "Not paused", cx));
            return;
        };
        let frame = chosen.1;
        let Some(session) = self.debug.session.clone() else { return };
        let generation = self.debug.generation;
        let handle = self.handle;
        let asked = expression.clone();
        cx.spawn(async move |_, cx| {
            let read = cx.background_executor().spawn(async move { session.evaluate(&asked, frame) }).await;
            let _ = handle.update(cx, |m, _, cx| {
                // Another frame chosen, a step or a stop since: the answer is no longer the frame shown's, and the
                // watch is asked again there.
                if m.debug.generation != generation || m.chosen_frame(cx) != Some(chosen) {
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

    /// The frame chosen in the Frames list, by its stop and id: what a watch or a hover is asked in, and what its answer
    /// must still be asked of when it lands. The values read for the frame after it is chosen do not change it.
    fn chosen_frame(&self, cx: &App) -> Option<(u64, i64)> {
        let selected = self.debug_view.read(cx).selected_frame();
        self.debug
            .frames
            .get(selected)
            .filter(|_| self.debug.phase == Phase::Paused)
            .map(|frame| (self.debug.stop, frame.id))
    }

    /// Every watch asked again, in the frame now shown.
    fn refresh_watches(&mut self, cx: &mut Context<Self>) {
        let asked: Vec<String> =
            self.debug_view.read(cx).watches().iter().map(|(expression, _)| expression.clone()).collect();
        for expression in asked {
            self.evaluate_watch(expression, cx);
        }
    }

    /// Resume: the paused program's edits are applied first, as Rider's "Changes will be applied on the next step or
    /// resume", then it goes on from where it stopped.
    pub(super) fn resume_debug(&mut self, cx: &mut Context<Self>) {
        self.debug.threads.clear();
        if self.debug.phase != Phase::Paused {
            return;
        }
        if self.hot_reload_apply(Some(AfterApply::Resume), cx) != HotReloadApply::Started {
            self.resume_now(cx);
        }
    }

    /// Sets the paused program going, its edits already in it.
    fn resume_now(&mut self, cx: &mut Context<Self>) {
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

    /// What is typed into the Debug console, sent to the program reading its input, as Rider's internal console does.
    pub fn debug_input(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(session) = self.debug.session.clone() else { return };
        let previous = self.debug.input_task.take();
        let text = text.to_string();
        let generation = self.debug.generation;
        let handle = self.handle;
        self.debug.input_task = Some(cx.spawn(async move |_, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let answer = cx.background_executor().spawn(async move { session.input_with(&text, false) }).await;
            if let Err(why) = answer {
                let _ = handle.update(cx, |m, _, cx| {
                    if m.debug.generation == generation {
                        m.fail_debug(&why, cx);
                    }
                });
            }
        }));
    }

    /// Step Over, Step Into or Step Out, as Rider's keys do them: the edits made while paused are applied first, as
    /// Rider's "Changes will be applied on the next step or resume".
    pub(super) fn step_debug(&mut self, how: Step, cx: &mut Context<Self>) {
        self.debug.threads.clear();
        if self.debug.phase != Phase::Paused {
            return;
        }
        if self.hot_reload_apply(Some(AfterApply::Step(how)), cx) != HotReloadApply::Started {
            self.step_now(how, cx);
        }
    }

    /// Takes the step, the paused program's edits already in it.
    fn step_now(&mut self, how: Step, cx: &mut Context<Self>) {
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
        // A session still waiting for its program's runtime: the adapter is told to end it, and the program with it.
        if let Some(starting) = self.debug.starting.take().filter(|_| self.debug.session.is_none()) {
            let attached = self.debug.attached.is_some();
            std::thread::spawn(move || if attached { starting.let_go() } else { starting.stop() });
            self.debug.generation += 1;
            self.debug._task = None;
            let name = self.debug_name();
            let message = format!("Debugging {name} was stopped as it started");
            self.debug_console.update(cx, |view, cx| view.append(&[(format!("{message}\n"), false)], cx));
            self.finish_debug(&message, cx);
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
        self.debug.input_task = None;
        self.end_hot_reload();
        self.debug.after_apply = None;
        self.debug.never_restart = false;
        self.choice_dialog = None;
        self.stop_debug_terminal(cx);
        self.step_dialog = None;
        self.debug.step_targets.clear();
        self.debug.threads.clear();
        self.evaluate_dialog = None;
        self.debug.session = None;
        self.debug.starting = None;
        self.debug.attached = None;
        self.debug.data_breakpoints.clear();
        self.debug.data_bound.clear();
        self.debug.phase = Phase::Idle;
        self.debug.frames.clear();
        self.debug.thread = None;
        self.debug.pending.clear();
        self.clear_execution_line(cx);
        self.debug_view.update(cx, |view, cx| view.finished(message, cx));
        self.show_data_breakpoints(cx);
        self.debug_console.update(cx, |view, cx| {
            view.running = false;
            view.set_input(false, cx);
            cx.notify();
        });
        self.said = Some(message.to_string().into());
        cx.notify();
    }

    /// Leaving the folder: the session ends and everything it started with it.
    pub(super) fn leave_debug(&mut self, cx: &mut Context<Self>) {
        self.debug.input_task = None;
        self.end_hot_reload();
        self.debug.after_apply = None;
        self.debug.never_restart = false;
        self.choice_dialog = None;
        self.stop_debug_terminal(cx);
        self.step_dialog = None;
        self.debug.step_targets.clear();
        self.debug.threads.clear();
        self.evaluate_dialog = None;
        self.debug.generation += 1;
        self.debug._task = None;
        self.debug.after_build = None;
        if let Some(session) = self.debug.session.take() {
            let terminate = self.debug.attached.is_none();
            std::thread::spawn(move || session.disconnect(terminate));
        } else if let Some(starting) = self.debug.starting.take() {
            let attached = self.debug.attached.is_some();
            std::thread::spawn(move || if attached { starting.let_go() } else { starting.stop() });
        }
        self.debug.starting = None;
        self.debug.phase = Phase::Idle;
        self.debug.attached = None;
        self.debug.data_breakpoints.clear();
        self.debug.data_bound.clear();
        self.debug.frames.clear();
        self.debug.thread = None;
        self.debug.pending.clear();
        self.debug.breakpoints = rsharp_core::debug::Breakpoints::default();
        self.debug.shown_at = None;
        self.debug.verification.clear();
        self.debug_view.update(cx, |view, cx| view.finished("Not debugging", cx));
        self.show_data_breakpoints(cx);
        self.debug_console.update(cx, |view, cx| view.clear(cx));
        self.debug_console.update(cx, |view, cx| view.set_input(false, cx));
    }

    fn refuse_debug(&mut self, message: &str, cx: &mut Context<Self>) {
        self.end_hot_reload();
        self.debug.after_apply = None;
        self.debug.never_restart = false;
        self.choice_dialog = None;
        self.step_dialog = None;
        self.debug.step_targets.clear();
        self.debug.threads.clear();
        self.evaluate_dialog = None;
        self.debug.generation += 1;
        self.debug._task = None;
        self.debug.session = None;
        self.debug.starting = None;
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

    /// Shift+click in the gutter: a tracepoint, which logs where it was reached and goes on, as Rider's does; one already
    /// on the line is cleared as a click clears it.
    pub(super) fn toggle_tracepoint_at(&mut self, path: &Path, line: usize, cx: &mut Context<Self>) {
        if self.debug.breakpoints.has(path, line) {
            self.toggle_breakpoint_at(path, line, cx);
            return;
        }
        self.debug.breakpoints.toggle(path, line);
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let message = format!("Breakpoint reached at {name}:{}", line + 1);
        self.debug.breakpoints.edit(path, line, |breakpoint| breakpoint.log_message = message);
        self.save_breakpoints(cx);
        self.show_breakpoints(path, cx);
        self.said = Some(format!("Tracepoint set at {name}:{}", line + 1).into());
        self.push_breakpoints(path.to_path_buf(), cx);
        cx.notify();
    }

    /// Alt+F9: the caret line's breakpoint popup, a breakpoint set there first when it has none, its condition ready.
    pub(super) fn edit_breakpoint(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else { return };
        let (path, line) = {
            let editor = editor.read(cx);
            (editor.path.clone(), editor.caret().line)
        };
        if !self.debug.breakpoints.has(&path, line) {
            self.toggle_breakpoint_at(&path, line, cx);
        }
        self.open_breakpoint_popup(&path, line, crate::breakpoint_popup::Field::Condition, window, cx);
    }

    /// Rider's popup over a line's breakpoint: what is typed in it is the breakpoint's at once.
    pub(super) fn open_breakpoint_popup(
        &mut self,
        path: &Path,
        line: usize,
        field: crate::breakpoint_popup::Field,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(breakpoint) = self.debug.breakpoints.breakpoint(path, line).cloned() else { return };
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let popup = cx.new(|cx| {
            crate::breakpoint_popup::BreakpointPopup::new(
                format!("{name}:{}", line + 1),
                &breakpoint.condition,
                &breakpoint.hit_count,
                &breakpoint.log_message,
                cx,
            )
        });
        cx.subscribe_in(&popup, window, |m, _, ended: &crate::breakpoint_popup::Ended, window, cx| match ended {
            crate::breakpoint_popup::Ended::Changed { condition, hit_count, log_message } => {
                let Some((_, path, line)) = m.breakpoint_popup.clone() else { return };
                m.debug.breakpoints.edit(&path, line, |breakpoint| {
                    breakpoint.condition = condition.clone();
                    breakpoint.hit_count = hit_count.clone();
                    breakpoint.log_message = log_message.clone();
                });
                m.save_breakpoints(cx);
                m.show_breakpoints(&path, cx);
                m.push_breakpoints(path, cx);
                cx.notify();
            }
            crate::breakpoint_popup::Ended::Dismissed => {
                m.breakpoint_popup = None;
                m.focus_editor(window, cx);
                cx.notify();
            }
        })
        .detach();
        popup.update(cx, |popup, cx| popup.focus_field(field, window, cx));
        self.breakpoint_popup = Some((popup, path.to_path_buf(), line));
        cx.notify();
    }

    /// The breakpoint popup open, for a test to drive.
    pub fn breakpoint_popup(&self) -> Option<Entity<crate::breakpoint_popup::BreakpointPopup>> {
        self.breakpoint_popup.as_ref().map(|(popup, _, _)| popup.clone())
    }

    /// Carries a file's breakpoints through an edit to it, as Rider does: a line moves with its code, the gutter follows,
    /// and a session that runs is told again. Two editors of one file make the move once.
    pub(super) fn follow_breakpoints(
        &mut self,
        buffer: &Entity<Buffer>,
        changes: &[rsharp_text::Change],
        cx: &mut Context<Self>,
    ) {
        if changes.is_empty() {
            return;
        }
        let (path, version) = {
            let buffer = buffer.read(cx);
            (rsharp_core::paths::canonical(&buffer.path), buffer.document.version())
        };
        let edit = (buffer.entity_id(), version);
        if self.followed_edit == Some(edit) {
            return;
        }
        self.followed_edit = Some(edit);
        let moved = self.debug.breakpoints.follow(&path, |line| rsharp_text::map_line(changes, line));
        if moved.is_empty() {
            return;
        }
        for (from, _) in &moved {
            self.debug.verification.remove(&(path.clone(), *from));
        }
        self.save_breakpoints(cx);
        self.show_breakpoints(&path, cx);
        if self.is_debugging() {
            self.push_breakpoints(path, cx);
        }
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
        let key = rsharp_core::paths::canonical(&path);
        // The program runs the text it was built from: a line is sent where the build has it, and one written since,
        // with no code in the build, is refused here rather than bound to whatever the build has on that line.
        let hunks = self.unbuilt_edits(&key, cx);
        let mut sent = Vec::new();
        let mut lines = Vec::new();
        for breakpoint in &breakpoints {
            match built_line(&hunks, breakpoint.line) {
                Some(built) => {
                    sent.push(SourceBreakpoint {
                        line: built,
                        column: breakpoint.column,
                        condition: filled(&breakpoint.condition),
                        hit_condition: filled(&breakpoint.hit_count),
                        log_message: filled(&breakpoint.log_message),
                    });
                    lines.push(breakpoint.line);
                }
                None => {
                    self.debug.verification.insert(
                        (key.clone(), breakpoint.line),
                        (false, Some("This line was written after the program being debugged was built".into())),
                    );
                }
            }
        }
        self.show_breakpoints(&path, cx);
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

    /// How an open file differs from the text the program being debugged was built from, while no update has given the
    /// program its edits; none when they are one.
    // shortcut: once Hot Reload has applied a file, its later edits are taken as in the program too; keep the text each
    // update applied when a breakpoint set between an update and the next must be told apart.
    fn unbuilt_edits(&self, path: &Path, cx: &App) -> Vec<rsharp_text::DiffHunk> {
        let Some(hot_reload) = &self.debug.hot_reload else { return Vec::new() };
        if hot_reload.applied.keys().any(|applied| rsharp_core::paths::same(applied, path)) {
            return Vec::new();
        }
        let Some(original) =
            hot_reload.originals.iter().find(|(file, _)| rsharp_core::paths::same(file, path)).map(|(_, text)| text)
        else {
            return Vec::new();
        };
        let Some(buffer) = crate::buffer::find(&path.to_path_buf(), cx) else { return Vec::new() };
        let text = buffer.read(cx).document.text().to_string();
        if &text == original { Vec::new() } else { rsharp_text::diff_hunks(original, &text) }
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
        self.push_function_breakpoints(cx);
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
    pub fn set_breakpoints_muted(&mut self, muted: bool, cx: &mut Context<Self>) {
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

    /// Marks the line the program is stopped at, with the frame's values set in at its end, clearing the line it was
    /// at before.
    fn show_stop_marks(
        &mut self,
        path: &Path,
        line: usize,
        inline: Option<String>,
        exception: Option<String>,
        cx: &mut App,
    ) {
        if let Some(buffer) = crate::buffer::find(&path.to_path_buf(), cx) {
            buffer.update(cx, |buffer, cx| {
                buffer.set_execution_line(Some(line), cx);
                buffer.set_inline_values(inline.map(|values| (line, values)), cx);
                buffer.set_exception(exception.map(|said| (line, said)), cx);
            });
            self.debug.shown_at = Some(path.to_path_buf());
        }
    }

    fn clear_execution_line(&mut self, cx: &mut App) {
        self.debug.stop += 1;
        // Every stop's end clears the marks, and with them the value cards read at it.
        let editors: Vec<Entity<CodeEditor>> = self.area.editors().cloned().collect();
        for editor in editors {
            editor.update(cx, |editor, cx| editor.clear_value(cx));
        }
        if let Some(path) = self.debug.shown_at.take()
            && let Some(buffer) = crate::buffer::find(&path, cx)
        {
            buffer.update(cx, |buffer, cx| {
                buffer.set_execution_line(None, cx);
                buffer.set_inline_values(None, cx);
                buffer.set_exception(None, cx);
            });
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
            DebugAction::OpenVariable { path, reference, tree } => self.open_debug_variable(path, reference, tree, cx),
            DebugAction::Evaluate(expression) => self.evaluate_watch(expression, cx),
            DebugAction::ToggleMute => self.set_breakpoints_muted(!self.debug.breakpoints.muted(), cx),
            DebugAction::SetValue { expression, value } => self.set_value(expression, value, cx),
            DebugAction::SelectThread(id) => self.select_debug_thread(id, window, cx),
            DebugAction::CopyValue(value) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(value));
            }
            DebugAction::SetDataBreakpoint { reference, name } => self.set_data_breakpoint(reference, name, cx),
            DebugAction::RemoveDataBreakpoint(data_id) => self.remove_data_breakpoint(&data_id, cx),
            DebugAction::AddWatch(expression) => {
                self.debug_view.update(cx, |view, cx| view.add_watch(&expression, cx));
                self.evaluate_watch(expression, cx);
            }
            DebugAction::CopyStack => {
                let stack = self
                    .debug
                    .frames
                    .iter()
                    .map(|frame| {
                        let place = frame.path().map(|path| {
                            format!("{}:{}", path.file_name().unwrap_or_default().to_string_lossy(), frame.line + 1)
                        });
                        match place {
                            Some(place) => format!("{} at {place}", frame.name),
                            None => frame.name.clone(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(stack));
            }
        }
    }

    /// Rider's Set Data Breakpoint on a value: the debugger names what a write to it would watch, and the session
    /// watches it until it ends.
    fn set_data_breakpoint(&mut self, reference: i64, name: String, cx: &mut Context<Self>) {
        if self.debug.phase != Phase::Paused {
            return;
        }
        let Some(session) = self.debug.session.clone() else { return };
        let generation = self.debug.generation;
        let handle = self.handle;
        self.said = Some(format!("Asking the debugger about {name}…").into());
        cx.notify();
        cx.spawn(async move |_, cx| {
            let asked = name.clone();
            let info =
                cx.background_executor().spawn(async move { session.data_breakpoint_info(reference, &asked) }).await;
            let _ = handle.update(cx, |m, _, cx| {
                if m.debug.generation != generation {
                    return;
                }
                match info {
                    Ok(info) => match info.data_id {
                        Some(data_id) => {
                            m.debug.data_breakpoints.push(rsharp_dap::DataBreakpoint {
                                data_id,
                                location: info.description,
                                condition: None,
                                hit_condition: None,
                                log_message: None,
                            });
                            m.said = Some(format!("Watching {name} for a write").into());
                            m.push_data_breakpoints(cx);
                            m.show_data_breakpoints(cx);
                        }
                        None => {
                            m.said = Some(format!("No data breakpoint on {name}: {}", info.description).into());
                        }
                    },
                    Err(why) => m.said = Some(why.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn remove_data_breakpoint(&mut self, data_id: &str, cx: &mut Context<Self>) {
        self.debug.data_breakpoints.retain(|breakpoint| breakpoint.data_id != data_id);
        self.push_data_breakpoints(cx);
        self.show_data_breakpoints(cx);
        cx.notify();
    }

    /// The session is told of the data breakpoints that are on, off the UI thread.
    fn push_data_breakpoints(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.debug.session.clone() else { return };
        let list = self.debug.data_breakpoints.clone();
        let generation = self.debug.generation;
        let handle = self.handle;
        cx.spawn(async move |_, cx| {
            let sent = cx.background_executor().spawn(async move { session.set_data_breakpoints(&list) }).await;
            let _ = handle.update(cx, |m, _, cx| {
                if m.debug.generation != generation {
                    return;
                }
                match sent {
                    Ok(answers) => {
                        // A watch the debugger could not set says so, as a line breakpoint it refuses does.
                        if let Some(why) = answers.iter().find(|answer| !answer.verified).map(|answer| {
                            answer.message.clone().unwrap_or_else(|| "the debugger did not set it".into())
                        }) {
                            m.said = Some(format!("The data breakpoint does not watch: {why}").into());
                        }
                        m.debug.data_bound =
                            answers.into_iter().map(|answer| (answer.verified, answer.message)).collect();
                    }
                    Err(why) => m.said = Some(why.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// What the debugger made of each data breakpoint, for a test: whether it watches, and why not.
    pub fn data_breakpoints_bound(&self) -> &[(bool, Option<String>)] {
        &self.debug.data_bound
    }

    /// The Debug window's list of the session's data breakpoints.
    fn show_data_breakpoints(&mut self, cx: &mut Context<Self>) {
        let list = self
            .debug
            .data_breakpoints
            .iter()
            .map(|breakpoint| (breakpoint.data_id.clone(), breakpoint.location.clone()))
            .collect();
        self.debug_view.update(cx, |view, cx| view.set_data_breakpoints(list, cx));
    }

    /// Set Value changes an expression in the stopped frame and reads its values again.
    fn set_value(&mut self, expression: String, value: String, cx: &mut Context<Self>) {
        let Some(frame) = self.debug.frames.first().cloned() else { return };
        let Some(session) = self.debug.session.clone() else { return };
        let generation = self.debug.generation;
        let handle = self.handle;
        let (asked, set_to) = (expression.clone(), value.clone());
        cx.spawn(async move |_, cx| {
            let read = cx
                .background_executor()
                .spawn(async move {
                    session.set_expression(&asked, &set_to, frame.id)?;
                    read_variables(&session, frame.id)
                })
                .await;
            let _ = handle.update(cx, |m, _, cx| {
                if m.debug.generation != generation {
                    return;
                }
                match read {
                    Ok(variables) => {
                        m.debug_view.update(cx, |view, cx| view.set_variables(variables, cx));
                        m.said = Some(format!("Set {expression} to {value}").into());
                        cx.notify();
                    }
                    Err(why) => m.fail_debug(&why, cx),
                }
            });
        })
        .detach();
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

    /// The calls Smart Step Into last offered, for a test.
    pub fn step_targets(&self) -> &[rsharp_dap::protocol::StepInTarget] {
        &self.debug.step_targets
    }

    /// Smart Step Into (Shift+Alt+F11): the calls the stopped statement makes, in the order they run, one chosen.
    pub(super) fn open_smart_step_into(&mut self, cx: &mut Context<Self>) {
        if self.debug.phase != Phase::Paused || self.step_dialog.is_some() {
            return;
        }
        let Some(frame) = self.debug.frames.first().map(|frame| frame.id) else { return };
        let Some(session) = self.debug.session.clone() else { return };
        let generation = self.debug.generation;
        let handle = self.handle;
        cx.spawn(async move |_, cx| {
            let read = cx.background_executor().spawn(async move { session.step_in_targets(frame) }).await;
            let _ = handle.update(cx, |m, window, cx| {
                if m.debug.generation != generation {
                    return;
                }
                match read {
                    Ok(targets) if targets.is_empty() => {
                        m.said = Some("There is no call to step into at this frame".into());
                        cx.notify();
                    }
                    Ok(targets) => m.show_step_dialog(targets, window, cx),
                    Err(why) => m.fail_debug(&why, cx),
                }
            });
        })
        .detach();
    }

    fn show_step_dialog(
        &mut self,
        targets: Vec<rsharp_dap::protocol::StepInTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialog = cx.new(|cx| crate::step_into_dialog::StepIntoDialog::new(&targets, cx));
        cx.subscribe_in(&dialog, window, |m, _, ended: &crate::step_into_dialog::Ended, window, cx| match ended {
            crate::step_into_dialog::Ended::Chosen(index) => {
                m.step_dialog = None;
                if let Some(target) = m.debug.step_targets.get(*index).map(|target| target.id) {
                    m.step_into_call(target, cx);
                }
            }
            crate::step_into_dialog::Ended::Dismissed => {
                m.step_dialog = None;
                m.focus_editor(window, cx);
            }
        })
        .detach();
        window.focus(&dialog.focus_handle(cx));
        self.debug.step_targets = targets;
        self.step_dialog = Some(dialog);
        cx.notify();
    }

    /// The chosen call stepped into, the debugger's answer waited for off the UI thread.
    fn step_into_call(&mut self, target: i64, cx: &mut Context<Self>) {
        let Some(thread) = self.debug.thread else { return };
        self.debug.phase = Phase::Running;
        self.debug.frames.clear();
        self.clear_execution_line(cx);
        self.debug_view.update(cx, |view, cx| view.running(cx));
        self.ask_session(cx, move |session| session.step_into_target(thread, target), |_, (), _, _| {});
    }

    /// The paused program's threads as last read, for a test.
    pub fn debug_threads(&self) -> &[rsharp_dap::protocol::Thread] {
        &self.debug.threads
    }

    /// The thread whose frames the Debug window shows.
    pub fn debug_thread(&self) -> Option<i64> {
        self.debug.thread
    }

    /// A thread chosen: its frames and values are read and shown, the debugger's steps going to it from now on.
    pub(super) fn select_debug_thread(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.debug.phase != Phase::Paused {
            return;
        }
        self.debug.thread = Some(id);
        self.debug_view.update(cx, |view, cx| view.choose_thread(id, cx));
        self.fetch_stopped("Paused".into(), None, None, false, id, window, cx);
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
                cx.notify();
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
                .on_action(cx.listener(|m, _: &EditBreakpoint, window, cx| m.edit_breakpoint(window, cx)))
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
                        .on_action(cx.listener(|m, _: &SmartStepInto, _, cx| m.open_smart_step_into(cx)))
                        .on_action(cx.listener(|m, _: &StepOver, _, cx| m.step_debug(Step::Over, cx)))
                        .on_action(cx.listener(|m, _: &StepInto, _, cx| m.step_debug(Step::Into, cx)))
                        .on_action(cx.listener(|m, _: &StepOut, _, cx| m.step_debug(Step::Out, cx)))
                })
                .when(self.debug.phase == Phase::Running, |d| {
                    d.on_action(cx.listener(|m, _: &PauseDebug, _, cx| m.pause_debug(cx)))
                })
                .when(self.is_debugging(), |d| {
                    d.on_action(cx.listener(|m, _: &Stop, _, cx| m.stop_debug(cx)))
                        .on_action(cx.listener(|m, _: &StopDebug, _, cx| m.stop_debug(cx)))
                })
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Step {
    Over,
    Into,
    Out,
}

/// What waited for the edits made while paused: the step or resume that goes on once they are in the program.
#[derive(Clone, Copy)]
pub(super) enum AfterApply {
    Resume,
    Step(Step),
}

/// What Apply Hot Reload Changes came to, so a caller can say what happened rather than take an update that did not
/// start for an empty one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum HotReloadApply {
    /// An update is on its way, or one already on its way takes these changes in hand.
    Started,
    /// Nothing managed changed since the program last took the text.
    Nothing,
    /// The C# language server restarted since the launch: only a new program takes the changes.
    Restart,
    /// The program cannot take an update now.
    Unavailable,
}

/// dotnet watch's question for an edit the runtime cannot take, and its answers in the order the dialog shows them (the
/// last its default), as Passenger asks it.
const RESTART_ANSWERS: [&str; 4] = ["Never", "Always", "No", "Yes"];

/// A frame as the Frames list shows it.
/// Where a line of the text as it is now stands in the text the program was built from; none for a line written since.
fn built_line(hunks: &[rsharp_text::DiffHunk], line: usize) -> Option<usize> {
    let mut shift = 0isize;
    for hunk in hunks {
        if line < hunk.after.start {
            break;
        }
        if line < hunk.after.end {
            // A changed line keeps its place while as many lines replace it; one more is new.
            let at = line - hunk.after.start;
            return (at < hunk.before.len()).then_some(hunk.before.start + at);
        }
        shift = hunk.before.end as isize - hunk.after.end as isize;
    }
    Some((line as isize + shift) as usize)
}

fn frame_row(frame: &rsharp_dap::protocol::Frame) -> FrameRow {
    FrameRow {
        name: frame.short_name(),
        place: match (&frame.path(), frame.line) {
            (Some(path), line) => {
                format!("{}:{}", path.file_name().unwrap_or_default().to_string_lossy(), line + 1)
            }
            _ => "external code".into(),
        },
        path: frame.path(),
        line: frame.line,
    }
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
        evaluate_name: variable.evaluate_name,
        reference: variable.variables_reference,
    }
}

/// An exception as Rider's stop names it: its type without its namespace, and its message.
fn describe_exception(info: &rsharp_dap::protocol::ExceptionInfo) -> String {
    let type_name = info
        .details
        .as_ref()
        .and_then(|details| details.type_name.clone())
        .unwrap_or_else(|| info.exception_id.trim_start_matches("CLR/").to_string());
    let short = type_name.rsplit('.').next().unwrap_or(&type_name).to_string();
    match info.details.as_ref().and_then(|details| details.message.clone()).or_else(|| info.description.clone()) {
        Some(message) if !message.is_empty() => format!("{short}: {message}"),
        _ => short,
    }
}

/// A stop's frames and the innermost frame's values, read on a thread that may wait.
fn read_stop(
    session: &Session,
    thread: i64,
) -> Result<(Vec<rsharp_dap::protocol::Thread>, Vec<rsharp_dap::protocol::Frame>, Vec<VariableRow>), String> {
    let threads = session.threads().unwrap_or_default();
    let frames = session.stack_trace(thread)?;
    // The values are the first frame with source's, as an exception stop's top frame may be the runtime's.
    let sourceful = frames.iter().find(|frame| frame.path().is_some()).or_else(|| frames.first());
    let variables = match sourceful {
        // A thread stopped outside the person's code has no locals to show, not a failure.
        Some(frame) => read_variables(session, frame.id).unwrap_or_default(),
        None => Vec::new(),
    };
    Ok((threads, frames, variables))
}

fn read_variables(session: &Session, frame: i64) -> Result<Vec<VariableRow>, String> {
    let scopes = session.scopes(frame)?;
    let Some(scope) = scopes.iter().find(|scope| scope.name == "Locals").or_else(|| scopes.first()) else {
        return Ok(Vec::new());
    };
    Ok(session.variables(scope.variables_reference)?.into_iter().map(variable_row).collect())
}

/// One save's Hot Reload, off the UI thread: Roslyn's Edit and Continue session started if it has not, the program's
/// state read, the edits emitted, the debugger given them, and Roslyn told whether they were committed or discarded.
async fn hot_reload_work(
    session: Arc<Session>,
    server: Arc<LanguageServer>,
    reload: Option<Arc<HotReloadSession>>,
    originals: Vec<(PathBuf, String)>,
    paused: bool,
    refresh: Option<i64>,
    breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)>,
) -> HotReloadOutcome {
    let began = std::time::Instant::now();
    let given: Vec<PathBuf> = originals.iter().map(|(path, _)| path.clone()).collect();
    let originals_json =
        json!(originals.iter().map(|(path, text)| json!({ "path": path, "text": text })).collect::<Vec<_>>());
    let reload = match reload {
        Some(reload) => reload,
        None => {
            let message = json!({ "capabilities": [], "originals": originals_json });
            match extension_request(&server, extension::START_HOT_RELOAD, message).await {
                Ok(started) => match started["session"].as_i64() {
                    Some(number) => Arc::new(HotReloadSession { number, server: server.clone() }),
                    None => {
                        let problem =
                            started["problem"].as_str().unwrap_or("the extension gave no session").to_string();
                        return HotReloadOutcome::failed(format!("Hot Reload could not start: {problem}"));
                    }
                },
                Err(why) => return HotReloadOutcome::failed(format!("Hot Reload could not start: {why}")),
            }
        }
    };
    let number = reload.number;
    let state = match session.hot_reload_state() {
        Ok(state) => state,
        Err(why) => {
            let said = format!("Hot Reload: the debugger could not say what the program can take: {why}");
            return HotReloadOutcome::failed(said).session(reload).given(given);
        }
    };
    let Some(capabilities) = state.capabilities.clone() else {
        let why = state
            .capabilities_unknown
            .clone()
            .unwrap_or_else(|| "the program's runtime did not say which edits it can take".into());
        return HotReloadOutcome::failed(format!("Hot Reload: {why}")).session(reload).given(given);
    };
    let modules: Vec<String> =
        state.modules.iter().filter(|module| module.editable).map(|module| module.mvid.clone()).collect();
    let locked: Vec<Value> = state
        .modules
        .iter()
        .filter(|module| !module.editable)
        .map(|module| {
            json!({
                "module": module.mvid,
                "why": module.why_not.clone().unwrap_or_else(|| format!("{} cannot be edited", module.name)),
            })
        })
        .collect();
    let active: Vec<Value> = state
        .active_statements
        .iter()
        .map(|statement| {
            json!({
                "module": statement.module, "token": statement.token, "version": statement.version,
                "ilOffset": statement.il_offset, "document": statement.document,
                "startLine": statement.start_line, "startColumn": statement.start_column,
                "endLine": statement.end_line, "endColumn": statement.end_column, "flags": statement.flags,
            })
        })
        .collect();
    let message = json!({
        "session": number,
        "paused": paused,
        "capabilities": capabilities,
        "modules": modules,
        "activeStatements": active,
        "locked": locked,
        "originals": originals_json,
    });
    let emitted = match extension_request(&server, extension::EMIT_HOT_RELOAD, message).await {
        Ok(emitted) => emitted,
        Err(why) => {
            let said = format!("Hot Reload failed in the C# language server: {why}");
            return HotReloadOutcome::failed(said).session(reload).given(given);
        }
    };
    if let Some(problem) = emitted["problem"].as_str() {
        let said = format!("Hot Reload failed: {problem}");
        return HotReloadOutcome::failed(said).session(reload).given(given);
    }
    let problems = hot_reload_problems(&emitted["problems"]);
    match emitted["status"].as_str().unwrap_or_default() {
        "none" => {
            let said = "Hot Reload: no managed code changes to apply".to_string();
            HotReloadOutcome {
                session: Some(reload),
                given,
                lines: vec![(format!("{said}\n"), false)],
                said,
                committed: true,
                verified: Vec::new(),
                refreshed: None,
                restart: false,
                stopped: None,
            }
        }
        "blocked" => {
            let compiles = !emitted["problems"].as_array().into_iter().flatten().any(|problem| {
                problem["error"].as_bool().unwrap_or(false) && !problem["restartRequired"].as_bool().unwrap_or(false)
            });
            let said = if compiles {
                format!("Hot Reload: restart is needed to apply the changes{}", first_problems(&problems))
            } else {
                format!("Hot Reload: unable to apply changes due to compilation errors{}", first_problems(&problems))
            };
            let mut lines: Vec<(String, bool)> =
                problems.iter().map(|problem| (format!("{problem}\n"), true)).collect();
            lines.push((format!("{said}\n"), true));
            HotReloadOutcome {
                session: Some(reload),
                given,
                lines,
                said,
                committed: false,
                verified: Vec::new(),
                refreshed: None,
                restart: compiles,
                stopped: None,
            }
        }
        _ => {
            let applied = match session.apply_changes(emitted["updates"].clone()) {
                Ok(applied) => applied,
                Err(why) => {
                    tell_hot_reload(&server, extension::DISCARD_HOT_RELOAD, number).await;
                    let said = format!("The debugger could not apply the changes: {why}");
                    return HotReloadOutcome::failed(said).session(reload).given(given);
                }
            };
            let mut lines: Vec<(String, bool)> =
                problems.iter().map(|problem| (format!("{problem}\n"), false)).collect();
            for module in &applied {
                lines.push(match (&module.message, module.applied) {
                    (_, true) => (format!("Hot Reload: {} updated\n", module.name), false),
                    (Some(message), false) => (format!("Hot Reload: {}: {message}\n", module.name), true),
                    (None, false) => (format!("Hot Reload: {} refused\n", module.name), true),
                });
            }
            if let Some(refused) = applied.iter().find(|module| !module.applied) {
                tell_hot_reload(&server, extension::DISCARD_HOT_RELOAD, number).await;
                let why = refused.message.clone().unwrap_or_else(|| {
                    format!("the runtime refused the update to {}: restart to apply changes", refused.name)
                });
                let said = format!("Hot Reload: {why}");
                lines.push((format!("{said}\n"), true));
                return HotReloadOutcome {
                    session: Some(reload),
                    given,
                    lines,
                    said,
                    committed: false,
                    verified: Vec::new(),
                    refreshed: None,
                    restart: false,
                    stopped: Some(why),
                };
            }
            tell_hot_reload(&server, extension::COMMIT_HOT_RELOAD, number).await;
            // The code the update replaced is not where the debugger bound a file's breakpoints: they are sent again at
            // their lines now, as Passenger does, and the debugger's word on each kept.
            let mut verified = Vec::new();
            for changed in changed_documents(&emitted) {
                let Some((path, lines)) = breakpoints.iter().find(|(path, _)| rsharp_core::paths::same(path, &changed))
                else {
                    continue;
                };
                if let Ok(answers) = session.set_breakpoints(path, lines) {
                    verified.push((path.clone(), lines.iter().map(|breakpoint| breakpoint.line).collect(), answers));
                }
            }
            let refreshed = match refresh {
                Some(thread) => read_stop(&session, thread).ok(),
                None => None,
            };
            let methods: usize = emitted["updates"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|update| update["updatedMethods"].as_array().map_or(0, Vec::len))
                .sum();
            let files: Vec<String> = changed_documents(&emitted)
                .iter()
                .filter_map(|path| path.file_name().and_then(|name| name.to_str()).map(str::to_string))
                .collect();
            let named = if files.is_empty() { String::new() } else { format!(": {}", files.join(", ")) };
            let said = format!(
                "Hot Reload: C# changes applied in {} ms{named} ({methods} method{})",
                began.elapsed().as_millis(),
                if methods == 1 { "" } else { "s" },
            );
            lines.push((format!("{said}\n"), false));
            HotReloadOutcome {
                session: Some(reload),
                given,
                lines,
                said,
                committed: true,
                verified,
                refreshed,
                restart: false,
                stopped: None,
            }
        }
    }
}

/// The documents Roslyn's update changed, as it names them.
fn changed_documents(emitted: &Value) -> Vec<PathBuf> {
    emitted["changedDocuments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|path| path.as_str())
        .map(PathBuf::from)
        .collect()
}

/// Asks one of Passenger's extension handlers in Roslyn's server, off the UI thread, for what it answered.
async fn extension_request(server: &LanguageServer, handler: &str, message: Value) -> Result<Value, String> {
    let (method, params) = extension::workspace_request(handler, &message);
    let answer =
        server.ask(method, params).await.map_err(|failed| format!("the C# language server stopped: {failed}"))?;
    extension::answer(&answer)
}

/// Tells Roslyn that the update it emitted was committed, or that it was discarded: a session already gone is no failure.
async fn tell_hot_reload(server: &LanguageServer, handler: &str, session: i64) {
    let _ = extension_request(server, handler, json!({ "session": session })).await;
}

/// The edits Roslyn could not take, as the Debug console writes them: each at its line and column, as a compiler says.
fn hot_reload_problems(problems: &Value) -> Vec<String> {
    problems
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|problem| {
            let message = problem["message"].as_str()?;
            Some(match problem["path"].as_str() {
                Some(path) => format!(
                    "{path}({},{}) : {} {message}",
                    problem["startLine"].as_i64().unwrap_or(0) + 1,
                    problem["startColumn"].as_i64().unwrap_or(0) + 1,
                    if problem["error"].as_bool().unwrap_or(false) { "error" } else { "warning" },
                ),
                None => message.to_string(),
            })
        })
        .collect()
}

/// The first two problems, with the rest counted, as Passenger's status bar says them.
fn first_problems(problems: &[String]) -> String {
    match problems {
        [] => String::new(),
        [one] => format!(": {one}"),
        [one, two] => format!(": {one}; {two}"),
        _ => format!(": {}; {}; and {} more", problems[0], problems[1], problems.len() - 2),
    }
}
