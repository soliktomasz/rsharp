//! Rider's TODO window: every TODO and FIXME comment in the folder's files, by file, found off the UI thread with each
//! file's own grammar and found again for the files the watcher says changed.

use super::*;

/// A TODO as listed: its file, its line and column (from 0), and the comment from its word on.
#[derive(Clone, Debug)]
pub struct TodoEntry {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub text: String,
}

/// The most files the folder's walk looks at, as Go to File's.
const MOST_FILES: usize = 100_000;

/// A file's TODOs, from its text on disk; none for a file no grammar here reads.
fn todos_in(path: &Path) -> Vec<TodoEntry> {
    let Some(grammar) = rsharp_languages::Grammar::for_path(path) else { return Vec::new() };
    let Ok(source) = std::fs::read_to_string(path) else { return Vec::new() };
    let text = rsharp_text::Rope::from_str(&source);
    let tree = rsharp_languages::parse(grammar, &text, None);
    rsharp_core::todo::scan(&source, &rsharp_languages::comments_in(&tree))
        .into_iter()
        .map(|todo| {
            let line = text.byte_to_line(todo.at);
            TodoEntry { path: path.to_path_buf(), line, column: todo.at - text.line_to_byte(line), text: todo.text }
        })
        .collect()
}

impl MainWindow {
    /// Finds the TODOs again: of the whole folder, or of the files given, replacing what they listed.
    pub(super) fn scan_todos(&mut self, files: Option<Vec<PathBuf>>, cx: &mut Context<Self>) {
        let Some(folder) = self.folder().map(Path::to_path_buf) else { return };
        let whole = files.is_none();
        self.todo_scanned = true;
        let task = cx.background_executor().spawn(async move {
            let files = files.unwrap_or_else(|| crate::workspace::all_files(&folder, MOST_FILES));
            let found: Vec<TodoEntry> = files.iter().flat_map(|path| todos_in(path)).collect();
            (files, found)
        });
        let scan = cx.spawn(async move |this, cx| {
            let (files, found) = task.await;
            let _ = this.update(cx, |w, cx| {
                if whole {
                    w.todos.clear();
                } else {
                    w.todos.retain(|t| !files.contains(&t.path));
                }
                w.todos.extend(found);
                w.todos.sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
                cx.notify();
            });
        });
        // A rescan of a few files runs beside the folder's walk rather than cancelling it.
        if whole { self.todo_task = Some(scan) } else { scan.detach() }
    }

    /// The watcher's burst: the changed files found again, once the window has been asked for TODOs.
    pub(super) fn todos_changed(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if !self.todo_scanned {
            return;
        }
        let files: Vec<PathBuf> = paths
            .iter()
            .filter(|p| rsharp_languages::Grammar::for_path(p).is_some() || self.todos.iter().any(|t| &t.path == *p))
            .cloned()
            .collect();
        if !files.is_empty() {
            self.scan_todos(Some(files), cx);
        }
    }

    pub(super) fn clear_todos(&mut self) {
        self.todos.clear();
        self.todo_task = None;
        self.todo_scanned = false;
    }

    pub(super) fn todo_actions<T: InteractiveElement + FluentBuilder>(&self, d: T, cx: &mut Context<Self>) -> T {
        d.when(self.workspace.is_some(), |d| {
            d.on_action(cx.listener(|w, _: &ShowTodo, window, cx| {
                if !w.dock.is_open(Tool::Todo) {
                    w.toggle_tool(Tool::Todo, window, cx);
                }
            }))
        })
    }

    /// The TODO window's rows as a person reads them: the file and line, then the comment.
    pub fn todo_rows(&self) -> Vec<String> {
        self.todos
            .iter()
            .map(|t| format!("{}:{}  {}", t.path.file_name().unwrap_or_default().to_string_lossy(), t.line + 1, t.text))
            .collect()
    }

    /// The folder's TODOs found the first time the window is shown.
    pub(super) fn todo_shown(&mut self, cx: &mut Context<Self>) {
        if !self.todo_scanned && self.workspace.is_some() {
            self.scan_todos(None, cx);
        }
    }

    pub(super) fn todo_content(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.todos.is_empty() {
            let said = if self.todo_task.is_some() { "Looking for TODOs…" } else { "No TODO or FIXME comments" };
            return empty_text(said).track_focus(&self.todo_focus).into_any_element();
        }
        let folder = self.folder().map(Path::to_path_buf);
        div()
            .id("todos")
            .track_focus(&self.todo_focus)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(6.))
            .children(self.todos.iter().enumerate().map(|(i, todo)| {
                let (path, line, column) = (todo.path.clone(), todo.line, todo.column);
                let shown = folder
                    .as_deref()
                    .and_then(|f| path.strip_prefix(f).ok())
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                div()
                    .id(("todo", i))
                    .debug_selector(move || format!("todo: {i}"))
                    .h(px(ROW_HEIGHT))
                    .px(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .hover(|d| d.bg(theme::row_hover()))
                    .child(icon(file_icon(&shown), 16.))
                    .child(div().flex_none().text_color(theme::text_secondary()).child(format!("{shown}:{}", line + 1)))
                    .child(
                        div()
                            .truncate()
                            .text_color(theme::syntax(rsharp_languages::Kind::Comment))
                            .child(todo.text.clone()),
                    )
                    .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| {
                        w.open_file(&path, Some((line, column)), window, cx)
                    }))
            }))
            .into_any_element()
    }
}
