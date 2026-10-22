//! The code editor: one open file's document, drawn line by line for only the lines on screen, coloured from
//! its syntax tree (parsed again off the UI thread after each edit) and kept in step with Roslyn.

use crate::theme;
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, ScrollWheelEvent, ShapedLine, Style, Task, TextRun,
    UTF16Selection, UnderlineStyle, Window, actions, div, fill, font, point, prelude::*, px, relative, size,
};
use rsharp_languages::{FoldRegion, Highlight, edit as edit_tree, folds, highlights, matching_bracket, parse};
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
        MatchBrace,
        ToggleFold,
        ExpandRegion,
        CollapseRegion,
        ExpandAll,
        CollapseAll,
        ToggleSoftWrap,
        ToggleWhitespace,
        IncreaseFontSize,
        DecreaseFontSize,
        ResetFontSize,
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
    /// The find bar's matches, sorted, drawn behind the text while it is open.
    pub search_matches: Vec<Range<usize>>,
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
    /// The regions that fold, from the last parse and kept in step with every edit since, each with whether it is
    /// folded.
    pub folds: Vec<(FoldRegion, bool)>,
    /// Lines wider than the view go on in rows below (View | Active Editor | Soft-Wrap).
    pub soft_wrap: bool,
    rows: Rows,
    /// What the rows were made from: the text's version, the folds' generation and the wrap width.
    rows_key: Option<(u64, u64, Option<usize>)>,
    fold_generation: u64,
    /// How many characters fit across the view, as the last frame measured it, for soft wrap.
    columns: Option<usize>,
    /// The document's version the tree was parsed from: brackets are paired only in a tree that is current.
    tree_version: u64,
    /// The pointer is over the gutter, which shows the chevrons of what is not folded.
    gutter_hovered: bool,
    layout: Option<Layout>,
    /// The line height in force, from the settings, as the last frame used it.
    line_height: Pixels,
}

/// Where the last frame put things, for the mouse and the input method to find their way back.
struct Layout {
    text_origin: Point<Pixels>,
    char_width: Pixels,
    viewport: Bounds<Pixels>,
    first_row: usize,
    lines: Vec<ShapedLine>,
    /// The gutter's chevrons and the folded regions' placeholders, each with the fold it stands for.
    chevrons: Vec<(Bounds<Pixels>, usize)>,
    placeholders: Vec<(Bounds<Pixels>, usize)>,
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
            search_matches: Vec::new(),
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
            folds: Vec::new(),
            soft_wrap: false,
            rows: Rows::Lines,
            rows_key: None,
            fold_generation: 0,
            columns: None,
            tree_version: u64::MAX,
            gutter_hovered: false,
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

    /// Selects a range, as the find bar shows a match, and brings it into view.
    pub fn select_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        self.set_carets(vec![Selection { anchor: range.start, head: range.end }], cx);
    }

    /// Applies an edit made from outside the editor (the find bar's replace) as one undo step.
    pub fn apply_plan(&mut self, plan: EditPlan, cx: &mut Context<Self>) {
        self.apply(plan, cx);
    }

    pub fn primary(&self) -> Selection {
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
            for (fold, _) in &mut self.folds {
                for change in &changes {
                    fold.range = map_through(fold.range.start, change)..map_through(fold.range.end, change);
                }
            }
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
            let (tree, regions) = cx
                .background_executor()
                .spawn(async move {
                    let tree = parse(&text, old.as_ref());
                    let regions = folds(&tree, &text);
                    (tree, regions)
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                if editor.document.version() == version {
                    editor.tree = Some(tree);
                    editor.tree_version = version;
                    // A region folded before the parse stays folded where it still starts.
                    let folded: Vec<usize> =
                        editor.folds.iter().filter(|(_, folded)| *folded).map(|(f, _)| f.range.start).collect();
                    editor.folds = regions
                        .into_iter()
                        .map(|f| {
                            let shut = folded.contains(&f.range.start);
                            (f, shut)
                        })
                        .collect();
                    editor.fold_generation += 1;
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

    fn vertical(&mut self, rows: isize, extend: bool, cx: &mut Context<Self>) {
        self.ensure_rows();
        let text = self.document.text().clone();
        let goals = match self.goal_columns.take() {
            Some(goals) if goals.len() == self.document.selections().len() => goals,
            _ => self.document.selections().iter().map(|s| self.column_in_row(&text, s.head)).collect(),
        };
        let targets: Vec<usize> = self
            .document
            .selections()
            .iter()
            .zip(&goals)
            .map(|(s, goal)| self.row_offset(&text, s.head, *goal, rows))
            .collect();
        let mut index = 0;
        self.move_carets(extend, cx, |_, _| {
            index += 1;
            targets[index - 1]
        });
        // Carets that met are one now, and their goals no longer line up with them.
        if self.document.selections().len() == goals.len() {
            self.goal_columns = Some(goals);
        }
    }

    /// Makes the rows again if the text, the folds or the wrap width changed since they were made.
    fn ensure_rows(&mut self) {
        let wrap = self.columns.filter(|_| self.soft_wrap);
        let key = (self.document.version(), self.fold_generation, wrap);
        if self.rows_key != Some(key) {
            self.rows = Rows::build(self.document.text(), &self.hidden_lines(), wrap);
            self.rows_key = Some(key);
        }
    }

    /// The lines folded regions hide, every one after a region's first through its last, sorted and merged.
    fn hidden_lines(&self) -> Vec<Range<usize>> {
        let text = self.document.text();
        let len = text.len_bytes();
        let mut hidden: Vec<Range<usize>> = Vec::new();
        for (fold, _) in self.folds.iter().filter(|(_, folded)| *folded) {
            let first = text.byte_to_line(fold.range.start.min(len));
            let last = text.byte_to_line(fold.range.end.min(len));
            if last <= first {
                continue;
            }
            match hidden.last_mut() {
                Some(h) if first < h.end => h.end = h.end.max(last + 1),
                _ => hidden.push(first + 1..last + 1),
            }
        }
        hidden
    }

    /// A caret's column in its row, in characters.
    fn column_in_row(&self, text: &Rope, offset: usize) -> usize {
        let row = self.rows.row(text, self.rows.row_of(text, offset));
        text.byte_slice(row.start..offset.max(row.start)).chars().count()
    }

    /// The offset `rows` rows from `offset` at `goal` column, or the text's start or end past its first or last row.
    fn row_offset(&self, text: &Rope, offset: usize, goal: usize, rows: isize) -> usize {
        let target = self.rows.row_of(text, offset) as isize + rows;
        if target < 0 {
            return 0;
        }
        if target as usize >= self.rows.len(text) {
            return text.len_bytes();
        }
        let row = self.rows.row(text, target as usize);
        let chars = text.byte_slice(row.start..row.end).chars().count();
        // A wrapped row's end is where the next row starts, so a caret stops a character short of it.
        let goal = if row.last { goal.min(chars) } else { goal.min(chars.saturating_sub(1)) };
        text.char_to_byte(text.byte_to_char(row.start) + goal)
    }

    /// Folds or unfolds the regions `which` picks, the carets kept on rows that show.
    fn set_folded(&mut self, which: impl Fn(usize, &FoldRegion, bool) -> bool, cx: &mut Context<Self>) {
        for (i, (fold, folded)) in self.folds.iter_mut().enumerate() {
            *folded = which(i, fold, *folded);
        }
        self.fold_generation += 1;
        // A caret in what folded goes to the end of the fold's first line, as Rider moves it.
        let text = self.document.text().clone();
        let hidden = self.hidden_lines();
        let carets: Vec<Selection> = self
            .document
            .selections()
            .iter()
            .map(|s| {
                let line = text.byte_to_line(s.head);
                match hidden.iter().find(|h| h.contains(&line)) {
                    Some(h) => Selection::caret(line_end(&text, h.start - 1)),
                    None => *s,
                }
            })
            .collect();
        self.document.set_selections(merge_overlapping(carets));
        self.autoscroll = true;
        cx.notify();
    }

    /// The widest region starting on the caret's line, else the innermost holding it, that `wanted` takes.
    fn fold_at_caret(&self, wanted: impl Fn(bool) -> bool) -> Option<usize> {
        let text = self.document.text();
        let len = text.len_bytes();
        let line = text.byte_to_line(self.primary().head);
        let lines =
            |f: &FoldRegion| text.byte_to_line(f.range.start.min(len))..=text.byte_to_line(f.range.end.min(len));
        let starts = self.folds.iter().position(|(f, folded)| *lines(f).start() == line && wanted(*folded));
        starts.or_else(|| self.folds.iter().rposition(|(f, folded)| lines(f).contains(&line) && wanted(*folded)))
    }

    /// Unfolds what hides a caret, as Rider does when a caret lands in folded lines (Undo, Find, a jump).
    fn reveal_carets(&mut self) {
        let text = self.document.text();
        let len = text.len_bytes();
        let mut opened = false;
        for s in self.document.selections() {
            let line = text.byte_to_line(s.head);
            for (fold, folded) in self.folds.iter_mut().filter(|(_, folded)| *folded) {
                let first = text.byte_to_line(fold.range.start.min(len));
                let last = text.byte_to_line(fold.range.end.min(len));
                if line > first && line <= last {
                    *folded = false;
                    opened = true;
                }
            }
        }
        if opened {
            self.fold_generation += 1;
        }
    }

    fn match_brace(&mut self, cx: &mut Context<Self>) {
        let Some(tree) = self.tree.as_ref().filter(|_| self.tree_version == self.document.version()) else { return };
        let carets: Vec<Selection> = self
            .document
            .selections()
            .iter()
            .map(|s| matching_bracket(tree, s.head).map_or(*s, |(_, pair)| Selection::caret(pair.start)))
            .collect();
        self.set_carets(carets, cx);
    }

    fn zoom(&mut self, size: impl Fn(f32) -> f32, cx: &mut Context<Self>) {
        let mut settings = crate::settings::settings(cx).clone();
        settings.editor_font_size = size(settings.editor_font_size);
        crate::settings::apply(settings, cx);
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

    fn row_for_point(&self, position: Point<Pixels>) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        Some(((position.y - layout.text_origin.y) / self.line_height).floor().max(0.) as usize)
    }

    fn offset_for_point(&mut self, position: Point<Pixels>) -> usize {
        self.ensure_rows();
        let Some(row) = self.row_for_point(position) else { return 0 };
        let Some(layout) = &self.layout else { return 0 };
        let text = self.document.text();
        if row >= self.rows.len(text) {
            return text.len_bytes();
        }
        let span = self.rows.row(text, row);
        match layout.lines.get(row.wrapping_sub(layout.first_row)) {
            Some(shaped) => (span.start + shaped.closest_index_for_x(position.x - layout.text_origin.x)).min(span.end),
            None => span.start,
        }
    }

    /// The line and character column under the pointer, past the line's end as far as the pointer is.
    fn place_for_point(&mut self, position: Point<Pixels>) -> (usize, usize) {
        self.ensure_rows();
        let (Some(row), Some(layout)) = (self.row_for_point(position), self.layout.as_ref()) else { return (0, 0) };
        let text = self.document.text();
        let span = self.rows.row(text, row);
        let column = ((position.x - layout.text_origin.x) / layout.char_width).round().max(0.) as usize;
        (span.line, column_of(text, span.start) + column)
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        let hit = |marks: &[(Bounds<Pixels>, usize)]| {
            marks.iter().find(|(b, _)| b.contains(&event.position)).map(|(_, fold)| *fold)
        };
        if let Some(layout) = &self.layout
            && let Some(fold) = hit(&layout.chevrons).or_else(|| hit(&layout.placeholders))
        {
            self.set_folded(|i, _, folded| if i == fold { !folded } else { folded }, cx);
            return;
        }
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
        let over_gutter = self
            .layout
            .as_ref()
            .is_some_and(|l| l.viewport.contains(&event.position) && event.position.x < l.text_origin.x);
        if over_gutter != self.gutter_hovered {
            self.gutter_hovered = over_gutter;
            cx.notify();
        }
        if let (true, Some(start)) = (self.selecting, self.column_drag)
            && event.pressed_button == Some(MouseButton::Left)
        {
            let head = self.place_for_point(event.position);
            self.set_block(start, head, cx);
        } else if self.selecting && event.pressed_button == Some(MouseButton::Left) {
            let anchor = self.primary().anchor;
            let head = self.offset_for_point(event.position);
            self.document.set_selections(vec![Selection { anchor, head }]);
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
        let content = self.line_height * self.rows.len(self.document.text()) as f32;
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
    MatchBrace => |e, cx| e.match_brace(cx);
    ToggleFold => |e, cx| { if let Some(at) = e.fold_at_caret(|_| true) { e.set_folded(|i, _, f| if i == at { !f } else { f }, cx) } };
    ExpandRegion => |e, cx| { if let Some(at) = e.fold_at_caret(|f| f) { e.set_folded(|i, _, f| f && i != at, cx) } };
    CollapseRegion => |e, cx| { if let Some(at) = e.fold_at_caret(|f| !f) { e.set_folded(|i, _, f| f || i == at, cx) } };
    ExpandAll => |e, cx| e.set_folded(|_, _, _| false, cx);
    CollapseAll => |e, cx| e.set_folded(|_, _, _| true, cx);
    ToggleSoftWrap => |e, cx| { e.soft_wrap = !e.soft_wrap; e.autoscroll = true; cx.notify() };
    ToggleWhitespace => |_, cx| { let mut s = crate::settings::settings(cx).clone(); s.show_whitespace = !s.show_whitespace; crate::settings::apply(s, cx) };
    IncreaseFontSize => |e, cx| e.zoom(|size| size + 1., cx);
    DecreaseFontSize => |e, cx| e.zoom(|size| size - 1., cx);
    ResetFontSize => |e, cx| e.zoom(|_| crate::settings::AppSettings::default().editor_font_size, cx);
}

/// An offset carried through a change: before it, kept; after it, moved by what it added; inside it, to its end.
fn map_through(offset: usize, change: &Change) -> usize {
    if offset <= change.start.offset {
        offset
    } else if offset >= change.old_end.offset {
        offset - change.old_end.offset + change.new_end.offset
    } else {
        change.new_end.offset
    }
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
            .on_hover(cx.listener(|e, hovered: &bool, _, cx| {
                if !hovered && e.gutter_hovered {
                    e.gutter_hovered = false;
                    cx.notify();
                }
            }))
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
        let offset = self.utf16_to_offset(range.start);
        self.ensure_rows();
        let layout = self.layout.as_ref()?;
        let text = self.document.text();
        let row = self.rows.row_of(text, offset);
        let shaped = layout.lines.get(row.checked_sub(layout.first_row)?)?;
        let x = shaped.x_for_index(offset - self.rows.row(text, row).start);
        let top = layout.text_origin.y + self.line_height * row as f32;
        Some(Bounds::new(point(layout.text_origin.x + x, top), size(px(2.), self.line_height)))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let offset = self.offset_for_point(point);
        Some(self.offset_to_utf16(offset))
    }
}

struct EditorElement {
    editor: Entity<CodeEditor>,
}

struct Prepainted {
    lines: Vec<ShapedLine>,
    rows: Vec<Row>,
    first_row: usize,
    text_origin: Point<Pixels>,
    gutter_width: Pixels,
    /// The rows with an empty caret, for the current row's band.
    caret_rows: Vec<usize>,
    line_numbers: Vec<Option<ShapedLine>>,
    line_height: Pixels,
    char_width: Pixels,
    /// Each shown row's fold, if a region starts on its line: the fold and whether it is folded.
    fold_marks: Vec<Option<(usize, bool)>>,
    placeholder: ShapedLine,
    /// The bracket by the caret and its pair, in a tree that is current.
    braces: Vec<Range<usize>>,
    show_whitespace: bool,
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
            let digits = editor.document.text().len_lines().to_string().len().max(3);
            let char_width = window
                .text_system()
                .shape_line("0".into(), font_size, &[run(1, theme::editor::text().into(), &code_font)], None)
                .width;
            // The numbers, then the folding column against the code.
            let gutter_width = if settings.show_line_numbers { char_width * digits as f32 + px(32.) } else { px(24.) };
            let text_left = bounds.origin.x + gutter_width + px(TEXT_PAD) - px(16.);
            editor.columns = Some(((bounds.right() - text_left - px(TEXT_PAD)) / char_width).floor().max(8.) as usize);
            editor.reveal_carets();
            editor.ensure_rows();
            let text = editor.document.text();
            let rows = &editor.rows;
            let caret_row = rows.row_of(text, editor.primary().head);
            let mut caret_rows: Vec<usize> = editor
                .document
                .selections()
                .iter()
                .filter(|s| s.is_empty())
                .map(|s| rows.row_of(text, s.head))
                .collect();
            caret_rows.sort_unstable();
            caret_rows.dedup();
            let mut caret_lines: Vec<usize> =
                editor.document.selections().iter().map(|s| text.byte_to_line(s.head)).collect();
            caret_lines.sort_unstable();

            if editor.autoscroll {
                editor.autoscroll = false;
                let caret_top = line_height * caret_row as f32;
                let margin = line_height * 2.;
                if caret_top - margin < editor.scroll_y {
                    editor.scroll_y = (caret_top - margin).max(px(0.));
                } else if caret_top + line_height + margin > editor.scroll_y + bounds.size.height {
                    editor.scroll_y = caret_top + line_height + margin - bounds.size.height;
                }
            }

            let first_row = (editor.scroll_y / line_height).floor() as usize;
            let visible = (bounds.size.height / line_height).ceil() as usize + 1;
            let last_row = (first_row + visible).min(rows.len(text));
            let shown: Vec<Row> = (first_row..last_row).map(|i| rows.row(text, i)).collect();
            let byte_range = shown.first().map_or(0, |r| r.start)..shown.last().map_or(0, |r| r.end);
            let colours: Vec<Highlight> =
                editor.tree.as_ref().map(|tree| highlights(tree, text, byte_range.clone())).unwrap_or_default();

            let len = text.len_bytes();
            let line_of = |byte: usize| text.byte_to_line(byte.min(len));
            let mut lines = Vec::with_capacity(shown.len());
            let mut line_numbers = Vec::with_capacity(shown.len());
            let mut fold_marks = Vec::with_capacity(shown.len());
            for row in &shown {
                let content: String = text.byte_slice(row.start..row.end).to_string();
                let runs = runs_for(&content, row.start, &colours, &code_font);
                lines.push(window.text_system().shape_line(content.into(), font_size, &runs, None));
                let number = (row.first && settings.show_line_numbers).then(|| {
                    let number = (row.line + 1).to_string();
                    let colour = if caret_lines.binary_search(&row.line).is_ok() {
                        theme::editor::line_number_current()
                    } else {
                        theme::editor::line_number()
                    };
                    window.text_system().shape_line(
                        number.clone().into(),
                        font_size,
                        &[run(number.len(), colour.into(), &code_font)],
                        None,
                    )
                });
                line_numbers.push(number);
                // The widest region starting on the line, which comes first among those starting there.
                let from = editor.folds.partition_point(|(f, _)| line_of(f.range.start) < row.line);
                let mark = editor.folds[from..]
                    .iter()
                    .take_while(|(f, _)| line_of(f.range.start) == row.line)
                    .position(|(f, _)| line_of(f.range.end) > row.line)
                    .map(|i| (from + i, editor.folds[from + i].1));
                fold_marks.push(mark);
            }
            let placeholder = window.text_system().shape_line(
                "...".into(),
                font_size,
                &[run(3, theme::editor::folded_text().into(), &code_font)],
                None,
            );
            let primary = editor.primary();
            let braces = match (&editor.tree, primary.is_empty() && editor.tree_version == editor.document.version()) {
                (Some(tree), true) => matching_bracket(tree, primary.head).map(|(a, b)| vec![a, b]).unwrap_or_default(),
                _ => Vec::new(),
            };
            Prepainted {
                lines,
                rows: shown,
                first_row,
                text_origin: point(text_left, bounds.origin.y - editor.scroll_y),
                gutter_width,
                caret_rows,
                line_numbers,
                line_height,
                char_width,
                fold_marks,
                placeholder,
                braces,
                show_whitespace: settings.show_whitespace,
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
        let gutter_hovered = editor.gutter_hovered;
        let shown_bytes = state.rows.first().map_or(0, |r| r.start)..state.rows.last().map_or(0, |r| r.end);
        let from = editor.search_matches.partition_point(|m| m.end < shown_bytes.start);
        let to = editor.search_matches.partition_point(|m| m.start <= shown_bytes.end);
        let matches = editor.search_matches[from..to.max(from)].to_vec();
        let caret_rows: Vec<usize> = selections.iter().map(|s| editor.rows.row_of(&text, s.head)).collect();
        let top_of = |row: usize| state.text_origin.y + line_height * row as f32;
        let mut chevrons = Vec::new();
        let mut placeholders = Vec::new();

        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            let shown = state.first_row..state.first_row + state.lines.len();
            for &row in state.caret_rows.iter().filter(|r| shown.contains(r)) {
                window.paint_quad(fill(
                    Bounds::new(point(bounds.origin.x, top_of(row)), size(bounds.size.width, line_height)),
                    theme::editor::current_line(),
                ));
            }
            for (i, shaped) in state.lines.iter().enumerate() {
                let row = state.first_row + i;
                let span = state.rows[i];
                let (start, end) = (span.start, span.end);
                let top = top_of(row);
                let band = |window: &mut Window, x0: Pixels, x1: Pixels, colour: gpui::Rgba| {
                    window.paint_quad(fill(
                        Bounds::from_corners(
                            point(state.text_origin.x + x0, top),
                            point(state.text_origin.x + x1, top + line_height),
                        ),
                        colour,
                    ));
                };
                for m in matches.iter().filter(|m| m.start <= end && m.end >= start && !m.is_empty()) {
                    let (x0, x1) =
                        (shaped.x_for_index(m.start.max(start) - start), shaped.x_for_index(m.end.min(end) - start));
                    band(window, x0, x1, theme::editor::search_result());
                }
                for brace in state.braces.iter().filter(|b| b.start >= start && b.end <= end) {
                    let (x0, x1) = (shaped.x_for_index(brace.start - start), shaped.x_for_index(brace.end - start));
                    band(window, x0, x1, theme::editor::matched_brace());
                }
                // A selection past a line's end shows its break as a space's width; a wrapped row has none.
                let past = if span.last { end + 1 } else { end };
                for selection in selections.iter().filter(|s| !s.is_empty()) {
                    let (from, to) = (selection.start().max(start), selection.end().min(past));
                    if from > to || (from == to && (selection.end() <= end || !span.last)) {
                        continue;
                    }
                    let x0 = shaped.x_for_index(from - start);
                    let x1 = match (selection.end() > end, span.last) {
                        (true, true) => shaped.width + px(7.),
                        (true, false) => shaped.width,
                        _ => shaped.x_for_index(to - start),
                    };
                    band(window, x0, x1, theme::editor::selection());
                }
                if let Some(Some(number)) = state.line_numbers.get(i) {
                    let _ = number.paint(
                        point(bounds.origin.x + state.gutter_width - number.width - px(20.), top),
                        line_height,
                        window,
                        cx,
                    );
                }
                if let Some(Some((fold, folded))) = state.fold_marks.get(i).filter(|_| span.first) {
                    if *folded || gutter_hovered {
                        let at = Bounds::new(
                            point(bounds.origin.x + state.gutter_width - px(16.), top + (line_height - px(14.)) / 2.),
                            size(px(14.), px(14.)),
                        );
                        let icon = if *folded { "icons/chevron-right.svg" } else { "icons/chevron-down.svg" };
                        let colour: Hsla = theme::editor::line_number_current().into();
                        let _ = window.paint_svg(at, icon.into(), Default::default(), colour, cx);
                    }
                    chevrons.push((
                        Bounds::new(
                            point(bounds.origin.x + state.gutter_width - px(18.), top),
                            size(px(18.), line_height),
                        ),
                        *fold,
                    ));
                }
                let _ = shaped.paint(point(state.text_origin.x, top), line_height, window, cx);
                if let Some(Some((fold, true))) = state.fold_marks.get(i).filter(|_| span.last) {
                    let x = state.text_origin.x + shaped.width + state.char_width;
                    let chip = Bounds::new(
                        point(x, top + px(2.)),
                        size(state.placeholder.width + state.char_width, line_height - px(4.)),
                    );
                    window.paint_quad(fill(chip, theme::editor::folded_background()).corner_radii(px(3.)));
                    let _ = state.placeholder.paint(point(x + state.char_width / 2., top), line_height, window, cx);
                    placeholders.push((chip, *fold));
                }
                if state.show_whitespace {
                    let content = text.byte_slice(start..end).to_string();
                    for (at, c) in content.char_indices().filter(|(_, c)| *c == ' ' || *c == '\t') {
                        let (x0, x1) = (shaped.x_for_index(at), shaped.x_for_index(at + 1));
                        let mid = state.text_origin.x + (x0 + x1) / 2.;
                        let y = top + line_height / 2.;
                        let mark = if c == ' ' {
                            Bounds::new(point(mid - px(1.), y - px(1.)), size(px(2.), px(2.)))
                        } else {
                            Bounds::new(point(state.text_origin.x + x0 + px(2.), y), size(x1 - x0 - px(4.), px(1.)))
                        };
                        window.paint_quad(fill(mark, theme::editor::whitespace()));
                    }
                }
                for diagnostic in diagnostics.iter().filter(|d| d.range.start <= end && d.range.end >= start) {
                    let x0 = shaped.x_for_index(diagnostic.range.start.max(start) - start);
                    let x1 = shaped.x_for_index(diagnostic.range.end.min(end) - start).max(x0 + px(6.));
                    let colour: Hsla = if diagnostic.error { theme::error().into() } else { theme::warning().into() };
                    window.paint_underline(
                        point(state.text_origin.x + x0, top + line_height - px(3.)),
                        x1 - x0,
                        &UnderlineStyle { color: Some(colour), thickness: px(1.), wavy: true },
                    );
                }
                if focused {
                    for (selection, _) in selections.iter().zip(&caret_rows).filter(|(_, r)| **r == row) {
                        let x = shaped.x_for_index(selection.head.clamp(start, end) - start);
                        window.paint_quad(fill(
                            Bounds::new(
                                point(state.text_origin.x + x, top + px(2.)),
                                size(px(2.), line_height - px(4.)),
                            ),
                            theme::editor::caret(),
                        ));
                    }
                }
            }
        });

        let lines = std::mem::take(&mut state.lines);
        let (first_row, text_origin, char_width) = (state.first_row, state.text_origin, state.char_width);
        self.editor.update(cx, |editor, _| {
            editor.layout =
                Some(Layout { text_origin, char_width, viewport: bounds, first_row, lines, chevrons, placeholders })
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
