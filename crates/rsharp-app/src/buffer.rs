//! One open file's text, shared by every editor that shows it (a split, an editor window): its undo, its syntax
//! tree, the server's problems with it and the server told of every change. Each editor keeps its own carets,
//! scroll and folds, and is told of each change made through another so its carets keep their place.

use crate::editor::Diagnostic;
use futures::StreamExt;
use gpui::{App, AppContext, Context, Entity, EntityId, EventEmitter, Global, Task, WeakEntity};
use rsharp_core::editorconfig::EditorConfig;
use rsharp_core::encodings::{Charset, decode, encode};
use rsharp_languages::{edit as edit_tree, parse};
use rsharp_lsp::LanguageServer;
use rsharp_text::{Change, Document, Edit, EditPlan, Indent, LineEnding, Rope, Selection, detect_indent, line_end};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tree_sitter::Tree;

pub enum BufferEvent {
    /// The text changed: by an edit through the editor `by`, or by Undo or Redo there.
    Edited { changes: Vec<Change>, by: EntityId },
    /// A parse landed: a new tree.
    Parsed,
    /// The file was renamed on disk (a type's rename): its new path.
    Moved(PathBuf),
    /// Another project context (target framework) chosen: what the server says of the file asked again.
    ContextChanged,
}

/// A breakpoint as the gutter draws it: its line from 0, whether the debugger will hit it, and whether it is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BreakpointMark {
    pub line: usize,
    /// The adapter's word, once a session has one: bound, or not and why not.
    pub verified: bool,
    /// Off, or every breakpoint muted: the dot is still there, dimmed.
    pub enabled: bool,
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
    pub git_base: Option<Arc<str>>,
    pub git_hunks: Arc<Vec<rsharp_text::DiffHunk>>,
    /// The breakpoints of the file, as the folder has them and the debugger last bound them.
    pub breakpoints: Vec<BreakpointMark>,
    /// The line the debugger is stopped at, if it is stopped in this file.
    pub execution_line: Option<usize>,
    /// The frame's values, set in at the end of the stopped line as Rider's inline values are.
    pub inline_values: Option<(usize, String)>,
    /// The exception a stop is on, named at the line that threw it.
    pub exception: Option<(usize, String)>,
    git_comparing: Option<Task<()>>,
    is_csharp: bool,
    parsing: Option<Task<()>>,
    server: Option<(Arc<LanguageServer>, String)>,
    checking: Option<Task<()>>,
    /// The projects Roslyn compiles the file in (a target framework of a multi-targeted project each), and the one its
    /// requests are about, as the editor's target framework switcher chose it.
    pub contexts: Vec<Value>,
    pub context: usize,
    asking_contexts: Option<Task<()>>,
    /// An MSBuild file's Host, the project it is evaluated through and its solution, and the last evaluation.
    msbuild: Option<(rsharp_lsp::SharedHost, PathBuf, Option<PathBuf>)>,
    pub insight: Option<Arc<rsharp_core::msbuild::insight::ProjectInsight>>,
    inspecting: Option<Task<()>>,
    /// A XAML file's host and the project it is read against.
    pub xaml: Option<(rsharp_lsp::xaml::XamlHost, PathBuf)>,
    checking_xaml: Option<Task<()>>,
    /// A XAML file's problems as the host last found them, and the last build's with the line each is on: the build's
    /// is shown where the host finds nothing on its line, and goes once that line is edited.
    xaml_live: Vec<Diagnostic>,
    xaml_built: Vec<(usize, Diagnostic)>,
    xaml_changes: Option<Task<()>>,
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

/// Workspace buffers across all editors, including detached windows, each file once.
pub fn in_folders(folders: &[PathBuf], cx: &App) -> Vec<Entity<Buffer>> {
    cx.try_global::<Buffers>()
        .into_iter()
        .flat_map(|buffers| buffers.0.iter())
        .filter(|(path, _)| folders.iter().any(|folder| path.starts_with(folder)))
        .filter_map(|(_, buffer)| buffer.upgrade())
        .collect()
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
            git_base: None,
            git_hunks: Arc::new(Vec::new()),
            breakpoints: Vec::new(),
            execution_line: None,
            inline_values: None,
            exception: None,
            git_comparing: None,
            is_csharp,
            parsing: None,
            server: None,
            checking: None,
            contexts: Vec::new(),
            context: 0,
            asking_contexts: None,
            msbuild: None,
            insight: None,
            xaml: None,
            checking_xaml: None,
            xaml_live: Vec::new(),
            xaml_built: Vec::new(),
            xaml_changes: None,
            inspecting: None,
        };
        buffer.on_disk = stamp(&buffer.path);
        buffer.reparse(cx);
        buffer
    }

    /// The file read again if another program changed it since, while nothing here is unsaved, as Rider reloads a
    /// file it sees changed; its new content kept in Local History as an External change.
    pub fn refresh_from_disk(&mut self, cx: &mut Context<Self>) {
        self.read_again(Some("External change"), cx);
    }

    /// The file read again after RSharp wrote it itself, which Local History has already kept.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.read_again(None, cx);
    }

    fn read_again(&mut self, external: Option<&'static str>, cx: &mut Context<Self>) {
        let now = stamp(&self.path);
        if now.is_none() || now == self.on_disk {
            return;
        }
        let Ok(bytes) = std::fs::read(&self.path) else { return };
        // shortcut: unsaved edits win over a change from elsewhere, which Local History keeps so the next save loses
        // nothing for good; ask which to keep, as Rider's File Cache Conflict does, once someone meets it.
        if self.is_modified() {
            if external.is_some() {
                crate::history_window::record(self.path.clone(), bytes, external, cx);
            }
            return;
        }
        if external.is_some() {
            crate::history_window::record(self.path.clone(), bytes.clone(), external, cx);
        }
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
        if !self.xaml_built.is_empty() {
            self.xaml_live.retain(|d| changes.iter().all(|c| d.range.end <= c.start.offset));
            for c in &changes {
                let delta = c.new_end.offset as isize - c.old_end.offset as isize;
                self.xaml_built.retain_mut(|(line, d)| {
                    if *line < c.start.line {
                        return true;
                    }
                    if *line <= c.old_end.line {
                        return false;
                    }
                    *line = *line + c.new_end.line - c.old_end.line;
                    d.range = d.range.start.saturating_add_signed(delta)..d.range.end.saturating_add_signed(delta);
                    true
                });
            }
            self.show_xaml();
        }
        // The stop's marks go with their code, as Rider's execution point and inline values do. They are the buffer's,
        // shared by every window showing it, so they move here once rather than once per window told of the edit.
        let follow = |line: usize| rsharp_text::map_line(&changes, line);
        self.execution_line = self.execution_line.map(follow);
        self.inline_values = self.inline_values.take().map(|(line, text)| (follow(line), text));
        self.exception = self.exception.take().map(|(line, text)| (follow(line), text));
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
        self.compare_git(cx);
        self.reparse(cx);
        self.check(Duration::from_millis(400), cx);
        self.inspect(Duration::from_millis(500), cx);
        self.check_xaml(Duration::from_millis(300), cx);
        cx.emit(BufferEvent::Edited { changes, by });
        cx.notify();
    }

    /// A file's breakpoints as the folder has them: the gutter shows a dot at each.
    pub fn set_breakpoints(&mut self, lines: Vec<BreakpointMark>, cx: &mut Context<Self>) {
        if self.breakpoints != lines {
            self.breakpoints = lines;
            cx.notify();
        }
    }

    /// The frame's values as the stopped line shows them, or none.
    pub fn set_inline_values(&mut self, values: Option<(usize, String)>, cx: &mut Context<Self>) {
        if self.inline_values != values {
            self.inline_values = values;
            cx.notify();
        }
    }

    /// The exception the stop is on, named at its line; none when the stop is not at one.
    pub fn set_exception(&mut self, exception: Option<(usize, String)>, cx: &mut Context<Self>) {
        if self.exception != exception {
            self.exception = exception;
            cx.notify();
        }
    }

    /// The line the program is stopped at here, or none.
    pub fn set_execution_line(&mut self, line: Option<usize>, cx: &mut Context<Self>) {
        if self.execution_line != line {
            self.execution_line = line;
            cx.notify();
        }
    }

    pub fn set_git_base(&mut self, baseline: Option<String>, cx: &mut Context<Self>) {
        self.set_git_baseline(baseline.map(Arc::from), cx);
    }

    pub fn set_git_baseline(&mut self, baseline: Option<Arc<str>>, cx: &mut Context<Self>) {
        if match (&self.git_base, &baseline) {
            (None, None) => true,
            (Some(old), Some(new)) => Arc::ptr_eq(old, new),
            _ => false,
        } {
            return;
        }
        self.git_base = baseline;
        self.compare_git(cx);
    }

    fn compare_git(&mut self, cx: &mut Context<Self>) {
        self.git_comparing = None;
        self.git_hunks = Arc::new(Vec::new());
        let Some(before) = self.git_base.clone() else {
            cx.notify();
            return;
        };
        // shortcut: no gutter change bars above 4 MiB, where a generated file would be copied and diffed whole on
        // every pause in typing and its bars read by nobody; diff one by its visible lines once someone edits one.
        if self.document.text().len_bytes() > 4 << 20 || before.len() > 4 << 20 {
            cx.notify();
            return;
        }
        let after = self.document.text().clone();
        let version = self.document.version();
        self.git_comparing = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(120)).await;
            let hunks = cx
                .background_executor()
                .spawn(async move { rsharp_text::diff_hunks(&before, &after.to_string()) })
                .await;
            let _ = this.update(cx, |b, cx| {
                if b.document.version() == version {
                    b.git_hunks = Arc::new(hunks);
                    cx.notify();
                }
            });
        }));
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
        self.ask_contexts(cx);
    }

    /// The file renamed on disk to `to`: known by its new path from now, the server told it is the new file, and every
    /// editor of it told. Refused, with nothing changed, when the rename fails.
    pub fn move_to(&mut self, to: PathBuf, cx: &mut Context<Self>) -> std::io::Result<()> {
        std::fs::rename(&self.path, &to)?;
        self.moved(to, cx);
        Ok(())
    }

    /// The file is at `to` now, moved with the folder it was in: the buffer goes by its new path, its text kept.
    pub fn moved(&mut self, to: PathBuf, cx: &mut Context<Self>) {
        let server = self.server.as_ref().map(|(s, _)| s.clone());
        self.detach_server();
        let me = cx.entity().downgrade();
        let registry = &mut cx.default_global::<Buffers>().0;
        registry.remove(&self.path);
        registry.insert(to.clone(), me);
        self.path = to.clone();
        self.on_disk = stamp(&self.path);
        if let Some(server) = server {
            self.attach_server(server, cx);
        }
        cx.emit(BufferEvent::Moved(to));
        cx.notify();
    }

    /// The server the file is open in, and its URI there.
    pub fn server(&self) -> Option<(Arc<LanguageServer>, String)> {
        self.server.clone()
    }

    /// Edits made from outside any editor (a rename, a context action), as one step Undo takes back; every editor's
    /// carets follow.
    pub fn apply_edits(&mut self, edits: Vec<Edit>, cx: &mut Context<Self>) {
        let selections = self.document.selections().to_vec();
        let changes = self.document.apply(EditPlan { edits, selections, ..Default::default() });
        let me = cx.entity_id();
        self.changed(changes, me, cx);
    }

    /// The projects Roslyn compiles the file in, asked as it is opened there; the one chosen before kept by its label.
    fn ask_contexts(&mut self, cx: &mut Context<Self>) {
        let Some((server, uri)) = self.server.clone() else { return };
        let answer = server
            .ask("textDocument/_vs_getProjectContexts", rsharp_lsp::contexts_request(&uri, self.document.version()));
        self.asking_contexts = Some(cx.spawn(async move |this, cx| {
            let Ok(answer) = answer.await else { return };
            let (contexts, default) = rsharp_lsp::project_contexts(&answer);
            let _ = this.update(cx, |b, cx| {
                let chosen = b.contexts.get(b.context).map(|c| c["_vs_label"].clone());
                b.context =
                    chosen.and_then(|label| contexts.iter().position(|c| c["_vs_label"] == label)).unwrap_or(default);
                b.contexts = contexts;
                cx.notify();
            });
        }));
    }

    /// The target framework switcher's choice: the file's requests about that project from now, its problems asked again
    /// and its editors told.
    pub fn choose_context(&mut self, index: usize, cx: &mut Context<Self>) {
        let (Some(context), Some((_, uri))) = (self.contexts.get(index).cloned(), self.server.clone()) else { return };
        self.context = index;
        rsharp_lsp::set_project_context(&uri, Some(context));
        self.check(Duration::ZERO, cx);
        cx.emit(BufferEvent::ContextChanged);
        cx.notify();
    }

    /// The chosen project context's label (its target framework), when the file is in more than one.
    pub fn context_label(&self) -> Option<String> {
        (self.contexts.len() > 1)
            .then(|| self.contexts[self.context]["_vs_label"].as_str().unwrap_or_default().to_string())
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
            let answer = server.ask("textDocument/diagnostic", json!({ "textDocument": rsharp_lsp::document(&uri) }));
            let Ok(report) = answer.await else { return };
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

    /// An MSBuild file's code help: evaluated through `project` in the folder's Host, now and once typing pauses.
    pub fn attach_msbuild(
        &mut self,
        host: rsharp_lsp::SharedHost,
        project: PathBuf,
        solution: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.msbuild = Some((host, project, solution));
        self.inspect(Duration::ZERO, cx);
    }

    /// The project evaluated on the text as typed, every open MSBuild file's unsaved text in place of the disk's; its
    /// errors in this file become the file's problems.
    pub fn inspect(&mut self, after: Duration, cx: &mut Context<Self>) {
        let Some((host, project, solution)) = self.msbuild.clone() else { return };
        let version = self.document.version();
        let mut texts = HashMap::new();
        if self.is_modified() {
            texts.insert(self.path.clone(), self.document.text().to_string());
        }
        for (path, other) in cx.try_global::<Buffers>().map(|b| b.0.clone()).unwrap_or_default() {
            if path != self.path
                && rsharp_core::msbuild::is_msbuild(&path)
                && let Some(other) = other.upgrade().filter(|o| o.read(cx).is_modified())
            {
                texts.insert(path, other.read(cx).document.text().to_string());
            }
        }
        self.inspecting = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(after).await;
            let read = cx
                .background_executor()
                .spawn(async move { host.inspect(&project, solution.as_deref(), &texts) })
                .await;
            let insight = match read {
                Ok(insight) => insight,
                Err(failed) => {
                    log::info!("MSBuild code help: {failed}");
                    return;
                }
            };
            let _ = this.update(cx, |b, cx| {
                if b.document.version() != version {
                    return;
                }
                let me = rsharp_core::paths::canonical(&b.path);
                let text = b.document.text().clone();
                b.diagnostics = insight
                    .diagnostics
                    .iter()
                    .filter(|d| rsharp_core::paths::canonical(&d.file) == me)
                    .map(|d| {
                        let line = d.line.saturating_sub(1).min(text.len_lines().saturating_sub(1));
                        let start = text.line_to_byte(line);
                        let end = line_end(&text, line);
                        let at = text.char_to_byte(
                            (text.byte_to_char(start) + d.column.saturating_sub(1)).min(text.byte_to_char(end)),
                        );
                        Diagnostic {
                            range: at..end.max(at + 1).min(text.len_bytes()),
                            error: d.error,
                            message: d.message.clone(),
                        }
                    })
                    .collect();
                b.insight = Some(Arc::new(insight));
                cx.notify();
            });
        }));
    }

    /// A XAML file's code help: read against `project` in the XAML host, now and once typing pauses.
    pub fn attach_xaml(&mut self, host: rsharp_lsp::xaml::XamlHost, project: PathBuf, cx: &mut Context<Self>) {
        if self.xaml.is_none() {
            cx.on_release(|b, cx| {
                if let Some((host, _)) = b.xaml.take() {
                    let path = b.path.clone();
                    cx.background_executor()
                        .spawn(async move {
                            if let Err(failed) = host.closed(&path) {
                                log::info!("XAML file closed: {failed}");
                            }
                        })
                        .detach();
                }
            })
            .detach();
        }
        let mut changes = host.subscribe();
        self.xaml = Some((host, project));
        self.xaml_changes = Some(cx.spawn(async move |this, cx| {
            while let Some(except) = changes.next().await {
                let _ = this.update(cx, |b, cx| {
                    if Some(&b.path) != except.as_ref() {
                        b.check_xaml(Duration::ZERO, cx);
                    }
                });
            }
        }));
        self.check_xaml(Duration::ZERO, cx);
    }

    /// The file checked as typed; what the host finds becomes the file's problems.
    fn check_xaml(&mut self, after: Duration, cx: &mut Context<Self>) {
        let Some((host, project)) = self.xaml.clone() else { return };
        let (version, path, text) = (self.document.version(), self.path.clone(), self.document.text().to_string());
        self.checking_xaml = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(after).await;
            let found = cx.background_executor().spawn(async move { host.check(&project, &path, &text) }).await;
            let problems = match found {
                Ok(problems) => problems.unwrap_or_default(),
                Err(failed) => {
                    log::info!("XAML code help: {failed}");
                    return;
                }
            };
            let _ = this.update(cx, |b, cx| {
                if b.document.version() != version {
                    return;
                }
                let at = |p: rsharp_lsp::Point| b.document.offset_of_utf16(p.line, p.character);
                b.xaml_live = problems
                    .iter()
                    .map(|p| Diagnostic {
                        range: at(p.span.start)..at(p.span.end),
                        error: p.error,
                        message: p.message.clone(),
                    })
                    .collect();
                b.show_xaml();
                cx.notify();
            });
        }));
    }

    /// The XAML compiler's problems in this file from the build that just ended, each on the name its place points at
    /// (a tag's or an attribute's), else the rest of its line.
    pub fn built(&mut self, problems: &[rsharp_core::build::Problem], cx: &mut Context<Self>) {
        let me = rsharp_core::paths::canonical(&self.path);
        let text = self.document.text().clone();
        self.xaml_built = problems
            .iter()
            .filter(|p| p.path.as_ref().is_some_and(|path| rsharp_core::paths::canonical(path) == me))
            .filter(|p| p.line < text.len_lines())
            .map(|p| {
                let (start, end) = (text.line_to_byte(p.line), line_end(&text, p.line));
                let line = text.byte_slice(start..end).to_string();
                let column = line.char_indices().nth(p.column).map_or(line.len(), |(i, _)| i);
                let name = column + usize::from(line[column..].starts_with('<'));
                let name = name + usize::from(line[name..].starts_with('/'));
                let length = line[name..]
                    .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '-')))
                    .unwrap_or(line.len() - name);
                let range = if length > 0 { start + name..start + name + length } else { start + column..end };
                let code = if p.code.is_empty() { String::new() } else { format!("{}: ", p.code) };
                let message = Diagnostic {
                    range,
                    error: p.severity == rsharp_core::build::Severity::Error,
                    message: code + compiler_message(&p.message),
                };
                (p.line, message)
            })
            .collect();
        self.show_xaml();
        cx.notify();
    }

    /// The host's problems, and the build's on the lines without a host error.
    fn show_xaml(&mut self) {
        let text = self.document.text();
        let line = |d: &Diagnostic| text.byte_to_line(d.range.start.min(text.len_bytes()));
        let lines: HashSet<usize> = self.xaml_live.iter().filter(|d| d.error).map(line).collect();
        let built = self.xaml_built.iter().filter(|(l, _)| !lines.contains(l)).map(|(_, d)| d.clone());
        self.diagnostics = self.xaml_live.iter().cloned().chain(built).collect();
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

/// A compiler's message without the place it repeats at its end: Avalonia's "Line 8, position 6.", WPF's MarkupCompile's
/// "Line 8 Position 6.".
fn compiler_message(message: &str) -> &str {
    match message.rfind(" Line ") {
        Some(at) if message[at..].to_ascii_lowercase().contains(" position ") => message[..at].trim_end(),
        _ => message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsharp_core::build::{Problem, Severity};

    #[gpui::test]
    fn xaml_build_errors_on_either_closing_tag_delimiter_mark_the_name(cx: &mut gpui::TestAppContext) {
        let path = std::env::temp_dir().join("rsharp-closing-tag.axaml");
        let buffer = cx.new(|cx| Buffer::new(path.clone(), b"  </local:Window>\n", cx));
        for column in [2, 3] {
            buffer.update(cx, |b, cx| {
                b.built(
                    &[Problem {
                        severity: Severity::Error,
                        message: "Mismatched tag".into(),
                        code: "AVLN1000".into(),
                        path: Some(path.clone()),
                        project: None,
                        line: 0,
                        column,
                    }],
                    cx,
                );
                let marked = b.document.text().byte_slice(b.diagnostics[0].range.clone()).to_string();
                assert_eq!(
                    marked, "local:Window",
                    "the build error at column {column} did not mark the closing tag's name"
                );
            });
        }
    }

    #[gpui::test]
    fn xaml_live_warnings_keep_build_errors_while_live_errors_take_precedence(cx: &mut gpui::TestAppContext) {
        let path = std::env::temp_dir().join("rsharp-live-warning.axaml");
        let buffer = cx.new(|cx| Buffer::new(path.clone(), b"<Window />\n", cx));
        for error in [true, false] {
            buffer.update(cx, |b, cx| {
                b.xaml_live = vec![Diagnostic { range: 1..7, error, message: "Live diagnostic".into() }];
                b.built(
                    &[Problem {
                        severity: Severity::Error,
                        message: "Build error".into(),
                        code: "AVLN1000".into(),
                        path: Some(path.clone()),
                        project: None,
                        line: 0,
                        column: 0,
                    }],
                    cx,
                );
                let messages: Vec<_> = b.diagnostics.iter().map(|d| d.message.as_str()).collect();
                let expected =
                    if error { vec!["Live diagnostic"] } else { vec!["Live diagnostic", "AVLN1000: Build error"] };
                assert_eq!(
                    messages, expected,
                    "a live diagnostic with error={error} gave the build error the wrong precedence"
                );
            });
        }
    }
}
