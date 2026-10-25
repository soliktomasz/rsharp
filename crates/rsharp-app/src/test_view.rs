//! Rider's docked Unit Tests window: source methods before a build, runner cases and results after it.

mod model;
use crate::assets::icon;
use crate::field::{Changed, TextField};
use crate::find_action::{SelectNext, SelectPrevious};
use crate::{test_finder::CodeTests, theme};
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Subscription, Window,
    anchored, deferred, div, point, prelude::*, px, uniform_list,
};
pub use model::{GroupLevel, PRESETS, TestModel, TestRow, UnitTestsState, categories, duration_grade, state_label};
use rsharp_core::testing::{TestItem, TestOutcome, TestState};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub enum TestAction {
    RunAll,
    RunSelected,
    RunFailed,
    Stop,
    Navigate { path: PathBuf, line: usize },
    Changed,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Popup {
    Grouping,
    Outcomes,
}

pub struct TestView {
    model: TestModel,
    state: UnitTestsState,
    search: Entity<TextField>,
    collapsed: BTreeSet<String>,
    rows: Vec<TestRow>,
    selected: Option<String>,
    running: bool,
    available: bool,
    looking: bool,
    status: String,
    log: String,
    popup: Option<Popup>,
    show_log: bool,
    focus: FocusHandle,
    _search_subscription: Subscription,
    results_refresh: Option<gpui::Task<()>>,
}
impl EventEmitter<TestAction> for TestView {}
impl Focusable for TestView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl TestView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextField::new("Search tests", cx));
        let subscription = cx.subscribe(&search, |v, _, _: &Changed, cx| v.refresh(cx));
        Self {
            model: TestModel::default(),
            state: UnitTestsState::default(),
            search,
            collapsed: BTreeSet::new(),
            rows: vec![],
            selected: None,
            running: false,
            available: true,
            looking: false,
            status: String::new(),
            log: String::new(),
            popup: None,
            show_log: false,
            focus: cx.focus_handle(),
            _search_subscription: subscription,
            results_refresh: None,
        }
    }
    pub fn set_code(&mut self, code: BTreeMap<PathBuf, CodeTests>, looking: bool, cx: &mut Context<Self>) {
        let changed = self.model.set_code(code);
        let looking_changed = self.looking != looking;
        self.looking = looking;
        if changed {
            self.refresh(cx);
        } else if looking_changed {
            cx.notify();
        }
    }
    pub fn discovered(&mut self, tests: Vec<TestItem>, cx: &mut Context<Self>) {
        self.model.discovered(tests);
        self.refresh(cx);
    }
    pub fn outcomes(&mut self, outcomes: Vec<TestOutcome>, cx: &mut Context<Self>) {
        self.model.outcomes(outcomes);
        if self.running {
            // Stream bursts repaint together so a large run does not rebuild its tree once per outcome.
            if self.results_refresh.is_none() {
                self.results_refresh = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(std::time::Duration::from_millis(50)).await;
                    let _ = this.update(cx, |view, cx| {
                        view.results_refresh = None;
                        view.refresh(cx);
                    });
                }));
            }
        } else {
            self.refresh(cx);
        }
    }
    pub fn clear_discovered(&mut self, cx: &mut Context<Self>) {
        self.model.clear_discovered();
        self.refresh(cx);
    }
    pub fn clear_discovered_in(&mut self, projects: &[PathBuf], cx: &mut Context<Self>) {
        self.model.clear_discovered_in(projects);
        self.refresh(cx);
    }
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.results_refresh = None;
        self.model = TestModel::default();
        self.rows.clear();
        self.selected = None;
        self.collapsed.clear();
        self.running = false;
        self.looking = false;
        self.status.clear();
        self.log.clear();
        self.popup = None;
        self.search.update(cx, |s, cx| s.set_text("", cx));
        cx.notify();
    }
    pub fn begin(&mut self, tests: &[TestItem], cx: &mut Context<Self>) {
        self.model.begin(tests);
        self.running = true;
        self.refresh(cx);
    }
    pub fn set_running(&mut self, running: bool, cx: &mut Context<Self>) {
        self.running = running;
        if !running {
            self.results_refresh = None;
            self.model.end();
        }
        self.refresh(cx);
    }
    pub fn is_running(&self) -> bool {
        self.running
    }
    pub fn set_available(&mut self, available: bool, cx: &mut Context<Self>) {
        if self.available != available {
            self.available = available;
            cx.notify();
        }
    }
    pub fn set_status(&mut self, status: String, cx: &mut Context<Self>) {
        self.status = status;
        cx.notify();
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn append_log(&mut self, line: String, cx: &mut Context<Self>) {
        self.log.push_str(&line);
        if !line.ends_with('\n') {
            self.log.push('\n');
        }
        // shortcut: retain the last 256 KiB of runner output, replace with Console if rich terminal output is needed.
        if self.log.len() > 262144 {
            let mut at = self.log.len() - 262144;
            while !self.log.is_char_boundary(at) {
                at += 1;
            }
            self.log.drain(..at);
        }
        cx.notify();
    }
    pub fn tests(&self) -> Vec<TestItem> {
        self.model.tests()
    }
    pub fn selected_tests(&self) -> Vec<TestItem> {
        self.selected_row().map(|r| r.tests.clone()).unwrap_or_default()
    }
    pub fn failed_tests(&self) -> Vec<TestItem> {
        self.model.failed_tests()
    }
    pub fn state(&self) -> UnitTestsState {
        self.state.clone()
    }
    pub fn restore(&mut self, state: UnitTestsState, cx: &mut Context<Self>) {
        self.state = state.clamped();
        self.refresh(cx);
    }
    pub fn rows(&self) -> &[TestRow] {
        &self.rows
    }
    pub fn search_field(&self) -> &Entity<TextField> {
        &self.search
    }
    pub fn grouping(&mut self, levels: Vec<GroupLevel>, cx: &mut Context<Self>) {
        self.state.grouping = levels;
        self.state = self.state.clone().clamped();
        self.changed(cx);
    }
    pub fn filter(&mut self, outcomes: Vec<TestState>, cx: &mut Context<Self>) {
        self.state.outcomes = outcomes;
        self.state = self.state.clone().clamped();
        self.changed(cx);
    }
    pub fn select(&mut self, key: Option<String>, cx: &mut Context<Self>) {
        self.selected = key;
        self.show_log = false;
        cx.notify();
    }
    pub fn toggle(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.collapsed.remove(key) {
            self.collapsed.insert(key.into());
        }
        self.refresh(cx);
    }
    fn selected_row(&self) -> Option<&TestRow> {
        self.selected.as_ref().and_then(|key| self.rows.iter().find(|r| &r.key == key))
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let selected = self
            .selected_row()
            .map(|row| (row.label.clone(), row.tests.iter().map(TestItem::key).collect::<BTreeSet<_>>()));
        self.rows = self.model.rows(&self.state, &self.search.read(cx).text(), &self.collapsed);
        if self.selected.as_ref().is_some_and(|key| !self.rows.iter().any(|row| &row.key == key)) {
            self.selected = selected.and_then(|(label, tests)| {
                self.rows
                    .iter()
                    .find(|row| {
                        row.label == label && row.tests.iter().map(TestItem::key).collect::<BTreeSet<_>>() == tests
                    })
                    .map(|row| row.key.clone())
            });
        }
        cx.notify();
    }
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.refresh(cx);
        cx.emit(TestAction::Changed);
    }
    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let at = self.selected.as_ref().and_then(|s| self.rows.iter().position(|r| &r.key == s));
        let next = at.map(|at| (at as isize + by).clamp(0, self.rows.len() as isize - 1) as usize).unwrap_or(0);
        self.select(Some(self.rows[next].key.clone()), cx);
    }
    fn command(
        &self,
        id: &'static str,
        label: &'static str,
        image: &'static str,
        enabled: bool,
        event: fn() -> TestAction,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        button(id, label, enabled)
            .debug_selector(move || format!("tests {id}"))
            .child(icon(image, 16.))
            .child(label)
            .when(enabled, |d| d.on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(event()))))
    }
    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .py(px(4.))
            .child(self.command(
                "run all",
                "Run All",
                "run",
                !self.running && self.available,
                || TestAction::RunAll,
                cx,
            ))
            .child(self.command(
                "run selected",
                "Run Selected",
                "run",
                !self.running && !self.selected_tests().is_empty(),
                || TestAction::RunSelected,
                cx,
            ))
            .child(self.command(
                "run failed",
                "Run Failed",
                "rerun",
                !self.running && !self.failed_tests().is_empty(),
                || TestAction::RunFailed,
                cx,
            ))
            .child(self.command("stop", "Stop", "stop", self.running, || TestAction::Stop, cx))
            .child(div().flex_1())
            .child(
                button("tests expand", "Expand All", true)
                    .child("Expand All")
                    .debug_selector(|| "tests expand all".into())
                    .on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                        v.collapsed.clear();
                        v.refresh(cx);
                    })),
            )
            .child(
                button("tests collapse", "Collapse All", true)
                    .child("Collapse All")
                    .debug_selector(|| "tests collapse all".into())
                    .on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                        v.collapsed.extend(v.rows.iter().filter(|r| r.expandable).map(|r| r.key.clone()));
                        v.refresh(cx);
                    })),
            )
    }
    fn filters(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let grouping = PRESETS
            .iter()
            .find(|(_, levels)| *levels == self.state.grouping.as_slice())
            .map(|(name, _)| *name)
            .unwrap_or("Custom");
        let filter = if self.state.outcomes.is_empty() {
            "All Outcomes".into()
        } else {
            self.state.outcomes.iter().map(|s| state_label(*s)).collect::<Vec<_>>().join(", ")
        };
        div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .items_center()
            .px(px(8.))
            .pb(px(6.))
            .child(
                div()
                    .id("tests search")
                    .debug_selector(|| "tests search".into())
                    .flex_1()
                    .min_w(px(120.))
                    .h(px(28.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(theme::control_border())
                    .child(icon("search", 16.))
                    .child(div().flex_1().child(self.search.clone())),
            )
            .child(
                button("tests grouping", "Group By", true)
                    .debug_selector(|| "tests group by".into())
                    .child(format!("Group By: {grouping}"))
                    .child(icon("chevron-down", 16.))
                    .children(self.popup(Popup::Grouping, cx))
                    .on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                        v.popup = if v.popup == Some(Popup::Grouping) { None } else { Some(Popup::Grouping) };
                        cx.notify();
                    })),
            )
            .child(
                button("tests outcomes", "Outcomes", true)
                    .debug_selector(|| "tests outcomes".into())
                    .child(filter)
                    .child(icon("chevron-down", 16.))
                    .children(self.popup(Popup::Outcomes, cx))
                    .on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                        v.popup = if v.popup == Some(Popup::Outcomes) { None } else { Some(Popup::Outcomes) };
                        cx.notify();
                    })),
            )
    }
    fn popup(&self, kind: Popup, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.popup != Some(kind) {
            return None;
        }
        let mut items = Vec::new();
        if kind == Popup::Grouping {
            for (i, (name, levels)) in PRESETS.iter().enumerate() {
                let levels = levels.to_vec();
                let chosen = self.state.grouping == levels;
                items.push(
                    choice(("tests preset", i), *name, chosen)
                        .debug_selector(move || format!("tests preset: {name}"))
                        .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            v.popup = None;
                            v.grouping(levels.clone(), cx);
                        }))
                        .into_any_element(),
                );
            }
            items.push(
                div().px(px(10.)).py(px(6.)).text_color(theme::text_secondary()).child("Levels").into_any_element(),
            );
            for (i, level) in GroupLevel::ALL.into_iter().enumerate() {
                items.push(
                    choice(("tests level", i), level.title(), self.state.grouping.contains(&level))
                        .debug_selector(move || format!("tests level: {}", level.title()))
                        .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            let mut levels = v.state.grouping.clone();
                            if levels.contains(&level) {
                                levels.retain(|l| l != &level);
                            } else {
                                levels.push(level);
                            }
                            v.grouping(levels, cx);
                        }))
                        .into_any_element(),
                );
            }
        } else {
            items.push(
                choice("tests all outcomes", "All Outcomes", self.state.outcomes.is_empty())
                    .debug_selector(|| "tests outcome: All".into())
                    .on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        v.filter(vec![], cx);
                    }))
                    .into_any_element(),
            );
            for (i, state) in
                [TestState::NotRun, TestState::Running, TestState::Passed, TestState::Failed, TestState::Skipped]
                    .into_iter()
                    .enumerate()
            {
                items.push(
                    choice(("tests outcome", i), state_label(state), self.state.outcomes.contains(&state))
                        .debug_selector(move || format!("tests outcome: {}", state_label(state)))
                        .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            let mut outcomes = v.state.outcomes.clone();
                            if outcomes.contains(&state) {
                                outcomes.retain(|s| s != &state);
                            } else {
                                outcomes.push(state);
                            }
                            v.filter(outcomes, cx);
                        }))
                        .into_any_element(),
                );
            }
        }
        Some(
            deferred(
                anchored()
                    .position_mode(gpui::AnchoredPositionMode::Local)
                    .position(point(px(0.), px(28.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .id("tests popup")
                            .min_w(px(230.))
                            .max_h(px(480.))
                            .overflow_y_scroll()
                            .py(px(4.))
                            .rounded(px(6.))
                            .bg(theme::popup())
                            .border_1()
                            .border_color(theme::popup_border())
                            .on_mouse_down_out(cx.listener(|v, _, _, cx| {
                                v.popup = None;
                                cx.notify();
                            }))
                            .children(items),
                    ),
            )
            .into_any_element(),
        )
    }
    fn tree_row(&self, index: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let row = &self.rows[index];
        let key = row.key.clone();
        let label = row.label.clone();
        let expand_key = key.clone();
        let selected = self.selected.as_ref() == Some(&key);
        let path = row.path.clone();
        let line = row.line;
        div()
            .id(("tests row", index))
            .debug_selector(move || format!("tests row: {label}"))
            .h(px(24.))
            .pl(px(6. + row.depth as f32 * 18.))
            .pr(px(8.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .gap(px(4.))
            .when(selected, |d| d.bg(theme::selection()))
            .hover(|d| d.bg(theme::row_hover()))
            .child(div().id(("tests toggle", index)).size(px(16.)).when(row.expandable, |d| {
                d.child(icon(if row.expanded { "chevron-down" } else { "chevron-right" }, 16.)).on_click(cx.listener(
                    move |v, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        v.toggle(&expand_key, cx);
                    },
                ))
            }))
            .child(icon(state_icon(row.state), 16.))
            .child(div().flex_1().min_w_0().truncate().child(row.label.clone()))
            .child(div().flex_none().text_color(theme::text_secondary()).child(if row.duration.is_zero() {
                String::new()
            } else {
                format!("{:.0} ms", row.duration.as_secs_f64() * 1000.)
            }))
            .on_click(cx.listener(move |v, event: &ClickEvent, window, cx| {
                window.focus(&v.focus);
                v.select(Some(key.clone()), cx);
                if event.click_count() == 2
                    && let Some(path) = &path
                {
                    cx.emit(TestAction::Navigate { path: path.clone(), line });
                }
            }))
            .into_any_element()
    }
    fn details(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut body = div()
            .id("tests details")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(12.))
            .py(px(6.))
            .flex()
            .flex_col()
            .gap(px(6.));
        if self.show_log {
            return body
                .font_family("JetBrains Mono")
                .child(if self.log.is_empty() { "No runner output yet".into() } else { self.log.clone() })
                .into_any_element();
        }
        let Some(row) = self.selected_row() else {
            return body
                .text_color(theme::text_secondary())
                .child("Select a test to see its result")
                .into_any_element();
        };
        body = body.child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(row.label.clone()));
        if let Some(path) = &row.path {
            let path = path.clone();
            let label = format!("{}:{}", path.display(), row.line);
            let line = row.line;
            body = body.child(
                div()
                    .id("tests source")
                    .debug_selector(|| "tests source".into())
                    .text_color(theme::accent())
                    .child(label)
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.emit(TestAction::Navigate { path: path.clone(), line })
                    })),
            );
        }
        if row.results.is_empty() {
            body = body.child(div().text_color(theme::text_secondary()).child(if row.state == TestState::Running {
                "Running…"
            } else {
                "This test has not run yet"
            }));
        }
        for (i, result) in row.results.iter().enumerate() {
            let name = if row.results.len() > 1 {
                format!("{} · {}", result.test.display_name, state_label(result.state))
            } else {
                state_label(result.state).into()
            };
            body = body.child(
                div().flex().items_center().gap(px(6.)).child(icon(state_icon(result.state), 16.)).child(name).child(
                    div()
                        .text_color(theme::text_secondary())
                        .child(format!("{:.0} ms", result.duration.as_secs_f64() * 1000.)),
                ),
            );
            if let Some(message) = &result.message {
                body = body.child(div().child(message.clone()));
            }
            if let Some(stack) = &result.stack_trace {
                for (j, line) in stack.lines().enumerate() {
                    let link = stack_location(line);
                    let text = line.to_string();
                    body = body.child(
                        div()
                            .id(SharedString::from(format!("tests stack {i} {j}")))
                            .font_family("JetBrains Mono")
                            .child(text)
                            .when_some(link, |d, (path, line)| {
                                d.text_color(theme::accent()).on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                    cx.emit(TestAction::Navigate { path: path.clone(), line })
                                }))
                            }),
                    );
                }
            }
            if let Some(output) = &result.output {
                body = body
                    .child(div().text_color(theme::text_muted()).child("Output"))
                    .child(div().font_family("JetBrains Mono").child(output.clone()));
            }
            for (j, attachment) in result.attachments.iter().enumerate() {
                let path = attachment.path.clone();
                body =
                    body.child(
                        div()
                            .id(SharedString::from(format!("tests attachment {i} {j}")))
                            .text_color(theme::accent())
                            .child(format!("{} · {}", attachment.name, path.display()))
                            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                cx.open_url(&rsharp_core::paths::to_uri(&path))
                            })),
                    );
            }
        }
        body.into_any_element()
    }
}
impl Render for TestView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.tests().len();
        let failed = self.failed_tests().len();
        let summary = if self.looking {
            "Looking for tests…".into()
        } else if self.status.is_empty() {
            format!("{count} tests · {failed} failed")
        } else {
            self.status.clone()
        };
        let empty = if self.looking {
            "Looking for tests…"
        } else if count == 0 {
            "No tests found. Add test methods to a test project."
        } else {
            "No tests match the search and outcome filters"
        };
        let tree = if self.rows.is_empty() {
            div().p(px(12.)).text_color(theme::text_secondary()).child(empty).into_any_element()
        } else {
            uniform_list(
                "unit tests tree",
                self.rows.len(),
                cx.processor(|v, range: std::ops::Range<usize>, _, cx| range.map(|i| v.tree_row(i, cx)).collect()),
            )
            .flex_1()
            .min_h_0()
            .into_any_element()
        };
        div()
            .id("unit tests")
            .debug_selector(|| "unit tests".into())
            .key_context("UnitTests")
            .track_focus(&self.focus)
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .text_size(px(13.))
            .text_color(theme::text())
            .on_action(cx.listener(|v, _: &SelectNext, _, cx| v.step(1, cx)))
            .on_action(cx.listener(|v, _: &SelectPrevious, _, cx| v.step(-1, cx)))
            .child(self.toolbar(cx))
            .child(self.filters(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(div().flex_1().min_w(px(180.)).min_h_0().flex().flex_col().child(tree))
                    .child(div().w(px(1.)).bg(theme::separator()))
                    .child(self.details(cx)),
            )
            .child(
                div()
                    .h(px(28.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().truncate().text_color(theme::text_muted()).child(summary))
                    .child(
                        button("tests log", "Runner Output", true)
                            .debug_selector(|| "tests runner output".into())
                            .child(if self.show_log { "Test Details" } else { "Runner Output" })
                            .on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                                v.show_log = !v.show_log;
                                cx.notify();
                            })),
                    ),
            )
    }
}
fn button(id: impl Into<gpui::ElementId>, _: impl Into<SharedString>, enabled: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(28.))
        .px(px(6.))
        .rounded(px(4.))
        .flex()
        .items_center()
        .gap(px(4.))
        .when(enabled, |d| d.hover(|d| d.bg(theme::button_hover())).active(|d| d.bg(theme::button_pressed())))
        .when(!enabled, |d| d.opacity(0.45))
}
fn choice(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, selected: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(24.))
        .px(px(8.))
        .mx(px(4.))
        .rounded(px(4.))
        .flex()
        .items_center()
        .gap(px(6.))
        .hover(|d| d.bg(theme::row_hover()))
        .child(div().size(px(16.)).when(selected, |d| d.child(icon("check", 16.))))
        .child(label.into())
}
fn state_icon(state: TestState) -> &'static str {
    match state {
        TestState::Failed => "error",
        TestState::Running => "run",
        TestState::Passed => "check",
        TestState::Skipped => "warning",
        TestState::NotRun => "file-csharp",
    }
}
/// Both .NET's `in path:line N` and adapter `path:N` locations.
fn stack_location(text: &str) -> Option<(PathBuf, usize)> {
    let text = text.trim();
    if let Some((before, line)) = text.rsplit_once(":line ") {
        let path = before.rsplit_once(" in ").map_or(before, |(_, path)| path).trim();
        return line.trim().parse().ok().map(|line| (PathBuf::from(path), line));
    }
    let (path, line) = text.rsplit_once(':')?;
    let line = line.trim().parse().ok()?;
    let path = path.rsplit_once(" in ").map_or(path, |(_, path)| path).trim();
    (!path.is_empty()).then(|| (path.into(), line))
}
