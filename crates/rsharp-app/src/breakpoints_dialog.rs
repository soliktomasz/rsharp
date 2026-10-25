//! Rider's Breakpoints dialog: the folder's line breakpoints and exception breakpoints down the left, the chosen one's
//! condition, hit count, log message and "Suspend if" on the right, Mute Breakpoints over them and Add/Remove below.
//! Every change applies at once, as Rider's does: Done only closes it.

use crate::field::{Changed, TextField};
use crate::find_action::{Choose, Dismiss};
use crate::settings_dialog::{button, checkbox};
use crate::theme;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, SharedString, Subscription,
    Window, div, prelude::*, px,
};
use rsharp_core::debug::{Breakpoints, SCENARIOS};
use std::path::{Path, PathBuf};

pub enum Ended {
    /// The breakpoints as the dialog now has them: kept at once, as Rider applies a change here.
    Changed(Breakpoints),
    Dismissed,
}

impl EventEmitter<Ended> for BreakpointsDialog {}

/// What the form edits: a line's breakpoint, one of the function breakpoints, "Any exception", or one of the typed
/// exception breakpoints.
#[derive(Clone, PartialEq)]
enum Chosen {
    Line { path: PathBuf, line: usize },
    Function(usize),
    AnyException,
    Exception(usize),
}

/// Rider's five "Suspend if" cases, as its own words name them.
const SCENARIO_LABELS: [(&str, &str); 5] = [
    ("thrownInUserCode", "Thrown in user code"),
    ("thrownInExternalCode", "Thrown in external code"),
    ("handledInUserCode", "Handled in user code"),
    ("handledInExternalCode", "Handled in external code"),
    ("unhandled", "Unhandled"),
];

pub struct BreakpointsDialog {
    breakpoints: Breakpoints,
    chosen: Chosen,
    /// What the debugger made of each line's breakpoint when the dialog opened: whether it will be hit, and why not.
    verification: Vec<(PathBuf, usize, bool, Option<String>)>,
    /// The function breakpoint's method name; the other kinds leave it empty.
    name: Entity<TextField>,
    condition: Entity<TextField>,
    hit_count: Entity<TextField>,
    log_message: Entity<TextField>,
    types: Entity<TextField>,
    /// Set while the fields show the chosen row, so filling them is no edit of it.
    showing: bool,
    focus: FocusHandle,
    _fields: Vec<Subscription>,
}

impl Focusable for BreakpointsDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn field(text: &str, cx: &mut App) -> Entity<TextField> {
    cx.new(|cx| {
        let mut field = TextField::new("", cx);
        field.set_text(text, cx);
        field
    })
}

impl BreakpointsDialog {
    pub fn new(
        breakpoints: Breakpoints,
        verification: Vec<(PathBuf, usize, bool, Option<String>)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let chosen = breakpoints
            .files()
            .into_iter()
            .find_map(|path| {
                breakpoints.of(&path).first().map(|breakpoint| Chosen::Line { path, line: breakpoint.line })
            })
            .unwrap_or(Chosen::AnyException);
        let mut dialog = Self {
            breakpoints,
            chosen,
            verification,
            name: field("", cx),
            condition: field("", cx),
            hit_count: field("", cx),
            log_message: field("", cx),
            types: field("", cx),
            showing: false,
            focus: cx.focus_handle(),
            _fields: Vec::new(),
        };
        for input in [&dialog.name, &dialog.condition, &dialog.hit_count, &dialog.log_message, &dialog.types] {
            dialog._fields.push(cx.subscribe(input, |dialog, _, _: &Changed, cx| dialog.write_back(cx)));
        }
        dialog.show(cx);
        dialog
    }

    /// The breakpoints as the dialog has them, for a test.
    pub fn breakpoints(&self) -> &Breakpoints {
        &self.breakpoints
    }

    /// The condition field's focus, for a test that types into it without clicking.
    pub fn condition_focus(&self, cx: &App) -> FocusHandle {
        self.condition.focus_handle(cx)
    }

    /// What the condition field holds, for a test.
    pub fn condition_text(&self, cx: &App) -> String {
        self.condition.read(cx).text()
    }

    /// The chosen item's name, as the list shows it, for a test.
    pub fn chosen(&self) -> String {
        match &self.chosen {
            Chosen::Line { path, line } => format!("{}:{}", file_name(path), line + 1),
            Chosen::Function(index) => {
                self.breakpoints
                    .functions()
                    .get(*index)
                    .map(|function| {
                        if function.name.trim().is_empty() {
                            "Function breakpoint".into()
                        } else {
                            function.name.clone()
                        }
                    })
                    .unwrap_or_else(|| "Function breakpoint".into())
            }
            Chosen::AnyException => "Any exception".into(),
            Chosen::Exception(index) => self
                .breakpoints
                .exceptions()
                .get(*index)
                .map(|exception| exception.types.join(", "))
                .unwrap_or_else(|| "Any exception".into()),
        }
    }

    /// Fills the form from the chosen item without making that an edit.
    fn show(&mut self, cx: &mut Context<Self>) {
        let (name, condition, hit, log, types) = match &self.chosen {
            Chosen::Line { path, line } => match self.breakpoints.breakpoint(path, *line) {
                Some(breakpoint) => (
                    String::new(),
                    breakpoint.condition.clone(),
                    breakpoint.hit_count.clone(),
                    breakpoint.log_message.clone(),
                    String::new(),
                ),
                None => Default::default(),
            },
            Chosen::Function(index) => match self.breakpoints.functions().get(*index) {
                Some(function) => (
                    function.name.clone(),
                    function.condition.clone(),
                    function.hit_count.clone(),
                    function.log_message.clone(),
                    String::new(),
                ),
                None => Default::default(),
            },
            Chosen::AnyException => {
                let exception = self.breakpoints.any_exception();
                (
                    String::new(),
                    exception.condition.clone(),
                    exception.hit_count.clone(),
                    exception.log_message.clone(),
                    String::new(),
                )
            }
            Chosen::Exception(index) => match self.breakpoints.exceptions().get(*index) {
                Some(exception) => (
                    String::new(),
                    exception.condition.clone(),
                    exception.hit_count.clone(),
                    exception.log_message.clone(),
                    exception.types.join(", "),
                ),
                None => Default::default(),
            },
        };
        self.showing = true;
        self.name.update(cx, |field, cx| field.set_text(&name, cx));
        self.condition.update(cx, |field, cx| field.set_text(&condition, cx));
        self.hit_count.update(cx, |field, cx| field.set_text(&hit, cx));
        self.log_message.update(cx, |field, cx| field.set_text(&log, cx));
        self.types.update(cx, |field, cx| field.set_text(&types, cx));
        self.showing = false;
    }

    /// The fields' text into the chosen breakpoint, and the window told to keep and send it.
    fn write_back(&mut self, cx: &mut Context<Self>) {
        if self.showing {
            return;
        }
        let name = self.name.read(cx).text();
        let condition = self.condition.read(cx).text();
        let hit = self.hit_count.read(cx).text();
        let log = self.log_message.read(cx).text();
        let types = self.types.read(cx).text();
        match self.chosen.clone() {
            Chosen::Line { path, line } => {
                self.breakpoints.edit(&path, line, |breakpoint| {
                    breakpoint.condition = condition;
                    breakpoint.hit_count = hit;
                    breakpoint.log_message = log;
                });
            }
            Chosen::Function(index) => {
                if let Some(function) = self.breakpoints.function_mut(index) {
                    function.name = name;
                    function.condition = condition;
                    function.hit_count = hit;
                    function.log_message = log;
                }
            }
            Chosen::AnyException => {
                let exception = self.breakpoints.any_exception_mut();
                exception.condition = condition;
                exception.hit_count = hit;
                exception.log_message = log;
            }
            Chosen::Exception(index) => {
                if let Some(exception) = self.breakpoints.exceptions_mut().get_mut(index) {
                    exception.condition = condition;
                    exception.hit_count = hit;
                    exception.log_message = log;
                    exception.types = types
                        .split([',', ';'])
                        .map(str::trim)
                        .filter(|kind| !kind.is_empty())
                        .map(str::to_string)
                        .collect();
                }
            }
        }
        self.changed(cx);
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(Ended::Changed(self.breakpoints.clone()));
        cx.notify();
    }

    fn choose(&mut self, chosen: Chosen, cx: &mut Context<Self>) {
        self.chosen = chosen;
        self.show(cx);
        cx.notify();
    }

    fn toggle_enabled(&mut self, chosen: Chosen, cx: &mut Context<Self>) {
        match chosen {
            Chosen::Line { path, line } => {
                let enabled = self.breakpoints.breakpoint(&path, line).is_none_or(|breakpoint| !breakpoint.enabled);
                self.breakpoints.set_enabled(&path, line, enabled);
            }
            Chosen::Function(index) => {
                if let Some(function) = self.breakpoints.function_mut(index) {
                    function.enabled = !function.enabled;
                }
            }
            Chosen::AnyException => {
                let any = self.breakpoints.any_exception_mut();
                any.enabled = !any.enabled;
            }
            Chosen::Exception(index) => {
                if let Some(exception) = self.breakpoints.exceptions_mut().get_mut(index) {
                    exception.enabled = !exception.enabled;
                }
            }
        }
        self.changed(cx);
    }

    fn toggle_scenario(&mut self, scenario: &str, cx: &mut Context<Self>) {
        let exception = match self.chosen {
            Chosen::AnyException => Some(self.breakpoints.any_exception_mut()),
            Chosen::Exception(index) => self.breakpoints.exceptions_mut().get_mut(index),
            Chosen::Line { .. } | Chosen::Function(_) => None,
        };
        if let Some(exception) = exception {
            match exception.scenarios.iter().position(|known| known == scenario) {
                Some(at) => {
                    exception.scenarios.remove(at);
                }
                None => exception.scenarios.push(scenario.to_string()),
            }
            self.changed(cx);
        }
    }

    fn scenarios(&self) -> Vec<String> {
        match self.chosen {
            Chosen::AnyException => self.breakpoints.any_exception().scenarios.clone(),
            Chosen::Exception(index) => self
                .breakpoints
                .exceptions()
                .get(index)
                .map(|exception| exception.scenarios.clone())
                .unwrap_or_default(),
            Chosen::Line { .. } | Chosen::Function(_) => Vec::new(),
        }
    }

    fn enabled(&self) -> bool {
        match &self.chosen {
            Chosen::Line { path, line } => self.breakpoints.breakpoint(path, *line).is_some_and(|b| b.enabled),
            Chosen::Function(index) => self.breakpoints.functions().get(*index).is_some_and(|f| f.enabled),
            Chosen::AnyException => self.breakpoints.any_exception().enabled,
            Chosen::Exception(index) => self.breakpoints.exceptions().get(*index).is_some_and(|b| b.enabled),
        }
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        let chosen = match self.chosen.clone() {
            Chosen::Line { path, line } => {
                self.breakpoints.remove(&path, line);
                self.breakpoints
                    .of(&path)
                    .first()
                    .map_or(Chosen::AnyException, |breakpoint| Chosen::Line { path, line: breakpoint.line })
            }
            Chosen::Function(index) => {
                self.breakpoints.remove_function(index);
                if index >= self.breakpoints.functions().len() { Chosen::AnyException } else { Chosen::Function(index) }
            }
            Chosen::Exception(index) => {
                self.breakpoints.remove_exception(index);
                if index >= self.breakpoints.exceptions().len() {
                    Chosen::AnyException
                } else {
                    Chosen::Exception(index)
                }
            }
            Chosen::AnyException => Chosen::AnyException,
        };
        self.choose(chosen, cx);
        self.changed(cx);
    }

    fn add_function(&mut self, cx: &mut Context<Self>) {
        self.breakpoints.add_function();
        let index = self.breakpoints.functions().len().saturating_sub(1);
        self.choose(Chosen::Function(index), cx);
        self.changed(cx);
    }

    fn add_exception(&mut self, cx: &mut Context<Self>) {
        self.breakpoints.add_exception();
        let index = self.breakpoints.exceptions().len().saturating_sub(1);
        self.choose(Chosen::Exception(index), cx);
        self.changed(cx);
    }

    fn muted(&self) -> bool {
        self.breakpoints.muted()
    }

    fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        let muted = !self.breakpoints.muted();
        self.breakpoints.set_muted(muted);
        self.changed(cx);
    }

    fn verification(&self, path: &Path, line: usize) -> Option<(bool, Option<String>)> {
        self.verification
            .iter()
            .find(|(file, at, _, _)| *at == line && rsharp_core::paths::same(file, path))
            .map(|(_, _, verified, message)| (*verified, message.clone()))
    }

    fn line_rows(&self, cx: &mut Context<Self>) -> Vec<gpui::Stateful<gpui::Div>> {
        let mut rows = Vec::new();
        for path in self.breakpoints.files() {
            for breakpoint in self.breakpoints.of(&path) {
                let chosen = Chosen::Line { path: path.clone(), line: breakpoint.line };
                let selected = self.chosen == chosen;
                let name = format!("{}:{}", file_name(&path), breakpoint.line + 1);
                let summary = summary(&breakpoint.condition, &breakpoint.hit_count, &breakpoint.log_message);
                let checked = breakpoint.enabled;
                let choose = chosen.clone();
                rows.push(
                    row(name, checked, summary, selected)
                        .on_click(cx.listener(move |dialog, _: &ClickEvent, _, cx| dialog.choose(choose.clone(), cx)))
                        .child(tick(gpui::ElementId::Name(rows_key(&path, breakpoint.line).into()), checked).on_click(
                            cx.listener(move |dialog, _: &ClickEvent, _, cx| dialog.toggle_enabled(chosen.clone(), cx)),
                        )),
                );
            }
        }
        rows
    }

    fn function_rows(&self, cx: &mut Context<Self>) -> Vec<gpui::Stateful<gpui::Div>> {
        let mut rows = Vec::new();
        for (index, function) in self.breakpoints.functions().iter().enumerate() {
            let chosen = Chosen::Function(index);
            let name =
                if function.name.trim().is_empty() { "Function breakpoint".to_string() } else { function.name.clone() };
            let summary = summary(&function.condition, &function.hit_count, &function.log_message);
            let checked = function.enabled;
            rows.push(
                row(name, checked, summary, self.chosen == chosen)
                    .on_click(
                        cx.listener(move |dialog, _: &ClickEvent, _, cx| dialog.choose(Chosen::Function(index), cx)),
                    )
                    .child(tick(("function-enabled", index), checked).on_click(cx.listener(
                        move |dialog, _: &ClickEvent, _, cx| dialog.toggle_enabled(Chosen::Function(index), cx),
                    ))),
            );
        }
        rows
    }

    fn exception_rows(&self, cx: &mut Context<Self>) -> Vec<gpui::Stateful<gpui::Div>> {
        let mut rows = Vec::new();
        let any = self.breakpoints.any_exception();
        rows.push(
            row(
                "Any exception".into(),
                any.enabled,
                scenario_summary(&any.scenarios),
                self.chosen == Chosen::AnyException,
            )
            .on_click(cx.listener(|dialog, _: &ClickEvent, _, cx| dialog.choose(Chosen::AnyException, cx)))
            .child(
                tick("any-exception-enabled", any.enabled).on_click(
                    cx.listener(|dialog, _: &ClickEvent, _, cx| dialog.toggle_enabled(Chosen::AnyException, cx)),
                ),
            ),
        );
        for (index, exception) in self.breakpoints.exceptions().iter().enumerate() {
            let chosen = Chosen::Exception(index);
            let name =
                if exception.types.is_empty() { "Any exception".to_string() } else { exception.types.join(", ") };
            let checked = exception.enabled;
            let summary = scenario_summary(&exception.scenarios);
            rows.push(
                row(name, checked, summary, self.chosen == chosen)
                    .on_click(
                        cx.listener(move |dialog, _: &ClickEvent, _, cx| dialog.choose(Chosen::Exception(index), cx)),
                    )
                    .child(tick(("exception-enabled", index), checked).on_click(cx.listener(
                        move |dialog, _: &ClickEvent, _, cx| dialog.toggle_enabled(Chosen::Exception(index), cx),
                    ))),
            );
        }
        rows
    }
}

/// A list row: its name, a summary in grey, and the tick the caller adds first.
fn row(name: String, _checked: bool, summary: Option<SharedString>, selected: bool) -> gpui::Stateful<gpui::Div> {
    let selector = format!("breakpoint row: {name}");
    div()
        .id(gpui::ElementId::Name(selector.clone().into()))
        .debug_selector(move || selector.clone())
        .h(px(26.))
        .px(px(8.))
        .rounded(px(4.))
        .flex()
        .items_center()
        .gap(px(6.))
        .when(selected, |d| d.bg(theme::selection()))
        .when(!selected, |d| d.hover(|d| d.bg(theme::row_hover())))
        .child(div().flex_1().min_w_0().truncate().child(name))
        .children(summary.map(|summary| div().text_color(theme::text_secondary()).child(summary)))
}

/// Rider's 16px tick, without a label: a row's own, so its click does not also choose the row.
fn tick(id: impl Into<gpui::ElementId>, checked: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(16.))
        .flex_none()
        .rounded(px(3.))
        .flex()
        .items_center()
        .justify_center()
        .when(checked, |d| {
            d.bg(theme::accent())
                .child(gpui::svg().path("icons/check.svg").size(px(14.)).text_color(theme::on_accent()))
        })
        .when(!checked, |d| d.border_1().border_color(theme::control_border()))
}

fn labelled(label: &'static str, content: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().w(px(110.)).flex_none().text_color(theme::text_secondary()).child(label))
        .child(content)
}

fn boxed(field: &Entity<TextField>, width: f32, name: &'static str, cx: &App) -> gpui::Stateful<gpui::Div> {
    let focus = field.focus_handle(cx);
    div()
        .id(gpui::ElementId::Name(format!("breakpoint field box {name}").into()))
        .debug_selector(move || format!("breakpoint field: {name}"))
        .track_focus(&focus)
        .on_mouse_down(gpui::MouseButton::Left, {
            let focus = focus.clone();
            move |_, window, _| {
                window.focus(&focus);
                // The dialog itself takes focus on a click that bubbles to it; a field keeps its own.
                window.prevent_default();
            }
        })
        .on_click(move |_, window, _| {
            window.focus(&focus);
            window.prevent_default();
        })
        .w(px(width))
        .h(px(26.))
        .px(px(6.))
        .rounded(px(4.))
        .border_1()
        .border_color(theme::control_border())
        .bg(theme::search_field())
        .child(field.clone())
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

fn rows_key(path: &Path, line: usize) -> String {
    format!("{}-{line}", path.display())
}

fn summary(condition: &str, hit: &str, log: &str) -> Option<SharedString> {
    let mut parts = Vec::new();
    if !condition.trim().is_empty() {
        parts.push(format!("if {condition}"));
    }
    if !hit.trim().is_empty() {
        parts.push(format!("hit {hit}"));
    }
    if !log.trim().is_empty() {
        parts.push(format!("log {log}"));
    }
    (!parts.is_empty()).then(|| parts.join(", ").into())
}

fn scenario_summary(scenarios: &[String]) -> Option<SharedString> {
    let labels: Vec<&str> = SCENARIO_LABELS
        .iter()
        .filter(|(name, _)| scenarios.iter().any(|scenario| scenario == name))
        .map(|(_, label)| *label)
        .collect();
    (!labels.is_empty()).then(|| labels.join(", ").into())
}

impl Render for BreakpointsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let line_rows = self.line_rows(cx);
        let function_rows = self.function_rows(cx);
        let exception_rows = self.exception_rows(cx);
        let is_function = matches!(self.chosen, Chosen::Function(_));
        let is_any = self.chosen == Chosen::AnyException;
        let is_exception = matches!(self.chosen, Chosen::Exception(_) | Chosen::AnyException);
        let chosen_name = self.chosen();
        let verification = match &self.chosen {
            Chosen::Line { path, line } => self.verification(path, *line),
            _ => None,
        };
        let scenarios = self.scenarios();
        let enabled = self.enabled();
        let muted = self.muted();
        div()
            .key_context("BreakpointsDialog")
            .track_focus(&self.focus)
            .debug_selector(|| "breakpoints dialog".into())
            .w(px(760.))
            .h(px(540.))
            .flex()
            .flex_col()
            .rounded(px(10.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .shadow_lg()
            .on_action(cx.listener(|_, _: &Choose, _, cx| cx.emit(Ended::Dismissed)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(Ended::Dismissed)))
            .child(
                div()
                    .h(px(44.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Breakpoints")
                    .child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::text_secondary())
                            .child(format!("{} breakpoint(s)", self.breakpoints.count())),
                    )
                    .child(div().flex_1())
                    .child(
                        checkbox("Mute Breakpoints", muted)
                            .on_click(cx.listener(|dialog, _: &ClickEvent, _, cx| dialog.toggle_mute(cx))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .id("breakpoint list")
                            .w(px(300.))
                            .flex_none()
                            .overflow_y_scroll()
                            .px(px(8.))
                            .border_r_1()
                            .border_color(theme::popup_border())
                            .child(section("Line breakpoints"))
                            .children(line_rows)
                            .child(section("Function breakpoints"))
                            .children(function_rows)
                            .child(section("Exception breakpoints"))
                            .children(exception_rows),
                    )
                    .child(
                        div()
                            .id("breakpoint form")
                            .flex_1()
                            .min_w_0()
                            .overflow_y_scroll()
                            .px(px(16.))
                            .py(px(8.))
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(div().text_color(theme::text_secondary()).child(chosen_name))
                            .child(checkbox("Enabled", enabled).on_click(cx.listener(
                                |dialog, _: &ClickEvent, _, cx| {
                                    let chosen = dialog.chosen.clone();
                                    dialog.toggle_enabled(chosen, cx)
                                },
                            )))
                            .children(
                                is_function
                                    .then(|| labelled("Function name", boxed(&self.name, 380., "Function name", cx))),
                            )
                            .children(
                                matches!(self.chosen, Chosen::Exception(_)).then(|| {
                                    labelled("Exception types", boxed(&self.types, 380., "Exception types", cx))
                                }),
                            )
                            .when(is_exception, |d| {
                                d.child(
                                    div()
                                        .text_color(theme::text_secondary())
                                        .child("Suspend if (one of each half must hold)"),
                                )
                                .child(div().flex().flex_wrap().gap(px(10.)).children(
                                    SCENARIO_LABELS.iter().map(|(name, label)| {
                                        checkbox(*label, scenarios.iter().any(|scenario| scenario == name)).on_click(
                                            cx.listener(move |dialog, _: &ClickEvent, _, cx| {
                                                dialog.toggle_scenario(name, cx)
                                            }),
                                        )
                                    }),
                                ))
                            })
                            .child(labelled("Condition", boxed(&self.condition, 380., "Condition", cx)))
                            .child(labelled("Hit count", boxed(&self.hit_count, 200., "Hit count", cx)))
                            .child(labelled("Log message", boxed(&self.log_message, 380., "Log message", cx)))
                            .children(verification.map(|(verified, message)| {
                                let (colour, text) = if verified {
                                    (theme::text_secondary(), "The debugger will hit it".to_string())
                                } else {
                                    (
                                        theme::warning(),
                                        format!(
                                            "The debugger will not hit it: {}",
                                            message.unwrap_or_else(|| "no code of it in the build".into())
                                        ),
                                    )
                                };
                                div().text_color(colour).child(text)
                            }))
                            .children((is_any).then(|| {
                                div()
                                    .text_color(theme::text_secondary())
                                    .child("Any exception is every folder's and is never removed.")
                            })),
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
                    .child(
                        button("Add Function Breakpoint", false, true)
                            .on_click(cx.listener(|dialog, _: &ClickEvent, _, cx| dialog.add_function(cx))),
                    )
                    .child(
                        button("Add Exception Breakpoint", false, true)
                            .on_click(cx.listener(|dialog, _: &ClickEvent, _, cx| dialog.add_exception(cx))),
                    )
                    .child(button("Remove", false, !is_any).on_click(cx.listener(|dialog, _: &ClickEvent, _, cx| {
                        if dialog.chosen != Chosen::AnyException {
                            dialog.remove(cx);
                        }
                    })))
                    .child(div().flex_1())
                    .child(
                        button("Done", true, true)
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(Ended::Dismissed))),
                    ),
            )
    }
}

fn section(title: &'static str) -> impl IntoElement {
    div().h(px(26.)).px(px(8.)).flex().items_center().text_color(theme::text_secondary()).child(title)
}

/// The five cases, each with its label, for a test that reads the dialog's words.
pub fn scenarios() -> [(&'static str, &'static str); 5] {
    SCENARIO_LABELS
}

/// Rider's names, as the protocol sends them.
pub fn scenario_names() -> [&'static str; 5] {
    SCENARIOS
}
