use super::{action, scratch_editor};
use crate::{editor::CodeEditor, theme};
use gpui::{App, Context, Entity, EventEmitter, FocusHandle, Focusable, Task, Window, div, prelude::*, px};
use rsharp_text::{MergeChoice, ThreeWayMerge};

#[derive(Clone, Debug)]
pub enum MergeRequest {
    Apply(String),
    Cancel,
}

/// An editable result has explicit conflict acknowledgements; typing alone never clears a conflict.
pub struct MergeView {
    focus: FocusHandle,
    yours: Entity<CodeEditor>,
    theirs: Entity<CodeEditor>,
    base: Entity<CodeEditor>,
    result: Entity<CodeEditor>,
    merge: Option<ThreeWayMerge>,
    selected: usize,
    show_base: bool,
    pending_result: Option<String>,
    scroll_offsets: [gpui::Pixels; 4],
    busy: bool,
    problem: Option<String>,
    _work: Task<()>,
}
impl EventEmitter<MergeRequest> for MergeView {}
impl Focusable for MergeView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl MergeView {
    pub fn new(base: String, yours: String, theirs: String, cx: &mut Context<Self>) -> Self {
        let base_editor = scratch_editor("BASE", &base, cx);
        let yours_editor = scratch_editor("YOURS", &yours, cx);
        let theirs_editor = scratch_editor("THEIRS", &theirs, cx);
        for editor in [&base_editor, &yours_editor, &theirs_editor] {
            let buffer = editor.read(cx).buffer.clone();
            buffer.update(cx, |b, _| b.read_only = true);
        }
        let result = scratch_editor("MERGE_RESULT", "", cx);
        for (index, editor) in [&yours_editor, &result, &theirs_editor, &base_editor].into_iter().enumerate() {
            cx.observe(editor, move |view, source, cx| {
                let offset = source.read(cx).scroll_offset();
                if view.scroll_offsets[index] == offset {
                    return;
                }
                view.scroll_offsets[index] = offset;
                let editors = [view.yours.clone(), view.result.clone(), view.theirs.clone(), view.base.clone()];
                for (i, editor) in editors.into_iter().enumerate() {
                    if i == index {
                        continue;
                    }
                    editor.update(cx, |e, cx| e.set_scroll_offset(offset, cx));
                    view.scroll_offsets[i] = editor.read(cx).scroll_offset();
                }
            })
            .detach();
        }
        let buffer = result.read(cx).buffer.clone();
        cx.observe(&buffer, |_, _, cx| cx.notify()).detach();
        let work = cx.background_executor().spawn(async move { ThreeWayMerge::new(&base, &yours, &theirs) });
        let task = cx.spawn(async move |this, cx| {
            let merge = work.await;
            let _ = this.update(cx, |v, cx| {
                let text = v.pending_result.take().unwrap_or_else(|| merge.result());
                v.merge = Some(merge);
                v.set_result(text, cx);
                cx.notify();
            });
        });
        Self {
            focus: cx.focus_handle(),
            yours: yours_editor,
            theirs: theirs_editor,
            base: base_editor,
            result,
            merge: None,
            selected: 0,
            show_base: false,
            pending_result: None,
            scroll_offsets: [px(0.); 4],
            busy: false,
            problem: None,
            _work: task,
        }
    }
    pub fn unresolved_count(&self) -> usize {
        self.merge.as_ref().map_or(0, ThreeWayMerge::unresolved_count)
    }
    pub fn result(&self, cx: &App) -> String {
        self.pending_result.clone().unwrap_or_else(|| self.result.read(cx).text().to_string())
    }
    pub fn result_editor(&self) -> &Entity<CodeEditor> {
        &self.result
    }
    pub fn set_problem(&mut self, problem: Option<String>, cx: &mut Context<Self>) {
        self.problem = problem;
        cx.notify();
    }
    pub fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.busy = busy;
        let buffer = self.result.read(cx).buffer.clone();
        buffer.update(cx, |buffer, cx| {
            buffer.read_only = busy;
            cx.notify();
        });
        cx.notify();
    }
    pub fn is_loading(&self) -> bool {
        self.merge.is_none()
    }
    pub fn set_result(&mut self, text: String, cx: &mut Context<Self>) {
        if self.merge.is_none() {
            self.pending_result = Some(text.clone());
        }
        let buffer = self.result.read(cx).buffer.clone();
        buffer.update(cx, |b, cx| b.replace_all(text, cx));
        cx.notify();
    }
    pub fn accept_chunk(&mut self, index: usize, choice: MergeChoice, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let edited = self.result(cx);
        if let Some(merge) = &mut self.merge {
            let before = merge.result();
            if merge.accept(index, choice) {
                let text = keep_manual_edits(&before, &merge.result(), &edited);
                self.set_result(text, cx);
            }
        }
    }
    pub fn acknowledge_chunk(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if let Some(chunk) = self.merge.as_mut().and_then(|m| m.chunks.get_mut(index)) {
            chunk.resolved = true;
            cx.notify();
        }
    }
    fn step(&mut self, back: bool, cx: &mut Context<Self>) {
        let Some(merge) = &self.merge else {
            return;
        };
        if merge.chunks.is_empty() {
            return;
        }
        self.selected = if back {
            self.selected.checked_sub(1).unwrap_or(merge.chunks.len() - 1)
        } else {
            (self.selected + 1) % merge.chunks.len()
        };
        let chunk = &merge.chunks[self.selected];
        let (left, right, base) = (chunk.left.start, chunk.right.start, chunk.base.start);
        self.yours.update(cx, |e, cx| e.place_caret(left, 0, cx));
        self.theirs.update(cx, |e, cx| e.place_caret(right, 0, cx));
        self.base.update(cx, |e, cx| e.place_caret(base, 0, cx));
        self.result.update(cx, |e, cx| e.place_caret(base, 0, cx));
        cx.notify();
    }
}

impl Render for MergeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loaded = self.merge.is_some();
        let unresolved = self.unresolved_count();
        let chunk = self.merge.as_ref().and_then(|m| m.chunks.get(self.selected));
        let active = chunk.is_some();
        let state = match chunk {
            Some(c) if c.is_conflict && !c.resolved => "Unresolved conflict",
            Some(c) if c.is_conflict => "Resolved conflict",
            Some(_) => "Change applied",
            None => "No changes",
        };
        div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::island())
            .text_color(theme::text())
            .font_family(theme::UI_FONT)
            .text_size(px(13.))
            .when_some(self.problem.clone(), |d, problem| {
                d.child(div().px(px(8.)).py(px(5.)).text_color(theme::error()).child(problem))
            })
            .child(
                div()
                    .flex_none()
                    .h(px(40.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().flex_1().child(if loaded {
                        format!("{unresolved} unresolved conflicts")
                    } else {
                        "Preparing merge…".into()
                    }))
                    .child(action("merge simple", "Resolve Simple Conflicts", loaded && unresolved > 0).on_click(
                        cx.listener(|v, _, _, cx| {
                            if v.busy {
                                return;
                            }
                            let edited = v.result(cx);
                            if let Some(merge) = &mut v.merge {
                                let before = merge.result();
                                merge.resolve_simple_conflicts();
                                let text = keep_manual_edits(&before, &merge.result(), &edited);
                                v.set_result(text, cx);
                            }
                        }),
                    ))
                    .child(action("merge base", "Show Base", loaded).on_click(cx.listener(|v, _, _, cx| {
                        v.show_base = !v.show_base;
                        cx.notify();
                    })))
                    .child(
                        action("merge cancel", "Cancel", !self.busy)
                            .when(!self.busy, |d| d.on_click(cx.listener(|_, _, _, cx| cx.emit(MergeRequest::Cancel)))),
                    )
                    .child(
                        action("merge apply", "Apply", loaded && unresolved == 0 && !self.busy)
                            .when(loaded && unresolved == 0 && !self.busy, |d| {
                                d.on_click(cx.listener(|v, _, _, cx| cx.emit(MergeRequest::Apply(v.result(cx)))))
                            }),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .h(px(34.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .border_b_1()
                    .border_color(theme::separator())
                    .child(
                        action("merge previous", "Previous", active)
                            .on_click(cx.listener(|v, _, _, cx| v.step(true, cx))),
                    )
                    .child(action("merge next", "Next", active).on_click(cx.listener(|v, _, _, cx| v.step(false, cx))))
                    .child(div().flex_1().child(format!(
                        "{} / {} · {state}",
                        if active { self.selected + 1 } else { 0 },
                        self.merge.as_ref().map_or(0, |m| m.chunks.len())
                    )))
                    .child(
                        action("merge accept yours", "Accept Yours", active)
                            .on_click(cx.listener(|v, _, _, cx| v.accept_chunk(v.selected, MergeChoice::Left, cx))),
                    )
                    .child(
                        action("merge accept theirs", "Accept Theirs", active)
                            .on_click(cx.listener(|v, _, _, cx| v.accept_chunk(v.selected, MergeChoice::Right, cx))),
                    )
                    .child(
                        action("merge accept both", "Accept Both", active)
                            .on_click(cx.listener(|v, _, _, cx| v.accept_chunk(v.selected, MergeChoice::Both, cx))),
                    )
                    .child(
                        action("merge accept base", "Accept Base", active)
                            .on_click(cx.listener(|v, _, _, cx| v.accept_chunk(v.selected, MergeChoice::Base, cx))),
                    )
                    .child(
                        action(
                            "merge acknowledge",
                            "Mark Resolved",
                            chunk.is_some_and(|c| c.is_conflict && !c.resolved),
                        )
                        .on_click(cx.listener(|v, _, _, cx| v.acknowledge_chunk(v.selected, cx))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(pane(
                        if self.show_base { "Base" } else { "Yours" },
                        if self.show_base { self.base.clone() } else { self.yours.clone() },
                    ))
                    .child(div().w(px(1.)).h_full().bg(theme::separator()))
                    .child(pane("Result — editable", self.result.clone()))
                    .child(div().w(px(1.)).h_full().bg(theme::separator()))
                    .child(pane("Theirs", self.theirs.clone())),
            )
    }
}
fn pane(title: &'static str, editor: Entity<CodeEditor>) -> impl IntoElement {
    div()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .flex()
        .flex_col()
        .child(div().h(px(28.)).flex_none().px(px(10.)).flex().items_center().child(title))
        .child(div().flex_1().min_h_0().child(editor))
}

/// Only the accepted ranges change; hand edits to other result lines survive later hunk choices.
fn keep_manual_edits(before: &str, after: &str, edited: &str) -> String {
    if before == edited {
        return after.to_string();
    }
    let manual = rsharp_text::diff_hunks(before, edited);
    let map = |line: usize, end: bool| {
        let mut offset = 0isize;
        for h in &manual {
            if line < h.before.start {
                break;
            }
            if line <= h.before.end {
                if line == h.before.end && !h.before.is_empty() {
                    return h.after.end;
                }
                return if end { h.after.end } else { h.after.start };
            }
            offset += h.after.len() as isize - h.before.len() as isize;
        }
        line.saturating_add_signed(offset)
    };
    let offsets = |text: &str| {
        std::iter::once(0)
            .chain(text.split_inclusive('\n').scan(0, |at, line| {
                *at += line.len();
                Some(*at)
            }))
            .collect::<Vec<_>>()
    };
    let current_offsets = offsets(edited);
    let after_offsets = offsets(after);
    let mut result = edited.to_string();
    for h in rsharp_text::diff_hunks(before, after).into_iter().rev() {
        let start = current_offsets[map(h.before.start, false).min(current_offsets.len() - 1)];
        let end = current_offsets[map(h.before.end, true).min(current_offsets.len() - 1)];
        result.replace_range(start..end, &after[after_offsets[h.after.start]..after_offsets[h.after.end]]);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepting_another_chunk_preserves_manual_result_lines() {
        assert_eq!(
            keep_manual_edits("one\nbase\ntwo\n", "one\nbase\ntheirs\n", "manual\nextra\nbase\ntwo\n"),
            "manual\nextra\nbase\ntheirs\n"
        );
    }
}
