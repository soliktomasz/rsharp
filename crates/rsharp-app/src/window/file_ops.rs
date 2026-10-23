//! The Solution view's file operations on what is on disk, as Rider's: New File, New Folder and a C# type where a row
//! holds things, Rename (a file's dependents and the project's items following it, a type offered its file's new
//! name) and Delete to the Trash.

use super::solution_actions::NameFor;
use super::*;
use crate::solution_menu::TYPE_KINDS;

/// Why a name cannot be a file's or a folder's here, or none; `folders` lets it name folders to make on the way.
pub(super) fn invalid_name(name: &str, folders: bool) -> Option<String> {
    let parts: Vec<&str> = if folders { name.split('/').collect() } else { vec![name] };
    if name.trim().is_empty() {
        return Some("A name is needed".into());
    }
    let bad = |part: &str| {
        part.trim().is_empty()
            || matches!(part, "." | "..")
            || part.contains(['\\', ':', '*', '?', '"', '<', '>', '|'])
            || (!folders && part.contains('/'))
    };
    parts.iter().any(|p| bad(p)).then(|| format!("{name} is not a file name"))
}

/// A name taken in a folder, as the file system would take it: Windows and macOS ignore letter case.
fn taken(folder: &Path, name: &str) -> bool {
    let wanted = name.to_lowercase();
    folder.join(name).exists()
        || std::fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| !cfg!(target_os = "linux") && e.file_name().to_string_lossy().to_lowercase() == wanted)
}

/// A namespace part as C# takes one: what cannot be in an identifier made `_`, and never a digit first.
fn identifier(part: &str) -> String {
    let mut out: String = part.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// A C# type's declaration as Rider's file templates write it.
fn declaration(kind: &str, name: &str) -> String {
    match kind {
        "Interface" => format!("public interface {name}\n{{\n}}\n"),
        "Record" => format!("public record {name}();\n"),
        "Struct" => format!("public struct {name}\n{{\n}}\n"),
        "Enum" => format!("public enum {name}\n{{\n}}\n"),
        _ => format!("public class {name}\n{{\n}}\n"),
    }
}

/// Where a declaration of a type of this name begins in a file's text, its name's byte offset.
fn declared(text: &str, name: &str) -> Option<usize> {
    let pattern =
        format!(r"\b(?:class|record|struct|interface|enum)\s+(?:class\s+|struct\s+)?({})\b", regex::escape(name));
    regex::Regex::new(&pattern).ok()?.captures(text)?.get(1).map(|m| m.start())
}

impl MainWindow {
    /// The folder a row puts what is made into: a project's, or the folder's own.
    pub(super) fn place_of(row: &TreeRow) -> Option<PathBuf> {
        match row.kind {
            NodeKind::Project => row.path.as_ref()?.parent().map(Path::to_path_buf),
            NodeKind::Folder => row.path.clone(),
            _ => None,
        }
    }

    /// The project a file or folder is in: the solution's project whose folder is nearest above it.
    fn project_of_path(&self, path: &Path) -> Option<PathBuf> {
        let tree = self.tree.as_ref()?;
        tree.solution()
            .projects()
            .into_iter()
            .filter(|p| p.path.parent().is_some_and(|f| path.starts_with(f)))
            .max_by_key(|p| p.path.components().count())
            .map(|p| p.path.clone())
    }

    /// The namespace Rider gives a type in a folder: the project's, then each folder down to it.
    fn namespace_for(&self, folder: &Path) -> String {
        // shortcut: the project's file name as its root namespace, not its RootNamespace property, until the Host's
        // evaluation carries RootNamespace.
        let Some(project) = self.project_of_path(folder) else {
            return identifier(&folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        };
        let root = project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut parts: Vec<String> = root.split('.').map(identifier).collect();
        if let Some(inside) = project.parent().and_then(|p| folder.strip_prefix(p).ok()) {
            parts.extend(inside.components().map(|c| identifier(&c.as_os_str().to_string_lossy())));
        }
        parts.join(".")
    }

    pub(super) fn choose_file_operation(
        &mut self,
        row: &TreeRow,
        title: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let into = Self::place_of(row);
        match (title, into) {
            ("Cut" | "Copy", _) if matches!(row.kind, NodeKind::File | NodeKind::Folder) => {
                self.hold(row, title == "Cut", cx)
            }
            ("Paste", _) => self.paste(row, window, cx),
            ("New File...", Some(into)) => {
                self.ask_name("New File", "", Vec::new(), NameFor::NewFile { into }, window, cx)
            }
            ("New Folder...", Some(into)) => {
                self.ask_name("New Folder", "", Vec::new(), NameFor::NewFolder { into }, window, cx)
            }
            ("Class/Interface...", Some(into)) => {
                self.ask_name("New Class/Interface", "", TYPE_KINDS.to_vec(), NameFor::NewType { into }, window, cx)
            }
            ("Rename...", _) if matches!(row.kind, NodeKind::File | NodeKind::Folder) => {
                let Some(path) = row.path.clone() else { return true };
                self.ask_name("Rename", &row.name, Vec::new(), NameFor::Rename { path }, window, cx);
            }
            ("Delete...", _) if matches!(row.kind, NodeKind::File | NodeKind::Folder) => {
                let Some(path) = row.path.clone() else { return true };
                let question = if path.is_dir() {
                    format!("Move the folder {} and everything in it to the Trash?", row.name)
                } else {
                    format!("Move {} to the Trash?", row.name)
                };
                let name = row.name.clone();
                self.confirm("Delete", question, "Delete", window, cx, move |w, window, cx| {
                    w.said = Some(
                        match w.delete_to_trash(&path, window, cx) {
                            Ok(()) => format!("Moved {name} to the Trash"),
                            Err(why) => why,
                        }
                        .into(),
                    );
                    w.evaluate_projects(cx);
                });
            }
            _ => return false,
        }
        true
    }

    /// New File, New Folder and New Class/Interface: made in the folder, shown in the tree, a file opened.
    pub(super) fn make(
        &mut self,
        into: &Path,
        name: &str,
        kind: Option<&'static str>,
        folder: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if let Some(why) = invalid_name(name, true) {
            return Err(why);
        }
        let mut name = name.trim().trim_matches('/').to_string();
        if kind.is_some() && !name.to_lowercase().ends_with(".cs") {
            name.push_str(".cs");
        }
        let path = into.join(&name);
        let (parent, file) = (
            path.parent().unwrap_or(into).to_path_buf(),
            path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        );
        if taken(&parent, &file) {
            return Err(format!("{file} is already there"));
        }
        std::fs::create_dir_all(&parent).map_err(|e| format!("{} could not be made: {e}", parent.display()))?;
        if folder {
            std::fs::create_dir(&path).map_err(|e| format!("{name} could not be made: {e}"))?;
        } else {
            let text = match kind {
                Some(kind) => {
                    let stem = file.trim_end_matches(".cs");
                    format!("namespace {};\n\n{}", self.namespace_for(&parent), declaration(kind, stem))
                }
                None => String::new(),
            };
            rsharp_core::files::write(&path, text.as_bytes())
                .map_err(|e| format!("{file} could not be written: {e}"))?;
        }
        let path = rsharp_core::paths::canonical(&path);
        if let Some(tree) = &mut self.tree {
            tree.forget(into);
            tree.reveal_path(&path);
        }
        self.rebuild_rows();
        if let Some(at) = self.rows.iter().position(|r| r.path.as_deref() == Some(path.as_path())) {
            self.selected = Some(at);
        }
        self.evaluate_projects(cx);
        if !folder {
            self.open_file(&path, None, window, cx);
        }
        self.save_state(cx);
        Ok(())
    }

    /// Rename of a file or folder on disk: the files nested under a file renamed with it (Counter.razor.cs with
    /// Counter.razor), and a `.cs` file named after its type offered the type's rename too, once Roslyn is there.
    pub(super) fn rename_path(
        &mut self,
        path: &Path,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if let Some(why) = invalid_name(name, false) {
            return Err(why);
        }
        let name = name.trim();
        let old = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name == old {
            return Ok(());
        }
        let folder = path.parent().unwrap_or(path).to_path_buf();
        // On macOS and Windows a name that differs only in letter case is the same file renamed, not one taken; on
        // Linux it is another name, which may be taken.
        let same = !cfg!(target_os = "linux") && name.eq_ignore_ascii_case(&old);
        if !same && taken(&folder, name) {
            return Err(format!("{name} is already there"));
        }
        let to = folder.join(name);
        let (old_stem, new_stem) = (
            path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            to.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        );
        let is_type = path.extension().is_some_and(|e| e == "cs")
            && to.extension().is_some_and(|e| e == "cs")
            && self.server.is_some()
            && std::fs::read_to_string(path).ok().and_then(|t| declared(&t, &old_stem)).is_some();
        if is_type {
            let question = format!("Rename the type {old_stem} to {new_stem} too?");
            self.asked = Some(question.clone());
            let chosen = window.prompt(
                gpui::PromptLevel::Info,
                "Rename",
                Some(&question),
                &["Rename Type", "Rename File Only"],
                cx,
            );
            let (path, to) = (path.to_path_buf(), to.clone());
            cx.spawn_in(window, async move |this, cx| {
                let answer = chosen.await;
                let _ = this.update_in(cx, |w, _, cx| {
                    let renamed = match answer {
                        Ok(0) => w.rename_type(&path, &old_stem, &new_stem, cx),
                        Ok(_) => w.move_paths(&[(path.clone(), to.clone())], cx),
                        Err(_) => Ok(()),
                    };
                    if let Err(why) = renamed {
                        w.said = Some(why.into());
                    }
                    cx.notify();
                });
            })
            .detach();
            return Ok(());
        }
        self.move_paths(&[(path.to_path_buf(), to)], cx)
    }

    /// Renames a type through Roslyn, at its declaration in `path`: its uses across the solution follow, and the files
    /// named after it are renamed with it, as Rider's Rename does.
    fn rename_type(&mut self, path: &Path, old: &str, new: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let server = self.server.clone().ok_or("Roslyn is not there to rename the type")?;
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let at = declared(&text, old).ok_or_else(|| format!("{old} is not declared in its file"))?;
        let rope = rsharp_text::Rope::from_str(&text);
        let uri = rsharp_core::paths::to_uri(path);
        let mut params = rsharp_lsp::at(&uri, crate::language::point(&rope, at));
        params["newName"] = serde_json::json!(new);
        let answer = crate::language::ask(&server, "textDocument/rename", params, cx);
        let (path, old, new) = (path.to_path_buf(), old.to_string(), new.to_string());
        cx.spawn(async move |this, cx| {
            let edit = answer.await.map(|a| rsharp_lsp::workspace_edit(&a)).unwrap_or_default();
            let _ = this.update(cx, |w, cx| {
                let applied = if edit.is_empty() {
                    Err(format!("{old} cannot be renamed"))
                } else {
                    crate::language::apply_workspace_edit(&edit, Some(&server), cx)
                };
                let moves = rsharp_lsp::files_named_after(std::slice::from_ref(&path), &old, &new);
                let moved = applied.and_then(|()| w.move_paths(&moves, cx));
                w.said = Some(moved.map(|()| format!("Renamed {old} to {new}")).unwrap_or_else(|why| why).into());
                cx.notify();
            });
        })
        .detach();
        Ok(())
    }

    /// Moves files and folders on disk: a file's dependents with it, renamed as it is; open tabs following, a folder's
    /// with it; Roslyn told; and each project's items naming what moved made to follow.
    pub(super) fn move_paths(&mut self, moves: &[(PathBuf, PathBuf)], cx: &mut Context<Self>) -> Result<(), String> {
        let mut all: Vec<(PathBuf, PathBuf)> = Vec::new();
        for (from, to) in moves {
            all.push((from.clone(), to.clone()));
            if from.is_file() {
                all.extend(self.dependents_of(from).into_iter().filter_map(|dependent| {
                    let renamed = renamed_dependent(&dependent, from, to);
                    to.parent().map(|folder| (dependent, folder.join(renamed)))
                }));
            }
        }
        let server = self.server.clone();
        for (from, to) in &all {
            if from.is_dir() {
                move_folder(from, to, server.as_deref(), cx)?;
            } else {
                crate::language::move_files(&[(from.clone(), to.clone())], server.as_deref(), cx)?;
            }
        }
        let projects: Vec<PathBuf> = all
            .iter()
            .filter_map(|(from, _)| self.project_of_path(from))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        for project in projects {
            let Ok(before) = std::fs::read(&project) else { continue };
            let (text, _) = rsharp_core::encodings::decode(&before, None);
            if rsharp_core::project_edits::move_items(&project, &text, &all).is_some() {
                self.write_edited(
                    &project,
                    |text| rsharp_core::project_edits::move_items(&project, text, &all).ok_or_else(String::new),
                    cx,
                )?;
            }
        }
        if let Some(tree) = &mut self.tree {
            for (from, to) in &all {
                tree.forget(from.parent().unwrap_or(from));
                tree.forget(to.parent().unwrap_or(to));
            }
            if let Some((_, to)) = all.first() {
                tree.reveal_path(to);
            }
        }
        self.rebuild_rows();
        if let Some((_, to)) = all.first()
            && let Some(at) = self.rows.iter().position(|r| r.path.as_deref() == Some(to.as_path()))
        {
            self.selected = Some(at);
        }
        self.evaluate_projects(cx);
        self.save_state(cx);
        cx.notify();
        Ok(())
    }

    /// The files shown under one, by Rider's nesting rules and its project's DependentUpon.
    pub(super) fn dependents_of(&self, path: &Path) -> Vec<PathBuf> {
        let Some(folder) = path.parent() else { return Vec::new() };
        let files: Vec<PathBuf> = std::fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| rsharp_core::paths::canonical(&e.path()))
            .filter(|p| p.is_file())
            .collect();
        let evaluated = self.project_of_path(path).and_then(|p| self.tree.as_ref()?.evaluated(&p).cloned());
        let parents = rsharp_core::nesting::parents(&files, |f| evaluated.as_ref()?.dependent_upon(f), true);
        let mut dependents: Vec<PathBuf> =
            parents.into_iter().filter(|(_, parent)| parent == path).map(|(child, _)| child).collect();
        dependents.sort();
        dependents
    }
}

/// A dependent's name once its parent is renamed: the part named after the parent (its name without its extension)
/// takes the new one's, Counter.razor.cs going with Counter.razor to Clicker.razor.cs; a name that does not begin with
/// it is kept.
pub(super) fn renamed_dependent(dependent: &Path, parent: &Path, renamed: &Path) -> String {
    let name = dependent.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = parent.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let new_stem = renamed.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    match name.strip_prefix(&format!("{stem}.")) {
        Some(rest) if !stem.is_empty() => format!("{new_stem}.{rest}"),
        _ => name,
    }
}

/// Moves a folder on disk, every open file in it going by its new path with its text kept, and Roslyn told of each
/// file that moved.
fn move_folder(
    from: &Path,
    to: &Path,
    server: Option<&rsharp_lsp::LanguageServer>,
    cx: &mut App,
) -> Result<(), String> {
    let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // Only macOS and Windows take a name differing in letter case alone for the same folder.
    let same = !cfg!(target_os = "linux") && to.to_string_lossy().eq_ignore_ascii_case(&from.to_string_lossy());
    if to.exists() && !same {
        return Err(format!("{name} was not moved: {} is there already", to.display()));
    }
    let moved: Vec<PathBuf> = crate::workspace::all_files(from, 100_000);
    std::fs::rename(from, to).map_err(|e| format!("{name} could not be moved: {e}"))?;
    let mut changes = Vec::new();
    for file in moved {
        let Ok(inside) = file.strip_prefix(from) else { continue };
        let now = to.join(inside);
        if let Some(open) = crate::buffer::find(&file, cx) {
            open.update(cx, |b, cx| b.moved(now.clone(), cx));
        }
        changes.push(serde_json::json!({ "uri": rsharp_core::paths::to_uri(&file), "type": 3 }));
        changes.push(serde_json::json!({ "uri": rsharp_core::paths::to_uri(&now), "type": 1 }));
    }
    if let Some(server) = server.filter(|_| !changes.is_empty()) {
        server.notify("workspace/didChangeWatchedFiles", serde_json::json!({ "changes": changes }));
    }
    Ok(())
}
