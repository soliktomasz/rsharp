//! Rider's Smart Step Into: the calls a paused statement can step into, in the order they run, one of them chosen with
//! the arrow keys and Enter or a click. Escape closes it and steps nowhere.

use crate::find_action::{Choose, Dismiss, SelectNext, SelectPrevious};
use crate::theme;
use gpui::{App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, FontWeight, Window, div, prelude::*, px};
use rsharp_dap::protocol::StepInTarget;

pub enum Ended {
    /// The target chosen, by its place in the list.
    Chosen(usize),
    Dismissed,
}

impl EventEmitter<Ended> for StepIntoDialog {}

pub struct StepIntoDialog {
    targets: Vec<StepInTarget>,
    chosen: usize,
    focus: FocusHandle,
}

impl Focusable for StepIntoDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl StepIntoDialog {
    pub fn new(targets: &[StepInTarget], cx: &mut Context<Self>) -> Self {
        Self { targets: targets.to_vec(), chosen: 0, focus: cx.focus_handle() }
    }

    /// The target chosen, by its place in the list, for a test.
    pub fn chosen(&self) -> usize {
        self.chosen
    }

    fn choose_by(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.targets.is_empty() {
            return;
        }
        self.chosen = (self.chosen as isize + step).clamp(0, self.targets.len() as isize - 1) as usize;
        cx.notify();
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        self.chosen = index;
        cx.emit(Ended::Chosen(index));
    }
}

impl Render for StepIntoDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<gpui::AnyElement> = self
            .targets
            .iter()
            .enumerate()
            .map(|(index, target)| {
                let chosen = index == self.chosen;
                let label = if target.label.is_empty() { format!("Call {index}") } else { target.label.clone() };
                let place = format!("{}:{}", target.line + 1, target.column + 1);
                div()
                    .id(("smart step into target", index))
                    .debug_selector(move || format!("smart step into target {index}"))
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(chosen, |d| d.bg(theme::selection()))
                    .when(!chosen, |d| d.hover(|d| d.bg(theme::row_hover())))
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .child(div().flex_none().text_color(theme::text_secondary()).child(place))
                    .on_click(cx.listener(move |dialog, _: &ClickEvent, _, cx| dialog.choose(index, cx)))
                    .into_any_element()
            })
            .collect();
        div()
            .key_context("SmartStepInto")
            .track_focus(&self.focus)
            .debug_selector(|| "smart step into dialog".into())
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
                    .gap(px(8.))
                    .px(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Smart Step Into")
                    .child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::text_secondary())
                            .child("The call to step into"),
                    ),
            )
            .child(div().px(px(8.)).pb(px(8.)).flex().flex_col().children(rows))
    }
}
