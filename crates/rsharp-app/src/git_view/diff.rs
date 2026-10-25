use super::action;
use crate::theme;
use gpui::{
    App, Context, FocusHandle, Focusable, HighlightStyle, SharedString, StyledText, Task, UniformListScrollHandle,
    Window, div, prelude::*, px, uniform_list,
};
use rsharp_text::{diff_hunks, word_diff};
use std::{ops::Range, sync::Arc};

#[derive(Clone)]
struct Side {
    number: usize,
    text: SharedString,
    changed: bool,
    words: Vec<Range<usize>>,
}
#[derive(Clone)]
struct Row {
    left: Option<Side>,
    right: Option<Side>,
}

/// Aligned rows in a single virtual list keep both revisions on the same vertical scroll position.
pub struct DiffView {
    focus: FocusHandle,
    left_title: String,
    right_title: String,
    rows: Arc<Vec<Row>>,
    differences: Vec<usize>,
    selected: usize,
    scroll: UniformListScrollHandle,
    loading: bool,
    horizontal: gpui::Pixels,
    _work: Task<()>,
}

impl Focusable for DiffView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl DiffView {
    pub fn new(left_title: String, left: String, right_title: String, right: String, cx: &mut Context<Self>) -> Self {
        let work = cx.background_executor().spawn(async move { aligned(&left, &right) });
        let task = cx.spawn(async move |this, cx| {
            let (rows, differences) = work.await;
            let _ = this.update(cx, |v, cx| {
                v.rows = Arc::new(rows);
                v.differences = differences;
                v.loading = false;
                cx.notify();
            });
        });
        Self {
            focus: cx.focus_handle(),
            left_title,
            right_title,
            rows: Arc::new(Vec::new()),
            differences: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            loading: true,
            horizontal: px(0.),
            _work: task,
        }
    }
    pub fn difference_count(&self) -> usize {
        self.differences.len()
    }
    pub fn next(&mut self, back: bool, cx: &mut Context<Self>) {
        if self.differences.is_empty() {
            return;
        }
        self.selected = if back {
            self.selected.checked_sub(1).unwrap_or(self.differences.len() - 1)
        } else {
            (self.selected + 1) % self.differences.len()
        };
        self.scroll.scroll_to_item(self.differences[self.selected], gpui::ScrollStrategy::Center);
        cx.notify();
    }
}

fn aligned(left: &str, right: &str) -> (Vec<Row>, Vec<usize>) {
    let before: Vec<_> = left.split_inclusive('\n').collect();
    let after: Vec<_> = right.split_inclusive('\n').collect();
    let hunks = diff_hunks(left, right);
    let mut rows = Vec::new();
    let mut differences = Vec::new();
    let (mut a, mut b) = (0, 0);
    let side = |lines: &Vec<&str>, n: usize, changed: bool| Side {
        number: n + 1,
        text: lines[n].trim_end_matches('\n').to_string().into(),
        changed,
        words: Vec::new(),
    };
    for h in hunks {
        while a < h.before.start && b < h.after.start {
            rows.push(Row { left: Some(side(&before, a, false)), right: Some(side(&after, b, false)) });
            a += 1;
            b += 1;
        }
        differences.push(rows.len());
        for i in 0..h.before.len().max(h.after.len()) {
            let mut l = (i < h.before.len()).then(|| side(&before, h.before.start + i, true));
            let mut r = (i < h.after.len()).then(|| side(&after, h.after.start + i, true));
            if let (Some(l), Some(r)) = (&mut l, &mut r) {
                for word in word_diff(&l.text, &r.text) {
                    l.words.push(word.before);
                    r.words.push(word.after);
                }
            }
            rows.push(Row { left: l, right: r });
        }
        a = h.before.end;
        b = h.after.end;
    }
    while a < before.len() || b < after.len() {
        rows.push(Row {
            left: (a < before.len()).then(|| side(&before, a, false)),
            right: (b < after.len()).then(|| side(&after, b, false)),
        });
        a += 1;
        b += 1;
    }
    // A missing newline remains a visible difference even though both displayed lines read alike.
    if left.ends_with('\n') != right.ends_with('\n') {
        rows.push(Row {
            left: (!left.ends_with('\n')).then(|| Side {
                number: 0,
                text: "No newline at end of file".into(),
                changed: true,
                words: Vec::new(),
            }),
            right: (!right.ends_with('\n')).then(|| Side {
                number: 0,
                text: "No newline at end of file".into(),
                changed: true,
                words: Vec::new(),
            }),
        });
    }
    (rows, differences)
}

fn cell(side: &Option<Side>, left: bool, horizontal: gpui::Pixels) -> impl IntoElement {
    let background = if left { theme::editor::diff_deleted() } else { theme::editor::diff_inserted() };
    div()
        .flex_1()
        .min_w_0()
        .h(px(22.))
        .flex()
        .items_center()
        .gap(px(8.))
        .overflow_hidden()
        .when(side.as_ref().is_some_and(|s| s.changed), |d| d.bg(background))
        .when_some(side.as_ref(), |d, s| {
            d.child(
                div()
                    .w(px(48.))
                    .flex_none()
                    .text_right()
                    .pr(px(5.))
                    .text_color(theme::text_muted())
                    .child(if s.number == 0 { String::new() } else { s.number.to_string() }),
            )
            .child(div().flex_1().min_w_0().overflow_hidden().child(
                div().relative().left(-horizontal).child(StyledText::new(s.text.clone()).with_highlights(
                    s.words.iter().filter(|r| !r.is_empty()).cloned().map(|range| {
                        (
                            range,
                            HighlightStyle {
                                background_color: Some(theme::selection().into()),
                                font_weight: Some(gpui::FontWeight::BOLD),
                                ..Default::default()
                            },
                        )
                    }),
                )),
            ))
        })
}

impl Render for DiffView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows.clone();
        div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::island())
            .text_color(theme::text())
            .font_family(theme::UI_FONT)
            .text_size(px(13.))
            .child(
                div()
                    .h(px(38.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .child(
                        action("diff previous", "Previous Difference", !self.differences.is_empty())
                            .on_click(cx.listener(|v, _, _, cx| v.next(true, cx))),
                    )
                    .child(
                        action("diff next", "Next Difference", !self.differences.is_empty())
                            .on_click(cx.listener(|v, _, _, cx| v.next(false, cx))),
                    )
                    .child(div().text_color(theme::text_muted()).child(if self.loading {
                        "Comparing…".into()
                    } else {
                        format!("{} differences", self.differences.len())
                    })),
            )
            .child(
                div()
                    .h(px(28.))
                    .flex_none()
                    .flex()
                    .border_b_1()
                    .border_color(theme::separator())
                    .child(div().flex_1().min_w_0().px(px(10.)).truncate().child(self.left_title.clone()))
                    .child(div().flex_1().min_w_0().px(px(10.)).truncate().child(self.right_title.clone())),
            )
            .child(
                uniform_list(
                    "git diff rows",
                    rows.len(),
                    cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|i| {
                                let row = &rows[i];
                                div()
                                    .h(px(22.))
                                    .flex()
                                    .font_family(theme::CODE_FONT)
                                    .whitespace_nowrap()
                                    .on_scroll_wheel(cx.listener(|v, event: &gpui::ScrollWheelEvent, _, cx| {
                                        let delta = event.delta.pixel_delta(px(22.));
                                        if delta.x != px(0.) {
                                            v.horizontal = (v.horizontal - delta.x).max(px(0.));
                                            cx.notify();
                                            cx.stop_propagation();
                                        }
                                    }))
                                    .child(cell(&row.left, true, view.horizontal))
                                    .child(div().w(px(1.)).h_full().bg(theme::separator()))
                                    .child(cell(&row.right, false, view.horizontal))
                            })
                            .collect()
                    }),
                )
                .track_scroll(self.scroll.clone())
                .flex_1()
                .min_h_0(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn added_lines_leave_a_gap_on_the_old_side() {
        let (rows, differences) = aligned("a\nc\n", "a\nb\nc\n");
        assert_eq!(differences, [1]);
        assert!(rows[1].left.is_none());
        assert_eq!(rows[1].right.as_ref().unwrap().text.as_ref(), "b");
        assert_eq!(rows[2].left.as_ref().unwrap().number, 2);
    }
}
