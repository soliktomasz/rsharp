//! The Solution view's Cut, Copy and Paste, and its drags, as Rider's (Passenger's `MainWindow.FileMoves`): files and
//! folders cut or copied go into the folder a row stands for, moved or copied (with the platform's copy key held, a
//! drag copies); a file's dependents go with it; a copy into its own folder is named by the person, as Rider's
//! Duplicate asks; a folder never goes inside itself.

use super::solution_actions::NameFor;
use super::*;

/// Rows dragged: their paths, and the name the drag shows.
#[derive(Clone)]
pub(super) struct RowDrag {
    pub paths: Vec<PathBuf>,
    pub name: SharedString,
}

impl Render for RowDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.))
            .h(px(ROW_HEIGHT))
            .flex()
            .items_center()
            .rounded(px(4.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .child(self.name.clone())
    }
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Copies a file, or a folder with all it holds. A link is passed over, never followed: one to the folder itself, or
/// above it, would copy without end, and one elsewhere would copy what is not the folder's.
fn copy_on_disk(from: &Path, to: &Path) -> std::io::Result<()> {
    let kind = std::fs::symlink_metadata(from)?.file_type();
    if kind.is_symlink() {
        // shortcut: a link inside a copied folder is left out of the copy; make it again as a link, as Rider's copy
        // does, once someone keeps links in a project.
        return Ok(());
    }
    if kind.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_on_disk(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

impl MainWindow {
    /// The folder a row stands for as a place to put things: a folder's own, a project's, a file's folder.
    pub(super) fn into_of(row: &TreeRow) -> Option<PathBuf> {
        match row.kind {
            NodeKind::Folder => row.path.clone(),
            NodeKind::Project => row.path.as_ref()?.parent().map(Path::to_path_buf),
            NodeKind::File if !row.key.starts_with("si:") => row.path.as_ref()?.parent().map(Path::to_path_buf),
            _ => None,
        }
    }

    /// Cut or Copy: the row's path on the clipboard as text, and remembered for a Paste while it is still there.
    pub(super) fn hold(&mut self, row: &TreeRow, cut: bool, cx: &mut Context<Self>) {
        let Some(path) = row.path.clone() else { return };
        let text = path.display().to_string();
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
        self.held = Some((vec![path], cut, text));
        self.said = Some(format!("{} {}", if cut { "Cut" } else { "Copied" }, row.name).into());
    }

    /// Paste on a row: what was cut or copied goes into the folder the row stands for.
    pub(super) fn paste(&mut self, row: &TreeRow, window: &mut Window, cx: &mut Context<Self>) {
        let Some(into) = Self::into_of(row) else { return };
        let on_clipboard = cx.read_from_clipboard().and_then(|c| c.text());
        let Some((paths, cut, _)) = self.held.clone().filter(|(_, _, text)| on_clipboard.as_deref() == Some(text))
        else {
            self.said = Some("Nothing cut or copied to paste".into());
            return;
        };
        match self.put(paths, into, !cut, window, cx) {
            Ok(_) if cut => self.held = None,
            Ok(_) => {}
            Err(why) => self.said = Some(why.into()),
        }
    }

    /// A drop of rows on a row: moved there, copied with the platform's copy key held.
    pub(super) fn dropped(&mut self, drag: &RowDrag, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(into) = self.rows.get(row).and_then(Self::into_of) else { return };
        let modifiers = window.modifiers();
        let copy = if cfg!(target_os = "macos") { modifiers.alt } else { modifiers.control };
        if let Err(why) = self.put(drag.paths.clone(), into, copy, window, cx) {
            self.said = Some(why.into());
        }
        cx.notify();
    }

    /// Puts files and folders into a folder, moved or copied, their dependents with them; a copy into its own folder
    /// asks its name as Rider's Duplicate does. The paths put, or why not.
    pub fn put(
        &mut self,
        paths: Vec<PathBuf>,
        into: PathBuf,
        copy: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Vec<PathBuf>, String> {
        let into = rsharp_core::paths::canonical(&into);
        let folder = name_of(&into);
        for path in &paths {
            if path.is_dir() && into.starts_with(path) {
                return Err(format!("{} cannot go inside itself", name_of(path)));
            }
        }
        if !copy {
            let mut moves = Vec::new();
            for path in paths.iter().filter(|p| p.parent() != Some(into.as_path())) {
                let to = into.join(name_of(path));
                if to.exists() {
                    // shortcut: a name taken in the target is refused, where Rider asks to replace it; ask once
                    // someone moves onto a file they mean to replace.
                    return Err(format!("{} is already in {folder}", name_of(path)));
                }
                moves.push((path.clone(), to));
            }
            self.move_paths(&moves, cx)?;
            let put: Vec<PathBuf> = moves.into_iter().map(|(_, to)| to).collect();
            if let [one] = put.as_slice() {
                self.said = Some(format!("Moved {} to {folder}", name_of(one)).into());
            }
            return Ok(put);
        }
        let mut put = Vec::new();
        for path in &paths {
            let to = into.join(name_of(path));
            if to.exists() {
                if path.parent() == Some(into.as_path()) && paths.len() == 1 {
                    let name = name_of(path);
                    let purpose = NameFor::Duplicate { from: path.clone(), into: into.clone() };
                    self.ask_name(&format!("Duplicate {name}"), &name, Vec::new(), purpose, window, cx);
                    return Ok(Vec::new());
                }
                return Err(format!("{} is already in {folder}", name_of(path)));
            }
            self.copy_with_dependents(path, &to)?;
            put.push(to);
        }
        self.after_put(&into, &put, cx);
        if let [one] = put.as_slice() {
            self.said = Some(format!("Copied {} to {folder}", name_of(one)).into());
        }
        Ok(put)
    }

    /// A copy, a file's dependents copied with it under names that follow its own.
    fn copy_with_dependents(&mut self, from: &Path, to: &Path) -> Result<(), String> {
        let mut copies = vec![(from.to_path_buf(), to.to_path_buf())];
        if from.is_file() {
            for dependent in self.dependents_of(from) {
                let renamed = super::file_ops::renamed_dependent(&dependent, from, to);
                copies.push((dependent, to.parent().unwrap_or(to).join(renamed)));
            }
        }
        // Every target checked before anything is copied, so a copy never lands on a file already there.
        if let Some((_, taken)) = copies.iter().find(|(_, target)| target.exists()) {
            return Err(format!("{} is already there", name_of(taken)));
        }
        for (from, to) in copies {
            copy_on_disk(&from, &to).map_err(|e| format!("{} could not be copied: {e}", name_of(&from)))?;
        }
        Ok(())
    }

    /// The Duplicate question answered: the copy made under the name chosen.
    pub(super) fn duplicate(
        &mut self,
        from: &Path,
        into: &Path,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if let Some(why) = super::file_ops::invalid_name(name, false) {
            return Err(why);
        }
        let to = into.join(name.trim());
        if to.exists() {
            return Err(format!("{} is already there", name.trim()));
        }
        self.copy_with_dependents(from, &to)?;
        self.after_put(into, std::slice::from_ref(&to), cx);
        self.said = Some(format!("Copied {} to {}", name_of(from), name.trim()).into());
        Ok(())
    }

    /// The tree and the projects after files were put in a folder: read again there, the first shown and selected.
    fn after_put(&mut self, into: &Path, put: &[PathBuf], cx: &mut Context<Self>) {
        if let Some(tree) = &mut self.tree {
            tree.forget(into);
            if let Some(first) = put.first() {
                tree.reveal_path(first);
            }
        }
        self.rebuild_rows();
        if let Some(first) = put.first()
            && let Some(at) = self.rows.iter().position(|r| r.path.as_deref() == Some(first.as_path()))
        {
            self.selected = Some(at);
        }
        self.evaluate_projects(cx);
        self.save_state(cx);
        cx.notify();
    }
}
