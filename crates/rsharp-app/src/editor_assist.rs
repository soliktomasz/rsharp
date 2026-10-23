//! What the editor asks Roslyn as a person works, each answer taken only while the text and caret are as asked:
//! completion as a word is typed, parameter info inside a call, the hover card, usage highlights where the caret
//! rests, the colours of what names are (for the lines on screen), context actions, Reformat Code, Rename, and the
//! places Go to Declaration and Find Usages go, which the window opens.

use super::*;
use crate::field::TextField;
use crate::find_action::{Choose, Dismiss};
use crate::language::{self, ask, offset};
use gpui::{AnyElement, FontWeight, Task};
use rsharp_lsp::{self as lsp, CodeAction, CompletionItem, CompletionList, Location, Signature};
use serde_json::json;
use std::time::Duration;

/// Rows a list popup shows at once.
const LIST_ROWS: usize = 10;

pub enum Popup {
    /// The completion list: the server's items, those the word typed matches (best first) and the chosen one; `start`
    /// is where the word began.
    Completion { list: CompletionList, shown: Vec<usize>, selected: usize, start: usize, explicit: bool },
    /// Parameter info over the caret.
    Signature(Signature),
    /// The hover card or Quick Documentation, for the word at the anchor; a card the pointer opened goes when it
    /// leaves the word.
    Card { text: String, from_mouse: bool, word: Range<usize> },
    /// Context actions (Alt+Enter), the chosen one.
    Actions { actions: Vec<CodeAction>, selected: usize },
    /// Rename's field over the word being renamed.
    Rename { field: Entity<TextField>, word: Range<usize> },
}

/// Go to Declaration's or Find Usages' places, for the window to open: one is gone to, several are listed.
pub struct Navigate {
    pub title: String,
    pub to: Vec<Location>,
    pub list: bool,
}

/// Something the editor says in the status bar: nothing found, or a change refused.
pub struct Said(pub String);

impl gpui::EventEmitter<Navigate> for CodeEditor {}
impl gpui::EventEmitter<Said> for CodeEditor {}

#[derive(Default)]
pub struct Assist {
    pub popup: Option<Popup>,
    /// Where the popup hangs: the word's start, or the caret.
    pub popup_at: usize,
    popup_task: Option<Task<()>>,
    /// The usages of what the caret is on, each written or only read; asked for the text's version and caret.
    pub usages: Vec<(Range<usize>, bool)>,
    usages_for: Option<(u64, usize)>,
    usage_task: Option<Task<()>>,
    /// Roslyn's colours over the syntax's, for the lines asked; carried through edits until asked again.
    pub tokens: Vec<(Range<usize>, Hsla)>,
    tokens_for: Option<(u64, Range<usize>)>,
    token_task: Option<Task<()>>,
    /// The lines the last frame showed.
    pub visible_lines: Range<usize>,
    hover_task: Option<Task<()>>,
    hover_at: Option<usize>,
}

impl Assist {
    /// After a change: the colours carried through it, the usages dropped until asked again.
    pub fn map_through(&mut self, changes: &[Change]) {
        for (range, _) in &mut self.tokens {
            for change in changes {
                *range = map_through(range.start, change)..map_through(range.end, change);
            }
        }
        self.usages.clear();
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Where the word ending at `offset` starts.
fn word_start(text: &Rope, offset: usize) -> usize {
    let mut at = text.byte_to_char(offset);
    while at > 0 && is_word(text.char(at - 1)) {
        at -= 1;
    }
    text.char_to_byte(at)
}

/// An offset carried through sorted, disjoint edits against the text it is in.
fn map_offset(offset: usize, edits: &[Edit]) -> usize {
    let mut shift: isize = 0;
    for edit in edits {
        if edit.range.end <= offset {
            shift += edit.text.len() as isize - edit.range.len() as isize;
        } else if edit.range.start < offset {
            return (edit.range.start as isize + shift) as usize + edit.text.len();
        }
    }
    (offset as isize + shift) as usize
}

impl CodeEditor {
    /// The completion list's labels as shown, best first, while it is open.
    pub fn completion_shown(&self) -> Option<Vec<String>> {
        match &self.assist.popup {
            Some(Popup::Completion { list, shown, .. }) => {
                Some(shown.iter().map(|&i| list.items[i].label.clone()).collect())
            }
            _ => None,
        }
    }

    /// The hover card's or parameter info's text while one is open.
    pub fn popup_text(&self) -> Option<String> {
        match &self.assist.popup {
            Some(Popup::Card { text, .. }) => Some(text.clone()),
            Some(Popup::Signature(signature)) => Some(signature.label.clone()),
            _ => None,
        }
    }

    /// Roslyn has loaded the projects: what it said about the file before is asked again.
    pub fn server_ready(&mut self, cx: &mut Context<Self>) {
        self.assist.tokens_for = None;
        self.assist.usages_for = None;
        cx.notify();
    }

    fn server(&self, cx: &App) -> Option<(std::sync::Arc<rsharp_lsp::LanguageServer>, String)> {
        self.buffer.read(cx).server()
    }

    pub fn close_popup(&mut self, cx: &mut Context<Self>) {
        if self.assist.popup.take().is_some() {
            self.assist.popup_task = None;
            cx.notify();
        }
    }

    fn show(&mut self, popup: Popup, at: usize, cx: &mut Context<Self>) {
        self.assist.popup = Some(popup);
        self.assist.popup_at = at;
        cx.notify();
    }

    /// The server's edits to this file, as one undo step, the carets carried through them.
    fn apply_server_edits(&mut self, edits: Vec<Edit>, cx: &mut Context<Self>) {
        if edits.is_empty() {
            return;
        }
        let selections = self
            .carets
            .iter()
            .map(|s| Selection { anchor: map_offset(s.anchor, &edits), head: map_offset(s.head, &edits) })
            .collect();
        self.apply(EditPlan { edits, selections, ..Default::default() }, cx);
    }

    /// After typing: completion opened at a word's first letter or a dot and kept to the word, parameter info at
    /// a call's parenthesis or comma.
    pub(super) fn typed(&mut self, typed: &str, cx: &mut Context<Self>) {
        let mut chars = typed.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            self.close_popup(cx);
            return;
        };
        let completing = matches!(self.assist.popup, Some(Popup::Completion { .. }));
        match c {
            c if is_word(c) => {
                if completing {
                    self.filter_completion(cx);
                    if matches!(&self.assist.popup, Some(Popup::Completion { list, .. }) if list.incomplete) {
                        self.request_completion(None, false, cx);
                    }
                } else if word_start(&self.text, self.primary().head) + c.len_utf8() == self.primary().head {
                    self.request_completion(None, false, cx);
                }
            }
            '.' => self.request_completion(Some('.'), false, cx),
            '(' | ',' => self.request_signature(true, cx),
            ')' if matches!(self.assist.popup, Some(Popup::Signature(_))) => self.close_popup(cx),
            _ if completing => self.close_popup(cx),
            _ => {}
        }
    }

    /// After Backspace: the list kept to the shorter word, or closed once the word is gone.
    pub(super) fn backspaced(&mut self, cx: &mut Context<Self>) {
        match &self.assist.popup {
            Some(Popup::Completion { start, .. }) if self.primary().head >= *start => self.filter_completion(cx),
            Some(Popup::Signature(_)) => {}
            Some(_) => self.close_popup(cx),
            None => {}
        }
    }

    pub(super) fn request_completion(&mut self, trigger: Option<char>, explicit: bool, cx: &mut Context<Self>) {
        let Some((server, uri)) = self.server(cx) else { return };
        let head = self.primary().head;
        let start = word_start(&self.text, head);
        let mut params = lsp::at(&uri, language::point(&self.text, head));
        params["context"] = match trigger {
            Some(c) => json!({ "triggerKind": 2, "triggerCharacter": c.to_string() }),
            None => json!({ "triggerKind": 1 }),
        };
        let answer = ask(&server, "textDocument/completion", params, cx);
        let list = cx.background_executor().spawn(async move { answer.await.map(|a| lsp::completion(&a)) });
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let Some(list) = list.await else { return };
            let _ = this.update(cx, |e, cx| {
                let head = e.primary().head;
                if head < start || e.text.byte_to_line(head) != e.text.byte_to_line(start) {
                    return;
                }
                let selected = match &e.assist.popup {
                    Some(Popup::Completion { selected, .. }) => *selected,
                    _ => 0,
                };
                e.show(Popup::Completion { list, shown: Vec::new(), selected, start, explicit }, start, cx);
                e.filter_completion(cx);
            });
        }));
    }

    /// The items the word typed so far matches, best first, the server's order among equals.
    fn filter_completion(&mut self, cx: &mut Context<Self>) {
        let head = self.primary().head;
        let Some(Popup::Completion { list, shown, selected, start, explicit }) = &mut self.assist.popup else { return };
        let typed = self.text.byte_slice(*start..head.max(*start)).to_string();
        let mut scored: Vec<(i32, usize)> = list
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| crate::fuzzy::score(&typed, &item.filter_text).map(|s| (s, i)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| list.items[a.1].sort_text.cmp(&list.items[b.1].sort_text)));
        *shown = scored.into_iter().map(|(_, i)| i).collect();
        *selected = (*selected).min(shown.len().saturating_sub(1));
        if shown.is_empty() && !*explicit {
            self.assist.popup = None;
        }
        cx.notify();
    }

    /// Up or Down in a list popup; false with none open, for the caret to move instead.
    pub(super) fn popup_step(&mut self, by: isize, cx: &mut Context<Self>) -> bool {
        let (selected, len) = match &mut self.assist.popup {
            Some(Popup::Completion { selected, shown, .. }) => (selected, shown.len()),
            Some(Popup::Actions { selected, actions }) => (selected, actions.len()),
            _ => return false,
        };
        if len > 0 {
            *selected = (*selected as isize + by).rem_euclid(len as isize) as usize;
        }
        cx.notify();
        true
    }

    /// Enter or Tab in a list popup: its choice made; false with none open.
    pub(super) fn popup_choose(&mut self, cx: &mut Context<Self>) -> bool {
        match &self.assist.popup {
            Some(Popup::Completion { shown, .. }) if !shown.is_empty() => self.accept_completion(cx),
            Some(Popup::Actions { .. }) => self.choose_action(cx),
            _ => return false,
        }
        true
    }

    /// Writes the chosen item over the word typed, then any usings it needs, which resolving it gives.
    fn accept_completion(&mut self, cx: &mut Context<Self>) {
        let Some(Popup::Completion { list, shown, selected, start, .. }) = self.assist.popup.take() else { return };
        let item: CompletionItem = list.items[shown[selected]].clone();
        let head = self.primary().head.max(start);
        let (from, written) = match &item.edit {
            Some(edit) => (offset(&self.text, edit.span.start).min(start), edit.text.clone()),
            None => (start, item.label.clone()),
        };
        let caret = from + written.len();
        self.apply(
            EditPlan {
                edits: vec![Edit { range: from..head, text: written }],
                selections: vec![Selection::caret(caret)],
                ..Default::default()
            },
            cx,
        );
        let Some((server, _)) = self.server(cx) else { return };
        let version = self.version;
        let resolved = ask(&server, "completionItem/resolve", item.raw.clone(), cx);
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let additional = match item.additional.is_empty() {
                false => item.additional,
                true => resolved.await.map(|r| lsp::text_edits(&r["additionalTextEdits"])).unwrap_or_default(),
            };
            let _ = this.update(cx, |e, cx| {
                // Only what lands before the word (a using) still holds once the word is written.
                let edits: Vec<Edit> =
                    language::edits(&e.text, &additional).into_iter().filter(|a| a.range.end <= from).collect();
                if e.version == version {
                    e.apply_server_edits(edits, cx);
                }
            });
        }));
    }

    /// Parameter info: the signature of the call the caret is in; `typing` keeps quiet where there is none.
    pub(super) fn request_signature(&mut self, typing: bool, cx: &mut Context<Self>) {
        let Some((server, uri)) = self.server(cx) else { return };
        let head = self.primary().head;
        let mut params = lsp::at(&uri, language::point(&self.text, head));
        params["context"] = json!({ "triggerKind": if typing { 2 } else { 1 }, "isRetrigger": false });
        if typing {
            params["context"]["triggerCharacter"] =
                json!(self.text.char(self.text.byte_to_char(head).saturating_sub(1)).to_string());
        }
        let answer = ask(&server, "textDocument/signatureHelp", params, cx);
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let found = answer.await.and_then(|a| lsp::signature(&a));
            let _ = this.update(cx, |e, cx| match found {
                Some(signature) if e.primary().head == head => e.show(Popup::Signature(signature), head, cx),
                Some(_) => {}
                None if typing => {}
                None => cx.emit(Said("No parameter info here".into())),
            });
        }));
    }

    /// Quick Documentation, or the card the pointer asked for at `at`: the hover's text, with the problems there.
    pub(super) fn request_hover(&mut self, at: usize, from_mouse: bool, cx: &mut Context<Self>) {
        let problems: Vec<String> = self
            .buffer
            .read(cx)
            .diagnostics
            .iter()
            .filter(|d| d.range.start <= at && at <= d.range.end.max(d.range.start + 1))
            .map(|d| d.message.clone())
            .collect();
        let word = word_at(&self.text, at).range();
        let server = self.server(cx);
        let answer = server.map(|(server, uri)| {
            ask(&server, "textDocument/hover", lsp::at(&uri, language::point(&self.text, at)), cx)
        });
        let version = self.version;
        let task = cx.spawn(async move |this, cx| {
            if from_mouse {
                cx.background_executor().timer(Duration::from_millis(500)).await;
            }
            let hover = match answer {
                Some(answer) => answer.await.and_then(|a| lsp::hover(&a)),
                None => None,
            };
            let text: Vec<String> = problems.into_iter().chain(hover).collect();
            let _ = this.update(cx, |e, cx| {
                if e.version != version || (from_mouse && e.assist.hover_at != Some(at)) {
                    return;
                }
                if text.is_empty() {
                    if !from_mouse {
                        cx.emit(Said("No documentation here".into()));
                    }
                    return;
                }
                e.show(Popup::Card { text: text.join("\n\n"), from_mouse, word: word.clone() }, word.start, cx);
            });
        });
        if from_mouse {
            self.assist.hover_task = Some(task);
        } else {
            self.assist.popup_task = Some(task);
        }
    }

    /// The pointer resting on the text: a card for the word under it, once it has stayed there.
    pub(super) fn pointer_at(&mut self, at: Option<usize>, cx: &mut Context<Self>) {
        if let Some(Popup::Card { from_mouse: true, word, .. }) = &self.assist.popup
            && !at.is_some_and(|a| word.contains(&a))
        {
            self.close_popup(cx);
        }
        if at == self.assist.hover_at {
            return;
        }
        self.assist.hover_at = at;
        self.assist.hover_task = None;
        let busy =
            matches!(self.assist.popup, Some(Popup::Completion { .. } | Popup::Actions { .. } | Popup::Rename { .. }));
        if let Some(at) = at.filter(|_| !busy) {
            self.request_hover(at, true, cx);
        }
    }

    /// Go to Declaration, Implementation or Type Declaration from the caret.
    pub(super) fn go_to(&mut self, method: &'static str, title: &'static str, cx: &mut Context<Self>) {
        let Some((server, uri)) = self.server(cx) else { return };
        let answer = ask(&server, method, lsp::at(&uri, language::point(&self.text, self.primary().head)), cx);
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let to = answer.await.map(|a| lsp::locations(&a)).unwrap_or_default();
            let _ = this.update(cx, |_, cx| {
                if to.is_empty() {
                    cx.emit(Said(format!("Cannot find {} to go to", title.to_lowercase())));
                } else {
                    cx.emit(Navigate { title: title.to_string(), to, list: false });
                }
            });
        }));
    }

    pub(super) fn find_usages(&mut self, cx: &mut Context<Self>) {
        let Some((server, uri)) = self.server(cx) else { return };
        let head = self.primary().head;
        let name = self.text.byte_slice(word_at(&self.text, head).range()).to_string();
        let mut params = lsp::at(&uri, language::point(&self.text, head));
        params["context"] = json!({ "includeDeclaration": false });
        let answer = ask(&server, "textDocument/references", params, cx);
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let to = answer.await.map(|a| lsp::locations(&a)).unwrap_or_default();
            let _ = this.update(cx, |_, cx| {
                if to.is_empty() {
                    cx.emit(Said(format!("No usages of {name} found")));
                } else {
                    cx.emit(Navigate { title: format!("Usages of {name}"), to, list: true });
                }
            });
        }));
    }

    /// Reformat Code: the selection's lines, or the whole file, as the server lays them out with the file's indent.
    pub(super) fn reformat(&mut self, cx: &mut Context<Self>) {
        let Some((server, uri)) = self.server(cx) else { return };
        let indent = self.buffer.read(cx).indent;
        let options = json!({ "tabSize": indent.size, "insertSpaces": !indent.tabs });
        let selection = self.primary();
        let (method, params) = if selection.is_empty() {
            ("textDocument/formatting", json!({ "textDocument": { "uri": uri }, "options": options }))
        } else {
            let span = lsp::Span {
                start: language::point(&self.text, selection.start()),
                end: language::point(&self.text, selection.end()),
            };
            (
                "textDocument/rangeFormatting",
                json!({ "textDocument": { "uri": uri }, "range": span.to_json(), "options": options }),
            )
        };
        let answer = ask(&server, method, params, cx);
        let version = self.version;
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let Some(answer) = answer.await else { return };
            let _ = this.update(cx, |e, cx| {
                if e.version == version {
                    let edits = language::edits(&e.text, &lsp::text_edits(&answer));
                    e.apply_server_edits(edits, cx);
                }
            });
        }));
    }

    /// Alt+Enter: the context actions and quick fixes for the selection, or the caret's place.
    pub(super) fn request_actions(&mut self, cx: &mut Context<Self>) {
        let Some((server, uri)) = self.server(cx) else { return };
        let selection = self.primary();
        let span = lsp::Span {
            start: language::point(&self.text, selection.start()),
            end: language::point(&self.text, selection.end()),
        };
        let params =
            json!({ "textDocument": { "uri": uri }, "range": span.to_json(), "context": { "diagnostics": [] } });
        let answer = ask(&server, "textDocument/codeAction", params, cx);
        let head = selection.head;
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let mut actions = answer.await.map(|a| lsp::code_actions(&a)).unwrap_or_default();
            // Quick fixes first, as Rider lists them above the context actions.
            actions.sort_by_key(|a| !a.quick_fix);
            let _ = this.update(cx, |e, cx| {
                if actions.is_empty() {
                    cx.emit(Said("No context actions here".into()));
                } else if e.primary().head == head {
                    e.show(Popup::Actions { actions, selected: 0 }, head, cx);
                }
            });
        }));
    }

    /// The chosen action resolved and made; one that holds others (Roslyn's nested actions) lists them instead.
    fn choose_action(&mut self, cx: &mut Context<Self>) {
        let Some(Popup::Actions { actions, selected }) = self.assist.popup.take() else { return };
        let Some((server, _)) = self.server(cx) else { return };
        let action = actions[selected].clone();
        let answer = ask(&server, "codeAction/resolve", action.raw.clone(), cx);
        let head = self.primary().head;
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let Some(resolved) = answer.await else { return };
            let _ = this.update(cx, |e, cx| {
                let nested = resolved["command"]["arguments"][0]["NestedCodeActions"].clone();
                if nested.is_array() {
                    let actions = lsp::code_actions(&nested);
                    if !actions.is_empty() {
                        e.show(Popup::Actions { actions, selected: 0 }, head, cx);
                        return;
                    }
                }
                let edit = lsp::workspace_edit(&resolved["edit"]);
                if edit.is_empty() {
                    cx.emit(Said(format!("{} cannot be made here yet", action.title)));
                    return;
                }
                if let Err(why) = language::apply_workspace_edit(&edit, Some(&server), cx) {
                    cx.emit(Said(why));
                }
            });
        }));
    }

    /// Rename: a field over the word at the caret, its name selected.
    pub(super) fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.server(cx).is_none() {
            return;
        }
        let word = word_at(&self.text, self.primary().head).range();
        if word.is_empty() {
            cx.emit(Said("Nothing to rename here".into()));
            return;
        }
        let name = self.text.byte_slice(word.clone()).to_string();
        let field = cx.new(|cx| {
            let mut field = TextField::new("New name", cx);
            field.set_text(&name, cx);
            field.select_all(cx);
            field
        });
        window.focus(&field.focus_handle(cx));
        self.show(Popup::Rename { field, word: word.clone() }, word.start, cx);
    }

    fn finish_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Popup::Rename { field, word }) = self.assist.popup.take() else { return };
        window.focus(&self.focus);
        let name = field.read(cx).text().trim().to_string();
        let old = self.text.byte_slice(word.clone()).to_string();
        let Some((server, uri)) = self.server(cx).filter(|_| !name.is_empty() && name != old) else {
            cx.notify();
            return;
        };
        let mut params = lsp::at(&uri, language::point(&self.text, word.start));
        params["newName"] = json!(name);
        let answer = ask(&server, "textDocument/rename", params, cx);
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let edit = answer.await.map(|a| lsp::workspace_edit(&a)).unwrap_or_default();
            let _ = this.update(cx, |_, cx| {
                if edit.is_empty() {
                    cx.emit(Said(format!("{old} cannot be renamed")));
                } else if let Err(why) = language::apply_workspace_edit(&edit, Some(&server), cx) {
                    cx.emit(Said(why));
                }
            });
        }));
        cx.notify();
    }

    /// Asked as each frame is drawn: the usages of what the caret rests on, and Roslyn's colours for the lines on
    /// screen, each once typing pauses.
    pub(super) fn refresh_assist(&mut self, cx: &mut Context<Self>) {
        let Some((server, uri)) = self.server(cx) else { return };
        let (version, head) = (self.version, self.primary().head);
        if self.assist.usages_for != Some((version, head)) {
            self.assist.usages_for = Some((version, head));
            if !self.assist.usages.iter().any(|(r, _)| r.start <= head && head <= r.end) {
                self.assist.usages.clear();
            }
            let text = self.text.clone();
            let answer =
                ask(&server, "textDocument/documentHighlight", lsp::at(&uri, language::point(&text, head)), cx);
            self.assist.usage_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let found = answer.await.unwrap_or_default();
                let _ = this.update(cx, |e, cx| {
                    if e.assist.usages_for != Some((version, head)) {
                        return;
                    }
                    // The protocol's kind 3 is a write.
                    let writes = found.as_array().into_iter().flatten().map(|h| h["kind"] == 3);
                    e.assist.usages = lsp::highlights(&found)
                        .into_iter()
                        .zip(writes)
                        .map(|(span, write)| (offset(&text, span.start)..offset(&text, span.end), write))
                        .collect();
                    cx.notify();
                });
            }));
        }
        let lines = &self.assist.visible_lines;
        let covered = self
            .assist
            .tokens_for
            .as_ref()
            .is_some_and(|(v, l)| *v == version && l.start <= lines.start && lines.end <= l.end);
        if !covered && !lines.is_empty() {
            // A screen's worth either side, so a little scrolling needs no new answer.
            let span = lines.len();
            let wanted = lines.start.saturating_sub(span)..(lines.end + span).min(self.text.len_lines());
            self.assist.tokens_for = Some((version, wanted.clone()));
            let text = self.text.clone();
            // Roslyn refuses a range ending past the last line, so the last asked ends at the text's end.
            let end = match wanted.end < self.text.len_lines() {
                true => lsp::Point::new(wanted.end, 0),
                false => language::point(&text, text.len_bytes()),
            };
            let range = lsp::Span { start: lsp::Point::new(wanted.start, 0), end };
            let pause = if self.assist.tokens.is_empty() { Duration::ZERO } else { Duration::from_millis(300) };
            self.assist.token_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(pause).await;
                let params = json!({ "textDocument": { "uri": uri }, "range": range.to_json() });
                let Ok(answer) = this.update(cx, |_, cx| ask(&server, "textDocument/semanticTokens/range", params, cx))
                else {
                    return;
                };
                let legend = rsharp_lsp::token_legend(&server);
                let tokens = cx
                    .background_executor()
                    .spawn(async move {
                        let answer = answer.await?;
                        let tokens = lsp::semantic_tokens(&answer, &legend)
                            .into_iter()
                            .filter_map(|t| {
                                let colour = theme::semantic(&t.kind)?;
                                let end = lsp::Point::new(t.at.line, t.at.character + t.length);
                                Some((offset(&text, t.at)..offset(&text, end), colour))
                            })
                            .collect::<Vec<_>>();
                        Some(tokens)
                    })
                    .await;
                let _ = this.update(cx, |e, cx| {
                    if let Some(tokens) = tokens.filter(|_| e.version == version) {
                        e.assist.tokens = tokens;
                        cx.notify();
                    }
                });
            }));
        }
    }

    /// Where an offset is drawn, from the editor's top left: its row's top, and its left.
    fn place_on_screen(&mut self, offset: usize) -> Option<Point<Pixels>> {
        self.ensure_rows();
        let layout = self.layout.as_ref()?;
        let text = &self.text;
        let row = self.rows.row_of(text, offset);
        let shaped = layout.lines.get(row.checked_sub(layout.first_row)?)?;
        let span = self.rows.row(text, row);
        let x = shaped.x_for_index(offset.clamp(span.start, span.end) - span.start)
            + layout.char_width * span.indent as f32;
        let top = layout.text_origin.y + self.line_height * row as f32;
        Some(point(layout.text_origin.x + x, top) - layout.viewport.origin)
    }

    /// The popup open, hung at its place: a list or card under the line, parameter info over it.
    pub(super) fn popup_view(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let at = self.place_on_screen(self.assist.popup_at)?;
        let line_height = self.line_height;
        let code = theme::CODE_FONT;
        let frame = || {
            div()
                .id("assist-popup")
                .occlude()
                .absolute()
                .rounded(px(6.))
                .bg(theme::popup())
                .border_1()
                .border_color(theme::popup_border())
                .shadow_lg()
                .text_size(px(13.))
                .text_color(theme::text())
        };
        let row = |text: String, detail: String, chosen: bool| {
            div()
                .h(px(22.))
                .px(px(8.))
                .mx(px(4.))
                .rounded(px(4.))
                .flex()
                .items_center()
                .gap(px(16.))
                .when(chosen, |d| d.bg(theme::selection()))
                .child(div().flex_1().truncate().font_family(code).child(text))
                .child(div().text_color(theme::text_secondary()).truncate().child(detail))
        };
        let below = point(at.x, at.y + line_height);
        let element = match self.assist.popup.as_ref()? {
            Popup::Completion { list, shown, selected, .. } => {
                let first = selected.saturating_sub(LIST_ROWS - 1);
                let rows: Vec<_> = shown
                    .iter()
                    .enumerate()
                    .skip(first)
                    .take(LIST_ROWS)
                    .map(|(i, &item)| {
                        let item = &list.items[item];
                        row(item.label.clone(), item.detail.clone(), i == *selected)
                    })
                    .collect();
                frame()
                    .left(below.x - px(8.))
                    .top(below.y)
                    .w(px(460.))
                    .py(px(4.))
                    .children(rows)
                    .when(shown.is_empty(), |d| {
                        d.child(div().px(px(12.)).text_color(theme::text_secondary()).child("No suggestions"))
                    })
                    .into_any_element()
            }
            Popup::Actions { actions, selected } => {
                let first = selected.saturating_sub(LIST_ROWS - 1);
                let rows: Vec<_> = actions
                    .iter()
                    .enumerate()
                    .skip(first)
                    .take(LIST_ROWS)
                    .map(|(i, a)| {
                        row(
                            a.title.clone(),
                            if a.quick_fix { "Quick fix".into() } else { String::new() },
                            i == *selected,
                        )
                    })
                    .collect();
                frame().left(below.x).top(below.y).w(px(420.)).py(px(4.)).children(rows).into_any_element()
            }
            Popup::Card { text, .. } => frame()
                .left(below.x)
                .top(below.y)
                .max_w(px(560.))
                .p(px(10.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .children(text.lines().map(|l| div().min_h(px(8.)).child(l.to_string())))
                .into_any_element(),
            Popup::Signature(signature) => {
                let label = &signature.label;
                let (before, current, after) = match &signature.parameter {
                    Some(p) => (&label[..p.start], &label[p.clone()], &label[p.end..]),
                    None => (label.as_str(), "", ""),
                };
                frame()
                    .left(at.x)
                    .top(at.y - px(32.))
                    .h(px(28.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .font_family(code)
                    .child(before.to_string())
                    .child(
                        div().font_weight(FontWeight::BOLD).text_color(theme::on_accent()).child(current.to_string()),
                    )
                    .child(after.to_string())
                    .when(signature.overload.1 > 1, |d| {
                        d.child(div().pl(px(12.)).text_color(theme::text_secondary()).child(format!(
                            "{} of {}",
                            signature.overload.0 + 1,
                            signature.overload.1
                        )))
                    })
                    .into_any_element()
            }
            Popup::Rename { field, .. } => frame()
                .key_context("RenamePopup")
                .left(at.x - px(6.))
                .top(at.y - px(4.))
                .w(px(280.))
                .p(px(6.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .on_action(cx.listener(|e, _: &Choose, window, cx| e.finish_rename(window, cx)))
                .on_action(cx.listener(|e, _: &Dismiss, window, cx| {
                    e.assist.popup = None;
                    window.focus(&e.focus);
                    cx.notify();
                }))
                .child(div().text_color(theme::text_secondary()).child("Rename to"))
                .child(
                    div()
                        .h(px(26.))
                        .px(px(6.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(theme::accent())
                        .child(field.clone()),
                )
                .into_any_element(),
        };
        Some(element)
    }
}
