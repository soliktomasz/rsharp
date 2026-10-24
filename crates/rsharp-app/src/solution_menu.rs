//! The Solution view's row menu, as Rider's (Passenger's `ExplorerView` builds the same): Add and Edit submenus by
//! what the row stands for, with the keys their actions run by; and the popup that asks a name for New and Rename,
//! a kind to choose among when it makes a C# type.

use crate::field::{Changed, TextField};
use crate::find_action::{Choose, Dismiss, SelectNext, SelectPrevious};
use crate::solution_view::{NodeKind, TreeRow};
use crate::theme;
use gpui::{Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Window, div, prelude::*, px};

/// Rider's kinds of C# type in its New Class/Interface popup, in its order.
pub const TYPE_KINDS: [&str; 5] = ["Class", "Interface", "Record", "Struct", "Enum"];

#[derive(Clone, Debug, PartialEq)]
pub enum MenuItem {
    /// A title, and the keys shown beside it.
    Action(&'static str, Option<String>),
    Submenu(&'static str, Vec<MenuItem>),
}

impl MenuItem {
    pub fn title(&self) -> &'static str {
        match self {
            MenuItem::Action(title, _) | MenuItem::Submenu(title, _) => title,
        }
    }
}

/// A key as Rider's menus show it: its macOS keymap's symbols on macOS, its Visual Studio keymap's names elsewhere.
fn key(mac: &str, other: &str) -> Option<String> {
    Some(if cfg!(target_os = "macos") { mac } else { other }.to_string())
}

pub fn rename_key() -> Option<String> {
    key("⇧F6", "F2")
}

pub fn delete_key() -> Option<String> {
    key("⌫", "Delete")
}

/// What a row offers. A solution's own (its folders, projects and items) only leaves the solution, so it is Remove;
/// what is on disk is Delete.
pub fn menu_for(row: &TreeRow) -> Vec<MenuItem> {
    use MenuItem::{Action, Submenu};
    let item = row.key.starts_with("si:");
    let cut = Action("Cut", key("⌘X", "Ctrl+X"));
    let copy = Action("Copy", key("⌘C", "Ctrl+C"));
    let paste = Action("Paste", key("⌘V", "Ctrl+V"));
    let holds = || vec![Action("New File...", None), Action("New Folder...", None), Action("Class/Interface...", None)];
    let copy_path = Action("Copy Path", None);
    match row.kind {
        NodeKind::Solution => vec![
            Submenu(
                "Add",
                vec![
                    Action("New Project...", key("⇧⌘N", "Ctrl+Shift+N")),
                    Action("New Solution Folder...", None),
                    Action("Existing Project...", None),
                ],
            ),
            copy_path,
        ],
        NodeKind::SolutionFolder => vec![
            Submenu(
                "Add",
                vec![
                    Action("New Project...", key("⇧⌘N", "Ctrl+Shift+N")),
                    Action("New Solution Folder...", None),
                    Action("Existing Project...", None),
                    Action("Existing Item...", None),
                ],
            ),
            Submenu("Edit", vec![Action("Remove...", delete_key()), Action("Rename...", rename_key())]),
        ],
        NodeKind::Project if row.path.as_ref().is_some_and(|p| p.is_file()) => {
            let mut add = holds();
            add.push(Action("Reference...", None));
            vec![Submenu("Add", add), Submenu("Edit", vec![paste, Action("Remove...", delete_key())]), copy_path]
        }
        NodeKind::Project => vec![Submenu("Edit", vec![Action("Remove...", delete_key())])],
        NodeKind::Dependencies => vec![Action("Add Reference...", None)],
        NodeKind::ProjectReference => vec![Action("Remove...", delete_key())],
        NodeKind::Folder => vec![
            Submenu("Add", holds()),
            Submenu(
                "Edit",
                vec![cut, copy, paste, Action("Delete...", delete_key()), Action("Rename...", rename_key())],
            ),
            copy_path,
        ],
        NodeKind::File if item => vec![Submenu("Edit", vec![Action("Remove...", delete_key())]), copy_path],
        NodeKind::File => vec![
            Submenu(
                "Edit",
                vec![cut, copy, paste, Action("Delete...", delete_key()), Action("Rename...", rename_key())],
            ),
            copy_path,
        ],
        NodeKind::Framework | NodeKind::Package => Vec::new(),
    }
}

/// A menu's items as the tests and Find Action read them: "Add > New File...", a key in brackets after it.
pub fn flattened(items: &[MenuItem]) -> Vec<String> {
    let mut out = Vec::new();
    for item in items {
        match item {
            MenuItem::Action(title, keys) => out.push(match keys {
                Some(keys) => format!("{title} ({keys})"),
                None => title.to_string(),
            }),
            MenuItem::Submenu(title, inner) => {
                out.extend(flattened(inner).into_iter().map(|i| format!("{title} > {i}")))
            }
        }
    }
    out
}

/// The name asked, and the kind chosen when there are kinds to choose among.
pub enum Named {
    Chosen { name: String, kind: Option<&'static str> },
    Dismissed,
}

/// Rider's small popup that asks a name: New Solution Folder, New File, Rename and the like, with a list of kinds for a
/// C# type. A name refused keeps it open, saying why under the field.
pub struct NamePopup {
    title: SharedString,
    field: Entity<TextField>,
    kinds: Vec<&'static str>,
    kind: usize,
    problem: Option<String>,
    focus: FocusHandle,
}

impl EventEmitter<Named> for NamePopup {}

impl Focusable for NamePopup {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.field.focus_handle(cx)
    }
}

impl NamePopup {
    pub fn new(title: impl Into<SharedString>, name: &str, kinds: Vec<&'static str>, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| {
            let mut field = TextField::new("Name", cx);
            field.set_text(name, cx);
            field.select_all(cx);
            field
        });
        // A name typed again is a new question: what was wrong with the last one no longer holds.
        cx.subscribe(&field, |popup, _, _: &Changed, cx| {
            popup.problem = None;
            cx.notify();
        })
        .detach();
        Self { title: title.into(), field, kinds, kind: 0, problem: None, focus: cx.focus_handle() }
    }

    pub fn text(&self, cx: &gpui::App) -> String {
        self.field.read(cx).text()
    }

    pub fn kind(&self) -> Option<&'static str> {
        self.kinds.get(self.kind).copied()
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    pub fn refuse(&mut self, why: String, cx: &mut Context<Self>) {
        self.problem = Some(why);
        cx.notify();
    }
}

impl Render for NamePopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kinds: Vec<_> = self
            .kinds
            .iter()
            .enumerate()
            .map(|(i, kind)| {
                div()
                    .id(("kind", i))
                    .h(px(24.))
                    .mx(px(6.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .when(i == self.kind, |d| d.bg(theme::selection()))
                    .when(i != self.kind, |d| d.hover(|d| d.bg(theme::row_hover())))
                    .child(*kind)
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.kind = i;
                        cx.notify()
                    }))
            })
            .collect();
        div()
            .key_context("NamePopup")
            .track_focus(&self.focus)
            .w(px(360.))
            .flex()
            .flex_col()
            .pb(px(6.))
            .rounded(px(10.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .shadow_lg()
            .on_action(cx.listener(|p, _: &SelectPrevious, _, cx| {
                p.kind = p.kind.saturating_sub(1);
                cx.notify()
            }))
            .on_action(cx.listener(|p, _: &SelectNext, _, cx| {
                p.kind = (p.kind + 1).min(p.kinds.len().saturating_sub(1));
                cx.notify()
            }))
            .on_action(cx.listener(|p, _: &Choose, _, cx| {
                let name = p.text(cx);
                cx.emit(Named::Chosen { name, kind: p.kind() })
            }))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(Named::Dismissed)))
            .child(
                div()
                    .h(px(30.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .text_color(theme::text_secondary())
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .mx(px(10.))
                    .px(px(8.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(if self.problem.is_some() { theme::error() } else { theme::popup_border() })
                    .child(self.field.clone()),
            )
            .when_some(self.problem.clone(), |d, problem| {
                d.child(div().px(px(14.)).pt(px(4.)).text_color(theme::error()).child(problem))
            })
            .when(!kinds.is_empty(), |d| d.child(div().pt(px(6.)).children(kinds)))
    }
}
