//! The Live Visual Tree (Passenger's LiveTreeController and LiveTreeView): a debugged Avalonia app's windows and their
//! visual trees, asked of Passenger's inspector inside it (`rsharp_dap::inspector`), the chosen element's properties with
//! where each value comes from and its XAML, which a click opens; and each binding the app says failed, written in the
//! Debug console in a compiler message's shape, so its place links to the XAML. Nothing is asked while the app is paused:
//! its UI thread answers nothing then.

use super::*;
use crate::debug_view::Phase;
use rsharp_dap::inspector::{LiveApp, Told};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

/// The launch's inspector and what it last said.
pub(super) struct Live {
    app: Arc<LiveApp>,
    /// What the window says of the app: waiting for it, its Avalonia, why it cannot be inspected, or that it ended.
    status: String,
    connected: bool,
    roots: Vec<Value>,
    open: HashSet<i64>,
    chosen: Option<i64>,
    details: Option<Value>,
    asking: Option<Task<()>>,
    _events: Task<()>,
}

/// One row of the tree as drawn: its element, how deep, and whether it has children.
struct Row {
    id: i64,
    depth: usize,
    label: String,
    sourced: bool,
    children: bool,
}

impl MainWindow {
    /// The inspector for a debugged Avalonia app, listening before it starts; the variables its launch adds. None for any
    /// other program, or where the inspector cannot listen (said in the Live Visual Tree).
    pub(super) fn inspect(
        &mut self,
        plan: &rsharp_core::run::LaunchPlan,
        cx: &mut Context<Self>,
    ) -> BTreeMap<String, String> {
        self.live = None;
        if !plan.built.as_ref().is_some_and(|built| rsharp_dap::inspector::inspects(built)) {
            return BTreeMap::new();
        };
        let (sender, mut told) = futures::channel::mpsc::unbounded();
        let listened = LiveApp::listen(&plan.environment, move |said| drop(sender.unbounded_send(said)));
        let (app, variables) = match listened {
            Ok(listened) => listened,
            Err(why) => {
                log::info!("Live Visual Tree: {why}");
                return BTreeMap::new();
            }
        };
        let events = cx.spawn(async move |this, cx| {
            while let Some(said) = told.next().await {
                if this.update(cx, |w, cx| w.live_told(said, cx)).is_err() {
                    return;
                }
            }
        });
        self.live = Some(Live {
            app: Arc::new(app),
            status: format!("Waiting for {} to show its UI…", plan.name),
            connected: false,
            roots: Vec::new(),
            open: HashSet::new(),
            chosen: None,
            details: None,
            asking: None,
            _events: events,
        });
        variables
    }

    fn live_told(&mut self, said: Told, cx: &mut Context<Self>) {
        let Some(live) = &mut self.live else { return };
        match said {
            Told::Hello { problem: Some(problem), .. } => live.status = problem,
            Told::Hello { avalonia, .. } => {
                live.connected = true;
                live.status = avalonia.map_or("Avalonia".into(), |v| format!("Avalonia {v}"));
                self.refresh_live_tree(cx);
            }
            Told::TreeChanged => self.refresh_live_tree(cx),
            Told::Picked(element) => {
                if let Some(element) = element {
                    self.choose_live(element, cx);
                }
            }
            Told::BindingFailed(failure) => self.binding_failed(&failure, cx),
            Told::ValuesChanged => {}
            Told::Ended => {
                live.connected = false;
                live.status = "The app has ended".into();
            }
        }
        cx.notify();
    }

    /// A failed binding, as Passenger shows one: a console line led by its XAML place in a compiler message's shape,
    /// which links to the line, once each time the app says it again.
    fn binding_failed(&mut self, failure: &Value, cx: &mut Context<Self>) {
        let text = |key: &str| failure[key].as_str().unwrap_or_default();
        let at = if text("errorPoint").is_empty() || text("errorPoint") == text("path") {
            String::new()
        } else {
            format!(" at {}", text("errorPoint"))
        };
        let described =
            format!("{}.{} to {}{at}: {}", text("control"), text("property"), text("path"), text("message"));
        let place = &failure["place"];
        let line = match place["file"].as_str() {
            Some(file) => format!("{file}({},{}): binding error: {described}\n", place["line"], place["column"]),
            None => format!("Binding error: {described}\n"),
        };
        self.debug_console.update(cx, |view, cx| view.append(&[(line, true)], cx));
    }

    /// The app's windows and their trees asked again, the chosen element's details with them; not while it is paused.
    pub(super) fn refresh_live_tree(&mut self, cx: &mut Context<Self>) {
        let paused = self.debug_phase() == Phase::Paused;
        let Some(live) = &mut self.live else { return };
        if !live.connected || paused {
            return;
        }
        let (app, chosen) = (live.app.clone(), live.chosen);
        let asked = cx.background_executor().spawn(async move {
            let tree = app.ask("tree", json!({}));
            let details = chosen.map(|element| app.ask("details", json!({ "element": element })));
            (tree, details)
        });
        live.asking = Some(cx.spawn(async move |this, cx| {
            let (tree, details) = asked.await;
            let _ = this.update(cx, |w, cx| {
                let Some(live) = &mut w.live else { return };
                match tree {
                    Ok(tree) => {
                        live.roots = tree["tree"]["roots"].as_array().cloned().unwrap_or_default();
                        // First shown, the tree is open down to the app's own XAML elements, as Passenger's opens.
                        if live.open.is_empty() {
                            fn sourced(nodes: &[Value], open: &mut HashSet<i64>) -> bool {
                                let mut any = false;
                                for node in nodes {
                                    let children = node["children"].as_array().map_or(&[][..], |c| c);
                                    if sourced(children, open) {
                                        open.insert(node["id"].as_i64().unwrap_or_default());
                                        any = true;
                                    }
                                    any |= node["sourced"] == true;
                                }
                                any
                            }
                            sourced(&live.roots, &mut live.open);
                        }
                    }
                    Err(why) => live.status = why,
                }
                if let Some(details) = details {
                    live.details = details.ok().map(|d| d["details"].clone());
                }
                cx.notify();
            });
        }));
    }

    /// An element chosen in the tree, or picked in the app: its ancestors opened, its details asked, it outlined there.
    pub(super) fn choose_live(&mut self, element: i64, cx: &mut Context<Self>) {
        let Some(live) = &mut self.live else { return };
        live.chosen = Some(element);
        live.details = None;
        fn path(nodes: &[Value], to: i64, through: &mut Vec<i64>) -> bool {
            nodes.iter().any(|node| {
                let id = node["id"].as_i64().unwrap_or_default();
                through.push(id);
                if id == to || path(node["children"].as_array().map_or(&[][..], |c| c), to, through) {
                    return true;
                }
                through.pop();
                false
            })
        }
        let mut through = Vec::new();
        if path(&live.roots, element, &mut through) {
            live.open.extend(through.into_iter().filter(|id| *id != element));
        }
        let app = live.app.clone();
        let asked = cx.background_executor().spawn(async move {
            let _ = app.ask("highlight", json!({ "element": element }));
            app.ask("details", json!({ "element": element }))
        });
        live.asking = Some(cx.spawn(async move |this, cx| {
            let details = asked.await;
            let _ = this.update(cx, |w, cx| {
                if let Some(live) = &mut w.live
                    && live.chosen == Some(element)
                {
                    match details {
                        Ok(details) => live.details = Some(details["details"].clone()),
                        Err(why) => live.status = why,
                    }
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn toggle_live(&mut self, element: i64, cx: &mut Context<Self>) {
        if let Some(live) = &mut self.live
            && !live.open.remove(&element)
        {
            live.open.insert(element);
        }
        cx.notify();
    }

    /// Select Element: the next click in the app chooses what is under it, as Passenger's pick does.
    fn pick_live(&mut self, cx: &mut Context<Self>) {
        let Some(live) = &self.live else { return };
        let app = live.app.clone();
        cx.background_executor()
            .spawn(async move {
                let _ = app.ask("pick", json!({ "on": true }));
            })
            .detach();
    }

    fn live_rows(live: &Live) -> Vec<Row> {
        fn walk(nodes: &[Value], depth: usize, open: &HashSet<i64>, rows: &mut Vec<Row>) {
            for node in nodes {
                let id = node["id"].as_i64().unwrap_or_default();
                let children = node["children"].as_array().map_or(&[][..], |c| c);
                let name = node["name"].as_str().map(|n| format!("#{n}")).unwrap_or_default();
                let classes: Vec<&str> =
                    node["classes"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                let classes = if classes.is_empty() { String::new() } else { format!(" .{}", classes.join(" .")) };
                rows.push(Row {
                    id,
                    depth,
                    label: format!("{}{name}{classes}", node["type"].as_str().unwrap_or("?")),
                    sourced: node["sourced"] == true,
                    children: !children.is_empty(),
                });
                if open.contains(&id) {
                    walk(children, depth + 1, open, rows);
                }
            }
        }
        let mut rows = Vec::new();
        walk(&live.roots, 0, &live.open, &mut rows);
        rows
    }

    /// Opens an element's XAML, or a style's or a value's place, where the app's build recorded it.
    fn open_live_place(&mut self, place: &Value, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = place["file"].as_str() else { return };
        let (line, column) = (place["line"].as_u64().unwrap_or(1), place["column"].as_u64().unwrap_or(1));
        let at = Some((line.saturating_sub(1) as usize, column.saturating_sub(1) as usize));
        self.open_file(&PathBuf::from(file), at, window, cx);
    }

    pub(super) fn live_tree_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(live) = &self.live else {
            return empty_text("The Live Visual Tree shows an Avalonia app you debug: start one with Debug.")
                .track_focus(&self.live_focus)
                .into_any_element();
        };
        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .py(px(4.))
            .child(div().flex_1().text_color(theme::text_secondary()).child(live.status.clone()))
            .child(
                div()
                    .id("live-pick")
                    .debug_selector(|| "live tree: Select Element".into())
                    .px(px(6.))
                    .rounded(px(4.))
                    .hover(|d| d.bg(theme::button_hover()))
                    .child("Select Element")
                    .on_click(cx.listener(|w, _: &ClickEvent, _, cx| w.pick_live(cx))),
            )
            .child({
                let on = self.breaks_on_binding_errors();
                div()
                    .id("live-binding-errors")
                    .debug_selector(|| "live tree: Break on binding errors".into())
                    .px(px(6.))
                    .rounded(px(4.))
                    .when(on, |d| d.bg(theme::selection()))
                    .hover(|d| d.bg(theme::button_hover()))
                    .child("Break on binding errors")
                    .on_click(cx.listener(|w, _: &ClickEvent, _, cx| w.toggle_binding_errors(cx)))
            })
            .child(
                div()
                    .id("live-refresh")
                    .debug_selector(|| "live tree: Refresh".into())
                    .px(px(6.))
                    .rounded(px(4.))
                    .hover(|d| d.bg(theme::button_hover()))
                    .child("Refresh")
                    .on_click(cx.listener(|w, _: &ClickEvent, _, cx| w.refresh_live_tree(cx))),
            );
        let details = live.details.as_ref().map(|details| self.live_details(details, cx).into_any_element());
        let rows: Vec<_> = Self::live_rows(live)
            .into_iter()
            .map(|row| {
                let (id, chosen) = (row.id, live.chosen == Some(row.id));
                let opened = live.open.contains(&id);
                div()
                    .id(("live", id as u64))
                    .debug_selector({
                        let label = row.label.clone();
                        move || format!("live element {label}")
                    })
                    .h(px(24.))
                    .pl(px(8. + row.depth as f32 * 16.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .when(chosen, |d| d.bg(theme::selection()))
                    .when(!chosen, |d| d.hover(|d| d.bg(theme::row_hover())))
                    .child(
                        div()
                            .w(px(14.))
                            .text_color(theme::text_muted())
                            .when(row.children, |d| d.child(if opened { "▾" } else { "▸" }))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |w, _, _, cx| w.toggle_live(id, cx)),
                            ),
                    )
                    .child(div().when(!row.sourced, |d| d.text_color(theme::text_secondary())).child(row.label.clone()))
                    .on_click(cx.listener(move |w, _: &ClickEvent, _, cx| w.choose_live(id, cx)))
            })
            .collect();
        div()
            .track_focus(&self.live_focus)
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(div().id("live-rows").flex_1().min_h_0().overflow_y_scroll().children(rows))
            .children(details.map(|d| {
                div()
                    .id("live-details")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .border_t_1()
                    .border_color(theme::frame())
                    .child(d)
            }))
            .into_any_element()
    }

    /// The chosen element: what it is, where its XAML is (a link), and every property with its value and its source.
    fn live_details(&self, details: &Value, cx: &mut Context<Self>) -> impl IntoElement {
        let text = |key: &str| details[key].as_str().unwrap_or_default().to_string();
        let source = details["source"].clone();
        let place = match source["file"].as_str() {
            Some(file) => format!(
                "{}:{}",
                std::path::Path::new(file).file_name().unwrap_or_default().to_string_lossy(),
                source["line"]
            ),
            None => details["noSource"].as_str().unwrap_or("No XAML place").to_string(),
        };
        let linked = source["file"].is_string();
        let element = details["id"].as_i64().unwrap_or_default();
        let properties = details["properties"].as_array().cloned().unwrap_or_default().into_iter().map(|p| {
            let field = |key: &str| p[key].as_str().unwrap_or_default().to_string();
            let name = field("name");
            let watched = format!("{}.{name}", text("fullType"));
            let breaks = self.is_property_break(&format!("{element}:{watched}"));
            div()
                .h(px(22.))
                .px(px(8.))
                .flex()
                .gap(px(8.))
                .child(
                    div()
                        .id(gpui::SharedString::from(format!("break {name}")))
                        .debug_selector({
                            let name = name.clone();
                            move || format!("live break: {name}")
                        })
                        .w(px(12.))
                        .flex_none()
                        .text_color(if breaks { theme::error() } else { theme::text_muted() })
                        .child(if breaks { "●" } else { "○" })
                        .on_click(cx.listener(move |w, _: &ClickEvent, _, cx| {
                            w.break_when_changes(element, name.clone(), watched.clone(), cx)
                        })),
                )
                .child(div().w(px(160.)).flex_none().child(field("name")))
                .child(div().flex_1().min_w_0().overflow_hidden().child(field("value")))
                .child(div().w(px(80.)).flex_none().text_color(theme::text_secondary()).child(field("source")))
        });
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(8.))
                    .py(px(4.))
                    .flex()
                    .gap(px(8.))
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(text("fullType")))
                    .child(
                        div()
                            .id("live-source")
                            .debug_selector(|| "live tree: source".into())
                            .when(linked, |d| d.text_color(theme::accent()).cursor_pointer())
                            .child(place)
                            .on_click(
                                cx.listener(move |w, _: &ClickEvent, window, cx| {
                                    w.open_live_place(&source, window, cx)
                                }),
                            ),
                    ),
            )
            .children(properties)
    }

    /// Break When Value Changes on an element's property, or off: the inspector watches it and calls where the debugger's
    /// breakpoint for it stops the app at each change, as Passenger's property breakpoints do.
    fn break_when_changes(&mut self, element: i64, property: String, watched: String, cx: &mut Context<Self>) {
        let Some(live) = &self.live else { return };
        let app = live.app.clone();
        let (id, on) = self.toggle_property_break(format!("{element}:{watched}"), cx);
        let asked = cx.background_executor().spawn(async move {
            let fields = if on {
                json!({ "element": element, "on": true, "property": property, "breakpoint": id })
            } else {
                json!({ "on": false, "breakpoint": id })
            };
            app.ask("watch", fields)
        });
        cx.spawn(async move |this, cx| {
            if let Err(why) = asked.await {
                let _ = this.update(cx, |w, cx| {
                    w.said = Some(format!("Break When Value Changes: {why}").into());
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// Test access: the tree as drawn, a row a line indented by its depth, and the chosen element's properties.
    pub fn live_tree(&self) -> Option<(Vec<String>, Option<Value>)> {
        let live = self.live.as_ref()?;
        let rows = Self::live_rows(live).into_iter().map(|r| format!("{}{}", "  ".repeat(r.depth), r.label)).collect();
        Some((rows, live.details.clone()))
    }

    pub fn live_status(&self) -> Option<String> {
        self.live.as_ref().map(|l| l.status.clone())
    }
}
