//! Rider's breakpoint popup over the editor (Alt+F9, or a right-click on a breakpoint): the line's condition, hit count and
//! log message, Tab going from one to the next. Every change applies at once, as the Breakpoints dialog's does; Enter
//! and Escape close it.

use crate::field::{Changed, TextField};
use crate::find_action::{Choose, Dismiss, SelectNext, SelectPrevious};
use crate::theme;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Subscription, Window, div, prelude::*, px,
};

pub enum Ended {
    /// The fields as typed: condition, hit count, log message.
    Changed {
        condition: String,
        hit_count: String,
        log_message: String,
    },
    Dismissed,
}

impl EventEmitter<Ended> for BreakpointPopup {}

/// The popup's fields, in Tab's order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Condition,
    HitCount,
    LogMessage,
}

pub struct BreakpointPopup {
    /// "Program.cs:4", the line it edits.
    place: String,
    fields: [Entity<TextField>; 3],
    focus: FocusHandle,
    _fields: Vec<Subscription>,
}

impl Focusable for BreakpointPopup {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BreakpointPopup {
    pub fn new(place: String, condition: &str, hit_count: &str, log_message: &str, cx: &mut Context<Self>) -> Self {
        let field = |text: &str, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut field = TextField::new("", cx);
                field.set_text(text, cx);
                field
            })
        };
        let fields = [field(condition, cx), field(hit_count, cx), field(log_message, cx)];
        let subscriptions = fields
            .iter()
            .map(|field| {
                cx.subscribe(field, |popup, _, _: &Changed, cx| {
                    let [condition, hit_count, log_message] = popup.texts(cx);
                    cx.emit(Ended::Changed { condition, hit_count, log_message });
                })
            })
            .collect();
        Self { place, fields, focus: cx.focus_handle(), _fields: subscriptions }
    }

    fn texts(&self, cx: &App) -> [String; 3] {
        [0, 1, 2].map(|index| self.fields[index].read(cx).text())
    }

    /// A field focused, its text chosen so what is typed replaces it.
    pub fn focus_field(&self, field: Field, window: &mut Window, cx: &mut App) {
        let entity = &self.fields[field as usize];
        entity.update(cx, |field, cx| field.select_all(cx));
        window.focus(&entity.focus_handle(cx));
    }

    /// The field that has the focus, for a test.
    pub fn focused(&self, window: &Window, cx: &App) -> Option<Field> {
        [Field::Condition, Field::HitCount, Field::LogMessage]
            .into_iter()
            .find(|field| self.fields[*field as usize].focus_handle(cx).is_focused(window))
    }

    fn step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let at = self.focused(window, cx).map_or(0, |field| field as usize);
        let next = if forward { (at + 1) % 3 } else { (at + 2) % 3 };
        let field = [Field::Condition, Field::HitCount, Field::LogMessage][next];
        self.focus_field(field, window, cx);
        cx.notify();
    }
}

impl Render for BreakpointPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = |label: &'static str, field: &Entity<TextField>, cx: &App| {
            let focus = field.focus_handle(cx);
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().w(px(90.)).flex_none().text_color(theme::text_secondary()).child(label))
                .child(
                    div()
                        .id(gpui::ElementId::Name(format!("breakpoint popup box {label}").into()))
                        .debug_selector(move || format!("breakpoint popup field: {label}"))
                        .track_focus(&focus)
                        .on_mouse_down(gpui::MouseButton::Left, move |_, window, _| {
                            window.focus(&focus);
                            window.prevent_default();
                        })
                        .w(px(300.))
                        .h(px(26.))
                        .px(px(6.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(theme::control_border())
                        .bg(theme::search_field())
                        .child(field.clone()),
                )
        };
        div()
            .key_context("BreakpointPopup")
            .track_focus(&self.focus)
            .debug_selector(|| "breakpoint popup".into())
            .on_action(cx.listener(|_, _: &Choose, _, cx| cx.emit(Ended::Dismissed)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(Ended::Dismissed)))
            .on_action(cx.listener(|popup, _: &SelectNext, window, cx| popup.step(true, window, cx)))
            .on_action(cx.listener(|popup, _: &SelectPrevious, window, cx| popup.step(false, window, cx)))
            .w(px(440.))
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .rounded(px(8.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .shadow_lg()
            .child(div().font_weight(FontWeight::SEMIBOLD).child(self.place.clone()))
            .child(row("Condition", &self.fields[0], cx))
            .child(row("Hit count", &self.fields[1], cx))
            .child(row("Log message", &self.fields[2], cx))
            .child(
                div()
                    .text_color(theme::text_secondary())
                    .child("A log message makes it a tracepoint: it logs and goes on"),
            )
    }
}
