//! Find Action: every action by name, as Rider's Ctrl+Shift+A / ⇧⌘A popup, with its keys and its menu, and
//! the ones that cannot run where the person was greyed with why.

use crate::actions::ALL;
use crate::assets::icon;
use crate::field::{Changed, TextField};
use crate::fuzzy;
use crate::theme;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Window, actions, div, prelude::*, px,
};

actions!(popup, [SelectPrevious, SelectNext, Choose, Dismiss]);

const ROW_HEIGHT: f32 = 24.;
const SHOWN: usize = 14;

/// What the popup ended with: an action chosen (its index in `ALL`), or nothing.
pub enum Ended {
    Chosen(usize),
    Dismissed,
}

pub struct FindActionPopup {
    field: Entity<TextField>,
    /// Whether each action of `ALL` can run where the person was.
    available: Vec<bool>,
    /// Each action's keys, as the platform writes them.
    keys: Vec<Option<SharedString>>,
    matches: Vec<usize>,
    selected: usize,
}

impl EventEmitter<Ended> for FindActionPopup {}

impl FindActionPopup {
    pub fn new(available: Vec<bool>, keys: Vec<Option<SharedString>>, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| TextField::new("Type an action name", cx));
        cx.subscribe(&field, |popup, _, _: &Changed, cx| popup.refilter(cx)).detach();
        let mut popup = Self { field, available, keys, matches: Vec::new(), selected: 0 };
        popup.refilter(cx);
        popup
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.field.read(cx).text();
        let mut scored: Vec<(i32, usize)> = ALL
            .iter()
            .enumerate()
            .filter_map(|(i, a)| {
                let best = std::iter::once(a.title)
                    .chain(a.words.iter().copied())
                    .filter_map(|name| fuzzy::score(&query, name))
                    .max()?;
                // Of equal matches, what can run comes first.
                Some((best * 2 + i32::from(self.available[i]), i))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.matches = scored.into_iter().map(|(_, i)| i).collect();
        self.selected = 0;
        cx.notify();
    }

    /// The actions the popup lists, in order, as a person reads them.
    pub fn listed(&self) -> Vec<&'static str> {
        self.matches.iter().map(|i| ALL[*i].title).collect()
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(action) = self.matches.get(index) {
            cx.emit(Ended::Chosen(*action));
        }
    }
}

impl Focusable for FindActionPopup {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.field.focus_handle(cx)
    }
}

impl Render for FindActionPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<_> = self
            .matches
            .iter()
            .take(SHOWN)
            .enumerate()
            .map(|(row, &i)| {
                let action = &ALL[i];
                let available = self.available[i];
                div()
                    .id(("action", row))
                    .h(px(ROW_HEIGHT))
                    .mx(px(6.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(row == self.selected, |d| d.bg(theme::selection()))
                    .when(row != self.selected, |d| d.hover(|d| d.bg(theme::row_hover())))
                    .when(!available, |d| d.text_color(theme::text_secondary()))
                    .child(div().flex_1().truncate().child(action.title))
                    .when(!available, |d| {
                        d.child(div().text_color(theme::text_secondary()).child("not available here"))
                    })
                    .child(div().text_color(theme::text_secondary()).child(action.menu))
                    .children(
                        self.keys[i]
                            .clone()
                            .map(|k| div().min_w(px(64.)).text_right().text_color(theme::text_muted()).child(k)),
                    )
                    .on_click(cx.listener(move |p, _, _, cx| p.choose(row, cx)))
            })
            .collect();
        let nothing = self.matches.is_empty();
        div()
            .key_context("FindAction")
            .w(px(620.))
            .flex()
            .flex_col()
            .pb(px(6.))
            .rounded(px(10.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .shadow_lg()
            .on_action(cx.listener(|p, _: &SelectPrevious, _, cx| {
                p.selected = p.selected.saturating_sub(1);
                cx.notify()
            }))
            .on_action(cx.listener(|p, _: &SelectNext, _, cx| {
                p.selected = (p.selected + 1).min(p.matches.len().min(SHOWN).saturating_sub(1));
                cx.notify()
            }))
            .on_action(cx.listener(|p, _: &Choose, _, cx| {
                let s = p.selected;
                p.choose(s, cx)
            }))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(Ended::Dismissed)))
            .child(
                div()
                    .h(px(40.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(theme::popup_border())
                    .mb(px(6.))
                    .child(icon("search", 16.))
                    .child(self.field.clone()),
            )
            .children(rows)
            .when(nothing, |d| {
                d.child(
                    div()
                        .h(px(ROW_HEIGHT * 2.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme::text_secondary())
                        .child("No actions match"),
                )
            })
    }
}
