//! One workspace's runs: the folder's run configurations, the plan a launch works out, the program in the Run console,
//! and the browser a web project opens once it listens.

use super::*;
use crate::console::ConsoleView;
use futures::{FutureExt, channel::mpsc};
use rsharp_core::run::{LaunchPlan, RunConfiguration, browser};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How a run stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    Exited(i32),
    Stopped,
    /// Nothing was started: no configuration, a profile this IDE does not run, a failed save or a failed build.
    Refused,
}

/// How a run ended, or why it did not start, in the person's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOutcome {
    pub status: RunStatus,
    pub message: String,
}

pub(super) enum Event {
    Output(String, bool),
    /// A plan, or why the configuration could not be resolved.
    Resolved(Result<Box<LaunchPlan>, String>),
    /// The URL the browser is given, once something listens there.
    Browser(String),
    /// The program never listened at its URL.
    NotResponding(String),
    /// The program could not be started.
    Unstarted(String),
    Ended(Option<i32>),
}

impl MainWindow {
    /// The folder's run configurations: its executable projects' launch profiles, and each project itself.
    pub fn run_configurations(&self) -> &[RunConfiguration] {
        &self.run_configs
    }

    /// The configuration the toolbar shows and Run launches.
    pub fn chosen_run_configuration(&self) -> Option<&RunConfiguration> {
        let chosen = self.run_selected.as_deref();
        self.run_configs
            .iter()
            .find(|configuration| Some(configuration.name.as_str()) == chosen)
            .or_else(|| self.run_configs.first())
    }

    pub fn run_outcome(&self) -> Option<&RunOutcome> {
        self.run_outcome.as_ref()
    }

    /// Whether a program the window started runs, or is between Run and its start.
    pub fn is_running(&self) -> bool {
        self.run_cancel.is_some()
    }

    pub fn run_output(&self, cx: &App) -> String {
        self.run_console.read(cx).text()
    }

    pub fn run_console(&self) -> &Entity<ConsoleView> {
        &self.run_console
    }

    /// Takes over how a run opens its browser, for a test that must not open one: it is given the URL instead.
    pub fn set_browser_opener(&mut self, opener: impl Fn(String) + Send + Sync + 'static) {
        self.browser_opener = Arc::new(opener);
    }

    /// Makes the list again from the projects the folder has now, with why each cannot launch, keeping the
    /// configuration the person chose while it is still there; the first stands when theirs is gone. It runs when the
    /// Solution view's rows are made — an open, an evaluation, a solution edit, the watcher's burst — so drawing the
    /// menu and pressing Run read it instead of walking the folder again.
    pub(super) fn refresh_run_configurations(&mut self) {
        self.run_projects = self.discovered_projects();
        // The folder's own first, as Rider lists them, then what its projects and launch profiles make.
        self.run_configs = self.run_own.clone();
        self.run_configs.extend(RunConfiguration::generate(&self.run_projects));
        self.run_why_not =
            self.run_configs.iter().map(|configuration| configuration.why_not_here(&self.run_projects)).collect();
        if self
            .run_selected
            .as_deref()
            .is_some_and(|chosen| !self.run_configs.iter().any(|configuration| configuration.name == chosen))
        {
            self.run_selected = None;
        }
    }

    /// The folder's project files, each with what the window evaluated of it: a launch profile says what runs even
    /// before an evaluation lands, so the list is the person's from the first moment.
    fn discovered_projects(&self) -> Vec<rsharp_core::project::EvaluatedProject> {
        use rsharp_core::project::EvaluatedProject;
        let Some(workspace) = &self.workspace else { return Vec::new() };
        let mut projects: Vec<PathBuf> = match &self.tree {
            Some(tree) => tree.solution().projects().into_iter().filter(|p| p.exists).map(|p| p.path.clone()).collect(),
            None => crate::workspace::all_files(&workspace.folder, usize::MAX)
                .into_iter()
                .filter(|path| is_project(path))
                .collect(),
        };
        projects.sort();
        projects
            .into_iter()
            .map(|path| {
                self.tree
                    .as_ref()
                    .and_then(|tree| tree.evaluated(&path))
                    .cloned()
                    .unwrap_or(EvaluatedProject { path, ..EvaluatedProject::default() })
            })
            .collect()
    }

    /// Why a configuration cannot launch among the projects as last evaluated; none when it can, or when the list no
    /// longer holds it and MSBuild has the last word.
    fn why_not_run(&self, configuration: &RunConfiguration) -> Option<String> {
        let at = self.run_configs.iter().position(|known| known.name == configuration.name)?;
        self.run_why_not.get(at).cloned().flatten()
    }

    pub(super) fn choose_run_configuration(&mut self, name: String, cx: &mut Context<Self>) {
        self.run_selected = Some(name);
        self.run_menu = false;
        self.save_run_configurations(cx);
        cx.notify();
    }

    /// Keeps the folder's own configurations and the one chosen with the folder, as `runConfigurations.json`, their
    /// projects relative to it.
    pub(super) fn save_run_configurations(&self, cx: &App) {
        let Some(workspace) = &self.workspace else { return };
        let document = rsharp_core::run::configuration::RunConfigurations {
            configurations: self.run_own.clone(),
            selected: self.run_selected.clone(),
        };
        crate::state::store(cx).write_for(&workspace.folder, &document.for_store(&workspace.folder));
    }

    /// The folder's own run configurations, as Edit Configurations last kept them.
    pub fn run_own(&self) -> &[RunConfiguration] {
        &self.run_own
    }

    /// Run: the chosen configuration's plan is resolved, its project built as it says, and its program started.
    pub(super) fn run(&mut self, cx: &mut Context<Self>) {
        let Some(configuration) = self.chosen_run_configuration().cloned() else {
            self.refuse_run("Nothing to run: this folder has no run configuration", cx);
            return;
        };
        self.launch(configuration, cx);
    }

    /// Rerun: the program that runs is stopped first, and the same configuration starts again once it has ended.
    pub(super) fn rerun(&mut self, cx: &mut Context<Self>) {
        if self.is_running() {
            self.run_after_stop = true;
            self.stop_run(cx);
            return;
        }
        if let Some(plan) = self.run_plan.clone() {
            let configuration = self
                .run_configs
                .iter()
                .find(|configuration| configuration.name == plan.name)
                .cloned()
                .or_else(|| self.chosen_run_configuration().cloned());
            if let Some(configuration) = configuration {
                self.launch(configuration, cx);
                return;
            }
        }
        self.run(cx);
    }

    fn launch(&mut self, configuration: RunConfiguration, cx: &mut Context<Self>) {
        if self.run_cancel.is_some() {
            self.refuse_run("Another run is in progress", cx);
            return;
        }
        if self.workspace.is_none() {
            self.refuse_run("Nothing to run: no folder is open", cx);
            return;
        }
        if let Some(why) = self.why_not_run(&configuration) {
            self.refuse_run(&format!("{} cannot run here: {why}", configuration.name), cx);
            return;
        }
        if let Err(message) = self.save_all(cx) {
            self.refuse_run(&message, cx);
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
        self.run_generation += 1;
        let generation = self.run_generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.run_cancel = Some(cancel.clone());
        self.run_outcome = None;
        self.run_plan = None;
        self.run_after_build = None;
        self.run_after_stop = false;
        self.said = Some(format!("Building and running {}…", configuration.name).into());
        if !self.dock.is_open(Tool::Run) {
            self.dock.show(Tool::Run);
            self.save_state(cx);
        }
        self.run_console.update(cx, |view, cx| view.start(folder, cx));
        let (sender, mut receiver) = mpsc::channel(64);
        let resolving = sender.clone();
        std::thread::spawn(move || {
            let needed = rsharp_core::run::plan::needed_properties(&configuration);
            let resolved =
                rsharp_core::run::plan::properties(&configuration.project, configuration.framework.as_deref(), &needed)
                    .and_then(|properties| LaunchPlan::resolve(&configuration, &project, &properties));
            let mut resolving = resolving;
            let _ = resolving.try_send(Event::Resolved(resolved.map(Box::new)));
        });
        self._run_task = Some(cx.spawn(async move |this, cx| {
            while let Some(first) = receiver.next().await {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let mut events = vec![first];
                while let Some(event) = receiver.next().now_or_never().flatten() {
                    let done = is_terminal(&event);
                    events.push(event);
                    if done {
                        break;
                    }
                }
                let ended = events.iter().any(is_terminal);
                let sender = sender.clone();
                let landed = this.update(cx, |m, cx| {
                    if m.run_generation != generation {
                        return false;
                    }
                    for event in events {
                        m.run_event(event, &cancel, &sender, cx);
                    }
                    cx.notify();
                    !ended
                });
                if !landed.unwrap_or(false) {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn run_event(
        &mut self,
        event: Event,
        cancel: &Arc<AtomicBool>,
        sender: &mpsc::Sender<Event>,
        cx: &mut Context<Self>,
    ) {
        match event {
            Event::Output(text, error) => {
                self.run_console.update(cx, |view, cx| view.append(&[(text, error)], cx));
            }
            Event::Resolved(Ok(plan)) => {
                if cancel.load(Ordering::Acquire) {
                    self.finish_run(RunOutcome { status: RunStatus::Stopped, message: "Run stopped".into() }, cx);
                    return;
                }
                self.begin_launch(*plan, sender.clone(), cx);
            }
            Event::Resolved(Err(why)) => {
                self.finish_run(RunOutcome { status: RunStatus::Refused, message: why }, cx);
            }
            Event::Browser(url) => {
                let message = format!("Opening {url} in the browser…");
                self.said = Some(message.clone().into());
                self.run_console.update(cx, |view, cx| view.append(&[(format!("\n{message}\n"), false)], cx));
                (self.browser_opener)(url);
            }
            Event::NotResponding(message) => {
                self.said = Some(message.clone().into());
                self.run_console.update(cx, |view, cx| view.append(&[(format!("\n{message}\n"), true)], cx));
            }
            Event::Unstarted(message) => {
                self.finish_run(RunOutcome { status: RunStatus::Refused, message }, cx);
            }
            Event::Ended(code) => {
                let name = self.run_plan.as_ref().map(|plan| plan.name.clone()).unwrap_or_default();
                let outcome = match code {
                    Some(code) => RunOutcome {
                        status: RunStatus::Exited(code),
                        message: format!("{name} exited with code {code}"),
                    },
                    None => RunOutcome { status: RunStatus::Stopped, message: format!("{name} stopped") },
                };
                self.finish_run(outcome, cx);
            }
        }
    }

    /// The plan is resolved: its project builds first when it says so, and the program starts after it.
    fn begin_launch(&mut self, plan: LaunchPlan, sender: mpsc::Sender<Event>, cx: &mut Context<Self>) {
        self.run_plan = Some(plan.clone());
        match plan.build.clone() {
            Some(step) => {
                self.run_after_build = Some((plan, sender));
                self.implicit_build = true;
                let generation = self.build_generation;
                let said = self.said.clone();
                self.begin_build(rsharp_core::build::Kind::Build, Some(vec![step.project]), cx);
                // A build that did not start — another one running, a failed save — leaves this run waiting for a build
                // nothing will end, so the run ends here, saying what the build said rather than staying "running".
                if self.build_generation == generation
                    && let Some((plan, _)) = self.run_after_build.take()
                {
                    self.implicit_build = false;
                    let why = self
                        .said
                        .clone()
                        .filter(|now| Some(now) != said.as_ref())
                        .unwrap_or_else(|| "its build did not start".into());
                    let message = format!("{} did not start: {why}", plan.name);
                    self.finish_run(RunOutcome { status: RunStatus::Refused, message }, cx);
                }
            }
            None => self.start_plan(plan, sender, cx),
        }
    }

    /// The build before the launch has ended: the program starts when it succeeded, and a failed or stopped build — or
    /// one whose run was stopped meanwhile — starts nothing.
    pub(super) fn after_run_build(&mut self, ended: BuildStatus, cx: &mut Context<Self>) {
        let Some((plan, sender)) = self.run_after_build.take() else { return };
        let stopped = self.run_cancel.as_ref().is_some_and(|cancel| cancel.load(Ordering::Acquire));
        if stopped || ended == BuildStatus::Stopped {
            let outcome = RunOutcome { status: RunStatus::Stopped, message: format!("{} stopped", plan.name) };
            self.finish_run(outcome, cx);
            return;
        }
        match ended {
            BuildStatus::Succeeded => self.start_plan(plan, sender, cx),
            _ => {
                let message = format!("{} did not start: its build failed", plan.name);
                self.finish_run(RunOutcome { status: RunStatus::Refused, message }, cx);
            }
        }
    }

    fn start_plan(&mut self, plan: LaunchPlan, sender: mpsc::Sender<Event>, cx: &mut Context<Self>) {
        if let Some(why) = plan.why_not("run") {
            self.finish_run(RunOutcome { status: RunStatus::Refused, message: why }, cx);
            return;
        }
        let Some(cancel) = self.run_cancel.clone() else { return };
        let browser_cancel = cancel.clone();
        let line = format!("\n{}\n", display_command(&plan));
        self.run_console.update(cx, |view, cx| {
            view.append(&[(line, false)], cx);
            view.running = true;
            view.can_rerun = true;
            cx.notify();
        });
        self.said = Some(format!("Running {}…", plan.name).into());
        self.run_outcome =
            Some(RunOutcome { status: RunStatus::Running, message: format!("Running {}…", plan.name) });
        let mut output = sender.clone();
        let mut ended = sender.clone();
        let program = plan.clone();
        std::thread::spawn(move || {
            let cancel_for_output = cancel.clone();
            let mut partial = [String::new(), String::new()];
            let result = rsharp_core::build::run_while(
                program.command(),
                &cancel,
                || true,
                |text, error| {
                    let line = &mut partial[usize::from(error)];
                    line.push_str(text);
                    let mut chunks = String::new();
                    while let Some(end) = line.find('\n') {
                        chunks.extend(line.drain(..=end));
                    }
                    if !chunks.is_empty() {
                        send(&mut output, Event::Output(chunks, error), &cancel_for_output);
                    }
                },
            );
            for (index, rest) in partial.iter_mut().enumerate() {
                if !rest.is_empty() {
                    send(&mut output, Event::Output(std::mem::take(rest), index == 1), &cancel);
                }
            }
            match result {
                Ok(Some(code)) => send(&mut ended, Event::Ended(Some(code)), &cancel),
                Ok(None) => send(&mut ended, Event::Ended(None), &cancel),
                Err(error) => send(
                    &mut ended,
                    Event::Unstarted(format!("Could not start {}: {error}", program.program_name())),
                    &cancel,
                ),
            };
        });
        if let Some(url) = plan.browser_url.clone() {
            self.wait_for_browser(url, browser_cancel, sender, cx);
        }
    }

    /// The browser opens once something listens at the configuration's URL, never on a refused connection and never
    /// keeping the window waiting: the wait is a thread of its own, ended with the run.
    fn wait_for_browser(
        &mut self,
        url: String,
        cancel: Arc<AtomicBool>,
        mut sender: mpsc::Sender<Event>,
        cx: &mut Context<Self>,
    ) {
        let Some(address) = browser::Address::parse(&url) else {
            self.said = Some(format!("{url} is no address a browser opens").into());
            return;
        };
        std::thread::spawn(move || {
            if let Some(listening) = browser::listening(&address, &browser::Tries::default(), &cancel) {
                let event = if listening {
                    Event::Browser(address.shown())
                } else {
                    Event::NotResponding(address.not_responding())
                };
                send(&mut sender, event, &cancel);
            }
        });
        cx.notify();
    }

    fn finish_run(&mut self, outcome: RunOutcome, cx: &mut Context<Self>) {
        // The program has ended: a browser still waiting for its port stops waiting.
        if let Some(cancel) = self.run_cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.run_after_build = None;
        self.said = Some(outcome.message.clone().into());
        self.run_console.update(cx, |view, cx| {
            view.running = false;
            view.can_rerun = true;
            cx.notify();
        });
        self.run_outcome = Some(outcome);
        if std::mem::take(&mut self.run_after_stop) {
            self.rerun(cx);
        }
    }

    /// Stop: the program and everything it started end, and no awaiting start follows. A run waiting on its build stops
    /// that build too, and its plan stays until the build's end finishes the run as stopped.
    pub(super) fn stop_run(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = &self.run_cancel {
            cancel.store(true, Ordering::Release);
        }
        if self.run_after_build.is_some() && self.is_building() {
            self.cancel_build();
        }
        self.run_console.update(cx, |view, cx| {
            view.running = false;
            cx.notify();
        });
        cx.notify();
    }

    /// Stop, from the Run menu, Find Action or the console: whichever of a build and a run is under way ends.
    pub(super) fn stop_everything(&mut self, cx: &mut Context<Self>) {
        if self.is_building() {
            self.cancel_build();
        }
        if self.run_cancel.is_some() {
            self.stop_run(cx);
        }
        cx.notify();
    }

    fn refuse_run(&mut self, message: &str, cx: &mut Context<Self>) {
        self.run_console.update(cx, |view, cx| {
            view.running = false;
            view.can_rerun = true;
            view.append(&[(format!("{message}\n"), true)], cx);
        });
        self.said = Some(message.to_string().into());
        self.run_outcome = Some(RunOutcome { status: RunStatus::Refused, message: message.to_string() });
        if let Some(cancel) = self.run_cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.run_after_build = None;
        self.run_after_stop = false;
        cx.notify();
    }

    /// Leaving the folder: the program ends and the console goes with it.
    pub(super) fn leave_run(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = self.run_cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.run_generation += 1;
        self.run_after_build = None;
        self.run_after_stop = false;
        self.run_plan = None;
        self.run_outcome = None;
        self.run_own.clear();
        self.run_configs.clear();
        self.run_projects.clear();
        self.run_why_not.clear();
        self.run_selected = None;
        self.run_menu = false;
        self.run_dialog = None;
        self._run_task = None;
        self.run_console.update(cx, |view, cx| {
            view.running = false;
            view.can_rerun = false;
            view.clear(cx);
        });
    }

    /// Whether a run's console has the links its output carries: a URL the browser opens, a stack frame's place.
    pub(super) fn follow_console_link(
        &mut self,
        link: editor::ConsoleTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match link {
            editor::ConsoleTarget::Url(url) => {
                let message = format!("Opening {url} in the browser…");
                self.said = Some(message.into());
                (self.browser_opener)(url);
            }
            editor::ConsoleTarget::File { path, line, column } => {
                self.open_file(&path, Some((line, column)), window, cx)
            }
        }
        cx.notify();
    }

    /// Rider's Run/Debug Configurations dialog over the window: it edits the folder's own configurations, and Apply
    /// keeps them with the folder while OK closes it too.
    pub(super) fn edit_run_configurations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::run_dialog::{Ended, RunDialog};
        let Some(folder) = self.workspace.as_ref().map(|workspace| workspace.folder.clone()) else {
            self.said = Some("Nothing to configure: no folder is open".into());
            cx.notify();
            return;
        };
        let own = self.run_own.clone();
        let generated = RunConfiguration::generate(&self.run_projects);
        let chosen = self.chosen_run_configuration().map(|configuration| configuration.name.clone());
        let dialog =
            cx.new(|cx| RunDialog::new(&folder, &own, &generated, self.run_projects.clone(), chosen.as_deref(), cx));
        cx.subscribe_in(&dialog, window, |m, _, ended: &Ended, window, cx| match ended {
            Ended::Applied { own, chosen } => {
                m.run_own = own.clone();
                m.run_selected = chosen.clone().or_else(|| m.run_selected.clone());
                m.save_run_configurations(cx);
                m.refresh_run_configurations();
                cx.notify();
            }
            // OK has applied the list already and only says the dialog may close: the keys go back where they were.
            Ended::Dismissed => {
                m.run_dialog = None;
                m.focus_editor(window, cx);
            }
        })
        .detach();
        window.focus(&dialog.focus_handle(cx));
        self.run_dialog = Some(dialog);
        cx.notify();
    }

    /// The dialog open, for a test to fill its fields and press its buttons.
    pub fn run_dialog(&self) -> Option<Entity<crate::run_dialog::RunDialog>> {
        self.run_dialog.clone()
    }

    pub(super) fn run_actions<T: InteractiveElement + FluentBuilder>(&self, d: T, cx: &mut Context<Self>) -> T {
        d.when(self.workspace.is_some(), |d| {
            d.on_action(cx.listener(|m, _: &ToggleRun, window, cx| m.toggle_tool(Tool::Run, window, cx)))
                .on_action(
                    cx.listener(|m, _: &EditRunConfigurations, window, cx| m.edit_run_configurations(window, cx)),
                )
                .when(self.chosen_run_configuration().is_some() && !self.is_running(), |d| {
                    d.on_action(cx.listener(|m, _: &RunProgram, _, cx| m.run(cx)))
                })
                .when(self.chosen_run_configuration().is_some(), |d| {
                    d.on_action(cx.listener(|m, _: &RerunProgram, _, cx| m.rerun(cx)))
                })
                .when(self.is_running() || self.is_building(), |d| {
                    d.on_action(cx.listener(|m, _: &Stop, _, cx| m.stop_everything(cx)))
                })
        })
    }

    /// The toolbar's run widget: the chosen configuration, and the menu of every one.
    pub(super) fn run_widget(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self
            .chosen_run_configuration()
            .map(|configuration| configuration.name.clone())
            .unwrap_or_else(|| "No run configuration".to_string());
        let running = self.run_cancel.is_some();
        widget("run-widget").debug_selector(|| "run widget".into()).when(self.workspace.is_some(), |d| {
            d.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(icon(if running { "stop" } else { "run" }, 16.))
                .child(div().max_w(px(200.)).truncate().child(name))
                .child(icon("chevron-down", 12.))
                .on_click(cx.listener(|m, _: &ClickEvent, _, cx| {
                    m.run_menu = !m.run_menu;
                    cx.notify();
                }))
        })
    }

    pub(super) fn run_menu_view(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let chosen = self.chosen_run_configuration().map(|configuration| configuration.name.clone());
        let rows: Vec<gpui::AnyElement> = self
            .run_configs
            .iter()
            .enumerate()
            .map(|(i, configuration)| {
                let why = self.run_why_not.get(i).cloned().flatten();
                let name = configuration.name.clone();
                let selector = format!("run configuration: {name}");
                let checked = chosen.as_deref() == Some(name.as_str());
                let row = div()
                    .id(("run-menu", i))
                    .debug_selector(move || selector.clone())
                    .h(px(26.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(4.))
                    .when(why.is_none(), |d| d.hover(|d| d.bg(theme::selection())))
                    .when(why.is_some(), |d| d.opacity(0.5))
                    .child(div().size(px(14.)).when(checked, |d| d.child(icon("check", 14.))))
                    .child(div().truncate().child(name.clone()))
                    .when_some(why.clone(), |d, why| {
                        d.tooltip(move |_, cx| cx.new(|_| Tip(why.clone().into())).into())
                    });
                match why {
                    Some(_) => row.into_any_element(),
                    None => row
                        .on_click(
                            cx.listener(move |m, _: &ClickEvent, _, cx| m.choose_run_configuration(name.clone(), cx)),
                        )
                        .into_any_element(),
                }
            })
            .collect();
        // A folder with nothing to run still opens the dialog: a configuration is how a person makes one.
        let nothing = self.run_configs.is_empty().then(|| {
            div()
                .px(px(10.))
                .py(px(4.))
                .text_color(theme::text_secondary())
                .child("No run configuration: this folder has no executable project")
        });
        div()
            .id("run-menu")
            .w(px(320.))
            .max_h(px(420.))
            .overflow_y_scroll()
            .py(px(6.))
            .on_mouse_down_out(cx.listener(|m, _, _, cx| {
                m.run_menu = false;
                cx.notify();
            }))
            .children(nothing)
            .children(rows)
            .child(div().h(px(1.)).mx(px(8.)).my(px(4.)).bg(theme::popup_border()))
            .child(
                div()
                    .id("run-menu: edit")
                    .debug_selector(|| "run configuration: Edit Configurations…".into())
                    .h(px(26.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .hover(|d| d.bg(theme::selection()))
                    .child("Edit Configurations…")
                    .on_click(cx.listener(|m, _: &ClickEvent, window, cx| {
                        m.run_menu = false;
                        m.edit_run_configurations(window, cx);
                    })),
            )
            .into_any_element()
    }
}

/// Whether nothing follows this event: the run has ended, could not start, or its plan was refused.
fn is_terminal(event: &Event) -> bool {
    matches!(event, Event::Ended(_) | Event::Unstarted(_) | Event::Resolved(Err(_)))
}

fn is_project(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "csproj" | "fsproj" | "vbproj"))
}

/// The program and its arguments as the console shows them, each quoted where it holds a space.
fn display_command(plan: &LaunchPlan) -> String {
    let quote = |text: &str| {
        if text.contains(' ') { format!("\"{text}\"") } else { text.to_string() }
    };
    std::iter::once(quote(&plan.program.to_string_lossy()))
        .chain(plan.arguments.iter().map(|argument| quote(argument)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn send(sender: &mut mpsc::Sender<Event>, mut event: Event, cancel: &AtomicBool) -> bool {
    loop {
        let terminal = is_terminal(&event);
        match sender.try_send(event) {
            Ok(()) => return true,
            Err(error) if error.is_full() => {
                if !terminal && cancel.load(Ordering::Acquire) {
                    return false;
                }
                event = error.into_inner();
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return false,
        }
    }
}
