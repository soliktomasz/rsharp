//! A single-line text field: Find Action's, Settings' search, and later every popup's. Its editing is
//! `rsharp-text`'s commands and the editor's own actions, so a key does here what it does in the editor.

use crate::editor::{
    Backspace, Copy, Cut, Delete, Left, LineEnd, LineStart, Paste, Right, SelectAll, SelectLeft, SelectLineEnd,
    SelectLineStart, SelectRight, WordLeft, WordRight,
};
use crate::theme;
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, MouseButton, MouseDownEvent, Pixels, ShapedLine, SharedString, Style, TextRun, UTF16Selection, Window,
    div, fill, point, prelude::*, px, relative, size,
};
use rsharp_text::{
    Document, Edit, EditPlan, Rope, Selection, backspace, delete, insert, next_char, previous_char, word_left,
    word_right,
};
use std::ops::Range;

/// The field's text changed.
pub struct Changed;

pub struct TextField {
    document: Document,
    focus: FocusHandle,
    placeholder: SharedString,
    marked: Option<Range<usize>>,
    layout: Option<(ShapedLine, Bounds<Pixels>)>,
}

impl EventEmitter<Changed> for TextField {}

impl TextField {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            document: Document::new(""),
            focus: cx.focus_handle(),
            placeholder: placeholder.into(),
            marked: None,
            layout: None,
        }
    }

    pub fn text(&self) -> String {
        self.document.text().to_string()
    }

    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let len = self.document.text().len_bytes();
        self.apply(
            EditPlan {
                edits: vec![Edit { range: 0..len, text: text.to_string() }],
                selections: vec![Selection::caret(text.len())],
                ..Default::default()
            },
            cx,
        );
    }

    /// Selects the whole text, as a field opened again with its last query does, so typing replaces it.
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let len = self.document.text().len_bytes();
        self.document.set_selections(vec![Selection { anchor: 0, head: len }]);
        cx.notify();
    }

    fn caret(&self) -> Selection {
        self.document.selections()[0]
    }

    fn apply(&mut self, mut plan: EditPlan, cx: &mut Context<Self>) {
        // One line: a pasted break becomes a space, as Rider's search fields take it.
        for edit in &mut plan.edits {
            if edit.text.contains(['\n', '\r']) {
                let flat = edit.text.replace("\r\n", " ").replace(['\n', '\r'], " ");
                let caret_shift = edit.text.len() as isize - flat.len() as isize;
                for s in &mut plan.selections {
                    s.anchor = (s.anchor as isize - caret_shift) as usize;
                    s.head = (s.head as isize - caret_shift) as usize;
                }
                edit.text = flat;
            }
        }
        let changed = !plan.edits.is_empty();
        self.document.apply(plan);
        if changed {
            cx.emit(Changed);
        }
        cx.notify();
    }

    fn select(&mut self, extend: bool, to: impl Fn(&Rope, usize) -> usize, cx: &mut Context<Self>) {
        let caret = self.caret();
        let head = to(self.document.text(), caret.head);
        self.document.set_selections(vec![Selection { anchor: if extend { caret.anchor } else { head }, head }]);
        cx.notify();
    }

    fn utf16_to_offset(&self, utf16: usize) -> usize {
        let text = self.document.text();
        text.char_to_byte(text.utf16_cu_to_char(utf16.min(text.len_utf16_cu())))
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let text = self.document.text();
        text.char_to_utf16_cu(text.byte_to_char(offset))
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        if let Some((line, bounds)) = &self.layout {
            let offset = line.closest_index_for_x(event.position.x - bounds.left());
            let anchor = if event.modifiers.shift { self.caret().anchor } else { offset };
            self.document.set_selections(vec![Selection { anchor, head: offset }]);
            cx.notify();
        }
    }
}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TextField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("text-field")
            .key_context("TextField")
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .flex_1()
            .h(px(20.))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_action(cx.listener(|f, _: &Left, _, cx| {
                let c = f.caret();
                if c.is_empty() { f.select(false, previous_char, cx) } else { f.select(false, |_, _| c.start(), cx) }
            }))
            .on_action(cx.listener(|f, _: &Right, _, cx| {
                let c = f.caret();
                if c.is_empty() { f.select(false, next_char, cx) } else { f.select(false, |_, _| c.end(), cx) }
            }))
            .on_action(cx.listener(|f, _: &SelectLeft, _, cx| f.select(true, previous_char, cx)))
            .on_action(cx.listener(|f, _: &SelectRight, _, cx| f.select(true, next_char, cx)))
            .on_action(cx.listener(|f, _: &WordLeft, _, cx| f.select(false, word_left, cx)))
            .on_action(cx.listener(|f, _: &WordRight, _, cx| f.select(false, word_right, cx)))
            .on_action(cx.listener(|f, _: &LineStart, _, cx| f.select(false, |_, _| 0, cx)))
            .on_action(cx.listener(|f, _: &LineEnd, _, cx| f.select(false, |t, _| t.len_bytes(), cx)))
            .on_action(cx.listener(|f, _: &SelectLineStart, _, cx| f.select(true, |_, _| 0, cx)))
            .on_action(cx.listener(|f, _: &SelectLineEnd, _, cx| f.select(true, |t, _| t.len_bytes(), cx)))
            .on_action(cx.listener(|f, _: &SelectAll, _, cx| {
                let len = f.document.text().len_bytes();
                f.document.set_selections(vec![Selection { anchor: 0, head: len }]);
                cx.notify()
            }))
            .on_action(cx.listener(|f, _: &Backspace, _, cx| {
                let p = backspace(f.document.text(), f.document.selections());
                f.apply(p, cx)
            }))
            .on_action(cx.listener(|f, _: &Delete, _, cx| {
                let p = delete(f.document.text(), f.document.selections());
                f.apply(p, cx)
            }))
            .on_action(cx.listener(|f, _: &Copy, _, cx| {
                let c = f.caret();
                if !c.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        f.document.text().byte_slice(c.range()).to_string(),
                    ))
                }
            }))
            .on_action(cx.listener(|f, _: &Cut, _, cx| {
                let c = f.caret();
                if !c.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        f.document.text().byte_slice(c.range()).to_string(),
                    ));
                    let p = insert(f.document.text(), f.document.selections(), "");
                    f.apply(p, cx)
                }
            }))
            .on_action(cx.listener(|f, _: &Paste, _, cx| {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    let p = insert(f.document.text(), f.document.selections(), &text);
                    f.apply(p, cx)
                }
            }))
            .child(FieldElement { field: cx.entity() })
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.utf16_to_offset(range.start)..self.utf16_to_offset(range.end);
        actual.replace(self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end));
        Some(self.document.text().byte_slice(range).to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let c = self.caret();
        Some(UTF16Selection {
            range: self.offset_to_utf16(c.start())..self.offset_to_utf16(c.end()),
            reversed: c.head < c.anchor,
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
        let range = range
            .map(|r| self.utf16_to_offset(r.start)..self.utf16_to_offset(r.end))
            .or(self.marked.take())
            .unwrap_or(self.caret().range());
        let caret = range.start + text.len();
        self.apply(
            EditPlan {
                edits: vec![Edit { range, text: text.to_string() }],
                selections: vec![Selection::caret(caret)],
                ..Default::default()
            },
            cx,
        );
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = range
            .as_ref()
            .map(|r| self.utf16_to_offset(r.start))
            .or(self.marked.as_ref().map(|m| m.start))
            .unwrap_or(self.caret().start());
        self.replace_text_in_range(range, text, window, cx);
        self.marked = (!text.is_empty()).then_some(start..start + text.len());
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (line, bounds) = self.layout.as_ref()?;
        let x = line.x_for_index(self.utf16_to_offset(range.start));
        Some(Bounds::new(point(bounds.left() + x, bounds.top()), size(px(2.), bounds.size.height)))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let (line, bounds) = self.layout.as_ref()?;
        Some(self.offset_to_utf16(line.closest_index_for_x(point.x - bounds.left())))
    }
}

struct FieldElement {
    field: Entity<TextField>,
}

impl IntoElement for FieldElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for FieldElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<ShapedLine>;

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
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<ShapedLine> {
        let field = self.field.read(cx);
        let style = window.text_style();
        let (text, color): (SharedString, gpui::Hsla) = match field.document.text().len_bytes() {
            0 => (field.placeholder.clone(), theme::text_secondary().into()),
            _ => (field.document.text().to_string().into(), style.color),
        };
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        Some(window.text_system().shape_line(text, style.font_size.to_pixels(window.rem_size()), &[run], None))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        line: &mut Option<ShapedLine>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let line = line.take().expect("prepaint shaped the line");
        let focus = self.field.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.field.clone()), cx);
        let field = self.field.read(cx);
        let caret = field.caret();
        let empty = field.document.text().len_bytes() == 0;
        if !caret.is_empty() {
            let (x0, x1) = (line.x_for_index(caret.start()), line.x_for_index(caret.end()));
            window.paint_quad(fill(
                Bounds::from_corners(
                    point(bounds.left() + x0, bounds.top()),
                    point(bounds.left() + x1, bounds.bottom()),
                ),
                theme::selection(),
            ));
        }
        let _ = line.paint(bounds.origin, bounds.size.height, window, cx);
        if focus.is_focused(window) {
            let x = if empty { px(0.) } else { line.x_for_index(caret.head) };
            window.paint_quad(fill(
                Bounds::new(
                    point(bounds.left() + x, bounds.top() + px(2.)),
                    size(px(1.5), bounds.size.height - px(4.)),
                ),
                theme::text(),
            ));
        }
        self.field.update(cx, |f, _| f.layout = Some((line, bounds)));
    }
}
