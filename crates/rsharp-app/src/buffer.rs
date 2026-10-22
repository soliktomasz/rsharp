//! One open file's text, shared by every editor that shows it (a split, an editor window): its undo, its syntax
//! tree, the server's problems with it and the server told of every change. Each editor keeps its own carets,
//! scroll and folds, and is told of each change made through another so its carets keep their place.

use crate::editor::Diagnostic;
use gpui::{App, AppContext, Context, Entity, EntityId, EventEmitter, Global, Task, WeakEntity};
use rsharp_languages::{FoldRegion, edit as edit_tree, folds, parse};
use rsharp_lsp::LanguageServer;
use rsharp_text::{Change, Document, EditPlan, Selection};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tree_sitter::Tree;

pub enum BufferEvent {
    /// The text changed: by an edit through the editor `by`, or by Undo or Redo there.
    Edited { changes: Vec<Change>, by: EntityId },
    /// A parse landed: a new tree and the regions that fold.
    Parsed,
}

pub struct Buffer {
    pub path: PathBuf,
    pub document: Document,
    pub diagnostics: Vec<Diagnostic>,
    pub tree: Option<Tree>,
    /// The document's version the tree was parsed from: brackets are paired only in a tree that is current.
    pub tree_version: u64,
    pub regions: Vec<FoldRegion>,
    is_csharp: bool,
    parsing: Option<Task<()>>,
    server: Option<(Arc<LanguageServer>, String)>,
    checking: Option<Task<()>>,
}

impl EventEmitter<BufferEvent> for Buffer {}

/// Every buffer open in the process, by its file, so a file shown in the main window and an editor window is one
/// text.
#[derive(Default)]
struct Buffers(HashMap<PathBuf, WeakEntity<Buffer>>);

impl Global for Buffers {}

/// The buffer of a file open anywhere in the process.
pub fn find(path: &PathBuf, cx: &App) -> Option<Entity<Buffer>> {
    cx.try_global::<Buffers>()?.0.get(path)?.upgrade()
}

/// Reads a file into a new buffer, known from now to every window.
pub fn open(path: PathBuf, cx: &mut App) -> std::io::Result<Entity<Buffer>> {
    let bytes = std::fs::read(&path)?;
    let buffer = cx.new(|cx| Buffer::new(path.clone(), &bytes, cx));
    let buffers = &mut cx.default_global::<Buffers>().0;
    buffers.retain(|_, b| b.upgrade().is_some());
    buffers.insert(path, buffer.downgrade());
    Ok(buffer)
}

/// The last editor of a file gone: the server is told it is closed.
impl Drop for Buffer {
    fn drop(&mut self) {
        self.detach_server();
    }
}

impl Buffer {
    pub fn new(path: PathBuf, bytes: &[u8], cx: &mut Context<Self>) -> Self {
        // shortcut: bytes that are not UTF-8 are read lossily; Passenger reads them in the file's 8-bit charset.
        let document = Document::new(&String::from_utf8_lossy(bytes));
        let is_csharp = path.extension().is_some_and(|e| e == "cs" || e == "csx");
        let mut buffer = Self {
            path,
            document,
            diagnostics: Vec::new(),
            tree: None,
            tree_version: u64::MAX,
            regions: Vec::new(),
            is_csharp,
            parsing: None,
            server: None,
            checking: None,
        };
        buffer.reparse(cx);
        buffer
    }

    /// Applies an edit made with `carets`, and gives back the carets after it and what changed.
    pub fn edit(
        &mut self,
        carets: Vec<Selection>,
        plan: EditPlan,
        by: EntityId,
        cx: &mut Context<Self>,
    ) -> (Vec<Selection>, Vec<Change>) {
        self.document.set_selections(carets);
        let changes = self.document.apply(plan);
        self.changed(changes.clone(), by, cx);
        (self.document.selections().to_vec(), changes)
    }

    /// Undo (or Redo) of the document's last step, the carets it puts back and what changed; none with nothing to
    /// take back.
    pub fn undo(&mut self, redo: bool, by: EntityId, cx: &mut Context<Self>) -> Option<(Vec<Selection>, Vec<Change>)> {
        let changes = if redo { self.document.redo()? } else { self.document.undo()? };
        self.changed(changes.clone(), by, cx);
        Some((self.document.selections().to_vec(), changes))
    }

    /// What every change goes through: the tree told, the server told, a parse and a check asked for, and the other
    /// editors told.
    fn changed(&mut self, changes: Vec<Change>, by: EntityId, cx: &mut Context<Self>) {
        if changes.is_empty() {
            return;
        }
        if let Some(tree) = &mut self.tree {
            for change in &changes {
                edit_tree(tree, change);
            }
        }
        self.diagnostics.retain(|d| changes.iter().all(|c| d.range.end <= c.start.offset));
        if let Some((server, uri)) = &self.server {
            let content: Vec<_> = changes
                .iter()
                .map(|c| {
                    json!({
                        "range": {
                            "start": { "line": c.start.line, "character": c.start.utf16_column },
                            "end": { "line": c.old_end.line, "character": c.old_end.utf16_column },
                        },
                        "text": c.text,
                    })
                })
                .collect();
            server.notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": uri, "version": self.document.version() }, "contentChanges": content,
                }),
            );
        }
        self.reparse(cx);
        self.check(Duration::from_millis(400), cx);
        cx.emit(BufferEvent::Edited { changes, by });
        cx.notify();
    }

    fn reparse(&mut self, cx: &mut Context<Self>) {
        if !self.is_csharp {
            return;
        }
        let text = self.document.text().clone();
        let old = self.tree.clone();
        let version = self.document.version();
        self.parsing = Some(cx.spawn(async move |this, cx| {
            let (tree, regions) = cx
                .background_executor()
                .spawn(async move {
                    let tree = parse(&text, old.as_ref());
                    let regions = folds(&tree, &text);
                    (tree, regions)
                })
                .await;
            let _ = this.update(cx, |buffer, cx| {
                if buffer.document.version() == version {
                    buffer.tree = Some(tree);
                    buffer.tree_version = version;
                    buffer.regions = regions;
                    cx.emit(BufferEvent::Parsed);
                    cx.notify();
                }
            });
        }));
    }

    /// Opens the file in the server and keeps it there, edit by edit.
    pub fn attach_server(&mut self, server: Arc<LanguageServer>, cx: &mut Context<Self>) {
        if self.server.is_some() {
            return;
        }
        let uri = rsharp_core::paths::to_uri(&self.path);
        server.notify("textDocument/didOpen", json!({ "textDocument": {
            "uri": uri, "languageId": "csharp", "version": self.document.version(), "text": self.document.text().to_string(),
        }}));
        self.server = Some((server, uri));
        self.check(Duration::ZERO, cx);
    }

    pub fn detach_server(&mut self) {
        if let Some((server, uri)) = self.server.take() {
            server.notify("textDocument/didClose", json!({ "textDocument": { "uri": uri } }));
        }
    }

    /// Asks the server for the file's problems once typing has paused.
    pub fn check(&mut self, after: Duration, cx: &mut Context<Self>) {
        let Some((server, uri)) = self.server.clone() else { return };
        let version = self.document.version();
        self.checking = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(after).await;
            let answer = server.request("textDocument/diagnostic", json!({ "textDocument": { "uri": uri } }));
            let Ok(Ok(report)) = cx.background_executor().spawn(async move { answer.recv() }).await else { return };
            let _ = this.update(cx, |buffer, cx| {
                if buffer.document.version() != version {
                    return;
                }
                let at = |p: &serde_json::Value| {
                    buffer.document.offset_of_utf16(
                        p["line"].as_u64().unwrap_or(0) as usize,
                        p["character"].as_u64().unwrap_or(0) as usize,
                    )
                };
                buffer.diagnostics = report["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|d| d["severity"].as_u64().is_some_and(|s| s <= 2))
                    .map(|d| Diagnostic {
                        range: at(&d["range"]["start"])..at(&d["range"]["end"]),
                        error: d["severity"] == 1,
                        message: d["message"].as_str().unwrap_or_default().to_string(),
                    })
                    .collect();
                cx.notify();
            });
        }));
    }

    pub fn save(&mut self, cx: &mut Context<Self>) -> std::io::Result<()> {
        rsharp_core::files::write(&self.path, self.document.contents().as_bytes())?;
        self.document.mark_saved();
        if let Some((server, uri)) = &self.server {
            server.notify("textDocument/didSave", json!({ "textDocument": { "uri": uri } }));
        }
        cx.notify();
        Ok(())
    }
}
