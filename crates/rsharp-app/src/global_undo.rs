//! Rider's global undo (Passenger's `EditorArea.Commands`): a change that edits several files, or makes or moves one
//! (a rename, Extract, create from usage), is undone and redone whole from an Undo or Redo in any file it changed,
//! asked first. Each file's text before and after is kept; an open file is taken back through its own undo where its
//! last step is the change, else set to the text; a file not open is written back. Nothing is done when any file no
//! longer reads as the change left it.

use crate::buffer;
use gpui::{App, Global};
use std::path::{Path, PathBuf};

/// One file's text as the change found it and as it left it, by the path the change edited it at, and whether the
/// change wrote it to disk (a file no editor showed), which taking it back does too.
pub(crate) struct FileChange {
    pub path: PathBuf,
    pub before: String,
    pub after: String,
    pub saved: bool,
}

/// A change to several files: the texts it changed, the files it made and the files it moved, in its order.
#[derive(Default)]
pub(crate) struct Command {
    pub files: Vec<FileChange>,
    pub created: Vec<PathBuf>,
    pub moves: Vec<(PathBuf, PathBuf)>,
}

#[derive(Default)]
struct Commands {
    done: Vec<Command>,
    undone: Vec<Command>,
}

impl Global for Commands {}

/// How many changes are kept to undo, as Rider keeps its global ones.
const KEPT: usize = 50;

impl Command {
    /// Where a file of the change is now: its moved name once the change is made.
    fn now(&self, path: &Path, made: bool) -> PathBuf {
        let moved = self.moves.iter().find(|(from, to)| if made { from == path } else { to == path });
        match moved {
            Some((from, to)) => if made { to } else { from }.clone(),
            None => path.to_path_buf(),
        }
    }

    fn touches(&self) -> usize {
        let mut paths: Vec<&PathBuf> = self.files.iter().map(|f| &f.path).chain(self.created.iter()).collect();
        paths.extend(self.moves.iter().map(|(from, _)| from));
        paths.sort();
        paths.dedup();
        paths.len()
    }
}

/// Keeps a change made across files, which a later Undo takes back whole; one file's change is that file's own undo.
pub(crate) fn record(command: Command, cx: &mut App) {
    if command.touches() < 2 && command.moves.is_empty() && command.created.is_empty() {
        return;
    }
    let commands = cx.default_global::<Commands>();
    commands.undone.clear();
    commands.done.push(command);
    if commands.done.len() > KEPT {
        commands.done.remove(0);
    }
}

/// How many files the change an Undo (or a Redo) in `path` would take back whole: the last change, while `text` reads
/// as it left the file (or found it, for a Redo).
pub fn offered(path: &Path, text: &str, redo: bool, cx: &App) -> Option<usize> {
    let commands = cx.try_global::<Commands>()?;
    let command = if redo { commands.undone.last()? } else { commands.done.last()? };
    let file = command.files.iter().find(|f| command.now(&f.path, !redo) == path)?;
    (text == if redo { &file.before } else { &file.after }).then(|| command.touches())
}

/// The file's text now: its open buffer's, else the disk's as a buffer reads it.
fn text_of(path: &Path, cx: &mut App) -> Result<Option<String>, String> {
    let path = path.to_path_buf();
    if let Some(open) = buffer::find(&path, cx) {
        return Ok(Some(open.read(cx).document.text().to_string()));
    }
    if !path.exists() {
        return Ok(None);
    }
    let read = buffer::open(path.clone(), cx).map_err(|e| format!("{} could not be read: {e}", name(&path)))?;
    Ok(Some(read.read(cx).document.text().to_string()))
}

fn name(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// Undoes (or redoes) the last change whole: every file it changed, made or moved; says what was done, or why not.
pub fn run(redo: bool, cx: &mut App) -> Result<String, String> {
    let Some(command) = cx.try_global::<Commands>().and_then(|c| if redo { c.undone.last() } else { c.done.last() })
    else {
        return Err("Nothing to take back across files".into());
    };
    // Every file must read as the change left it (or found it): a later edit is not taken back with it.
    let mut checks = Vec::new();
    for file in &command.files {
        let path = command.now(&file.path, !redo);
        let expected = if redo { &file.before } else { &file.after };
        checks.push((path, expected.clone()));
    }
    for (path, expected) in checks {
        if text_of(&path, cx)?.is_some_and(|text| text != expected) {
            return Err(format!("{} was changed since, so only that file's own Undo takes it back", name(&path)));
        }
    }
    let commands = cx.default_global::<Commands>();
    let command = if redo { commands.undone.pop() } else { commands.done.pop() }.expect("checked above");
    let count = command.touches();
    let made = take(&command, redo, cx);
    let commands = cx.default_global::<Commands>();
    if redo {
        commands.done.push(command)
    } else {
        commands.undone.push(command)
    }
    made.map(|_| format!("{} the change to {count} files", if redo { "Redid" } else { "Undid" }))
}

/// The change taken back (or made again): moves reversed first and files made gone last on Undo, the other way on Redo.
fn take(command: &Command, redo: bool, cx: &mut App) -> Result<(), String> {
    if redo {
        for path in &command.created {
            rsharp_core::files::write(path, b"").map_err(|e| format!("{} could not be made: {e}", name(path)))?;
        }
    } else {
        let back: Vec<(PathBuf, PathBuf)> =
            command.moves.iter().rev().map(|(from, to)| (to.clone(), from.clone())).collect();
        crate::language::move_files(&back, None, cx)?;
    }
    for file in &command.files {
        let target = if redo { &file.after } else { &file.before };
        if !redo && command.created.contains(&file.path) {
            continue;
        }
        set(&file.path, target, redo, file.saved, cx)?;
    }
    if redo {
        crate::language::move_files(&command.moves, None, cx)?;
    } else {
        for path in &command.created {
            rsharp_core::trash::move_to_trash(path).map_err(|e| format!("{} could not be removed: {e}", name(path)))?;
        }
    }
    Ok(())
}

/// One file set to `target`: an open one through its own undo where that reaches it, else as an edit of its own, and
/// saved where the change saved it; one not open written back.
fn set(path: &Path, target: &str, redo: bool, saved: bool, cx: &mut App) -> Result<(), String> {
    let path = path.to_path_buf();
    if let Some(open) = buffer::find(&path, cx) {
        return open.update(cx, |b, cx| {
            let id = cx.entity_id();
            let stepped = b.undo(redo, id, cx).is_some();
            if b.document.text() != target {
                if stepped {
                    b.undo(!redo, id, cx);
                }
                b.replace_all(target.to_string(), cx);
            }
            if saved { b.save(cx).map_err(|e| format!("{} could not be written: {e}", name(&path))) } else { Ok(()) }
        });
    }
    let read = buffer::open(path.clone(), cx).map_err(|e| format!("{} could not be read: {e}", name(&path)))?;
    read.update(cx, |b, cx| {
        b.replace_all(target.to_string(), cx);
        b.save(cx).map_err(|e| format!("{} could not be written: {e}", name(&path)))
    })
}
