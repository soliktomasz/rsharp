//! The code editor: one open file's document, drawn line by line for only the lines on screen, coloured from
//! its syntax tree (parsed again off the UI thread after each edit) and kept in step with Roslyn.

use crate::theme;
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, ScrollWheelEvent, ShapedLine, Style, Task, TextRun,
    UTF16Selection, UnderlineStyle, Window, actions, div, fill, font, point, prelude::*, px, relative, size,
};
use rsharp_languages::{Highlight, edit as edit_tree, highlights, parse};
use rsharp_lsp::LanguageServer;
use rsharp_text::*;
use serde_json::json;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tree_sitter::Tree;

actions!(
    editor,
    [
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        LineStart,
        LineEnd,
        SelectLineStart,
        SelectLineEnd,
        DocumentStart,
        DocumentEnd,
        SelectDocumentStart,
        SelectDocumentEnd,
        PageUp,
        PageDown,
        SelectAll,
        Backspace,
        Delete,
        Newline,
        Tab,
        Undo,
        Redo,
        Copy,
        Cut,
        Paste,
        Save,
        SelectNextOccurrence,
        UnselectOccurrence,
        SelectAllOccurrences,
        CloneCaretAbove,
        CloneCaretBelow,
        CaretPerLine,
        ToggleColumnMode,
        Escape,
    ]
);

const TEXT_PAD: f32 = 16.;

/// A line and a character column, which may lie past the line's end.
type Place = (usize, usize);

/// A problem the server found, where it is now.
#[derive(Clone)]
pub struct Diagnostic {
    pub range: Range<usize>,
    pub error: bool,
    pub message: String,
}

pub struct CodeEditor {
    pub path: PathBuf,
    pub document: Document,
    pub diagnostics: Vec<Diagnostic>,
    focus: FocusHandle,
    is_csharp: bool,
    tree: Option<Tree>,
    parsing: Option<Task<()>>,
    server: Option<(Arc<LanguageServer>, String)>,
    checking: Option<Task<()>>,
    scroll_y: Pixels,
    autoscroll: bool,
    /// Each caret's column for a run of Up and Down, so a short line on the way doesn't pull it in.
    goal_columns: Option<Vec<usize>>,
    marked: Option<Range<usize>>,
    selecting: bool,
    /// Where a column drag began, while it lasts.
    column_drag: Option<Place>,
    /// The column selection last made, from its start to its head, and the carets it made: Shift and the arrows
    /// go on with it while those carets are untouched.
    block: Option<(Place, Place, Vec<Selection>)>,
    /// Column Selection Mode: a drag and Shift with the arrows select a column.
    pub column_mode: bool,
    layout: Option<Layout>,
    /// The line height in force, from the settings, as the last frame used it.
    line_height: Pixels,
}

/// Where the last frame put things, for the mouse and the input method to find their way back.
struct Layout {
    text_origin: Point<Pixels>,
    char_width: Pixels,
    viewport: Bounds<Pixels>,
    first_line: usize,
    lines: Vec<ShapedLine>,
}

impl CodeEditor {
    pub fn new(path: PathBuf, bytes: &[u8], cx: &mut Context<Self>) -> Self {
        // shortcut: bytes that are not UTF-8 are read lossily; Passenger reads them in the file's 8-bit charset.
        let document = Document::new(&String::from_utf8_lossy(bytes));
        let is_csharp = path.extension().is_some_and(|e| e == "cs" || e == "csx");
        let mut editor = Self {
            path,
            document,
            diagnostics: Vec::new(),
            focus: cx.focus_handle(),
            is_csharp,
            tree: None,
            parsing: None,
            server: None,
            checking: None,
            scroll_y: px(0.),
            autoscroll: false,
            goal_columns: None,
            marked: None,
            selecting: false,
            column_drag: None,
            block: None,
            column_mode: false,
            layout: None,
            line_height: px(crate::settings::settings(cx).line_height()),
        };
        cx.observe_global::<crate::settings::Settings>(|_, cx| cx.notify()).detach();
        editor.reparse(cx);
        editor
    }

    pub fn caret(&self) -> Position {
        self.document.position(self.primary().head)
    }

    pub fn place_caret(&mut self, line: usize, column: usize, cx: &mut Context<Self>) {
        let offset = offset_at(self.document.text(), line, column);
        self.document.set_selections(vec![Selection::caret(offset)]);
        self.autoscroll = true;
        cx.notify();
    }

    fn primary(&self) -> Selection {
        *self.document.selections().last().expect("a document always has a caret")
    }

    /// Opens the file in the server and keeps it there, edit by edit.
    pub fn attach_server(&mut self, server: Arc<LanguageServer>, cx: &mut Context<Self>) {
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
            let _ = this.update(cx, |editor, cx| {
                if editor.document.version() != version {
                    return;
                }
                let at = |p: &serde_json::Value| {
                    editor.document.offset_of_utf16(
                        p["line"].as_u64().unwrap_or(0) as usize,
                        p["character"].as_u64().unwrap_or(0) as usize,
                    )
                };
                editor.diagnostics = report["items"]
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

    fn apply(&mut self, plan: EditPlan, cx: &mut Context<Self>) {
        let changes = self.document.apply(plan);
        self.changed(changes, cx);
    }

    /// What every change goes through: the tree told, the server told, a parse and a check asked for.
    fn changed(&mut self, changes: Vec<Change>, cx: &mut Context<Self>) {
        self.goal_columns = None;
        self.autoscroll = true;
        if !changes.is_empty() {
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
        }
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
            let tree = cx.background_executor().spawn(async move { parse(&text, old.as_ref()) }).await;
            let _ = this.update(cx, |editor, cx| {
                if editor.document.version() == version {
                    editor.tree = Some(tree);
                    cx.notify();
                }
            });
        }));
    }

    fn move_carets(&mut self, extend: bool, cx: &mut Context<Self>, mut to: impl FnMut(&Rope, &Selection) -> usize) {
        let text = self.document.text().clone();
        let moved: Vec<Selection> = self
            .document
            .selections()
            .iter()
            .map(|s| {
                let head = to(&text, s);
                Selection { anchor: if extend { s.anchor } else { head }, head }
            })
            .collect();
        self.document.set_selections(merge_overlapping(moved));
        self.autoscroll = true;
        cx.notify();
    }

    fn horizontal(
        &mut self,
        extend: bool,
        cx: &mut Context<Self>,
        to: fn(&Rope, usize) -> usize,
        collapse_to_start: bool,
    ) {
        self.goal_columns = None;
        self.move_carets(extend, cx, |text, s| match (extend || s.is_empty(), collapse_to_start) {
            (true, _) => to(text, s.head),
            (false, true) => s.start(),
            (false, false) => s.end(),
        });
    }

    fn vertical(&mut self, lines: isize, extend: bool, cx: &mut Context<Self>) {
        let text = self.document.text();
        let goals = match self.goal_columns.take() {
            Some(goals) if goals.len() == self.document.selections().len() => goals,
            _ => self.document.selections().iter().map(|s| column_of(text, s.head)).collect(),
        };
        let mut index = 0;
        self.move_carets(extend, cx, |text, s| {
            index += 1;
            vertical(text, s.head, goals[index - 1], lines)
        });
        // Carets that met are one now, and their goals no longer line up with them.
        if self.document.selections().len() == goals.len() {
            self.goal_columns = Some(goals);
        }
    }

    fn place_of(&self, offset: usize) -> Place {
        let text = self.document.text();
        (text.byte_to_line(offset), column_of(text, offset))
    }

    fn set_block(&mut self, start: Place, head: Place, cx: &mut Context<Self>) {
        let carets = column_selection(self.document.text(), start, head);
        self.set_carets(carets.clone(), cx);
        self.block = Some((start, head, carets));
    }

    /// Shift and an arrow in Column Selection Mode: the column selection's head a line or a column further.
    fn extend_block(&mut self, lines: isize, columns: isize, cx: &mut Context<Self>) {
        let (start, head) = match &self.block {
            Some((start, head, carets)) if carets.as_slice() == self.document.selections() => (*start, *head),
            _ => (self.place_of(self.primary().anchor), self.place_of(self.primary().head)),
        };
        let last = self.document.text().len_lines() as isize - 1;
        let head = ((head.0 as isize + lines).clamp(0, last) as usize, (head.1 as isize + columns).max(0) as usize);
        self.set_block(start, head, cx);
    }

    fn set_carets(&mut self, selections: Vec<Selection>, cx: &mut Context<Self>) {
        self.document.set_selections(selections);
        self.goal_columns = None;
        self.autoscroll = true;
        cx.notify();
    }

    /// Escape: the extra carets go, then the selection; with neither, the key is the window's.
    fn escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        let primary = self.primary();
        if self.document.selections().len() > 1 {
            self.set_carets(vec![primary], cx);
        } else if !primary.is_empty() {
            self.set_carets(vec![Selection::caret(primary.head)], cx);
        } else {
            cx.propagate();
        }
    }

    fn page(&self) -> isize {
        self.layout.as_ref().map_or(20, |l| (l.viewport.size.height / self.line_height) as isize - 1).max(1)
    }

    fn selected_text(&self) -> String {
        let text = self.document.text();
        let mut selections = self.document.selections().to_vec();
        selections.sort_by_key(Selection::start);
        selections
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| text.byte_slice(s.range()).to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn offset_for_point(&self, position: Point<Pixels>) -> usize {
        let Some(layout) = &self.layout else { return 0 };
        let text = self.document.text();
        let line = ((position.y - layout.text_origin.y) / self.line_height).floor().max(0.) as usize;
        if line >= text.len_lines() {
            return text.len_bytes();
        }
        let start = text.line_to_byte(line);
        match layout.lines.get(line.wrapping_sub(layout.first_line)) {
            Some(shaped) => start + shaped.closest_index_for_x(position.x - layout.text_origin.x),
            None => start,
        }
    }

    /// The line and character column under the pointer, past the line's end as far as the pointer is.
    fn place_for_point(&self, position: Point<Pixels>) -> (usize, usize) {
        let Some(layout) = &self.layout else { return (0, 0) };
        let line = ((position.y - layout.text_origin.y) / self.line_height).floor().max(0.) as usize;
        let column = ((position.x - layout.text_origin.x) / layout.char_width).round().max(0.) as usize;
        (line, column)
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        let offset = self.offset_for_point(event.position);
        let text = self.document.text().clone();
        let m = event.modifiers;
        // Rider's macOS keymap adds a caret with Alt+Shift+click, its Visual Studio keymap with Ctrl+Alt+click.
        let adds_caret = if cfg!(target_os = "macos") { m.alt && m.shift } else { m.control && m.alt };
        if adds_caret {
            let mut carets = self.document.selections().to_vec();
            match carets.iter().position(|s| s.range().contains(&offset) || s.head == offset) {
                Some(at) if carets.len() > 1 => drop(carets.remove(at)),
                Some(_) => {}
                None if carets.len() < MAX_CARETS => carets.push(Selection::caret(offset)),
                None => {}
            }
            self.document.set_selections(carets);
            self.goal_columns = None;
            cx.notify();
            return;
        }
        let place = self.place_for_point(event.position);
        // Shift+Alt+click, the Visual Studio keymap's, selects a column from the caret to the click.
        if m.alt && m.shift {
            let start = self.place_of(self.primary().anchor);
            self.column_drag = Some(start);
            self.set_block(start, place, cx);
            self.selecting = true;
            return;
        }
        if m.alt || (self.column_mode && event.click_count == 1 && !m.shift) {
            self.column_drag = Some(place);
            self.set_block(place, place, cx);
            self.selecting = true;
            return;
        }
        let selection = match event.click_count {
            2 => Selection { anchor: word_left(&text, next_char(&text, offset)), head: word_right(&text, offset) },
            n if n >= 3 => {
                let line = text.byte_to_line(offset);
                let end = if line + 1 < text.len_lines() { text.line_to_byte(line + 1) } else { text.len_bytes() };
                Selection { anchor: text.line_to_byte(line), head: end }
            }
            _ if event.modifiers.shift => Selection { anchor: self.primary().anchor, head: offset },
            _ => Selection::caret(offset),
        };
        self.document.set_selections(vec![selection]);
        self.goal_columns = None;
        self.selecting = true;
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let (true, Some(start)) = (self.selecting, self.column_drag)
            && event.pressed_button == Some(MouseButton::Left)
        {
            self.set_block(start, self.place_for_point(event.position), cx);
        } else if self.selecting && event.pressed_button == Some(MouseButton::Left) {
            let anchor = self.primary().anchor;
            self.document.set_selections(vec![Selection { anchor, head: self.offset_for_point(event.position) }]);
            self.autoscroll = true;
            cx.notify();
        }
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(self.line_height);
        self.scroll_y = (self.scroll_y - delta.y).clamp(px(0.), self.max_scroll());
        cx.notify();
    }

    fn max_scroll(&self) -> Pixels {
        let content = self.line_height * self.document.text().len_lines() as f32;
        let viewport = self.layout.as_ref().map_or(px(0.), |l| l.viewport.size.height);
        (content - viewport + self.line_height * 4.).max(px(0.))
    }

    fn utf16_to_offset(&self, utf16: usize) -> usize {
        let text = self.document.text();
        text.char_to_byte(text.utf16_cu_to_char(utf16.min(text.len_utf16_cu())))
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let text = self.document.text();
        text.char_to_utf16_cu(text.byte_to_char(offset))
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.utf16_to_offset(range.start)..self.utf16_to_offset(range.end)
    }
}

macro_rules! moves {
    ($($name:ident => $body:expr;)*) => {
        impl CodeEditor { $( #[allow(non_snake_case)] fn $name(&mut self, _: &$name, _: &mut Window, cx: &mut Context<Self>) { let f: &dyn Fn(&mut Self, &mut Context<Self>) = &$body; f(self, cx) } )* }
        fn bind_moves(div: gpui::Stateful<gpui::Div>, cx: &mut Context<CodeEditor>) -> gpui::Stateful<gpui::Div> {
            div $( .on_action(cx.listener(CodeEditor::$name)) )*
        }
    };
}

moves! {
    Left => |e, cx| e.horizontal(false, cx, previous_char, true);
    Right => |e, cx| e.horizontal(false, cx, next_char, false);
    SelectLeft => |e, cx| if e.column_mode { e.extend_block(0, -1, cx) } else { e.horizontal(true, cx, previous_char, true) };
    SelectRight => |e, cx| if e.column_mode { e.extend_block(0, 1, cx) } else { e.horizontal(true, cx, next_char, false) };
    WordLeft => |e, cx| e.horizontal(false, cx, word_left, true);
    WordRight => |e, cx| e.horizontal(false, cx, word_right, false);
    SelectWordLeft => |e, cx| e.horizontal(true, cx, word_left, true);
    SelectWordRight => |e, cx| e.horizontal(true, cx, word_right, false);
    LineStart => |e, cx| e.horizontal(false, cx, smart_home, true);
    LineEnd => |e, cx| e.horizontal(false, cx, end_of_line, false);
    SelectLineStart => |e, cx| e.horizontal(true, cx, smart_home, true);
    SelectLineEnd => |e, cx| e.horizontal(true, cx, end_of_line, false);
    DocumentStart => |e, cx| e.horizontal(false, cx, |_, _| 0, true);
    DocumentEnd => |e, cx| e.horizontal(false, cx, |t, _| t.len_bytes(), false);
    SelectDocumentStart => |e, cx| e.horizontal(true, cx, |_, _| 0, true);
    SelectDocumentEnd => |e, cx| e.horizontal(true, cx, |t, _| t.len_bytes(), false);
    Up => |e, cx| e.vertical(-1, false, cx);
    Down => |e, cx| e.vertical(1, false, cx);
    SelectUp => |e, cx| if e.column_mode { e.extend_block(-1, 0, cx) } else { e.vertical(-1, true, cx) };
    SelectDown => |e, cx| if e.column_mode { e.extend_block(1, 0, cx) } else { e.vertical(1, true, cx) };
    PageUp => |e, cx| { let p = e.page(); e.vertical(-p, false, cx) };
    PageDown => |e, cx| { let p = e.page(); e.vertical(p, false, cx) };
    SelectAll => |e, cx| { let len = e.document.text().len_bytes(); e.document.set_selections(vec![Selection { anchor: 0, head: len }]); cx.notify() };
    Backspace => |e, cx| { let plan = backspace(e.document.text(), e.document.selections()); e.apply(plan, cx) };
    Delete => |e, cx| { let plan = delete(e.document.text(), e.document.selections()); e.apply(plan, cx) };
    Newline => |e, cx| { let plan = newline(e.document.text(), e.document.selections()); e.apply(plan, cx) };
    Tab => |e, cx| { let plan = tab(e.document.text(), e.document.selections()); e.apply(plan, cx) };
    Undo => |e, cx| { if let Some(changes) = e.document.undo() { e.changed(changes, cx) } };
    Redo => |e, cx| { if let Some(changes) = e.document.redo() { e.changed(changes, cx) } };
    Copy => |e, cx| { let text = e.selected_text(); if !text.is_empty() { cx.write_to_clipboard(ClipboardItem::new_string(text)) } };
    Cut => |e, cx| {
        let text = e.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            let plan = insert(e.document.text(), e.document.selections(), "");
            e.apply(plan, cx);
        }
    };
    Paste => |e, cx| {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = text.replace("\r\n", "\n");
            let lines: Vec<&str> = text.split('\n').collect();
            let carets = e.document.selections();
            let plan = if carets.len() > 1 && lines.len() == carets.len() {
                insert_each(carets, &lines)
            } else {
                insert(e.document.text(), carets, &text)
            };
            e.apply(plan, cx);
        }
    };
    SelectNextOccurrence => |e, cx| { let next = select_next_occurrence(e.document.text(), e.document.selections()); e.set_carets(next, cx) };
    UnselectOccurrence => |e, cx| { let next = unselect_last(e.document.selections()); e.set_carets(next, cx) };
    SelectAllOccurrences => |e, cx| { let next = select_all_occurrences(e.document.text(), e.document.selections()); e.set_carets(next, cx) };
    CloneCaretAbove => |e, cx| { let next = clone_caret(e.document.text(), e.document.selections(), -1); e.set_carets(next, cx) };
    CaretPerLine => |e, cx| { let next = caret_per_line(e.document.text(), e.document.selections()); e.set_carets(next, cx) };
    ToggleColumnMode => |e, cx| { e.column_mode = !e.column_mode; cx.notify() };
    CloneCaretBelow => |e, cx| { let next = clone_caret(e.document.text(), e.document.selections(), 1); e.set_carets(next, cx) };
}

impl Focusable for CodeEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CodeEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let element = div()
            .id("code-editor")
            .key_context("CodeEditor")
            .track_focus(&self.focus)
            .size_full()
            .cursor(CursorStyle::IBeam)
            .bg(theme::island())
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|e, _, _, _| {
                    e.selecting = false;
                    e.column_drag = None;
                }),
            )
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll));
        bind_moves(element, cx).on_action(cx.listener(Self::escape)).child(EditorElement { editor: cx.entity() })
    }
}

impl EntityInputHandler for CodeEditor {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range);
        actual.replace(self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end));
        Some(self.document.text().byte_slice(range).to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let s = self.primary();
        Some(UTF16Selection {
            range: self.offset_to_utf16(s.start())..self.offset_to_utf16(s.end()),
            reversed: s.head < s.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.offset_to_utf16(r.start)..self.offset_to_utf16(r.end))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range.map(|r| self.range_from_utf16(&r)).or(self.marked.take());
        let plan = match range {
            Some(range) => {
                let caret = range.start + text.len();
                EditPlan {
                    edits: vec![Edit { range, text: text.to_string() }],
                    selections: vec![Selection::caret(caret)],
                    ..Default::default()
                }
            }
            None => {
                let plan = insert(self.document.text(), self.document.selections(), text);
                EditPlan { kind: EditKind::Typing, ..plan }
            }
        };
        self.apply(plan, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range =
            range.map(|r| self.range_from_utf16(&r)).or(self.marked.clone()).unwrap_or_else(|| self.primary().range());
        let start = range.start;
        let caret = match selected {
            Some(s) => start + text.chars().take(s.end).map(char::len_utf8).sum::<usize>(),
            None => start + text.len(),
        };
        self.apply(
            EditPlan {
                edits: vec![Edit { range, text: text.to_string() }],
                selections: vec![Selection::caret(caret)],
                ..Default::default()
            },
            cx,
        );
        self.marked = (!text.is_empty()).then_some(start..start + text.len());
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let offset = self.utf16_to_offset(range.start);
        let text = self.document.text();
        let line = text.byte_to_line(offset);
        let shaped = layout.lines.get(line.checked_sub(layout.first_line)?)?;
        let x = shaped.x_for_index(offset - text.line_to_byte(line));
        let top = layout.text_origin.y + self.line_height * line as f32;
        Some(Bounds::new(point(layout.text_origin.x + x, top), size(px(2.), self.line_height)))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.offset_to_utf16(self.offset_for_point(point)))
    }
}

struct EditorElement {
    editor: Entity<CodeEditor>,
}

struct Prepainted {
    lines: Vec<ShapedLine>,
    first_line: usize,
    text_origin: Point<Pixels>,
    gutter_width: Pixels,
    caret_lines: Vec<usize>,
    line_numbers: Vec<ShapedLine>,
    line_height: Pixels,
    char_width: Pixels,
}

impl IntoElement for EditorElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepainted {
        let settings = crate::settings::settings(cx).clone();
        let line_height = px(settings.line_height());
        let font_size = px(settings.editor_font_size);
        // Rider draws code without ligatures until a person turns them on.
        let code_font = gpui::Font { features: gpui::FontFeatures::disable_ligatures(), ..font(theme::CODE_FONT) };
        self.editor.update(cx, |editor, _| {
            editor.line_height = line_height;
            let text = editor.document.text();
            let digits = text.len_lines().to_string().len().max(3);
            let char_width = window
                .text_system()
                .shape_line("0".into(), font_size, &[run(1, theme::editor::text().into(), &code_font)], None)
                .width;
            let gutter_width = if settings.show_line_numbers { char_width * digits as f32 + px(32.) } else { px(16.) };
            let caret_line = text.byte_to_line(editor.primary().head);
            let mut caret_lines: Vec<usize> = editor
                .document
                .selections()
                .iter()
                .filter(|s| s.is_empty())
                .map(|s| text.byte_to_line(s.head))
                .collect();
            caret_lines.sort_unstable();
            caret_lines.dedup();

            if editor.autoscroll {
                editor.autoscroll = false;
                let caret_top = line_height * caret_line as f32;
                let margin = line_height * 2.;
                if caret_top - margin < editor.scroll_y {
                    editor.scroll_y = (caret_top - margin).max(px(0.));
                } else if caret_top + line_height + margin > editor.scroll_y + bounds.size.height {
                    editor.scroll_y = caret_top + line_height + margin - bounds.size.height;
                }
            }

            let first_line = (editor.scroll_y / line_height).floor() as usize;
            let visible = (bounds.size.height / line_height).ceil() as usize + 1;
            let last_line = (first_line + visible).min(text.len_lines());
            let byte_range = text.line_to_byte(first_line.min(text.len_lines()))..text.line_to_byte(last_line);
            let colours: Vec<Highlight> =
                editor.tree.as_ref().map(|tree| highlights(tree, text, byte_range)).unwrap_or_default();

            let mut lines = Vec::with_capacity(last_line.saturating_sub(first_line));
            let mut line_numbers = Vec::with_capacity(lines.capacity());
            for line in first_line..last_line {
                let start = text.line_to_byte(line);
                let content: String = text.byte_slice(start..line_end(text, line)).to_string();
                let runs = runs_for(&content, start, &colours, &code_font);
                lines.push(window.text_system().shape_line(content.into(), font_size, &runs, None));
                let number = (line + 1).to_string();
                let colour = if caret_lines.binary_search(&line).is_ok() {
                    theme::editor::line_number_current()
                } else {
                    theme::editor::line_number()
                };
                line_numbers.push(window.text_system().shape_line(
                    number.clone().into(),
                    font_size,
                    &[run(number.len(), colour.into(), &code_font)],
                    None,
                ));
            }
            let text_origin =
                point(bounds.origin.x + gutter_width + px(TEXT_PAD) - px(16.), bounds.origin.y - editor.scroll_y);
            if !settings.show_line_numbers {
                line_numbers.clear();
            }
            Prepainted {
                lines,
                first_line,
                text_origin,
                gutter_width,
                caret_lines,
                line_numbers,
                line_height,
                char_width,
            }
        })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        state: &mut Prepainted,
        window: &mut Window,
        cx: &mut App,
    ) {
        let line_height = state.line_height;
        let focus = self.editor.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.editor.clone()), cx);
        let focused = focus.is_focused(window);
        let editor = self.editor.read(cx);
        let text = editor.document.text().clone();
        let selections = editor.document.selections().to_vec();
        let diagnostics = editor.diagnostics.clone();
        let top_of = |line: usize| state.text_origin.y + line_height * line as f32;

        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            let shown = state.first_line..state.first_line + state.lines.len();
            for &line in state.caret_lines.iter().filter(|l| shown.contains(l)) {
                window.paint_quad(fill(
                    Bounds::new(point(bounds.origin.x, top_of(line)), size(bounds.size.width, line_height)),
                    theme::editor::current_line(),
                ));
            }
            for (i, shaped) in state.lines.iter().enumerate() {
                let line = state.first_line + i;
                let start = text.line_to_byte(line);
                let end = line_end(&text, line);
                for selection in selections.iter().filter(|s| !s.is_empty()) {
                    let (from, to) = (selection.start().max(start), selection.end().min(end + 1));
                    if from > to || (from == to && selection.end() <= end) {
                        continue;
                    }
                    let x0 = shaped.x_for_index(from - start);
                    // A selection that goes on past the line's end shows its line break as a space's width.
                    let x1 = if selection.end() > end { shaped.width + px(7.) } else { shaped.x_for_index(to - start) };
                    window.paint_quad(fill(
                        Bounds::from_corners(
                            point(state.text_origin.x + x0, top_of(line)),
                            point(state.text_origin.x + x1, top_of(line) + line_height),
                        ),
                        theme::editor::selection(),
                    ));
                }
                if let Some(number) = state.line_numbers.get(i) {
                    let _ = number.paint(
                        point(bounds.origin.x + state.gutter_width - number.width - px(20.), top_of(line)),
                        line_height,
                        window,
                        cx,
                    );
                }
                let _ = shaped.paint(point(state.text_origin.x, top_of(line)), line_height, window, cx);
                for diagnostic in diagnostics.iter().filter(|d| d.range.start <= end && d.range.end >= start) {
                    let x0 = shaped.x_for_index(diagnostic.range.start.max(start) - start);
                    let x1 = shaped.x_for_index(diagnostic.range.end.min(end) - start).max(x0 + px(6.));
                    let colour: Hsla = if diagnostic.error { theme::error().into() } else { theme::warning().into() };
                    window.paint_underline(
                        point(state.text_origin.x + x0, top_of(line) + line_height - px(3.)),
                        x1 - x0,
                        &UnderlineStyle { color: Some(colour), thickness: px(1.), wavy: true },
                    );
                }
                if focused {
                    for selection in selections.iter().filter(|s| text.byte_to_line(s.head) == line) {
                        let x = shaped.x_for_index(selection.head - start);
                        window.paint_quad(fill(
                            Bounds::new(
                                point(state.text_origin.x + x, top_of(line) + px(2.)),
                                size(px(2.), line_height - px(4.)),
                            ),
                            theme::editor::caret(),
                        ));
                    }
                }
            }
        });

        let lines = std::mem::take(&mut state.lines);
        let (first_line, text_origin, char_width) = (state.first_line, state.text_origin, state.char_width);
        self.editor.update(cx, |editor, _| {
            editor.layout = Some(Layout { text_origin, char_width, viewport: bounds, first_line, lines })
        });
    }
}

fn run(len: usize, color: Hsla, font: &gpui::Font) -> TextRun {
    TextRun { len, font: font.clone(), color, background_color: None, underline: None, strikethrough: None }
}

/// A line's text runs: its coloured pieces, plain text between them.
fn runs_for(content: &str, line_start: usize, colours: &[Highlight], font: &gpui::Font) -> Vec<TextRun> {
    let plain: Hsla = theme::editor::text().into();
    let mut runs = Vec::new();
    let mut at = 0;
    let line_end = line_start + content.len();
    for h in colours.iter().filter(|h| h.range.end > line_start && h.range.start < line_end) {
        let from = h.range.start.max(line_start) - line_start;
        let to = h.range.end.min(line_end) - line_start;
        // A tree a parse has yet to catch up with may cut a character; its colour waits for the parse.
        if from < at || !content.is_char_boundary(from) || !content.is_char_boundary(to) {
            continue;
        }
        if from > at {
            runs.push(run(from - at, plain, font));
        }
        runs.push(run(to - from, theme::syntax(h.kind), font));
        at = to;
    }
    if at < content.len() || runs.is_empty() {
        runs.push(run(content.len() - at, plain, font));
    }
    runs
}
