//! A console: the editor's plain text, selection and visible-line rendering, with bounded output. Build's and Run's
//! are the same view, each under its own name and its own window.

use crate::assets::icon;
use crate::buffer::Buffer;
use crate::editor::CodeEditor;
use crate::theme;
use gpui::{App, Context, Entity, EventEmitter, FocusHandle, Focusable, Window, div, prelude::*, px};
use rsharp_text::Rope;
use std::ops::Range;
use std::path::PathBuf;

pub enum ConsoleAction {
    Rerun,
    Stop,
}

impl EventEmitter<ConsoleAction> for ConsoleView {}

pub const OUTPUT_LIMIT: usize = 1024 * 1024;

pub struct ConsoleView {
    /// Which console this is, as its selectors and its output's name say: "build" or "run".
    pub name: &'static str,
    editor: Entity<CodeEditor>,
    text: Rope,
    errors: Vec<Range<usize>>,
    pub running: bool,
    pub can_rerun: bool,
}

impl ConsoleView {
    pub fn new(name: &'static str, cx: &mut Context<Self>) -> Self {
        let buffer = cx.new(|cx| Buffer::new(PathBuf::from(format!("{name}.output")), b"", cx));
        let editor = cx.new(|cx| {
            let mut editor = CodeEditor::new(buffer, cx);
            editor.console(PathBuf::new());
            editor
        });
        Self { name, editor, text: Rope::new(), errors: Vec::new(), running: false, can_rerun: false }
    }

    pub fn editor(&self) -> &Entity<CodeEditor> {
        &self.editor
    }

    pub fn text(&self) -> String {
        self.text.to_string()
    }

    pub fn start(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        self.running = true;
        self.can_rerun = true;
        self.editor.update(cx, |e, _| e.console(folder));
        self.clear(cx);
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        let removed = self.text.len_bytes();
        self.text = Rope::new();
        self.errors.clear();
        self.sync(removed, cx);
    }

    pub fn append(&mut self, chunks: &[(String, bool)], cx: &mut Context<Self>) {
        for (text, error) in chunks {
            let start = self.text.len_bytes();
            self.text.insert(self.text.len_chars(), text);
            if *error && !text.is_empty() {
                if let Some(previous) = self.errors.last_mut().filter(|range| range.end == start) {
                    previous.end += text.len();
                } else {
                    self.errors.push(start..start + text.len());
                }
            }
        }
        let mut removed = self.text.len_bytes().saturating_sub(OUTPUT_LIMIT);
        if removed > 0 {
            let mut chars = self.text.byte_to_char(removed);
            if self.text.char_to_byte(chars) < removed {
                chars += 1;
            }
            removed = self.text.char_to_byte(chars);
            self.text.remove(..chars);
            self.errors.retain_mut(|range| {
                range.start = range.start.saturating_sub(removed);
                range.end = range.end.saturating_sub(removed);
                range.start < range.end
            });
        }
        self.sync(removed, cx);
    }

    fn sync(&mut self, removed: usize, cx: &mut Context<Self>) {
        let (text, errors) = (self.text.clone(), self.errors.clone());
        self.editor.update(cx, |e, cx| e.output(text, errors, removed, cx));
        cx.notify();
    }
}

impl Focusable for ConsoleView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for ConsoleView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let console = self.name;
        let tool = move |id: &'static str, image: &'static str, enabled: bool| {
            div()
                .id(id)
                .debug_selector(move || format!("{console}: {id}"))
                .size(px(28.))
                .flex_none()
                .rounded(px(4.))
                .flex()
                .items_center()
                .justify_center()
                .when(enabled, |d| d.hover(|d| d.bg(theme::button_hover())))
                .when(!enabled, |d| d.opacity(0.35))
                .child(icon(image, 16.))
                .tooltip(move |_, cx| cx.new(|_| crate::window::Tip(id.into())).into())
        };
        let rerun = self.can_rerun && !self.running;
        let running = self.running;
        let name = self.name;
        div()
            .debug_selector(move || format!("{name} console"))
            .flex_1()
            .min_h_0()
            .flex()
            .child(
                div()
                    .w(px(36.))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(2.))
                    .py(px(4.))
                    .child(tool("Rerun", "rerun", rerun).on_click(cx.listener(move |_, _, _, cx| {
                        if rerun {
                            cx.emit(ConsoleAction::Rerun);
                        }
                    })))
                    .child(tool("Stop", "stop", running).on_click(cx.listener(move |_, _, _, cx| {
                        if running {
                            cx.emit(ConsoleAction::Stop);
                        }
                    })))
                    .child(tool("Clear", "clear", true).on_click(cx.listener(|v, _, _, cx| v.clear(cx)))),
            )
            .child(div().flex_1().min_w_0().child(self.editor.clone()))
    }
}
