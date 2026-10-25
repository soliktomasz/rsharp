//! Rider's Evaluate Expression (Alt+F8 over a paused program): the expression to evaluate, the frame it is evaluated
//! in, and its value or the debugger's reason. Enter evaluates; Escape closes it.

use crate::field::{Changed, TextField};
use crate::find_action::{Choose, Dismiss};
use crate::settings_dialog::button;
use crate::theme;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Subscription, Window, div,
    prelude::*, px,
};

pub enum Ended {
    Evaluate(String),
    Dismissed,
}

impl EventEmitter<Ended> for EvaluateDialog {}

pub struct EvaluateDialog {
    expression: Entity<TextField>,
    /// The frame's name, as the value is read in it.
    frame: String,
    /// The value or the debugger's reason, and whether it is a failure.
    result: Option<(String, bool)>,
    /// The value's members, a level deep, as the debugger names and shows them.
    members: Vec<(String, String, Option<String>)>,
    focus: FocusHandle,
    _expression: Subscription,
}

impl Focusable for EvaluateDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EvaluateDialog {
    pub fn new(initial: &str, frame: Option<String>, cx: &mut Context<Self>) -> Self {
        let expression = cx.new(|cx| {
            let mut field = TextField::new("An expression, as C# writes it", cx);
            field.set_text(initial, cx);
            field
        });
        let subscription = cx.subscribe(&expression, |_, _, _: &Changed, cx| cx.notify());
        Self {
            expression,
            frame: frame.unwrap_or_else(|| "no frame".into()),
            result: None,
            members: Vec::new(),
            focus: cx.focus_handle(),
            _expression: subscription,
        }
    }

    /// Where the expression is typed, which the window focuses as the dialog opens.
    pub fn expression_focus(&self, cx: &App) -> FocusHandle {
        self.expression.focus_handle(cx)
    }

    /// What the debugger answered, or why it could not: shown under the expression.
    pub fn set_result(&mut self, result: String, error: bool, cx: &mut Context<Self>) {
        self.result = Some((result, error));
        self.members.clear();
        cx.notify();
    }

    /// The value's members, when it has any: a name, its value and its type.
    pub fn set_members(&mut self, members: Vec<(String, String, Option<String>)>, cx: &mut Context<Self>) {
        self.members = members;
        cx.notify();
    }

    /// The members shown, for a test.
    pub fn members(&self) -> &[(String, String, Option<String>)] {
        &self.members
    }

    /// What the expression field holds, for a test.
    pub fn expression(&self, cx: &App) -> String {
        self.expression.read(cx).text()
    }

    /// The value or reason shown, for a test.
    pub fn result(&self) -> Option<&str> {
        self.result.as_ref().map(|(result, _)| result.as_str())
    }

    fn evaluate(&mut self, cx: &mut Context<Self>) {
        let expression = self.expression.read(cx).text().trim().to_string();
        if !expression.is_empty() {
            cx.emit(Ended::Evaluate(expression));
        }
    }
}

impl Render for EvaluateDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.expression.focus_handle(cx);
        let result = self.result.clone();
        div()
            .key_context("EvaluateDialog")
            .track_focus(&self.focus)
            .debug_selector(|| "evaluate dialog".into())
            .w(px(560.))
            .flex()
            .flex_col()
            .rounded(px(10.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .shadow_lg()
            .on_action(cx.listener(|dialog, _: &Choose, _, cx| dialog.evaluate(cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(Ended::Dismissed)))
            .child(
                div()
                    .h(px(44.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Evaluate Expression")
                    .child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::text_secondary())
                            .truncate()
                            .child(format!("in {}", self.frame)),
                    ),
            )
            .child(
                div()
                    .px(px(16.))
                    .pb(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("evaluate expression box")
                            .debug_selector(|| "evaluate field: Expression".into())
                            .track_focus(&focus)
                            .h(px(26.))
                            .px(px(6.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(theme::control_border())
                            .bg(theme::search_field())
                            .on_mouse_down(gpui::MouseButton::Left, {
                                let focus = focus.clone();
                                move |_, window, _| {
                                    window.focus(&focus);
                                    // The dialog itself takes focus on a click that bubbles to it; the field keeps it.
                                    window.prevent_default();
                                }
                            })
                            .on_click(move |_, window, _| {
                                window.focus(&focus);
                                window.prevent_default();
                            })
                            .child(self.expression.clone()),
                    )
                    .child(div().text_color(theme::text_secondary()).child("Result"))
                    .child(
                        div()
                            .debug_selector(|| "evaluate result".into())
                            .min_h(px(48.))
                            .px(px(6.))
                            .py(px(4.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(theme::control_border())
                            .text_color(match result {
                                Some((_, true)) => theme::warning(),
                                _ => theme::text(),
                            })
                            .child(match &result {
                                Some((result, _)) => result.clone(),
                                None => "Enter evaluates the expression".into(),
                            }),
                    )
                    .children((!self.members.is_empty()).then(|| {
                        let rows: Vec<gpui::AnyElement> =
                            self.members
                                .iter()
                                .enumerate()
                                .map(|(index, (name, value, kind))| {
                                    div()
                                        .id(("evaluate member", index))
                                        .debug_selector(move || format!("evaluate member {index}"))
                                        .h(px(22.))
                                        .px(px(6.))
                                        .rounded(px(4.))
                                        .flex()
                                        .items_center()
                                        .gap(px(4.))
                                        .hover(|d| d.bg(theme::row_hover()))
                                        .child(div().flex_none().child(name.clone()))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(theme::text_muted())
                                                .child(format!("= {value}")),
                                        )
                                        .children(kind.clone().map(|kind| {
                                            div().flex_none().text_color(theme::text_secondary()).child(kind)
                                        }))
                                        .into_any_element()
                                })
                                .collect();
                        div()
                            .id("evaluate members")
                            .max_h(px(160.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .child(div().text_color(theme::text_secondary()).child("Members"))
                            .children(rows)
                    }))
                    .child(
                        div()
                            .text_color(theme::text_secondary())
                            .child("Enter evaluates again after an edit; the Variables tree has the whole frame."),
                    ),
            )
            .child(
                div()
                    .h(px(56.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(16.))
                    .border_t_1()
                    .border_color(theme::frame())
                    .child(div().flex_1())
                    .child(
                        button("Close", false, true)
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(Ended::Dismissed))),
                    )
                    .child(
                        button("Evaluate", true, true)
                            .on_click(cx.listener(|dialog, _: &ClickEvent, _, cx| dialog.evaluate(cx))),
                    ),
            )
    }
}
