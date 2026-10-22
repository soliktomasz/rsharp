//! One open file's text, shared by every editor that shows it (a split, an editor window): its undo, its syntax
//! tree, the server's problems with it and the server told of every change. Each editor keeps its own carets,
//! scroll and folds, and is told of each change made through another so its carets keep their place.

use crate::editor::Diagnostic;
use gpui::{App, AppContext, Context, Entity, EntityId, EventEmitter, Global, Task, WeakEntity};
use rsharp_core::editorconfig::EditorConfig;
use rsharp_core::encodings::{Charset, decode, encode};
use rsharp_languages::{edit as edit_tree, parse};
use rsharp_lsp::LanguageServer;
use rsharp_text::{Change, Document, Edit, EditPlan, Indent, LineEnding, Rope, Selection, detect_indent, line_end};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tree_sitter::Tree;

pub enum BufferEvent {
    /// The text changed: by an edit through the editor `by`, or by Undo or Redo there.
    Edited { changes: Vec<Change>, by: EntityId },
    /// A parse landed: a new tree.
    Parsed,
}

/// Where a file's indentation comes from, the first of these that says, as Rider's indent widget names it.
#[derive(Clone, PartialEq, Debug)]
pub enum IndentSource {
    EditorConfig(PathBuf),
    Detected,
    Default,
}

pub struct Buffer {
    pub path: PathBuf,
    pub document: Document,
    /// How the file's bytes are its text, as read, or as chosen since in the status bar.
    pub charset: Charset,
    saved_charset: Charset,
    pub indent: Indent,
    pub indent_source: IndentSource,
    config: EditorConfig,
    /// Whether the file may not be written; its first edit and its save ask to clear that.
    pub read_only: bool,
    /// The line separator chosen in the status bar, which wins over the .editorconfig's while the file is open.
    line_ending_chosen: bool,
    /// When the file on disk was last read or written, and its length, to tell another program's change.
    on_disk: Option<(std::time::SystemTime, u64)>,
    pub diagnostics: Vec<Diagnostic>,
    pub tree: Option<Tree>,
    /// The document's version the tree was parsed from: brackets are paired only in a tree that is current.
    pub tree_version: u64,
    is_csharp: bool,
    parsing: Option<Task<()>>,
    server: Option<(Arc<LanguageServer>, String)>,
    checking: Option<Task<()>>,
}

impl EventEmitter<BufferEvent> for Buffer {}

impl Buffer {
    /// The .editorconfig's trim_trailing_whitespace and insert_final_newline, as one edit every editor sees.
    // shortcut: whitespace is trimmed on the caret's line too; Rider keeps it there while the caret stays.
    fn tidy(&mut self, cx: &mut Context<Self>) {
        let text = self.document.text().clone();
        let mut edits = Vec::new();
        if self.config.flag("trim_trailing_whitespace") == Some(true) {
            for line in 0..text.len_lines() {
                let (start, end) = (text.line_to_byte(line), line_end(&text, line));
                let content = text.byte_slice(start..end).to_string();
                let kept = content.trim_end_matches([' ', '\t']).len();
                if kept < content.len() {
                    edits.push(Edit { range: start + kept..end, text: String::new() });
                }
            }
        }
        let len = text.len_bytes();
        if self.config.flag("insert_final_newline") == Some(true) && len > 0 && text.char(text.len_chars() - 1) != '\n'
        {
            edits.push(Edit { range: len..len, text: "\n".into() });
        }
        if edits.is_empty() {
            return;
        }
        let selections = self.document.selections().to_vec();
        let changes = self.document.apply(EditPlan { edits, selections, ..Default::default() });
        let me = cx.entity_id();
        self.changed(changes, me, cx);
    }
}

/// A file's indentation: its .editorconfig's, else what its own lines show, else C#'s.
fn choose_indent(config: &EditorConfig, text: &Rope) -> (Indent, IndentSource) {
    let number = |key: &str| config.get(key).and_then(|v| v.parse::<usize>().ok()).filter(|n| *n > 0);
    let size = match config.get("indent_size") {
        Some("tab") => number("tab_width"),
        _ => number("indent_size").or_else(|| number("tab_width")),
    };
    let configured = match config.get("indent_style") {
        Some("tab") => Some(Indent { tabs: true, size: size.unwrap_or(4) }),
        Some("space") => Some(Indent { tabs: false, size: size.unwrap_or(4) }),
        _ => size.map(|size| Indent { tabs: false, size }),
    };
    let from = config.source("indent_style").or_else(|| config.source("indent_size"));
    match (configured, from) {
        (Some(indent), Some(from)) => (indent, IndentSource::EditorConfig(from.to_path_buf())),
        _ => match detect_indent(text) {
            Some(indent) => (indent, IndentSource::Detected),
            None => (Indent::default(), IndentSource::Default),
        },
    }
}

fn stamp(path: &std::path::Path) -> Option<(std::time::SystemTime, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

fn line_ending_named(value: &str) -> Option<LineEnding> {
    match value {
        "lf" => Some(LineEnding::Lf),
        "crlf" => Some(LineEnding::CrLf),
        "cr" => Some(LineEnding::Cr),
        _ => None,
    }
}

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
    // What the file is as it opens: a version of its own when another program changed it since the last kept.
    crate::history_window::record(path.clone(), bytes.clone(), Some("External change"), cx);
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
        let config = EditorConfig::for_file(&path);
        let (text, charset) = decode(bytes, config.get("charset").and_then(Charset::from_editorconfig));
        let document = Document::new(&text);
        let (indent, indent_source) = choose_indent(&config, document.text());
        let read_only = std::fs::metadata(&path).is_ok_and(|m| m.permissions().readonly());
        let is_csharp = path.extension().is_some_and(|e| e == "cs" || e == "csx");
        let mut buffer = Self {
            path,
            document,
            charset,
            saved_charset: charset,
            indent,
            indent_source,
            config,
            read_only,
            line_ending_chosen: false,
            on_disk: None,
            diagnostics: Vec::new(),
            tree: None,
            tree_version: u64::MAX,
            is_csharp,
            parsing: None,
            server: None,
            checking: None,
        };
        buffer.on_disk = stamp(&buffer.path);
        buffer.reparse(cx);
        buffer
    }

    /// The file read again if another program changed it since, while nothing here is unsaved, as Rider reloads a
    /// file it sees changed; its new content kept in Local History as an External change.
    // shortcut: looked at only when the file is opened again; the folder watcher (phase 4) tells it as it happens.
    pub fn refresh_from_disk(&mut self, cx: &mut Context<Self>) {
        let now = stamp(&self.path);
        if now.is_none() || now == self.on_disk || self.is_modified() {
            return;
        }
        let Ok(bytes) = std::fs::read(&self.path) else { return };
        crate::history_window::record(self.path.clone(), bytes.clone(), Some("External change"), cx);
        let (text, charset) = decode(&bytes, self.config.get("charset").and_then(Charset::from_editorconfig));
        let read = Document::new(&text);
        self.charset = charset;
        self.saved_charset = charset;
        self.document.line_ending = read.line_ending;
        self.replace_all(read.text().to_string(), cx);
        self.document.mark_saved();
        self.on_disk = now;
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
        // tree-sitter parses again in time that grows with the members of one class; a file as large as a
        // generated one waits for typing to pause, the tree told of each edit meanwhile.
        let pause = if text.len_bytes() > 4 << 20 { Duration::from_millis(150) } else { Duration::ZERO };
        self.parsing = Some(cx.spawn(async move |this, cx| {
            if pause > Duration::ZERO {
                cx.background_executor().timer(pause).await;
            }
            let tree = cx.background_executor().spawn(async move { parse(&text, old.as_ref()) }).await;
            let _ = this.update(cx, |buffer, cx| {
                if buffer.document.version() == version {
                    buffer.tree = Some(tree);
                    buffer.tree_version = version;
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

    /// Whether the file on disk differs: its text, its line separator or its encoding.
    pub fn is_modified(&self) -> bool {
        self.document.is_modified() || self.charset != self.saved_charset
    }

    /// The status bar's Line Separators: the file's breaks written as `ending` from its next save.
    pub fn set_line_ending(&mut self, ending: LineEnding, cx: &mut Context<Self>) {
        self.document.line_ending = ending;
        self.line_ending_chosen = true;
        cx.notify();
    }

    /// The status bar's encoding: the text kept and written in `charset` from its next save (Rider's Convert).
    // shortcut: no Reload, which reads the file's bytes again in the encoding chosen; offer it once a file opens
    // in the wrong one.
    pub fn set_charset(&mut self, charset: Charset, cx: &mut Context<Self>) {
        self.charset = charset;
        cx.notify();
    }

    /// The whole text replaced, as one step Undo takes back (Local History's Revert); every editor's carets follow.
    pub fn replace_all(&mut self, text: String, cx: &mut Context<Self>) {
        let len = self.document.text().len_bytes();
        let plan = EditPlan {
            edits: vec![Edit { range: 0..len, text }],
            selections: vec![Selection::caret(0)],
            ..Default::default()
        };
        let changes = self.document.apply(plan);
        let me = cx.entity_id();
        self.changed(changes, me, cx);
    }

    /// Clear Read-Only Status: the file made writable.
    pub fn clear_read_only(&mut self, cx: &mut Context<Self>) -> std::io::Result<()> {
        let mut permissions = std::fs::metadata(&self.path)?.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&self.path, permissions)?;
        self.read_only = false;
        cx.notify();
        Ok(())
    }

    /// Writes the file: the .editorconfig's save-time rules applied, then its text in its encoding with its line
    /// separator. A read-only file, or a character the encoding cannot hold, is refused before anything is written.
    pub fn save(&mut self, cx: &mut Context<Self>) -> std::io::Result<()> {
        if self.read_only {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "the file is read-only"));
        }
        self.tidy(cx);
        if !self.line_ending_chosen
            && let Some(ending) = self.config.get("end_of_line").and_then(line_ending_named)
        {
            self.document.line_ending = ending;
        }
        let bytes = encode(&self.document.contents(), self.charset)
            .map_err(|c| std::io::Error::other(format!("'{c}' cannot be written in {}", self.charset.name())))?;
        rsharp_core::files::write(&self.path, &bytes)?;
        crate::history_window::record(self.path.clone(), bytes, None, cx);
        self.on_disk = stamp(&self.path);
        self.saved_charset = self.charset;
        self.document.mark_saved();
        if let Some((server, uri)) = &self.server {
            server.notify("textDocument/didSave", json!({ "textDocument": { "uri": uri } }));
        }
        cx.notify();
        Ok(())
    }
}
