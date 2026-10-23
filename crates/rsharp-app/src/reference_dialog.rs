//! Rider's Add Reference dialog (Passenger's `AddReferenceDialog`): the solution's other projects as boxes to tick,
//! those already referenced ticked; a project that references this one, directly or through others, greyed with why,
//! as it would make a cycle. OK sets the references to those ticked.

use crate::assets::icon;
use crate::find_action::{Choose, Dismiss, SelectNext, SelectPrevious};
use crate::theme;
use gpui::{
    ClickEvent, Context, EventEmitter, FocusHandle, Focusable, SharedString, Window, actions, div, prelude::*, px,
};
use std::path::PathBuf;

actions!(reference_dialog, [ToggleChoice]);

pub struct Choice {
    pub name: String,
    pub path: PathBuf,
    pub chosen: bool,
    /// Why it cannot be ticked, when it cannot.
    pub refused: Option<String>,
}

pub enum Ended {
    Chosen(Vec<PathBuf>),
    Dismissed,
}

pub struct ReferenceDialog {
    project: SharedString,
    choices: Vec<Choice>,
    selected: usize,
    focus: FocusHandle,
}

impl EventEmitter<Ended> for ReferenceDialog {}

impl Focusable for ReferenceDialog {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ReferenceDialog {
    pub fn new(project: String, choices: Vec<Choice>, cx: &mut Context<Self>) -> Self {
        Self { project: project.into(), choices, selected: 0, focus: cx.focus_handle() }
    }

    /// Each choice as (name, whether it can be ticked, whether it is).
    pub fn choices(&self) -> Vec<(String, bool, bool)> {
        self.choices.iter().map(|c| (c.name.clone(), c.refused.is_none(), c.chosen)).collect()
    }

    pub fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(choice) = self.choices.get_mut(index).filter(|c| c.refused.is_none()) {
            choice.chosen = !choice.chosen;
            cx.notify();
        }
    }

    fn ok(&mut self, cx: &mut Context<Self>) {
        cx.emit(Ended::Chosen(self.choices.iter().filter(|c| c.chosen).map(|c| c.path.clone()).collect()));
    }
}

impl Render for ReferenceDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<_> = self
            .choices
            .iter()
            .enumerate()
            .map(|(i, choice)| {
                let enabled = choice.refused.is_none();
                div()
                    .id(("reference", i))
                    .h(px(26.))
                    .mx(px(6.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(i == self.selected, |d| d.bg(theme::selection()))
                    .when(!enabled, |d| d.text_color(theme::text_secondary()))
                    .child(
                        div()
                            .size(px(14.))
                            .rounded(px(3.))
                            .border_1()
                            .border_color(theme::popup_border())
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(choice.chosen, |d| d.child(icon("check", 12.))),
                    )
                    .child(icon("project", 16.))
                    .child(div().flex_1().truncate().child(choice.name.clone()))
                    .children(choice.refused.clone().map(|why| div().text_color(theme::text_secondary()).child(why)))
                    .on_click(cx.listener(move |d, _: &ClickEvent, _, cx| {
                        d.selected = i;
                        d.toggle(i, cx);
                    }))
            })
            .collect();
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px(px(14.))
                .h(px(26.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .border_1()
                .border_color(theme::popup_border())
                .hover(|d| d.bg(theme::row_hover()))
                .child(label)
        };
        div()
            .key_context("ReferenceDialog")
            .track_focus(&self.focus)
            .w(px(420.))
            .flex()
            .flex_col()
            .pb(px(10.))
            .rounded(px(10.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .shadow_lg()
            .on_action(cx.listener(|d, _: &SelectPrevious, _, cx| {
                d.selected = d.selected.saturating_sub(1);
                cx.notify()
            }))
            .on_action(cx.listener(|d, _: &SelectNext, _, cx| {
                d.selected = (d.selected + 1).min(d.choices.len().saturating_sub(1));
                cx.notify()
            }))
            .on_action(cx.listener(|d, _: &ToggleChoice, _, cx| {
                let at = d.selected;
                d.toggle(at, cx)
            }))
            .on_action(cx.listener(|d, _: &Choose, _, cx| d.ok(cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(Ended::Dismissed)))
            .child(
                div().h(px(36.)).px(px(14.)).flex().items_center().child(format!("Add Reference to {}", self.project)),
            )
            .when(rows.is_empty(), |d| {
                d.child(div().px(px(14.)).text_color(theme::text_secondary()).child("No other project in the solution"))
            })
            .children(rows)
            .child(
                div()
                    .pt(px(10.))
                    .px(px(14.))
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        button("reference-cancel", "Cancel")
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(Ended::Dismissed))),
                    )
                    .child(button("reference-ok", "OK").on_click(cx.listener(|d, _: &ClickEvent, _, cx| d.ok(cx)))),
            )
    }
}
