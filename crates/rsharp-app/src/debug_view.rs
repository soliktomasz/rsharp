//! Rider's Debug window: the toolbar's step and run keys, the stopped thread's frames, the variables of the frame chosen
//! and the watches asked of it, over the debug console. What it shows is the window's to fill; what a person presses it
//! says as an action.

use crate::assets::icon;
use crate::console::ConsoleView;
use crate::field::{Changed, TextField};
use crate::theme;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Subscription, Window, div,
    prelude::*, px,
};
use std::path::PathBuf;

#[derive(Clone)]
pub enum DebugAction {
    Resume,
    Pause,
    Stop,
    StepOver,
    StepInto,
    StepOut,
    /// A frame of the stopped thread chosen, from 0.
    SelectFrame(usize),
    /// A variable with members opened: where it is in the tree, and the debugger's reference to them.
    OpenVariable {
        path: Vec<usize>,
        reference: i64,
    },
    /// A watch asked as it is typed.
    Evaluate(String),
    /// Mute Breakpoints, or let them stop the program again.
    ToggleMute,
}

impl EventEmitter<DebugAction> for DebugView {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Idle,
    Starting,
    Running,
    Paused,
}

/// A frame as the Frames list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameRow {
    pub name: String,
    /// "Program.cs:8", where it is.
    pub place: String,
    pub path: Option<PathBuf>,
    /// From 0.
    pub line: usize,
}

/// A value of the chosen frame, as Variables shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct VariableRow {
    pub name: String,
    pub value: String,
    pub r#type: Option<String>,
    /// The debugger's handle on its members; 0 for one that has none.
    pub reference: i64,
}

struct Node {
    row: VariableRow,
    open: bool,
    /// None until the debugger's answer for its members arrives.
    children: Option<Vec<Node>>,
}

pub struct DebugView {
    phase: Phase,
    /// What the window says of the session: starting, paused on what, why it ended.
    status: SharedString,
    frames: Vec<FrameRow>,
    selected: usize,
    variables: Vec<Node>,
    watch: Entity<TextField>,
    watches: Vec<(String, String)>,
    console: Entity<ConsoleView>,
    /// Whether every breakpoint is muted, as the folder has it.
    muted: bool,
    focus: FocusHandle,
    _watch_subscription: Subscription,
}

impl Focusable for DebugView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DebugView {
    pub fn new(console: Entity<ConsoleView>, cx: &mut Context<Self>) -> Self {
        let watch = cx.new(|cx| TextField::new("Add a watch", cx));
        let subscription = cx.subscribe(&watch, |view, _, _: &Changed, cx| {
            let expression = view.watch.read(cx).text();
            if expression.trim().is_empty() {
                view.watches.clear();
                cx.notify();
                return;
            }
            cx.emit(DebugAction::Evaluate(expression.trim().to_string()));
        });
        Self {
            phase: Phase::Idle,
            status: "Not debugging".into(),
            frames: Vec::new(),
            selected: 0,
            variables: Vec::new(),
            watch,
            watches: Vec::new(),
            console,
            muted: false,
            focus: cx.focus_handle(),
            _watch_subscription: subscription,
        }
    }

    pub fn console(&self) -> &Entity<ConsoleView> {
        &self.console
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Mute Breakpoints is on or off, as the folder says.
    pub fn set_muted(&mut self, muted: bool, cx: &mut Context<Self>) {
        self.muted = muted;
        cx.notify();
    }

    /// What the window says of the session, as the island's subtitle and the toolbar show it.
    pub fn status(&self) -> &SharedString {
        &self.status
    }

    /// The frame chosen, if the program is paused.
    pub fn frame(&self) -> Option<&FrameRow> {
        self.frames.get(self.selected)
    }

    pub fn frames(&self) -> &[FrameRow] {
        &self.frames
    }

    /// A session begins: the lists empty, and what it is told.
    pub fn starting(&mut self, name: &str, cx: &mut Context<Self>) {
        self.phase = Phase::Starting;
        self.status = format!("Starting {name}…").into();
        self.frames.clear();
        self.variables.clear();
        self.selected = 0;
        cx.notify();
    }

    pub fn running(&mut self, cx: &mut Context<Self>) {
        self.phase = Phase::Running;
        self.status = "Running".into();
        self.frames.clear();
        self.variables.clear();
        cx.notify();
    }

    /// The program stopped: its frames, with the innermost chosen.
    pub fn stopped(&mut self, status: &str, frames: Vec<FrameRow>, cx: &mut Context<Self>) {
        self.phase = Phase::Paused;
        self.status = status.to_string().into();
        self.frames = frames;
        self.selected = 0;
        self.variables.clear();
        cx.notify();
    }

    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        self.variables.clear();
        cx.notify();
    }

    pub fn set_variables(&mut self, variables: Vec<VariableRow>, cx: &mut Context<Self>) {
        self.variables = variables.into_iter().map(|row| Node { row, open: false, children: None }).collect();
        cx.notify();
    }

    /// The debugger's answer for a value's members: the rows of its children, shown at once.
    pub fn set_children(&mut self, path: &[usize], variables: Vec<VariableRow>, cx: &mut Context<Self>) {
        if let Some(node) = find(&mut self.variables, path) {
            node.children = Some(variables.into_iter().map(|row| Node { row, open: false, children: None }).collect());
            node.open = true;
        }
        cx.notify();
    }

    pub fn set_watch(&mut self, expression: &str, result: &str, cx: &mut Context<Self>) {
        match self.watches.iter_mut().find(|(asked, _)| asked == expression) {
            Some((_, answer)) => *answer = result.to_string(),
            None => self.watches.push((expression.to_string(), result.to_string())),
        }
        cx.notify();
    }

    /// What the window says of the session, when the adapter refused something.
    pub fn set_status(&mut self, status: &str, cx: &mut Context<Self>) {
        self.status = status.to_string().into();
        cx.notify();
    }

    /// The session ended: the lists go and what it made of the program is said.
    pub fn finished(&mut self, status: &str, cx: &mut Context<Self>) {
        self.phase = Phase::Idle;
        self.status = status.to_string().into();
        self.frames.clear();
        self.variables.clear();
        cx.notify();
    }

    fn tool(
        &self,
        id: &'static str,
        image: &'static str,
        enabled: bool,
        action: DebugAction,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.tool_lit(id, image, enabled, false, action, cx)
    }

    fn tool_lit(
        &self,
        id: &'static str,
        image: &'static str,
        enabled: bool,
        lit: bool,
        action: DebugAction,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .debug_selector(move || format!("debug: {id}"))
            .size(px(28.))
            .flex_none()
            .rounded(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .when(lit, |d| d.bg(theme::button_pressed()))
            .when(enabled, |d| d.hover(|d| d.bg(theme::button_hover())).active(|d| d.bg(theme::button_pressed())))
            .when(!enabled, |d| d.opacity(0.35))
            .child(icon(image, 16.))
            .tooltip(move |_, cx| cx.new(|_| crate::window::Tip(id.into())).into())
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                if enabled {
                    cx.emit(action.clone());
                }
            }))
    }

    fn rows(&self) -> Vec<(Vec<usize>, usize, VariableRow, bool)> {
        fn walk(
            nodes: &[Node],
            depth: usize,
            path: &mut Vec<usize>,
            rows: &mut Vec<(Vec<usize>, usize, VariableRow, bool)>,
        ) {
            for (index, node) in nodes.iter().enumerate() {
                path.push(index);
                rows.push((path.clone(), depth, node.row.clone(), node.open));
                if node.open {
                    if let Some(children) = &node.children {
                        walk(children, depth + 1, path, rows);
                    }
                }
                path.pop();
            }
        }
        let mut rows = Vec::new();
        let mut path = Vec::new();
        walk(&self.variables, 0, &mut path, &mut rows);
        rows
    }
}

fn find<'a>(nodes: &'a mut Vec<Node>, path: &[usize]) -> Option<&'a mut Node> {
    let (first, rest) = path.split_first()?;
    let node = nodes.get_mut(*first)?;
    if rest.is_empty() { Some(node) } else { find(node.children.as_mut()?, rest) }
}

impl Render for DebugView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let paused = self.phase == Phase::Paused;
        let running = matches!(self.phase, Phase::Running | Phase::Starting);
        let live = running || paused;
        let choosing = self.selected;
        let rows = self.rows();
        let frames = self.frames.clone();
        let label = |text: &'static str| {
            div().h(px(24.)).px(px(8.)).flex().items_center().text_color(theme::text_secondary()).child(text)
        };
        div()
            .debug_selector(|| "debug window".into())
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(32.))
                    .flex_none()
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(self.tool("Resume", "debug-resume", paused, DebugAction::Resume, cx))
                    .child(self.tool("Pause", "debug-pause", running, DebugAction::Pause, cx))
                    .child(self.tool("Step Over", "debug-step-over", paused, DebugAction::StepOver, cx))
                    .child(self.tool("Step Into", "debug-step-into", paused, DebugAction::StepInto, cx))
                    .child(self.tool("Step Out", "debug-step-out", paused, DebugAction::StepOut, cx))
                    .child(self.tool("Stop", "stop", live, DebugAction::Stop, cx))
                    .child(self.tool_lit(
                        "Mute Breakpoints",
                        "debug-mute",
                        true,
                        self.muted,
                        DebugAction::ToggleMute,
                        cx,
                    ))
                    .child(div().flex_1())
                    .child(div().pr(px(8.)).truncate().text_color(theme::text_secondary()).child(self.status.clone())),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .w(px(300.))
                            .flex_none()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .border_r_1()
                            .border_color(theme::separator())
                            .child(label("Frames"))
                            .child(
                                div()
                                    .id("debug frames")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .px(px(4.))
                                    .children(frames.is_empty().then(|| {
                                        div()
                                            .px(px(6.))
                                            .py(px(2.))
                                            .text_color(theme::text_secondary())
                                            .child("No frames while the program is not paused")
                                    }))
                                    .children(frames.iter().enumerate().map(|(index, frame)| {
                                        let chosen = index == choosing;
                                        div()
                                            .id(("frame", index))
                                            .debug_selector(move || format!("debug frame {index}"))
                                            .h(px(24.))
                                            .px(px(6.))
                                            .rounded(px(4.))
                                            .flex()
                                            .items_center()
                                            .gap(px(6.))
                                            .when(chosen, |d| d.bg(theme::selection()))
                                            .when(!chosen, |d| d.hover(|d| d.bg(theme::row_hover())))
                                            .child(div().flex_1().min_w_0().truncate().child(frame.name.clone()))
                                            .child(
                                                div()
                                                    .flex_none()
                                                    .text_color(theme::text_secondary())
                                                    .child(frame.place.clone()),
                                            )
                                            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                                view.select(index, cx);
                                                cx.emit(DebugAction::SelectFrame(index));
                                            }))
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .child(label("Variables"))
                            .child(
                                div()
                                    .id("debug variables")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .px(px(4.))
                                    .children(rows.is_empty().then(|| {
                                        div()
                                            .px(px(6.))
                                            .py(px(2.))
                                            .text_color(theme::text_secondary())
                                            .child("No values while the program is not paused")
                                    }))
                                    .children(rows.iter().map(|(path, depth, row, open)| {
                                        let path = path.clone();
                                        let reference = row.reference;
                                        let expandable = reference != 0;
                                        div()
                                            .id(gpui::ElementId::Name(format!("variable {path:?}").into()))
                                            .debug_selector({
                                                let name = row.name.clone();
                                                move || format!("debug variable {name}")
                                            })
                                            .h(px(24.))
                                            .pl(px(6. + *depth as f32 * 18.))
                                            .pr(px(6.))
                                            .rounded(px(4.))
                                            .flex()
                                            .items_center()
                                            .gap(px(4.))
                                            .hover(|d| d.bg(theme::row_hover()))
                                            .child(div().w(px(12.)).flex_none().child(if expandable {
                                                icon(if *open { "chevron-down" } else { "chevron-right" }, 12.)
                                                    .into_any_element()
                                            } else {
                                                div().into_any_element()
                                            }))
                                            .child(div().flex_none().child(row.name.clone()))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_color(theme::text_muted())
                                                    .child(format!("= {}", row.value)),
                                            )
                                            .children(row.r#type.clone().map(|kind| {
                                                div()
                                                    .flex_none()
                                                    .pr(px(4.))
                                                    .text_color(theme::text_secondary())
                                                    .child(kind)
                                            }))
                                            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                                if !expandable {
                                                    return;
                                                }
                                                let opened = toggle(&mut view.variables, &path);
                                                if opened {
                                                    cx.emit(DebugAction::OpenVariable {
                                                        path: path.clone(),
                                                        reference,
                                                    });
                                                }
                                                cx.notify();
                                            }))
                                    })),
                            )
                            .child(label("Watches"))
                            .child(
                                div()
                                    .h(px(28.))
                                    .mx(px(6.))
                                    .px(px(6.))
                                    .rounded(px(4.))
                                    .bg(theme::search_field())
                                    .flex()
                                    .items_center()
                                    .child(self.watch.clone()),
                            )
                            .child(div().id("debug watches").max_h(px(110.)).overflow_y_scroll().px(px(4.)).children(
                                self.watches.iter().map(|(expression, result)| {
                                    div()
                                        .h(px(24.))
                                        .px(px(6.))
                                        .flex()
                                        .items_center()
                                        .gap(px(4.))
                                        .child(div().flex_none().child(expression.clone()))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(theme::text_muted())
                                                .child(format!("= {result}")),
                                        )
                                }),
                            )),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(80.))
                    .flex()
                    .flex_col()
                    .border_t_1()
                    .border_color(theme::separator())
                    .child(
                        div()
                            .h(px(24.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .text_color(theme::text_secondary())
                            .child("Console"),
                    )
                    .child(div().flex_1().min_h_0().child(self.console.read(cx).editor().clone())),
            )
    }
}

/// Opens a value's members, or closes them: whether they now need asking for.
fn toggle(nodes: &mut Vec<Node>, path: &[usize]) -> bool {
    let Some(node) = find(nodes, path) else { return false };
    node.open = !node.open;
    node.open && node.children.is_none()
}
