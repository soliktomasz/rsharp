//! Rider's Attach to Process: the .NET programs running on this machine the debugger can attach to, each by Rider's
//! columns (executable, process id, user, command line), those attached to before under "Recently attached" first. The
//! search box narrows them as it is typed, the arrow keys choose, and Enter or Attach with .NET Debugger closes it with
//! the program chosen. Refresh asks the window to list them again.

use crate::field::{Changed, TextField};
use crate::find_action::{Choose, Dismiss, SelectNext, SelectPrevious};
use crate::settings_dialog::button;
use crate::theme;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Subscription, Window, div,
    prelude::*, px,
};
use rsharp_core::debug::DotnetProcess;

pub enum Ended {
    Chosen(DotnetProcess),
    /// List them again: a program started since the dialog opened.
    Refresh,
    Dismissed,
}

impl EventEmitter<Ended> for AttachDialog {}

pub struct AttachDialog {
    processes: Vec<DotnetProcess>,
    recent: Option<DotnetProcess>,
    chosen: usize,
    search: Entity<TextField>,
    focus: FocusHandle,
    _search: Subscription,
}

impl Focusable for AttachDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl AttachDialog {
    pub fn new(processes: Vec<DotnetProcess>, recent: Option<DotnetProcess>, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextField::new("Search by name, process ID, user or command line", cx));
        let subscription = cx.subscribe(&search, |dialog, _, _: &Changed, cx| {
            dialog.chosen = 0;
            cx.notify();
        });
        Self { processes, recent, chosen: 0, search, focus: cx.focus_handle(), _search: subscription }
    }

    /// The programs shown now, the recently attached ones first and the search applied, in their order.
    pub fn shown(&self, cx: &App) -> Vec<DotnetProcess> {
        let search = self.search.read(cx).text();
        let mut shown: Vec<DotnetProcess> = match &self.recent {
            Some(recent) => self
                .processes
                .iter()
                .filter(|process| process.program() == recent.program() && process.matches(&search))
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        let others: Vec<DotnetProcess> = self
            .processes
            .iter()
            .filter(|process| process.matches(&search) && !shown.iter().any(|shown| shown.id == process.id))
            .cloned()
            .collect();
        shown.extend(others);
        shown
    }

    pub fn chosen(&self, cx: &App) -> Option<DotnetProcess> {
        self.shown(cx).into_iter().nth(self.chosen)
    }

    /// The chosen row moves by one, within the rows shown.
    fn choose_by(&mut self, step: isize, cx: &mut Context<Self>) {
        let count = self.shown(cx).len();
        if count == 0 {
            return;
        }
        self.chosen = (self.chosen as isize + step).clamp(0, count as isize - 1) as usize;
        cx.notify();
    }

    fn attach(&mut self, cx: &mut Context<Self>) {
        if let Some(process) = self.chosen(cx) {
            cx.emit(Ended::Chosen(process));
        }
    }

    fn heading(title: &'static str) -> impl IntoElement {
        div().h(px(24.)).px(px(8.)).flex().items_center().text_color(theme::text_secondary()).child(title)
    }

    fn row(&self, index: usize, process: &DotnetProcess, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let chosen = index == self.chosen;
        div()
            .id(gpui::ElementId::Name(format!("attach row {} {}", process.program(), process.id).into()))
            .debug_selector({
                let name = process.program();
                move || format!("attach row: {name}")
            })
            .h(px(24.))
            .px(px(6.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .gap(px(8.))
            .when(chosen, |d| d.bg(theme::selection()))
            .when(!chosen, |d| d.hover(|d| d.bg(theme::row_hover())))
            .child(div().w(px(190.)).flex_none().truncate().child(process.program()))
            .child(div().w(px(70.)).flex_none().text_color(theme::text_secondary()).child(process.id.to_string()))
            .child(
                div()
                    .w(px(100.))
                    .flex_none()
                    .truncate()
                    .text_color(theme::text_secondary())
                    .child(process.user.clone()),
            )
            .child(
                div().flex_1().min_w_0().truncate().text_color(theme::text_muted()).child(process.command_line.clone()),
            )
            .on_click(cx.listener(move |dialog, _: &ClickEvent, _, cx| {
                dialog.chosen = index;
                cx.notify();
            }))
    }
}

impl Render for AttachDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.shown(cx);
        if self.chosen >= shown.len() {
            self.chosen = shown.len().saturating_sub(1);
        }
        let recent_count = self
            .recent
            .as_ref()
            .map(|recent| shown.iter().filter(|process| process.program() == recent.program()).count())
            .unwrap_or(0);
        let rows: Vec<gpui::AnyElement> = shown
            .iter()
            .enumerate()
            .flat_map(|(index, process)| {
                let row = self.row(index, process, cx).into_any_element();
                let heading = if index == 0 && recent_count > 0 {
                    Some(Self::heading("Recently attached").into_any_element())
                } else if index == recent_count && recent_count > 0 && recent_count < shown.len() {
                    Some(Self::heading("Running processes").into_any_element())
                } else {
                    None
                };
                heading.into_iter().chain(std::iter::once(row))
            })
            .collect();
        let has = !shown.is_empty();
        let count = shown.len();
        let search_focus = self.search.focus_handle(cx);
        div()
            .key_context("AttachDialog")
            .track_focus(&self.focus)
            .debug_selector(|| "attach dialog".into())
            .w(px(820.))
            .h(px(520.))
            .flex()
            .flex_col()
            .rounded(px(10.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .shadow_lg()
            .on_action(cx.listener(|dialog, _: &Choose, _, cx| dialog.attach(cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(Ended::Dismissed)))
            .on_action(cx.listener(|dialog, _: &SelectNext, _, cx| dialog.choose_by(1, cx)))
            .on_action(cx.listener(|dialog, _: &SelectPrevious, _, cx| dialog.choose_by(-1, cx)))
            .child(
                div()
                    .h(px(44.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Attach to Process")
                    .child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::text_secondary())
                            .child(format!("{count} .NET program(s)")),
                    ),
            )
            .child(
                div()
                    .h(px(40.))
                    .flex_none()
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("attach search box")
                            .debug_selector(|| "attach field: Search processes".into())
                            .flex_1()
                            .h(px(26.))
                            .px(px(6.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(theme::control_border())
                            .bg(theme::search_field())
                            .on_mouse_down(gpui::MouseButton::Left, move |_, window, _| {
                                window.focus(&search_focus);
                                window.prevent_default();
                            })
                            .child(self.search.clone()),
                    )
                    .child(
                        button("Refresh", false, true)
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(Ended::Refresh))),
                    ),
            )
            .child(
                div()
                    .h(px(24.))
                    .flex_none()
                    .px(px(22.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_color(theme::text_secondary())
                    .child(div().w(px(190.)).flex_none().child("Executable"))
                    .child(div().w(px(70.)).flex_none().child("Process ID"))
                    .child(div().w(px(100.)).flex_none().child("User"))
                    .child(div().flex_1().min_w_0().child("Command Line")),
            )
            .child(div().id("attach list").flex_1().min_h_0().overflow_y_scroll().px(px(16.)).children(rows).children(
                (count == 0).then(|| {
                    div()
                        .px(px(8.))
                        .py(px(6.))
                        .text_color(theme::text_secondary())
                        .child("No .NET program to attach to: start one, then Refresh")
                }),
            ))
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
                        button("Cancel", false, true)
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(Ended::Dismissed))),
                    )
                    .child(
                        button("Attach with .NET Debugger", true, has)
                            .when(has, |d| d.on_click(cx.listener(|dialog, _: &ClickEvent, _, cx| dialog.attach(cx)))),
                    ),
            )
    }
}
