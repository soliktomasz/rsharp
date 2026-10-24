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
    NewFile { into: PathBuf },
    NewFolder { into: PathBuf },
    NewType { into: PathBuf },
    Rename { path: PathBuf },
    Duplicate { from: PathBuf, into: PathBuf },
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
            (NodeKind::Solution | NodeKind::SolutionFolder | NodeKind::Project, "Build" | "Rebuild" | "Clean") => {
                let kind = match title {
                    "Rebuild" => rsharp_core::build::Kind::Rebuild,
                    "Clean" => rsharp_core::build::Kind::Clean,
                    _ => rsharp_core::build::Kind::Build,
                };
                let selected = if found.kind == NodeKind::Solution { None } else { self.selected_build_targets() };
                if found.kind != NodeKind::Solution && selected.is_none() {
                    self.said = Some("Nothing to build: no projects selected".into());
                } else {
                    self.begin_build(kind, selected, cx);
                }
            }
            (NodeKind::Solution | NodeKind::SolutionFolder, "New Project...") => {
                self.show_new_project(solution_folder_of(&found), window, cx)
            }
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
            (NodeKind::Solution | NodeKind::SolutionFolder, "Existing Project...") => {
                self.pick_and_add(solution_folder_of(&found), true, window, cx)
            }
            (NodeKind::SolutionFolder, "Existing Item...") => {
                self.pick_and_add(solution_folder_of(&found), false, window, cx)
            }
            (NodeKind::Project, "Remove...") => self.remove_project(&found, window, cx),
            (NodeKind::File, "Remove...") => self.remove_item(&found, window, cx),
            (NodeKind::Project, "Reference...") => {
                if let Some(project) = found.path.clone() {
                    self.open_add_reference(&project, window, cx);
                }
            }
            (NodeKind::Dependencies, "Add Reference...") => {
                if let Some(project) = self.project_of_key(&found.key, "d:") {
                    self.open_add_reference(&project, window, cx);
                }
            }
            (NodeKind::ProjectReference, "Remove...") => self.remove_reference(&found, window, cx),
            (_, "Copy Path") => {
                if let Some(path) = &found.path {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(path.display().to_string()));
                }
            }
            (_, title) => {
                if !self.choose_file_operation(&found, title, window, cx) {
                    self.said = Some(format!("{title} is not in RSharp yet").into());
                }
            }
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
        kind: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let name = name.trim().to_string();
        let solution = self.tree.as_ref().map(|t| t.solution().path.clone()).ok_or("There is no solution")?;
        match purpose {
            NameFor::NewFile { into } => return self.make(&into, &name, None, false, window, cx),
            NameFor::NewFolder { into } => return self.make(&into, &name, None, true, window, cx),
            NameFor::NewType { into } => return self.make(&into, &name, kind, false, window, cx),
            NameFor::Rename { path } => return self.rename_path(&path, &name, window, cx),
            NameFor::Duplicate { from, into } => return self.duplicate(&from, &into, &name, cx),
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

    /// Add Existing Project or Item: the files the person picks, put in a solution folder (a project at the solution's
    /// top with none).
    fn pick_and_add(&mut self, into: Option<String>, projects: bool, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(if projects { "Add Project" } else { "Add Item" }.into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await {
                let _ = this.update(cx, |w, cx| {
                    if let Err(why) = w.add_existing(into, paths, projects, cx) {
                        w.said = Some(why.into());
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// What Add Existing Project and Item do with the files picked: each put in the folder, the tree opened to the last.
    pub fn add_existing(
        &mut self,
        into: Option<String>,
        picked: Vec<PathBuf>,
        projects: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let solution = self.tree.as_ref().map(|t| t.solution().path.clone()).ok_or("There is no solution")?;
        let picked: Vec<PathBuf> = picked.iter().map(|p| rsharp_core::paths::canonical(p)).collect();
        if projects {
            for path in &picked {
                if !path.is_file()
                    || !path
                        .extension()
                        .is_some_and(|e| ["csproj", "fsproj", "vbproj"].iter().any(|s| e.eq_ignore_ascii_case(s)))
                {
                    return Err(format!("{} is not an existing .NET project", path.display()));
                }
            }
            self.write_edited(
                &solution,
                |text| {
                    let mut text = text.to_string();
                    for path in &picked {
                        text = solution_edits::edit(
                            &solution,
                            &text,
                            SolutionEdit::AddProject { folder: into.as_deref(), project: path },
                        )?;
                    }
                    Ok(text)
                },
                cx,
            )?;
        } else {
            for path in &picked {
                let edit = match into.as_deref() {
                    Some(folder) => SolutionEdit::AddFile { folder, path },
                    None => return Err("A file goes in a solution folder".into()),
                };
                self.write_edited(&solution, |text| solution_edits::edit(&solution, text, edit), cx)?;
            }
        }
        if projects && let Some(server) = &self.server {
            rsharp_lsp::reload_projects_of(server, &picked);
        }
        if let Some(folder) = &into {
            self.reveal_solution_folder(folder, cx);
            if let Some(tree) = &mut self.tree {
                tree.reveal(&[format!("sf:{folder}")]);
            }
            self.rebuild_rows();
        }
        if let Some(last) = picked.last() {
            if let Some(at) = self.rows.iter().position(|r| r.path.as_deref() == Some(last.as_path())) {
                self.selected = Some(at);
            }
            let name = last.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.said = Some(format!("Added {name}").into());
        }
        cx.notify();
        Ok(())
    }

    /// Asks a question before a change, and runs `then` in the window once its first answer is chosen.
    pub(super) fn confirm(
        &mut self,
        title: &str,
        question: String,
        answer: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        self.asked = Some(question.clone());
        let chosen = window.prompt(gpui::PromptLevel::Warning, title, Some(&question), &[answer, "Cancel"], cx);
        cx.spawn_in(window, async move |this, cx| {
            if chosen.await == Ok(0) {
                let _ = this.update_in(cx, |w, window, cx| {
                    then(w, window, cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Rider's Remove on a project: it leaves the solution and its files stay. Moving its folder to the Trash is a second
    /// question, never asked of the open folder, a folder above it, or one holding the solution or another of its
    /// projects.
    fn remove_project(&mut self, row: &TreeRow, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(tree)) = (row.path.clone(), &self.tree) else { return };
        let solution = tree.solution().path.clone();
        let others: Vec<PathBuf> =
            tree.solution().projects().iter().filter(|p| p.path != project).map(|p| p.path.clone()).collect();
        let name = row.name.clone();
        let question = format!("Remove {name} from the solution? Its folder and files stay on disk.");
        self.confirm("Remove Project", question, "Remove", window, cx, move |w, window, cx| {
            let edit = SolutionEdit::RemoveProject { project: &project };
            if let Err(why) = w.write_edited(&solution, |text| solution_edits::edit(&solution, text, edit), cx) {
                w.said = Some(format!("{name} could not be removed: {why}").into());
                return;
            }
            w.said = Some(format!("Removed {name} from the solution").into());
            if let Some(server) = &w.server {
                rsharp_lsp::reload_projects_of(server, std::slice::from_ref(&project));
            }
            let own = project.parent().map(Path::to_path_buf).unwrap_or_default();
            let folder = w.workspace.as_ref().map(|ws| ws.folder.clone()).unwrap_or_default();
            let apart =
                !folder.starts_with(&own) && !solution.starts_with(&own) && !others.iter().any(|p| p.starts_with(&own));
            if apart {
                let what = own.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let question = format!("Also move the folder {what} and everything in it to the Trash?");
                w.confirm("Delete Project Files", question, "Move to Trash", window, cx, move |w, window, cx| {
                    w.said = Some(
                        match w.delete_to_trash(&own, window, cx) {
                            Ok(()) => format!("Moved {what} to the Trash"),
                            Err(why) => why,
                        }
                        .into(),
                    );
                });
            }
        });
    }

    /// Takes a file out of the solution folder that lists it; the file stays on disk.
    fn remove_item(&mut self, row: &TreeRow, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(path), Some(tree)) = (row.path.clone(), &self.tree) else { return };
        fn holder(entries: &[SolutionEntry], path: &Path) -> Option<String> {
            entries.iter().find_map(|e| match e {
                SolutionEntry::Folder { path: folder, children, .. } => children
                    .iter()
                    .any(|c| matches!(c, SolutionEntry::Item { path: p, .. } if p == path))
                    .then(|| folder.clone())
                    .or_else(|| holder(children, path)),
                _ => None,
            })
        }
        let Some(folder) = holder(&tree.solution().entries, &path) else { return };
        let solution = tree.solution().path.clone();
        let folder_name = folder.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_string();
        let question = format!("Remove {} from the solution folder {folder_name}? The file stays on disk.", row.name);
        let name = row.name.clone();
        self.confirm("Remove Item", question, "Remove", window, cx, move |w, _, cx| {
            let edit = SolutionEdit::RemoveFile { folder: &folder, path: &path };
            w.said = Some(
                match w.write_edited(&solution, |text| solution_edits::edit(&solution, text, edit), cx) {
                    Ok(()) => format!("Removed {name} from {folder_name}"),
                    Err(why) => why,
                }
                .into(),
            );
        });
    }

    /// Moves a file or folder to the Trash: its open files saved first, so the Trash and Local History hold what the
    /// editor showed, and their tabs closed once it is gone; kept open when the Trash refused.
    pub(super) fn delete_to_trash(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let open: Vec<_> = self.area.editors().filter(|e| e.read(cx).path.starts_with(path)).cloned().collect();
        for editor in &open {
            let buffer = editor.read(cx).buffer.clone();
            if buffer.read(cx).is_modified() {
                buffer.update(cx, |b, cx| b.save(cx)).map_err(|e| format!("{name} could not be saved: {e}"))?;
            }
        }
        if path.is_file()
            && let Ok(bytes) = std::fs::read(path)
        {
            crate::history_window::record(path.to_path_buf(), bytes, Some("Deleted"), cx);
        }
        rsharp_core::trash::move_to_trash(path).map_err(|e| format!("{name} could not be moved to the Trash: {e}"))?;
        self.close_tabs_under(path, window, cx);
        if let Some(tree) = &mut self.tree {
            tree.forget(path.parent().unwrap_or(path));
        }
        self.rebuild_rows();
        Ok(())
    }

    /// Closes every tab of a file at or under a path.
    pub(super) fn close_tabs_under(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        loop {
            let found = self.area.groups.iter().enumerate().find_map(|(g, group)| {
                group.tabs.iter().position(|t| t.editor.read(cx).path.starts_with(path)).map(|i| (g, i))
            });
            let Some((group, index)) = found else { return };
            self.close_tab(group, index, window, cx);
        }
    }

    /// The project a Dependencies or reference row belongs to, from its key ("d:{project}…", "ref:d:{project}|…").
    fn project_of_key(&self, key: &str, prefix: &str) -> Option<PathBuf> {
        let base = key.strip_prefix(prefix)?.split('|').next()?;
        let root = self.tree.as_ref()?.solution().path.parent()?.to_path_buf();
        Some(rsharp_core::paths::canonical(&root.join(base)))
    }

    /// The projects a project file references, as it says now.
    fn references_of(project: &Path) -> Vec<PathBuf> {
        let Ok(bytes) = std::fs::read(project) else { return Vec::new() };
        let (text, _) = rsharp_core::encodings::decode(&bytes, None);
        rsharp_core::project_edits::references(project, &text)
    }

    pub fn reference_dialog(&self) -> Option<Entity<crate::reference_dialog::ReferenceDialog>> {
        self.reference_dialog.as_ref().map(|(dialog, _)| dialog.clone())
    }

    /// Rider's Add Reference: the solution's other projects, those referenced ticked, those that reference this one
    /// (directly or through others) refused, as a reference to them would make a cycle.
    fn open_add_reference(&mut self, project: &Path, window: &mut Window, cx: &mut Context<Self>) {
        use crate::reference_dialog::{Choice, Ended, ReferenceDialog};
        let Some(tree) = &self.tree else { return };
        let projects: Vec<(String, PathBuf)> =
            tree.solution().projects().iter().filter(|p| p.exists).map(|p| (p.name.clone(), p.path.clone())).collect();
        let references: Vec<(PathBuf, Vec<PathBuf>)> =
            projects.iter().map(|(_, p)| (p.clone(), Self::references_of(p))).collect();
        // Every project that reaches this one through its references.
        let mut dependents: Vec<PathBuf> = vec![project.to_path_buf()];
        let mut more = true;
        while more {
            more = false;
            for (p, refs) in &references {
                if !dependents.contains(p) && refs.iter().any(|r| dependents.contains(r)) {
                    dependents.push(p.clone());
                    more = true;
                }
            }
        }
        let current = Self::references_of(project);
        let choices = projects
            .into_iter()
            .filter(|(_, p)| p != project)
            .map(|(name, path)| Choice {
                chosen: current.contains(&path),
                refused: dependents.contains(&path).then(|| "references this project".to_string()),
                name,
                path,
            })
            .collect();
        let name = project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let dialog = cx.new(|cx| ReferenceDialog::new(name, choices, cx));
        let project = project.to_path_buf();
        cx.subscribe_in(&dialog, window, move |w, _, ended: &Ended, window, cx| {
            if let Ended::Chosen(wanted) = ended
                && let Err(why) = w.set_references(&project, wanted.clone(), cx)
            {
                w.said = Some(why.into());
            }
            w.reference_dialog = None;
            window.focus(&w.explorer_focus);
            cx.notify();
        })
        .detach();
        window.focus(&dialog.focus_handle(cx));
        self.reference_dialog = Some((dialog, PathBuf::new()));
        cx.notify();
    }

    /// What OK does with the boxes ticked: references added and taken out in one write, the project read again.
    pub fn set_references(
        &mut self,
        project: &Path,
        wanted: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let project = rsharp_core::paths::canonical(project);
        let wanted: Vec<PathBuf> = wanted.iter().map(|p| rsharp_core::paths::canonical(p)).collect();
        let current = Self::references_of(&project);
        let (added, removed): (Vec<&PathBuf>, Vec<&PathBuf>) = (
            wanted.iter().filter(|p| !current.contains(p)).collect(),
            current.iter().filter(|p| !wanted.contains(p)).collect(),
        );
        if added.is_empty() && removed.is_empty() {
            return Ok(());
        }
        let named = |p: &PathBuf| p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let name = named(&project);
        self.write_edited(
            &project,
            |text| {
                let mut text = text.to_string();
                for reference in &added {
                    text = rsharp_core::project_edits::add_reference(&project, &text, reference)?;
                }
                for reference in &removed {
                    text = rsharp_core::project_edits::remove_reference(&project, &text, reference)?;
                }
                Ok(text)
            },
            cx,
        )?;
        self.said = Some(
            match (added.as_slice(), removed.as_slice()) {
                ([one], []) => format!("Added a reference to {} to {name}", named(one)),
                ([], [one]) => format!("Removed the reference to {} from {name}", named(one)),
                _ => format!("Changed the references of {name}"),
            }
            .into(),
        );
        Ok(())
    }

    /// Remove on a project reference row, once asked.
    fn remove_reference(&mut self, row: &TreeRow, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(referenced), Some(project)) = (row.path.clone(), self.project_of_key(&row.key, "ref:d:")) else {
            return;
        };
        let name = project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let question = format!("Remove the reference to {} from {name}?", row.name);
        self.confirm("Remove Reference", question, "Remove", window, cx, move |w, _, cx| {
            let wanted: Vec<PathBuf> = Self::references_of(&project).into_iter().filter(|p| *p != referenced).collect();
            if let Err(why) = w.set_references(&project, wanted, cx) {
                w.said = Some(why.into());
            }
        });
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
        crate::history_window::record_all(vec![(path.clone(), before, None), (path.clone(), bytes, None)], cx);
        if let Some(buffer) = crate::buffer::find(&path, cx) {
            buffer.update(cx, |b, cx| b.reload(cx));
        }
        self.follow_written(&path, cx);
        Ok(())
    }

    /// Watches the open folder for what other programs change in it.
    pub(super) fn start_watching(&mut self, folder: &Path, cx: &mut Context<Self>) {
        let (sender, mut bursts) = futures::channel::mpsc::unbounded();
        // shortcut: on Linux, notify's inotify adds a watch per folder, bin, obj and node_modules too, and a folder
        // past max_user_watches is not watched at all; watch each counted folder alone, as Passenger does, once
        // Linux users open large folders.
        let watching = {
            let folder = folder.to_path_buf();
            // Started off the UI thread: inotify walks the whole folder before it answers.
            cx.background_executor().spawn(async move {
                rsharp_core::watcher::watch(&folder, move |paths| {
                    let _ = sender.unbounded_send(paths);
                })
                .map_err(|e| log::warn!("{} is not watched: {e}", folder.display()))
                .ok()
            })
        };
        self._watch_task = Some(cx.spawn(async move |this, cx| {
            let watcher = watching.await;
            if this.update(cx, |w, _| w._watcher = watcher).is_err() {
                return;
            }
            while let Some(paths) = bursts.next().await {
                if this.update(cx, |w, cx| w.folder_changed(paths, cx)).is_err() {
                    return;
                }
            }
        }));
    }

    /// What another program changed in the folder: the tree reads those places again, an open file its text (one with
    /// unsaved edits kept as it is), a file not open is kept in Local History as an External change, the solution is
    /// read again when it changed, and the projects whose inputs changed are evaluated again.
    fn folder_changed(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.package_sources_changed(&paths, cx);
        let solution = self.tree.as_ref().map(|t| t.solution().path.clone());
        let mut closed = Vec::new();
        for path in &paths {
            if let Some(tree) = &mut self.tree {
                tree.forget(path.parent().unwrap_or(path));
                tree.forget(path);
            }
            // Git's own files only say the folder moved: they are read again, never kept in Local History.
            let git = path.components().any(|c| c.as_os_str() == ".git");
            match crate::buffer::find(path, cx) {
                Some(open) => open.update(cx, |b, cx| b.refresh_from_disk(cx)),
                None if !git => closed.push(path.clone()),
                None => {}
            }
        }
        if solution.as_ref().is_some_and(|s| paths.contains(s))
            && let Some(read) = solution.as_deref().and_then(rsharp_core::solution::read)
            && let Some(tree) = &mut self.tree
        {
            tree.set_solution(read);
            self.sync_nuget(cx);
        }
        // Read off the UI thread: a pull can change a thousand files at once.
        let reads = cx.background_executor().spawn(async move {
            closed
                .into_iter()
                .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() < 4 << 20))
                .filter_map(|p| std::fs::read(&p).ok().map(|bytes| (p, bytes)))
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |_, cx| {
            let read = reads.await;
            let _ = cx.update(|cx| {
                for (path, bytes) in read {
                    crate::history_window::record(path, bytes, Some("External change"), cx);
                }
            });
        })
        .detach();
        self.rebuild_rows();
        self.evaluate_projects(cx);
        cx.notify();
    }

    /// The tree, the projects and Roslyn after a solution or project file changed on disk.
    pub(super) fn follow_written(&mut self, path: &Path, cx: &mut Context<Self>) {
        let is_solution = self.tree.as_ref().is_some_and(|t| t.solution().path == path);
        if is_solution && let Some(solution) = rsharp_core::solution::read(path) {
            if let Some(tree) = &mut self.tree {
                tree.set_solution(solution);
            }
            self.rebuild_rows();
            self.sync_nuget(cx);
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
