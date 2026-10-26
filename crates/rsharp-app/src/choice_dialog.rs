//! A question with a few answers, as Rider asks Hot Reload's restart: the arrow keys or a click choose, Enter takes the
//! chosen one, and Escape answers nothing.

use crate::find_action::{Choose, Dismiss, SelectNext, SelectPrevious};
use crate::theme;
use gpui::{App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, FontWeight, Window, div, prelude::*, px};

pub enum Ended {
    /// The answer chosen, as its label.
    Chosen(&'static str),
    Dismissed,
}

impl EventEmitter<Ended> for ChoiceDialog {}

pub struct ChoiceDialog {
    title: String,
    message: String,
    answers: &'static [&'static str],
    chosen: usize,
    focus: FocusHandle,
}

impl Focusable for ChoiceDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ChoiceDialog {
    /// A question, `answers` in the order they are shown and `chosen` the one Enter takes.
    pub fn new(
        title: &str,
        message: &str,
        answers: &'static [&'static str],
        chosen: usize,
        cx: &mut Context<Self>,
    ) -> Self {
        let chosen = chosen.min(answers.len().saturating_sub(1));
        Self { title: title.into(), message: message.into(), answers, chosen, focus: cx.focus_handle() }
    }

    /// The answer Enter takes now, for a test.
    pub fn chosen(&self) -> Option<&'static str> {
        self.answers.get(self.chosen).copied()
    }

    fn choose_by(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.answers.is_empty() {
            return;
        }
        self.chosen = (self.chosen as isize + step).clamp(0, self.answers.len() as isize - 1) as usize;
        cx.notify();
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(answer) = self.answers.get(index) {
            self.chosen = index;
            cx.emit(Ended::Chosen(answer));
        }
    }
}

impl Render for ChoiceDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let buttons: Vec<gpui::AnyElement> = self
            .answers
            .iter()
            .enumerate()
            .map(|(index, answer)| {
                let chosen = index == self.chosen;
                div()
                    .id(("choice", index))
                    .debug_selector(move || format!("choice: {answer}"))
                    .h(px(28.))
                    .px(px(14.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .when(chosen, |d| d.bg(theme::selection()))
                    .when(!chosen, |d| d.bg(theme::search_field()).hover(|d| d.bg(theme::row_hover())))
                    .child(*answer)
                    .on_click(cx.listener(move |dialog, _: &ClickEvent, _, cx| dialog.choose(index, cx)))
                    .into_any_element()
            })
            .collect();
        div()
            .key_context("ChoiceDialog")
            .track_focus(&self.focus)
            .debug_selector(|| "choice dialog".into())
            .w(px(460.))
            .flex()
            .flex_col()
            .rounded(px(10.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .shadow_lg()
            .on_action(cx.listener(|dialog, _: &Choose, _, cx| dialog.choose(dialog.chosen, cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(Ended::Dismissed)))
            .on_action(cx.listener(|dialog, _: &SelectNext, _, cx| dialog.choose_by(1, cx)))
            .on_action(cx.listener(|dialog, _: &SelectPrevious, _, cx| dialog.choose_by(-1, cx)))
            .child(
                div()
                    .h(px(40.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .px(px(14.))
                    .pb(px(14.))
                    .text_color(theme::text_secondary())
                    .whitespace_normal()
                    .child(self.message.clone()),
            )
            .child(div().px(px(12.)).pb(px(12.)).flex().justify_end().gap(px(8.)).children(buttons))
    }
}
