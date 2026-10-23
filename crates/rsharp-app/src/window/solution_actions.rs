//! The Solution view's changes, as Rider's: the row's menu and keys, the name popup, solution folders, and the one way
//! RSharp writes a solution or project file (`write_edited`), which saves it first if it is open, keeps what it held
//! in Local History, writes in its own encoding and reads back what was written.

use super::*;
use crate::solution_menu::{MenuItem, NamePopup, Named, flattened, menu_for};
use rsharp_core::solution::SolutionEntry;
use rsharp_core::solution_edits::{self, SolutionEdit};

/// The menu open on a row: where, its items, and the submenu shown beside it.
pub(super) struct SolutionMenu {
    pub row: usize,
    pub at: Point<Pixels>,
    pub items: Vec<MenuItem>,
    pub submenu: Option<usize>,
}

/// What a name typed into the popup is for.
#[derive(Clone)]
pub(super) enum NameFor {
    NewSolutionFolder { parent: Option<String> },
    RenameSolutionFolder { folder: String },
}

/// Names in a sentence, as Rider's questions list them: "a", "a and b", "a, b and c".
fn together(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn find_folder<'a>(entries: &'a [SolutionEntry], path: &str) -> Option<&'a SolutionEntry> {
    entries.iter().find_map(|e| match e {
        SolutionEntry::Folder { path: p, children, .. } => {
            if p == path {
                Some(e)
            } else {
                find_folder(children, path)
            }
        }
        _ => None,
    })
}

/// A solution folder's place in the solution, from its row.
fn solution_folder_of(row: &TreeRow) -> Option<String> {
    row.key.strip_prefix("sf:").map(str::to_string)
}

impl MainWindow {
    /// The row's menu as it reads: "Add > New Solution Folder...", a key in brackets after it.
    pub fn solution_menu(&self, row: usize) -> Vec<String> {
        self.rows.get(row).map(|r| flattened(&menu_for(r))).unwrap_or_default()
    }

    /// The menu open now, as it reads.
    pub fn shown_menu(&self) -> Option<Vec<String>> {
        self.solution_menu_open.as_ref().map(|m| flattened(&m.items))
    }

    pub fn name_popup(&self) -> Option<Entity<NamePopup>> {
        self.name_popup.as_ref().map(|(popup, _)| popup.clone())
    }

    /// The question last asked before a change, as its prompt put it.
    pub fn asked(&self) -> Option<&str> {
        self.asked.as_deref()
    }

    pub(super) fn open_row_menu(&mut self, row: usize, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(found) = self.rows.get(row) else { return };
        let items = menu_for(found);
        if items.is_empty() {
            return;
        }
        self.selected = Some(row);
        self.solution_menu_open = Some(SolutionMenu { row, at, items, submenu: None });
        cx.notify();
    }

    /// Rider's New on a row (its macOS ⌘N, Alt+Insert elsewhere): the row's Add items, at the row.
    pub(super) fn open_new_menu(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.selected else { return };
        let Some(MenuItem::Submenu(_, add)) =
            self.rows.get(row).map(menu_for).and_then(|m| m.into_iter().find(|i| i.title() == "Add"))
        else {
            return;
        };
        let at = point(px(40.), px(TOOLBAR_HEIGHT + 80. + row as f32 * ROW_HEIGHT));
        self.solution_menu_open = Some(SolutionMenu { row, at, items: add, submenu: None });
        cx.notify();
    }

    /// Runs the item of the row's menu that a key stands for, if the row's menu has it.
    pub(super) fn run_row_item(&mut self, titles: &[&str], window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.selected else { return };
        let offered = self.solution_menu(row);
        let item = titles.iter().find(|t| offered.iter().any(|o| item_title(o) == **t));
        if let Some(item) = item {
            self.choose(row, item, window, cx);
        }
    }

    /// Runs a menu item on a row, as clicking it does: by its title, its submenu before it ("Add > New File...") or not.
    pub fn choose(&mut self, row: usize, item: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.solution_menu_open = None;
        let Some(found) = self.rows.get(row).cloned() else { return };
        self.selected = Some(row);
        let title = item_title(item);
        match (found.kind, title) {
            (NodeKind::Solution | NodeKind::SolutionFolder, "New Solution Folder...") => {
                let parent = solution_folder_of(&found);
                self.ask_name("New Solution Folder", "", Vec::new(), NameFor::NewSolutionFolder { parent }, window, cx);
            }
            (NodeKind::SolutionFolder, "Rename...") => {
                let Some(folder) = solution_folder_of(&found) else { return };
                self.ask_name(
                    "Rename Solution Folder",
                    &found.name,
                    Vec::new(),
                    NameFor::RenameSolutionFolder { folder },
                    window,
                    cx,
                );
            }
            (NodeKind::SolutionFolder, "Remove...") => self.remove_solution_folder(&found, window, cx),
            (_, "Copy Path") => {
                if let Some(path) = &found.path {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(path.display().to_string()));
                }
            }
            (_, title) => self.said = Some(format!("{title} is not in RSharp yet").into()),
        }
        cx.notify();
    }

    pub(super) fn ask_name(
        &mut self,
        title: &str,
        name: &str,
        kinds: Vec<&'static str>,
        purpose: NameFor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let popup = cx.new(|cx| NamePopup::new(title.to_string(), name, kinds, cx));
        cx.subscribe_in(&popup, window, move |w, popup, named: &Named, window, cx| match named {
            Named::Chosen { name, kind } => {
                let Some((_, purpose)) = w.name_popup.clone() else { return };
                match w.named(purpose, name.clone(), *kind, window, cx) {
                    Ok(()) => {
                        w.name_popup = None;
                        window.focus(&w.explorer_focus);
                    }
                    Err(why) => popup.update(cx, |p, cx| p.refuse(why, cx)),
                }
                cx.notify();
            }
            Named::Dismissed => {
                w.name_popup = None;
                window.focus(&w.explorer_focus);
                cx.notify();
            }
        })
        .detach();
        window.focus(&popup.focus_handle(cx));
        self.name_popup = Some((popup, purpose));
        cx.notify();
    }

    /// What a name chosen in the popup does; why not, when it cannot, keeps the popup open saying so.
    fn named(
        &mut self,
        purpose: NameFor,
        name: String,
        _kind: Option<&'static str>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let name = name.trim().to_string();
        let solution = self.tree.as_ref().map(|t| t.solution().path.clone()).ok_or("There is no solution")?;
        match purpose {
            NameFor::NewSolutionFolder { parent } => {
                let edit = SolutionEdit::AddFolder { parent: parent.as_deref(), name: &name };
                self.write_edited(&solution, |text| solution_edits::edit(&solution, text, edit), cx)?;
                let path = format!("{}{name}/", parent.as_deref().unwrap_or("/"));
                self.reveal_solution_folder(&path, cx);
                self.said = Some(format!("Added solution folder {name}").into());
            }
            NameFor::RenameSolutionFolder { folder } => {
                let renamed =
                    format!("{}{name}/", &folder[..folder.trim_end_matches('/').rfind('/').map_or(0, |i| i + 1)]);
                let was_open = self.tree.as_ref().is_some_and(|t| t.is_open(&format!("sf:{folder}")));
                let edit = SolutionEdit::RenameFolder { folder: &folder, name: &name };
                self.write_edited(&solution, |text| solution_edits::edit(&solution, text, edit), cx)?;
                if was_open && let Some(tree) = &mut self.tree {
                    tree.reveal(&[format!("sf:{renamed}")]);
                }
                self.reveal_solution_folder(&renamed, cx);
                self.said = Some(format!("Renamed solution folder to {name}").into());
            }
        }
        Ok(())
    }

    /// Opens the solution folders down to one and selects it.
    fn reveal_solution_folder(&mut self, path: &str, cx: &mut Context<Self>) {
        let mut keys = Vec::new();
        let mut above = String::from("/");
        for part in path.split('/').filter(|p| !p.is_empty()) {
            if above != "/" {
                keys.push(format!("sf:{above}"));
            }
            above = format!("{above}{part}/");
        }
        if let Some(tree) = &mut self.tree {
            tree.reveal(&keys);
        }
        self.rebuild_rows();
        self.select_key(&format!("sf:{path}"));
        self.save_state(cx);
    }

    pub(super) fn select_key(&mut self, key: &str) {
        if let Some(at) = self.rows.iter().position(|r| r.key == key) {
            self.selected = Some(at);
        }
    }

    /// Rider's Remove on a solution folder: once the person agrees to what goes with it (its projects, the folders and
    /// files in it), all of it leaves the solution; every file stays on disk.
    fn remove_solution_folder(&mut self, row: &TreeRow, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(folder), Some(tree)) = (solution_folder_of(row), &self.tree) else { return };
        let solution_name = tree.solution().name.clone();
        let Some(SolutionEntry::Folder { name, children, .. }) = find_folder(&tree.solution().entries, &folder) else {
            return;
        };
        let (mut projects, mut folders, mut files) = (Vec::new(), Vec::new(), Vec::new());
        fn under(
            entries: &[SolutionEntry],
            projects: &mut Vec<String>,
            folders: &mut Vec<String>,
            files: &mut Vec<String>,
        ) {
            for entry in entries {
                match entry {
                    SolutionEntry::Project(p) => projects.push(p.name.clone()),
                    SolutionEntry::Folder { name, children, .. } => {
                        folders.push(name.clone());
                        under(children, projects, folders, files);
                    }
                    SolutionEntry::Item { name, .. } => files.push(name.clone()),
                }
            }
        }
        under(children, &mut projects, &mut folders, &mut files);
        let mut going = Vec::new();
        for (mut names, kind) in [(projects, "project"), (folders, "solution folder"), (files, "file")] {
            if !names.is_empty() {
                names.sort_by_key(|n| n.to_lowercase());
                let plural = if names.len() == 1 { "" } else { "s" };
                going.push(format!("the {kind}{plural} {}", together(&names)));
            }
        }
        let question = if going.is_empty() {
            format!("Remove the empty solution folder {name} from {solution_name}?")
        } else {
            format!(
                "Remove the solution folder {name} from {solution_name}, and with it {}? Their files stay on disk.",
                together(&going)
            )
        };
        self.asked = Some(question.clone());
        let name = name.clone();
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            "Remove Solution Folder",
            Some(&question),
            &["Remove", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |w, cx| {
                let Some(solution) = w.tree.as_ref().map(|t| t.solution().path.clone()) else { return };
                let edit = SolutionEdit::RemoveFolder { folder: &folder };
                w.said = Some(
                    match w.write_edited(&solution, |text| solution_edits::edit(&solution, text, edit), cx) {
                        Ok(()) => format!("Removed solution folder {name} from {solution_name}"),
                        Err(why) => why,
                    }
                    .into(),
                );
                cx.notify();
            });
        })
        .detach();
    }

    /// Writes a solution or project file through `change`: an open one saved first, so its edits are kept; the bytes
    /// it replaces kept in Local History; the text written back in its own encoding; then the open tab, the tree and
    /// Roslyn read what was written.
    pub(super) fn write_edited(
        &mut self,
        path: &Path,
        change: impl FnOnce(&str) -> Result<String, String>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let path = rsharp_core::paths::canonical(path);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(buffer) = crate::buffer::find(&path, cx).filter(|b| b.read(cx).is_modified()) {
            buffer.update(cx, |b, cx| b.save(cx)).map_err(|e| format!("{name} could not be saved: {e}"))?;
        }
        let before = std::fs::read(&path).map_err(|e| format!("{name} could not be read: {e}"))?;
        let (text, charset) = rsharp_core::encodings::decode(&before, None);
        let after = change(&text)?;
        let bytes = rsharp_core::encodings::encode(&after, charset)
            .map_err(|c| format!("'{c}' cannot be written in {}", charset.name()))?;
        rsharp_core::files::write(&path, &bytes).map_err(|e| format!("{name} could not be written: {e}"))?;
        crate::history_window::record(path.clone(), before, None, cx);
        crate::history_window::record(path.clone(), bytes, None, cx);
        if let Some(buffer) = crate::buffer::find(&path, cx) {
            buffer.update(cx, |b, cx| b.reload(cx));
        }
        self.follow_written(&path, cx);
        Ok(())
    }

    /// The tree, the projects and Roslyn after a solution or project file changed on disk.
    pub(super) fn follow_written(&mut self, path: &Path, cx: &mut Context<Self>) {
        let is_solution = self.tree.as_ref().is_some_and(|t| t.solution().path == path);
        if is_solution && let Some(solution) = rsharp_core::solution::read(path) {
            if let Some(tree) = &mut self.tree {
                tree.set_solution(solution);
            }
            self.rebuild_rows();
        }
        let extension = path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default();
        if extension.ends_with("proj") || extension == "props" || extension == "targets" || is_solution {
            if let Some(server) = &self.server {
                rsharp_lsp::reload_projects_of(server, &[path.to_path_buf()]);
            }
            self.evaluate_projects(cx);
        }
        cx.notify();
    }

    /// The open menu, drawn at where it opened, its submenu beside it.
    pub(super) fn solution_menu_view(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let menu = self.solution_menu_open.as_ref()?;
        let row = menu.row;
        let panel = |id: &'static str| {
            div()
                .id(id)
                .w(px(240.))
                .py(px(6.))
                .rounded(px(8.))
                .bg(theme::popup())
                .border_1()
                .border_color(theme::popup_border())
                .shadow_lg()
        };
        let line = |title: &'static str, keys: Option<String>, index: usize, sub: bool| {
            div()
                .id(("solution-menu", index))
                .debug_selector(move || format!("solution menu: {title}"))
                .h(px(24.))
                .mx(px(6.))
                .px(px(8.))
                .rounded(px(4.))
                .flex()
                .items_center()
                .justify_between()
                .hover(|d| d.bg(theme::selection()))
                .child(title)
                .children(keys.map(|k| div().text_color(theme::text_secondary()).child(k)))
                .when(sub, |d| d.child(icon("chevron-right", 16.)))
        };
        let top: Vec<_> = menu
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| match *item {
                MenuItem::Action(title, ref keys) => line(title, keys.clone(), i, false)
                    .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| w.choose(row, title, window, cx)))
                    .into_any_element(),
                MenuItem::Submenu(title, _) => line(title, None, i, true)
                    .on_hover(cx.listener(move |w, hovered: &bool, _, cx| {
                        if *hovered && let Some(menu) = &mut w.solution_menu_open {
                            menu.submenu = Some(i);
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |w, _: &ClickEvent, _, cx| {
                        if let Some(menu) = &mut w.solution_menu_open {
                            menu.submenu = Some(i);
                            cx.notify();
                        }
                    }))
                    .into_any_element(),
            })
            .collect();
        let sub = menu.submenu.and_then(|i| match &menu.items[i] {
            MenuItem::Submenu(_, inner) => Some((i, inner.clone())),
            MenuItem::Action(..) => None,
        });
        let sub = sub.map(|(at, inner)| {
            let rows: Vec<_> = inner
                .into_iter()
                .enumerate()
                .map(|(j, item)| {
                    let title = item.title();
                    let keys = match &item {
                        MenuItem::Action(_, keys) => keys.clone(),
                        MenuItem::Submenu(..) => None,
                    };
                    line(title, keys, 100 + j, false)
                        .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| w.choose(row, title, window, cx)))
                        .into_any_element()
                })
                .collect();
            div().absolute().left(px(236.)).top(px(at as f32 * 24.)).child(panel("solution-submenu").children(rows))
        });
        Some(
            div()
                .absolute()
                .top(menu.at.y)
                .left(menu.at.x)
                .on_mouse_down_out(cx.listener(|w, _, _, cx| {
                    w.solution_menu_open = None;
                    cx.notify();
                }))
                .child(panel("solution-menu").children(top))
                .children(sub)
                .into_any_element(),
        )
    }
}

/// A menu item's title from how it reads: without its submenu before it or its keys after it.
fn item_title(item: &str) -> &str {
    let title = item.rsplit(" > ").next().unwrap_or(item);
    match title.rfind(" (") {
        Some(at) if title.ends_with(')') => &title[..at],
        _ => title,
    }
}
