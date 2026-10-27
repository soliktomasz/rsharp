//! Rider's Extract Resource and Extract Style dialogs over a XAML file (Passenger's XamlRefactoringDialogs): the XAML
//! host says what is at the caret and where it can go, the dialog asks the name, the place and what to take, and OK
//! makes the host's changes across the files, or says why not.

use crate::assets::icon;
use crate::field::TextField;
use crate::find_action::{Choose, Dismiss};
use crate::refactor_dialog::{Closed, boxed, field, labelled, small_button};
use crate::theme;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Task, Window, div, prelude::*,
    px,
};
use rsharp_lsp::Point;
use rsharp_lsp::xaml::XamlHost;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Extract {
    Resource,
    Style,
}

impl Extract {
    pub fn title(self) -> &'static str {
        match self {
            Extract::Resource => "Extract Resource",
            Extract::Style => "Extract Style",
        }
    }

    pub fn kind(self) -> &'static str {
        match self {
            Extract::Resource => "resource",
            Extract::Style => "style",
        }
    }
}

/// What the editor found at the caret for an extraction, for the window to ask its options.
pub struct OpenExtract {
    pub extract: Extract,
    pub host: XamlHost,
    pub project: PathBuf,
    pub path: PathBuf,
    pub text: String,
    pub at: Point,
    pub found: Value,
}

/// A property Extract Style may take: its name, as it reads, and whether it is chosen.
struct Property {
    name: String,
    text: String,
    chosen: bool,
}

pub struct XamlDialog {
    open: OpenExtract,
    name: Entity<TextField>,
    places: Vec<String>,
    place: usize,
    /// The value's other uses in the file, which Extract Resource replaces too while `everywhere` is on.
    others: u64,
    everywhere: bool,
    properties: Vec<Property>,
    problem: Option<String>,
    first: FocusHandle,
    working: Option<Task<()>>,
}

impl EventEmitter<Closed> for XamlDialog {}

impl XamlDialog {
    pub fn new(open: OpenExtract, cx: &mut Context<Self>) -> Self {
        let found = &open.found;
        let name = field(found["suggested"].as_str().unwrap_or_default(), cx);
        let strings = |key: &str| -> Vec<String> {
            found[key].as_array().into_iter().flatten().filter_map(|v| v.as_str()).map(String::from).collect()
        };
        let properties = found["properties"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| {
                let name = p["name"].as_str().unwrap_or_default().to_string();
                let text = format!("{name} = {}", p["written"].as_str().unwrap_or_default());
                // A binding is left out at first, as Rider leaves it.
                Property { name, text, chosen: p["binding"] != true }
            })
            .collect();
        let first = name.focus_handle(cx);
        Self {
            places: strings("places"),
            place: 0,
            others: found["others"].as_u64().unwrap_or(0),
            everywhere: true,
            properties,
            problem: None,
            name,
            first,
            working: None,
            open,
        }
    }

    /// OK: the changes made where chosen, or why not shown over the buttons.
    pub fn accept(&mut self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).text().to_string();
        let chosen: Vec<&str> = self.properties.iter().filter(|p| p.chosen).map(|p| p.name.as_str()).collect();
        if self.open.extract == Extract::Style && chosen.is_empty() {
            self.problem = Some("At least one property must be selected".into());
            cx.notify();
            return;
        }
        let more = json!({
            "name": name, "make": true, "place": self.place, "everywhere": self.everywhere, "chosen": chosen,
        });
        let o = &self.open;
        let (host, project, path, text, at, extract) =
            (o.host.clone(), o.project.clone(), o.path.clone(), o.text.clone(), o.at, o.extract);
        let answer = cx
            .background_executor()
            .spawn(async move { host.extract(&project, &path, &text, at, extract.kind(), more) });
        self.working = Some(cx.spawn(async move |this, cx| {
            let answer = answer.await.map_err(|e| e.0);
            let _ = this.update(cx, |dialog, cx| {
                // The edit was made from the file as the dialog opened on it; one edited since would take it misplaced.
                let current =
                    crate::buffer::find(&dialog.open.path, cx).map(|b| b.read(cx).document.text().to_string());
                let stale = current.is_some_and(|text| text != dialog.open.text);
                let made = answer.and_then(|a| match a["problem"].as_str() {
                    Some(problem) => Err(problem.to_string()),
                    None if stale => Err(format!("{} was not made: the file was edited meanwhile", extract.title())),
                    None => {
                        let edit = rsharp_lsp::workspace_edit(&a["edit"]);
                        crate::language::apply_workspace_edit(&edit, None, cx)?;
                        let title = a["title"].as_str().unwrap_or_default();
                        Ok(match extract {
                            Extract::Resource => {
                                let uses = a["uses"].as_u64().unwrap_or(1);
                                let places = if uses == 1 { "place" } else { "places" };
                                format!("Extracted resource '{name}' to {title}, used in {uses} {places}")
                            }
                            Extract::Style => format!("Extracted style '{name}' to {title}"),
                        })
                    }
                });
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

    /// Test access: the name typed over.
    pub fn set_name(&mut self, text: &str, cx: &mut Context<Self>) {
        self.name.update(cx, |f, cx| f.set_text(text, cx));
    }

    pub fn name(&self, cx: &App) -> String {
        self.name.read(cx).text().to_string()
    }

    pub fn places(&self) -> &[String] {
        &self.places
    }

    pub fn choose_place(&mut self, place: usize, cx: &mut Context<Self>) {
        self.place = place.min(self.places.len().saturating_sub(1));
        cx.notify();
    }

    /// Whether the value's other uses are offered, and replaced too.
    pub fn every_use(&self) -> Option<bool> {
        (self.others > 0).then_some(self.everywhere)
    }

    /// Extract Style's properties as they read, each with whether it is chosen.
    pub fn properties(&self) -> Vec<(String, bool)> {
        self.properties.iter().map(|p| (p.text.clone(), p.chosen)).collect()
    }

    pub fn toggle_property(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(p) = self.properties.get_mut(index) {
            p.chosen = !p.chosen;
            cx.notify();
        }
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }
}

impl Focusable for XamlDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.first.clone()
    }
}

fn check_row(id: (&'static str, usize), text: &str, chosen: bool) -> gpui::Stateful<gpui::Div> {
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
}

impl Render for XamlDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let places = self.places.iter().enumerate().map(|(i, place)| {
            check_row(("place", i), place, i == self.place)
                .on_click(cx.listener(move |d, _: &ClickEvent, _, cx| d.choose_place(i, cx)))
        });
        let every_use = self.every_use().map(|on| {
            let others = self.others;
            check_row(
                ("every-use", 0),
                &format!("Replace the {} other {}", others, if others == 1 { "use" } else { "uses" }),
                on,
            )
            .on_click(cx.listener(|d, _: &ClickEvent, _, cx| {
                d.everywhere = !d.everywhere;
                cx.notify();
            }))
        });
        let properties = self.properties.iter().enumerate().map(|(i, p)| {
            check_row(("property", i), &p.text, p.chosen)
                .on_click(cx.listener(move |d, _: &ClickEvent, _, cx| d.toggle_property(i, cx)))
        });
        div()
            .key_context("RefactorDialog")
            .w(px(560.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .rounded(px(10.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .shadow_lg()
            .on_action(cx.listener(|d, _: &Choose, _, cx| d.accept(cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(Closed(None))))
            .child(div().font_weight(FontWeight::SEMIBOLD).child(self.open.extract.title()))
            .child(labelled("Name", boxed(&self.name, 300.)))
            .child(div().text_color(theme::text_secondary()).child("Put it in"))
            .children(places)
            .children(every_use)
            .when(!self.properties.is_empty(), |d| {
                d.child(div().text_color(theme::text_secondary()).child("Properties")).children(properties)
            })
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
                        small_button("ok", "Next")
                            .bg(theme::accent())
                            .text_color(theme::on_accent())
                            .on_click(cx.listener(|d, _: &ClickEvent, _, cx| d.accept(cx))),
                    ),
            )
    }
}
