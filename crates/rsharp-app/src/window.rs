//! The main window, laid out as Rider's Islands: the toolbar, a stripe and the status bar on the frame, and the
//! Solution view and the editor as islands over it. One folder or solution per window.

use crate::assets::{file_icon, icon};
use crate::editor::{self, CodeEditor};
use crate::theme;
use crate::workspace::{Entry, Workspace};
use futures::StreamExt;
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, MouseButton, PathPromptOptions, SharedString,
    Task, Window, actions, div, prelude::*, px, uniform_list,
};
use rsharp_lsp::{LanguageServer, ServerEvent, start_roslyn};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

actions!(
    window,
    [CloseTab, OpenFolder, ToggleSolution, ExplorerUp, ExplorerDown, ExplorerExpand, ExplorerCollapse, ExplorerOpen]
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
    solution_shown: bool,
    explorer_focus: FocusHandle,
    tabs: Vec<Entity<CodeEditor>>,
    active: Option<usize>,
    server: Option<Arc<LanguageServer>>,
    server_state: ServerState,
    said: Option<SharedString>,
    _server_task: Option<Task<()>>,
}

impl MainWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            workspace: None,
            branch: None,
            expanded: HashSet::new(),
            rows: Vec::new(),
            selected: None,
            solution_shown: true,
            explorer_focus: cx.focus_handle(),
            tabs: Vec::new(),
            active: None,
            server: None,
            server_state: ServerState::None,
            said: None,
            _server_task: None,
        }
    }

    pub fn open_workspace(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        for tab in self.tabs.drain(..) {
            tab.update(cx, |e, _| e.detach_server());
        }
        self.active = None;
        let workspace = Workspace::open(path);
        window.set_window_title(&workspace.name());
        self.branch = current_branch(&workspace.folder);
        self.expanded = HashSet::from([workspace.folder.clone()]);
        self.workspace = Some(workspace);
        self.selected = Some(0);
        self.rebuild_rows();
        self.start_server(cx);
        window.focus(&self.explorer_focus);
        cx.notify();
    }

    fn start_server(&mut self, cx: &mut Context<Self>) {
        self.server = None;
        let Some(workspace) = &self.workspace else { return };
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
        let (folder, solution) = (workspace.folder.clone(), workspace.solution.clone());
        let logs = std::env::temp_dir().join("rsharp-logs");
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
                "$/progress" => match params["value"]["kind"].as_str() {
                    Some("begin") => {
                        if let Some(title) = params["value"]["title"].as_str() {
                            self.server_state = ServerState::Loading(title.to_string());
                        }
                    }
                    Some("end") if !matches!(self.server_state, ServerState::Ready) => {}
                    _ => {}
                },
                "workspace/projectInitializationComplete" => {
                    self.server_state = ServerState::Ready;
                    for tab in &self.tabs {
                        tab.update(cx, |e, cx| e.check(std::time::Duration::ZERO, cx));
                    }
                }
                "window/showMessage" if params["type"].as_u64() == Some(1) => {
                    self.said = params["message"].as_str().map(|m| m.to_string().into());
                }
                _ => {}
            },
            ServerEvent::Ended => {
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
                        self.said = Some(format!("Could not open {}: {error}", path.display()).into());
                        cx.notify();
                        return;
                    }
                };
                let editor = cx.new(|cx| CodeEditor::new(path.clone(), &bytes, cx));
                cx.observe(&editor, |_, _, cx| cx.notify()).detach();
                if let Some(server) = self.server.clone().filter(|_| is_csharp(&path)) {
                    editor.update(cx, |e, cx| e.attach_server(server, cx));
                }
                self.tabs.push(editor);
                self.tabs.len() - 1
            }
        };
        self.active = Some(index);
        let editor = self.tabs[index].clone();
        if let Some((line, column)) = caret {
            editor.update(cx, |e, cx| e.place_caret(line, column, cx));
        }
        window.focus(&editor.focus_handle(cx));
        cx.notify();
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
            None => window.focus(&self.explorer_focus),
        }
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
                let _ = this.update_in(cx, |w, window, cx| w.open_workspace(&path, window, cx));
            }
        })
        .detach();
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
            .on_action(cx.listener(Self::choose_folder))
            .on_action(cx.listener(|w, _: &CloseTab, window, cx| {
                if let Some(active) = w.active {
                    w.close_tab(active, window, cx)
                }
            }))
            .on_action(cx.listener(|w, _: &editor::Save, _, cx| {
                if let Some(editor) = w.active_editor().cloned() {
                    w.said = editor.update(cx, |e, cx| e.save(cx)).err().map(|e| format!("Could not save: {e}").into());
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|w, _: &ToggleSolution, _, cx| {
                w.solution_shown = !w.solution_shown;
                cx.notify()
            }))
            .child(self.toolbar(window, cx))
            .child(
                div().flex().flex_1().min_h_0().when(self.workspace.is_some(), |d| d.child(self.stripe(cx))).child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(GAP))
                        .pr(px(GAP))
                        .when(self.workspace.is_none(), |d| d.pl(px(GAP)))
                        .children(
                            match self.workspace {
                                Some(_) => vec![
                                    self.solution_shown.then(|| self.solution_island(window, cx).into_any_element()),
                                    Some(self.editor_island(cx).into_any_element()),
                                ],
                                None => vec![Some(self.welcome(cx).into_any_element())],
                            }
                            .into_iter()
                            .flatten(),
                        ),
                ),
            )
            .child(self.status_bar(cx))
    }
}

impl MainWindow {
    fn toolbar(&self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
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
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                if event.click_count == 2 {
                    window.titlebar_double_click();
                } else {
                    window.start_window_move();
                }
            })
            .children(name.map(|name| {
                let initials: String =
                    name.chars().filter(|c| c.is_alphanumeric()).take(2).collect::<String>().to_uppercase();
                widget("project-widget")
                    .child(
                        div()
                            .size(px(20.))
                            .rounded(px(4.))
                            .bg(theme::accent())
                            .text_color(theme::on_accent())
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(initials),
                    )
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
            }))
            .children(
                self.branch.clone().map(|branch| widget("branch-widget").child(icon("branch", 16.)).child(branch)),
            )
    }

    fn stripe(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.solution_shown && self.workspace.is_some();
        div().w(px(STRIPE_WIDTH)).flex_none().flex().flex_col().items_center().pt(px(2.)).children(
            self.workspace.as_ref().map(|_| {
                div()
                    .id("stripe-solution")
                    .size(px(32.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(shown, |d| d.bg(theme::accent()))
                    .when(!shown, |d| {
                        d.hover(|d| d.bg(theme::button_hover())).active(|d| d.bg(theme::button_pressed()))
                    })
                    .child(icon("tw-project", 20.))
                    .on_click(cx.listener(|w, _: &ClickEvent, _, cx| {
                        w.solution_shown = !w.solution_shown;
                        cx.notify()
                    }))
            }),
        )
    }

    fn solution_island(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.explorer_focus.contains_focused(window, cx);
        let row_count = self.rows.len();
        island()
            .id("solution")
            .w(px(280.))
            .flex_none()
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
            .child(header("Solution"))
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

    fn editor_island(&self, cx: &mut Context<Self>) -> impl IntoElement {
        island()
            .flex_1()
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
            .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| {
                w.active = Some(index);
                window.focus(&w.tabs[index].focus_handle(cx));
                cx.notify();
            }))
            .into_any_element()
    }

    fn welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                status_widget(format!("{errors} errors, {warnings} warnings")).text_color(if errors > 0 {
                    theme::error()
                } else {
                    theme::warning()
                })
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

fn island() -> gpui::Div {
    div().flex().flex_col().overflow_hidden().rounded(px(ISLAND_RADIUS)).bg(theme::island()).mb(px(GAP))
}

fn header(title: &str) -> impl IntoElement {
    div()
        .h(px(40.))
        .flex_none()
        .flex()
        .items_center()
        .px(px(14.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(title.to_string())
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
        self.explorer_focus.clone()
    }
}
