//! The refactorings that take options, as Rider's dialogs ask them: Change Signature, Extract Interface, Move, Pull
//! Members Up and Push Members Down. Passenger's extension in Roslyn describes what is at the caret; the dialog asks the
//! options over the window; OK sends them to the extension and makes what it answers across the files, or says why not.

use crate::assets::icon;
use crate::field::TextField;
use crate::find_action::{Choose, Dismiss};
use crate::language::{self, ask};
use crate::theme;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Task, Window, div, prelude::*,
    px,
};
use rsharp_lsp::{LanguageServer, Point, extension};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    ChangeSignature,
    ExtractInterface,
    Move,
    PullMembersUp,
    PushMembersDown,
}

impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Kind::ChangeSignature => "Change Signature",
            Kind::ExtractInterface => "Extract Interface",
            Kind::Move => "Move",
            Kind::PullMembersUp => "Pull Members Up",
            Kind::PushMembersDown => "Push Members Down",
        }
    }

    /// The handler that describes what is at the caret for it.
    pub fn describer(self) -> &'static str {
        match self {
            Kind::ChangeSignature => extension::DESCRIBE_SIGNATURE,
            Kind::ExtractInterface => extension::DESCRIBE_INTERFACE,
            Kind::Move => extension::DESCRIBE_TYPE,
            Kind::PullMembersUp | Kind::PushMembersDown => extension::DESCRIBE_HIERARCHY,
        }
    }
}

/// What the editor found at the caret for a refactoring, for the window to ask its options.
pub struct OpenRefactoring {
    pub kind: Kind,
    pub described: Value,
    pub at: Point,
    pub uri: String,
    pub server: Arc<LanguageServer>,
}

/// The dialog ended: what it changed (to say in the status bar), or nothing.
pub struct Closed(pub Option<String>);

/// A parameter of the signature asked for: the one it was (or -1 for a new one) and what it is now.
struct Parameter {
    from: i64,
    modifier: String,
    default_value: String,
    kind: Entity<TextField>,
    name: Entity<TextField>,
    call_value: Option<Entity<TextField>>,
}

/// A member to choose, by its id, as it reads, and whether it is chosen.
struct Member {
    id: String,
    text: String,
    chosen: bool,
}

enum Options {
    Signature { name: Entity<TextField>, returns: Entity<TextField>, parameters: Vec<Parameter> },
    Interface { name: Entity<TextField>, file: Entity<TextField>, members: Vec<Member> },
    Move { to_file: bool, file: Entity<TextField>, namespace: Entity<TextField> },
    Members { members: Vec<Member>, targets: Vec<(String, String)>, target: usize },
}

pub struct RefactorDialog {
    kind: Kind,
    options: Options,
    at: Point,
    uri: String,
    server: Arc<LanguageServer>,
    /// Why the last OK changed nothing, shown over the buttons.
    problem: Option<String>,
    /// Where the keys go as it opens (its first field), and its own, for a dialog of choices alone.
    first: FocusHandle,
    own: FocusHandle,
    working: Option<Task<()>>,
}

impl EventEmitter<Closed> for RefactorDialog {}

pub(crate) fn field(text: &str, cx: &mut App) -> Entity<TextField> {
    cx.new(|cx| {
        let mut field = TextField::new("", cx);
        field.set_text(text, cx);
        field
    })
}

fn members(described: &Value, key: &str, chosen: bool) -> Vec<Member> {
    described[key]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| Member {
            id: m["id"].as_str().unwrap_or_default().into(),
            text: m["text"].as_str().unwrap_or_default().into(),
            chosen,
        })
        .collect()
}

impl RefactorDialog {
    pub fn new(open: OpenRefactoring, cx: &mut Context<Self>) -> Self {
        let d = &open.described;
        let text = |key: &str| d[key].as_str().unwrap_or_default().to_string();
        let options = match open.kind {
            Kind::ChangeSignature => Options::Signature {
                name: field(&text("name"), cx),
                returns: field(&text("returnType"), cx),
                parameters: d["parameters"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(|(i, p)| Parameter {
                        from: i as i64,
                        modifier: p["modifier"].as_str().unwrap_or_default().into(),
                        default_value: p["defaultValue"].as_str().unwrap_or_default().into(),
                        kind: field(p["type"].as_str().unwrap_or_default(), cx),
                        name: field(p["name"].as_str().unwrap_or_default(), cx),
                        call_value: None,
                    })
                    .collect(),
            },
            Kind::ExtractInterface => Options::Interface {
                name: field(&text("name"), cx),
                file: field(&text("fileName"), cx),
                members: members(d, "members", true),
            },
            Kind::Move => Options::Move {
                to_file: true,
                file: field(&text("targetFileName"), cx),
                namespace: field(&text("namespace"), cx),
            },
            Kind::PullMembersUp | Kind::PushMembersDown => {
                let targets = if open.kind == Kind::PullMembersUp { "bases" } else { "derived" };
                Options::Members {
                    members: members(d, "members", false),
                    targets: members(d, targets, true).into_iter().map(|m| (m.id, m.text)).collect(),
                    target: 0,
                }
            }
        };
        let own = cx.focus_handle();
        let first = match &options {
            Options::Signature { name, .. } | Options::Interface { name, .. } => name.focus_handle(cx),
            Options::Move { file, .. } => file.focus_handle(cx),
            Options::Members { .. } => own.clone(),
        };
        Self {
            kind: open.kind,
            options,
            at: open.at,
            uri: open.uri,
            server: open.server,
            problem: None,
            first,
            own,
            working: None,
        }
    }

    /// What OK sends: the handler and its message, from the options as they stand.
    fn message(&self, cx: &App) -> Result<(&'static str, Value), String> {
        let at = extension::at(self.at);
        let text = |f: &Entity<TextField>| f.read(cx).text().trim().to_string();
        let chosen =
            |members: &[Member]| -> Vec<String> { members.iter().filter(|m| m.chosen).map(|m| m.id.clone()).collect() };
        Ok(match &self.options {
            Options::Signature { name, returns, parameters } => {
                let parameters: Vec<Value> = parameters
                    .iter()
                    .map(|p| {
                        json!({
                            "from": p.from, "modifier": p.modifier, "type": text(&p.kind), "name": text(&p.name),
                            "defaultValue": p.default_value, "callValue": p.call_value.as_ref().map(text).unwrap_or_default(),
                        })
                    })
                    .collect();
                (
                    extension::CHANGE_SIGNATURE,
                    json!({ "at": at, "name": text(name), "returnType": text(returns), "parameters": parameters }),
                )
            }
            Options::Interface { name, file, members } => (
                extension::EXTRACT_INTERFACE,
                json!({ "at": at, "name": text(name), "fileName": text(file), "members": chosen(members) }),
            ),
            Options::Move { to_file: true, file, .. } => {
                (extension::MOVE_TYPE_TO_FILE, json!({ "at": at, "fileName": text(file) }))
            }
            Options::Move { namespace, .. } => {
                (extension::MOVE_TYPE_TO_NAMESPACE, json!({ "at": at, "namespace": text(namespace) }))
            }
            Options::Members { members, targets, target } => {
                let moved = chosen(members);
                if moved.is_empty() {
                    return Err("Choose the members to move".into());
                }
                let (handler, target) = match self.kind {
                    Kind::PullMembersUp => {
                        let (id, _) =
                            targets.get(*target).ok_or("There is no base type in the solution to move them to")?;
                        (extension::PULL_MEMBERS_UP, id.clone())
                    }
                    _ if targets.is_empty() => return Err("No type derives from it to move them to".into()),
                    _ => (
                        extension::PUSH_MEMBERS_DOWN,
                        targets.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>().join("\n"),
                    ),
                };
                (handler, json!({ "at": at, "target": target, "members": moved, "abstract": [] }))
            }
        })
    }

    /// OK: the options sent to the extension and what it answers made across the files; its problem kept in the dialog.
    pub fn accept(&mut self, cx: &mut Context<Self>) {
        let (handler, message) = match self.message(cx) {
            Ok(sent) => sent,
            Err(why) => {
                self.problem = Some(why);
                cx.notify();
                return;
            }
        };
        let (method, params) = extension::request(handler, &message, &self.uri);
        let answer = ask(&self.server, method, params, cx);
        let (server, title) = (self.server.clone(), self.kind.title());
        self.working = Some(cx.spawn(async move |this, cx| {
            let answer = answer.await.ok_or_else(|| "Roslyn refused it".to_string());
            let _ = this.update(cx, |dialog, cx| {
                let made = answer.and_then(|a| extension::answer(&a)).and_then(|a| extension::refactored(&a)).and_then(
                    |edit| {
                        let files = edit.files.len();
                        language::apply_workspace_edit(&edit, Some(&server), cx).map(|_| {
                            format!("{title}: {files} {}", if files == 1 { "file changed" } else { "files changed" })
                        })
                    },
                );
                match made {
                    Ok(said) => cx.emit(Closed(Some(said))),
                    Err(why) => {
                        dialog.problem = Some(why);
                        cx.notify();
                    }
                }
            });
        }));
    }

    /// Test access: a text field of the dialog by its label, typed over.
    pub fn set(&mut self, label: &str, text: &str, cx: &mut Context<Self>) {
        let target = match (&self.options, label) {
            (Options::Signature { name, .. }, "name") | (Options::Interface { name, .. }, "name") => Some(name.clone()),
            (Options::Signature { returns, .. }, "returns") => Some(returns.clone()),
            (Options::Interface { file, .. }, "file") | (Options::Move { file, .. }, "file") => Some(file.clone()),
            (Options::Move { namespace, .. }, "namespace") => Some(namespace.clone()),
            (Options::Signature { parameters, .. }, label) => label
                .strip_prefix("parameter ")
                .and_then(|i| i.parse::<usize>().ok())
                .and_then(|i| parameters.get(i))
                .map(|p| p.name.clone()),
            _ => None,
        };
        if let Some(field) = target {
            field.update(cx, |f, cx| f.set_text(text, cx));
        }
    }

    /// Test access: Move's choice of a namespace over a file, and a member chosen by how it reads.
    pub fn choose(&mut self, what: &str, cx: &mut Context<Self>) {
        match &mut self.options {
            Options::Move { to_file, .. } => *to_file = what != "namespace",
            Options::Interface { members, .. } | Options::Members { members, .. } => {
                for m in members.iter_mut().filter(|m| m.text.contains(what)) {
                    m.chosen = !m.chosen;
                }
            }
            Options::Signature { .. } => {}
        }
        cx.notify();
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    fn add_parameter(&mut self, cx: &mut Context<Self>) {
        if let Options::Signature { parameters, .. } = &mut self.options {
            let (kind, name, call) = (field("int", cx), field("value", cx), field("0", cx));
            parameters.push(Parameter {
                from: -1,
                modifier: String::new(),
                default_value: String::new(),
                kind,
                name,
                call_value: Some(call),
            });
            cx.notify();
        }
    }

    fn move_parameter(&mut self, index: usize, by: isize, cx: &mut Context<Self>) {
        if let Options::Signature { parameters, .. } = &mut self.options {
            let to = index as isize + by;
            if by == 0 {
                parameters.remove(index);
            } else if to >= 0 && (to as usize) < parameters.len() {
                parameters.swap(index, to as usize);
            }
            cx.notify();
        }
    }
}

impl Focusable for RefactorDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.first.clone()
    }
}

pub(crate) fn boxed(field: &Entity<TextField>, width: f32) -> gpui::Div {
    div()
        .w(px(width))
        .h(px(26.))
        .px(px(6.))
        .rounded(px(4.))
        .border_1()
        .border_color(theme::control_border())
        .bg(theme::search_field())
        .child(field.clone())
}

pub(crate) fn labelled(label: &str, content: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().w(px(110.)).text_color(theme::text_secondary()).child(label.to_string()))
        .child(content)
}

pub(crate) fn small_button(id: impl Into<gpui::ElementId>, text: &str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(24.))
        .px(px(8.))
        .rounded(px(4.))
        .flex()
        .items_center()
        .border_1()
        .border_color(theme::control_border())
        .hover(|d| d.bg(theme::button_hover()))
        .child(text.to_string())
}

impl Render for RefactorDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let check_row = |id: (&'static str, usize), text: &str, chosen: bool| {
            div()
                .id(id)
                .h(px(24.))
                .px(px(6.))
                .rounded(px(4.))
                .flex()
                .items_center()
                .gap(px(8.))
                .hover(|d| d.bg(theme::row_hover()))
                .child(
                    div()
                        .size(px(16.))
                        .rounded(px(3.))
                        .border_1()
                        .border_color(theme::control_border())
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(chosen, |d| d.bg(theme::accent()).child(icon("check", 12.))),
                )
                .child(div().font_family(theme::CODE_FONT).truncate().child(text.to_string()))
        };
        let body: gpui::AnyElement =
            match &self.options {
                Options::Signature { name, returns, parameters } => {
                    let rows: Vec<_> =
                        parameters
                            .iter()
                            .enumerate()
                            .map(|(i, p)| {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(boxed(&p.kind, 140.))
                                    .child(boxed(&p.name, 140.))
                                    .children(p.call_value.as_ref().map(|c| boxed(c, 90.)))
                                    .child(small_button(("up", i), "↑").on_click(
                                        cx.listener(move |d, _: &ClickEvent, _, cx| d.move_parameter(i, -1, cx)),
                                    ))
                                    .child(small_button(("down", i), "↓").on_click(
                                        cx.listener(move |d, _: &ClickEvent, _, cx| d.move_parameter(i, 1, cx)),
                                    ))
                                    .child(small_button(("remove", i), "Remove").on_click(
                                        cx.listener(move |d, _: &ClickEvent, _, cx| d.move_parameter(i, 0, cx)),
                                    ))
                                    .into_any_element()
                            })
                            .collect();
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(labelled("Name", boxed(name, 300.)))
                        .child(labelled("Return type", boxed(returns, 300.)))
                        .child(
                            div()
                                .text_color(theme::text_secondary())
                                .child("Parameters (type, name, and for a new one the value each call passes)"),
                        )
                        .children(rows)
                        // In a row of its own, so the column does not stretch it.
                        .child(
                            div().flex().child(
                                small_button("add-parameter", "Add Parameter")
                                    .on_click(cx.listener(|d, _: &ClickEvent, _, cx| d.add_parameter(cx))),
                            ),
                        )
                        .into_any_element()
                }
                Options::Interface { name, file, members } => div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(labelled("Name", boxed(name, 300.)))
                    .child(labelled("File", boxed(file, 300.)))
                    .child(div().text_color(theme::text_secondary()).child("Members"))
                    .children(members.iter().enumerate().map(|(i, m)| {
                        let text = m.text.clone();
                        check_row(("member", i), &m.text, m.chosen)
                            .on_click(cx.listener(move |d, _: &ClickEvent, _, cx| d.choose(&text, cx)))
                    }))
                    .into_any_element(),
                Options::Move { to_file, file, namespace } => {
                    let chip = |id: &'static str, text: &'static str, on: bool| {
                        small_button(id, text).when(on, |d| d.bg(theme::tab_selected()))
                    };
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex()
                                .gap(px(6.))
                                .child(
                                    chip("to-file", "To Another File", *to_file)
                                        .on_click(cx.listener(|d, _: &ClickEvent, _, cx| d.choose("file", cx))),
                                )
                                .child(
                                    chip("to-namespace", "To Namespace", !*to_file)
                                        .on_click(cx.listener(|d, _: &ClickEvent, _, cx| d.choose("namespace", cx))),
                                ),
                        )
                        .child(if *to_file {
                            labelled("File", boxed(file, 300.))
                        } else {
                            labelled("Namespace", boxed(namespace, 300.))
                        })
                        .into_any_element()
                }
                Options::Members { members, targets, target } => {
                    let up = self.kind == Kind::PullMembersUp;
                    let to = targets.iter().enumerate().map(|(i, (_, text))| {
                        let chosen = !up || i == *target;
                        check_row(("target", i), text, chosen).on_click(cx.listener(move |d, _: &ClickEvent, _, cx| {
                            if let Options::Members { target, .. } = &mut d.options {
                                *target = i;
                            }
                            cx.notify();
                        }))
                    });
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(div().text_color(theme::text_secondary()).child(if up {
                            "To the base type"
                        } else {
                            "Into each derived type"
                        }))
                        .children(to)
                        .child(div().text_color(theme::text_secondary()).child("Members"))
                        .children(members.iter().enumerate().map(|(i, m)| {
                            let text = m.text.clone();
                            check_row(("member", i), &m.text, m.chosen)
                                .on_click(cx.listener(move |d, _: &ClickEvent, _, cx| d.choose(&text, cx)))
                        }))
                        .into_any_element()
                }
            };
        div()
            .key_context("RefactorDialog")
            .track_focus(&self.own)
            .w(px(620.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .rounded(px(10.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .shadow_lg()
            .on_action(cx.listener(|d, _: &Choose, _, cx| d.accept(cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(Closed(None))))
            .child(div().font_weight(FontWeight::SEMIBOLD).child(self.kind.title()))
            .child(body)
            .children(self.problem.clone().map(|p| div().text_color(theme::error()).child(p)))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        small_button("cancel", "Cancel")
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(Closed(None)))),
                    )
                    .child(
                        small_button("ok", "Refactor")
                            .bg(theme::accent())
                            .text_color(theme::on_accent())
                            .on_click(cx.listener(|d, _: &ClickEvent, _, cx| d.accept(cx))),
                    ),
            )
    }
}
