//! The Live Visual Tree (Passenger's LiveTreeController and LiveTreeView): a debugged Avalonia app's windows and their
//! visual trees, asked of Passenger's inspector inside it (`rsharp_dap::inspector`), the chosen element's properties with
//! where each value comes from and its XAML, which a click opens; and each binding the app says failed, written in the
//! Debug console in a compiler message's shape, so its place links to the XAML. Nothing is asked while the app is paused:
//! its UI thread answers nothing then.

use super::*;
use crate::debug_view::Phase;
use rsharp_dap::inspector::{Framework, LiveApp, Told};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

/// The launch's inspector and what it last said.
pub(super) struct Live {
    app: Arc<LiveApp>,
    framework: Framework,
    /// What the window says of the app: waiting for it, its Avalonia, why it cannot be inspected, or that it ended.
    status: String,
    connected: bool,
    roots: Vec<Value>,
    open: HashSet<i64>,
    chosen: Option<i64>,
    details: Option<Value>,
    /// Each failed binding the app told of, once by its id, with how many times it failed (Problems' Bindings tab).
    pub(super) failures: Vec<Value>,
    asking: Option<Task<()>>,
    /// The XAML file whose binding values are shown, and the read under way.
    valued: Option<Entity<crate::buffer::Buffer>>,
    reading: Option<Task<()>>,
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
        let Some(framework) = plan.built.as_ref().and_then(|built| rsharp_dap::inspector::framework(built)) else {
            return BTreeMap::new();
        };
        let (sender, mut told) = futures::channel::mpsc::unbounded();
        let listened =
            LiveApp::listen_to(framework, &plan.environment, plan.project.parent(), &self.live_overlays, move |said| {
                drop(sender.unbounded_send(said))
            });
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
            framework,
            status: format!("Waiting for {} to show its UI…", plan.name),
            connected: false,
            roots: Vec::new(),
            open: HashSet::new(),
            chosen: None,
            details: None,
            failures: Vec::new(),
            asking: None,
            valued: None,
            reading: None,
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
            Told::ValuesChanged => self.refresh_binding_values(cx),
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
        if let Some(live) = &mut self.live {
            match live.failures.iter_mut().find(|f| f["id"] == failure["id"]) {
                Some(known) => *known = failure.clone(),
                None => live.failures.push(failure.clone()),
            }
        }
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

    /// The binding values of the XAML file in the editor, after each binding's line, as Passenger's
    /// BindingValuesController reads them: at a stop through the debugger's evaluation of the inspector's read on the
    /// app's UI thread, and as the app runs from the inspector itself for the element chosen in the Live Visual Tree,
    /// which tells an element's instances apart. None otherwise.
    // shortcut: a binding is placed by its line in the text as typed, not mapped back to the line the running build
    // has, so a line moved since the build shows nothing; map through the build's text once edits during a session need it.
    pub(super) fn refresh_binding_values(&mut self, cx: &mut Context<Self>) {
        let paused = self.debug_phase() == Phase::Paused;
        let buffer = self
            .active_editor()
            .map(|e| e.read(cx).buffer.clone())
            .filter(|b| b.read(cx).path.extension().is_some_and(|e| e == "axaml"));
        let Some(live) = &mut self.live else { return };
        let running = !paused && live.connected && live.chosen.is_some();
        if let Some(old) = live.valued.take().filter(|old| Some(old) != buffer.as_ref()) {
            old.update(cx, |b, cx| b.set_binding_values(Vec::new(), Vec::new(), cx));
        }
        let (Some(buffer), true) = (buffer, paused || running) else {
            live.reading = None;
            return;
        };
        live.valued = Some(buffer.clone());
        let Some((host, _)) = buffer.read(cx).xaml.clone() else { return };
        let (text, path, chosen, app) =
            (buffer.read(cx).document.text().to_string(), buffer.read(cx).path.clone(), live.chosen, live.app.clone());
        live.reading = Some(cx.spawn(async move |this, cx| {
            let bindings =
                cx.background_executor().spawn(async move { host.bindings(&text) }).await.unwrap_or_default();
            // Each element once, by where the app's build recorded it (from one), with the properties its bindings set.
            let mut spots: Vec<Value> = Vec::new();
            for binding in &bindings {
                let (line, column) =
                    (binding["line"].as_i64().unwrap_or(0) + 1, binding["column"].as_i64().unwrap_or(0) + 1);
                match spots.iter_mut().find(|s| s["line"] == line && s["column"] == column) {
                    Some(spot) => spot["properties"].as_array_mut().unwrap().push(binding["property"].clone()),
                    None => spots.push(json!({
                        "line": line, "column": column, "class": binding["class"], "properties": [binding["property"]],
                    })),
                }
            }
            if spots.is_empty() {
                return;
            }
            let read = json!({ "file": path, "elements": spots, "chosen": chosen });
            let asked = this.update(cx, |w, cx| match paused {
                true => {
                    let literal = serde_json::to_string(&read.to_string()).unwrap_or_default();
                    w.evaluate_on_ui_thread(format!("global::Passenger.Inspector.BindingValues.Read({literal})"), cx)
                }
                false => {
                    let read = read.clone();
                    Some(cx.background_executor().spawn(async move {
                        app.ask("values", json!({ "xaml": read })).map(|reply| reply["values"].to_string())
                    }))
                }
            });
            let Ok(Some(asked)) = asked else { return };
            let values =
                match asked.await.and_then(|json| serde_json::from_str::<Value>(&json).map_err(|e| e.to_string())) {
                    Ok(values) => values,
                    Err(why) => json!({ "problem": why }),
                };
            let (shown, cards) = (shown_values(&bindings, &values), binding_cards(&bindings, &values));
            let _ = this.update(cx, |_, cx| buffer.update(cx, |b, cx| b.set_binding_values(shown, cards, cx)));
        }));
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
                    w.refresh_binding_values(cx);
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

    /// Avalonia's renderer debug overlay `name` on every window of the app, or off, and kept for the next launch.
    fn toggle_overlay(&mut self, name: &'static str, cx: &mut Context<Self>) {
        match self.live_overlays.iter().position(|o| o == name) {
            Some(at) => drop(self.live_overlays.remove(at)),
            None => self.live_overlays.push(name.into()),
        }
        // The protocol takes them in Avalonia's own order.
        self.live_overlays.sort_by_key(|o| rsharp_dap::inspector::OVERLAYS.iter().position(|n| n == o));
        if let Some(live) = self.live.as_ref().filter(|l| l.connected) {
            let (app, names) = (live.app.clone(), self.live_overlays.clone());
            cx.background_executor()
                .spawn(async move { drop(app.ask("overlays", json!({ "overlays": names }))) })
                .detach();
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
            .children((live.framework == Framework::Avalonia).then(|| {
                div().flex().gap(px(2.)).children(rsharp_dap::inspector::OVERLAYS.map(|name| {
                    let on = self.live_overlays.iter().any(|o| o == name);
                    div()
                        .id(name)
                        .debug_selector(move || format!("live tree: overlay {name}"))
                        .px(px(6.))
                        .rounded(px(4.))
                        .when(on, |d| d.bg(theme::selection()))
                        .hover(|d| d.bg(theme::button_hover()))
                        .child(name)
                        .on_click(cx.listener(move |w, _: &ClickEvent, _, cx| w.toggle_overlay(name, cx)))
                }))
            }))
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

    /// How many failed bindings Problems' Bindings tab lists.
    pub(super) fn binding_failure_count(&self) -> usize {
        self.live.as_ref().map_or(0, |l| l.failures.len())
    }

    /// Problems' Bindings tab: the bindings that failed in the app, a click opening the XAML that wrote each.
    pub(super) fn bindings_problems_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let failures = self.live.as_ref().map(|l| l.failures.clone()).unwrap_or_default();
        if failures.is_empty() {
            return empty_text("No binding has failed in the app you debug.")
                .track_focus(&self.problems_focus)
                .into_any_element();
        }
        div()
            .id("binding-problems")
            .track_focus(&self.problems_focus)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(6.))
            .children(failures.into_iter().enumerate().map(|(i, failure)| {
                let (message, place, count) = failure_row(&failure);
                let spot = failure["place"].clone();
                self.problem_row(
                    ("binding", i),
                    failure["level"] == "Error",
                    message,
                    place,
                    move |w, window, cx| w.open_live_place(&spot, window, cx),
                    cx,
                )
                .child(
                    div().flex_none().text_color(theme::text_muted()).when(count > 1, |d| d.child(format!("×{count}"))),
                )
            }))
            .into_any_element()
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

/// A failed binding as a Problems row: what it binds and why, where its XAML is, how many times it failed.
pub(super) fn failure_row(failure: &Value) -> (String, String, u64) {
    let text = |key: &str| failure[key].as_str().unwrap_or_default();
    let at = if text("errorPoint").is_empty() || text("errorPoint") == text("path") {
        String::new()
    } else {
        format!(" at {}", text("errorPoint"))
    };
    let message = format!("{}.{} to {}{at}: {}", text("control"), text("property"), text("path"), text("message"));
    let place = match failure["place"]["file"].as_str() {
        Some(file) => format!(
            "{}:{}",
            std::path::Path::new(file).file_name().unwrap_or_default().to_string_lossy(),
            failure["place"]["line"]
        ),
        None => String::new(),
    };
    (message, place, failure["count"].as_u64().unwrap_or(1))
}

/// What a XAML file shows after its lines (Passenger's Shown): each binding's value, or that it failed and where, or of
/// an element made more than once with none chosen how many there are; and why none could be read, after the first.
fn shown_values(bindings: &[Value], values: &Value) -> Vec<(usize, String)> {
    const LONGEST: usize = 60;
    let cut = |text: &str| {
        if text.chars().count() > LONGEST {
            format!("{}…", text.chars().take(LONGEST).collect::<String>())
        } else {
            text.into()
        }
    };
    let line_of = |binding: &Value| binding["endLine"].as_u64().unwrap_or(0) as usize;
    let mut shown = Vec::new();
    if let (Some(problem), Some(first)) = (values["problem"].as_str(), bindings.first()) {
        shown.push((line_of(first), format!("binding values: {}", cut(problem))));
    }
    let elements = values["elements"].as_array().cloned().unwrap_or_default();
    let mut counted = HashSet::new();
    for binding in bindings {
        let (line, column) = (binding["line"].as_i64().unwrap_or(0) + 1, binding["column"].as_i64().unwrap_or(0) + 1);
        let Some(element) = elements.iter().find(|e| e["line"] == line && e["column"] == column) else { continue };
        let property = binding["property"].as_str().unwrap_or_default();
        let bound = element["bindings"].as_array().and_then(|b| b.iter().find(|b| b["property"] == property));
        match bound {
            None if element["instances"].as_i64().unwrap_or(0) > 1 && counted.insert((line, column)) => shown.push((
                line_of(binding),
                format!("{property}: made {} times, choose one in the Live Visual Tree", element["instances"]),
            )),
            None => {}
            Some(bound) => match (bound["failure"].as_str(), bound["value"]["text"].as_str()) {
                (Some(failure), _) => {
                    let at = bound["failedAt"].as_str().map(|s| format!(" at {s}")).unwrap_or_default();
                    shown.push((line_of(binding), format!("{property}: binding failed{at}: {}", cut(failure))))
                }
                (None, Some(text)) => {
                    let since = if bound["unbound"] == true { " (set since: no binding now)" } else { "" };
                    shown.push((line_of(binding), format!("{property}: {}{since}", cut(text))))
                }
                _ => {}
            },
        }
    }
    shown
}

/// The card over a binding's line: each binding there with its path walked from its source, every step's value, and where
/// a failed one stopped, as Passenger's hover shows it.
fn binding_cards(bindings: &[Value], values: &Value) -> Vec<(usize, String)> {
    let elements = values["elements"].as_array().cloned().unwrap_or_default();
    let mut cards: Vec<(usize, String)> = Vec::new();
    for binding in bindings {
        let (line, column) = (binding["line"].as_i64().unwrap_or(0) + 1, binding["column"].as_i64().unwrap_or(0) + 1);
        let property = binding["property"].as_str().unwrap_or_default();
        let Some(bound) = elements
            .iter()
            .find(|e| e["line"] == line && e["column"] == column)
            .and_then(|e| e["bindings"].as_array()?.iter().find(|b| b["property"] == property))
        else {
            continue;
        };
        let value = |v: &Value| v["text"].as_str().map(|t| format!(" = {t}")).unwrap_or_default();
        let mut card = format!("{property}{} ← {}", value(&bound["value"]), bound["path"].as_str().unwrap_or_default());
        for step in bound["steps"].as_array().into_iter().flatten() {
            let name = step["name"].as_str().unwrap_or_default();
            let shown = if step["value"].is_null() { " (not reached)".to_string() } else { value(&step["value"]) };
            card.push_str(&format!("\n  {name}{shown}"));
        }
        if let Some(failure) = bound["failure"].as_str() {
            let at = bound["failedAt"].as_str().map(|s| format!(" at {s}")).unwrap_or_default();
            card.push_str(&format!("\n  failed{at}: {failure}"));
        }
        let at = binding["endLine"].as_u64().unwrap_or(0) as usize;
        match cards.iter_mut().find(|(l, _)| *l == at) {
            Some((_, text)) => text.push_str(&format!("\n\n{card}")),
            None => cards.push((at, card)),
        }
    }
    cards
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn binding_values_show_after_their_lines_failed_ones_saying_why() {
        let bindings = [
            json!({ "line": 7, "column": 9, "property": "Text", "endLine": 7 }),
            json!({ "line": 8, "column": 9, "property": "Text", "endLine": 9 }),
        ];
        let values = json!({ "elements": [
            { "line": 8, "column": 10, "instances": 1, "bindings": [{ "property": "Text", "value": { "text": "\"Hello\"" } }] },
            { "line": 9, "column": 10, "instances": 1, "bindings": [{ "property": "Text", "failure": "Could not find Nmae", "failedAt": "Nmae" }] },
        ] });
        let cards = super::binding_cards(&bindings, &values);
        assert_eq!(cards[0], (7, "Text = \"Hello\" ← ".to_string()));
        assert!(cards[1].1.contains("failed at Nmae"));
        assert_eq!(
            super::shown_values(&bindings, &values),
            [(7, "Text: \"Hello\"".to_string()), (9, "Text: binding failed at Nmae: Could not find Nmae".to_string())]
        );
    }
}
