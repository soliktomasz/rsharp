//! The code editor: a view of one open file's buffer, drawn line by line for only the lines on screen, coloured from
//! its syntax tree (parsed again off the UI thread after each edit) and kept in step with Roslyn.

use crate::buffer::{Buffer, BufferEvent};
use crate::theme;
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollWheelEvent, ShapedLine, Style,
    Subscription, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill, font, point, prelude::*, px,
    relative, size,
};
use rsharp_languages::{FoldRegion, Highlight, folds, folds_in, highlights, matching_bracket};
use rsharp_text::*;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

#[path = "editor_assist.rs"]
mod assist;
#[path = "editor_msbuild.rs"]
mod msbuild_help;
#[path = "editor_refactor.rs"]
mod refactor;
#[path = "editor_templates.rs"]
mod templates;
#[path = "editor_xaml.rs"]
mod xaml_help;
pub use assist::{Navigate, Said, ValueAsked};
pub use xaml_help::{CodeBehind, XamlField, generated_field, is_markup_compiled};

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
        Complete,
        ParameterInfo,
        QuickDocumentation,
        ShowContextActions,
        Rename,
        Reformat,
        GoToDeclaration,
        GoToImplementation,
        GoToTypeDeclaration,
        BaseSymbols,
        DerivedSymbols,
        FindUsages,
        OptimizeUsings,
        IntroduceVariable,
        IntroduceField,
        IntroduceParameter,
        EncapsulateField,
        ExtractMethod,
        Inline,
        SafeDelete,
        SurroundWith,
        CompleteStatement,
        ChangeSignature,
        ExtractInterface,
        MoveType,
        PullMembersUp,
        PushMembersDown,
        ExtractResource,
        ExtractStyle,
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
    pub buffer: Entity<Buffer>,
    /// The file, as its buffer has it.
    pub path: PathBuf,
    /// The buffer's text as of its last change, a snapshot as cheap as a pointer, so the carets' moves need no
    /// trip to the buffer.
    text: Rope,
    version: u64,
    /// This editor's carets, the last the primary; another editor of the same file has its own.
    carets: Vec<Selection>,
    /// The find bar's matches, sorted, drawn behind the text while it is open.
    pub search_matches: Vec<Range<usize>>,
    focus: FocusHandle,
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
    /// The regions folded here, sorted, kept in step with every edit; the others are found in the tree as the lines
    /// on screen or a fold action need them.
    pub folded: Vec<FoldRegion>,
    /// Lines wider than the view go on in rows below (View | Active Editor | Soft-Wrap).
    pub soft_wrap: bool,
    rows: Rows,
    /// What the rows were made from: the text's version, the folds' generation and the wrap width.
    rows_key: Option<(u64, u64, Option<usize>)>,
    fold_generation: u64,
    /// How many characters fit across the view, as the last frame measured it, for soft wrap.
    columns: Option<usize>,
    /// The pointer is over the gutter, which shows the chevrons of what is not folded.
    gutter_hovered: bool,
    /// The line the execution arrow was taken from, while it is dragged to where the program should go on.
    arrow_drag: Option<usize>,
    git_popup: Option<GitPopup>,
    layout: Option<Layout>,
    /// The line height in force, from the settings, as the last frame used it.
    line_height: Pixels,
    _buffer_events: Subscription,
    /// An edit a read-only file refused, made once its read-only status is cleared if the text is as it was then.
    pending: Option<(EditPlan, u64)>,
    /// What Roslyn says about the file as it is worked on: popups, usages, colours.
    assist: assist::Assist,
    /// A console uses the editor's selection and visible-line drawing, without a file or undo history.
    console: Option<Console>,
    /// A console whose program reads its input: what is typed goes to the program rather than into the text.
    console_input: bool,
}

struct GitPopup {
    at: Point<Pixels>,
    version: u64,
    baseline: Arc<str>,
    edit: Edit,
    before: Arc<Vec<String>>,
    after: Arc<Vec<String>>,
}

struct Console {
    folder: PathBuf,
    errors: Vec<Range<usize>>,
    follow: bool,
}

/// What a console's link opens: a place in the code, as a compiler's message and a stack frame both name, or a URL,
/// which the browser opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleTarget {
    File { path: PathBuf, line: usize, column: usize },
    Url(String),
}

pub struct ConsoleLink(pub ConsoleTarget);

impl gpui::EventEmitter<ConsoleLink> for CodeEditor {}

/// A click in the gutter's breakpoint column, on a line from 0: set one there or clear the one there. A Shift+click
/// sets a tracepoint, which logs and goes on, as Rider's does.
pub struct ToggleBreakpoint {
    pub line: usize,
    pub tracepoint: bool,
}

impl gpui::EventEmitter<ToggleBreakpoint> for CodeEditor {}

/// A right-click on a line's breakpoint: its properties, as Rider's popup shows them.
pub struct EditBreakpointAt {
    pub line: usize,
}

impl gpui::EventEmitter<EditBreakpointAt> for CodeEditor {}

/// The execution arrow dragged to another line: the paused program goes on from there, as Rider's Set Next Statement.
pub struct MoveExecution {
    pub line: usize,
}

impl gpui::EventEmitter<MoveExecution> for CodeEditor {}

/// Text typed into a console whose program reads its input: the program's own, not the console's text.
pub struct ConsoleInput(pub String);

impl gpui::EventEmitter<ConsoleInput> for CodeEditor {}

/// An edit refused because the file is read-only: the window asks to clear that.
pub struct ReadOnlyEdit;

impl gpui::EventEmitter<ReadOnlyEdit> for CodeEditor {}

/// What follows a read-only file's status being cleared.
#[derive(Clone, Copy, PartialEq)]
pub enum AfterClearing {
    Edit,
    Save,
}

/// Rider's Clear Read-Only Status, asked of a read-only file's first edit or save: on yes the file is made
/// writable and the edit made, or the file saved; a failure is logged.
pub fn ask_clear_read_only(editor: Entity<CodeEditor>, then: AfterClearing, window: &mut Window, cx: &mut App) {
    let name = editor.read(cx).path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let answer = window.prompt(
        gpui::PromptLevel::Warning,
        &format!("{name} is read-only"),
        Some("Clear its read-only status to change it?"),
        &["Clear Read-Only Status", "Cancel"],
        cx,
    );
    cx.spawn(async move |cx| {
        if answer.await != Ok(0) {
            return;
        }
        let _ = editor.update(cx, |e, cx| {
            let buffer = e.buffer.clone();
            if let Err(error) = buffer.update(cx, |b, cx| b.clear_read_only(cx)) {
                log::warn!("Could not clear the read-only status of {}: {error}", e.path.display());
                return;
            }
            match then {
                AfterClearing::Edit => {
                    if let Some((plan, _)) = e.pending.take().filter(|(_, v)| *v == e.version) {
                        e.apply(plan, cx);
                    }
                }
                AfterClearing::Save => {
                    if let Err(error) = e.save(cx) {
                        log::warn!("Could not save {}: {error}", e.path.display());
                    }
                }
            }
        });
    })
    .detach();
}

/// Where the last frame put things, for the mouse and the input method to find their way back.
struct Layout {
    text_origin: Point<Pixels>,
    char_width: Pixels,
    viewport: Bounds<Pixels>,
    first_row: usize,
    lines: Vec<Shaped>,
    /// The gutter's chevrons and the folded regions' placeholders, each with the fold it stands for.
    chevrons: Vec<(Bounds<Pixels>, FoldRegion)>,
    placeholders: Vec<(Bounds<Pixels>, FoldRegion)>,
    git_changes: Vec<(Bounds<Pixels>, DiffHunk)>,
    /// The colour swatches, each with its colour's place in the buffer's list.
    swatches: Vec<(Bounds<Pixels>, usize)>,
}

impl CodeEditor {
    /// An editor of a file: of its buffer if it is open already, else of one read from `bytes`.
    pub fn new(buffer: Entity<Buffer>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&buffer, |editor, buffer, event: &BufferEvent, cx| {
            match event {
                BufferEvent::Edited { changes, by } => {
                    let before = editor.version;
                    editor.sync(&buffer, cx);
                    // The editor that made the change has its carets from it already.
                    if *by != cx.entity_id() {
                        let carets = editor
                            .carets
                            .iter()
                            .map(|s| Selection {
                                anchor: changes.iter().fold(s.anchor, |o, c| map_through(o, c)),
                                head: changes.iter().fold(s.head, |o, c| map_through(o, c)),
                            })
                            .collect();
                        editor.set_selections(carets);
                        editor.map_folds(changes);
                        editor.assist.map_through(changes);
                        editor.rows_after(before, changes);
                    }
                }
                BufferEvent::Parsed => {}
                BufferEvent::Moved(to) => editor.path = to.clone(),
                BufferEvent::ContextChanged => editor.server_ready(cx),
            }
            cx.notify();
        });
        cx.observe_global::<crate::settings::Settings>(|_, cx| cx.notify()).detach();
        cx.observe(&buffer, |_, _, cx| cx.notify()).detach();
        let read = buffer.read(cx);
        Self {
            path: read.path.clone(),
            text: read.document.text().clone(),
            version: read.document.version(),
            carets: vec![Selection::default()],
            buffer: buffer.clone(),
            search_matches: Vec::new(),
            focus: cx.focus_handle(),
            scroll_y: px(0.),
            autoscroll: false,
            goal_columns: None,
            marked: None,
            selecting: false,
            column_drag: None,
            block: None,
            column_mode: false,
            folded: Vec::new(),
            soft_wrap: false,
            rows: Rows::Lines,
            rows_key: None,
            fold_generation: 0,
            columns: None,
            gutter_hovered: false,
            arrow_drag: None,
            git_popup: None,
            layout: None,
            line_height: px(crate::settings::settings(cx).line_height()),
            _buffer_events: subscription,
            pending: None,
            assist: assist::Assist::default(),
            console: None,
            console_input: false,
        }
    }

    pub(crate) fn console(&mut self, folder: PathBuf) {
        self.console = Some(Console { folder, errors: Vec::new(), follow: true });
        self.soft_wrap = true;
    }

    /// Whether what is typed goes to the program reading the console's input.
    pub(crate) fn console_input(&mut self, on: bool) {
        self.console_input = on;
    }

    pub(crate) fn output(&mut self, text: Rope, errors: Vec<Range<usize>>, removed: usize, cx: &mut Context<Self>) {
        let follow = self.console.as_ref().is_some_and(|c| c.follow);
        self.text = text;
        self.version += 1;
        self.rows_key = None;
        let len = self.text.len_bytes();
        for selection in &mut self.carets {
            selection.anchor = selection.anchor.saturating_sub(removed).min(len);
            selection.head = selection.head.saturating_sub(removed).min(len);
        }
        if follow {
            self.carets = vec![Selection::caret(len)];
            self.autoscroll = true;
        }
        if let Some(console) = &mut self.console {
            console.errors = errors;
        }
        cx.notify();
    }

    /// The text and its version as the buffer has them now.
    fn sync(&mut self, buffer: &Entity<Buffer>, cx: &App) {
        let read = buffer.read(cx);
        if self.version != read.document.version() {
            self.git_popup = None;
        }
        self.text = read.document.text().clone();
        self.version = read.document.version();
    }

    fn map_folds(&mut self, changes: &[Change]) {
        for fold in &mut self.folded {
            for change in changes {
                fold.range = map_through(fold.range.start, change)..map_through(fold.range.end, change);
            }
        }
    }

    pub fn text(&self) -> &Rope {
        &self.text
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn selections(&self) -> &[Selection] {
        &self.carets
    }

    pub fn is_modified(&self, cx: &App) -> bool {
        self.buffer.read(cx).is_modified()
    }

    pub fn save(&mut self, cx: &mut Context<Self>) -> std::io::Result<()> {
        self.buffer.update(cx, |b, cx| b.save(cx))
    }

    /// The carets, kept inside the text.
    fn set_selections(&mut self, selections: Vec<Selection>) {
        let len = self.text.len_bytes();
        self.carets =
            selections.into_iter().map(|s| Selection { anchor: s.anchor.min(len), head: s.head.min(len) }).collect();
        if self.carets.is_empty() {
            self.carets.push(Selection::default());
        }
    }

    pub fn caret(&self) -> Position {
        position(&self.text, self.primary().head)
    }

    pub fn place_caret(&mut self, line: usize, column: usize, cx: &mut Context<Self>) {
        let offset = offset_at(&self.text, line, column);
        self.set_selections(vec![Selection::caret(offset)]);
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
        *self.carets.last().expect("an editor always has a caret")
    }

    fn apply(&mut self, plan: EditPlan, cx: &mut Context<Self>) {
        if self.console.is_some() {
            return;
        }
        if self.buffer.read(cx).read_only {
            self.pending = Some((plan, self.version));
            cx.emit(ReadOnlyEdit);
            return;
        }
        let (carets, me) = (self.carets.clone(), cx.entity_id());
        let (after, changes) = self.buffer.update(cx, |b, cx| b.edit(carets, plan, me, cx));
        self.edited(after, &changes, cx);
    }

    fn undo(&mut self, redo: bool, cx: &mut Context<Self>) {
        if self.console.is_some() {
            return;
        }
        if self.buffer.read(cx).read_only {
            // Undo is not kept to make once cleared, and an edit refused before it must not be made in its place.
            self.pending = None;
            cx.emit(ReadOnlyEdit);
            return;
        }
        let me = cx.entity_id();
        if let Some((after, changes)) = self.buffer.update(cx, |b, cx| b.undo(redo, me, cx)) {
            self.edited(after, &changes, cx);
        }
    }

    /// After this editor's own change: the text taken from the buffer, the folds carried through what changed (the
    /// buffer's event comes after, too late for a key that follows).
    fn edited(&mut self, carets: Vec<Selection>, changes: &[Change], cx: &mut Context<Self>) {
        let buffer = self.buffer.clone();
        let before = self.version;
        self.sync(&buffer, cx);
        self.map_folds(changes);
        self.assist.map_through(changes);
        self.rows_after(before, changes);
        self.set_selections(carets);
        self.goal_columns = None;
        self.autoscroll = true;
        cx.notify();
    }

    fn move_carets(&mut self, extend: bool, cx: &mut Context<Self>, mut to: impl FnMut(&Rope, &Selection) -> usize) {
        let text = self.text.clone();
        let moved: Vec<Selection> = self
            .carets
            .iter()
            .map(|s| {
                let head = to(&text, s);
                Selection { anchor: if extend { s.anchor } else { head }, head }
            })
            .collect();
        self.set_selections(merge_overlapping(moved));
        self.autoscroll = true;
        self.close_popup(cx);
        // The caret leaving a template ends its filling in, as Rider's does.
        self.assist.template = None;
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
        let text = self.text.clone();
        let goals = match self.goal_columns.take() {
            Some(goals) if goals.len() == self.carets.as_slice().len() => goals,
            _ => self.carets.as_slice().iter().map(|s| self.column_in_row(&text, s.head)).collect(),
        };
        let targets: Vec<usize> =
            self.carets.iter().zip(&goals).map(|(s, goal)| self.row_offset(&text, s.head, *goal, rows)).collect();
        let mut index = 0;
        self.move_carets(extend, cx, |_, _| {
            index += 1;
            targets[index - 1]
        });
        // The move puts the carets in the text's order: each keeps the goal of the caret that went where it is. Carets
        // that met are one now, and their goals no longer line up with them.
        let kept: Option<Vec<usize>> =
            self.carets.iter().map(|c| targets.iter().position(|t| *t == c.head).map(|i| goals[i])).collect();
        self.goal_columns = kept.filter(|g| g.len() == goals.len());
    }

    /// Makes the rows again if the text, the folds or the wrap width changed since they were made.
    fn ensure_rows(&mut self) {
        let wrap = self.columns.filter(|_| self.soft_wrap);
        let key = (self.version, self.fold_generation, wrap);
        if self.rows_key != Some(key) {
            self.rows = Rows::build(&self.text, &self.hidden_lines(), wrap);
            self.rows_key = Some(key);
        }
    }

    /// After a change, the rows of only the lines it touched made again, where the rows were made for the text
    /// just before it; anything else (several changes, a fold beside it) leaves them to be made whole.
    fn rows_after(&mut self, before: u64, changes: &[Change]) {
        let wrap = self.columns.filter(|_| self.soft_wrap);
        let made_for = Some((before, self.fold_generation, wrap));
        if let ([change], true) = (changes, self.rows_key == made_for)
            && self.rows.apply_change(&self.text, change, &self.hidden_lines(), wrap)
        {
            self.rows_key = Some((self.version, self.fold_generation, wrap));
        }
    }

    /// The lines folded regions hide, every one after a region's first through its last, sorted and merged.
    fn hidden_lines(&self) -> Vec<Range<usize>> {
        let text = &self.text;
        let len = text.len_bytes();
        let mut hidden: Vec<Range<usize>> = Vec::new();
        for fold in &self.folded {
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

    /// Changes what is folded, the carets kept on rows that show.
    fn set_folded(&mut self, change: impl FnOnce(&mut Vec<FoldRegion>), cx: &mut Context<Self>) {
        change(&mut self.folded);
        self.folded.sort_by_key(|f| (f.range.start, std::cmp::Reverse(f.range.end)));
        self.folded.dedup();
        self.fold_generation += 1;
        // A caret in what folded goes to the end of the fold's first line, as Rider moves it.
        let text = self.text.clone();
        let hidden = self.hidden_lines();
        let carets: Vec<Selection> = self
            .carets
            .iter()
            .map(|s| {
                let line = text.byte_to_line(s.head);
                match hidden.iter().find(|h| h.contains(&line)) {
                    Some(h) => Selection::caret(line_end(&text, h.start - 1)),
                    None => *s,
                }
            })
            .collect();
        self.set_selections(merge_overlapping(carets));
        self.autoscroll = true;
        cx.notify();
    }

    fn toggle_fold(&mut self, region: FoldRegion, cx: &mut Context<Self>) {
        self.set_folded(
            |folded| match folded.iter().position(|f| *f == region) {
                Some(at) => drop(folded.remove(at)),
                None => folded.push(region),
            },
            cx,
        );
    }

    /// The widest region starting on the caret's line, else the innermost holding it, folded here or not as `folded`
    /// asks (either, for none).
    fn fold_at_caret(&self, folded: Option<bool>, cx: &App) -> Option<FoldRegion> {
        let text = &self.text;
        let len = text.len_bytes();
        let line = text.byte_to_line(self.primary().head);
        let tree = self.buffer.read(cx).tree.as_ref()?;
        let around = text.line_to_byte(line)..line_end(text, line);
        let lines =
            |f: &FoldRegion| text.byte_to_line(f.range.start.min(len))..=text.byte_to_line(f.range.end.min(len));
        let wanted = |f: &FoldRegion| folded.is_none_or(|want| self.folded.contains(f) == want);
        let regions = folds_in(tree, text, around);
        let starts = regions.iter().find(|f| *lines(f).start() == line && *lines(f).end() > line && wanted(f));
        starts.or_else(|| regions.iter().rev().find(|f| lines(f).contains(&line) && wanted(f))).cloned()
    }

    /// Unfolds what hides a caret, as Rider does when a caret lands in folded lines (Undo, Find, a jump).
    fn reveal_carets(&mut self) {
        let text = &self.text;
        let len = text.len_bytes();
        let mut opened = false;
        for s in self.carets.as_slice() {
            let line = text.byte_to_line(s.head);
            self.folded.retain(|fold| {
                let first = text.byte_to_line(fold.range.start.min(len));
                let last = text.byte_to_line(fold.range.end.min(len));
                let hides = line > first && line <= last;
                opened |= hides;
                !hides
            });
        }
        if opened {
            self.fold_generation += 1;
        }
    }

    fn match_brace(&mut self, cx: &mut Context<Self>) {
        let buffer = self.buffer.read(cx);
        let Some(tree) = buffer.tree.as_ref().filter(|_| buffer.tree_version == self.version) else { return };
        let carets: Vec<Selection> = self
            .carets
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
        let text = &self.text;
        (text.byte_to_line(offset), column_of(text, offset))
    }

    fn set_block(&mut self, start: Place, head: Place, cx: &mut Context<Self>) {
        let carets = column_selection(&self.text, start, head);
        self.set_carets(carets.clone(), cx);
        self.block = Some((start, head, carets));
    }

    /// Shift and an arrow in Column Selection Mode: the column selection's head a line or a column further.
    fn extend_block(&mut self, lines: isize, columns: isize, cx: &mut Context<Self>) {
        let (start, head) = match &self.block {
            Some((start, head, carets)) if carets.as_slice() == self.carets.as_slice() => (*start, *head),
            _ => (self.place_of(self.primary().anchor), self.place_of(self.primary().head)),
        };
        let last = self.text.len_lines() as isize - 1;
        let head = ((head.0 as isize + lines).clamp(0, last) as usize, (head.1 as isize + columns).max(0) as usize);
        self.set_block(start, head, cx);
    }

    fn set_carets(&mut self, selections: Vec<Selection>, cx: &mut Context<Self>) {
        self.set_selections(selections);
        self.goal_columns = None;
        self.autoscroll = true;
        cx.notify();
    }

    /// Escape: the extra carets go, then the selection; with neither, the key is the window's.
    fn escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        let primary = self.primary();
        if self.git_popup.take().is_some() {
            cx.notify();
        } else if self.assist.popup.is_some() {
            self.close_popup(cx);
        } else if self.assist.template.take().is_some() {
            cx.notify();
        } else if self.carets.as_slice().len() > 1 {
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
        let text = &self.text;
        let mut selections = self.carets.as_slice().to_vec();
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
        let text = &self.text;
        if row >= self.rows.len(text) {
            return text.len_bytes();
        }
        let span = self.rows.row(text, row);
        match layout.lines.get(row.wrapping_sub(layout.first_row)) {
            Some(shaped) => {
                let x = position.x - layout.text_origin.x - layout.char_width * span.indent as f32;
                (span.start + shaped.closest_index_for_x(x)).min(span.end)
            }
            None => span.start,
        }
    }

    /// The character under the pointer, if it is over the text rather than past a line's end or in the gutter.
    fn text_under(&mut self, position: Point<Pixels>) -> Option<usize> {
        self.ensure_rows();
        let row = self.row_for_point(position)?;
        let layout = self.layout.as_ref()?;
        let shaped = layout.lines.get(row.checked_sub(layout.first_row)?)?;
        let span = self.rows.row(&self.text, row);
        let x = position.x - layout.text_origin.x - layout.char_width * span.indent as f32;
        if x < px(0.) || x >= shaped.width {
            return None;
        }
        Some((span.start + shaped.index_for_x(x).unwrap_or(0)).min(span.end))
    }

    /// The line and character column under the pointer, past the line's end as far as the pointer is.
    fn place_for_point(&mut self, position: Point<Pixels>) -> (usize, usize) {
        self.ensure_rows();
        let (Some(row), Some(layout)) = (self.row_for_point(position), self.layout.as_ref()) else { return (0, 0) };
        let text = &self.text;
        let span = self.rows.row(text, row);
        let column = ((position.x - layout.text_origin.x) / layout.char_width).round().max(0.) as usize;
        (span.line, column_of(text, span.start) + column)
    }

    /// Where a byte of the text is drawn, the middle of its line, in the window's coordinates as last laid out; none
    /// while its line is off screen.
    pub fn text_point(&self, offset: usize) -> Option<Point<Pixels>> {
        let layout = self.layout.as_ref()?;
        let row = self.rows.row_of(&self.text, offset);
        let shaped = layout.lines.get(row.checked_sub(layout.first_row)?)?;
        let span = self.rows.row(&self.text, row);
        let x = shaped.x_for_index(offset - span.start) + layout.char_width * span.indent as f32;
        Some(point(layout.text_origin.x + x, layout.text_origin.y + self.line_height * (row as f32 + 0.5)))
    }

    /// The hover card's text, while one is shown, for a test.
    pub fn card_text(&self) -> Option<&str> {
        match &self.assist.popup {
            Some(assist::Popup::Card { text, .. }) => Some(text),
            _ => None,
        }
    }

    /// The middle of a line's breakpoint column, where its dot and the execution arrow are drawn, in the window's
    /// coordinates as last laid out; none while the line is off screen.
    pub fn gutter_point(&self, line: usize) -> Option<Point<Pixels>> {
        let layout = self.layout.as_ref()?;
        let offset = self.text.try_line_to_byte(line).ok()?;
        let row = self.rows.row_of(&self.text, offset);
        let y = self.line_height * (row as f32 + 0.5) - self.scroll_y;
        (y >= px(0.) && y < layout.viewport.size.height)
            .then(|| point(layout.viewport.origin.x + px(10.), layout.viewport.origin.y + y))
    }

    /// The line whose breakpoint column the pointer is in, if it is in it.
    fn gutter_line(&self, position: Point<Pixels>) -> Option<usize> {
        let layout = self.layout.as_ref().filter(|_| self.console.is_none())?;
        if !layout.viewport.contains(&position) || position.x >= layout.viewport.origin.x + px(20.) {
            return None;
        }
        let rows = self.rows.len(&self.text);
        let y = position.y - layout.viewport.origin.y + self.scroll_y;
        let row = (f32::from(y) / f32::from(self.line_height)).floor().max(0.) as usize;
        Some(self.rows.row(&self.text, row.min(rows.saturating_sub(1))).line)
    }

    /// The arrow's drag let go: on another line the program goes on from there; on its own line it was a click.
    fn on_mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selecting = false;
        self.column_drag = None;
        let Some(from) = self.arrow_drag.take() else { return };
        match self
            .gutter_line(event.position)
            .or_else(|| self.row_for_point(event.position).map(|row| self.rows.row(&self.text, row).line))
        {
            Some(line) if line != from => cx.emit(MoveExecution { line }),
            Some(line) => cx.emit(ToggleBreakpoint { line, tracepoint: false }),
            None => {}
        }
        cx.notify();
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.close_popup(cx);
        if self.git_popup.take().is_some() {
            cx.notify();
        }
        self.assist.template = None;
        if let Some(layout) = &self.layout
            && let Some((_, hunk)) = layout.git_changes.iter().find(|(bounds, _)| bounds.contains(&event.position))
        {
            let hunk = hunk.clone();
            let at = point(
                px(10.),
                (event.position.y - layout.viewport.origin.y + self.line_height)
                    .min((layout.viewport.size.height - px(240.)).max(px(0.))),
            );
            let buffer = self.buffer.read(cx);
            if !buffer.git_hunks.contains(&hunk) {
                return;
            }
            if let Some(baseline) = &buffer.git_base {
                let line_byte = |text: &Rope, line: usize| text.try_line_to_byte(line).unwrap_or(text.len_bytes());
                let mut lines = baseline.split_inclusive('\n');
                let old_from: usize = lines.by_ref().take(hunk.before.start).map(str::len).sum();
                let old_len: usize = lines.take(hunk.before.len()).map(str::len).sum();
                let new = line_byte(&self.text, hunk.after.start)..line_byte(&self.text, hunk.after.end);
                let old_text = baseline[old_from..old_from + old_len].to_owned();
                let new_text = self.text.byte_slice(new.clone()).to_string();
                self.git_popup = Some(GitPopup {
                    at,
                    version: self.version,
                    baseline: baseline.clone(),
                    edit: Edit { range: new, text: old_text.clone() },
                    before: Arc::new(git_preview_lines(&old_text)),
                    after: Arc::new(git_preview_lines(&new_text)),
                });
                self.selecting = false;
                cx.notify();
                return;
            }
        }
        let hit = |marks: &[(Bounds<Pixels>, FoldRegion)]| {
            marks.iter().find(|(b, _)| b.contains(&event.position)).map(|(_, fold)| fold.clone())
        };
        if let Some(layout) = &self.layout
            && let Some(fold) = hit(&layout.chevrons).or_else(|| hit(&layout.placeholders))
        {
            self.toggle_fold(fold, cx);
            return;
        }
        if let Some(index) = self
            .layout
            .as_ref()
            .and_then(|l| l.swatches.iter().find(|(b, _)| b.contains(&event.position)).map(|(_, index)| *index))
        {
            self.selecting = false;
            self.pick_colour(index, window, cx);
            return;
        }
        // Rider's gutter: a click in the breakpoint column sets or clears one on its line, a Shift+click a tracepoint;
        // pressed on the execution arrow, it starts the arrow's drag, which the release ends.
        if let Some(line) = self.gutter_line(event.position) {
            self.selecting = false;
            if event.modifiers.shift {
                cx.emit(ToggleBreakpoint { line, tracepoint: true });
            } else if self.buffer.read(cx).execution_line == Some(line) {
                self.arrow_drag = Some(line);
            } else {
                cx.emit(ToggleBreakpoint { line, tracepoint: false });
            }
            cx.notify();
            return;
        }
        let offset = self.offset_for_point(event.position);
        let m = event.modifiers;
        if let Some(console) = &mut self.console {
            console.follow = false;
            // Only Cmd+click (Ctrl+click elsewhere) follows a console's link, as Rider's console does: a plain click
            // has to place a caret and start a selection, even at the start of an error's line.
            if m.secondary() {
                let line = self.text.byte_to_line(offset);
                let start = self.text.line_to_byte(line);
                let text = self.text.line(line).to_string();
                if let Some(found) = rsharp_core::run::link::find(&text, offset.saturating_sub(start)) {
                    match found {
                        rsharp_core::run::link::Link::Url { url, .. } => {
                            cx.emit(ConsoleLink(ConsoleTarget::Url(url)));
                        }
                        rsharp_core::run::link::Link::File { path, line, .. } => {
                            cx.emit(ConsoleLink(ConsoleTarget::File { path, line, column: 0 }));
                        }
                    }
                    return;
                }
                let prefix = text.find(": error").or_else(|| text.find(": warning"));
                if prefix.is_some_and(|end| offset.saturating_sub(start) <= end) {
                    let mut parser = rsharp_core::build::Parser::new(console.folder.clone());
                    parser.push(&text);
                    if let Some(problem) = parser.finish().into_iter().next()
                        && let Some(path) = problem.path
                    {
                        cx.emit(ConsoleLink(ConsoleTarget::File { path, line: problem.line, column: problem.column }));
                        return;
                    }
                }
            }
        }
        let text = self.text.clone();
        // Rider's macOS keymap adds a caret with Alt+Shift+click, its Visual Studio keymap with Ctrl+Alt+click.
        let adds_caret = if cfg!(target_os = "macos") { m.alt && m.shift } else { m.control && m.alt };
        if adds_caret {
            let mut carets = self.carets.as_slice().to_vec();
            match carets.iter().position(|s| s.range().contains(&offset) || s.head == offset) {
                Some(at) if carets.len() > 1 => drop(carets.remove(at)),
                Some(_) => {}
                None if carets.len() < MAX_CARETS => carets.push(Selection::caret(offset)),
                None => {}
            }
            self.set_selections(carets);
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
        // Cmd+click (Ctrl+click elsewhere) goes to the declaration of what was clicked, as Rider's does. A console has
        // no declaration to go to: its links are compiler locations, followed above.
        if m.secondary() && !m.shift && !m.alt && event.click_count == 1 && self.console.is_none() {
            self.set_selections(vec![Selection::caret(offset)]);
            self.go_to("textDocument/definition", "Declaration", cx);
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
        self.set_selections(vec![selection]);
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
        if event.pressed_button.is_none() {
            let at = self.text_under(event.position);
            self.pointer_at(at, cx);
        }
        if let (true, Some(start)) = (self.selecting, self.column_drag)
            && event.pressed_button == Some(MouseButton::Left)
        {
            let head = self.place_for_point(event.position);
            self.set_block(start, head, cx);
        } else if self.selecting && event.pressed_button == Some(MouseButton::Left) {
            let anchor = self.primary().anchor;
            let head = self.offset_for_point(event.position);
            self.set_selections(vec![Selection { anchor, head }]);
            self.autoscroll = true;
            cx.notify();
        }
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.git_popup = None;
        let delta = event.delta.pixel_delta(self.line_height);
        self.scroll_y = (self.scroll_y - delta.y).clamp(px(0.), self.max_scroll());
        let follow = self.scroll_y >= self.max_scroll() - self.line_height * 4.;
        if let Some(console) = &mut self.console {
            console.follow = follow;
        }
        cx.notify();
    }

    fn max_scroll(&self) -> Pixels {
        let content = self.line_height * self.rows.len(&self.text) as f32;
        let viewport = self.layout.as_ref().map_or(px(0.), |l| l.viewport.size.height);
        (content - viewport + self.line_height * 4.).max(px(0.))
    }

    pub fn scroll_offset(&self) -> Pixels {
        self.scroll_y
    }

    pub fn set_scroll_offset(&mut self, offset: Pixels, cx: &mut Context<Self>) {
        let offset = offset.clamp(px(0.), self.max_scroll());
        if offset == self.scroll_y {
            return;
        }
        self.scroll_y = offset;
        self.autoscroll = false;
        cx.notify();
    }

    fn git_popup_view(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let popup = self.git_popup.as_ref()?;
        if popup.version != self.version
            || !self.buffer.read(cx).git_base.as_ref().is_some_and(|base| Arc::ptr_eq(base, &popup.baseline))
        {
            self.git_popup = None;
            return None;
        }
        let width = self.layout.as_ref().map_or(px(600.), |l| (l.viewport.size.width - px(20.)).min(px(600.)));
        let height = px(20.) * popup.before.len().max(popup.after.len()).min(8) as f32;
        let column = |side: &'static str, title: &'static str, lines: Arc<Vec<String>>| {
            div()
                .flex_1()
                .min_w_0()
                .child(div().px(px(8.)).py(px(4.)).text_color(theme::text_secondary()).child(title))
                .child(
                    gpui::uniform_list(side, lines.len(), move |range, _, _| {
                        range
                            .map(|i| {
                                let text = lines[i].clone();
                                div()
                                    .h(px(20.))
                                    .px(px(8.))
                                    .font_family(theme::CODE_FONT)
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .debug_selector(move || format!("git {side}: {text}"))
                                    .child(lines[i].clone())
                            })
                            .collect()
                    })
                    .h(height)
                    .w_full(),
                )
        };
        Some(
            div()
                .id("git-change-popup")
                .debug_selector(|| "git change popup".into())
                .occlude()
                .absolute()
                .left(popup.at.x)
                .top(popup.at.y)
                .w(width)
                .rounded(px(6.))
                .bg(theme::popup())
                .border_1()
                .border_color(theme::popup_border())
                .shadow_lg()
                .text_size(px(13.))
                .text_color(theme::text())
                .cursor(CursorStyle::Arrow)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(div().flex().child(column("before", "Before", popup.before.clone())).child(column(
                    "after",
                    "After",
                    popup.after.clone(),
                )))
                .child(
                    div().flex().justify_end().p(px(6.)).child(
                        div()
                            .id("git-rollback-lines")
                            .debug_selector(|| "git rollback lines".into())
                            .cursor(CursorStyle::PointingHand)
                            .px(px(8.))
                            .py(px(4.))
                            .rounded(px(4.))
                            .hover(|d| d.bg(theme::button_hover()))
                            .child("Rollback Lines")
                            .on_click(cx.listener(|editor, _, window, cx| {
                                let Some(popup) = editor.git_popup.take() else {
                                    return;
                                };
                                if popup.version == editor.version
                                    && editor
                                        .buffer
                                        .read(cx)
                                        .git_base
                                        .as_ref()
                                        .is_some_and(|base| Arc::ptr_eq(base, &popup.baseline))
                                {
                                    let head = popup.edit.range.start;
                                    editor.apply(
                                        EditPlan {
                                            edits: vec![popup.edit],
                                            selections: vec![Selection::caret(head)],
                                            ..EditPlan::default()
                                        },
                                        cx,
                                    );
                                }
                                window.focus(&editor.focus);
                                cx.notify();
                            })),
                    ),
                )
                .into_any_element(),
        )
    }

    fn utf16_to_offset(&self, utf16: usize) -> usize {
        let text = &self.text;
        text.char_to_byte(text.utf16_cu_to_char(utf16.min(text.len_utf16_cu())))
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let text = &self.text;
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
    Up => |e, cx| if !e.popup_step(-1, cx) { e.vertical(-1, false, cx) };
    Down => |e, cx| if !e.popup_step(1, cx) { e.vertical(1, false, cx) };
    SelectUp => |e, cx| if e.column_mode { e.extend_block(-1, 0, cx) } else { e.vertical(-1, true, cx) };
    SelectDown => |e, cx| if e.column_mode { e.extend_block(1, 0, cx) } else { e.vertical(1, true, cx) };
    PageUp => |e, cx| { let p = e.page(); e.vertical(-p, false, cx) };
    PageDown => |e, cx| { let p = e.page(); e.vertical(p, false, cx) };
    SelectAll => |e, cx| { let len = e.text.len_bytes(); e.set_selections(vec![Selection { anchor: 0, head: len }]); cx.notify() };
    Backspace => |e, cx| { let plan = backspace(&e.text, e.carets.as_slice()); e.apply(plan, cx); e.backspaced(cx) };
    Delete => |e, cx| { let plan = delete(&e.text, e.carets.as_slice()); e.apply(plan, cx) };
    Newline => |e, cx| { if e.popup_choose(cx) { return } if e.console_input { cx.emit(ConsoleInput("\n".into())); return } let indent = e.buffer.read(cx).indent; let plan = newline(&e.text, e.carets.as_slice(), &indent); e.apply(plan, cx) };
    Tab => |e, cx| { if e.popup_choose(cx) || e.next_template_stop(cx) || e.expand_template(cx) { return } let indent = e.buffer.read(cx).indent; let plan = tab(&e.text, e.carets.as_slice(), &indent); e.apply(plan, cx) };
    Undo => |e, cx| e.undo(false, cx);
    Redo => |e, cx| e.undo(true, cx);
    Copy => |e, cx| { let text = e.selected_text(); if !text.is_empty() { cx.write_to_clipboard(ClipboardItem::new_string(text)) } };
    Cut => |e, cx| {
        let text = e.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            let plan = insert(&e.text, e.carets.as_slice(), "");
            e.apply(plan, cx);
        }
    };
    Paste => |e, cx| {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = text.replace("\r\n", "\n");
            if e.console_input {
                cx.emit(ConsoleInput(text));
                return;
            }
            let lines: Vec<&str> = text.split('\n').collect();
            let carets = e.carets.as_slice();
            let plan = if carets.len() > 1 && lines.len() == carets.len() {
                insert_each(carets, &lines)
            } else {
                insert(&e.text, carets, &text)
            };
            e.apply(plan, cx);
        }
    };
    SelectNextOccurrence => |e, cx| { let next = select_next_occurrence(&e.text, e.carets.as_slice()); e.set_carets(next, cx) };
    UnselectOccurrence => |e, cx| { let next = unselect_last(e.carets.as_slice()); e.set_carets(next, cx) };
    SelectAllOccurrences => |e, cx| { let next = select_all_occurrences(&e.text, e.carets.as_slice()); e.set_carets(next, cx) };
    CloneCaretAbove => |e, cx| { let next = clone_caret(&e.text, e.carets.as_slice(), -1); e.set_carets(next, cx) };
    CaretPerLine => |e, cx| { let next = caret_per_line(&e.text, e.carets.as_slice()); e.set_carets(next, cx) };
    ToggleColumnMode => |e, cx| { e.column_mode = !e.column_mode; cx.notify() };
    CloneCaretBelow => |e, cx| { let next = clone_caret(&e.text, e.carets.as_slice(), 1); e.set_carets(next, cx) };
    MatchBrace => |e, cx| e.match_brace(cx);
    ToggleFold => |e, cx| { if let Some(region) = e.fold_at_caret(None, cx) { e.toggle_fold(region, cx) } };
    ExpandRegion => |e, cx| { if let Some(region) = e.fold_at_caret(Some(true), cx) { e.toggle_fold(region, cx) } };
    CollapseRegion => |e, cx| { if let Some(region) = e.fold_at_caret(Some(false), cx) { e.toggle_fold(region, cx) } };
    ExpandAll => |e, cx| e.set_folded(Vec::clear, cx);
    CollapseAll => |e, cx| {
        let all = e.buffer.read(cx).tree.as_ref().map(|tree| folds(tree, &e.text)).unwrap_or_default();
        e.set_folded(|folded| *folded = all, cx)
    };
    ToggleSoftWrap => |e, cx| { e.soft_wrap = !e.soft_wrap; e.autoscroll = true; cx.notify() };
    ToggleWhitespace => |_, cx| { let mut s = crate::settings::settings(cx).clone(); s.show_whitespace = !s.show_whitespace; crate::settings::apply(s, cx) };
    IncreaseFontSize => |e, cx| e.zoom(|size| size + 1., cx);
    DecreaseFontSize => |e, cx| e.zoom(|size| size - 1., cx);
    ResetFontSize => |e, cx| e.zoom(|_| crate::settings::AppSettings::default().editor_font_size, cx);
    Complete => |e, cx| e.request_completion(None, true, cx);
    ParameterInfo => |e, cx| e.request_signature(false, cx);
    QuickDocumentation => |e, cx| { let head = e.primary().head; e.request_hover(head, false, cx) };
    ShowContextActions => |e, cx| e.request_actions(cx);
    Reformat => |e, cx| e.reformat(cx);
    GoToDeclaration => |e, cx| e.go_to("textDocument/definition", "Declaration", cx);
    GoToImplementation => |e, cx| e.go_to("textDocument/implementation", "Implementation", cx);
    GoToTypeDeclaration => |e, cx| e.go_to("textDocument/typeDefinition", "Type Declaration", cx);
    FindUsages => |e, cx| e.find_usages(cx);
    BaseSymbols => |e, cx| e.go_to_related(true, cx);
    DerivedSymbols => |e, cx| e.go_to_related(false, cx);
    OptimizeUsings => |e, cx| e.named_refactoring("Optimize Usings", &[&["Remove unnecessary usings"], &["Sort Usings"]], cx);
    IntroduceVariable => |e, cx| e.named_refactoring("Introduce Variable", &[&["Introduce local"]], cx);
    IntroduceField => |e, cx| e.named_refactoring("Introduce Field", &[&["Introduce field", "Introduce constant"]], cx);
    IntroduceParameter => |e, cx| e.named_refactoring("Introduce Parameter", &[&["Introduce parameter"]], cx);
    EncapsulateField => |e, cx| e.named_refactoring("Encapsulate Field", &[&["Encapsulate field"]], cx);
    ExtractMethod => |e, cx| e.named_refactoring("Extract Method", &[&["Extract method"]], cx);
    Inline => |e, cx| e.named_refactoring("Inline", &[&["Inline"]], cx);
    SurroundWith => |e, cx| e.open_surround(cx);
    CompleteStatement => |e, cx| e.complete_statement(cx);
    ChangeSignature => |e, cx| e.open_refactoring(crate::refactor_dialog::Kind::ChangeSignature, cx);
    ExtractInterface => |e, cx| e.open_refactoring(crate::refactor_dialog::Kind::ExtractInterface, cx);
    MoveType => |e, cx| e.open_refactoring(crate::refactor_dialog::Kind::Move, cx);
    PullMembersUp => |e, cx| e.open_refactoring(crate::refactor_dialog::Kind::PullMembersUp, cx);
    PushMembersDown => |e, cx| e.open_refactoring(crate::refactor_dialog::Kind::PushMembersDown, cx);
    ExtractResource => |e, cx| e.xaml_extract(crate::xaml_dialog::Extract::Resource, cx);
    ExtractStyle => |e, cx| e.xaml_extract(crate::xaml_dialog::Extract::Style, cx);
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
        if self.console.is_none() {
            self.refresh_assist(cx);
        }
        let popup = self.popup_view(cx);
        let git_popup = self.git_popup_view(cx);
        let element = div()
            .id("code-editor")
            .debug_selector(|| "editor surface".into())
            .key_context("CodeEditor")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .cursor(CursorStyle::IBeam)
            .bg(theme::island())
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|e, event: &MouseDownEvent, _, cx| {
                    if let Some(line) = e.gutter_line(event.position) {
                        cx.emit(EditBreakpointAt { line });
                    }
                }),
            )
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_hover(cx.listener(|e, hovered: &bool, _, cx| {
                if !hovered && e.gutter_hovered {
                    e.gutter_hovered = false;
                    cx.notify();
                }
                if !hovered {
                    e.pointer_at(None, cx);
                }
            }))
            .on_scroll_wheel(cx.listener(Self::on_scroll));
        bind_moves(element, cx)
            .on_action(cx.listener(Self::escape))
            .on_action(cx.listener(|e, _: &Rename, window, cx| e.start_rename(window, cx)))
            .on_action(cx.listener(|e, _: &SafeDelete, window, cx| e.safe_delete(window, cx)))
            .child(EditorElement { editor: cx.entity() })
            .children(popup)
            .children(git_popup)
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
        Some(self.text.byte_slice(range).to_string())
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
        // Surround With's list takes the digit that chooses an item, as Rider's does.
        if let Some(assist::Popup::Surround { items, .. }) = &self.assist.popup
            && let Some(chosen) = items.iter().position(|i| i.digit.is_some_and(|d| text.chars().eq([d])))
            && let Some(assist::Popup::Surround { items, target, .. }) = self.assist.popup.take()
        {
            self.surround(&items[chosen], target, cx);
            return;
        }
        // A console whose program reads its input takes what is typed as the program's, not as its own text.
        if self.console_input {
            cx.emit(ConsoleInput(text.to_string()));
            return;
        }
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
                let plan = insert(&self.text, self.carets.as_slice(), text);
                let plan = EditPlan { kind: EditKind::Typing, ..plan };
                self.apply(plan, cx);
                self.typed(text, cx);
                return;
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
        let text = &self.text;
        let row = self.rows.row_of(text, offset);
        let shaped = layout.lines.get(row.checked_sub(layout.first_row)?)?;
        let span = self.rows.row(text, row);
        let x = shaped.x_for_index(offset - span.start) + layout.char_width * span.indent as f32;
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
    lines: Vec<Shaped>,
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
    fold_marks: Vec<Option<(FoldRegion, bool)>>,
    placeholder: ShapedLine,
    /// The bracket by the caret and its pair, in a tree that is current.
    braces: Vec<Range<usize>>,
    show_whitespace: bool,
    /// The file's breakpoints and the line the debugger is stopped at.
    breakpoints: Vec<crate::buffer::BreakpointMark>,
    execution_line: Option<usize>,
    /// Each colour the file writes or names, by its line, its colour and its place in the buffer's list; and the
    /// gutter's column for their swatches, none in a file without colours.
    swatches: Vec<(usize, Hsla, usize)>,
    swatch_column: Pixels,
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
        self.editor.update(cx, |editor, cx| {
            editor.line_height = line_height;
            let digits = editor.text.len_lines().to_string().len().max(3);
            let char_width = window
                .text_system()
                .shape_line("0".into(), font_size, &[run(1, theme::editor::text().into(), &code_font)], None)
                .width;
            // The numbers, then the folding column against the code.
            let show_numbers = settings.show_line_numbers && editor.console.is_none();
            // Rider's colour swatches, in a column of their own beside the numbers while the file has colours.
            let swatch_column =
                if editor.console.is_none() && !editor.buffer.read(cx).colours.is_empty() { px(16.) } else { px(0.) };
            let gutter_width = if editor.console.is_some() {
                px(8.)
            } else if show_numbers {
                char_width * digits as f32 + px(32.)
            } else {
                px(24.)
            } + swatch_column;
            let text_left = bounds.origin.x + gutter_width + px(TEXT_PAD) - px(16.);
            editor.columns = Some(((bounds.right() - text_left - px(TEXT_PAD)) / char_width).floor().max(8.) as usize);
            editor.reveal_carets();
            editor.ensure_rows();
            let text = &editor.text;
            let rows = &editor.rows;
            let caret_row = rows.row_of(text, editor.primary().head);
            let mut caret_rows: Vec<usize> =
                editor.carets.iter().filter(|s| s.is_empty()).map(|s| rows.row_of(text, s.head)).collect();
            caret_rows.sort_unstable();
            caret_rows.dedup();
            let mut caret_lines: Vec<usize> =
                editor.carets.as_slice().iter().map(|s| text.byte_to_line(s.head)).collect();
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
            let buffer = editor.buffer.clone();
            let buffer = buffer.read(cx);
            let breakpoints = buffer.breakpoints.clone();
            let execution_line = buffer.execution_line;
            let swatches: Vec<(usize, Hsla, usize)> = buffer
                .colours
                .iter()
                .enumerate()
                .filter_map(|(i, c)| Some((c.line, swatch_colour(&c.colour)?, i)))
                .collect();
            let inline_values = buffer.inline_values.clone();
            let exception = buffer.exception.clone();
            let syntax: Vec<Highlight> =
                buffer.tree.as_ref().map(|tree| highlights(tree, text, byte_range.clone())).unwrap_or_default();
            // Roslyn's colours win where it named a word; the syntax's fill the rest.
            let tokens: Vec<(Range<usize>, Hsla)> = editor
                .assist
                .tokens
                .iter()
                .filter(|(r, _)| r.end > byte_range.start && r.start < byte_range.end)
                .cloned()
                .collect();
            let mut colours: Vec<(Range<usize>, Hsla)> = syntax
                .into_iter()
                .filter(|h| !tokens.iter().any(|(r, _)| r.start < h.range.end && h.range.start < r.end))
                .map(|h| (h.range, theme::syntax(h.kind)))
                .collect();
            colours.extend(tokens);
            if let Some(console) = &editor.console {
                colours.extend(console.errors.iter().map(|range| (range.clone(), theme::error().into())));
            }
            colours.sort_by_key(|(r, _)| r.start);
            editor.assist.visible_lines = shown.first().map_or(0, |r| r.line)..shown.last().map_or(0, |r| r.line + 1);

            let len = text.len_bytes();
            let line_of = |byte: usize| text.byte_to_line(byte.min(len));
            let mut lines = Vec::with_capacity(shown.len());
            let mut line_numbers = Vec::with_capacity(shown.len());
            let mut fold_marks = Vec::with_capacity(shown.len());
            // The regions on screen, found in the tree; a folded one is among them, its first line being shown.
            let regions = buffer.tree.as_ref().map(|tree| folds_in(tree, text, byte_range.clone())).unwrap_or_default();
            for row in &shown {
                let content: String = text.byte_slice(row.start..row.end).to_string();
                let runs = runs_for(&content, row.start, &colours, &code_font);
                // The hints on the row, a wrapped row's last place being the next row's first; the debugger's inline
                // values are one more, at the stopped row's end.
                let mut hints: Vec<(usize, &str)> = editor
                    .assist
                    .hints
                    .iter()
                    .filter(|(at, _)| (row.start..row.end).contains(at) || (*at == row.end && row.last))
                    .map(|(at, label)| (at - row.start, label.as_str()))
                    .collect();
                // The exception a stop is on is named first, then the frame's values.
                if let Some((line, said)) = &exception
                    && *line == row.line
                    && row.last
                {
                    hints.push((row.end - row.start, said.as_str()));
                }
                if let Some((line, values)) = &inline_values
                    && *line == row.line
                    && row.last
                {
                    hints.push((row.end - row.start, values.as_str()));
                }
                lines.push(Shaped::new(&content, runs, &hints, font_size, &code_font, window));
                let number = (row.first && show_numbers).then(|| {
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
                let from = regions.partition_point(|f| line_of(f.range.start) < row.line);
                let mark = regions[from..]
                    .iter()
                    .take_while(|f| line_of(f.range.start) == row.line)
                    .find(|f| line_of(f.range.end) > row.line)
                    .map(|f| (f.clone(), editor.folded.contains(f)));
                fold_marks.push(mark);
            }
            let placeholder = window.text_system().shape_line(
                "...".into(),
                font_size,
                &[run(3, theme::editor::folded_text().into(), &code_font)],
                None,
            );
            let primary = editor.primary();
            let braces = match (&buffer.tree, primary.is_empty() && buffer.tree_version == editor.version) {
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
                breakpoints,
                execution_line,
                swatches,
                swatch_column,
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
        let text = editor.text.clone();
        let selections = editor.carets.as_slice().to_vec();
        let diagnostics = editor.buffer.read(cx).diagnostics.clone();
        let gutter_hovered = editor.gutter_hovered;
        let git_hunks = editor.buffer.read(cx).git_hunks.clone();
        let git_lines = editor.buffer.read(cx).document.text().len_lines();
        let git_visible = editor.console.is_none();
        let shown_bytes = state.rows.first().map_or(0, |r| r.start)..state.rows.last().map_or(0, |r| r.end);
        let from = editor.search_matches.partition_point(|m| m.end < shown_bytes.start);
        let to = editor.search_matches.partition_point(|m| m.start <= shown_bytes.end);
        let matches = editor.search_matches[from..to.max(from)].to_vec();
        let usages = editor.assist.usages.clone();
        let caret_rows: Vec<usize> = selections.iter().map(|s| editor.rows.row_of(&text, s.head)).collect();
        let top_of = |row: usize| state.text_origin.y + line_height * row as f32;
        let mut chevrons = Vec::new();
        let mut swatches = Vec::new();
        let mut placeholders = Vec::new();
        let mut git_changes = Vec::new();

        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            let shown = state.first_row..state.first_row + state.lines.len();
            for &row in state.caret_rows.iter().filter(|r| shown.contains(*r)) {
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
                if span.first && state.execution_line == Some(span.line) {
                    window.paint_quad(fill(
                        Bounds::new(point(bounds.origin.x, top), size(bounds.size.width, line_height)),
                        theme::editor::execution_point(),
                    ));
                }
                let left = state.text_origin.x + state.char_width * span.indent as f32;
                let band = |window: &mut Window, x0: Pixels, x1: Pixels, colour: gpui::Rgba| {
                    window.paint_quad(fill(
                        Bounds::from_corners(point(left + x0, top), point(left + x1, top + line_height)),
                        colour,
                    ));
                };
                for (x0, x1) in shaped.hint_spans() {
                    let chip = Bounds::from_corners(
                        point(left + x0 - px(1.), top + px(3.)),
                        point(left + x1 + px(1.), top + line_height - px(3.)),
                    );
                    window.paint_quad(fill(chip, theme::editor::inlay_hint_background()).corner_radii(px(3.)));
                }
                for m in matches.iter().filter(|m| m.start <= end && m.end >= start && !m.is_empty()) {
                    let (x0, x1) =
                        (shaped.x_for_index(m.start.max(start) - start), shaped.x_for_index(m.end.min(end) - start));
                    band(window, x0, x1, theme::editor::search_result());
                }
                for (usage, write) in usages.iter().filter(|(u, _)| u.start <= end && u.end >= start && !u.is_empty()) {
                    let (x0, x1) = (
                        shaped.x_for_index(usage.start.max(start) - start),
                        shaped.x_for_index(usage.end.min(end) - start),
                    );
                    band(
                        window,
                        x0,
                        x1,
                        if *write { theme::editor::usage_write() } else { theme::editor::usage_read() },
                    );
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
                if git_visible {
                    let last_line = state
                        .fold_marks
                        .get(i)
                        .and_then(|f| f.as_ref())
                        .filter(|(_, folded)| *folded)
                        .filter(|_| span.last)
                        .map_or(span.line, |(fold, _)| text.byte_to_line(fold.range.end.min(text.len_bytes())));
                    let from = git_hunks.partition_point(|h| {
                        if h.after.is_empty() {
                            h.after.start.min(git_lines.saturating_sub(1)) < span.line
                        } else {
                            h.after.end <= span.line
                        }
                    });
                    if let Some(hunk) = git_hunks.get(from).filter(|h| {
                        if h.after.is_empty() {
                            h.after.start.min(git_lines.saturating_sub(1)) <= last_line
                        } else {
                            h.after.start <= last_line
                        }
                    }) {
                        let color = if hunk.after.is_empty() {
                            theme::editor::diff_deleted()
                        } else if hunk.before.is_empty() {
                            theme::editor::diff_inserted()
                        } else {
                            theme::modified()
                        };
                        window.paint_quad(fill(
                            Bounds::new(point(bounds.origin.x + px(2.), top), size(px(3.), line_height)),
                            color,
                        ));
                        git_changes
                            .push((Bounds::new(point(bounds.origin.x, top), size(px(10.), line_height)), hunk.clone()));
                    }
                }
                if span.first
                    && let Some(mark) = state.breakpoints.iter().find(|mark| mark.line == span.line)
                {
                    // Rider's dot: full red where the debugger will hit it, a warning where it will not, dim where
                    // it is off.
                    let (diameter, colour) = match (mark.enabled, mark.verified) {
                        (true, true) => (px(10.), theme::editor::breakpoint()),
                        (true, false) => (px(8.), theme::warning()),
                        (false, _) => (px(8.), theme::text_secondary()),
                    };
                    let dot = Bounds::new(
                        point(
                            bounds.origin.x + px(7.) + (px(10.) - diameter) / 2.,
                            top + (line_height - diameter) / 2.,
                        ),
                        size(diameter, diameter),
                    );
                    window.paint_quad(fill(dot, colour).corner_radii(diameter / 2.));
                }
                if let Some(Some(number)) = state.line_numbers.get(i) {
                    let _ = number.paint(
                        point(bounds.origin.x + state.gutter_width - number.width - px(20.) - state.swatch_column, top),
                        line_height,
                        window,
                        cx,
                    );
                }
                if span.first
                    && let Some(&(_, colour, index)) = state.swatches.iter().find(|(line, ..)| *line == span.line)
                {
                    let swatch = Bounds::new(
                        point(bounds.origin.x + state.gutter_width - px(32.), top + (line_height - px(12.)) / 2.),
                        size(px(12.), px(12.)),
                    );
                    window.paint_quad(
                        fill(swatch, colour)
                            .corner_radii(px(2.))
                            .border_widths(px(1.))
                            .border_color(theme::control_border()),
                    );
                    swatches.push((swatch, index));
                }
                if let Some(Some((fold, folded))) = state.fold_marks.get(i).filter(|_| span.first) {
                    let fold = fold.clone();
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
                        fold,
                    ));
                }
                let _ = shaped.paint(point(left, top), line_height, window, cx);
                if let Some(Some((fold, true))) = state.fold_marks.get(i).filter(|_| span.last) {
                    let x = left + shaped.width + state.char_width;
                    let chip = Bounds::new(
                        point(x, top + px(2.)),
                        size(state.placeholder.width + state.char_width, line_height - px(4.)),
                    );
                    window.paint_quad(fill(chip, theme::editor::folded_background()).corner_radii(px(3.)));
                    let _ = state.placeholder.paint(point(x + state.char_width / 2., top), line_height, window, cx);
                    placeholders.push((chip, fold.clone()));
                }
                if state.show_whitespace {
                    let content = text.byte_slice(start..end).to_string();
                    for (at, c) in content.char_indices().filter(|(_, c)| *c == ' ' || *c == '\t') {
                        let (x0, x1) = (shaped.x_for_index(at), shaped.x_for_index(at + 1));
                        let mid = left + (x0 + x1) / 2.;
                        let y = top + line_height / 2.;
                        let mark = if c == ' ' {
                            Bounds::new(point(mid - px(1.), y - px(1.)), size(px(2.), px(2.)))
                        } else {
                            Bounds::new(point(left + x0 + px(2.), y), size(x1 - x0 - px(4.), px(1.)))
                        };
                        window.paint_quad(fill(mark, theme::editor::whitespace()));
                    }
                }
                for diagnostic in diagnostics.iter().filter(|d| d.range.start <= end && d.range.end >= start) {
                    let x0 = shaped.x_for_index(diagnostic.range.start.max(start) - start);
                    let x1 = shaped.x_for_index(diagnostic.range.end.min(end) - start).max(x0 + px(6.));
                    let colour: Hsla = if diagnostic.error { theme::error().into() } else { theme::warning().into() };
                    window.paint_underline(
                        point(left + x0, top + line_height - px(3.)),
                        x1 - x0,
                        &UnderlineStyle { color: Some(colour), thickness: px(1.), wavy: true },
                    );
                }
                if focused {
                    for (selection, _) in selections.iter().zip(&caret_rows).filter(|(_, r)| **r == row) {
                        let x = shaped.x_for_index(selection.head.clamp(start, end) - start);
                        window.paint_quad(fill(
                            Bounds::new(point(left + x, top + px(2.)), size(px(2.), line_height - px(4.))),
                            theme::editor::caret(),
                        ));
                    }
                }
            }
        });

        let lines = std::mem::take(&mut state.lines);
        let (first_row, text_origin, char_width) = (state.first_row, state.text_origin, state.char_width);
        self.editor.update(cx, |editor, _| {
            editor.layout = Some(Layout {
                text_origin,
                char_width,
                viewport: bounds,
                first_row,
                lines,
                chevrons,
                placeholders,
                git_changes,
                swatches,
            })
        });
    }
}

fn git_preview_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<_> =
        text.split_inclusive('\n').map(|line| line.trim_end_matches(['\r', '\n']).to_owned()).collect();
    if text.is_empty() {
        lines.push("(empty)".to_owned());
    } else if !text.ends_with('\n') {
        lines.push("\\ No newline at end of file".to_owned());
    }
    lines
}

/// A row as shaped: its text with the inlay hints set in it, and where each hint went (its byte in the row and
/// its length), so a place in the text finds its place on screen past them; the line's own fields and painting are
/// the shaped line's.
struct Shaped {
    line: ShapedLine,
    hints: Vec<(usize, usize)>,
}

impl std::ops::Deref for Shaped {
    type Target = ShapedLine;
    fn deref(&self) -> &ShapedLine {
        &self.line
    }
}

impl Shaped {
    fn new(
        content: &str,
        runs: Vec<TextRun>,
        hints: &[(usize, &str)],
        size: Pixels,
        font: &gpui::Font,
        window: &mut Window,
    ) -> Self {
        if hints.is_empty() {
            let line = window.text_system().shape_line(content.to_string().into(), size, &runs, None);
            return Self { line, hints: Vec::new() };
        }
        let colour: Hsla = theme::editor::inlay_hint().into();
        let (mut shown, mut shown_runs, mut placed) = (String::new(), Vec::new(), Vec::new());
        let mut runs = runs.into_iter();
        let mut current = runs.next();
        let mut at = 0;
        for &(hint_at, label) in hints.iter().chain(std::iter::once(&(content.len(), ""))) {
            // The text up to the hint, its runs cut where the hint falls.
            while at < hint_at {
                let Some(mut run) = current.take() else { break };
                let take = run.len.min(hint_at - at);
                shown_runs.push(TextRun { len: take, ..run.clone() });
                run.len -= take;
                at += take;
                current = if run.len > 0 { Some(run) } else { runs.next() };
            }
            shown.push_str(&content[shown.len() - placed.iter().map(|(_, l)| l).sum::<usize>()..hint_at]);
            if !label.is_empty() {
                placed.push((hint_at, label.len()));
                shown.push_str(label);
                shown_runs.push(run(label.len(), colour, font));
            }
        }
        let line = window.text_system().shape_line(shown.into(), size, &shown_runs, None);
        Self { line, hints: placed }
    }

    /// Where a byte of the row is drawn: past every hint before it or at it.
    fn x_for_index(&self, index: usize) -> Pixels {
        let past: usize = self.hints.iter().filter(|(at, _)| *at <= index).map(|(_, len)| len).sum();
        self.line.x_for_index(index + past)
    }

    /// The byte of the row a place on the shaped line stands for; a place inside a hint is the hint's own byte.
    fn text_index(&self, shown: usize) -> usize {
        let mut past = 0;
        for &(at, len) in &self.hints {
            if shown < at + past {
                break;
            }
            if shown < at + past + len {
                return at;
            }
            past += len;
        }
        shown - past
    }

    fn closest_index_for_x(&self, x: Pixels) -> usize {
        self.text_index(self.line.closest_index_for_x(x))
    }

    fn index_for_x(&self, x: Pixels) -> Option<usize> {
        self.line.index_for_x(x).map(|i| self.text_index(i))
    }

    /// Each hint's span on screen, for its chip.
    fn hint_spans(&self) -> Vec<(Pixels, Pixels)> {
        let mut past = 0;
        self.hints
            .iter()
            .map(|&(at, len)| {
                let span = (self.line.x_for_index(at + past), self.line.x_for_index(at + past + len));
                past += len;
                span
            })
            .collect()
    }
}

fn run(len: usize, color: Hsla, font: &gpui::Font) -> TextRun {
    TextRun { len, font: font.clone(), color, background_color: None, underline: None, strikethrough: None }
}

/// A line's text runs: its coloured pieces, plain text between them.
fn runs_for(content: &str, line_start: usize, colours: &[(Range<usize>, Hsla)], font: &gpui::Font) -> Vec<TextRun> {
    let plain: Hsla = theme::editor::text().into();
    let mut runs = Vec::new();
    let mut at = 0;
    let line_end = line_start + content.len();
    for (range, colour) in colours.iter().filter(|(r, _)| r.end > line_start && r.start < line_end) {
        let from = range.start.max(line_start) - line_start;
        let to = range.end.min(line_end) - line_start;
        // A tree a parse has yet to catch up with may cut a character; its colour waits for the parse.
        if from < at || !content.is_char_boundary(from) || !content.is_char_boundary(to) {
            continue;
        }
        if from > at {
            runs.push(run(from - at, plain, font));
        }
        runs.push(run(to - from, *colour, font));
        at = to;
    }
    if at < content.len() || runs.is_empty() {
        runs.push(run(content.len() - at, plain, font));
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open<'a>(
        name: &str,
        text: &'static [u8],
        cx: &'a mut gpui::TestAppContext,
    ) -> (Entity<CodeEditor>, &'a mut gpui::VisualTestContext) {
        cx.update(|cx| {
            crate::init(std::env::temp_dir().join(format!("rsharp-editor-{name}-{}", std::process::id())), cx);
            cx.global_mut::<crate::state::Ide>().servers = false;
            cx.global_mut::<crate::templates::Templates>().enabled = false;
        });
        let (editor, cx) = cx.add_window_view(|_, cx| {
            let buffer = cx.new(|cx| Buffer::new(PathBuf::from("Program.cs"), text, cx));
            CodeEditor::new(buffer, cx)
        });
        editor.update_in(cx, |editor, window, _| window.focus(&editor.focus));
        (editor, cx)
    }

    #[gpui::test]
    fn pasting_into_a_console_sends_the_clipboard_to_the_program(cx: &mut gpui::TestAppContext) {
        let (editor, cx) = open("paste-input", b"", cx);
        editor.update(cx, |editor, _| {
            editor.console(PathBuf::new());
            editor.console_input(true);
        });
        let inputs = Arc::new(std::sync::Mutex::new(Vec::new()));
        let received = inputs.clone();
        cx.update(|_, cx| {
            cx.subscribe(&editor, move |_, input: &ConsoleInput, _| received.lock().unwrap().push(input.0.clone()))
                .detach();
            cx.write_to_clipboard(ClipboardItem::new_string("Ada\r\nLovelace".into()));
        });
        cx.dispatch_action(Paste);
        cx.dispatch_action(Newline);
        cx.run_until_parked();
        assert_eq!(*inputs.lock().unwrap(), ["Ada\nLovelace", "\n"], "Paste lost the program's input");
        assert_eq!(editor.read_with(cx, |editor, _| editor.text.to_string()), "", "input changed the console's output");
    }

    #[gpui::test]
    fn an_exception_and_its_local_values_are_both_drawn_on_the_stopped_line(cx: &mut gpui::TestAppContext) {
        let (editor, cx) = open("exception-values", b"values[index];", cx);
        editor.update(cx, |editor, cx| {
            editor.buffer.update(cx, |buffer, cx| {
                buffer.set_exception(Some((0, "  IndexOutOfRangeException".into())), cx);
                buffer.set_inline_values(Some((0, "  index = 5".into())), cx);
            });
        });
        cx.run_until_parked();
        let shown = editor.read_with(cx, |editor, _| editor.layout.as_ref().unwrap().lines[0].line.text.to_string());
        assert!(shown.contains("IndexOutOfRangeException"), "the exception is missing from the drawn line: {shown}");
        assert!(shown.contains("index = 5"), "the exception hid the stopped frame's values: {shown}");
    }

    #[test]
    fn a_place_on_a_hinted_line_is_its_byte_past_the_hints_before_it() {
        // "var text": a 7-byte hint ("string ") set in before `text`, at byte 4.
        let shaped = Shaped { line: ShapedLine::default(), hints: vec![(4, 7)] };
        assert_eq!(shaped.text_index(2), 2, "a place before the hint moved");
        assert_eq!(shaped.text_index(6), 4, "a place inside the hint is not the hint's own byte");
        assert_eq!(shaped.text_index(12), 5, "a place past the hint is not shifted back by its length");
    }
}

/// A colour as the XAML host gives it, `#AARRGGBB`, as the swatch paints it.
fn swatch_colour(hex: &str) -> Option<Hsla> {
    let value = u32::from_str_radix(hex.strip_prefix('#')?, 16).ok()?;
    let part = |shift: u32| ((value >> shift) & 0xFF) as f32 / 255.;
    Some(gpui::Rgba { r: part(16), g: part(8), b: part(0), a: part(24) }.into())
}
