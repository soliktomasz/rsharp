//! The main window, laid out as Rider's Islands: the toolbar, a stripe and the status bar on the frame, and the
//! Solution view and the editor as islands over it. One folder or solution per window.

use crate::actions::ALL;
use crate::assets::{file_icon, icon};
use crate::dock::{DockLayout, Place, Rect, Tool};
use crate::editor::{self, CodeEditor};
use crate::find_action::{Ended, FindActionPopup};
use crate::state::{FolderState, KeptTab, RecentSolutions, store};
use crate::theme;
use crate::workspace::{Entry, Workspace};
use futures::StreamExt;
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, MouseButton, PathPromptOptions, Pixels,
    Point, SharedString, Task, Window, actions, div, prelude::*, px, uniform_list,
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
        ToggleSolution,
        ToggleProblems,
        HideToolWindow,
        FloatToolWindow,
        RestoreDefaultLayout,
        ExplorerUp,
        ExplorerDown,
        ExplorerExpand,
        ExplorerCollapse,
        ExplorerOpen
    ]
);

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

struct Row {
    entry: Entry,
    depth: usize,
    expanded: bool,
}

pub struct MainWindow {
    workspace: Option<Workspace>,
    branch: Option<String>,
    expanded: HashSet<PathBuf>,
    rows: Vec<Row>,
    selected: Option<usize>,
    dock: DockLayout,
    /// Where a floating window's header was taken, from its top left, while it is dragged.
    float_grab: Point<Pixels>,
    opened_as: Option<PathBuf>,
    explorer_focus: FocusHandle,
    problems_focus: FocusHandle,
    /// The window's own focus, where keys go when no view has them (the Welcome screen), so window-wide keys
    /// always reach the window.
    root_focus: FocusHandle,
    tabs: Vec<Entity<CodeEditor>>,
    active: Option<usize>,
    server: Option<Arc<LanguageServer>>,
    server_state: ServerState,
    /// Roslyn's work under way, by its progress token, with its title.
    progress: Vec<(String, String)>,
    projects_loaded: bool,
    said: Option<SharedString>,
    find_action: Option<Entity<FindActionPopup>>,
    /// The main menu's dropdown while open (Windows and Linux, which have no menu bar): each action's
    /// availability and keys where the person was, and what had the keys.
    main_menu: Option<(Vec<bool>, Vec<Option<SharedString>>, Option<FocusHandle>)>,
    _server_task: Option<Task<()>>,
}

impl MainWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let root_focus = cx.focus_handle();
        window.focus(&root_focus);
        Self {
            root_focus,
            workspace: None,
            branch: None,
            expanded: HashSet::new(),
            rows: Vec::new(),
            selected: None,
            dock: store(cx).read::<DockLayout>().complete().clamped(),
            float_grab: Point::default(),
            opened_as: None,
            explorer_focus: cx.focus_handle(),
            problems_focus: cx.focus_handle(),
            tabs: Vec::new(),
            active: None,
            server: None,
            server_state: ServerState::None,
            progress: Vec::new(),
            projects_loaded: false,
            said: None,
            find_action: None,
            main_menu: None,
            _server_task: None,
        }
    }

    pub fn open_workspace(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.save_state(cx);
        for tab in self.tabs.drain(..) {
            tab.update(cx, |e, _| e.detach_server());
        }
        self.active = None;
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
        self.branch = current_branch(&folder);
        self.expanded = std::iter::once(folder.clone()).chain(kept.expanded.iter().map(|p| folder.join(p))).collect();
        self.dock = kept.dock.clone().unwrap_or_else(|| store.read::<DockLayout>()).complete().clamped();
        self.workspace = Some(workspace);
        self.selected = Some(0);
        self.rebuild_rows();
        self.start_server(cx);
        window.focus(&self.explorer_focus);
        for tab in &kept.tabs {
            let file = folder.join(&tab.path);
            if file.is_file() {
                self.open_file(&file, Some((tab.line, tab.column)), window, cx);
            }
        }
        if let Some(active) = kept.active.filter(|a| *a < self.tabs.len()) {
            self.show_tab(active, window, cx);
        }
        cx.notify();
    }

    /// Keeps the folder's tabs, carets and Solution view for the next time it opens.
    fn save_state(&self, cx: &App) {
        let Some(workspace) = &self.workspace else { return };
        let folder = &workspace.folder;
        let relative = |p: &Path| p.strip_prefix(folder).ok().map(Path::to_path_buf);
        let tabs = self
            .tabs
            .iter()
            .filter_map(|tab| {
                let editor = tab.read(cx);
                let caret = editor.caret();
                Some(KeptTab { path: relative(&editor.path)?, line: caret.line, column: caret.utf16_column })
            })
            .collect();
        let mut expanded: Vec<PathBuf> =
            self.expanded.iter().filter_map(|p| relative(p)).filter(|p| !p.as_os_str().is_empty()).collect();
        expanded.sort();
        let store = store(cx);
        store.write_for(folder, &FolderState { tabs, active: self.active, expanded, dock: Some(self.dock.clone()) });
        store.write(&self.dock);
    }

    fn show_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = Some(index);
        window.focus(&self.tabs[index].focus_handle(cx));
        self.save_state(cx);
        cx.notify();
    }

    fn start_server(&mut self, cx: &mut Context<Self>) {
        self.server = None;
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
        let (sender, mut events) = futures::channel::mpsc::unbounded();
        self._server_task = Some(cx.spawn(async move |this, cx| {
            let started = cx
                .background_executor()
                .spawn(async move {
                    start_roslyn(&folder, solution.as_deref(), &logs, move |event| {
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
                w.server = Some(server.clone());
                w.server_state = ServerState::Loading("Loading solution".into());
                for tab in &w.tabs {
                    if is_csharp(&tab.read(cx).path) {
                        tab.update(cx, |e, cx| e.attach_server(server.clone(), cx));
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
                "workspace/projectInitializationComplete" => {
                    log::info!(target: "roslyn", "projects loaded");
                    self.projects_loaded = true;
                    if self.progress.is_empty() {
                        self.server_state = ServerState::Ready;
                    }
                    for tab in &self.tabs {
                        tab.update(cx, |e, cx| e.check(std::time::Duration::ZERO, cx));
                    }
                }
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

    fn rebuild_rows(&mut self) {
        let Some(workspace) = &mut self.workspace else { return };
        let root = workspace.folder.clone();
        let mut rows = vec![Row {
            entry: Entry { path: root.clone(), name: workspace.name(), is_folder: true },
            depth: 0,
            expanded: self.expanded.contains(&root),
        }];
        fn walk(
            workspace: &mut Workspace,
            expanded: &HashSet<PathBuf>,
            folder: &Path,
            depth: usize,
            rows: &mut Vec<Row>,
        ) {
            for entry in workspace.entries(folder).to_vec() {
                let open = entry.is_folder && expanded.contains(&entry.path);
                let path = entry.path.clone();
                rows.push(Row { entry, depth, expanded: open });
                if open {
                    walk(workspace, expanded, &path, depth + 1, rows);
                }
            }
        }
        if rows[0].expanded {
            walk(workspace, &self.expanded, &root, 1, &mut rows);
        }
        self.rows = rows;
        if let Some(selected) = self.selected {
            self.selected = Some(selected.min(self.rows.len().saturating_sub(1)));
        }
    }

    fn toggle(&mut self, index: usize, open: Option<bool>, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index) else { return };
        if !row.entry.is_folder {
            return;
        }
        let path = row.entry.path.clone();
        let opening = open.unwrap_or(!row.expanded);
        if opening {
            if let Some(workspace) = &mut self.workspace {
                workspace.forget(&path);
            }
            self.expanded.insert(path);
        } else {
            self.expanded.remove(&path);
        }
        self.rebuild_rows();
        self.save_state(cx);
        cx.notify();
    }

    fn activate_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index) else { return };
        if row.entry.is_folder {
            self.toggle(index, None, cx);
        } else {
            let path = row.entry.path.clone();
            self.open_file(&path, None, window, cx);
        }
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
        let index = match self.tabs.iter().position(|t| t.read(cx).path == path) {
            Some(index) => index,
            None => {
                let bytes = match std::fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        log::warn!("Could not open {}: {error}", path.display());
                        self.said = Some(format!("Could not open {}: {error}", path.display()).into());
                        cx.notify();
                        return;
                    }
                };
                let editor = cx.new(|cx| CodeEditor::new(path.clone(), &bytes, cx));
                cx.observe(&editor, |w, _, cx| {
                    w.save_state(cx);
                    cx.notify()
                })
                .detach();
                if let Some(server) = self.server.clone().filter(|_| is_csharp(&path)) {
                    editor.update(cx, |e, cx| e.attach_server(server, cx));
                }
                self.tabs.push(editor);
                self.tabs.len() - 1
            }
        };
        if let Some((line, column)) = caret {
            self.tabs[index].update(cx, |e, cx| e.place_caret(line, column, cx));
        }
        self.show_tab(index, window, cx);
    }

    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(index);
        tab.update(cx, |e, _| e.detach_server());
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(active) if active > index || active == self.tabs.len() => Some(active.saturating_sub(1)),
            other => other,
        };
        match self.active {
            Some(active) => window.focus(&self.tabs[active].focus_handle(cx)),
            None if self.dock.is_open(Tool::Solution) => window.focus(&self.explorer_focus),
            None => window.focus(&self.root_focus),
        }
        self.save_state(cx);
        cx.notify();
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
        self.tabs
            .iter()
            .map(|t| {
                let editor = t.read(cx);
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
        self.save_state(cx);
        for tab in self.tabs.drain(..) {
            tab.update(cx, |e, _| e.detach_server());
        }
        self.active = None;
        self.workspace = None;
        self.opened_as = None;
        self.rows.clear();
        self.server = None;
        self.server_state = ServerState::None;
        self._server_task = None;
        self.branch = None;
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
                    w.said = Some(format!("{} is not available here", ALL[*index].title.trim_end_matches('…')).into());
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
                .w(px(300.))
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
            self.said = Some(format!("{} is not available here", ALL[index].title.trim_end_matches('…')).into());
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

    fn active_editor(&self) -> Option<&Entity<CodeEditor>> {
        self.active.and_then(|i| self.tabs.get(i))
    }
}

fn is_csharp(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "cs" || e == "csx")
}

/// The branch `.git/HEAD` names, or the commit's short hash when detached.
fn current_branch(folder: &Path) -> Option<String> {
    let head = folder.ancestors().map(|f| f.join(".git/HEAD")).find(|h| h.is_file())?;
    let head = std::fs::read_to_string(head).ok()?;
    let head = head.trim();
    Some(head.strip_prefix("ref: refs/heads/").map(str::to_string).unwrap_or_else(|| head.chars().take(8).collect()))
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
            .on_action(cx.listener(Self::choose_folder))
            .on_action(cx.listener(|w, _: &FindAction, window, cx| w.find_action(window, cx)))
            // Each acts only while it has something to act on, so menus and Find Action grey it otherwise.
            .when(self.workspace.is_some(), |d| {
                d.on_action(cx.listener(|w, _: &CloseSolution, window, cx| w.close_solution(window, cx)))
                    .on_action(
                        cx.listener(|w, _: &ToggleSolution, window, cx| w.toggle_tool(Tool::Solution, window, cx)),
                    )
                    .on_action(
                        cx.listener(|w, _: &ToggleProblems, window, cx| w.toggle_tool(Tool::Problems, window, cx)),
                    )
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
                                (r.w, r.h) = (f32::from(at.x) - r.x, f32::from(at.y) - r.y);
                            } else {
                                (r.x, r.y) = (f32::from(at.x - grab.x), f32::from(at.y - grab.y));
                            }
                        }
                        w.dock = w.dock.clone().clamped();
                        w.save_state(cx);
                        cx.notify();
                    }))
            })
            .when(self.active.is_some(), |d| {
                d.on_action(cx.listener(|w, _: &CloseTab, window, cx| {
                    if let Some(active) = w.active {
                        w.close_tab(active, window, cx)
                    }
                }))
                .on_action(cx.listener(|w, _: &editor::Save, _, cx| {
                    if let Some(editor) = w.active_editor().cloned() {
                        w.said =
                            editor.update(cx, |e, cx| e.save(cx)).err().map(|e| format!("Could not save: {e}").into());
                        cx.notify();
                    }
                }))
            })
            .child(self.toolbar(window, cx))
            .child(match self.workspace {
                Some(_) => self.body(window, cx).into_any_element(),
                None => div().flex().flex_1().min_h_0().px(px(GAP)).child(self.welcome(cx)).into_any_element(),
            })
            .child(self.status_bar(cx))
            .children(floating)
            .children(self.main_menu.as_ref().map(|(available, keys, _)| self.main_menu_dropdown(available, keys, cx)))
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
                    .child(project_badge(&name, 20.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
            }))
            .children(
                self.branch.clone().map(|branch| widget("branch-widget").child(icon("branch", 16.)).child(branch)),
            )
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
            div().h(px(dock.bottom_height)).flex_none().flex().gap(px(GAP)).children(
                bottom
                    .into_iter()
                    .map(|t| self.tool_island(t, window, cx).flex_1().min_w_0().mb(px(GAP)).into_any_element())
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
        let this = cx.entity().downgrade();
        let header = div()
            .id(("tool-header", tool as usize))
            .when(self.dock.is_floating(tool), |d| {
                d.debug_selector(move || format!("floating: {}", tool.title())).on_drag(
                    FloatDrag { tool, resize: false },
                    move |dragged, grab, _, cx| {
                        let _ = this.update(cx, |w, _| w.float_grab = grab);
                        cx.new(|_| *dragged)
                    },
                )
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
        };
        island().child(header).child(content)
    }

    fn tool_subtitle(&self, tool: Tool, cx: &App) -> Option<String> {
        match tool {
            Tool::Solution => None,
            Tool::Problems => self
                .active_editor()
                .map(|e| e.read(cx).path.file_name().unwrap_or_default().to_string_lossy().into_owned()),
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
                            .on_drag(FloatDrag { tool, resize: true }, |dragged, _, _, cx| cx.new(|_| *dragged)),
                    )
                    .into_any_element()
            })
            .collect()
    }

    fn focused_tool(&self, window: &Window, cx: &App) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| self.tool_focus(*t).contains_focused(window, cx))
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
        window.focus(self.tool_focus(tool));
        self.save_state(cx);
        cx.notify();
    }

    /// Floats a window where its button was dropped, its header under the pointer.
    fn float_tool(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        let at = window.mouse_position();
        let (x, y) = (f32::from(at.x) - 40., f32::from(at.y) - 20.);
        self.dock.float(tool, Rect { x, y, w: 420., h: 360. });
        self.dock = self.dock.clone().clamped();
        window.focus(self.tool_focus(tool));
        self.save_state(cx);
        cx.notify();
    }

    fn tool_focus(&self, tool: Tool) -> &FocusHandle {
        match tool {
            Tool::Solution => &self.explorer_focus,
            Tool::Problems => &self.problems_focus,
        }
    }

    /// Opens or closes a tool window; one opened takes the keys, one closed with them gives them to the editor.
    pub fn toggle_tool(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        let had_focus = self.tool_focus(tool).contains_focused(window, cx);
        self.dock.toggle(tool);
        if self.dock.is_open(tool) {
            window.focus(self.tool_focus(tool));
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

    /// The shown file's problems, as Problems' File tab: each a row that goes to its place.
    fn problems_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(editor) = self.active_editor().cloned() else {
            return empty_text("Open a file to see its problems.").track_focus(&self.problems_focus).into_any_element();
        };
        let read = editor.read(cx);
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
        let chevron = row.entry.is_folder.then(|| if row.expanded { "chevron-down" } else { "chevron-right" });
        let kind = if index == 0 {
            "project"
        } else if row.entry.is_folder {
            "folder"
        } else {
            file_icon(&row.entry.name)
        };
        div()
            .id(("row", index))
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
                    .child(row.entry.name.clone()),
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
        island()
            .min_w_0()
            .when(!self.tabs.is_empty(), |d| {
                d.child(
                    div()
                        .id("tabs")
                        .h(px(40.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .px(px(6.))
                        .border_b_1()
                        .border_color(theme::frame())
                        .overflow_x_scroll()
                        .children({
                            let tabs: Vec<_> = self.tabs.clone();
                            let mut out = Vec::with_capacity(tabs.len());
                            for (i, tab) in tabs.iter().enumerate() {
                                out.push(self.tab(i, tab, cx));
                            }
                            out
                        }),
                )
            })
            .child(match self.active_editor() {
                Some(editor) => div().flex_1().min_h_0().child(editor.clone()).into_any_element(),
                None => empty_editor().into_any_element(),
            })
    }

    fn tab(&self, index: usize, tab: &Entity<CodeEditor>, cx: &mut Context<Self>) -> gpui::AnyElement {
        let editor = tab.read(cx);
        let name = editor.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let selected = self.active == Some(index);
        let modified = editor.document.is_modified();
        let group = SharedString::from(format!("tab-{index}"));
        div()
            .id(("tab", index))
            .group(group.clone())
            .h(px(28.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(8.))
            .pr(px(4.))
            .rounded(px(6.))
            .border_1()
            .when(selected, |d| d.bg(theme::tab_selected()).border_color(theme::tab_selected_border()))
            .when(!selected, |d| {
                d.border_color(gpui::transparent_black())
                    .text_color(theme::text_muted())
                    .hover(|d| d.bg(theme::button_hover()))
            })
            .child(icon(file_icon(&name), 16.))
            .child(name)
            .child(
                div()
                    .id(("close", index))
                    .size(px(16.))
                    .rounded(px(3.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|d| d.bg(theme::button_hover()))
                    .active(|d| d.bg(theme::button_pressed()))
                    .child(if modified {
                        div().size(px(7.)).rounded_full().bg(theme::modified()).into_any_element()
                    } else {
                        icon("close", 16.)
                            .invisible()
                            .group_hover(group, |s| s.visible())
                            .when(selected, |i| i.visible())
                            .into_any_element()
                    })
                    .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        w.close_tab(index, window, cx)
                    })),
            )
            .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| w.show_tab(index, window, cx)))
            .into_any_element()
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
                (e.diagnostics.iter().filter(|d| d.error).count(), e.diagnostics.iter().filter(|d| !d.error).count())
            }),
            match &self.server_state {
                ServerState::None => None,
                ServerState::Starting => Some(("Starting Roslyn…".to_string(), theme::text_muted())),
                ServerState::Loading(what) => Some((format!("{what}…"), theme::text_muted())),
                ServerState::Ready => Some(("Roslyn".to_string(), theme::text_muted())),
                ServerState::Failed(why) => Some((why.clone(), theme::error())),
            },
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
            .children(self.said.clone().map(|said| status_widget(said.to_string()).text_color(theme::error())))
            .children(problems.filter(|(e, w)| e + w > 0).map(|(errors, warnings)| {
                status_widget(format!("{}, {}", count(errors, "error"), count(warnings, "warning")))
                    .text_color(if errors > 0 { theme::error() } else { theme::warning() })
            }))
            .children(server.map(|(text, colour)| status_widget(text).text_color(colour)))
            .children(
                editor
                    .map(|e| {
                        let caret = e.caret();
                        [
                            status_widget(format!("{}:{}", caret.line + 1, caret.utf16_column + 1)),
                            status_widget(match e.document.line_ending {
                                rsharp_text::LineEnding::Lf => "LF".into(),
                                rsharp_text::LineEnding::CrLf => "CRLF".into(),
                            }),
                            status_widget("UTF-8".into()),
                        ]
                    })
                    .into_iter()
                    .flatten(),
            )
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
