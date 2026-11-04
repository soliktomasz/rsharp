//! The Hierarchy tool window: Call Hierarchy's callers and callees and Type Hierarchy's supertypes and subtypes, from
//! Roslyn, each level asked as it opens, as Passenger's `MainWindow.Hierarchy` shows them.

use crate::assets::icon;
use crate::theme;
use gpui::{
    App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, SharedString, Task, Window, div, prelude::*, px,
};
use rsharp_lsp::{self as lsp, HierarchyItem, LanguageServer, Span};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

/// A symbol's name as Rider's hierarchy shows it, without the types it is in: `Shop.Calls.B()` is `B()`.
pub fn short_name(name: &str) -> &str {
    let params = name.find('(').unwrap_or(name.len());
    name[..params].rfind('.').map_or(name, |dot| &name[dot + 1..])
}

/// Which way the hierarchy is walked from its root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Callers,
    Callees,
    Supertypes,
    Subtypes,
}

impl Direction {
    fn title(self) -> &'static str {
        match self {
            Direction::Callers => "Callers",
            Direction::Callees => "Callees",
            Direction::Supertypes => "Supertypes",
            Direction::Subtypes => "Subtypes",
        }
    }

    fn method(self) -> &'static str {
        match self {
            Direction::Callers => "callHierarchy/incomingCalls",
            Direction::Callees => "callHierarchy/outgoingCalls",
            Direction::Supertypes => "typeHierarchy/supertypes",
            Direction::Subtypes => "typeHierarchy/subtypes",
        }
    }

    fn of_types(self) -> bool {
        matches!(self, Direction::Supertypes | Direction::Subtypes)
    }
}

/// Where a row goes: a file, a line and a column, from 0.
pub enum HierarchyEvent {
    Go { path: PathBuf, line: usize, column: usize },
}

struct Node {
    item: HierarchyItem,
    depth: usize,
    /// Where a double-click goes: a caller's call, else the symbol's name.
    at: Span,
    open: bool,
}

pub struct HierarchyView {
    server: Option<Arc<LanguageServer>>,
    title: String,
    directions: [Direction; 2],
    direction: Direction,
    nodes: Vec<Node>,
    selected: Option<usize>,
    focus: FocusHandle,
    asking: Vec<Task<()>>,
}

impl EventEmitter<HierarchyEvent> for HierarchyView {}

impl Focusable for HierarchyView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl HierarchyView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            server: None,
            title: String::new(),
            directions: [Direction::Callers, Direction::Callees],
            direction: Direction::Callers,
            nodes: Vec::new(),
            selected: None,
            focus: cx.focus_handle(),
            asking: Vec::new(),
        }
    }

    /// A hierarchy from `root`, walked the first of `directions`' ways; its first level asked at once.
    pub fn show(
        &mut self,
        server: Arc<LanguageServer>,
        title: String,
        root: HierarchyItem,
        directions: [Direction; 2],
        cx: &mut Context<Self>,
    ) {
        self.server = Some(server);
        self.title = title;
        self.directions = directions;
        self.walk(directions[0], root, cx);
    }

    pub fn title(&self) -> Option<String> {
        (!self.nodes.is_empty()).then(|| self.title.clone())
    }

    /// Each row as a person reads it: its name, indented two spaces a level.
    pub fn rows(&self) -> Vec<String> {
        self.nodes.iter().map(|n| format!("{}{}", "  ".repeat(n.depth), short_name(&n.item.name))).collect()
    }

    fn walk(&mut self, direction: Direction, root: HierarchyItem, cx: &mut Context<Self>) {
        self.asking.clear();
        self.direction = direction;
        let at = root.selection;
        self.nodes = vec![Node { item: root, depth: 0, at, open: false }];
        self.selected = Some(0);
        self.open(0, cx);
    }

    /// Opens a row: its next level asked of the server and set in under it.
    fn open(&mut self, index: usize, cx: &mut Context<Self>) {
        let (Some(server), Some(node)) = (self.server.clone(), self.nodes.get_mut(index)) else { return };
        node.open = true;
        let (raw, depth, direction) = (node.item.raw.clone(), node.depth, self.direction);
        let answer = crate::language::ask(&server, direction.method(), json!({ "item": raw }), cx);
        let task = cx.spawn(async move |this, cx| {
            let Some(answer) = answer.await else { return };
            let found: Vec<(HierarchyItem, Option<Span>)> = match direction {
                Direction::Callers => {
                    lsp::hierarchy_calls(&answer, "from").into_iter().map(|(i, r)| (i, r.first().copied())).collect()
                }
                Direction::Callees => lsp::hierarchy_calls(&answer, "to").into_iter().map(|(i, _)| (i, None)).collect(),
                _ => lsp::hierarchy_items(&answer).into_iter().map(|i| (i, None)).collect(),
            };
            let _ = this.update(cx, |view, cx| {
                // The row may have moved or gone while the server answered; it is found again by its item.
                let Some(at) = view.nodes.iter().position(|n| n.item.raw == raw && n.depth == depth && n.open) else {
                    return;
                };
                let children = found.into_iter().map(|(item, call)| {
                    let at = call.unwrap_or(item.selection);
                    Node { item, depth: depth + 1, at, open: false }
                });
                view.nodes.splice(at + 1..at + 1, children);
                cx.notify();
            });
        });
        self.asking.push(task);
        cx.notify();
    }

    fn close(&mut self, index: usize, cx: &mut Context<Self>) {
        let depth = self.nodes[index].depth;
        let end =
            self.nodes[index + 1..].iter().position(|n| n.depth <= depth).map_or(self.nodes.len(), |p| index + 1 + p);
        self.nodes.drain(index + 1..end);
        self.nodes[index].open = false;
        cx.notify();
    }

    fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.nodes[index].open { self.close(index, cx) } else { self.open(index, cx) }
    }

    fn go(&mut self, index: usize, cx: &mut Context<Self>) {
        let node = &self.nodes[index];
        cx.emit(HierarchyEvent::Go {
            path: node.item.path.clone(),
            line: node.at.start.line,
            column: node.at.start.character,
        });
    }

    fn row(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let node = &self.nodes[index];
        let chosen = self.selected == Some(index);
        let file = node.item.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let kind = if self.direction.of_types() { "node-class" } else { "node-method" };
        div()
            .id(("hierarchy row", index))
            .debug_selector(move || format!("hierarchy row: {index}"))
            .h(px(24.))
            .pl(px(6. + node.depth as f32 * 18.))
            .pr(px(8.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .gap(px(4.))
            .when(chosen, |d| d.bg(theme::selection()))
            .when(!chosen, |d| d.hover(|d| d.bg(theme::row_hover())))
            .child(
                div()
                    .id(("hierarchy toggle", index))
                    .size(px(16.))
                    .child(icon(if node.open { "chevron-down" } else { "chevron-right" }, 16.))
                    .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        v.toggle(index, cx);
                    })),
            )
            .child(icon(kind, 16.))
            .child(div().flex_none().child(short_name(&node.item.name).to_string()))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme::text_secondary())
                    .child(format!("{file}:{}", node.at.start.line + 1)),
            )
            .on_click(cx.listener(move |v, event: &ClickEvent, window, cx| {
                window.focus(&v.focus);
                v.selected = Some(index);
                if event.click_count() == 2 {
                    v.go(index, cx);
                }
                cx.notify();
            }))
    }
}

impl Render for HierarchyView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let segment = |direction: Direction, chosen: bool, cx: &mut Context<Self>| {
            let label = direction.title();
            div()
                .id(SharedString::from(format!("hierarchy {label}")))
                .debug_selector(move || format!("hierarchy: {label}"))
                .h(px(24.))
                .px(px(8.))
                .rounded(px(4.))
                .flex()
                .items_center()
                .when(chosen, |d| d.bg(theme::tab_selected()))
                .when(!chosen, |d| d.text_color(theme::text_muted()).hover(|d| d.bg(theme::button_hover())))
                .child(label)
                .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                    if v.direction != direction
                        && let Some(root) = v.nodes.first().map(|n| n.item.clone())
                    {
                        v.walk(direction, root, cx);
                    }
                }))
        };
        let body = if self.nodes.is_empty() {
            div()
                .p(px(12.))
                .text_color(theme::text_secondary())
                .child("Call Hierarchy (Ctrl+Alt+H) and Type Hierarchy show here.")
                .into_any_element()
        } else {
            div()
                .id("hierarchy rows")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(6.))
                .children((0..self.nodes.len()).map(|i| self.row(i, cx).into_any_element()))
                .into_any_element()
        };
        let [first, second] = self.directions;
        div()
            .key_context("Hierarchy")
            .track_focus(&self.focus)
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .text_size(px(13.))
            .text_color(theme::text())
            .when(!self.nodes.is_empty(), |d| {
                d.child(
                    div()
                        .h(px(32.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(segment(first, self.direction == first, cx))
                        .child(segment(second, self.direction == second, cx)),
                )
            })
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_name_is_shown_without_the_types_it_is_in() {
        assert_eq!(super::short_name("Shop.Calls.B()"), "B()");
        assert_eq!(super::short_name("Calls.Add(int a, System.String b)"), "Add(int a, System.String b)");
        assert_eq!(super::short_name("Shop.Calls"), "Calls");
        assert_eq!(super::short_name("Base"), "Base");
    }
}
