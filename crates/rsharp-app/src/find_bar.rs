//! Find and replace in the open file, as Rider's bar over the editor: matches are found as the query is typed and
//! drawn in the editor, the one nearest after the caret selected, and the count says which of how many.

use crate::assets::icon;
use crate::editor::CodeEditor;
use crate::field::{Changed, TextField};
use crate::theme;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, MouseButton, SharedString, Subscription, Window,
    actions, div, prelude::*, px,
};
use rsharp_text::{
    EditPlan, MAX_MATCHES, SearchQuery, SearchResult, Selection, find_all, replace_all, replacement, word_at,
};

actions!(find, [Find, Replace, FindNext, FindPrevious, CloseFind, ReplaceOne]);

/// The bar was closed; the editor takes the keys again.
pub struct Closed;

pub struct FindBar {
    query: Entity<TextField>,
    with: Entity<TextField>,
    replacing: bool,
    match_case: bool,
    whole_words: bool,
    regex: bool,
    editor: Entity<CodeEditor>,
    result: SearchResult,
    current: Option<usize>,
    /// The document's version the result was found in, so an edit finds again and a caret move does not.
    version: u64,
    count: SharedString,
    watching: Subscription,
    _query_changed: Subscription,
}

impl EventEmitter<Closed> for FindBar {}

impl FindBar {
    pub fn new(editor: Entity<CodeEditor>, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| TextField::new("Search", cx));
        let with = cx.new(|cx| TextField::new("Replace", cx));
        let query_changed = cx.subscribe(&query, |bar, _, _: &Changed, cx| bar.search(true, cx));
        Self {
            query,
            with,
            replacing: false,
            match_case: false,
            whole_words: false,
            regex: false,
            watching: Self::watch(&editor, cx),
            editor,
            result: SearchResult::default(),
            current: None,
            version: u64::MAX,
            count: SharedString::default(),
            _query_changed: query_changed,
        }
    }

    fn watch(editor: &Entity<CodeEditor>, cx: &mut Context<Self>) -> Subscription {
        cx.observe(editor, |bar, editor, cx| {
            if editor.read(cx).document.version() != bar.version {
                bar.search(false, cx);
            }
        })
    }

    /// Follows the file another tab shows, its matches found there.
    pub fn attach(&mut self, editor: Entity<CodeEditor>, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| {
            e.search_matches.clear();
            cx.notify()
        });
        self.watching = Self::watch(&editor, cx);
        self.editor = editor;
        self.search(false, cx);
    }

    /// Opens the bar (with the replace row if `replacing`), its query the selection when that is on one line, or
    /// the word under the caret.
    pub fn open(&mut self, replacing: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.replacing = replacing;
        let editor = self.editor.read(cx);
        let text = editor.document.text();
        let primary = editor.primary();
        let word = word_at(text, primary.head);
        let seed = if primary.is_empty() {
            (!word.is_empty()).then(|| text.byte_slice(word.range()).to_string())
        } else if text.byte_to_line(primary.start()) == text.byte_to_line(primary.end()) {
            Some(text.byte_slice(primary.range()).to_string())
        } else {
            None
        };
        // The word under the caret is the match the search starts from, as Rider's, not the one after it.
        if primary.is_empty() && !word.is_empty() {
            self.editor.update(cx, |e, cx| e.select_range(word.range(), cx));
        }
        match seed {
            Some(seed) if seed != self.query.read(cx).text() => self.query.update(cx, |f, cx| f.set_text(&seed, cx)),
            _ => self.search(false, cx),
        }
        self.query.update(cx, |f, cx| f.select_all(cx));
        window.focus(&self.query.focus_handle(cx));
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| {
            e.search_matches.clear();
            cx.notify()
        });
        cx.emit(Closed);
    }

    /// What the count beside the arrows says: "2/5", "0 results", a broken pattern's reason.
    pub fn count(&self) -> &str {
        &self.count
    }

    fn query(&self, cx: &App) -> SearchQuery {
        SearchQuery {
            text: self.query.read(cx).text(),
            match_case: self.match_case,
            whole_words: self.whole_words,
            regex: self.regex,
        }
    }

    fn search(&mut self, jump: bool, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let editor = self.editor.read(cx);
        self.version = editor.document.version();
        self.result = find_all(editor.document.text(), &query, MAX_MATCHES);
        self.current = self.result.next_from(editor.primary().start());
        let matches = self.result.matches.clone();
        let select = self.current.filter(|_| jump).map(|i| matches[i].clone());
        self.editor.update(cx, |e, cx| {
            e.search_matches = matches;
            match select {
                Some(range) => e.select_range(range, cx),
                None => cx.notify(),
            }
        });
        self.show(cx);
    }

    /// Find Next (or Previous) from the editor, the bar left open where it is.
    pub fn find_next(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.step(forward, cx);
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.result.matches.is_empty() {
            return;
        }
        let primary = self.editor.read(cx).primary();
        let mut current =
            if forward { self.result.next_from(primary.end()) } else { self.result.previous_from(primary.start()) };
        // An empty match the caret already stands on (a line's start) is passed, or Next would stay on it.
        if let Some(i) = current
            && forward
            && primary.is_empty()
            && self.result.matches[i] == (primary.end()..primary.end())
        {
            current = Some((i + 1) % self.result.matches.len());
        }
        self.current = current;
        if let Some(i) = current {
            let range = self.result.matches[i].clone();
            self.editor.update(cx, |e, cx| e.select_range(range, cx));
        }
        self.show(cx);
    }

    /// Replace: the current match, once it is selected, and on to the next; a first press only selects it.
    fn replace_one(&mut self, cx: &mut Context<Self>) {
        let Some(range) = self.current.and_then(|i| self.result.matches.get(i)).cloned() else { return };
        let primary = self.editor.read(cx).primary();
        if primary.range() != range {
            self.editor.update(cx, |e, cx| e.select_range(range, cx));
            return;
        }
        let with = self.with.read(cx).text();
        let query = self.query(cx);
        let text = replacement(self.editor.read(cx).document.text(), &query, &range, &with);
        let after = range.start + text.len();
        let plan = EditPlan {
            edits: vec![rsharp_text::Edit { range, text }],
            selections: vec![Selection::caret(after)],
            ..Default::default()
        };
        self.editor.update(cx, |e, cx| e.apply_plan(plan, cx));
        self.search(false, cx);
        self.step(true, cx);
    }

    fn replace_all(&mut self, cx: &mut Context<Self>) {
        let with = self.with.read(cx).text();
        let query = self.query(cx);
        let edits = replace_all(self.editor.read(cx).document.text(), &query, &with);
        let Some(first) = edits.first().map(|e| e.range.start) else { return };
        let count = edits.len();
        let plan = EditPlan { edits, selections: vec![Selection::caret(first)], ..Default::default() };
        self.editor.update(cx, |e, cx| e.apply_plan(plan, cx));
        self.search(false, cx);
        self.count = format!("Replaced {count}").into();
        cx.notify();
    }

    fn show(&mut self, cx: &mut Context<Self>) {
        let total = self.result.matches.len();
        self.count = if let Some(problem) = &self.result.problem {
            problem.clone().into()
        } else if self.query.read(cx).text().is_empty() {
            SharedString::default()
        } else if total == 0 {
            "0 results".into()
        } else {
            let more = if self.result.truncated { "+" } else { "" };
            format!("{}/{total}{more}", self.current.map_or(0, |i| i + 1)).into()
        };
        cx.notify();
    }

    fn toggle(
        &self,
        id: &'static str,
        name: &'static str,
        on: bool,
        flip: fn(&mut Self),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .size(px(22.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .when(on, |d| d.bg(theme::button_pressed()))
            .when(!on, |d| d.hover(|d| d.bg(theme::button_hover())))
            .child(icon(name, 16.))
            .on_click(cx.listener(move |bar, _, _, cx| {
                flip(bar);
                bar.search(true, cx);
            }))
    }

    fn button(&self, id: &'static str, child: impl IntoElement) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .h(px(24.))
            .px(px(8.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .hover(|d| d.bg(theme::button_hover()))
            .active(|d| d.bg(theme::button_pressed()))
            .child(child)
    }

    fn field_box(field: Entity<TextField>) -> gpui::Div {
        div()
            .w(px(280.))
            .h(px(28.))
            .px(px(8.))
            .flex()
            .items_center()
            .rounded(px(4.))
            .border_1()
            .border_color(theme::control_border())
            .child(field)
    }
}

impl Focusable for FindBar {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.focus_handle(cx)
    }
}

impl Render for FindBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let problem = self.result.problem.is_some();
        let search_row = div()
            .h(px(36.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(
                Self::field_box(self.query.clone())
                    .gap(px(6.))
                    .when(problem, |d| d.border_color(theme::error()))
                    .child(self.toggle("match-case", "match-case", self.match_case, |b| b.match_case ^= true, cx))
                    .child(self.toggle("words", "words", self.whole_words, |b| b.whole_words ^= true, cx))
                    .child(self.toggle("regex", "regex", self.regex, |b| b.regex ^= true, cx)),
            )
            .child(
                div()
                    .min_w(px(72.))
                    .px(px(6.))
                    .when(problem, |d| d.text_color(theme::error()))
                    .when(!problem, |d| d.text_color(theme::text_muted()))
                    .child(self.count.clone()),
            )
            .child(self.button("previous", icon("up", 16.)).on_click(cx.listener(|b, _, _, cx| b.step(false, cx))))
            .child(self.button("next", icon("down", 16.)).on_click(cx.listener(|b, _, _, cx| b.step(true, cx))))
            .child(div().flex_1())
            .child(self.button("close-find", icon("close", 16.)).on_click(cx.listener(|b, _, _, cx| b.close(cx))));
        div()
            .key_context("FindBar")
            // A press on the bar's buttons leaves the keys where they were, not with the window behind it.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme::search_field())
            .border_b_1()
            .border_color(theme::frame())
            .on_action(cx.listener(|b, _: &FindNext, _, cx| b.step(true, cx)))
            .on_action(cx.listener(|b, _: &FindPrevious, _, cx| b.step(false, cx)))
            .on_action(cx.listener(|b, _: &CloseFind, _, cx| b.close(cx)))
            .on_action(cx.listener(|b, _: &ReplaceOne, _, cx| b.replace_one(cx)))
            .child(search_row)
            .when(self.replacing, |d| {
                d.child(
                    div()
                        .h(px(36.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(
                            Self::field_box(self.with.clone())
                                .id("replace-field")
                                .debug_selector(|| "replace-field".into())
                                .key_context("ReplaceField"),
                        )
                        .child(self.button("replace", "Replace").on_click(cx.listener(|b, _, _, cx| b.replace_one(cx))))
                        .child(
                            self.button("replace-all", "Replace All")
                                .on_click(cx.listener(|b, _, _, cx| b.replace_all(cx))),
                        ),
                )
            })
    }
}
