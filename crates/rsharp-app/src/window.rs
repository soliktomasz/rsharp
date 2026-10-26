//! The main window, laid out as Rider's Islands: the toolbar, a stripe and the status bar on the frame, and the
//! Solution view and the editor as islands over it. One folder or solution per window.

use crate::actions::ALL;
use crate::assets::{file_icon, icon};
use crate::buffer::Buffer;
use crate::dock::{DockLayout, Place, Rect, Tool};
use crate::editor::{self, CodeEditor};
use crate::editor_area::{EditorArea, Split};
use crate::find_action::{Ended, FindActionPopup};
use crate::find_bar::{self, FindBar};
use crate::solution_view::{NodeKind, SolutionTree, TreeRow};
use crate::state::{FolderState, KeptTab, RecentSolutions, store};
use crate::theme;
use crate::workspace::Workspace;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement, MouseButton,
    MouseDownEvent, PathPromptOptions, Pixels, Point, SharedString, Task, Window, actions, div, point, prelude::*, px,
    uniform_list,
};
use rsharp_lsp::{LanguageServer, ServerEvent, start_roslyn};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

actions!(
    window,
    [
        CloseTab,
        CloseSolution,
        FindAction,
        OpenFolder,
        NewSolution,
        NewProject,
        ToggleSolution,
        ToggleProblems,
        ToggleNuGet,
        ToggleBuild,
        ToggleRun,
        ToggleDebug,
        ShowBreakpoints,
        EditBreakpoint,
        AttachToProcess,
        ReattachToProcess,
        DetachDebug,
        EvaluateExpression,
        SetNextStatement,
        SmartStepInto,
        MuteBreakpoints,
        ToggleUnitTests,
        ToggleTerminal,
        ToggleCommit,
        ToggleGit,
        GitRefresh,
        GitPush,
        GitUpdate,
        GitHistory,
        GitSelectionHistory,
        GitCompare,
        GitResolve,
        GitContinue,
        GitAbort,
        NextChange,
        PreviousChange,
        RollbackLines,
        RunAllTests,
        RunTests,
        RerunFailedTests,
        RefreshTests,
        BuildSolution,
        RebuildSolution,
        CleanSolution,
        BuildSelected,
        RebuildSelected,
        CleanSelected,
        Stop,
        RunProgram,
        DebugProgram,
        ResumeDebug,
        PauseDebug,
        StepOver,
        StepInto,
        StepOut,
        StopDebug,
        ToggleBreakpoint,
        RerunProgram,
        EditRunConfigurations,
        ApplyHotReloadChanges,
        HideToolWindow,
        FloatToolWindow,
        RestoreDefaultLayout,
        SplitRight,
        SplitDown,
        Unsplit,
        UnsplitAll,
        PinTab,
        CloseOtherTabs,
        CloseAllTabs,
        CloseAllButPinned,
        CloseTabsToLeft,
        CloseTabsToRight,
        ReopenClosedTab,
        OpenTabInNewWindow,
        FileEncoding,
        LineSeparators,
        ShowHistory,
        ExplorerUp,
        ExplorerDown,
        ExplorerExpand,
        ExplorerCollapse,
        ExplorerOpen,
        SolutionNew,
        SolutionRename,
        SolutionDelete,
        SolutionCut,
        SolutionCopy,
        SolutionPaste,
        Back,
        Forward,
        LastEditLocation,
        RecentFiles,
        GoToFile,
        GoToType,
        GoToSymbol,
        FindInFiles,
        SearchEverywhere,
        RecentLocations,
        ReplaceInFiles
    ]
);

mod build;
mod debug;
mod file_moves;
mod file_ops;
mod git;
mod msbuild;
mod new_project;
mod nuget;
mod run;
mod solution_actions;
mod terminal;
mod tests;

pub use build::{BuildOutcome, BuildStatus};
pub use run::{RunOutcome, RunStatus};

use solution_actions::{NameFor, SolutionMenu};

const TOOLBAR_HEIGHT: f32 = 40.;
const STRIPE_WIDTH: f32 = 40.;
const GAP: f32 = 6.;
const ISLAND_RADIUS: f32 = 10.;
const ROW_HEIGHT: f32 = 24.;

/// What the window says about Roslyn in the status bar.
enum ServerState {
    None,
    Starting,
    Loading(String),
    Ready,
    Failed(String),
}

/// Problems' tabs, as Rider's: the shown file's, the whole solution's, and the toolset's (projects that would not load).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProblemsTab {
    File,
    Solution,
    Toolset,
    Build,
}

/// A place the caret was: its file, line and UTF-16 column, from 0.
// shortcut: a place keeps its line and column, so an edit above it since moves it off its code; carry it through
// the buffer's changes, as Passenger's NavigationHistory does, once that is missed.
#[derive(Clone, PartialEq, Debug)]
struct Spot {
    path: PathBuf,
    line: usize,
    column: usize,
}

impl Spot {
    /// Two places in a file a few lines apart are one, as IntelliJ counts them.
    fn near(&self, other: &Spot) -> bool {
        self.path == other.path && self.line.abs_diff(other.line) < 5
    }
}

/// IntelliJ's own limit on places kept each way (editor.navigation.history.stack.size).
const HISTORY: usize = 50;

/// A place in the Find window: its file, line and column (UTF-16, from 0), and its line's text.
struct Found {
    path: PathBuf,
    line: usize,
    column: usize,
    text: String,
}

pub struct MainWindow {
    workspace: Option<Workspace>,
    branch: Option<String>,
    git: git::GitState,
    expanded: HashSet<PathBuf>,
    rows: Vec<TreeRow>,
    /// The solution as the Solution view shows it; none for a folder without one, which shows its files.
    tree: Option<SolutionTree>,
    /// The Solution view's row menu, the name popup asking for New or Rename, and the last question asked.
    solution_menu_open: Option<SolutionMenu>,
    name_popup: Option<(Entity<crate::solution_menu::NamePopup>, NameFor)>,
    asked: Option<String>,
    reference_dialog: Option<(Entity<crate::reference_dialog::ReferenceDialog>, PathBuf)>,
    /// What Cut or Copy last put on the clipboard: its paths, whether cut, and the clipboard's text then.
    held: Option<(Vec<PathBuf>, bool, String)>,
    /// The open folder's watcher, and the task taking its bursts to the window.
    _watcher: Option<rsharp_core::watcher::Watcher>,
    _watch_task: Option<Task<()>>,
    selected: Option<usize>,
    dock: DockLayout,
    /// Where a floating window was taken, from its top left (the header) or bottom right (the corner), so it
    /// follows the pointer from where it was pressed rather than from where the drag began.
    float_grab: Point<Pixels>,
    opened_as: Option<PathBuf>,
    explorer_focus: FocusHandle,
    problems_focus: FocusHandle,
    find_focus: FocusHandle,
    /// The Find window's list: what was looked for, and each place found with its line's text.
    found: Option<(String, Vec<Found>)>,
    /// The window's own focus, where keys go when no view has them (the Welcome screen), so window-wide keys
    /// always reach the window.
    root_focus: FocusHandle,
    area: EditorArea,
    /// A tab's menu while open, where it was asked for.
    tab_menu: Option<Point<Pixels>>,
    /// The status bar's line separator or encoding menu while open.
    status_menu: Option<StatusMenu>,
    /// A tab being dragged, and whether the pointer is outside the window, where letting go opens it in a window.
    tab_drag: Option<(TabDrag, bool)>,
    server: Option<Arc<LanguageServer>>,
    server_state: ServerState,
    /// Roslyn's work under way, by its progress token, with its title.
    progress: Vec<(String, String)>,
    projects_loaded: bool,
    said: Option<SharedString>,
    find_action: Option<Entity<FindActionPopup>>,
    go_to: Option<Entity<crate::goto::GoToPopup>>,
    refactoring: Option<Entity<crate::refactor_dialog::RefactorDialog>>,
    problems_tab: ProblemsTab,
    /// Roslyn's errors and warnings in every file of the solution, as last asked.
    solution_problems: Vec<(PathBuf, Vec<rsharp_lsp::Problem>)>,
    /// Requests for them, counted; and the latest whose answer is shown. Roslyn holds a request open until something
    /// changes and may answer an older one first, so none is dropped and no older answer replaces a newer one.
    solution_asked: u64,
    solution_shown: u64,
    /// Rider's solution-wide analysis: every file analysed, not only the open ones; off until turned on, as it costs.
    solution_wide: bool,
    /// What Roslyn said last (its log, its notifications, its error output), for a failure to say where it was.
    roslyn_said: std::collections::VecDeque<String>,
    /// The projects Roslyn could not load whole: each project, whether it is an error, and what was wrong.
    load_problems: Vec<(PathBuf, bool, String)>,
    /// Back and Forward's places, the latest last.
    back: Vec<Spot>,
    forward: Vec<Spot>,
    /// The places edited, the latest last, and where Last Edit Location stands among them while pressed again.
    edits: Vec<Spot>,
    edit_step: Option<usize>,
    /// The files shown, the latest first, for Recent Files.
    // shortcut: not kept with the folder; keep it in FolderState once a restart's empty list is missed.
    recent: Vec<PathBuf>,
    /// The find bar over the editor while open, on the active tab's file.
    find_bar: Option<Entity<FindBar>>,
    /// The main menu's dropdown while open (Windows and Linux, which have no menu bar): each action's
    /// availability and keys where the person was, and what had the keys.
    main_menu: Option<(Vec<bool>, Vec<Option<SharedString>>, Option<FocusHandle>)>,
    _server_task: Option<Task<()>>,
    /// The folder's one Passenger Host, which Roslyn's builds and the Solution view's evaluations share.
    host: Option<rsharp_lsp::SharedHost>,
    /// A round of evaluations is running; `evaluate_again` asks for one more after it.
    evaluating: bool,
    evaluate_again: bool,
    _evaluate_task: Option<Task<()>>,
    creation_dialog: Option<gpui::WindowHandle<crate::new_project_dialog::NewProjectDialog>>,
    creation_opening: bool,
    _creation_subscription: Option<gpui::Subscription>,
    project_menu: bool,
    nuget: Entity<crate::nuget_view::NuGetView>,
    _nuget_subscription: gpui::Subscription,
    build: Entity<crate::console::ConsoleView>,
    build_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    build_generation: u64,
    _build_task: Option<Task<()>>,
    last_build: Option<(rsharp_core::build::Kind, Option<Vec<PathBuf>>)>,
    build_problems: Vec<rsharp_core::build::Problem>,
    build_outcome: Option<build::BuildOutcome>,
    /// The build under way is a run's, which only a failure brings forward.
    implicit_build: bool,
    terminal: Entity<crate::terminal::TerminalPanel>,
    run_console: Entity<crate::console::ConsoleView>,
    /// The run configurations of the folder's own, from `runConfigurations.json`, then the ones its projects make: made
    /// again as its projects and their profiles change, with the projects they came from and why each cannot launch, so
    /// a drawing of the toolbar's menu reads these and never the disk.
    run_own: Vec<rsharp_core::run::RunConfiguration>,
    run_configs: Vec<rsharp_core::run::RunConfiguration>,
    run_projects: Vec<rsharp_core::project::EvaluatedProject>,
    run_why_not: Vec<Option<String>>,
    /// The configuration chosen in the toolbar, by name; the first stands while it is gone.
    run_selected: Option<String>,
    /// Whether the toolbar's run menu is open.
    run_menu: bool,
    run_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    run_generation: u64,
    _run_task: Option<Task<()>>,
    /// The plan Run last resolved, and the one whose build is under way.
    run_plan: Option<rsharp_core::run::LaunchPlan>,
    run_after_build: Option<(rsharp_core::run::LaunchPlan, mpsc::Sender<run::Event>)>,
    /// Rerun was pressed while the program ran: it starts again once the program has ended.
    run_after_stop: bool,
    /// The program that runs was started under `dotnet watch`, so a save reaches it (Settings' Hot Reload).
    run_watching: bool,
    run_outcome: Option<run::RunOutcome>,
    /// How the browser is opened; a test takes it over, so no browser opens.
    browser_opener: Arc<dyn Fn(String) + Send + Sync>,
    /// Rider's Run/Debug Configurations dialog while it is open.
    run_dialog: Option<Entity<crate::run_dialog::RunDialog>>,
    /// Rider's Breakpoints dialog while it is open.
    breakpoints_dialog: Option<Entity<crate::breakpoints_dialog::BreakpointsDialog>>,
    /// Rider's breakpoint popup over the editor, and the line it edits.
    breakpoint_popup: Option<(Entity<crate::breakpoint_popup::BreakpointPopup>, PathBuf, usize)>,
    /// Rider's Attach to Process dialog while it is open.
    attach_dialog: Option<Entity<crate::attach_dialog::AttachDialog>>,
    /// Rider's Evaluate Expression dialog while it is open.
    evaluate_dialog: Option<Entity<crate::evaluate_dialog::EvaluateDialog>>,
    /// Rider's Smart Step Into dialog while it is open.
    step_dialog: Option<Entity<crate::step_into_dialog::StepIntoDialog>>,
    /// A question with a few answers while it asks one: Hot Reload's restart for an edit the runtime cannot take.
    choice_dialog: Option<Entity<crate::choice_dialog::ChoiceDialog>>,
    /// The buffer version whose breakpoints last followed its edit, so two editors of one file move them once.
    followed_edit: Option<(gpui::EntityId, u64)>,
    tests: tests::Tests,
    /// This window's own handle, so a task without a window (a build's end) reaches one to open a stopped file.
    handle: gpui::WindowHandle<MainWindow>,
    debug: debug::DebugState,
    debug_view: Entity<crate::debug_view::DebugView>,
    debug_console: Entity<crate::console::ConsoleView>,
}

impl Drop for MainWindow {
    fn drop(&mut self) {
        self.cancel_build();
        self.cancel_tests();
        if let Some(cancel) = &self.run_cancel {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
    }
}

impl MainWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let owner = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            let _ = owner.update(cx, |m, cx| {
                m.cancel_build();
                m.stop_run(cx);
                m.leave_debug(cx);
                m.cancel_tests();
                m.leave_git(cx);
                m.terminal.update(cx, |t, cx| t.close_all(cx));
                if let Some(handle) = m.creation_dialog {
                    let _ = handle.update(cx, |d, w, cx| d.cancel(w, cx));
                }
            });
            true
        });
        let terminal = cx.new(crate::terminal::TerminalPanel::new);
        cx.subscribe_in(&terminal, window, |m, _, event: &crate::terminal::TerminalEvent, window, cx| {
            match event {
                crate::terminal::TerminalEvent::Empty => {
                    m.dock.open.retain(|t| *t != Tool::Terminal);
                    if let Some(e) = m.active_editor() {
                        window.focus(&e.focus_handle(cx));
                    }
                }
                crate::terminal::TerminalEvent::Link(link) => m.follow_terminal_link(link.clone(), window, cx),
            }
            cx.notify();
        })
        .detach();
        let root_focus = cx.focus_handle();
        window.focus(&root_focus);
        let nuget = cx.new(crate::nuget_view::NuGetView::new);
        let build = cx.new(|cx| crate::console::ConsoleView::new("build", cx));
        let run_console = cx.new(|cx| crate::console::ConsoleView::new("run", cx));
        let test_view = cx.new(crate::test_view::TestView::new);
        cx.subscribe_in(&test_view, window, |m, _, action: &crate::test_view::TestAction, window, cx| {
            m.test_action(action, window, cx);
        })
        .detach();
        cx.subscribe_in(&build, window, |m, _, action: &crate::console::ConsoleAction, _, cx| {
            match action {
                crate::console::ConsoleAction::Rerun => {
                    if let Some((kind, targets)) = m.last_build.clone() {
                        m.begin_build(kind, targets, cx);
                    }
                }
                crate::console::ConsoleAction::Stop => m.stop_everything(cx),
                crate::console::ConsoleAction::Input(_) => {}
            }
            cx.notify();
        })
        .detach();
        cx.subscribe_in(&run_console, window, |m, _, action: &crate::console::ConsoleAction, _, cx| {
            match action {
                crate::console::ConsoleAction::Rerun => m.rerun(cx),
                crate::console::ConsoleAction::Stop => m.stop_run(cx),
                crate::console::ConsoleAction::Input(_) => {}
            }
            cx.notify();
        })
        .detach();
        let run_output = run_console.read(cx).editor().clone();
        cx.subscribe_in(&run_output, window, |m, _, link: &editor::ConsoleLink, window, cx| {
            m.follow_console_link(link.0.clone(), window, cx)
        })
        .detach();
        let console = build.read(cx).editor().clone();
        cx.subscribe_in(&console, window, |m, _, link: &editor::ConsoleLink, window, cx| {
            m.follow_console_link(link.0.clone(), window, cx);
        })
        .detach();
        let debug_console = cx.new(|cx| crate::console::ConsoleView::new("debug", cx));
        let debug_view = cx.new(|cx| crate::debug_view::DebugView::new(debug_console.clone(), cx));
        cx.subscribe_in(&debug_view, window, |m, _, action: &crate::debug_view::DebugAction, window, cx| {
            m.debug_action(action.clone(), window, cx)
        })
        .detach();
        cx.subscribe_in(&debug_console, window, |m, _, action: &crate::console::ConsoleAction, _, cx| {
            match action {
                crate::console::ConsoleAction::Input(text) => m.debug_input(text, cx),
                crate::console::ConsoleAction::Stop => m.stop_debug(cx),
                crate::console::ConsoleAction::Rerun => {}
            }
            cx.notify();
        })
        .detach();
        let debug_output = debug_console.read(cx).editor().clone();
        cx.subscribe_in(&debug_output, window, |m, _, link: &editor::ConsoleLink, window, cx| {
            m.follow_console_link(link.0.clone(), window, cx)
        })
        .detach();
        let _nuget_subscription = cx.subscribe(&nuget, |m, _, wrote: &crate::nuget_view::Wrote, cx| {
            for path in &wrote.0 {
                m.follow_written(path, cx);
            }
        });
        let git = git::GitState::new(cx);
        for view in [&git.commit, &git.log] {
            cx.subscribe_in(view, window, |m, _, request: &crate::git_view::GitRequest, window, cx| {
                m.git_request(request.clone(), window, cx);
            })
            .detach();
        }
        let draft_buffer = git.commit.read(cx).message_editor().read(cx).buffer.clone();
        cx.observe(&draft_buffer, |m, _, cx| m.save_git_choices(cx)).detach();
        cx.observe(&git.log, |m, _, cx| m.save_git_choices(cx)).detach();
        Self {
            git,
            tests: tests::Tests::new(test_view),
            build,
            terminal,
            run_console,
            run_own: Vec::new(),
            run_configs: Vec::new(),
            run_projects: Vec::new(),
            run_why_not: Vec::new(),
            run_selected: None,
            run_menu: false,
            run_cancel: None,
            run_generation: 0,
            _run_task: None,
            run_plan: None,
            run_after_build: None,
            run_after_stop: false,
            run_watching: false,
            run_outcome: None,
            run_dialog: None,
            breakpoints_dialog: None,
            breakpoint_popup: None,
            attach_dialog: None,
            evaluate_dialog: None,
            step_dialog: None,
            choice_dialog: None,
            followed_edit: None,
            handle: window.window_handle().downcast::<MainWindow>().expect("this window is a MainWindow"),
            debug: debug::DebugState::new(),
            debug_view,
            debug_console,
            browser_opener: Arc::new(|url| {
                if let Err(error) = rsharp_core::run::browser::open(&url) {
                    log::warn!("Could not open {url}: {error}");
                }
            }),
            build_cancel: None,
            build_generation: 0,
            _build_task: None,
            last_build: None,
            build_problems: Vec::new(),
            build_outcome: None,
            implicit_build: false,
            nuget,
            _nuget_subscription,
            root_focus,
            workspace: None,
            branch: None,
            expanded: HashSet::new(),
            rows: Vec::new(),
            tree: None,
            solution_menu_open: None,
            name_popup: None,
            asked: None,
            reference_dialog: None,
            held: None,
            _watcher: None,
            _watch_task: None,
            selected: None,
            dock: store(cx).read::<DockLayout>().complete().clamped(),
            float_grab: Point::default(),
            opened_as: None,
            explorer_focus: cx.focus_handle(),
            problems_focus: cx.focus_handle(),
            find_focus: cx.focus_handle(),
            found: None,
            area: EditorArea::default(),
            tab_menu: None,
            tab_drag: None,
            status_menu: None,
            server: None,
            server_state: ServerState::None,
            progress: Vec::new(),
            projects_loaded: false,
            said: None,
            find_action: None,
            go_to: None,
            refactoring: None,
            problems_tab: ProblemsTab::File,
            solution_problems: Vec::new(),
            solution_asked: 0,
            solution_shown: 0,
            solution_wide: false,
            roslyn_said: Default::default(),
            load_problems: Vec::new(),
            back: Vec::new(),
            forward: Vec::new(),
            edits: Vec::new(),
            edit_step: None,
            recent: Vec::new(),
            find_bar: None,
            main_menu: None,
            _server_task: None,
            host: None,
            evaluating: false,
            evaluate_again: false,
            _evaluate_task: None,
            creation_dialog: None,
            creation_opening: false,
            _creation_subscription: None,
            project_menu: false,
        }
    }

    pub fn open_workspace(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_tests(cx);
        self.leave_git(cx);
        self.leave_build(cx);
        self.leave_run(cx);
        self.leave_debug(cx);
        self.terminal.update(cx, |t, cx| t.close_all(cx));
        self.save_state(cx);
        self.leave_editors();
        let path = rsharp_core::paths::canonical(path);
        let workspace = Workspace::open(&path);
        window.set_window_title(&workspace.name());
        let store = store(cx);
        let mut recent: RecentSolutions = store.read();
        recent.opened(path.clone(), workspace.name());
        self.opened_as = Some(path);
        store.write(&recent);

        let kept: FolderState = store.read_for(&workspace.folder);
        let folder = workspace.folder.clone();
        self.branch = None;
        self.expanded = std::iter::once(folder.clone()).chain(kept.expanded.iter().map(|p| folder.join(p))).collect();
        self.dock = kept.dock.clone().unwrap_or_else(|| store.read::<DockLayout>()).complete().clamped();
        self.solution_wide = kept.solution_wide;
        self.tree = workspace.solution.as_deref().and_then(rsharp_core::solution::read).map(|solution| {
            let cached: rsharp_core::project::Evaluations = store.read_for(&folder);
            let evaluated = cached.projects.into_iter().map(|p| (p.path.clone(), p)).collect();
            SolutionTree::new(solution, evaluated, kept.open.iter().cloned().collect())
        });
        self.start_watching(&folder, cx);
        // Install the shared client before either a build or Roslyn can start, so both hold the same Host.
        self.host =
            cx.global::<crate::state::Ide>().servers.then(|| rsharp_lsp::SharedHost::carried(&folder)).flatten();
        self.load_breakpoints(&folder, cx);
        self.workspace = Some(workspace);
        self.open_git(cx);
        let test_state = store.read_for::<crate::test_view::UnitTestsState>(&folder);
        self.tests.view.update(cx, |view, cx| view.restore(test_state, cx));
        self.selected = Some(0);
        let kept_runs: rsharp_core::run::configuration::RunConfigurations = store.read_for(&folder);
        let kept_runs = kept_runs.resolved(&folder);
        self.run_own = kept_runs.configurations;
        self.run_selected = kept_runs.selected.filter(|name| !name.is_empty());
        self.rebuild_rows();
        self.start_server(cx);
        window.focus(&self.explorer_focus);
        for tab in &kept.tabs {
            let file = folder.join(&tab.path);
            if file.is_file() {
                self.open_file(&file, Some((tab.line, tab.column)), window, cx);
                if tab.pinned
                    && let Some(at) = self.area.tab_of(0, &rsharp_core::paths::canonical(&file), cx)
                {
                    self.area.pin(0, at, true);
                }
            }
        }
        if let Some(active) = kept.active.filter(|a| *a < self.area.group().tabs.len()) {
            self.show_tab(0, active, window, cx);
        }
        self.sync_nuget(cx);
        self.sync_tests(cx);
        cx.notify();
    }

    /// Keeps the folder's tabs, carets and Solution view for the next time it opens.
    fn save_state(&self, cx: &App) {
        let Some(workspace) = &self.workspace else { return };
        let folder = &workspace.folder;
        let relative = |p: &Path| p.strip_prefix(folder).ok().map(Path::to_path_buf);
        // shortcut: the splits' tabs are kept as one group's; keep the layout too once a person misses it.
        let mut tabs: Vec<KeptTab> = Vec::new();
        for tab in self.area.groups.iter().flat_map(|g| &g.tabs) {
            let editor = tab.editor.read(cx);
            let caret = editor.caret();
            if let Some(path) = relative(&editor.path).filter(|p| tabs.iter().all(|t| t.path != *p)) {
                tabs.push(KeptTab { path, line: caret.line, column: caret.utf16_column, pinned: tab.pinned });
            }
        }
        let active = self
            .area
            .active_editor()
            .and_then(|e| relative(&e.read(cx).path))
            .and_then(|p| tabs.iter().position(|t| t.path == p));
        let mut expanded: Vec<PathBuf> =
            self.expanded.iter().filter_map(|p| relative(p)).filter(|p| !p.as_os_str().is_empty()).collect();
        expanded.sort();
        let store = store(cx);
        store.write_for(
            folder,
            &FolderState {
                tabs,
                active,
                expanded,
                open: self.tree.as_ref().map(SolutionTree::open_keys).unwrap_or_default(),
                dock: Some(self.dock.clone()),
                solution_wide: self.solution_wide,
            },
        );
        store.write(&self.dock);
    }

    fn show_tab(&mut self, group: usize, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.area.current = group;
        self.area.groups[group].active = Some(index);
        let path = self.area.groups[group].tabs[index].editor.read(cx).path.clone();
        self.recent.retain(|p| *p != path);
        self.recent.insert(0, path);
        self.recent.truncate(HISTORY);
        window.focus(&self.area.groups[group].tabs[index].editor.focus_handle(cx));
        self.follow_find_bar(cx);
        self.save_state(cx);
        cx.notify();
    }

    fn start_server(&mut self, cx: &mut Context<Self>) {
        self.server = None;
        (self.evaluating, self.evaluate_again, self._evaluate_task) = (false, false, None);
        let Some(workspace) = &self.workspace else { return };
        if !cx.global::<crate::state::Ide>().servers {
            self.server_state = ServerState::None;
            return;
        }
        let has_projects = workspace.solution.is_some()
            || std::fs::read_dir(&workspace.folder)
                .into_iter()
                .flatten()
                .flatten()
                .any(|e| e.path().extension().is_some_and(|x| x == "csproj"));
        if !has_projects {
            self.server_state = ServerState::None;
            return;
        }
        self.server_state = ServerState::Starting;
        self.progress.clear();
        self.projects_loaded = false;
        let (folder, solution) = (workspace.folder.clone(), workspace.solution.clone());
        let logs = store(cx).root().join("logs").join("roslyn");
        let host = self.host.clone();
        let (sender, mut events) = futures::channel::mpsc::unbounded();
        self._server_task = Some(cx.spawn(async move |this, cx| {
            let given = host.clone();
            if this
                .update(cx, |w, cx| {
                    w.host = given;
                    w.evaluate_projects(cx);
                    w.attach_msbuild_files(cx);
                })
                .is_err()
            {
                return;
            }
            let started = cx
                .background_executor()
                .spawn(async move {
                    start_roslyn(&folder, solution.as_deref(), &logs, host, move |event| {
                        let _ = sender.unbounded_send(event);
                    })
                })
                .await;
            let server = match started {
                Ok(server) => Arc::new(server),
                Err(failed) => {
                    log::error!("Roslyn did not start: {failed}");
                    let _ = this.update(cx, |w, cx| {
                        w.server_state = ServerState::Failed(failed.0);
                        cx.notify()
                    });
                    return;
                }
            };
            let _ = this.update(cx, |w, cx| {
                if w.solution_wide {
                    for option in rsharp_lsp::SOLUTION_WIDE {
                        server.set_option(option, serde_json::json!("fullSolution"));
                    }
                }
                w.server = Some(server.clone());
                w.server_state = ServerState::Loading("Loading solution".into());
                let editors: Vec<_> = w.area.editors().cloned().collect();
                for editor in editors {
                    if is_csharp(&editor.read(cx).path) {
                        editor.read(cx).buffer.clone().update(cx, |b, cx| b.attach_server(server.clone(), cx));
                    }
                }
                cx.notify();
            });
            while let Some(event) = events.next().await {
                let alive = this.update(cx, |w, cx| {
                    w.on_server_event(event, cx);
                    cx.notify()
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    fn on_server_event(&mut self, event: ServerEvent, cx: &mut Context<Self>) {
        if rsharp_lsp::says_projects_loaded(&event) && !self.projects_loaded {
            self.projects_loaded(cx);
        }
        let said = match &event {
            ServerEvent::Notification { method, params } if method == "window/logMessage" => {
                params["message"].as_str().unwrap_or_default().chars().take(300).collect()
            }
            ServerEvent::Notification { method, params } => {
                format!("{method} {}", params.to_string().chars().take(160).collect::<String>())
            }
            ServerEvent::Stderr(line) => format!("stderr: {}", line.chars().take(300).collect::<String>()),
            ServerEvent::Ended => "Roslyn ended".into(),
        };
        if self.roslyn_said.len() == 50 {
            self.roslyn_said.pop_front();
        }
        self.roslyn_said.push_back(said);
        match event {
            ServerEvent::Notification { method, params } => match method.as_str() {
                // Followed by token: Roslyn's restore can begin after it said the projects loaded.
                "$/progress" => {
                    let token = params["token"].to_string();
                    let value = &params["value"];
                    let message = value["message"].as_str().unwrap_or_default();
                    match value["kind"].as_str() {
                        // A restore is held until it names a project: Roslyn also restores its own stand-in for
                        // files no project holds (temp's roslyn-canonical-misc/…/Canonical.csproj), which is
                        // never shown or waited for, as Passenger's ProjectLoad holds it.
                        Some("begin") if value["title"] == "Restore" => {}
                        Some("begin") => {
                            let title = value["title"].as_str().unwrap_or("Working").to_string();
                            self.progress.retain(|(t, _)| *t != token);
                            self.progress.push((token, title));
                        }
                        Some("report") if message.starts_with("Restoring ") => {
                            self.progress.retain(|(t, _)| *t != token);
                            if !message.contains("Canonical.csproj") {
                                self.progress.push((token, message.to_string()));
                            }
                        }
                        Some("end") => {
                            log::info!(target: "roslyn", "{} done", self.progress.iter().find(|(t, _)| *t == token).map_or("work", |(_, title)| title.as_str()));
                            self.progress.retain(|(t, _)| *t != token);
                        }
                        _ => {}
                    }
                    self.server_state = match (self.progress.last(), self.projects_loaded) {
                        (Some((_, title)), _) => ServerState::Loading(title.clone()),
                        (None, true) => ServerState::Ready,
                        (None, false) => ServerState::Loading("Loading solution".into()),
                    };
                }
                // An answer asked before Roslyn read solution-wide analysis's scope was the open files' only.
                "workspace/configuration" => self.refresh_solution_problems(cx),
                // Roslyn's answers about the files changed (its load went on, a file on disk changed): asked again.
                "workspace/semanticTokens/refresh" | "workspace/inlayHint/refresh" | "workspace/diagnostic/refresh" => {
                    if method == "workspace/diagnostic/refresh" {
                        self.refresh_solution_problems(cx);
                    }
                    let editors: Vec<_> = self.area.editors().cloned().collect();
                    for editor in editors {
                        if method == "workspace/diagnostic/refresh" {
                            editor.read(cx).buffer.clone().update(cx, |b, cx| b.check(std::time::Duration::ZERO, cx));
                        } else {
                            editor.update(cx, |e, cx| e.server_ready(cx));
                        }
                    }
                }
                "window/logMessage" if self.read_load_report(params["message"].as_str().unwrap_or_default(), cx) => {}
                "window/logMessage" if params["type"].as_u64().is_some_and(|t| t <= 2) => {
                    log::warn!(target: "roslyn", "{}", params["message"].as_str().unwrap_or_default());
                }
                "window/showMessage" if params["type"].as_u64() == Some(1) => {
                    self.said = params["message"].as_str().map(|m| m.to_string().into());
                }
                _ => {}
            },
            ServerEvent::Stderr(line) => log::warn!(target: "roslyn", "{line}"),
            ServerEvent::Ended => {
                log::warn!("Roslyn stopped");
                self.server = None;
                self.server_state = ServerState::Failed("Roslyn stopped".into());
            }
        }
    }

    /// Roslyn has the projects: each open file's problems and colours asked again.
    fn projects_loaded(&mut self, cx: &mut Context<Self>) {
        self.refresh_solution_problems(cx);
        log::info!(target: "roslyn", "projects loaded");
        self.projects_loaded = true;
        if self.progress.is_empty() {
            self.server_state = ServerState::Ready;
        }
        self.reopen_files(cx);
    }

    /// Every open file opened in Roslyn again, its problems and colours asked again: a file opened while a project
    /// was loading sits in Roslyn's stand-in project, and with the whole solution analysed it stays there until it
    /// is opened again.
    fn reopen_files(&mut self, cx: &mut Context<Self>) {
        let server = self.server.clone();
        let editors: Vec<_> = self.area.editors().cloned().collect();
        for editor in editors {
            editor.read(cx).buffer.clone().update(cx, |b, cx| {
                if let Some(server) = server.clone().filter(|_| is_csharp(&b.path)) {
                    b.detach_server();
                    b.attach_server(server, cx);
                }
                b.check(std::time::Duration::ZERO, cx)
            });
            editor.update(cx, |e, cx| e.server_ready(cx));
        }
    }

    /// A line of Roslyn's log that says how a project loaded: a problem kept for Problems' Toolset, or a project
    /// loaded whole, its problems gone. Roslyn's own stand-in for files no project holds is never one.
    fn read_load_report(&mut self, line: &str, cx: &mut Context<Self>) -> bool {
        if line.contains("Canonical.csproj") {
            return false;
        }
        if let Some((project, error, message)) = rsharp_lsp::load_problem(line) {
            log::warn!(target: "roslyn", "{line}");
            let project = rsharp_core::paths::canonical(&project);
            self.load_problems.retain(|(p, _, m)| !(*p == project && *m == message));
            self.load_problems.push((project, error, message));
            return true;
        }
        if let Some(loaded) = line.split("Successfully completed load of ").nth(1) {
            let loaded = rsharp_core::paths::canonical(Path::new(loaded.trim()));
            self.load_problems.retain(|(p, _, _)| *p != loaded);
            // A project, not a file loaded as a program of its own, which reopening would load again.
            if loaded.extension().is_some_and(|e| e.to_string_lossy().ends_with("proj")) {
                self.reopen_files(cx);
            }
        }
        false
    }

    /// Asks Roslyn for the whole solution's problems, for Problems' Solution tab.
    fn refresh_solution_problems(&mut self, cx: &mut Context<Self>) {
        let Some(server) = self.server.clone().filter(|_| self.solution_wide) else { return };
        let answer =
            crate::language::ask(&server, "workspace/diagnostic", serde_json::json!({ "previousResultIds": [] }), cx);
        self.solution_asked += 1;
        let asked = self.solution_asked;
        cx.spawn(async move |this, cx| {
            let Some(answer) = answer.await else { return };
            let found = cx.background_executor().spawn(async move { rsharp_lsp::workspace_problems(&answer) }).await;
            let _ = this.update(cx, |w, cx| {
                if !w.solution_wide || asked <= w.solution_shown {
                    return;
                }
                w.solution_shown = asked;
                w.solution_problems = found;
                w.solution_problems.sort_by(|a, b| a.0.cmp(&b.0));
                cx.notify();
            });
        })
        .detach();
    }

    /// Turns solution-wide analysis on or off: Roslyn told, and the Solution tab asked again or emptied.
    pub fn set_solution_wide(&mut self, on: bool, cx: &mut Context<Self>) {
        self.solution_wide = on;
        if let Some(server) = &self.server {
            let scope = if on { serde_json::json!("fullSolution") } else { serde_json::Value::Null };
            for option in rsharp_lsp::SOLUTION_WIDE {
                server.set_option(option, scope.clone());
            }
        }
        self.solution_problems.clear();
        // An answer to a request made before this no longer belongs to the tab.
        self.solution_shown = self.solution_asked;
        if on {
            self.refresh_solution_problems(cx);
        }
        self.save_state(cx);
        cx.notify();
    }

    /// The Problems tab shown, as a test or a click chooses it.
    pub fn show_problems_tab(&mut self, tab: ProblemsTab, cx: &mut Context<Self>) {
        self.problems_tab = tab;
        if tab == ProblemsTab::Solution {
            self.refresh_solution_problems(cx);
        }
        cx.notify();
    }

    /// What a Problems tab lists, each as "file:line message", as a person reads it.
    pub fn problems_listed(&self, tab: ProblemsTab, cx: &App) -> Vec<String> {
        let name = |p: &Path| p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        match tab {
            ProblemsTab::File => self
                .active_editor()
                .map(|e| {
                    let buffer = e.read(cx).buffer.read(cx);
                    buffer
                        .diagnostics
                        .iter()
                        .map(|d| {
                            format!(
                                "{}:{} {}",
                                name(&buffer.path),
                                buffer.document.position(d.range.start).line + 1,
                                d.message
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            ProblemsTab::Solution => self
                .solution_problems
                .iter()
                .flat_map(|(path, found)| {
                    found.iter().map(move |p| format!("{}:{} {}", name(path), p.span.start.line + 1, p.message))
                })
                .collect(),
            ProblemsTab::Toolset => self.load_problems.iter().map(|(p, _, m)| format!("{} {m}", name(p))).collect(),
            ProblemsTab::Build => self
                .build_problems
                .iter()
                .map(|problem| {
                    format!(
                        "{}:{} {}: {}",
                        problem.path.as_deref().map(&name).unwrap_or_default(),
                        problem.line + 1,
                        problem.code,
                        problem.message
                    )
                })
                .collect(),
        }
    }

    fn rebuild_rows(&mut self) {
        // The selection follows its row, which an evaluation landing above it moves.
        let selected = self.selected.and_then(|at| self.rows.get(at)).map(|r| r.key.clone());
        if let Some(tree) = &self.tree {
            self.rows = tree.rows().to_vec();
        } else {
            self.rebuild_folder_rows();
        }
        let found = selected.and_then(|key| self.rows.iter().position(|r| r.key == key));
        if let Some(at) = self.selected {
            self.selected = Some(found.unwrap_or(at).min(self.rows.len().saturating_sub(1)));
        }
        // A project's evaluation says what it builds, and its launch profiles say what runs: the list follows both.
        self.refresh_run_configurations();
    }

    /// A folder without a solution: its files on disk.
    fn rebuild_folder_rows(&mut self) {
        let Some(workspace) = &mut self.workspace else { return };
        let root = workspace.folder.clone();
        let row = |path: PathBuf, name: String, is_folder: bool, depth: usize, expanded: bool| TreeRow {
            kind: if is_folder { NodeKind::Folder } else { NodeKind::File },
            name,
            detail: None,
            depth,
            expandable: is_folder,
            expanded,
            dimmed: false,
            tip: None,
            key: path.display().to_string(),
            path: Some(path),
        };
        let mut rows = vec![row(root.clone(), workspace.name(), true, 0, self.expanded.contains(&root))];
        fn walk(
            workspace: &mut Workspace,
            expanded: &HashSet<PathBuf>,
            folder: &Path,
            depth: usize,
            rows: &mut Vec<TreeRow>,
            row: &dyn Fn(PathBuf, String, bool, usize, bool) -> TreeRow,
        ) {
            for entry in workspace.entries(folder).to_vec() {
                let open = entry.is_folder && expanded.contains(&entry.path);
                rows.push(row(entry.path.clone(), entry.name, entry.is_folder, depth, open));
                if open {
                    walk(workspace, expanded, &entry.path, depth + 1, rows, row);
                }
            }
        }
        if rows[0].expanded {
            walk(workspace, &self.expanded, &root, 1, &mut rows, &row);
        }
        self.rows = rows;
    }

    /// Evaluates, in the Host, each project of the solution never evaluated or changed since, the active file's first,
    /// each landing in the Solution view and the folder's cache as it comes. One round runs at a time.
    fn evaluate_projects(&mut self, cx: &mut Context<Self>) {
        let (Some(host), Some(tree), Some(workspace)) = (self.host.clone(), &self.tree, &self.workspace) else {
            return;
        };
        if self.evaluating {
            self.evaluate_again = true;
            return;
        }
        let folder = workspace.folder.clone();
        let solution = tree.solution().path.clone();
        let active = self.active_editor().and_then(|e| rsharp_lsp::project_of(&e.read(cx).path));
        let mut projects: Vec<(PathBuf, Option<rsharp_core::project::EvaluatedProject>)> = tree
            .solution()
            .projects()
            .into_iter()
            .filter(|p| p.exists)
            .map(|p| (p.path.clone(), tree.evaluated(&p.path).cloned()))
            .collect();
        projects.sort_by_key(|(p, _)| Some(p) != active.as_ref());
        self.evaluating = true;
        self._evaluate_task = Some(cx.spawn(async move |this, cx| {
            for (project, known) in projects {
                let (host, solution) = (host.clone(), solution.clone());
                let evaluated = cx
                    .background_executor()
                    .spawn(async move {
                        // Stamped files are read here, off the UI thread.
                        if known.is_some_and(|k| !k.is_stale()) {
                            return None;
                        }
                        Some(host.evaluate(&project, Some(&solution)))
                    })
                    .await;
                let landed = this.update(cx, |w, cx| {
                    if w.workspace.as_ref().map(|ws| &ws.folder) != Some(&folder) {
                        return false;
                    }
                    match evaluated {
                        Some(Ok(project)) => w.set_evaluated(project, cx),
                        Some(Err(failed)) => {
                            // A Host that cannot start or answer fails the same for every project: the round ends
                            // here, saying why, rather than starting it again for each.
                            log::warn!("{}", failed.0);
                            if let Some(tree) = &mut w.tree {
                                tree.set_problem(Some(failed.0));
                            }
                            w.rebuild_rows();
                            cx.notify();
                            return false;
                        }
                        None => {}
                    }
                    true
                });
                if !landed.unwrap_or(false) {
                    break;
                }
            }
            // Kept once a round, serialized off the UI thread.
            let kept = this
                .update(cx, |w, cx| {
                    let tree =
                        w.tree.as_ref().filter(|_| w.workspace.as_ref().map(|ws| &ws.folder) == Some(&folder))?;
                    Some((store(cx), rsharp_core::project::Evaluations { projects: tree.all_evaluated() }))
                })
                .ok()
                .flatten();
            if let Some((store, kept)) = kept {
                let folder = folder.clone();
                cx.background_executor().spawn(async move { store.write_for(&folder, &kept) }).await;
            }
            let _ = this.update(cx, |w, cx| {
                w.evaluating = false;
                if std::mem::take(&mut w.evaluate_again) {
                    w.evaluate_projects(cx);
                }
                cx.notify();
            });
        }));
    }

    /// A project's evaluation landing in the Solution view.
    pub fn set_evaluated(&mut self, project: rsharp_core::project::EvaluatedProject, cx: &mut Context<Self>) {
        let Some(tree) = &mut self.tree else { return };
        tree.set_evaluated(project);
        self.rebuild_rows();
        self.sync_nuget(cx);
        self.sync_tests(cx);
        cx.notify();
    }

    /// Selects a row of the Solution view, as a click does.
    pub fn select_row(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = Some(index.min(self.rows.len().saturating_sub(1)));
        cx.notify();
    }

    /// What hovering a row of the Solution view says.
    pub fn solution_row_tip(&self, index: usize) -> Option<String> {
        self.rows.get(index)?.tip.clone()
    }

    /// The selected row of the Solution view, as `solution_rows` shows it.
    pub fn selected_row(&self) -> Option<String> {
        self.selected.and_then(|at| self.solution_rows().get(at).cloned())
    }

    /// Whether the Solution view is waiting on the Host for an evaluation.
    pub fn evaluating(&self) -> bool {
        self.evaluating
    }

    /// Opens or closes a row of the Solution view, as its chevron does.
    pub fn toggle_row(&mut self, index: usize, cx: &mut Context<Self>) {
        self.toggle(index, None, cx);
    }

    fn toggle(&mut self, index: usize, open: Option<bool>, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index) else { return };
        if !row.expandable {
            return;
        }
        if let Some(tree) = &mut self.tree {
            tree.toggle(index, open);
        } else {
            let path = row.path.clone().unwrap_or_default();
            let opening = open.unwrap_or(!row.expanded);
            if opening {
                if let Some(workspace) = &mut self.workspace {
                    workspace.forget(&path);
                }
                self.expanded.insert(path);
            } else {
                self.expanded.remove(&path);
            }
        }
        self.rebuild_rows();
        self.save_state(cx);
        cx.notify();
    }

    /// What a double-click or Enter on a row does: a file opens, anything else opens or closes.
    pub fn activate_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index) else { return };
        match (&row.kind, &row.path) {
            (NodeKind::File, Some(path)) => {
                let path = path.clone();
                self.open_file(&path, None, window, cx);
            }
            _ => self.toggle(index, None, cx),
        }
    }

    /// Each row of the Solution view as shown: indented two spaces a level, its detail after it in brackets.
    pub fn solution_rows(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|r| {
                let detail = r.detail.as_ref().map(|d| format!(" ({d})")).unwrap_or_default();
                format!("{}{}{detail}", "  ".repeat(r.depth), r.name)
            })
            .collect()
    }

    /// Opens a file in a tab, or shows its tab, with the caret at a line and column counted from 0.
    pub fn open_file(
        &mut self,
        path: &Path,
        caret: Option<(usize, usize)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = rsharp_core::paths::canonical(path);
        // Leaving a place for another file, or for another part of this one, is a place Back goes to.
        if let Some(here) = self.here(cx) {
            let there = Spot { path: path.clone(), line: caret.map_or(usize::MAX, |c| c.0), column: 0 };
            if !here.near(&there) {
                self.back.push(here);
                if self.back.len() > HISTORY {
                    self.back.remove(0);
                }
                self.forward.clear();
            }
        }
        self.show_file(&path, caret, window, cx);
    }

    /// Where the caret is in the editor the keys go to.
    fn here(&self, cx: &App) -> Option<Spot> {
        let editor = self.active_editor()?.read(cx);
        let caret = editor.caret();
        Some(Spot { path: editor.path.clone(), line: caret.line, column: caret.utf16_column })
    }

    /// Back (or Forward): the place left last, the place here kept the other way.
    fn go_back(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.here(cx);
        let (from, to) =
            if forward { (&mut self.forward, &mut self.back) } else { (&mut self.back, &mut self.forward) };
        let Some(spot) = from.pop() else { return };
        to.extend(here);
        if spot.path.is_file() {
            self.show_file(&spot.path, Some((spot.line, spot.column)), window, cx);
        }
    }

    /// Last Edit Location: the latest place edited away from the caret, then each before it as it is pressed again.
    fn last_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.here(cx);
        let from = self.edit_step.unwrap_or(self.edits.len());
        let Some(at) = (0..from).rev().find(|&i| here.as_ref().is_none_or(|h| !self.edits[i].near(h))) else { return };
        self.edit_step = Some(at);
        let spot = self.edits[at].clone();
        if spot.path.is_file() {
            self.open_file(&spot.path, Some((spot.line, spot.column)), window, cx);
        }
    }

    /// An edit made in `editor`: its place kept for Last Edit Location, one near the last taking its place.
    fn edited_at(&mut self, editor: &Entity<CodeEditor>, cx: &App) {
        let editor = editor.read(cx);
        let caret = editor.caret();
        let spot = Spot { path: editor.path.clone(), line: caret.line, column: caret.utf16_column };
        if self.edits.last().is_some_and(|last| last.near(&spot)) {
            self.edits.pop();
        }
        self.edits.push(spot);
        if self.edits.len() > HISTORY {
            self.edits.remove(0);
        }
        self.edit_step = None;
    }

    /// Opens a Go to popup over the window: the files it looks among, the open files' text and the server given it.
    fn open_go_to(&mut self, kind: crate::goto::Kind, window: &mut Window, cx: &mut Context<Self>) {
        use crate::goto::{Ended, Given, GoToPopup, Kind};
        let Some(folder) = self.folder().map(Path::to_path_buf) else { return };
        let shown = self.active_editor().map(|e| e.read(cx).path.clone());
        let mut given = Given { server: self.server.clone(), ..Default::default() };
        given.here = shown.as_ref().and_then(|p| p.parent().map(Path::to_path_buf));
        match kind {
            Kind::RecentFiles => {
                given.files = self.recent.iter().filter(|p| Some(*p) != shown.as_ref()).cloned().collect();
            }
            Kind::RecentLocations => given.places = self.recent_locations(cx),
            Kind::FindInFiles | Kind::ReplaceInFiles => {
                given.open = self
                    .area
                    .editors()
                    .map(|e| {
                        let buffer = e.read(cx).buffer.read(cx);
                        (buffer.path.clone(), buffer.document.text().to_string())
                    })
                    .collect();
            }
            _ => {}
        }
        let before = window.focused(cx);
        let popup = cx.new(|cx| GoToPopup::new(kind, folder, given, cx));
        cx.subscribe_in(&popup, window, move |w, _, ended: &Ended, window, cx| {
            w.go_to = None;
            match ended {
                Ended::Chosen(place) => w.open_file(&place.path.clone(), place.at, window, cx),
                Ended::ReplaceAll { query, replacement, files } => {
                    let server = w.server.clone();
                    w.said = Some(
                        match crate::language::replace_in_files(files, query, replacement, server.as_deref(), cx) {
                            Ok(n) => format!("Replaced {n} {}", if n == 1 { "match" } else { "matches" }),
                            Err(why) => why,
                        }
                        .into(),
                    );
                    w.focus_editor(window, cx);
                }
                Ended::Dismissed => {
                    if let Some(before) = &before {
                        window.focus(before);
                    }
                }
            }
            cx.notify();
        })
        .detach();
        window.focus(&popup.focus_handle(cx));
        self.go_to = Some(popup);
        cx.notify();
    }

    /// Recent Locations' places: those left and edited, the latest first, one for each stretch of a file, each with its
    /// line's text.
    fn recent_locations(&self, cx: &App) -> Vec<crate::goto::Place> {
        let mut spots: Vec<&Spot> = Vec::new();
        for spot in self.back.iter().chain(&self.edits).rev() {
            if spots.iter().all(|s| !s.near(spot)) {
                spots.push(spot);
            }
        }
        spots
            .into_iter()
            .filter(|s| s.path.is_file())
            .map(|s| {
                let line = match crate::buffer::find(&s.path, cx) {
                    Some(buffer) => buffer.read(cx).document.text().line(s.line).to_string(),
                    None => std::fs::read_to_string(&s.path)
                        .ok()
                        .and_then(|t| t.lines().nth(s.line).map(str::to_string))
                        .unwrap_or_default(),
                };
                let file = s.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                crate::goto::Place {
                    name: line.trim().to_string(),
                    detail: format!("{file}:{}", s.line + 1),
                    path: s.path.clone(),
                    at: Some((s.line, s.column)),
                }
            })
            .collect()
    }

    /// A refactoring's dialog over the window, the keys back with the editor once it closes.
    fn open_refactoring(
        &mut self,
        open: &crate::refactor_dialog::OpenRefactoring,
        editor: Entity<CodeEditor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::refactor_dialog::{Closed, OpenRefactoring, RefactorDialog};
        let open = OpenRefactoring {
            kind: open.kind,
            described: open.described.clone(),
            at: open.at,
            uri: open.uri.clone(),
            server: open.server.clone(),
        };
        let dialog = cx.new(|cx| RefactorDialog::new(open, cx));
        cx.subscribe_in(&dialog, window, move |w, _, closed: &Closed, window, cx| {
            w.refactoring = None;
            if let Some(said) = &closed.0 {
                w.said = Some(said.clone().into());
            }
            window.focus(&editor.focus_handle(cx));
            cx.notify();
        })
        .detach();
        window.focus(&dialog.focus_handle(cx));
        self.refactoring = Some(dialog);
        cx.notify();
    }

    /// The refactoring dialog open, for a test to set its options and press OK.
    pub fn refactoring(&self) -> Option<Entity<crate::refactor_dialog::RefactorDialog>> {
        self.refactoring.clone()
    }

    /// The Go to popup open, for a test to set its scope, mask or replacement.
    pub fn go_to_popup(&self) -> Option<Entity<crate::goto::GoToPopup>> {
        self.go_to.clone()
    }

    /// What the open Go to popup lists, as a person reads it.
    pub fn go_to_listed(&self, cx: &App) -> Option<Vec<String>> {
        self.go_to.as_ref().map(|p| p.read(cx).listed())
    }

    /// Shows a file in the current group, opening it there if it is not, with the caret at a line and column.
    fn show_file(&mut self, path: &Path, caret: Option<(usize, usize)>, window: &mut Window, cx: &mut Context<Self>) {
        let path = path.to_path_buf();
        let group = self.area.current;
        let index = match self.area.tab_of(group, &path, cx) {
            Some(index) => index,
            None => {
                let buffer = match crate::buffer::find(&path, cx) {
                    Some(buffer) => {
                        buffer.update(cx, |b, cx| b.refresh_from_disk(cx));
                        buffer
                    }
                    None => match crate::buffer::open(path.clone(), cx) {
                        Ok(buffer) => buffer,
                        Err(error) => {
                            log::warn!("Could not open {}: {error}", path.display());
                            self.said = Some(format!("Could not open {}: {error}", path.display()).into());
                            cx.notify();
                            return;
                        }
                    },
                };
                if let Some(server) = self.server.clone().filter(|_| is_csharp(&path)) {
                    buffer.update(cx, |b, cx| b.attach_server(server, cx));
                }
                self.attach_msbuild(&buffer, cx);
                self.attach_git_buffer(&buffer, cx);
                self.note_hot_reload_original(&buffer, cx);
                self.show_breakpoints(&path, cx);
                let editor = self.new_editor(buffer, window, cx);
                self.area.add(editor, false)
            }
        };
        if let Some((line, column)) = caret {
            self.area.groups[group].tabs[index].editor.update(cx, |e, cx| e.place_caret(line, column, cx));
        }
        self.show_tab(group, index, window, cx);
    }

    /// Where Go to Declaration or Find Usages leads: the one place opened, or every place listed in the Find window.
    fn navigate(&mut self, go: &editor::Navigate, window: &mut Window, cx: &mut Context<Self>) {
        if let ([to], false) = (go.to.as_slice(), go.list) {
            self.open_file(&to.path, Some((to.span.start.line, to.span.start.character)), window, cx);
            return;
        }
        let found = go
            .to
            .iter()
            .map(|to| {
                let (line, column) = (to.span.start.line, to.span.start.character);
                let text = match crate::buffer::find(&to.path, cx) {
                    Some(buffer) => buffer.read(cx).document.text().line(line).to_string(),
                    None => std::fs::read_to_string(&to.path)
                        .ok()
                        .and_then(|t| t.lines().nth(line).map(str::to_string))
                        .unwrap_or_default(),
                };
                Found { path: to.path.clone(), line, column, text: text.trim().to_string() }
            })
            .collect();
        self.found = Some((go.title.clone(), found));
        if !self.dock.is_open(Tool::Find) {
            self.toggle_tool(Tool::Find, window, cx);
        }
        cx.notify();
    }

    /// The Find window's places, by file, each a row that goes there.
    fn find_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((_, found)) = &self.found else {
            return empty_text("Find Usages lists the places here.").track_focus(&self.find_focus).into_any_element();
        };
        let folder = self.folder().map(Path::to_path_buf);
        div()
            .id("find-results")
            .track_focus(&self.find_focus)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(6.))
            .children(found.iter().enumerate().map(|(i, place)| {
                let (path, line, column) = (place.path.clone(), place.line, place.column);
                let shown = folder
                    .as_deref()
                    .and_then(|f| path.strip_prefix(f).ok())
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                div()
                    .id(("found", i))
                    .debug_selector(move || format!("found: {i}"))
                    .h(px(ROW_HEIGHT))
                    .px(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .hover(|d| d.bg(theme::row_hover()))
                    .child(icon(file_icon(&shown), 16.))
                    .child(div().flex_none().text_color(theme::text_secondary()).child(format!("{shown}:{}", line + 1)))
                    .child(div().truncate().font_family(theme::CODE_FONT).child(place.text.clone()))
                    .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| {
                        w.open_file(&path, Some((line, column)), window, cx)
                    }))
            }))
            .into_any_element()
    }

    /// The places the Find window lists, each as "file:line", as a person reads them.
    pub fn found(&self) -> Option<Vec<String>> {
        self.found.as_ref().map(|(_, found)| {
            found
                .iter()
                .map(|f| format!("{}:{}", f.path.file_name().unwrap_or_default().to_string_lossy(), f.line + 1))
                .collect()
        })
    }

    /// An editor of a buffer, whose every change the window keeps for the folder.
    fn new_editor(
        &mut self,
        buffer: Entity<Buffer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<CodeEditor> {
        let editor = cx.new(|cx| CodeEditor::new(buffer.clone(), cx));
        cx.subscribe(&buffer, |w, _, event: &crate::buffer::BufferEvent, cx| match event {
            crate::buffer::BufferEvent::Parsed => w.refresh_open_tests(cx),
            crate::buffer::BufferEvent::Moved(_) => w.refresh_tests(cx),
            _ => {}
        })
        .detach();
        cx.subscribe(&buffer, |w, buffer, event: &crate::buffer::BufferEvent, cx| {
            if let crate::buffer::BufferEvent::Edited { changes, .. } = event {
                w.follow_breakpoints(&buffer, changes, cx);
            }
        })
        .detach();
        cx.observe_release(&buffer, |w, _, cx| w.refresh_tests(cx)).detach();
        let edits_by = editor.downgrade();
        cx.subscribe(&buffer, move |w, _, event: &crate::buffer::BufferEvent, cx| {
            if let crate::buffer::BufferEvent::Edited { by, .. } = event
                && *by == edits_by.entity_id()
                && let Some(editor) = edits_by.upgrade()
            {
                w.edited_at(&editor, cx);
            }
        })
        .detach();
        cx.subscribe_in(&editor, window, |_, editor, _: &editor::ReadOnlyEdit, window, cx| {
            editor::ask_clear_read_only(editor.clone(), editor::AfterClearing::Edit, window, cx)
        })
        .detach();
        cx.subscribe_in(&editor, window, |w, editor, toggle: &editor::ToggleBreakpoint, _, cx| {
            let path = editor.read(cx).path.clone();
            if toggle.tracepoint {
                w.toggle_tracepoint_at(&path, toggle.line, cx);
            } else {
                w.toggle_breakpoint_at(&path, toggle.line, cx);
            }
        })
        .detach();
        cx.subscribe_in(&editor, window, |w, editor, edit: &editor::EditBreakpointAt, window, cx| {
            let path = editor.read(cx).path.clone();
            if w.breakpoints().has(&path, edit.line) {
                w.open_breakpoint_popup(&path, edit.line, crate::breakpoint_popup::Field::Condition, window, cx);
            }
        })
        .detach();
        cx.subscribe_in(&editor, window, |w, editor, asked: &editor::ValueAsked, _, cx| {
            w.hover_value(editor.clone(), asked.expression.clone(), asked.at, cx);
        })
        .detach();
        cx.subscribe_in(&editor, window, |w, editor, moved: &editor::MoveExecution, _, cx| {
            let path = editor.read(cx).path.clone();
            w.set_next_statement_at(path, moved.line, cx);
        })
        .detach();
        cx.subscribe_in(&editor, window, |w, _, go: &editor::Navigate, window, cx| w.navigate(go, window, cx)).detach();
        cx.subscribe_in(&editor, window, |w, editor, open: &crate::refactor_dialog::OpenRefactoring, window, cx| {
            w.open_refactoring(open, editor.clone(), window, cx)
        })
        .detach();
        cx.subscribe(&editor, |w, _, said: &editor::Said, cx| {
            w.said = Some(said.0.clone().into());
            cx.notify()
        })
        .detach();
        cx.observe(&editor, |w, _, cx| {
            w.save_state(cx);
            cx.notify()
        })
        .detach();
        self.refresh_open_tests(cx);
        editor
    }

    /// Every editor closed, as a folder closing or another opening does; a file no other window shows leaves the
    /// server as its buffer goes.
    fn leave_editors(&mut self) {
        self.area = EditorArea::default();
        self.find_bar = None;
        self.tab_menu = None;
    }

    /// The keys back with the current group's editor, else the Solution view, else the window.
    fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.area.active_editor() {
            Some(editor) => window.focus(&editor.focus_handle(cx)),
            None if self.dock.is_open(Tool::Solution) => window.focus(&self.explorer_focus),
            None => window.focus(&self.root_focus),
        }
        self.follow_find_bar(cx);
        self.save_state(cx);
        cx.notify();
    }

    /// Closes the current group's unpinned tabs that `which` picks by index.
    fn close_tabs(&mut self, which: impl Fn(usize) -> bool, window: &mut Window, cx: &mut Context<Self>) {
        let group = self.area.current;
        for index in self.area.unpinned(group, which) {
            self.area.close(group, index, cx);
        }
        self.focus_editor(window, cx);
        self.refresh_tests(cx);
    }

    /// Split Right or Down: the file the current group shows, in a new group beside it, at the same caret.
    fn split(&mut self, right: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shown) = self.area.active_editor().cloned() else { return };
        let (buffer, caret) = {
            let read = shown.read(cx);
            (read.buffer.clone(), read.caret())
        };
        let editor = self.new_editor(buffer, window, cx);
        editor.update(cx, |e, cx| e.place_caret(caret.line, caret.column, cx));
        self.area.split(right, editor);
        self.focus_editor(window, cx);
    }

    /// Open Tab in New Window, or a tab let go outside the window: the tab moves to a window of its own at `at` on
    /// the screen, an editor of the same buffer.
    fn move_to_new_window(
        &mut self,
        group: usize,
        index: usize,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.area.groups.get(group).and_then(|g| g.tabs.get(index)) else { return };
        let (buffer, caret) = {
            let read = tab.editor.read(cx);
            (read.buffer.clone(), read.caret())
        };
        let editor = cx.new(|cx| CodeEditor::new(buffer, cx));
        editor.update(cx, |e, cx| e.place_caret(caret.line, caret.column, cx));
        self.close_tab(group, index, window, cx);
        let editor = editor.clone();
        cx.defer(move |cx| {
            crate::editor_window::open(editor, at, cx);
        });
    }

    fn reopen_closed_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(closed) = self.area.closed.pop() else { return };
        if !closed.path.is_file() {
            self.said = Some(format!("{} is no longer there", closed.path.display()).into());
            cx.notify();
            return;
        }
        self.open_file(&closed.path, Some(closed.caret), window, cx);
        let group = self.area.current;
        if closed.pinned
            && let Some(at) = self.area.tab_of(group, &closed.path, cx)
        {
            self.area.pin(group, at, true);
            self.show_tab(group, self.area.tab_of(group, &closed.path, cx).unwrap_or(at), window, cx);
        }
    }

    fn close_tab(&mut self, group: usize, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.area.close(group, index, cx);
        self.focus_editor(window, cx);
        self.refresh_tests(cx);
    }

    /// The find bar on the active tab's file, or gone with the last tab.
    fn follow_find_bar(&mut self, cx: &mut Context<Self>) {
        let Some(bar) = self.find_bar.clone() else { return };
        match self.active_editor().cloned() {
            Some(editor) => bar.update(cx, |b, cx| b.attach(editor, cx)),
            None => self.find_bar = None,
        }
    }

    /// Find (with the replace row, Replace): the bar opened, or brought back, on the active tab's file.
    fn open_find_bar(&mut self, replacing: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor().cloned() else { return };
        let bar = match &self.find_bar {
            Some(bar) => bar.clone(),
            None => {
                let bar = cx.new(|cx| FindBar::new(editor, cx));
                cx.subscribe_in(&bar, window, |w, _, _: &find_bar::Closed, window, cx| {
                    w.find_bar = None;
                    if let Some(editor) = w.active_editor() {
                        window.focus(&editor.focus_handle(cx));
                    }
                    cx.notify();
                })
                .detach();
                self.find_bar = Some(bar.clone());
                bar
            }
        };
        bar.update(cx, |b, cx| b.open(replacing, window, cx));
        cx.notify();
    }

    /// What the find bar's count says while it is open.
    pub fn find_count(&self, cx: &App) -> Option<String> {
        self.find_bar.as_ref().map(|b| b.read(cx).count().to_string())
    }

    fn choose_folder(&mut self, _: &OpenFolder, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |_, cx| {
                    cx.defer(move |cx| {
                        crate::projects::open_project(&path, cx);
                    })
                });
            }
        })
        .detach();
    }

    /// Each tab's file name and caret (line and column from 0), as the tab strip and status bar show them.
    pub fn tab_carets(&self, cx: &App) -> Vec<(String, (usize, usize))> {
        self.area
            .group()
            .tabs
            .iter()
            .map(|t| {
                let editor = t.editor.read(cx);
                let caret = editor.caret();
                (
                    editor.path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                    (caret.line, caret.utf16_column),
                )
            })
            .collect()
    }

    pub fn active_tab_name(&self, cx: &App) -> Option<String> {
        self.active_editor().map(|e| e.read(cx).path.file_name().unwrap_or_default().to_string_lossy().into_owned())
    }

    /// Back to the Welcome screen, the folder's state kept, as Rider's File | Close Solution.
    pub fn close_solution(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_tests(cx);
        self.leave_git(cx);
        self.leave_build(cx);
        self.leave_run(cx);
        self.leave_debug(cx);
        self.terminal.update(cx, |t, cx| t.close_all(cx));
        self.save_state(cx);
        self.leave_editors();
        self.workspace = None;
        self.opened_as = None;
        self.rows.clear();
        self.tree = None;
        (self._watcher, self._watch_task) = (None, None);
        self.host = None;
        (self.evaluating, self.evaluate_again, self._evaluate_task) = (false, false, None);
        self.server = None;
        self.server_state = ServerState::None;
        self._server_task = None;
        self.branch = None;
        self.sync_nuget(cx);
        window.set_window_title("RSharp");
        window.focus(&self.root_focus);
        cx.notify();
    }

    /// Opens Find Action over the window, each action's availability judged where the person was.
    /// Whether each action of `ALL` can run where the person is, and its keys as the platform writes them.
    fn actions_here(window: &mut Window, cx: &mut App) -> (Vec<bool>, Vec<Option<SharedString>>) {
        ALL.iter()
            .map(|entry| {
                let action = (entry.action)();
                let keys = window.highest_precedence_binding_for_action(&*action).map(|binding| {
                    binding.keystrokes().iter().map(ToString::to_string).collect::<Vec<_>>().join(" ").into()
                });
                (window.is_action_available(&*action, cx), keys)
            })
            .unzip()
    }

    pub fn find_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let before = window.focused(cx);
        let (available, keys) = Self::actions_here(window, cx);
        let popup = cx.new(|cx| FindActionPopup::new(available.clone(), keys, cx));
        cx.subscribe_in(&popup, window, move |w, _, ended: &Ended, window, cx| {
            w.find_action = None;
            if let Some(before) = &before {
                window.focus(before);
            }
            if let Ended::Chosen(index) = ended {
                if available[*index] {
                    window.dispatch_action((ALL[*index].action)(), cx);
                } else {
                    w.said = Some(unavailable_action(*index));
                }
            }
            cx.notify();
        })
        .detach();
        window.focus(&popup.focus_handle(cx));
        self.find_action = Some(popup);
        cx.notify();
    }

    /// What Find Action lists now, or `None` while it is closed.
    fn main_menu_dropdown(
        &self,
        available: &[bool],
        keys: &[Option<SharedString>],
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        for menu in crate::actions::MENUS {
            let items: Vec<usize> = (0..ALL.len()).filter(|i| ALL[*i].menu == *menu).collect();
            if items.is_empty() {
                continue;
            }
            rows.push(
                div()
                    .h(px(24.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .text_color(theme::text_secondary())
                    .child(*menu)
                    .into_any_element(),
            );
            for i in items {
                let enabled = available[i];
                rows.push(
                    div()
                        .id(("menu", i))
                        .debug_selector(move || format!("menu: {}", ALL[i].title))
                        .h(px(24.))
                        .mx(px(6.))
                        .px(px(8.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .gap(px(24.))
                        .when(enabled, |d| d.hover(|d| d.bg(theme::selection())))
                        .when(!enabled, |d| d.text_color(theme::text_secondary()))
                        .child(div().flex_1().child(ALL[i].title))
                        .children(keys[i].clone().map(|k| div().text_color(theme::text_muted()).child(k)))
                        .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| w.choose_from_menu(i, window, cx)))
                        .into_any_element(),
                );
            }
        }
        div().absolute().top(px(TOOLBAR_HEIGHT - 2.)).left(px(8.)).child(
            div()
                .id("main-menu-dropdown")
                .debug_selector(|| "main menu dropdown".into())
                .w(px(300.))
                // Every menu as one list is taller than most windows; it scrolls within the window instead.
                .max_h(window.viewport_size().height - px(TOOLBAR_HEIGHT + GAP))
                .overflow_y_scroll()
                .py(px(6.))
                .rounded(px(8.))
                .bg(theme::popup())
                .border_1()
                .border_color(theme::popup_border())
                .shadow_lg()
                .on_mouse_down_out(cx.listener(|w, _, window, cx| {
                    if let Some((_, _, before)) = w.main_menu.take() {
                        if let Some(before) = before {
                            window.focus(&before);
                        }
                        cx.notify();
                    }
                }))
                .children(rows),
        )
    }

    /// Opens the main menu's dropdown under its button.
    pub fn open_main_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let before = window.focused(cx);
        let (available, keys) = Self::actions_here(window, cx);
        self.main_menu = Some((available, keys, before));
        cx.notify();
    }

    /// Runs the main menu's choice where the person was, or says why it cannot run.
    fn choose_from_menu(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((available, _, before)) = self.main_menu.take() else { return };
        if let Some(before) = &before {
            window.focus(before);
        }
        if available[index] {
            window.dispatch_action((ALL[index].action)(), cx);
        } else {
            self.said = Some(unavailable_action(index));
        }
        cx.notify();
    }

    pub fn find_action_listed(&self, cx: &App) -> Option<Vec<&'static str>> {
        self.find_action.as_ref().map(|p| p.read(cx).listed())
    }

    pub fn said(&self) -> Option<&str> {
        self.said.as_ref().map(|s| s.as_ref())
    }

    pub fn has_workspace(&self) -> bool {
        self.workspace.is_some()
    }

    /// Whether Roslyn has said the solution's projects are loaded.
    pub fn server_ready(&self) -> bool {
        matches!(self.server_state, ServerState::Ready)
    }

    /// What the status bar says of Roslyn.
    pub fn server_status(&self) -> Option<String> {
        match &self.server_state {
            ServerState::None => None,
            ServerState::Starting => Some("Starting Roslyn…".into()),
            ServerState::Loading(what) => Some(format!("{what}…")),
            ServerState::Ready => Some("Roslyn".into()),
            ServerState::Failed(why) => Some(why.clone()),
        }
    }

    /// Roslyn's last log messages, the latest last.
    pub fn roslyn_said(&self) -> Vec<String> {
        self.roslyn_said.iter().cloned().collect()
    }

    /// Whether Roslyn is starting or loading the projects.
    pub fn server_busy(&self) -> bool {
        matches!(self.server_state, ServerState::Starting | ServerState::Loading(_))
    }

    /// Whether Roslyn could not start or stopped, with why.
    pub fn server_failed(&self) -> Option<&str> {
        match &self.server_state {
            ServerState::Failed(why) => Some(why),
            _ => None,
        }
    }

    /// The open folder, canonical.
    pub fn folder(&self) -> Option<&Path> {
        self.workspace.as_ref().map(|w| w.folder.as_path())
    }

    /// What was opened: the solution or project file, else the folder.
    pub fn opened_as(&self) -> Option<&Path> {
        self.opened_as.as_deref()
    }

    /// The editor the keys go to, as a test reads it.
    pub fn editor(&self) -> Option<Entity<CodeEditor>> {
        self.area.active_editor().cloned()
    }

    fn active_editor(&self) -> Option<&Entity<CodeEditor>> {
        self.area.active_editor()
    }

    /// Each group's tabs by file name, a pinned one marked with a leading "*", as a person reads the tab strips.
    pub fn groups(&self, cx: &App) -> Vec<Vec<String>> {
        self.area
            .groups
            .iter()
            .map(|g| {
                g.tabs
                    .iter()
                    .map(|t| {
                        let name =
                            t.editor.read(cx).path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        if t.pinned { format!("*{name}") } else { name }
                    })
                    .collect()
            })
            .collect()
    }

    /// The current group's index, the one keys go to.
    pub fn current_group(&self) -> usize {
        self.area.current
    }
}

fn is_csharp(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "cs" || e == "csx")
}

fn unavailable_action(index: usize) -> SharedString {
    if ALL[index].title == "New Project…" {
        "New Project requires an open solution".into()
    } else {
        format!("{} is not available here", ALL[index].title.trim_end_matches('…')).into()
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let floating = if self.workspace.is_some() { self.floating_islands(window, cx) } else { Vec::new() };
        div()
            .id("main-window")
            .key_context("MainWindow")
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::frame())
            .text_color(theme::text())
            .font_family(theme::UI_FONT)
            .text_size(px(13.))
            .relative()
            .track_focus(&self.root_focus)
            // A tab dragged out past the window's edge and let go there opens in a window of its own.
            .on_drag_move(cx.listener(|w, event: &gpui::DragMoveEvent<TabDrag>, _, cx| {
                let outside = !event.bounds.contains(&event.event.position);
                w.tab_drag = Some((event.drag(cx).clone(), outside));
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|w, _, _, _| w.tab_drag = None))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|w, event: &gpui::MouseUpEvent, window, cx| {
                    if let Some((drag, true)) = w.tab_drag.take() {
                        let at = window.bounds().origin + event.position;
                        w.move_to_new_window(drag.group, drag.index, at, window, cx);
                    }
                }),
            )
            .on_action(cx.listener(Self::choose_folder))
            .on_action(cx.listener(|m, _: &NewSolution, w, cx| m.show_new_solution(w, cx)))
            .when(self.tree.is_some(), |d| {
                d.on_action(cx.listener(|m, _: &NewProject, w, cx| {
                    let into = m
                        .selected
                        .and_then(|i| m.rows.get(i))
                        .filter(|r| r.kind == NodeKind::SolutionFolder)
                        .and_then(|r| r.key.strip_prefix("sf:").map(str::to_string));
                    m.show_new_project(into, w, cx);
                }))
            })
            .on_action(cx.listener(|w, _: &FindAction, window, cx| w.find_action(window, cx)))
            .on_action(cx.listener(|w, _: &ToggleTerminal, window, cx| w.toggle_terminal(window, cx)))
            .on_action(cx.listener(|w, _: &ToggleCommit, window, cx| w.toggle_tool(Tool::Commit, window, cx)))
            .on_action(cx.listener(|w, _: &ToggleGit, window, cx| w.toggle_tool(Tool::Git, window, cx)))
            .on_action(cx.listener(|w, _: &GitRefresh, _, cx| w.refresh_git(cx)))
            .when(self.git_status().is_some() && !self.git_busy(), |d| {
                d.on_action(
                    cx.listener(|w, _: &GitPush, win, cx| w.git_request(crate::git_view::GitRequest::Push, win, cx)),
                )
                .on_action(
                    cx.listener(|w, _: &GitUpdate, win, cx| {
                        w.git_request(crate::git_view::GitRequest::Update, win, cx)
                    }),
                )
                .on_action(cx.listener(|w, _: &GitHistory, win, cx| w.git_history(false, win, cx)))
                .on_action(cx.listener(|w, _: &GitSelectionHistory, win, cx| w.git_history(true, win, cx)))
                .on_action(cx.listener(|w, _: &GitCompare, win, cx| w.git_compare(win, cx)))
                .on_action(cx.listener(|w, _: &GitResolve, win, cx| w.git_resolve_selected(win, cx)))
                .on_action(cx.listener(|w, _: &GitContinue, win, cx| {
                    w.git_request(crate::git_view::GitRequest::Continue, win, cx)
                }))
                .on_action(
                    cx.listener(|w, _: &GitAbort, win, cx| w.git_request(crate::git_view::GitRequest::Abort, win, cx)),
                )
                .on_action(cx.listener(|w, _: &NextChange, _, cx| w.git_change(false, false, cx)))
                .on_action(cx.listener(|w, _: &PreviousChange, _, cx| w.git_change(true, false, cx)))
                .on_action(cx.listener(|w, _: &RollbackLines, _, cx| w.git_change(false, true, cx)))
            })
            // Each acts only while it has something to act on, so menus and Find Action grey it otherwise.
            .when(self.workspace.is_some(), |d| {
                d.on_action(cx.listener(|w, _: &CloseSolution, window, cx| w.close_solution(window, cx)))
                    .on_action(
                        cx.listener(|w, _: &ToggleSolution, window, cx| w.toggle_tool(Tool::Solution, window, cx)),
                    )
                    .on_action(
                        cx.listener(|w, _: &ToggleProblems, window, cx| w.toggle_tool(Tool::Problems, window, cx)),
                    )
                    .on_action(cx.listener(|w, _: &ToggleNuGet, window, cx| w.toggle_tool(Tool::NuGet, window, cx)))
                    .on_action(cx.listener(|w, _: &RestoreDefaultLayout, _, cx| {
                        w.dock = DockLayout::default();
                        w.save_state(cx);
                        cx.notify()
                    }))
                    .on_action(cx.listener(|w, _: &HideToolWindow, window, cx| {
                        if let Some(tool) = w.focused_tool(window, cx) {
                            w.toggle_tool(tool, window, cx)
                        }
                    }))
                    .on_action(cx.listener(|w, _: &FloatToolWindow, window, cx| {
                        if let Some(tool) = w.focused_tool(window, cx) {
                            w.toggle_floating(tool, window, cx)
                        }
                    }))
                    .on_drag_move(cx.listener(|w, event: &gpui::DragMoveEvent<FloatDrag>, _, cx| {
                        let (at, grab) = (event.event.position, w.float_grab);
                        let FloatDrag { tool, resize } = *event.drag(cx);
                        if let Some((_, r)) = w.dock.floating.iter_mut().find(|(t, _)| *t == tool) {
                            if resize {
                                (r.w, r.h) = (f32::from(at.x - grab.x) - r.x, f32::from(at.y - grab.y) - r.y);
                            } else {
                                (r.x, r.y) = (f32::from(at.x - grab.x), f32::from(at.y - grab.y));
                            }
                        }
                        w.dock = w.dock.clone().clamped();
                        w.save_state(cx);
                        cx.notify();
                    }))
            })
            .when(self.area.active_editor().is_some(), |d| {
                d.on_action(cx.listener(|w, _: &find_bar::Find, window, cx| w.open_find_bar(false, window, cx)))
                    .on_action(cx.listener(|w, _: &find_bar::Replace, window, cx| w.open_find_bar(true, window, cx)))
            })
            .when_some(self.find_bar.clone(), |d, bar| {
                let (next, previous, close) = (bar.clone(), bar.clone(), bar);
                d.on_action(
                    cx.listener(move |_, _: &find_bar::FindNext, _, cx| next.update(cx, |b, cx| b.find_next(true, cx))),
                )
                .on_action(cx.listener(move |_, _: &find_bar::FindPrevious, _, cx| {
                    previous.update(cx, |b, cx| b.find_next(false, cx))
                }))
                // Escape in the editor, once it has no carets or selection to drop, closes the bar.
                .on_action(cx.listener(move |_, _: &editor::Escape, _, cx| close.update(cx, |b, cx| b.close(cx))))
            })
            .map(|d| self.tab_actions(d, cx))
            .map(|d| self.build_actions(d, cx))
            .map(|d| self.run_actions(d, cx))
            .map(|d| self.debug_actions(d, cx))
            .map(|d| self.test_actions(d, cx))
            .when(self.workspace.is_some(), |d| {
                use crate::goto::Kind;
                d.on_action(cx.listener(|w, _: &GoToFile, window, cx| w.open_go_to(Kind::File, window, cx)))
                    .on_action(cx.listener(|w, _: &GoToType, window, cx| w.open_go_to(Kind::Type, window, cx)))
                    .on_action(cx.listener(|w, _: &GoToSymbol, window, cx| w.open_go_to(Kind::Symbol, window, cx)))
                    .on_action(
                        cx.listener(|w, _: &FindInFiles, window, cx| w.open_go_to(Kind::FindInFiles, window, cx)),
                    )
                    .on_action(
                        cx.listener(|w, _: &ReplaceInFiles, window, cx| w.open_go_to(Kind::ReplaceInFiles, window, cx)),
                    )
                    .on_action(
                        cx.listener(|w, _: &SearchEverywhere, window, cx| w.open_go_to(Kind::Everything, window, cx)),
                    )
            })
            .when(!self.recent.is_empty(), |d| {
                d.on_action(cx.listener(|w, _: &RecentFiles, window, cx| {
                    w.open_go_to(crate::goto::Kind::RecentFiles, window, cx)
                }))
            })
            .when(!self.back.is_empty() || !self.edits.is_empty(), |d| {
                d.on_action(cx.listener(|w, _: &RecentLocations, window, cx| {
                    w.open_go_to(crate::goto::Kind::RecentLocations, window, cx)
                }))
            })
            .when(!self.back.is_empty(), |d| {
                d.on_action(cx.listener(|w, _: &Back, window, cx| w.go_back(false, window, cx)))
            })
            .when(!self.forward.is_empty(), |d| {
                d.on_action(cx.listener(|w, _: &Forward, window, cx| w.go_back(true, window, cx)))
            })
            .when(!self.edits.is_empty(), |d| {
                d.on_action(cx.listener(|w, _: &LastEditLocation, window, cx| w.last_edit(window, cx)))
            })
            .when(self.area.active_editor().is_some(), |d| {
                d.on_action(cx.listener(|w, _: &CloseTab, window, cx| {
                    if let Some(active) = w.area.group().active {
                        w.close_tab(w.area.current, active, window, cx)
                    }
                }))
                .on_action(cx.listener(|w, _: &editor::Save, window, cx| {
                    if let Some(editor) = w.active_editor().cloned() {
                        if editor.read(cx).buffer.read(cx).read_only {
                            editor::ask_clear_read_only(editor, editor::AfterClearing::Save, window, cx);
                            return;
                        }
                        w.said =
                            editor.update(cx, |e, cx| e.save(cx)).err().map(|e| format!("Could not save: {e}").into());
                        let saved = editor.read(cx).path.extension().and_then(|e| e.to_str()).map(str::to_lowercase);
                        if w.said.is_none()
                            && saved.is_some_and(|e| e.ends_with("proj") || e == "props" || e == "targets")
                        {
                            w.evaluate_projects(cx);
                        }
                        if w.said.is_none() {
                            // A save made while the program is debugged applies its edits through Hot Reload.
                            w.hot_reload_saved(&editor, cx);
                        }
                        w.refresh_git(cx);
                        w.refresh_solution_problems(cx);
                        cx.notify();
                    }
                }))
            })
            .child(self.toolbar(window, cx))
            .child(match self.workspace {
                Some(_) => self.body(window, cx).into_any_element(),
                None => div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .px(px(GAP))
                    .gap(px(GAP))
                    .child(self.welcome(cx))
                    .when(self.dock.is_open(Tool::Terminal), |d| {
                        d.child(div().h(px(self.dock.bottom_height)).child(self.tool_island(
                            Tool::Terminal,
                            window,
                            cx,
                        )))
                    })
                    .into_any_element(),
            })
            .child(self.status_bar(cx))
            .children(floating)
            .children(
                self.main_menu
                    .as_ref()
                    .map(|(available, keys, _)| self.main_menu_dropdown(available, keys, window, cx)),
            )
            .children(self.project_menu.then(|| self.project_widget_menu(cx)))
            .children(self.run_menu.then(|| {
                div()
                    .absolute()
                    .top(px(TOOLBAR_HEIGHT))
                    .left(px(if cfg!(target_os = "macos") { 300. } else { 264. }))
                    .rounded(px(6.))
                    .bg(theme::popup())
                    .border_1()
                    .border_color(theme::popup_border())
                    .shadow_lg()
                    .child(self.run_menu_view(cx))
            }))
            .children(self.tab_menu.map(|at| self.tab_menu_view(at, window, cx)))
            .children(self.status_menu.map(|kind| self.status_menu_view(kind, cx)))
            .children(self.refactoring.clone().map(|dialog| {
                div().absolute().top(px(TOOLBAR_HEIGHT + 60.)).left_0().right_0().flex().justify_center().child(dialog)
            }))
            .children(self.run_dialog.clone().map(|dialog| {
                div().absolute().top(px(TOOLBAR_HEIGHT + 40.)).left_0().right_0().flex().justify_center().child(dialog)
            }))
            .children(self.breakpoint_popup.clone().map(|(popup, _, _)| {
                div().absolute().top(px(TOOLBAR_HEIGHT + 80.)).left_0().right_0().flex().justify_center().child(popup)
            }))
            .children(self.breakpoints_dialog.clone().map(|dialog| {
                div().absolute().top(px(TOOLBAR_HEIGHT + 20.)).left_0().right_0().flex().justify_center().child(dialog)
            }))
            .children(self.attach_dialog.clone().map(|dialog| {
                div().absolute().top(px(TOOLBAR_HEIGHT + 20.)).left_0().right_0().flex().justify_center().child(dialog)
            }))
            .children(self.evaluate_dialog.clone().map(|dialog| {
                div().absolute().top(px(TOOLBAR_HEIGHT + 80.)).left_0().right_0().flex().justify_center().child(dialog)
            }))
            .children(self.step_dialog.clone().map(|dialog| {
                div().absolute().top(px(TOOLBAR_HEIGHT + 80.)).left_0().right_0().flex().justify_center().child(dialog)
            }))
            .children(self.choice_dialog.clone().map(|dialog| {
                div().absolute().top(px(TOOLBAR_HEIGHT + 80.)).left_0().right_0().flex().justify_center().child(dialog)
            }))
            .children(self.go_to.clone().map(|popup| {
                div().absolute().top(px(TOOLBAR_HEIGHT + 60.)).left_0().right_0().flex().justify_center().child(
                    div()
                        .on_mouse_down_out(cx.listener(|w, _, window, cx| {
                            if w.go_to.take().is_some() {
                                w.focus_editor(window, cx);
                            }
                        }))
                        .child(popup),
                )
            }))
            .children(self.solution_menu_view(cx))
            .children(self.reference_dialog.as_ref().map(|(dialog, _)| {
                div()
                    .absolute()
                    .top(px(TOOLBAR_HEIGHT + 60.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(dialog.clone())
            }))
            .children(self.name_popup.as_ref().map(|(popup, _)| {
                div()
                    .absolute()
                    .top(px(TOOLBAR_HEIGHT + 60.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(popup.clone())
            }))
            .children(self.find_action.clone().map(|popup| {
                div().absolute().top(px(TOOLBAR_HEIGHT + 60.)).left_0().right_0().flex().justify_center().child(
                    div()
                        .on_mouse_down_out(cx.listener(|w, _, window, cx| {
                            if let Some(popup) = w.find_action.take() {
                                drop(popup);
                                window.focus(&w.root_focus);
                                cx.notify();
                            }
                        }))
                        .child(popup),
                )
            }))
    }
}

impl MainWindow {
    /// The toolbar, which is the window's title bar on macOS and Windows, as Rider's header is.
    fn toolbar(&self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.workspace.as_ref().map(Workspace::name);
        div()
            .id("toolbar")
            .h(px(TOOLBAR_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            // Room for macOS's window buttons, drawn by the system over the merged title bar.
            .pl(px(if cfg!(target_os = "macos") { 84. } else { 8. }))
            // Windows itself moves, snaps and maximizes the window by the bar, and shows its snap layouts.
            .when(cfg!(windows), |d| d.window_control_area(gpui::WindowControlArea::Drag))
            .when(cfg!(target_os = "macos"), |d| {
                d.on_mouse_down(MouseButton::Left, |event, window, _| {
                    if event.click_count == 2 {
                        window.titlebar_double_click();
                    } else {
                        window.start_window_move();
                    }
                })
            })
            .when(!cfg!(target_os = "macos"), |d| {
                d.child(
                    widget("main-menu")
                        .debug_selector(|| "main menu".into())
                        .child(icon("main-menu", 20.))
                        .on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.open_main_menu(window, cx))),
                )
            })
            .children(name.map(|name| {
                widget("project-widget")
                    .debug_selector(|| "project widget".into())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(project_badge(&name, 20.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                    .child(icon("chevron-down", 12.))
                    .on_click(cx.listener(|m, _: &ClickEvent, _, cx| {
                        m.project_menu = !m.project_menu;
                        cx.notify();
                    }))
            }))
            .children(self.workspace.is_some().then(|| self.run_widget(cx)))
            .children(self.branch.clone().map(|branch| {
                widget("branch-widget")
                    .debug_selector(|| "branch widget".into())
                    .child(icon("branch", 16.))
                    .child(branch)
                    .child(icon("chevron-down", 12.))
                    .on_click(cx.listener(|m, _, window, cx| {
                        m.dock.show(Tool::Git);
                        m.refresh_git_log(cx);
                        window.focus(&m.git.log.focus_handle(cx));
                        cx.notify();
                    }))
            }))
            .when(cfg!(windows), |d| d.child(div().flex_1()).child(caption_buttons()))
    }

    /// The work area: a stripe down each side, the open tool windows as islands round the editor, and the gaps
    /// between them, which resize what they divide.
    fn body(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dock = &self.dock;
        let column = |places: [Place; 2], this: &Self, window: &mut Window, cx: &mut Context<Self>| {
            let open: Vec<Tool> = places.iter().filter_map(|p| dock.open_in(*p)).collect();
            (!open.is_empty()).then(|| {
                div().h_full().flex().flex_col().gap(px(GAP)).children(
                    open.into_iter()
                        .map(|t| this.tool_island(t, window, cx).flex_1().min_h_0().into_any_element())
                        .collect::<Vec<_>>(),
                )
            })
        };
        let left = column([Place::LeftTop, Place::LeftBottom], self, window, cx);
        let right = column([Place::RightTop, Place::RightBottom], self, window, cx);
        let bottom: Vec<Tool> =
            [Place::BottomLeft, Place::BottomRight].iter().filter_map(|p| dock.open_in(*p)).collect();
        let bottom = (!bottom.is_empty()).then(|| {
            // The band is the height a person dragged it to, so a tool's content stays inside it: the island takes
            // the band's height and its window scrolls what does not fit, instead of drawing past the status bar.
            div().h(px(dock.bottom_height)).flex_none().flex().gap(px(GAP)).pb(px(GAP)).children(
                bottom
                    .into_iter()
                    .map(|t| self.tool_island(t, window, cx).h_full().min_h_0().flex_1().min_w_0().into_any_element())
                    .collect::<Vec<_>>(),
            )
        });
        div()
            .id("body")
            .flex()
            .flex_1()
            .min_h_0()
            .on_drag_move(cx.listener(|w, event: &gpui::DragMoveEvent<Resize>, _, cx| {
                let (bounds, at) = (event.bounds, event.event.position);
                match event.drag(cx) {
                    Resize::Left => w.dock.left_width = f32::from(at.x - bounds.left()) - STRIPE_WIDTH - GAP / 2.,
                    Resize::Right => w.dock.right_width = f32::from(bounds.right() - at.x) - STRIPE_WIDTH - GAP / 2.,
                    Resize::Bottom => w.dock.bottom_height = f32::from(bounds.bottom() - at.y) - GAP / 2.,
                }
                w.dock = w.dock.clone().clamped();
                w.save_state(cx);
                cx.notify();
            }))
            .child(self.stripe(Place::LEFT_STRIPE, cx))
            .children(left.map(|l| l.w(px(dock.left_width)).flex_none().pb(px(GAP))))
            .child(if self.has_left() { splitter(Resize::Left).into_any_element() } else { div().into_any_element() })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        self.editor_island(cx)
                            .flex_1()
                            .min_h_0()
                            .when(bottom.is_none(), |d| d.mb(px(GAP)))
                            .debug_selector(|| "editor island".into())
                            // A window's button dropped outside the stripes floats it there, as IntelliJ's does.
                            .on_drop(
                                cx.listener(|w, dragged: &DraggedTool, window, cx| w.float_tool(dragged.0, window, cx)),
                            ),
                    )
                    .children(bottom.map(|b| div().flex().flex_col().child(splitter(Resize::Bottom)).child(b))),
            )
            .child(if self.has_right() { splitter(Resize::Right).into_any_element() } else { div().into_any_element() })
            .children(right.map(|r| r.w(px(dock.right_width)).flex_none().pb(px(GAP))))
            .child(self.stripe(Place::RIGHT_STRIPE, cx))
    }

    fn has_left(&self) -> bool {
        self.dock.open_in(Place::LeftTop).is_some() || self.dock.open_in(Place::LeftBottom).is_some()
    }

    fn has_right(&self) -> bool {
        self.dock.open_in(Place::RightTop).is_some() || self.dock.open_in(Place::RightBottom).is_some()
    }

    /// A stripe: its side's top group, its split group under a rule, and the bottom panel's half at its foot.
    /// Each group takes a window's button dropped on it.
    fn stripe(&self, places: [Place; 3], cx: &mut Context<Self>) -> impl IntoElement {
        let group = |place: Place, this: &Self, cx: &mut Context<Self>| {
            div()
                .id(place.name())
                .debug_selector(move || format!("stripe group: {}", place.name()))
                .min_h(px(36.))
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.))
                .rounded(px(6.))
                .drag_over::<DraggedTool>(|style, _, _, _| style.bg(theme::drop_zone()))
                .on_drop(
                    cx.listener(move |w, dragged: &DraggedTool, window, cx| w.move_tool(dragged.0, place, window, cx)),
                )
                .children({
                    let mut buttons = Vec::new();
                    for tool in this.dock.in_place(place).collect::<Vec<_>>() {
                        buttons.push(this.stripe_button(tool, cx));
                    }
                    buttons
                })
        };
        let split_used = self.dock.in_place(places[1]).next().is_some();
        div()
            .w(px(STRIPE_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .px(px(4.))
            .pt(px(2.))
            .pb(px(GAP))
            .child(group(places[0], self, cx))
            .when(split_used, |d| d.child(div().w(px(20.)).h(px(1.)).my(px(4.)).bg(theme::separator())))
            .child(group(places[1], self, cx))
            .child(div().flex_1())
            .child(group(places[2], self, cx))
    }

    fn stripe_button(&self, tool: Tool, cx: &mut Context<Self>) -> gpui::AnyElement {
        let open = self.dock.is_open(tool);
        div()
            .id(tool.title())
            .debug_selector(move || format!("stripe: {}", tool.title()))
            .size(px(32.))
            .flex_none()
            .rounded(px(6.))
            .flex()
            .items_center()
            .justify_center()
            .when(open, |d| d.bg(theme::accent()))
            .when(!open, |d| d.hover(|d| d.bg(theme::button_hover())).active(|d| d.bg(theme::button_pressed())))
            .child(icon(tool.icon(), 20.))
            .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| w.toggle_tool(tool, window, cx)))
            .on_drag(DraggedTool(tool), |dragged, _, _, cx| cx.new(|_| dragged.clone()))
            .into_any_element()
    }

    /// A tool window's island: its header (the title and Hide) over its content.
    fn tool_island(&self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        let header = div()
            .id(("tool-header", tool as usize))
            .when(self.dock.is_floating(tool), |d| {
                d.debug_selector(move || format!("floating: {}", tool.title()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |w, e: &MouseDownEvent, _, _| w.grab(tool, e, false)),
                    )
                    .on_drag(FloatDrag { tool, resize: false }, |dragged, _, _, cx| cx.new(|_| *dragged))
            })
            .h(px(40.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(14.))
            .pr(px(6.))
            .child(div().font_weight(FontWeight::SEMIBOLD).child(tool.title()))
            .children(
                self.tool_subtitle(tool, cx).map(|s| div().truncate().text_color(theme::text_secondary()).child(s)),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id(("hide", tool as usize))
                    .size(px(24.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|d| d.bg(theme::button_hover()))
                    .active(|d| d.bg(theme::button_pressed()))
                    .child(icon("hide", 16.))
                    .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| w.toggle_tool(tool, window, cx))),
            );
        let content = match tool {
            Tool::Solution => self.solution_content(window, cx).into_any_element(),
            Tool::Problems => self.problems_content(cx).into_any_element(),
            Tool::Find => self.find_content(cx).into_any_element(),
            Tool::NuGet => self.nuget.clone().into_any_element(),
            Tool::Build => self.build.clone().into_any_element(),
            Tool::Run => self.run_console.clone().into_any_element(),
            Tool::Debug => self.debug_view.clone().into_any_element(),
            Tool::UnitTests => self.tests.view.clone().into_any_element(),
            Tool::Commit => self.git.commit.clone().into_any_element(),
            Tool::Git => self.git.log.clone().into_any_element(),
            Tool::Terminal => {
                self.ensure_terminal(cx);
                self.terminal.clone().into_any_element()
            }
        };
        island().child(header).child(content)
    }

    fn tool_subtitle(&self, tool: Tool, cx: &App) -> Option<String> {
        match tool {
            Tool::Solution => None,
            Tool::Problems => self
                .active_editor()
                .map(|e| e.read(cx).path.file_name().unwrap_or_default().to_string_lossy().into_owned()),
            Tool::Find => self.found.as_ref().map(|(title, found)| format!("{title}: {}", found.len())),
            Tool::NuGet => None,
            Tool::Build => self.build_outcome.as_ref().map(|outcome| outcome.message.clone()),
            Tool::Run => self.run_outcome.as_ref().map(|outcome| outcome.message.clone()),
            Tool::Debug => Some(self.debug_view.read(cx).status().to_string()),
            Tool::UnitTests => None,
            Tool::Terminal | Tool::Commit | Tool::Git => None,
        }
    }

    /// The floating windows open, over the frame where they were left.
    // shortcut: drawn inside the main window, not as windows of their own; a window shrunk past one leaves it
    // out of reach until Restore Default Layout. Upgrade when gpui's child windows serve (IntelliJ's Window mode).
    fn floating_islands(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let open: Vec<(Tool, Rect)> = self.dock.open_floating().collect();
        open.into_iter()
            .map(|(tool, r)| {
                self.tool_island(tool, window, cx)
                    .occlude()
                    .absolute()
                    .left(px(r.x))
                    .top(px(r.y))
                    .w(px(r.w))
                    .h(px(r.h))
                    .border_1()
                    .border_color(theme::popup_border())
                    .shadow_lg()
                    .child(
                        div()
                            .id(("float-resize", tool as usize))
                            .debug_selector(move || format!("floating corner: {}", tool.title()))
                            .absolute()
                            .right_0()
                            .bottom_0()
                            .size(px(12.))
                            .cursor(gpui::CursorStyle::ResizeUpLeftDownRight)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |w, e: &MouseDownEvent, _, _| w.grab(tool, e, true)),
                            )
                            .on_drag(FloatDrag { tool, resize: true }, |dragged, _, _, cx| cx.new(|_| *dragged)),
                    )
                    .into_any_element()
            })
            .collect()
    }

    /// Notes where a floating window was pressed, from the point a drag of it moves.
    fn grab(&mut self, tool: Tool, event: &MouseDownEvent, resize: bool) {
        if let Some(r) = self.dock.floating_at(tool) {
            let from = if resize { gpui::point(px(r.x + r.w), px(r.y + r.h)) } else { gpui::point(px(r.x), px(r.y)) };
            self.float_grab = event.position - from;
        }
    }

    fn focused_tool(&self, window: &Window, cx: &App) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| self.tool_area(*t, cx).contains_focused(window, cx))
    }

    /// Floats a docked window in the middle of the window, or docks a floating one back in its place.
    pub fn toggle_floating(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        if self.dock.is_floating(tool) {
            self.dock.dock(tool);
        } else {
            let size = window.viewport_size();
            let (w, h) = (420., 360.);
            let (x, y) = ((f32::from(size.width) - w) / 2., (f32::from(size.height) - h) / 2.);
            self.dock.float(tool, Rect { x, y, w, h });
            self.dock = self.dock.clone().clamped();
        }
        window.focus(&self.tool_focus(tool, cx));
        self.save_state(cx);
        cx.notify();
    }

    /// Floats a window where its button was dropped, its header under the pointer.
    fn float_tool(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        let at = window.mouse_position();
        let (x, y) = (f32::from(at.x) - 40., f32::from(at.y) - 20.);
        self.dock.float(tool, Rect { x, y, w: 420., h: 360. });
        self.dock = self.dock.clone().clamped();
        window.focus(&self.tool_focus(tool, cx));
        self.save_state(cx);
        cx.notify();
    }

    /// Where a tool window takes the keys when it opens: NuGet's search field, as Rider's.
    fn tool_focus(&self, tool: Tool, cx: &App) -> FocusHandle {
        match tool {
            Tool::NuGet => self.nuget.read(cx).search_field().focus_handle(cx),
            _ => self.tool_area(tool, cx),
        }
    }

    /// The focus that holds everything in a tool window, which has the keys while any part of it does.
    fn tool_area(&self, tool: Tool, cx: &App) -> FocusHandle {
        match tool {
            Tool::Solution => self.explorer_focus.clone(),
            Tool::Problems => self.problems_focus.clone(),
            Tool::Find => self.find_focus.clone(),
            Tool::NuGet => self.nuget.focus_handle(cx),
            Tool::Build => self.build.focus_handle(cx),
            Tool::Run => self.run_console.focus_handle(cx),
            Tool::Debug => self.debug_view.focus_handle(cx),
            Tool::UnitTests => self.tests.view.focus_handle(cx),
            Tool::Terminal => self.terminal.focus_handle(cx),
            Tool::Commit => self.git.commit.focus_handle(cx),
            Tool::Git => self.git.log.focus_handle(cx),
        }
    }

    /// Opens or closes a tool window; one opened takes the keys, one closed with them gives them to the editor.
    pub fn toggle_tool(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        if tool == Tool::Git {
            self.refresh_git_log(cx);
        }
        if tool == Tool::Terminal {
            self.ensure_terminal(cx);
        }
        let had_focus = self.tool_area(tool, cx).contains_focused(window, cx);
        self.dock.toggle(tool);
        if self.dock.is_open(tool) {
            window.focus(&self.tool_focus(tool, cx));
        } else if had_focus {
            match self.active_editor() {
                Some(editor) => window.focus(&editor.focus_handle(cx)),
                None => window.focus(&self.root_focus),
            }
        }
        self.save_state(cx);
        cx.notify();
    }

    fn move_tool(&mut self, tool: Tool, place: Place, _: &mut Window, cx: &mut Context<Self>) {
        self.dock.move_to(tool, place);
        self.save_state(cx);
        cx.notify();
    }

    pub fn dock(&self) -> &DockLayout {
        &self.dock
    }

    fn solution_content(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.explorer_focus.contains_focused(window, cx);
        let row_count = self.rows.len();
        div()
            .id("solution")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .key_context("Explorer")
            .track_focus(&self.explorer_focus)
            .on_action(cx.listener(|w, _: &ExplorerUp, _, cx| {
                w.selected = Some(w.selected.map_or(0, |s| s.saturating_sub(1)));
                cx.notify()
            }))
            .on_action(cx.listener(|w, _: &ExplorerDown, _, cx| {
                w.selected = Some(w.selected.map_or(0, |s| (s + 1).min(w.rows.len().saturating_sub(1))));
                cx.notify()
            }))
            .on_action(cx.listener(|w, _: &ExplorerExpand, _, cx| {
                if let Some(s) = w.selected {
                    w.toggle(s, Some(true), cx)
                }
            }))
            .on_action(cx.listener(|w, _: &ExplorerCollapse, _, cx| {
                if let Some(s) = w.selected {
                    w.toggle(s, Some(false), cx)
                }
            }))
            .on_action(cx.listener(|w, _: &ExplorerOpen, window, cx| {
                if let Some(s) = w.selected {
                    w.activate_row(s, window, cx)
                }
            }))
            .on_action(cx.listener(|w, _: &SolutionNew, _, cx| w.open_new_menu(cx)))
            .on_action(cx.listener(|w, _: &SolutionRename, window, cx| w.run_row_item(&["Rename..."], window, cx)))
            .on_action(
                cx.listener(|w, _: &SolutionDelete, window, cx| {
                    w.run_row_item(&["Delete...", "Remove..."], window, cx)
                }),
            )
            .on_action(cx.listener(|w, _: &SolutionCut, window, cx| w.run_row_item(&["Cut"], window, cx)))
            .on_action(cx.listener(|w, _: &SolutionCopy, window, cx| w.run_row_item(&["Copy"], window, cx)))
            .on_action(cx.listener(|w, _: &SolutionPaste, window, cx| w.run_row_item(&["Paste"], window, cx)))
            .child(
                uniform_list(
                    "solution-rows",
                    row_count,
                    cx.processor(move |w, range: std::ops::Range<usize>, _, cx| {
                        range.map(|i| w.solution_row(i, focused, cx).into_any_element()).collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .px(px(6.))
                .pb(px(6.)),
            )
    }

    /// Problems: its tabs, and the one chosen.
    fn problems_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let solution_count: usize = self.solution_problems.iter().map(|(_, p)| p.len()).sum();
        let tab = |id: &'static str, title: String, which: ProblemsTab, cx: &mut Context<Self>| {
            let chosen = self.problems_tab == which;
            div()
                .id(id)
                .debug_selector(move || format!("problems tab: {id}"))
                .h(px(26.))
                .px(px(10.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .when(chosen, |d| d.bg(theme::tab_selected()))
                .when(!chosen, |d| d.text_color(theme::text_muted()).hover(|d| d.bg(theme::button_hover())))
                .child(title)
                .on_click(cx.listener(move |w, _: &ClickEvent, _, cx| w.show_problems_tab(which, cx)))
        };
        let titled = |name: &str, n: usize| if n == 0 { name.to_string() } else { format!("{name} {n}") };
        let body = match self.problems_tab {
            ProblemsTab::File => self.file_problems(cx).into_any_element(),
            ProblemsTab::Solution => self.solution_problems_view(cx).into_any_element(),
            ProblemsTab::Toolset => self.toolset_problems().into_any_element(),
            ProblemsTab::Build => self.build_problems_view(cx).into_any_element(),
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .px(px(8.))
                    .pb(px(4.))
                    .child(tab("file", "File".into(), ProblemsTab::File, cx))
                    .child(tab("solution", titled("Solution", solution_count), ProblemsTab::Solution, cx))
                    .child(tab("build", titled("Build", self.build_problems.len()), ProblemsTab::Build, cx))
                    .child(tab("toolset", titled("Toolset", self.load_problems.len()), ProblemsTab::Toolset, cx)),
            )
            .child(body)
    }

    /// One problem's row, going to its place when clicked.
    fn problem_row(
        &self,
        id: (&'static str, usize),
        error: bool,
        message: String,
        place: String,
        go: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .h(px(ROW_HEIGHT))
            .px(px(8.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .gap(px(6.))
            .hover(|d| d.bg(theme::row_hover()))
            .child(icon(if error { "error" } else { "warning" }, 16.))
            .child(div().truncate().child(message))
            .child(div().flex_none().text_color(theme::text_secondary()).child(place))
            .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| go(w, window, cx)))
    }

    /// The whole solution's problems, by file, as Problems' Solution tab.
    fn solution_problems_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.solution_wide {
            return div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .track_focus(&self.problems_focus)
                .child(div().text_color(theme::text_secondary()).child("Solution-wide analysis is off."))
                .child(
                    widget("solution-wide-on")
                        .debug_selector(|| "turn on solution-wide analysis".into())
                        .text_color(theme::accent())
                        .child("Turn On Solution-Wide Analysis")
                        .on_click(cx.listener(|w, _: &ClickEvent, _, cx| w.set_solution_wide(true, cx))),
                )
                .into_any_element();
        }
        if self.solution_problems.is_empty() {
            let said = if self.server.is_some() { "No problems in the solution." } else { "Roslyn is not running." };
            return empty_text(said).track_focus(&self.problems_focus).into_any_element();
        }
        let mut rows = Vec::new();
        for (path, found) in &self.solution_problems {
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            rows.push(
                div()
                    .h(px(ROW_HEIGHT))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon(file_icon(&name), 16.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                    .child(div().text_color(theme::text_secondary()).child(count(found.len(), "problem")))
                    .into_any_element(),
            );
            for problem in found {
                let (path, at) = (path.clone(), (problem.span.start.line, problem.span.start.character));
                let index = rows.len();
                rows.push(
                    self.problem_row(
                        ("solution-problem", index),
                        problem.error,
                        problem.message.clone(),
                        format!(":{}", at.0 + 1),
                        move |w, window, cx| w.open_file(&path, Some(at), window, cx),
                        cx,
                    )
                    .pl(px(30.))
                    .into_any_element(),
                );
            }
        }
        div()
            .id("solution-problems")
            .track_focus(&self.problems_focus)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(6.))
            .children(rows)
            .into_any_element()
    }

    /// The projects that would not load whole, as Problems' Toolset tab.
    fn toolset_problems(&self) -> impl IntoElement {
        if self.load_problems.is_empty() {
            return empty_text("Every project loaded.").track_focus(&self.problems_focus).into_any_element();
        }
        div()
            .id("toolset-problems")
            .track_focus(&self.problems_focus)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(6.))
            .children(self.load_problems.iter().map(|(project, error, message)| {
                let name = project.file_name().unwrap_or_default().to_string_lossy().into_owned();
                div()
                    .min_h(px(ROW_HEIGHT))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon(if *error { "error" } else { "warning" }, 16.))
                    .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).child(name))
                    .child(div().child(message.clone()))
            }))
            .into_any_element()
    }

    /// The shown file's problems, as Problems' File tab: each a row that goes to its place.
    fn file_problems(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(editor) = self.active_editor().cloned() else {
            return empty_text("Open a file to see its problems.").track_focus(&self.problems_focus).into_any_element();
        };
        let read = editor.read(cx).buffer.read(cx);
        let rows: Vec<(bool, String, usize, usize, usize)> = read
            .diagnostics
            .iter()
            .map(|d| {
                let at = read.document.position(d.range.start);
                (d.error, d.message.clone(), at.line, at.utf16_column, d.range.start)
            })
            .collect();
        if rows.is_empty() {
            return empty_text("No problems in this file.").track_focus(&self.problems_focus).into_any_element();
        }
        div()
            .id("problems")
            .key_context("Problems")
            .track_focus(&self.problems_focus)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(6.))
            .children(rows.into_iter().enumerate().map(|(i, (error, message, line, column, _))| {
                let editor = editor.clone();
                div()
                    .id(("problem", i))
                    .debug_selector(move || format!("problem: {i}"))
                    .h(px(ROW_HEIGHT))
                    .px(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .hover(|d| d.bg(theme::row_hover()))
                    .child(icon(if error { "error" } else { "warning" }, 16.))
                    .child(div().truncate().child(message))
                    .child(div().flex_none().text_color(theme::text_secondary()).child(format!(":{}", line + 1)))
                    .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                        editor.update(cx, |e, cx| e.place_caret(line, column, cx));
                        window.focus(&editor.focus_handle(cx));
                    }))
            }))
            .into_any_element()
    }

    fn solution_row(&self, index: usize, focused: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let row = &self.rows[index];
        let selected = self.selected == Some(index);
        let chevron = row.expandable.then(|| if row.expanded { "chevron-down" } else { "chevron-right" });
        let kind = match row.kind {
            NodeKind::Solution => "solution",
            NodeKind::SolutionFolder => "solution-folder",
            NodeKind::Project | NodeKind::ProjectReference => "project",
            NodeKind::Dependencies => "dependencies",
            NodeKind::Framework => "framework",
            NodeKind::Package => "package",
            NodeKind::Folder if index == 0 => "project",
            NodeKind::Folder => "folder",
            NodeKind::File => file_icon(&row.name),
        };
        let tip = row.tip.clone();
        let git_kind =
            row.path.as_ref().and_then(|p| self.git_status()?.changes.iter().find(|c| &c.path == p).map(|c| c.kind));
        div()
            .id(("row", index))
            .debug_selector({
                let name = row.name.clone();
                move || format!("solution row: {name}")
            })
            .h(px(ROW_HEIGHT))
            .w_full()
            .rounded(px(4.))
            .flex()
            .items_center()
            .gap(px(4.))
            .pl(px(6. + row.depth as f32 * 18.))
            .when(selected, |d| d.bg(if focused { theme::selection() } else { theme::selection_inactive() }))
            .when(!selected, |d| d.hover(|d| d.bg(theme::row_hover())))
            .child(match chevron {
                Some(chevron) => div()
                    .id(("chevron", index))
                    .on_click(cx.listener(move |w, _: &ClickEvent, _, cx| w.toggle(index, None, cx)))
                    .child(icon(chevron, 16.))
                    .into_any_element(),
                None => div().w(px(16.)).flex_none().into_any_element(),
            })
            .child(icon(kind, 16.))
            .child(
                div()
                    .truncate()
                    .when(index == 0, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .when(row.dimmed, |d| d.text_color(theme::text_muted()))
                    .when_some(git_kind, |d, kind| {
                        d.text_color(match kind {
                            rsharp_core::git::ChangeKind::Conflicted => theme::error(),
                            rsharp_core::git::ChangeKind::Added
                            | rsharp_core::git::ChangeKind::Unversioned
                            | rsharp_core::git::ChangeKind::Modified
                            | rsharp_core::git::ChangeKind::Renamed => theme::modified(),
                            _ => theme::text_muted(),
                        })
                    })
                    .child(row.name.clone()),
            )
            .when_some(row.detail.clone(), |d, detail| {
                d.child(div().flex_none().text_color(theme::text_secondary()).child(detail))
            })
            .when_some(tip, |d, tip| d.tooltip(move |_, cx| cx.new(|_| Tip(tip.clone().into())).into()))
            .when_some(
                row.path
                    .clone()
                    .filter(|_| matches!(row.kind, NodeKind::File | NodeKind::Folder) && !row.key.starts_with("si:")),
                |d, path| {
                    let drag = file_moves::RowDrag { paths: vec![path], name: row.name.clone().into() };
                    d.on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                },
            )
            .on_drop(cx.listener(move |w, drag: &file_moves::RowDrag, window, cx| w.dropped(drag, index, window, cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |w, event: &MouseDownEvent, window, cx| {
                    window.focus(&w.explorer_focus);
                    w.open_row_menu(index, event.position, cx);
                }),
            )
            .on_click(cx.listener(move |w, event: &ClickEvent, window, cx| {
                w.selected = Some(index);
                window.focus(&w.explorer_focus);
                if event.click_count() >= 2 {
                    w.activate_row(index, window, cx);
                }
                cx.notify();
            }))
    }

    fn editor_island(&self, cx: &mut Context<Self>) -> gpui::Div {
        let open = self.area.groups.iter().any(|g| !g.tabs.is_empty());
        island().min_w_0().child(if open {
            self.split_view(&self.area.layout.clone(), cx)
        } else {
            empty_editor().into_any_element()
        })
    }

    /// The groups as laid out: side by side or one above another, a line of the frame between them.
    fn split_view(&self, split: &Split, cx: &mut Context<Self>) -> gpui::AnyElement {
        let (children, row) = match split {
            Split::Group(group) => return self.group_view(*group, cx).into_any_element(),
            Split::Row(children) => (children, true),
            Split::Column(children) => (children, false),
        };
        // shortcut: the groups share the room equally; let the line between them be dragged once a person asks.
        div()
            .flex()
            .when(row, |d| d.flex_row())
            .when(!row, |d| d.flex_col())
            .flex_1()
            .min_w_0()
            .min_h_0()
            .children(children.iter().enumerate().map(|(i, child)| {
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .when(i > 0 && row, |d| d.border_l_1().border_color(theme::frame()))
                    .when(i > 0 && !row, |d| d.border_t_1().border_color(theme::frame()))
                    .child(self.split_view(child, cx))
            }))
            .into_any_element()
    }

    fn group_view(&self, group: usize, cx: &mut Context<Self>) -> gpui::Div {
        let g = &self.area.groups[group];
        let tabs: Vec<_> = (0..g.tabs.len()).map(|i| self.tab(group, i, cx)).collect();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .debug_selector(move || format!("group {group}"))
            // A press anywhere in a group makes it the one keys and the IDE's actions go to.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |w, _, _, cx| {
                    if w.area.current != group {
                        w.area.current = group;
                        w.follow_find_bar(cx);
                        cx.notify();
                    }
                }),
            )
            .child(
                div()
                    .id(("tabs", group))
                    .h(px(40.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .px(px(6.))
                    .border_b_1()
                    .border_color(theme::frame())
                    .overflow_x_scroll()
                    .children(tabs),
            )
            .when(group == self.area.current, |d| d.children(self.find_bar.clone()))
            .child(match g.active_editor() {
                Some(editor) => div().flex_1().min_h_0().child(editor.clone()).into_any_element(),
                None => empty_editor().into_any_element(),
            })
    }

    fn tab(&self, group: usize, index: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let tab = &self.area.groups[group].tabs[index];
        let pinned = tab.pinned;
        let editor = tab.editor.read(cx);
        let name = editor.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let name_for_drag = name.clone();
        let selected = self.area.groups[group].active == Some(index);
        let current = group == self.area.current;
        let modified = editor.is_modified(cx);
        let read_only = editor.buffer.read(cx).read_only;
        let hover = SharedString::from(format!("tab-{group}-{index}"));
        let id = |kind: &str| gpui::ElementId::NamedInteger(format!("{kind}-{group}").into(), index as u64);
        let selector = format!("tab {group} {name}");
        div()
            .id(id("tab"))
            .debug_selector(move || selector.clone())
            .group(hover.clone())
            .h(px(28.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(8.))
            .pr(px(4.))
            .rounded(px(6.))
            .border_1()
            .when(selected && current, |d| d.bg(theme::tab_selected()).border_color(theme::tab_selected_border()))
            .when(selected && !current, |d| {
                d.bg(theme::tab_selected_inactive()).border_color(gpui::transparent_black())
            })
            .when(!selected, |d| {
                d.border_color(gpui::transparent_black())
                    .text_color(theme::text_muted())
                    .hover(|d| d.bg(theme::button_hover()))
            })
            .child(icon(file_icon(&name), 16.))
            .child(name)
            .when(read_only, |d| d.child(icon("locked", 14.)))
            .child(
                div()
                    .id(id("close"))
                    .size(px(16.))
                    .rounded(px(3.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|d| d.bg(theme::button_hover()))
                    .active(|d| d.bg(theme::button_pressed()))
                    .child(if pinned {
                        icon("pin", 16.).into_any_element()
                    } else if modified {
                        div().size(px(7.)).rounded_full().bg(theme::modified()).into_any_element()
                    } else {
                        icon("close", 16.)
                            .invisible()
                            .group_hover(hover, |s| s.visible())
                            .when(selected, |i| i.visible())
                            .into_any_element()
                    })
                    // A pinned tab's pin unpins it, as Rider's does; anything else's button closes it.
                    .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        if pinned {
                            w.area.pin(group, index, false);
                            w.save_state(cx);
                            cx.notify();
                        } else {
                            w.close_tab(group, index, window, cx)
                        }
                    })),
            )
            .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| w.show_tab(group, index, window, cx)))
            .on_drag(TabDrag { group, index, name: SharedString::from(name_for_drag) }, |drag, _, _, cx| {
                cx.new(|_| drag.clone())
            })
            // shortcut: the menu acts on the tab it opened on by showing it first; Rider leaves the tab shown as it was.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |w, event: &MouseDownEvent, window, cx| {
                    w.show_tab(group, index, window, cx);
                    w.tab_menu = Some(event.position);
                    cx.notify();
                }),
            )
            .into_any_element()
    }

    /// The tab actions, each registered only while it has something to act on.
    fn tab_actions<T: InteractiveElement + FluentBuilder>(&self, d: T, cx: &mut Context<Self>) -> T {
        let group = self.area.current;
        let active = self.area.group().active;
        let some = |which: &dyn Fn(usize) -> bool| !self.area.unpinned(group, which).is_empty();
        let others = active.is_some_and(|a| some(&|i| i != a));
        let left = active.is_some_and(|a| some(&|i| i < a));
        let right = active.is_some_and(|a| some(&|i| i > a));
        let any = some(&|_| true);
        let split = self.area.is_split();
        let closed = !self.area.closed.is_empty();
        d.when(active.is_some(), |d| {
            d.on_action(cx.listener(|w, _: &ShowHistory, _, cx| {
                if let Some(editor) = w.active_editor() {
                    let buffer = editor.read(cx).buffer.clone();
                    cx.defer(move |cx| {
                        crate::history_window::open(buffer, cx);
                    });
                }
            }))
            .on_action(cx.listener(|w, _: &FileEncoding, _, cx| {
                w.status_menu = Some(StatusMenu::Encodings);
                cx.notify()
            }))
            .on_action(cx.listener(|w, _: &LineSeparators, _, cx| {
                w.status_menu = Some(StatusMenu::LineSeparators);
                cx.notify()
            }))
            .on_action(cx.listener(|w, _: &OpenTabInNewWindow, window, cx| {
                let (group, index) = (w.area.current, w.area.group().active.unwrap_or(0));
                let at = window.bounds().origin + gpui::point(px(80.), px(80.));
                w.move_to_new_window(group, index, at, window, cx)
            }))
            .on_action(cx.listener(|w, _: &SplitRight, window, cx| w.split(true, window, cx)))
            .on_action(cx.listener(|w, _: &SplitDown, window, cx| w.split(false, window, cx)))
            .on_action(cx.listener(|w, _: &PinTab, _, cx| {
                let group = w.area.current;
                if let Some(at) = w.area.group().active {
                    let pinned = w.area.group().tabs[at].pinned;
                    w.area.pin(group, at, !pinned);
                    w.save_state(cx);
                    cx.notify();
                }
            }))
        })
        .when(others, |d| {
            d.on_action(cx.listener(move |w, _: &CloseOtherTabs, window, cx| {
                let keep = w.area.group().active;
                w.close_tabs(|i| Some(i) != keep, window, cx)
            }))
        })
        .when(left, |d| {
            d.on_action(cx.listener(|w, _: &CloseTabsToLeft, window, cx| {
                let at = w.area.group().active.unwrap_or(0);
                w.close_tabs(|i| i < at, window, cx)
            }))
        })
        .when(right, |d| {
            d.on_action(cx.listener(|w, _: &CloseTabsToRight, window, cx| {
                let at = w.area.group().active.unwrap_or(0);
                w.close_tabs(|i| i > at, window, cx)
            }))
        })
        .when(any, |d| d.on_action(cx.listener(|w, _: &CloseAllTabs, window, cx| w.close_tabs(|_| true, window, cx))))
        .when(self.area.groups.iter().any(|g| g.tabs.iter().any(|t| !t.pinned)), |d| {
            d.on_action(cx.listener(|w, _: &CloseAllButPinned, window, cx| {
                // From the last group, so one closing as it empties leaves the earlier ones' indices standing.
                for group in (0..w.area.groups.len()).rev() {
                    for index in w.area.unpinned(group, |_| true) {
                        w.area.close(group, index, cx);
                    }
                }
                w.focus_editor(window, cx)
            }))
        })
        .when(split, |d| {
            d.on_action(cx.listener(|w, _: &Unsplit, window, cx| {
                w.area.unsplit(false, cx);
                w.focus_editor(window, cx)
            }))
            .on_action(cx.listener(|w, _: &UnsplitAll, window, cx| {
                w.area.unsplit(true, cx);
                w.focus_editor(window, cx)
            }))
        })
        .when(closed, |d| {
            d.on_action(cx.listener(|w, _: &ReopenClosedTab, window, cx| w.reopen_closed_tab(window, cx)))
        })
    }

    /// The status bar's Line Separators or encoding menu, the file's own checked; a choice is how it is written
    /// from its next save.
    fn status_menu_view(&self, kind: StatusMenu, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(buffer) = self.active_editor().map(|e| e.read(cx).buffer.clone()) else { return div() };
        let read = buffer.read(cx);
        let items: Vec<(String, bool)> = match kind {
            StatusMenu::LineSeparators => rsharp_text::LineEnding::ALL
                .iter()
                .map(|e| (e.title().to_string(), *e == read.document.line_ending))
                .collect(),
            StatusMenu::Encodings => {
                rsharp_core::encodings::LISTED.iter().map(|c| (c.name().to_string(), *c == read.charset)).collect()
            }
            StatusMenu::TargetFramework => read
                .contexts
                .iter()
                .enumerate()
                .map(|(i, c)| (c["_vs_label"].as_str().unwrap_or_default().to_string(), i == read.context))
                .collect(),
        };
        let rows: Vec<_> = items
            .into_iter()
            .enumerate()
            .map(|(i, (title, checked))| {
                let buffer = buffer.clone();
                let selector = format!("status menu: {title}");
                div()
                    .id(("status-menu", i))
                    .debug_selector(move || selector.clone())
                    .h(px(24.))
                    .mx(px(6.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .hover(|d| d.bg(theme::selection()))
                    .child(div().size(px(14.)).when(checked, |d| d.child(icon("check", 14.))))
                    .child(title)
                    .on_click(cx.listener(move |w, _: &ClickEvent, _, cx| {
                        w.status_menu = None;
                        buffer.update(cx, |b, cx| match kind {
                            StatusMenu::LineSeparators => b.set_line_ending(rsharp_text::LineEnding::ALL[i], cx),
                            StatusMenu::Encodings => b.set_charset(rsharp_core::encodings::LISTED[i], cx),
                            StatusMenu::TargetFramework => b.choose_context(i, cx),
                        });
                        cx.notify();
                    }))
            })
            .collect();
        div().absolute().bottom(px(32.)).right(px(10.)).child(
            div()
                .id("status-menu")
                .w(px(260.))
                .max_h(px(420.))
                .overflow_y_scroll()
                .py(px(6.))
                .rounded(px(8.))
                .bg(theme::popup())
                .border_1()
                .border_color(theme::popup_border())
                .shadow_lg()
                .on_mouse_down_out(cx.listener(|w, _, _, cx| {
                    w.status_menu = None;
                    cx.notify();
                }))
                .children(rows),
        )
    }

    /// A tab's menu, Rider's items in its order, each greyed where it cannot run.
    fn tab_menu_view(&self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        const ITEMS: &[&str] = &[
            "Close Tab",
            "Close Other Tabs",
            "Close All Tabs",
            "Close All but Pinned",
            "Close Tabs to the Left",
            "Close Tabs to the Right",
            "",
            "Split Right",
            "Split Down",
            "Unsplit",
            "Unsplit All",
            "",
            "Pin Tab",
            "Open Tab in New Window",
            "",
            "Reopen Closed Tab",
        ];
        let pinned = self.area.group().active.is_some_and(|a| self.area.group().tabs[a].pinned);
        let rows: Vec<gpui::AnyElement> = ITEMS
            .iter()
            .map(|title| {
                let Some(i) = ALL.iter().position(|a| a.title == *title) else {
                    return div().h(px(1.)).my(px(4.)).mx(px(8.)).bg(theme::popup_border()).into_any_element();
                };
                let enabled = window.is_action_available(&*(ALL[i].action)(), cx);
                let shown = if *title == "Pin Tab" && pinned { "Unpin Tab" } else { title };
                div()
                    .id(("tab-menu", i))
                    .debug_selector(move || format!("tab menu: {}", ALL[i].title))
                    .h(px(24.))
                    .mx(px(6.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .when(enabled, |d| d.hover(|d| d.bg(theme::selection())))
                    .when(!enabled, |d| d.text_color(theme::text_secondary()))
                    .child(shown)
                    .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| {
                        w.tab_menu = None;
                        if enabled {
                            window.dispatch_action((ALL[i].action)(), cx);
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        div().absolute().top(at.y).left(at.x).child(
            div()
                .id("tab-menu")
                .w(px(240.))
                .py(px(6.))
                .rounded(px(8.))
                .bg(theme::popup())
                .border_1()
                .border_color(theme::popup_border())
                .shadow_lg()
                .on_mouse_down_out(cx.listener(|w, _, _, cx| {
                    w.tab_menu = None;
                    cx.notify();
                }))
                .children(rows),
        )
    }

    fn welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let recent: Vec<_> = store(cx).read::<RecentSolutions>().existing().take(10).cloned().collect();
        island()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).child("RSharp"))
            .child(div().text_color(theme::text_muted()).child("Open a folder, solution or project to start."))
            .child(
                crate::settings_dialog::button("New Solution", true, true)
                    .mt(px(12.))
                    .on_click(cx.listener(|m, _: &ClickEvent, w, cx| m.show_new_solution(w, cx))),
            )
            .child(
                div()
                    .id("open")
                    .mt(px(12.))
                    .h(px(28.))
                    .px(px(14.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .bg(theme::accent())
                    .text_color(theme::on_accent())
                    .child(format!("Open…  {}", if cfg!(target_os = "macos") { "⌘O" } else { "Ctrl+O" }))
                    .on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.choose_folder(&OpenFolder, window, cx))),
            )
            .when(!recent.is_empty(), |d| {
                d.child(
                    div()
                        .mt(px(28.))
                        .w(px(420.))
                        .flex()
                        .flex_col()
                        .child(div().px(px(10.)).pb(px(6.)).text_color(theme::text_secondary()).child("Recent"))
                        .children(recent.into_iter().enumerate().map(|(i, entry)| {
                            let path = entry.path.clone();
                            div()
                                .id(("recent", i))
                                .h(px(44.))
                                .px(px(10.))
                                .rounded(px(6.))
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .hover(|d| d.bg(theme::row_hover()))
                                .active(|d| d.bg(theme::button_pressed()))
                                .child(project_badge(&entry.name, 28.))
                                .child(
                                    div().min_w_0().flex().flex_col().child(div().truncate().child(entry.name)).child(
                                        div().truncate().text_color(theme::text_secondary()).child(tilde(&entry.path)),
                                    ),
                                )
                                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                    let path = path.clone();
                                    cx.defer(move |cx| {
                                        crate::projects::open_project(&path, cx);
                                    });
                                }))
                        })),
                )
            })
    }

    fn status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = self.active_editor().map(|e| e.read(cx));
        let mut crumbs: Vec<String> = Vec::new();
        if let Some(workspace) = &self.workspace {
            crumbs.push(workspace.name());
            if let Some(path) = editor.and_then(|e| e.path.strip_prefix(&workspace.folder).ok()) {
                crumbs.extend(path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()));
            }
        }
        let (problems, server) = (
            editor.map(|e| {
                let found = &e.buffer.read(cx).diagnostics;
                (found.iter().filter(|d| d.error).count(), found.iter().filter(|d| !d.error).count())
            }),
            self.server_status()
                .map(|said| (said, if self.server_failed().is_some() { theme::error() } else { theme::text_muted() })),
        );
        div()
            .h(px(30.))
            .flex_none()
            .flex()
            .items_center()
            .px(px(10.))
            .text_color(theme::text_muted())
            .child(div().flex().items_center().gap(px(2.)).min_w_0().overflow_hidden().children(
                crumbs.into_iter().enumerate().flat_map(|(i, crumb)| {
                    [
                        (i > 0).then(|| icon("chevron-right", 12.).opacity(0.6).into_any_element()),
                        Some(status_widget(crumb).into_any_element()),
                    ]
                    .into_iter()
                    .flatten()
                }),
            ))
            .child(div().flex_1())
            .children(self.said.clone().map(|said| {
                let result = self.build_outcome.as_ref().filter(|outcome| outcome.message == said.as_ref());
                let ran = self.run_outcome.as_ref().filter(|outcome| outcome.message == said.as_ref());
                let error = match (result, ran) {
                    (Some(outcome), _) => {
                        matches!(outcome.status, BuildStatus::Failed | BuildStatus::NotStarted)
                    }
                    (None, Some(outcome)) => matches!(outcome.status, RunStatus::Refused | RunStatus::Stopped),
                    (None, None) => !self.is_building(),
                };
                status_widget(said.to_string())
                    .id("status-message")
                    .debug_selector(|| "status message".to_string())
                    .text_color(if error { theme::error() } else { theme::text_muted() })
                    .when(result.is_some() || ran.is_some(), |d| {
                        d.hover(|d| d.bg(theme::button_hover())).on_click(cx.listener(
                            |m, _: &ClickEvent, window, cx| {
                                // A run's message opens the Run window; a build's opens what says what it found.
                                let tool = if m.said.clone().is_some_and(|said| {
                                    m.run_outcome.as_ref().is_some_and(|outcome| outcome.message == said.as_ref())
                                }) {
                                    Tool::Run
                                } else if m.build_problems.is_empty() {
                                    Tool::Build
                                } else {
                                    m.problems_tab = ProblemsTab::Build;
                                    Tool::Problems
                                };
                                if !m.dock.is_open(tool) {
                                    m.dock.show(tool);
                                }
                                m.save_state(cx);
                                window.focus(&m.tool_focus(tool, cx));
                                cx.notify();
                            },
                        ))
                    })
            }))
            .children(problems.filter(|(e, w)| e + w > 0).map(|(errors, warnings)| {
                status_widget(format!("{}, {}", count(errors, "error"), count(warnings, "warning")))
                    .text_color(if errors > 0 { theme::error() } else { theme::warning() })
            }))
            .children((!self.load_problems.is_empty()).then(|| {
                let errors = self.load_problems.iter().any(|(_, e, _)| *e);
                status_widget(count(self.load_problems.len(), "project load problem"))
                    .id("load-problems")
                    .text_color(if errors { theme::error() } else { theme::warning() })
                    .hover(|d| d.bg(theme::button_hover()))
                    .on_click(cx.listener(|w, _: &ClickEvent, window, cx| {
                        if !w.dock.is_open(Tool::Problems) {
                            w.toggle_tool(Tool::Problems, window, cx);
                        }
                        w.show_problems_tab(ProblemsTab::Toolset, cx);
                    }))
            }))
            .children(server.map(|(text, colour)| status_widget(text).text_color(colour)))
            .children(editor.map(|e| {
                let caret = e.caret();
                let buffer = e.buffer.clone();
                let read = buffer.read(cx);
                let (ending, charset, indent, read_only) =
                    (read.document.line_ending, read.charset, read.indent, read.read_only);
                let indent_tip = match &read.indent_source {
                    crate::buffer::IndentSource::EditorConfig(from) => format!("from {}", from.display()),
                    crate::buffer::IndentSource::Detected => "detected from the file's own lines".into(),
                    crate::buffer::IndentSource::Default => "C#'s default".into(),
                };
                let menu = |kind: StatusMenu| {
                    cx.listener(move |w, _: &ClickEvent, _, cx| {
                        w.status_menu = if w.status_menu == Some(kind) { None } else { Some(kind) };
                        cx.notify();
                    })
                };
                let framework = read.context_label();
                div()
                    .flex()
                    .items_center()
                    .children(framework.map(|label| {
                        status_widget(label)
                            .id("target-framework")
                            .debug_selector(|| "status: target framework".into())
                            .hover(|d| d.bg(theme::button_hover()))
                            .on_click(menu(StatusMenu::TargetFramework))
                    }))
                    .child(status_widget(format!("{}:{}", caret.line + 1, caret.utf16_column + 1)))
                    .child(
                        status_widget(ending.name().into())
                            .id("line-separator")
                            .debug_selector(|| "status: line separator".into())
                            .hover(|d| d.bg(theme::button_hover()))
                            .on_click(menu(StatusMenu::LineSeparators)),
                    )
                    .child(
                        status_widget(charset.name().into())
                            .id("encoding")
                            .debug_selector(|| "status: encoding".into())
                            .hover(|d| d.bg(theme::button_hover()))
                            .on_click(menu(StatusMenu::Encodings)),
                    )
                    .child(
                        status_widget(indent.name())
                            .id("indent")
                            .tooltip(move |_, cx| cx.new(|_| Tip(format!("Indent: {indent_tip}").into())).into()),
                    )
                    .when(read_only, |d| {
                        d.child(
                            div()
                                .id("read-only")
                                .debug_selector(|| "status: read-only".into())
                                .h(px(22.))
                                .px(px(6.))
                                .flex()
                                .items_center()
                                .rounded(px(4.))
                                .hover(|d| d.bg(theme::button_hover()))
                                .child(icon("locked", 14.))
                                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                    if let Err(error) = buffer.update(cx, |b, cx| b.clear_read_only(cx)) {
                                        log::warn!("Could not clear the read-only status: {error}");
                                    }
                                })),
                        )
                    })
            }))
    }
}

/// A project's two letters on the accent, as Rider marks a project in its toolbar and recent list.
fn project_badge(name: &str, size: f32) -> impl IntoElement {
    let initials: String = name.chars().filter(|c| c.is_alphanumeric()).take(2).collect::<String>().to_uppercase();
    div()
        .size(px(size))
        .flex_none()
        .rounded(px(size / 5.))
        .bg(theme::accent())
        .text_color(theme::on_accent())
        .text_size(px(size / 2.))
        .font_weight(FontWeight::SEMIBOLD)
        .flex()
        .items_center()
        .justify_center()
        .child(initials)
}

/// A path as a person reads it, the home folder as `~`.
fn tilde(path: &Path) -> String {
    let home = rsharp_core::programs::home();
    match path.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

/// Windows' caption buttons, in its own glyphs (Segoe's), handed to Windows by their control areas.
fn caption_buttons() -> impl IntoElement {
    let button = |id: &'static str, glyph: &'static str, area: gpui::WindowControlArea, close: bool| {
        div()
            .id(id)
            .w(px(46.))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .font_family("Segoe Fluent Icons")
            .text_size(px(10.))
            .window_control_area(area)
            .when(close, |d| d.hover(|d| d.bg(theme::close_hover()).text_color(theme::on_accent())))
            .when(!close, |d| d.hover(|d| d.bg(theme::button_hover())))
            .child(glyph)
    };
    div()
        .h_full()
        .flex()
        .child(button("minimize", "\u{E921}", gpui::WindowControlArea::Min, false))
        .child(button("maximize", "\u{E922}", gpui::WindowControlArea::Max, false))
        .child(button("close", "\u{E8BB}", gpui::WindowControlArea::Close, true))
}

/// `1 error`, `2 errors`.
fn count(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

fn island() -> gpui::Div {
    div().flex().flex_col().overflow_hidden().rounded(px(ISLAND_RADIUS)).bg(theme::island())
}

/// A tool window's empty state: what it would show, and how to get it.
fn empty_text(text: &'static str) -> gpui::Div {
    div().flex_1().flex().items_center().justify_center().px(px(16.)).text_color(theme::text_secondary()).child(text)
}

/// What a gap between islands resizes when dragged.
#[derive(Clone, Copy)]
enum Resize {
    Left,
    Right,
    Bottom,
}

impl Render for Resize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// The gap between islands, which resizes them as it is dragged.
fn splitter(edge: Resize) -> impl IntoElement {
    let horizontal = matches!(edge, Resize::Bottom);
    div()
        .id(match edge {
            Resize::Left => "split-left",
            Resize::Right => "split-right",
            Resize::Bottom => "split-bottom",
        })
        .debug_selector(move || {
            format!(
                "splitter: {}",
                if horizontal {
                    "bottom"
                } else if matches!(edge, Resize::Left) {
                    "left"
                } else {
                    "right"
                }
            )
        })
        .flex_none()
        .when(horizontal, |d| d.h(px(GAP)).w_full().cursor_row_resize())
        .when(!horizontal, |d| d.w(px(GAP)).h_full().cursor_col_resize())
        .on_drag(edge, |edge, _, _, cx| cx.new(|_| *edge))
}

/// A floating window's header (moving it) or corner (resizing it), carried in a drag; the window itself follows.
#[derive(Clone, Copy)]
struct FloatDrag {
    tool: Tool,
    resize: bool,
}

impl Render for FloatDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// A tool window's stripe button, carried in a drag, drawn as a chip of its title.
/// A tooltip: a line of text on a popup, as Rider's.
pub(crate) struct Tip(pub(crate) SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(480.))
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .text_color(theme::text())
            .font_family(theme::UI_FONT)
            .text_size(px(13.))
            .child(self.0.clone())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum StatusMenu {
    LineSeparators,
    Encodings,
    /// The target frameworks a multi-targeted project compiles the file for, as Rider's switcher offers them.
    TargetFramework,
}

/// A tab on its way somewhere: its group and index, and its name for the preview under the pointer.
#[derive(Clone)]
pub struct TabDrag {
    group: usize,
    index: usize,
    name: SharedString,
}

impl Render for TabDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(10.))
            .h(px(28.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(6.))
            .bg(theme::tab_selected())
            .border_1()
            .border_color(theme::tab_selected_border())
            .text_color(theme::text())
            .font_family(theme::UI_FONT)
            .text_size(px(13.))
            .child(icon(file_icon(&self.name), 16.))
            .child(self.name.clone())
    }
}

#[derive(Clone)]
struct DraggedTool(Tool);

impl Render for DraggedTool {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(10.))
            .h(px(28.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(6.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .text_color(theme::text())
            .font_family(theme::UI_FONT)
            .text_size(px(13.))
            .child(icon(self.0.icon(), 16.))
            .child(self.0.title())
    }
}

/// A toolbar button that holds a value (the project, the branch), as Rider's toolbar widgets.
fn widget(id: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(30.))
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(8.))
        .rounded(px(6.))
        .hover(|d| d.bg(theme::button_hover()))
        .active(|d| d.bg(theme::button_pressed()))
}

fn status_widget(text: String) -> gpui::Div {
    div().h(px(22.)).flex().items_center().px(px(6.)).rounded(px(4.)).whitespace_nowrap().child(text)
}

fn empty_editor() -> impl IntoElement {
    let hints = [
        ("Open a file", "Double-click it in Solution"),
        ("Move in Solution", "↑ ↓ ← →  Enter"),
        ("Open another folder", if cfg!(target_os = "macos") { "⌘O" } else { "Ctrl+O" }),
    ];
    let column = || div().flex().flex_col().gap(px(10.));
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .text_color(theme::text_secondary())
        .child(column().items_end().children(hints.iter().map(|(what, _)| what.to_string())))
        // An instruction reads as text; only the keys a person presses take the accent, as Rider's hints do.
        .child(
            column().children(
                hints
                    .iter()
                    .enumerate()
                    .map(|(i, (_, how))| div().when(i > 0, |d| d.text_color(theme::accent())).child(how.to_string())),
            ),
        )
}

impl Focusable for MainWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root_focus.clone()
    }
}
