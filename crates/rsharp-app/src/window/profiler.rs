//! The Profiler (Passenger's `App/Profiling`): Profile runs the chosen configuration's program under the profiler host
//! (`rsharp_lsp::profiler`), its output in the Run console as Run's is; when the run ends its snapshot is shown as Hot
//! Spots, the Call Tree and the Flame Graph, each from the host's JSON.

use super::run::{Event, send};
use super::*;
use crate::state::store;
use futures::channel::mpsc;
use rsharp_core::run::LaunchPlan;
use rsharp_lsp::profiler::{Ended, Profiler, Target};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// A flame graph box smaller than this share of the run is not drawn.
const SMALLEST: f64 = 0.002;
const BOX: f32 = 18.;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileTab {
    HotSpots,
    CallTree,
    FlameGraph,
    Frames,
    Memory,
}

/// The snapshot shown and what is chosen in it.
pub(super) struct Profile {
    name: String,
    snapshot: Value,
    tab: ProfileTab,
    /// The call tree's open nodes, each by its indices from the thread's first.
    open: HashSet<Vec<usize>>,
    /// The flame graph's chosen box, in words.
    chosen: Option<String>,
    /// The run's memory snapshots as the host described them, and the last two compared.
    memory: Vec<Value>,
    comparison: Option<Value>,
}

impl Profile {
    fn new(name: String) -> Self {
        Self {
            name,
            snapshot: Value::Null,
            tab: ProfileTab::HotSpots,
            open: HashSet::new(),
            chosen: None,
            memory: Vec::new(),
            comparison: None,
        }
    }
}

/// One row of the call tree as drawn.
struct Row {
    path: Vec<usize>,
    depth: usize,
    node: Value,
    children: bool,
}

impl MainWindow {
    /// Profile: Run's launch, its program started under the profiler instead of by the console's process.
    pub(super) fn profile(&mut self, cx: &mut Context<Self>) {
        self.profiling = true;
        self.run(cx);
        // A run that did not begin (nothing to run, another in progress) must not leave the next Run profiling.
        if self.run_cancel.is_none() {
            self.profiling = false;
        }
    }

    /// Starts the plan's program under the profiler on a thread of its own: its output becomes Run's events, the run's
    /// snapshot a `Profiled` one, and Stop ends the run (the program with it) once the snapshot is taken.
    pub(super) fn start_profile(
        &mut self,
        plan: &LaunchPlan,
        cancel: Arc<AtomicBool>,
        mut events: mpsc::Sender<Event>,
        cx: &App,
    ) {
        let target = Target {
            program: plan.program.clone(),
            arguments: plan.arguments.clone(),
            working_directory: plan.working_directory.clone(),
            environment: plan.environment.clone(),
        };
        let folder = store(cx).root().join("profiler");
        let inspected = plan.built.as_ref().is_some_and(|built| rsharp_dap::inspector::inspects(built));
        let mut target = target;
        std::thread::spawn(move || {
            // An Avalonia app's frames come from Passenger's inspector in it, which writes them to the trace whether or not
            // anything listens, so it is loaded as for a debugged run, its socket held until the run ends.
            let _inspector = inspected
                .then(|| rsharp_dap::inspector::LiveApp::listen(&target.environment, |_| {}).ok())
                .flatten()
                .map(|(app, variables)| {
                    target.environment.extend(variables);
                    app
                });
            let unstarted = |events: &mut mpsc::Sender<Event>, why: String| {
                send(events, Event::Unstarted(why), &AtomicBool::new(false));
            };
            let profiler = match Profiler::start() {
                Ok(profiler) => profiler,
                Err(failed) => return unstarted(&mut events, format!("The profiler could not start: {}", failed.0)),
            };
            if let Err(why) = std::fs::create_dir_all(&folder)
                .map_err(|e| e.to_string())
                .and_then(|()| profiler.profile(&target, &folder).map(drop))
            {
                return unstarted(&mut events, why);
            }
            send(&mut events, Event::ProfilerReady(profiler.clone()), &AtomicBool::new(false));
            let finished = Arc::new(AtomicBool::new(false));
            let (taken, snapshot) = std::sync::mpsc::channel();
            {
                let (profiler, finished, cancel) = (profiler.clone(), finished.clone(), cancel.clone());
                std::thread::spawn(move || {
                    while !finished.load(Ordering::Acquire) {
                        if cancel.load(Ordering::Acquire) {
                            let result = profiler.stop();
                            let _ = profiler.kill();
                            let _ = taken.send(result);
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                });
            }
            let mut output = events.clone();
            let ended = profiler.run(|text, error| {
                send(&mut output, Event::Output(text.to_string(), error), &cancel);
            });
            finished.store(true, Ordering::Release);
            let always = AtomicBool::new(false);
            match ended {
                Ok(Ended::Exited(taken)) => {
                    send(&mut events, Event::Profiled(Box::new(taken)), &always);
                    send(&mut events, Event::Ended(Some(0)), &always);
                }
                Ok(Ended::Stopped) => {
                    if let Ok(Ok(taken)) = snapshot.recv_timeout(Duration::from_secs(60)) {
                        send(&mut events, Event::Profiled(Box::new(taken)), &always);
                    }
                    send(&mut events, Event::Ended(None), &always);
                }
                Err(why) => unstarted(&mut events, format!("The profiler stopped: {why}")),
            }
        });
    }

    /// A run began (or was attached to): its Profiler opens, empty until the run's snapshot arrives.
    pub(super) fn profiler_ready(&mut self, profiler: Profiler, name: String, cx: &mut Context<Self>) {
        self.profiler = Some(profiler);
        self.profile = Some(Profile::new(name));
        self.dock.show(Tool::Profiler);
        self.save_state(cx);
        cx.notify();
    }

    /// A run's snapshot arrived, and the Profiler shows it.
    pub(super) fn show_profile(&mut self, snapshot: Value, name: String, cx: &mut Context<Self>) {
        match &mut self.profile {
            Some(profile) => profile.snapshot = snapshot,
            None => {
                let mut profile = Profile::new(name);
                profile.snapshot = snapshot;
                self.profile = Some(profile);
            }
        }
        if !self.dock.is_open(Tool::Profiler) {
            self.dock.show(Tool::Profiler);
            self.save_state(cx);
        }
        cx.notify();
    }

    /// Attach Profiler to Running Program: the dialog of the debugger's Attach to Process lists the programs, and the chosen
    /// one is profiled as it runs; Stop in the Profiler ends the profile and leaves the program running.
    pub(super) fn open_attach_profiler(&mut self, cx: &mut Context<Self>) {
        if self.profiler.is_some() {
            self.said = Some("A profile is under way: stop it first".into());
            cx.notify();
            return;
        }
        self.attach_for_profile = true;
        self.open_attach_dialog(cx);
    }

    pub(super) fn attach_profiler(&mut self, process: rsharp_core::debug::DotnetProcess, cx: &mut Context<Self>) {
        let (folder, pid, name) = (store(cx).root().join("profiler"), process.id, process.program());
        self.said = Some(format!("Attaching the profiler to {name}…").into());
        self.profile_task = Some(cx.spawn(async move |this, cx| {
            let started = {
                let name = name.clone();
                cx.background_executor()
                    .spawn(async move {
                        let profiler = Profiler::start().map_err(|f| f.0)?;
                        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
                        profiler.attach(pid, &name, &folder)?;
                        Ok::<_, String>(profiler)
                    })
                    .await
            };
            let profiler = match started {
                Ok(profiler) => profiler,
                Err(why) => {
                    let _ = this.update(cx, |w, cx| {
                        w.said = Some(format!("The profiler could not attach: {why}").into());
                        cx.notify();
                    });
                    return;
                }
            };
            let follow = profiler.clone();
            let _ = this.update(cx, |w, cx| {
                w.said = Some(format!("Profiling {name}").into());
                w.profiler_ready(profiler, name.clone(), cx);
            });
            let ended = cx.background_executor().spawn(async move { follow.run(|_, _| {}) }).await;
            let _ = this.update(cx, |w, cx| {
                w.profiler = None;
                match ended {
                    Ok(Ended::Exited(snapshot)) => w.show_profile(snapshot, name, cx),
                    Ok(Ended::Stopped) => {}
                    Err(why) => w.said = Some(format!("The profiler stopped: {why}").into()),
                }
                cx.notify();
            });
        }));
    }

    /// Stop in the Profiler: a run this window started ends with its program, as Run's Stop does; one attached to ends
    /// with its snapshot taken and the program left running.
    pub(super) fn stop_profile(&mut self, cx: &mut Context<Self>) {
        if self.is_running() {
            return self.stop_run(cx);
        }
        let Some(profiler) = self.profiler.clone() else { return };
        cx.spawn(async move |this, cx| {
            let taken = cx.background_executor().spawn(async move { profiler.stop() }).await;
            let _ = this.update(cx, |w, cx| match taken {
                Ok(snapshot) => w.show_profile(snapshot, String::new(), cx),
                Err(why) => {
                    w.said = Some(format!("The profiler could not stop: {why}").into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Get Memory Snapshot: the running program's heap, and a comparison with the snapshot before it.
    fn take_memory(&mut self, cx: &mut Context<Self>) {
        let Some(profiler) = self.profiler.clone() else { return };
        let before = self.profile.as_ref().and_then(|p| p.memory.last()).and_then(|m| m["number"].as_i64());
        self.said = Some("Taking a memory snapshot…".into());
        cx.spawn(async move |this, cx| {
            let taken = cx
                .background_executor()
                .spawn(async move {
                    let snapshot = profiler.memory()?;
                    let comparison = match (before, snapshot["number"].as_i64()) {
                        (Some(older), Some(newer)) => Some(profiler.compare(older, newer)),
                        _ => None,
                    };
                    Ok::<_, String>((snapshot, comparison))
                })
                .await;
            let _ = this.update(cx, |w, cx| {
                match (taken, &mut w.profile) {
                    (Ok((snapshot, comparison)), Some(profile)) => {
                        profile.memory.push(snapshot);
                        match comparison {
                            Some(Ok(comparison)) => profile.comparison = Some(comparison),
                            // Two snapshots a collection did not tell apart cannot be compared: the host says why.
                            Some(Err(why)) => w.said = Some(why.into()),
                            None => {}
                        }
                        profile.tab = ProfileTab::Memory;
                    }
                    (Err(why), _) => w.said = Some(format!("No memory snapshot: {why}").into()),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn show_profile_tab(&mut self, tab: ProfileTab, cx: &mut Context<Self>) {
        if let Some(profile) = &mut self.profile {
            profile.tab = tab;
            cx.notify();
        }
    }

    /// Test access: the hot spots as "self samples name", heaviest first, and the call tree's first row.
    pub fn profile_hot_spots(&self) -> Vec<String> {
        let Some(profile) = &self.profile else { return Vec::new() };
        profile.snapshot["hotSpots"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|h| format!("{} {}", h["self"], h["name"].as_str().unwrap_or_default()))
            .collect()
    }

    /// Whether the profile shown has its snapshot (the run ended or was stopped).
    pub(super) fn profile_snapshot_ready(&self) -> bool {
        self.profile.as_ref().is_some_and(|p| !p.snapshot.is_null())
    }

    /// The hot spots of the snapshot shown: each method's own time in milliseconds and its share of the run, heaviest first.
    pub(super) fn profile_summary(&self, top: usize) -> Value {
        let Some(profile) = &self.profile else { return Value::Null };
        let (samples, each) = (
            profile.snapshot["samples"].as_f64().unwrap_or(1.).max(1.),
            profile.snapshot["sampleMs"].as_f64().unwrap_or(1.),
        );
        let spots: Vec<Value> = profile.snapshot["hotSpots"]
            .as_array()
            .into_iter()
            .flatten()
            .take(top)
            .map(|h| {
                let own = h["self"].as_f64().unwrap_or(0.);
                serde_json::json!({ "method": h["name"], "ownMs": (own * each).round(), "ownPercent": (own / samples * 1000.).round() / 10.,
                    "totalPercent": (h["total"].as_f64().unwrap_or(0.) / samples * 1000.).round() / 10. })
            })
            .collect();
        serde_json::json!({ "samples": profile.snapshot["samples"], "durationMs": profile.snapshot["durationMs"], "hotSpots": spots })
    }

    pub fn profile_subtitle(&self) -> Option<String> {
        let profile = self.profile.as_ref()?;
        let samples = profile.snapshot["samples"].as_i64().unwrap_or(0);
        let seconds = profile.snapshot["durationMs"].as_f64().unwrap_or(0.) / 1000.;
        Some(format!("{}: {samples} samples in {seconds:.1} s", profile.name))
    }

    pub(super) fn profiler_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(profile) = &self.profile else {
            return empty_text("Profile runs the program under the profiler and shows where its time went.")
                .into_any_element();
        };
        let tab = |id: &'static str, title: &'static str, which: ProfileTab, cx: &mut Context<Self>| {
            let chosen = profile.tab == which;
            div()
                .id(id)
                .debug_selector(move || format!("profiler tab: {title}"))
                .h(px(26.))
                .px(px(10.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .when(chosen, |d| d.bg(theme::tab_selected()))
                .when(!chosen, |d| d.text_color(theme::text_muted()).hover(|d| d.bg(theme::button_hover())))
                .child(title)
                .on_click(cx.listener(move |w, _: &ClickEvent, _, cx| w.show_profile_tab(which, cx)))
        };
        let waiting = profile.snapshot.is_null() && profile.tab != ProfileTab::Memory;
        let body = match profile.tab {
            _ if waiting => {
                empty_text("The snapshot appears when the program ends or Stop is pressed.").into_any_element()
            }
            ProfileTab::HotSpots => Self::hot_spots_view(&profile.snapshot).into_any_element(),
            ProfileTab::CallTree => self.call_tree_view(profile, cx).into_any_element(),
            ProfileTab::FlameGraph => self.flame_graph_view(profile, cx).into_any_element(),
            ProfileTab::Frames => Self::frames_view(&profile.snapshot).into_any_element(),
            ProfileTab::Memory => Self::memory_view(profile).into_any_element(),
        };
        let button =
            |id: &'static str, label: &'static str, cx: &mut Context<Self>, go: fn(&mut Self, &mut Context<Self>)| {
                div()
                    .id(id)
                    .debug_selector(move || format!("profiler: {label}"))
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .hover(|d| d.bg(theme::button_hover()))
                    .child(label)
                    .on_click(cx.listener(move |w, _: &ClickEvent, _, cx| go(w, cx)))
            };
        let live = self.profiler.is_some();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .px(px(8.))
                    .pb(px(4.))
                    .child(tab("hot", "Hot Spots", ProfileTab::HotSpots, cx))
                    .child(tab("tree", "Call Tree", ProfileTab::CallTree, cx))
                    .child(tab("flame", "Flame Graph", ProfileTab::FlameGraph, cx))
                    .child(tab("frames", "Frames", ProfileTab::Frames, cx))
                    .child(tab("memory", "Memory", ProfileTab::Memory, cx))
                    .child(div().flex_1())
                    .children(live.then(|| button("memory-snapshot", "Get Memory Snapshot", cx, Self::take_memory)))
                    .children(live.then(|| button("profile-stop", "Stop", cx, Self::stop_profile))),
            )
            .child(body)
            .into_any_element()
    }

    fn hot_spots_view(snapshot: &Value) -> impl IntoElement {
        let (samples, each) =
            (snapshot["samples"].as_f64().unwrap_or(1.).max(1.), snapshot["sampleMs"].as_f64().unwrap_or(1.));
        let rows: Vec<_> = snapshot["hotSpots"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, h)| {
                let (own, total) = (h["self"].as_f64().unwrap_or(0.), h["total"].as_f64().unwrap_or(0.));
                div()
                    .id(("hot-spot", i))
                    .h(px(ROW_HEIGHT))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .hover(|d| d.bg(theme::row_hover()))
                    .child(div().w(px(70.)).flex_none().child(format!("{:.0} ms", own * each)))
                    .child(
                        div()
                            .w(px(50.))
                            .flex_none()
                            .text_color(theme::text_secondary())
                            .child(format!("{:.1}%", own / samples * 100.)),
                    )
                    .child(
                        div()
                            .w(px(50.))
                            .flex_none()
                            .text_color(theme::text_muted())
                            .child(format!("{:.1}%", total / samples * 100.)),
                    )
                    .child(
                        div().flex_1().min_w_0().truncate().child(h["name"].as_str().unwrap_or_default().to_string()),
                    )
            })
            .collect();
        div()
            .id("profile-hot-spots")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(6.))
            .child(
                div()
                    .h(px(ROW_HEIGHT))
                    .px(px(8.))
                    .flex()
                    .gap(px(8.))
                    .text_color(theme::text_muted())
                    .child(div().w(px(70.)).flex_none().child("Own time"))
                    .child(div().w(px(50.)).flex_none().child("Own"))
                    .child(div().w(px(50.)).flex_none().child("Total"))
                    .child("Method"),
            )
            .children(rows)
    }

    /// The UI frames, the slowest first with the run's median and slowest 5%: layout and drawing work of an Avalonia app.
    fn frames_view(snapshot: &Value) -> impl IntoElement {
        let mut frames: Vec<&Value> = snapshot["uiFrames"].as_array().into_iter().flatten().collect();
        if frames.is_empty() {
            return empty_text(
                "No UI frames: only an Avalonia app run with its inspector (Debug or Profile) writes them.",
            )
            .into_any_element();
        }
        frames.sort_by(|a, b| {
            b["durationMs"].as_f64().partial_cmp(&a["durationMs"].as_f64()).unwrap_or(std::cmp::Ordering::Equal)
        });
        let nth = |fraction: f64| {
            frames[((frames.len() - 1) as f64 * fraction) as usize]["durationMs"].as_f64().unwrap_or(0.)
        };
        let summary =
            format!("{} frames: median {:.1} ms, slowest 5% from {:.1} ms", frames.len(), nth(0.5), nth(0.05));
        let rows: Vec<_> = frames
            .iter()
            .take(300)
            .enumerate()
            .map(|(i, f)| {
                let ms = |key: &str| format!("{:.1} ms", f[key].as_f64().unwrap_or(0.));
                div()
                    .id(("frame", i))
                    .h(px(ROW_HEIGHT))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .hover(|d| d.bg(theme::row_hover()))
                    .child(div().w(px(70.)).flex_none().child(format!("#{}", f["number"])))
                    .child(div().w(px(80.)).flex_none().child(ms("durationMs")))
                    .child(
                        div()
                            .w(px(110.))
                            .flex_none()
                            .text_color(theme::text_secondary())
                            .child(format!("layout {}", ms("layoutMs"))),
                    )
                    .child(
                        div()
                            .w(px(110.))
                            .flex_none()
                            .text_color(theme::text_secondary())
                            .child(format!("record {}", ms("recordMs"))),
                    )
                    .child(div().text_color(theme::text_secondary()).child(format!("draw {}", ms("compositeMs"))))
            })
            .collect();
        div()
            .id("profile-frames")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(6.))
            .child(div().h(px(ROW_HEIGHT)).px(px(8.)).text_color(theme::text_muted()).child(summary))
            .children(rows)
            .into_any_element()
    }

    /// The memory snapshots taken, and the last two compared where there are two, else the one's types by size.
    fn memory_view(profile: &Profile) -> impl IntoElement {
        let Some(last) = profile.memory.last() else {
            return empty_text(
                "Get Memory Snapshot reads the running program's heap; a second one is compared with the first.",
            )
            .into_any_element();
        };
        let mb = |bytes: f64| format!("{:.2} MB", bytes / 1_048_576.);
        let header = profile
            .memory
            .iter()
            .map(|m| {
                format!(
                    "{} {}: {} objects, {}",
                    m["name"].as_str().unwrap_or_default(),
                    m["taken"].as_str().unwrap_or_default(),
                    m["objects"],
                    mb(m["bytes"].as_f64().unwrap_or(0.))
                )
            })
            .collect::<Vec<_>>()
            .join("   ·   ");
        let cell = |text: String, width: f32| div().w(px(width)).flex_none().child(text);
        let rows: Vec<gpui::AnyElement> = match &profile.comparison {
            Some(comparison) => comparison["types"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(i, t)| {
                    let signed = |key: &str| format!("{:+}", t[key].as_i64().unwrap_or(0));
                    div()
                        .id(("memory-type", i))
                        .h(px(ROW_HEIGHT))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .hover(|d| d.bg(theme::row_hover()))
                        .child(cell(signed("objectsDelta"), 80.))
                        .child(cell(signed("bytesDelta"), 100.))
                        .child(cell(format!("new {}", t["newObjects"]), 90.))
                        .child(cell(format!("dead {}", t["deadObjects"]), 90.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(t["name"].as_str().unwrap_or_default().to_string()),
                        )
                        .into_any_element()
                })
                .collect(),
            None => last["types"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(i, t)| {
                    div()
                        .id(("memory-type", i))
                        .h(px(ROW_HEIGHT))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .hover(|d| d.bg(theme::row_hover()))
                        .child(cell(format!("{}", t["count"]), 80.))
                        .child(cell(mb(t["bytes"].as_f64().unwrap_or(0.)), 100.))
                        .child(cell(mb(t["retained"].as_f64().unwrap_or(0.)), 100.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(t["name"].as_str().unwrap_or_default().to_string()),
                        )
                        .into_any_element()
                })
                .collect(),
        };
        let title = profile.comparison.as_ref().map_or("Objects, size and retained size by type".to_string(), |c| {
            format!(
                "{}: {} new, {} dead, {} survived",
                c["title"].as_str().unwrap_or_default(),
                c["newObjects"],
                c["deadObjects"],
                c["survivedObjects"]
            )
        });
        div()
            .id("profile-memory")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(6.))
            .child(div().min_h(px(ROW_HEIGHT)).px(px(8.)).text_color(theme::text_muted()).child(header))
            .child(div().h(px(ROW_HEIGHT)).px(px(8.)).font_weight(FontWeight::SEMIBOLD).child(title))
            .children(rows)
            .into_any_element()
    }

    fn call_rows(profile: &Profile) -> Vec<Row> {
        fn walk(nodes: &[Value], depth: usize, path: &[usize], open: &HashSet<Vec<usize>>, rows: &mut Vec<Row>) {
            for (i, node) in nodes.iter().enumerate() {
                let mut here = path.to_vec();
                here.push(i);
                let children = node["children"].as_array().map_or(&[][..], |c| c);
                rows.push(Row { path: here.clone(), depth, node: node.clone(), children: !children.is_empty() });
                if open.contains(&here) {
                    walk(children, depth + 1, &here, open, rows);
                }
            }
        }
        let mut rows = Vec::new();
        walk(profile.snapshot["threads"].as_array().map_or(&[][..], |t| t), 0, &[], &profile.open, &mut rows);
        rows
    }

    fn call_tree_view(&self, profile: &Profile, cx: &mut Context<Self>) -> impl IntoElement {
        let (samples, each) = (
            profile.snapshot["samples"].as_f64().unwrap_or(1.).max(1.),
            profile.snapshot["sampleMs"].as_f64().unwrap_or(1.),
        );
        let rows: Vec<_> = Self::call_rows(profile)
            .into_iter()
            .enumerate()
            .map(|(i, row)| {
                let (total, own) = (row.node["total"].as_f64().unwrap_or(0.), row.node["self"].as_f64().unwrap_or(0.));
                let opened = profile.open.contains(&row.path);
                let path = row.path.clone();
                div()
                    .id(("call", i))
                    .debug_selector({
                        let name = row.node["name"].as_str().unwrap_or_default().to_string();
                        move || format!("call tree: {name}")
                    })
                    .h(px(ROW_HEIGHT))
                    .pl(px(8. + row.depth as f32 * 16.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .hover(|d| d.bg(theme::row_hover()))
                    .child(
                        div()
                            .w(px(14.))
                            .flex_none()
                            .text_color(theme::text_muted())
                            .when(row.children, |d| d.child(if opened { "▾" } else { "▸" })),
                    )
                    .child(div().w(px(50.)).flex_none().child(format!("{:.1}%", total / samples * 100.)))
                    .child(
                        div()
                            .w(px(70.))
                            .flex_none()
                            .text_color(theme::text_secondary())
                            .child(format!("{:.0} ms", own * each)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(row.node["name"].as_str().unwrap_or_default().to_string()),
                    )
                    .on_click(cx.listener(move |w, _: &ClickEvent, _, cx| {
                        if let Some(profile) = &mut w.profile
                            && !profile.open.remove(&path)
                        {
                            profile.open.insert(path.clone());
                        }
                        cx.notify();
                    }))
            })
            .collect();
        div().id("profile-call-tree").flex_1().min_h_0().overflow_y_scroll().px(px(6.)).children(rows)
    }

    /// The call tree drawn as boxes: a node's width is its share of the run, its children above it as Passenger's flame
    /// graph stacks them (callers at the bottom), the threads side by side along the base.
    fn flame_graph_view(&self, profile: &Profile, cx: &mut Context<Self>) -> impl IntoElement {
        let threads = profile.snapshot["threads"].as_array().cloned().unwrap_or_default();
        let whole: f64 = threads.iter().map(|t| t["total"].as_f64().unwrap_or(0.)).sum::<f64>().max(1.);
        let mut boxes: Vec<(usize, f64, f64, String, f64)> = Vec::new();
        fn place(
            nodes: &[Value],
            depth: usize,
            mut left: f64,
            whole: f64,
            boxes: &mut Vec<(usize, f64, f64, String, f64)>,
        ) {
            for node in nodes {
                let total = node["total"].as_f64().unwrap_or(0.);
                if total / whole < SMALLEST {
                    continue;
                }
                boxes.push((
                    depth,
                    left / whole,
                    total / whole,
                    node["name"].as_str().unwrap_or_default().to_string(),
                    total,
                ));
                place(node["children"].as_array().map_or(&[][..], |c| c), depth + 1, left, whole, boxes);
                left += total;
            }
        }
        place(&threads, 0, 0., whole, &mut boxes);
        let depth = boxes.iter().map(|b| b.0).max().map_or(0, |d| d + 1);
        let each = profile.snapshot["sampleMs"].as_f64().unwrap_or(1.);
        let chosen = profile.chosen.clone();
        let drawn: Vec<_> = boxes
            .into_iter()
            .enumerate()
            .map(|(i, (level, left, width, name, total))| {
                let on = chosen.as_deref() == Some(name.as_str());
                let said = format!("{name}: {:.0} ms, {:.1}% of the run", total * each, width * 100.);
                div()
                    .id(("flame", i))
                    .absolute()
                    .left(gpui::relative(left as f32))
                    .w(gpui::relative(width as f32))
                    .top(px(((depth - 1 - level) as f32) * BOX))
                    .h(px(BOX - 1.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_xs()
                    .px(px(2.))
                    .rounded(px(2.))
                    .bg(if on { theme::accent() } else { theme::selection() })
                    .hover(|d| d.bg(theme::accent()))
                    .child(name)
                    .on_click(cx.listener(move |w, _: &ClickEvent, _, cx| {
                        if let Some(profile) = &mut w.profile {
                            profile.chosen = Some(said.clone());
                            cx.notify();
                        }
                    }))
            })
            .collect();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(8.))
                    .h(px(ROW_HEIGHT))
                    .text_color(theme::text_secondary())
                    .child(profile.chosen.clone().unwrap_or_else(|| "Click a box for its method.".into())),
            )
            .child(
                div()
                    .id("profile-flame")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(8.))
                    .child(div().relative().w_full().h(px(depth as f32 * BOX)).children(drawn)),
            )
    }
}
