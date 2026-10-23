//! Where the language server's answers meet the files: its places as offsets, its edits as the editor's, and a
//! workspace edit (a rename, a context action) made across the files it names, open or not.

use crate::buffer::{self, Buffer};
use gpui::{App, Entity, Task};
use rsharp_lsp::{LanguageServer, Point, TextEdit, WorkspaceEdit};
use rsharp_text::{Edit, Rope, offset_of_utf16, position};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn point(text: &Rope, offset: usize) -> Point {
    let at = position(text, offset.min(text.len_bytes()));
    Point::new(at.line, at.utf16_column)
}

pub fn offset(text: &Rope, point: Point) -> usize {
    offset_of_utf16(text, point.line, point.character)
}

/// The server's edits as the editor's: in the text's order, as `Document::apply` takes them, overlaps dropped.
pub fn edits(text: &Rope, edits: &[TextEdit]) -> Vec<Edit> {
    let mut made: Vec<Edit> = edits
        .iter()
        .map(|e| Edit { range: offset(text, e.span.start)..offset(text, e.span.end), text: e.text.clone() })
        .collect();
    made.sort_by_key(|e| (e.range.start, e.range.end));
    let mut end = 0;
    made.retain(|e| {
        let keep = e.range.start >= end;
        end = end.max(e.range.end);
        keep
    });
    made
}

/// Asks the server, its answer awaited without holding a thread; None when it refused or stopped, which the log
/// says.
pub fn ask(server: &LanguageServer, method: &'static str, params: Value, cx: &App) -> Task<Option<Value>> {
    let answer = server.ask(method, params);
    cx.background_executor().spawn(async move {
        match answer.await {
            Ok(value) => Some(value),
            Err(failed) => {
                log::info!(target: "roslyn", "{method} refused: {failed}");
                None
            }
        }
    })
}

/// Makes a workspace edit: an open file's text edited as one undo step (left unsaved, as Rider leaves it), a file
/// not open written in its own encoding, the files it makes made first. The server is told of the files written.
/// Says which file could not be changed, and stops there.
// shortcut: a move (a rename's file renamed with its type) is made only for a file not open, and is not one step
// with the edits for Undo; make it follow an open file's tab once Rename moves files.
pub fn apply_workspace_edit(edit: &WorkspaceEdit, server: Option<&LanguageServer>, cx: &mut App) -> Result<(), String> {
    let mut written: Vec<(PathBuf, u8)> = Vec::new();
    for path in &edit.created {
        if !path.exists() {
            if let Some(folder) = path.parent() {
                std::fs::create_dir_all(folder).map_err(|e| format!("{} could not be made: {e}", name(path)))?;
            }
            rsharp_core::files::write(path, b"").map_err(|e| format!("{} could not be made: {e}", name(path)))?;
            written.push((path.clone(), 1));
        }
    }
    for (path, text_edits) in &edit.files {
        let open = buffer::find(path, cx);
        let shown = open.is_some();
        let buffer: Entity<Buffer> = match open {
            Some(buffer) => buffer,
            None => buffer::open(path.clone(), cx).map_err(|e| format!("{} could not be read: {e}", name(path)))?,
        };
        buffer.update(cx, |b, cx| {
            let made = edits(b.document.text(), text_edits);
            b.apply_edits(made, cx);
            if shown { Ok(()) } else { b.save(cx).map_err(|e| format!("{} could not be written: {e}", name(path))) }
        })?;
        if !shown {
            written.push((path.clone(), 2));
        }
    }
    for (from, to) in &edit.moves {
        if buffer::find(from, cx).is_some() {
            return Err(format!("{} is open, so it was not renamed to {}", name(from), name(to)));
        }
        if to.exists() {
            return Err(format!("{} was not renamed: {} is there already", name(from), name(to)));
        }
        std::fs::rename(from, to).map_err(|e| format!("{} could not be renamed: {e}", name(from)))?;
        written.push((from.clone(), 3));
        written.push((to.clone(), 1));
    }
    // A file made or renamed is in no design-time build the server has: its project is loaded again.
    let new_files: Vec<PathBuf> = written.iter().filter(|(_, kind)| *kind == 1).map(|(path, _)| path.clone()).collect();
    if let Some(server) = server.filter(|_| !new_files.is_empty()) {
        rsharp_lsp::reload_projects_of(server, &new_files);
    }
    if let Some(server) = server.filter(|_| !written.is_empty()) {
        let changes: Vec<Value> =
            written.iter().map(|(p, kind)| json!({ "uri": rsharp_core::paths::to_uri(p), "type": kind })).collect();
        server.notify("workspace/didChangeWatchedFiles", json!({ "changes": changes }));
    }
    Ok(())
}

/// Replace in Files: every match of `query` in `files` (ASCII case ignored, as Find in Files matches) replaced, an
/// open file's as one undo step left unsaved, a file not open written in its own encoding; the server told of the
/// files written. Says how many matches were replaced, or which file could not be changed.
pub fn replace_in_files(
    files: &[PathBuf],
    query: &str,
    replacement: &str,
    server: Option<&LanguageServer>,
    cx: &mut App,
) -> Result<usize, String> {
    let mut replaced = 0;
    let mut written = Vec::new();
    for path in files {
        let open = buffer::find(path, cx);
        let shown = open.is_some();
        let buffer = match open {
            Some(buffer) => buffer,
            None if crate::workspace::searched_text(path, &Default::default()).is_none() => continue,
            None => buffer::open(path.clone(), cx).map_err(|e| format!("{} could not be read: {e}", name(path)))?,
        };
        let count = buffer.update(cx, |b, cx| {
            let text = b.document.text().to_string();
            let edits: Vec<Edit> = crate::workspace::matches(&text, query)
                .into_iter()
                .map(|range| Edit { range, text: replacement.to_string() })
                .collect();
            let count = edits.len();
            if count > 0 {
                b.apply_edits(edits, cx);
                if !shown {
                    b.save(cx).map_err(|e| format!("{} could not be written: {e}", name(path)))?;
                }
            }
            Ok::<usize, String>(count)
        })?;
        if count > 0 && !shown {
            written.push(json!({ "uri": rsharp_core::paths::to_uri(path), "type": 2 }));
        }
        replaced += count;
    }
    if let Some(server) = server.filter(|_| !written.is_empty()) {
        server.notify("workspace/didChangeWatchedFiles", json!({ "changes": written }));
    }
    Ok(replaced)
}

/// Renames files, as a type's rename does: an open one keeps its tab and its unsaved text under its new name, one not
/// open is renamed on disk, and the server is told of both. Says which could not be, and stops there.
pub fn move_files(moves: &[(PathBuf, PathBuf)], server: Option<&LanguageServer>, cx: &mut App) -> Result<(), String> {
    let mut changes = Vec::new();
    for (from, to) in moves {
        // On macOS and Windows a name differing only in letter case is the same file, there already.
        let same = !cfg!(target_os = "linux") && from.to_string_lossy().eq_ignore_ascii_case(&to.to_string_lossy());
        if to.exists() && !same {
            return Err(format!("{} was not renamed: {} is there already", name(from), name(to)));
        }
        let renamed = match buffer::find(from, cx) {
            Some(open) => open.update(cx, |b, cx| b.move_to(to.clone(), cx)),
            None => std::fs::rename(from, to),
        };
        renamed.map_err(|e| format!("{} could not be renamed: {e}", name(from)))?;
        changes.push(json!({ "uri": rsharp_core::paths::to_uri(from), "type": 3 }));
        changes.push(json!({ "uri": rsharp_core::paths::to_uri(to), "type": 1 }));
    }
    if let Some(server) = server.filter(|_| !changes.is_empty()) {
        server.notify("workspace/didChangeWatchedFiles", json!({ "changes": changes }));
        let renamed: Vec<PathBuf> = moves.iter().map(|(_, to)| to.clone()).collect();
        rsharp_lsp::reload_projects_of(server, &renamed);
    }
    Ok(())
}

fn name(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsharp_lsp::Span;

    #[test]
    fn the_servers_edits_land_in_the_texts_order_on_utf16_columns() {
        let text = Rope::from("😀ab\ncd\n");
        let at = |line, character| Point::new(line, character);
        let made = edits(
            &text,
            &[
                TextEdit { span: Span { start: at(1, 0), end: at(1, 1) }, text: "C".into() },
                TextEdit { span: Span { start: at(0, 2), end: at(0, 3) }, text: "A".into() },
            ],
        );
        assert_eq!(
            made.iter().map(|e| (e.range.clone(), e.text.as_str())).collect::<Vec<_>>(),
            [(4..5, "A"), (7..8, "C")]
        );
    }
}
