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

gpui::actions!(debug_view, [ApplySetValue]);

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
    /// Set Value: an expression's new value, in the frame the program is stopped in.
    SetValue {
        expression: String,
        value: String,
    },
    /// A thread of the paused program chosen: its frames and values are shown.
    SelectThread(i64),
    /// Copy a value to the clipboard, as the Variables menu does.
    CopyValue(String),
    /// Watch an expression, as the Variables menu does.
    AddWatch(String),
    /// Copy the stopped thread's stack, as the Frames menu does.
    CopyStack,
    /// Watch a value for a write that changes it, as Rider's Set Data Breakpoint does.
    SetDataBreakpoint(String),
    /// Remove the data breakpoint with this id.
    RemoveDataBreakpoint(String),
}

impl EventEmitter<DebugAction> for DebugView {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Idle,
    Starting,
    Running,
    Paused,
}

/// A thread of the paused program, as the Threads list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadRow {
    pub id: i64,
    pub name: String,
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
    /// What a Set Value or a watch asks of it again; none for one the debugger cannot name.
    pub evaluate_name: Option<String>,
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
    /// The paused program's threads, shown above the frames when there is more than one.
    threads: Vec<ThreadRow>,
    chosen_thread: Option<i64>,
    frames: Vec<FrameRow>,
    selected: usize,
    variables: Vec<Node>,
    watch: Entity<TextField>,
    watches: Vec<(String, String)>,
    /// The values a step over left at the top of the frame, as Rider's Return values group shows them.
    return_values: Vec<VariableRow>,
    /// The variable chosen in the tree, by its place in it, and the field that sets its value.
    selected_variable: Option<Vec<usize>>,
    set_value: Entity<TextField>,
    /// The chosen variable's expression in the debugger; none while no variable that can be set is chosen.
    set_expression: Option<String>,
    console: Entity<ConsoleView>,
    /// Whether every breakpoint is muted, as the folder has it.
    muted: bool,
    /// The data breakpoints of the session: each one's id and the member it watches.
    data_breakpoints: Vec<(String, String)>,
    /// The Variables or Frames menu open, and what it acts on.
    menu: Option<Menu>,
    focus: FocusHandle,
    _watch_subscription: Subscription,
}

/// A menu a row of the Debug window opens: a value's, or the frames'.
#[derive(Clone)]
enum Menu {
    Variable { name: String, value: String, evaluate: Option<String> },
    Frames,
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
            threads: Vec::new(),
            chosen_thread: None,
            frames: Vec::new(),
            selected: 0,
            variables: Vec::new(),
            watch,
            watches: Vec::new(),
            return_values: Vec::new(),
            selected_variable: None,
            set_value: cx.new(|cx| TextField::new("A new value", cx)),
            set_expression: None,
            console,
            muted: false,
            data_breakpoints: Vec::new(),
            menu: None,
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

    /// The paused program's threads, as the Threads list shows them.
    pub fn threads(&self) -> &[ThreadRow] {
        &self.threads
    }

    /// The thread chosen, whose frames the list shows.
    pub fn chosen_thread(&self) -> Option<i64> {
        self.chosen_thread
    }

    /// The program's threads and the one chosen, as a stop or a choice gives them.
    pub fn set_threads(&mut self, threads: Vec<ThreadRow>, chosen: Option<i64>, cx: &mut Context<Self>) {
        self.threads = threads;
        self.chosen_thread = chosen;
        cx.notify();
    }

    pub fn choose_thread(&mut self, id: i64, cx: &mut Context<Self>) {
        self.chosen_thread = Some(id);
        cx.notify();
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
        self.return_values.clear();
        self.threads.clear();
        self.chosen_thread = None;
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

    /// A frame's place, once the adapter decompiled its assembly: the Frames list shows where it is now.
    pub fn resolved(&mut self, index: usize, row: FrameRow, cx: &mut Context<Self>) {
        if let Some(slot) = self.frames.get_mut(index) {
            *slot = row;
        }
        cx.notify();
    }

    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        self.variables.clear();
        cx.notify();
    }

    pub fn set_variables(&mut self, variables: Vec<VariableRow>, cx: &mut Context<Self>) {
        // A step over puts the values its calls returned at the top of the frame, named "$ReturnValue…": Rider shows
        // them apart from the locals, as its Return values group does.
        let (returned, locals): (Vec<VariableRow>, Vec<VariableRow>) = variables
            .into_iter()
            .partition(|row| row.evaluate_name.as_deref().is_some_and(|name| name.starts_with("$ReturnValue")));
        self.return_values = returned;
        self.variables = locals.into_iter().map(|row| Node { row, open: false, children: None }).collect();
        // The chosen variable stays chosen if its place still holds one, its value read again.
        if let Some(path) = self.selected_variable.clone() {
            match find(&mut self.variables, &path) {
                Some(node) => {
                    let value = node.row.value.clone();
                    self.set_expression = node.row.evaluate_name.clone().filter(|name| !name.is_empty());
                    self.set_value.update(cx, |field, cx| field.set_text(&value, cx));
                }
                None => {
                    self.selected_variable = None;
                    self.set_expression = None;
                }
            }
        }
        cx.notify();
    }

    /// The values a step over returned, as the Return values group shows them.
    pub fn return_values(&self) -> &[VariableRow] {
        &self.return_values
    }

    /// The watches and what they answered, for a test.
    pub fn watches(&self) -> &[(String, String)] {
        &self.watches
    }

    /// A row's Add to Watches: the watch stands at once, its answer when the debugger gives it.
    pub fn add_watch(&mut self, expression: &str, cx: &mut Context<Self>) {
        if !expression.trim().is_empty() && !self.watches.iter().any(|(asked, _)| asked == expression) {
            self.watches.push((expression.to_string(), String::new()));
        }
        cx.notify();
    }

    /// Whether a row's menu is open, for a test.
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// The session's data breakpoints, as the window's own list shows them.
    pub fn set_data_breakpoints(&mut self, breakpoints: Vec<(String, String)>, cx: &mut Context<Self>) {
        self.data_breakpoints = breakpoints;
        cx.notify();
    }

    /// The data breakpoints' members, for a test.
    pub fn data_breakpoints(&self) -> Vec<String> {
        self.data_breakpoints.iter().map(|(_, location)| location.clone()).collect()
    }

    /// The names in the Variables tree, for a test.
    pub fn variable_names(&self) -> Vec<String> {
        fn walk(nodes: &[Node], names: &mut Vec<String>) {
            for node in nodes {
                names.push(node.row.name.clone());
                if let Some(children) = &node.children {
                    walk(children, names);
                }
            }
        }
        let mut names = Vec::new();
        walk(&self.variables, &mut names);
        names
    }

    /// A value as the Variables tree shows it, for a test.
    pub fn variable_value(&self, name: &str) -> Option<String> {
        fn walk(nodes: &[Node], name: &str) -> Option<String> {
            for node in nodes {
                if node.row.name == name {
                    return Some(node.row.value.clone());
                }
                if let Some(found) = node.children.as_deref().and_then(|children| walk(children, name)) {
                    return Some(found);
                }
            }
            None
        }
        walk(&self.variables, name)
    }

    /// A variable chosen in the tree: the field starts at its value, and Set Value asks the debugger for the rest.
    fn select_variable(
        &mut self,
        path: Vec<usize>,
        value: String,
        evaluate_name: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.selected_variable = Some(path);
        self.set_expression = evaluate_name.filter(|name| !name.is_empty());
        self.set_value.update(cx, |field, cx| field.set_text(&value, cx));
        cx.notify();
    }

    /// Set Value, from the field or its button: the chosen expression with the text typed.
    fn apply_set_value(&mut self, cx: &mut Context<Self>) {
        let Some(expression) = self.set_expression.clone() else { return };
        let value = self.set_value.read(cx).text();
        if value.trim().is_empty() {
            return;
        }
        cx.emit(DebugAction::SetValue { expression, value });
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
        self.return_values.clear();
        self.threads.clear();
        self.chosen_thread = None;
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

    /// A row's menu, in the column it was opened from, so every item is laid out where it is drawn.
    fn menu_panel(&self, menu: Menu, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut panel = div()
            .id("debug menu")
            .debug_selector(|| "debug menu".into())
            .mx(px(4.))
            .mb(px(4.))
            .flex()
            .flex_col()
            .py(px(4.))
            .rounded(px(4.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border());
        match menu {
            Menu::Variable { name, value, evaluate } => {
                // A local the debugger names nothing to evaluate is watched by the name its row shows.
                let expression = evaluate.filter(|expression| !expression.is_empty()).unwrap_or_else(|| name.clone());
                panel = panel
                    .child(self.menu_item("Copy Value", DebugAction::CopyValue(value), cx))
                    .child(self.menu_item("Copy Name", DebugAction::CopyValue(name), cx))
                    .child(self.menu_item("Add to Watches", DebugAction::AddWatch(expression.clone()), cx))
                    .child(self.menu_item("Set Data Breakpoint", DebugAction::SetDataBreakpoint(expression), cx));
            }
            Menu::Frames => {
                panel = panel.child(self.menu_item("Copy Stack", DebugAction::CopyStack, cx));
            }
        }
        panel.into_any_element()
    }

    fn menu_item(&self, label: &'static str, action: DebugAction, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        div()
            .id(gpui::ElementId::Name(format!("debug menu item {label}").into()))
            .debug_selector(move || format!("debug menu: {label}"))
            .h(px(26.))
            .px(px(10.))
            .flex()
            .items_center()
            .hover(|d| d.bg(theme::row_hover()))
            .child(label)
            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                view.menu = None;
                cx.emit(action.clone());
                cx.notify();
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
            .key_context("DebugWindow")
            .on_action(cx.listener(|view, _: &ApplySetValue, _, cx| view.apply_set_value(cx)))
            .relative()
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
                            .children((self.threads.len() > 1).then(|| {
                                let rows: Vec<gpui::AnyElement> = self
                                    .threads
                                    .iter()
                                    .enumerate()
                                    .map(|(index, thread)| {
                                        let chosen = self.chosen_thread == Some(thread.id);
                                        let id = thread.id;
                                        div()
                                            .id(("debug thread", index))
                                            .debug_selector(move || format!("debug thread {index}"))
                                            .h(px(24.))
                                            .px(px(6.))
                                            .rounded(px(4.))
                                            .flex()
                                            .items_center()
                                            .gap(px(6.))
                                            .when(chosen, |d| d.bg(theme::selection()))
                                            .when(!chosen, |d| d.hover(|d| d.bg(theme::row_hover())))
                                            .child(div().flex_1().min_w_0().truncate().child(thread.name.clone()))
                                            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                                cx.emit(DebugAction::SelectThread(id))
                                            }))
                                            .into_any_element()
                                    })
                                    .collect();
                                div().flex().flex_col().child(label("Threads")).children(rows)
                            }))
                            .child(label("Frames"))
                            .children(match &self.menu {
                                Some(menu @ Menu::Frames) => Some(self.menu_panel(menu.clone(), cx)),
                                _ => None,
                            })
                            .child(
                                div()
                                    .id("debug frames")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .px(px(4.))
                                    .on_mouse_down(
                                        gpui::MouseButton::Right,
                                        cx.listener(|view, _: &gpui::MouseDownEvent, _, cx| {
                                            view.menu = Some(Menu::Frames);
                                            cx.notify();
                                        }),
                                    )
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
                            // shortcut: a returned value's members are not opened here; Evaluate Expression can, until
                            // the group carries the tree's members.
                            .children((!self.return_values.is_empty()).then(|| {
                                let rows: Vec<gpui::AnyElement> = self
                                    .return_values
                                    .iter()
                                    .enumerate()
                                    .map(|(index, row)| {
                                        div()
                                            .id(("debug return value", index))
                                            .debug_selector(move || format!("debug return value {index}"))
                                            .h(px(24.))
                                            .px(px(6.))
                                            .rounded(px(4.))
                                            .flex()
                                            .items_center()
                                            .gap(px(4.))
                                            .hover(|d| d.bg(theme::row_hover()))
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
                                            .into_any_element()
                                    })
                                    .collect();
                                div().flex().flex_col().child(label("Return values")).children(rows)
                            }))
                            .child(label("Variables"))
                            .children(match &self.menu {
                                Some(menu @ Menu::Variable { .. }) => Some(self.menu_panel(menu.clone(), cx)),
                                _ => None,
                            })
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
                                            .when(self.selected_variable.as_ref() == Some(&path), |d| {
                                                d.bg(theme::selection())
                                            })
                                            .on_mouse_down(
                                                gpui::MouseButton::Right,
                                                cx.listener({
                                                    let name = row.name.clone();
                                                    let value = row.value.clone();
                                                    let evaluate = row.evaluate_name.clone();
                                                    move |view, _: &gpui::MouseDownEvent, _, cx| {
                                                        view.menu = Some(Menu::Variable {
                                                            name: name.clone(),
                                                            value: value.clone(),
                                                            evaluate: evaluate.clone(),
                                                        });
                                                        cx.notify();
                                                    }
                                                }),
                                            )
                                            .on_click(cx.listener({
                                                let path = path.clone();
                                                let row = row.clone();
                                                move |view, _: &ClickEvent, _, cx| {
                                                    view.select_variable(
                                                        path.clone(),
                                                        row.value.clone(),
                                                        row.evaluate_name.clone(),
                                                        cx,
                                                    );
                                                    if expandable {
                                                        let opened = toggle(&mut view.variables, &path);
                                                        if opened {
                                                            cx.emit(DebugAction::OpenVariable {
                                                                path: path.clone(),
                                                                reference,
                                                            });
                                                        }
                                                    }
                                                    cx.notify();
                                                }
                                            }))
                                    })),
                            )
                            .children(self.set_expression.clone().map(|_| {
                                let focus = self.set_value.focus_handle(cx);
                                div()
                                    .h(px(28.))
                                    .mx(px(6.))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(div().text_color(theme::text_secondary()).child("Set value"))
                                    .child(
                                        div()
                                            .id("set value box")
                                            .debug_selector(|| "debug field: Set value".into())
                                            .track_focus(&focus)
                                            .flex_1()
                                            .min_w_0()
                                            .h(px(24.))
                                            .px(px(6.))
                                            .rounded(px(4.))
                                            .border_1()
                                            .border_color(theme::control_border())
                                            .bg(theme::search_field())
                                            .on_mouse_down(gpui::MouseButton::Left, {
                                                let focus = focus.clone();
                                                move |_, window, _| {
                                                    window.focus(&focus);
                                                    window.prevent_default();
                                                }
                                            })
                                            .on_click(move |_, window, _| {
                                                window.focus(&focus);
                                                window.prevent_default();
                                            })
                                            .child(self.set_value.clone()),
                                    )
                                    .child(
                                        div()
                                            .id("debug: Set Value")
                                            .debug_selector(|| "debug: Set Value".into())
                                            .h(px(24.))
                                            .px(px(8.))
                                            .rounded(px(4.))
                                            .border_1()
                                            .border_color(theme::control_border())
                                            .hover(|d| d.bg(theme::button_hover()))
                                            .child("Set")
                                            .on_click(
                                                cx.listener(|view, _: &ClickEvent, _, cx| view.apply_set_value(cx)),
                                            ),
                                    )
                            }))
                            .children((!self.data_breakpoints.is_empty()).then(|| {
                                let rows: Vec<gpui::AnyElement> = self
                                    .data_breakpoints
                                    .iter()
                                    .enumerate()
                                    .map(|(index, (data_id, location))| {
                                        let id = data_id.clone();
                                        div()
                                            .id(("debug data breakpoint", index))
                                            .debug_selector(move || format!("debug data breakpoint {index}"))
                                            .h(px(24.))
                                            .px(px(6.))
                                            .flex()
                                            .items_center()
                                            .gap(px(4.))
                                            .hover(|d| d.bg(theme::row_hover()))
                                            .child(div().flex_1().min_w_0().truncate().child(location.clone()))
                                            .child(div().flex_none().text_color(theme::text_secondary()).child("×"))
                                            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                                view.menu = None;
                                                cx.emit(DebugAction::RemoveDataBreakpoint(id.clone()));
                                                cx.notify();
                                            }))
                                            .into_any_element()
                                    })
                                    .collect();
                                div().flex().flex_col().child(label("Data breakpoints")).children(rows)
                            }))
                            .child(label("Watches"))
                            .child(
                                div()
                                    .id("watch box")
                                    .debug_selector(|| "debug field: Watch".into())
                                    .h(px(28.))
                                    .mx(px(6.))
                                    .px(px(6.))
                                    .rounded(px(4.))
                                    .bg(theme::search_field())
                                    .flex()
                                    .items_center()
                                    .track_focus(&self.watch.focus_handle(cx))
                                    .on_mouse_down(gpui::MouseButton::Left, {
                                        let focus = self.watch.focus_handle(cx);
                                        move |_, window, _| {
                                            window.focus(&focus);
                                            window.prevent_default();
                                        }
                                    })
                                    .on_click({
                                        let focus = self.watch.focus_handle(cx);
                                        move |_, window, _| {
                                            window.focus(&focus);
                                            window.prevent_default();
                                        }
                                    })
                                    .child(self.watch.clone()),
                            )
                            .child(div().id("debug watches").max_h(px(110.)).overflow_y_scroll().px(px(4.)).children(
                                self.watches.iter().enumerate().map(|(index, (expression, result))| {
                                    div()
                                        .id(("debug watch", index))
                                        .debug_selector(move || format!("debug watch {index}"))
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
                    .child(
                        div()
                            .debug_selector(|| "debug console".into())
                            .flex_1()
                            .min_h_0()
                            .child(self.console.read(cx).editor().clone()),
                    ),
            )
    }
}

/// Opens a value's members, or closes them: whether they now need asking for.
fn toggle(nodes: &mut Vec<Node>, path: &[usize]) -> bool {
    let Some(node) = find(nodes, path) else { return false };
    node.open = !node.open;
    node.open && node.children.is_none()
}
