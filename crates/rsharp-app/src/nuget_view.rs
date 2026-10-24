//! Rider's NuGet window (Passenger's `NuGetView`), its Packages and Sources tabs. Packages: the projects' packages
//! with nothing typed, else what the chosen feed finds; the chosen package's versions and a row per project to
//! Install, Update or Remove it through `dotnet add package`, whose restore uses the folder's feeds. Sources: the
//! nuget.config files in effect and their feeds, as restore reads them.

use crate::assets::icon;
use crate::field::{Changed, TextField};
use crate::find_action::{SelectNext, SelectPrevious};
use crate::theme;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Subscription, Task, Window,
    anchored, deferred, div, point, prelude::*, px,
};
use rsharp_core::nuget::feed::{FeedSet, PackageFeed};
use rsharp_core::nuget::sources::{Places, SourceSet};
use rsharp_core::nuget::vulnerabilities::{Advisory, Vulnerabilities};
use rsharp_core::nuget::{change, version};

mod sources;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// Rider's name for the chooser's every-feed choice.
pub const ALL_FEEDS: &str = "All Feeds";
const PAUSE: Duration = Duration::from_millis(300);

/// A project's packages as its evaluation lists them, the SDK's implicit ones left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectPackages {
    pub path: PathBuf,
    pub name: String,
    pub packages: Vec<(String, Option<String>)>,
}

impl ProjectPackages {
    fn version_of(&self, id: &str) -> Option<Option<&str>> {
        self.packages.iter().find(|(p, _)| p.eq_ignore_ascii_case(id)).map(|(_, v)| v.as_deref())
    }
}

/// A package as the window lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub version: String,
    pub description: String,
    pub authors: String,
    pub feeds: Vec<String>,
    pub installed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Packages,
    Sources,
    /// What each change's dotnet said, as Rider's NuGet window keeps it.
    Log,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Popup {
    Feeds,
    Versions,
}

/// Files a change wrote, for the window to follow (the tree, the projects, Roslyn).
pub struct Wrote(pub Vec<PathBuf>);

pub struct NuGetView {
    root: Option<PathBuf>,
    places: Places,
    projects: Vec<ProjectPackages>,
    sources: Option<Arc<SourceSet>>,
    search: Entity<TextField>,
    feed: Option<String>,
    prerelease: bool,
    rows: Vec<Row>,
    selected: Option<usize>,
    versions: Result<Vec<String>, String>,
    version: Option<String>,
    status: String,
    log: String,
    busy: bool,
    tab: Tab,
    popup: Option<Popup>,
    revision: u64,
    focus: FocusHandle,
    _search_task: Option<Task<()>>,
    _versions_task: Option<Task<()>>,
    _subscription: Subscription,
    /// The Sources tab: the file whose feeds are shown, the feed form when open, and why a write failed.
    chosen_file: Option<PathBuf>,
    form: Option<sources::FeedForm>,
    sources_problem: Option<String>,
    /// The audit's known vulnerabilities, read once the sources are and a project has a package.
    vulnerabilities: Option<Arc<Vulnerabilities>>,
    auditing: bool,
}

impl EventEmitter<Wrote> for NuGetView {}

impl Focusable for NuGetView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl NuGetView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextField::new("Search", cx));
        let subscription = cx.subscribe(&search, |v, _, _: &Changed, cx| v.query_changed(cx));
        Self {
            root: None,
            places: Places::current(),
            projects: Vec::new(),
            sources: None,
            search,
            feed: None,
            prerelease: false,
            rows: Vec::new(),
            selected: None,
            versions: Ok(Vec::new()),
            version: None,
            status: String::new(),
            log: String::new(),
            busy: false,
            tab: Tab::Packages,
            popup: None,
            revision: 0,
            focus: cx.focus_handle(),
            _search_task: None,
            _versions_task: None,
            _subscription: subscription,
            chosen_file: None,
            form: None,
            sources_problem: None,
            vulnerabilities: None,
            auditing: false,
        }
    }

    /// The user's and the machine's NuGet folders a test reads in place of the person's.
    pub fn set_places(&mut self, places: Places, cx: &mut Context<Self>) {
        self.places = places;
        self.reload_sources(cx);
    }

    /// The folder whose nuget.config files are read: the solution's, or the folder's without one.
    pub fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.root != root {
            self.root = root;
            self.sources = None;
            self.feed = None;
            self.chosen_file = None;
            self.form = None;
            self.vulnerabilities = None;
            self.reload_sources(cx);
        }
    }

    /// The solution's projects and their packages, after each evaluation.
    pub fn set_projects(&mut self, projects: Vec<ProjectPackages>, cx: &mut Context<Self>) {
        if self.projects != projects {
            self.projects = projects;
            self.audit(cx);
            self.list(cx);
        }
    }

    /// Reads the folder's nuget.config files again (after one changed on disk), off the UI thread.
    pub fn reload_sources(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let places = self.places.clone();
        cx.spawn(async move |this, cx| {
            let read = cx.background_executor().spawn(async move { SourceSet::load(&root, &places) }).await;
            let _ = this.update(cx, |v, cx| {
                if v.feed.as_ref().is_some_and(|f| !read.enabled().any(|e| &e.name == f)) {
                    v.feed = None;
                }
                v.sources = Some(Arc::new(read));
                v.vulnerabilities = None;
                v.audit(cx);
                v.list(cx);
            });
        })
        .detach();
    }

    /// Reads the known vulnerabilities from the audit's feeds, as restore's NuGetAudit does, once there are
    /// sources and a project has a package to judge.
    fn audit(&mut self, cx: &mut Context<Self>) {
        let Some(sources) = self.sources.clone() else { return };
        if self.vulnerabilities.is_some() || self.auditing || self.projects.iter().all(|p| p.packages.is_empty()) {
            return;
        }
        self.auditing = true;
        cx.spawn(async move |this, cx| {
            let feeds = sources.audited();
            let read = cx.background_executor().spawn(async move { Vulnerabilities::read(&feeds) }).await;
            let _ = this.update(cx, |v, cx| {
                v.auditing = false;
                // Read for sources since replaced: the new ones are asked again.
                if v.sources.as_ref().is_some_and(|s| Arc::ptr_eq(s, &sources)) {
                    v.vulnerabilities = Some(Arc::new(read));
                    v.default_to_fix();
                    cx.notify();
                } else {
                    v.audit(cx);
                }
            });
        })
        .detach();
    }

    /// The advisories that affect a version some project has of a package, the gravest first, each once.
    pub fn advisories(&self, id: &str) -> Vec<Advisory> {
        let Some(known) = &self.vulnerabilities else { return Vec::new() };
        let mut found: Vec<Advisory> = Vec::new();
        for installed in self.projects.iter().filter_map(|p| p.version_of(id).flatten()) {
            for advisory in known.of(id, installed) {
                if !found.contains(advisory) {
                    found.push(advisory.clone());
                }
            }
        }
        found.sort_by(|a, b| b.severity.cmp(&a.severity));
        found
    }

    /// For a vulnerable installed package, Update puts in the least offered version that fixes it.
    fn default_to_fix(&mut self) {
        let (Some(row), Some(known), Ok(offered)) = (self.selected(), &self.vulnerabilities, &self.versions) else {
            return;
        };
        let vulnerable = self
            .projects
            .iter()
            .filter_map(|p| p.version_of(&row.id).flatten())
            .filter(|v| !known.of(&row.id, v).is_empty())
            .min_by(|a, b| version::compare(a, b));
        if let Some(fix) = vulnerable.and_then(|v| known.fixed_version(&row.id, v, offered)) {
            self.version = Some(fix.clone());
        }
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }
    pub fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.popup = None;
        cx.notify();
    }
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
    pub fn selected(&self) -> Option<&Row> {
        self.selected.and_then(|i| self.rows.get(i))
    }
    pub fn versions(&self) -> Result<&[String], &str> {
        self.versions.as_deref().map_err(String::as_str)
    }
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn log(&self) -> &str {
        &self.log
    }
    pub fn is_busy(&self) -> bool {
        self.busy
    }
    pub fn sources(&self) -> Option<&SourceSet> {
        self.sources.as_deref()
    }
    pub fn search_field(&self) -> &Entity<TextField> {
        &self.search
    }

    /// What the chooser offers: All Feeds, then each enabled feed by name.
    pub fn feed_choices(&self) -> Vec<String> {
        let mut choices = vec![ALL_FEEDS.to_string()];
        choices.extend(self.sources.iter().flat_map(|s| s.enabled()).map(|f| f.name.clone()));
        choices
    }
    pub fn choose_feed(&mut self, name: &str, cx: &mut Context<Self>) {
        self.feed = (name != ALL_FEEDS).then(|| name.to_string());
        self.popup = None;
        self.list(cx);
    }
    pub fn set_prerelease(&mut self, on: bool, cx: &mut Context<Self>) {
        self.prerelease = on;
        self.list(cx);
    }

    fn query_changed(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        let revision = self.revision;
        // The search waits for typing to pause, so each key does not ask every feed.
        self._search_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PAUSE).await;
            let _ = this.update(cx, |v, cx| {
                if v.revision == revision {
                    v.list(cx);
                }
            });
        }));
    }

    /// Lists again: the projects' packages with nothing typed, else what the chosen feeds find.
    fn list(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        let revision = self.revision;
        let query = self.search.read(cx).text().trim().to_string();
        let previous = self.selected().map(|r| r.id.clone());
        if query.is_empty() {
            let mut rows: Vec<Row> = Vec::new();
            for (id, installed) in self.projects.iter().flat_map(|p| &p.packages) {
                let installed = installed.clone().unwrap_or_default();
                match rows.iter_mut().find(|r| r.id.eq_ignore_ascii_case(id)) {
                    Some(row) if version::compare(&installed, &row.version).is_gt() => row.version = installed,
                    Some(_) => {}
                    None => rows.push(Row {
                        id: id.clone(),
                        version: installed,
                        description: String::new(),
                        authors: String::new(),
                        feeds: Vec::new(),
                        installed: true,
                    }),
                }
            }
            rows.sort_by_key(|r| r.id.to_lowercase());
            self.status = match rows.len() {
                0 => "No packages installed".into(),
                n => format!("{n} installed"),
            };
            self.show(rows, previous, cx);
            return;
        }
        let Some(sources) = self.sources.clone() else {
            self.status = "Reading package sources…".into();
            cx.notify();
            return;
        };
        let feeds = self.searched(&sources);
        if feeds.is_empty() {
            self.status = "No enabled package source".into();
            self.show(Vec::new(), previous, cx);
            return;
        }
        let prerelease = self.prerelease;
        self.status = "Searching…".into();
        cx.notify();
        self._search_task = Some(cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move { FeedSet { feeds }.search(&query, prerelease, &|f, id| sources.allows(f, id)) })
                .await;
            let _ = this.update(cx, |v, cx| {
                if v.revision != revision {
                    return;
                }
                let rows = found
                    .packages
                    .into_iter()
                    .map(|p| Row {
                        installed: v.projects.iter().any(|pr| pr.version_of(&p.id).is_some()),
                        id: p.id,
                        version: p.version,
                        description: p.description,
                        authors: p.authors,
                        feeds: p.feeds,
                    })
                    .collect::<Vec<_>>();
                let mut said = vec![match rows.len() {
                    0 => "Nothing found".to_string(),
                    1 => "1 package".to_string(),
                    n => format!("{n} packages"),
                }];
                if found.unmapped > 0 {
                    said.push(format!("{} left out by package source mapping", found.unmapped));
                }
                said.extend(found.failed.iter().map(|(feed, why)| format!("{feed}: {why}")));
                v.status = said.join("; ");
                v.show(rows, previous, cx);
            });
        }));
    }

    fn searched(&self, sources: &SourceSet) -> Vec<PackageFeed> {
        sources
            .enabled()
            .filter(|f| self.feed.as_ref().is_none_or(|chosen| chosen == &f.name))
            .map(PackageFeed::for_source)
            .collect()
    }

    /// Shows rows, the package chosen before chosen again, else the first.
    fn show(&mut self, rows: Vec<Row>, previous: Option<String>, cx: &mut Context<Self>) {
        self.rows = rows;
        let again = previous.and_then(|id| self.rows.iter().position(|r| r.id.eq_ignore_ascii_case(&id)));
        self.selected = None;
        self.select(again.or((!self.rows.is_empty()).then_some(0)), cx);
    }

    pub fn select(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let same = index.is_some() && index == self.selected;
        self.selected = index.filter(|i| *i < self.rows.len());
        self.popup = None;
        cx.notify();
        if same {
            return;
        }
        let Some(row) = self.selected().cloned() else {
            self.versions = Ok(Vec::new());
            self.version = None;
            return;
        };
        self.versions = Ok(Vec::new());
        self.version = Some(row.version.clone()).filter(|v| !v.is_empty());
        let Some(sources) = self.sources.clone() else { return };
        let feeds = self.searched(&sources);
        let prerelease = self.prerelease || version::is_prerelease(&row.version);
        self._versions_task = Some(cx.spawn(async move |this, cx| {
            let id = row.id.clone();
            let found = cx
                .background_executor()
                .spawn(async move { FeedSet { feeds }.versions(&id, prerelease, &|f, id| sources.allows(f, id)) })
                .await;
            let _ = this.update(cx, |v, cx| {
                if v.selected().map(|r| &r.id) != Some(&row.id) {
                    return;
                }
                // The newest a feed offers, unless the person already chose one of them.
                if let Ok(found) = &found
                    && !v.version.as_ref().is_some_and(|chosen| found.contains(chosen) && !row.installed)
                {
                    v.version = found.first().cloned().or(v.version.take());
                }
                v.versions = found;
                v.default_to_fix();
                cx.notify();
            });
        }));
    }

    pub fn choose_version(&mut self, version: &str, cx: &mut Context<Self>) {
        self.version = Some(version.to_string());
        self.popup = None;
        cx.notify();
    }

    /// Installs the chosen version into a project, or moves the project to it.
    pub fn install(&mut self, project: usize, cx: &mut Context<Self>) {
        let (Some(row), Some(version)) = (self.selected().cloned(), self.version.clone()) else { return };
        self.run(project, format!("Installing {} {version}", row.id), cx, move |env, path, cancel, out| {
            change::install(env, path, &row.id, &version, cancel, out)
        });
    }

    pub fn remove(&mut self, project: usize, cx: &mut Context<Self>) {
        let Some(row) = self.selected().cloned() else { return };
        self.run(project, format!("Removing {}", row.id), cx, move |env, path, cancel, out| {
            change::remove(env, path, &row.id, cancel, out)
        });
    }

    /// Runs one change off the UI thread: open copies of the files it may write saved first, the bytes it replaces
    /// kept in Local History, then the open tabs take what it wrote and the window follows.
    fn run(
        &mut self,
        project: usize,
        title: String,
        cx: &mut Context<Self>,
        change: impl FnOnce(
            &rsharp_core::templates::TemplateEnvironment,
            &std::path::Path,
            &AtomicBool,
            &mut dyn FnMut(&str),
        ) -> Result<(), String>
        + Send
        + 'static,
    ) {
        let Some(path) = self.projects.get(project).map(|p| p.path.clone()) else { return };
        if self.busy {
            return;
        }
        let files = change::touched(&path);
        for file in &files {
            if let Some(buffer) = crate::buffer::find(file, cx).filter(|b| b.read(cx).is_modified())
                && let Err(e) = buffer.update(cx, |b, cx| b.save(cx))
            {
                self.status = format!("{} could not be saved: {e}", file.display());
                cx.notify();
                return;
            }
        }
        let before: Vec<_> = files.iter().map(|f| (f.clone(), std::fs::read(f).ok())).collect();
        self.busy = true;
        self.log.push_str(&format!("{title}…\n"));
        self.status = format!("{title}…");
        cx.notify();
        let shared = crate::templates::shared(cx);
        let (sender, mut lines) = futures::channel::mpsc::unbounded::<String>();
        let work = cx.background_executor().spawn(async move {
            let service = crate::templates::service(&shared)?;
            let cancel = AtomicBool::new(false);
            change(service.environment(), &path, &cancel, &mut |s| {
                let _ = sender.unbounded_send(s.to_string());
            })
        });
        cx.spawn(async move |this, cx| {
            use futures::{FutureExt, StreamExt};
            let mut work = work.fuse();
            let result = loop {
                futures::select! {
                    result = work => break result,
                    line = lines.next() => if let Some(line) = line {
                        let _ = this.update(cx, |v, cx| { v.log.push_str(&line); cx.notify(); });
                    },
                }
            };
            let rest: String = lines.collect::<Vec<_>>().await.concat();
            let _ = this.update(cx, |v, cx| {
                v.log.push_str(&rest);
                v.busy = false;
                let mut wrote = Vec::new();
                let mut kept = Vec::new();
                for (file, old) in before {
                    let new = std::fs::read(&file).ok();
                    if new == old {
                        continue;
                    }
                    if let Some(old) = old {
                        kept.push((file.clone(), old, None));
                    }
                    if let Some(new) = new {
                        kept.push((file.clone(), new, None));
                    }
                    if let Some(buffer) = crate::buffer::find(&file, cx) {
                        buffer.update(cx, |b, cx| b.reload(cx));
                    }
                    wrote.push(file);
                }
                crate::history_window::record_all(kept, cx);
                match result {
                    Ok(()) => {
                        v.status = format!("{title}: done");
                        v.log.push_str("Done.\n");
                    }
                    Err(why) => {
                        v.status = format!("{title} failed: {why}");
                        v.log.push_str(&format!("Failed: {why}\n"));
                    }
                }
                if !wrote.is_empty() {
                    cx.emit(Wrote(wrote));
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let at = self.selected.map_or(0, |i| (i as isize + by).clamp(0, self.rows.len() as isize - 1) as usize);
        self.select(Some(at), cx);
    }
}

fn button(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, enabled: bool) -> gpui::Stateful<gpui::Div> {
    let label = label.into();
    let id = id.into();
    div()
        .id(id)
        .h(px(24.))
        .px(px(10.))
        .rounded(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(theme::control_border())
        .when(enabled, |d| d.hover(|d| d.bg(theme::button_hover())).active(|d| d.bg(theme::button_pressed())))
        .when(!enabled, |d| d.text_color(theme::text_muted()))
        .child(label)
}

impl NuGetView {
    fn tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = |which: Tab, title: &'static str, cx: &mut Context<Self>| {
            let chosen = self.tab == which;
            div()
                .id(title)
                .debug_selector(move || format!("nuget tab: {title}"))
                .h(px(26.))
                .px(px(10.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .when(chosen, |d| d.bg(theme::tab_selected()))
                .when(!chosen, |d| d.text_color(theme::text_muted()).hover(|d| d.bg(theme::button_hover())))
                .child(title)
                .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.set_tab(which, cx)))
        };
        div()
            .flex()
            .gap(px(4.))
            .px(px(8.))
            .pb(px(4.))
            .child(tab(Tab::Packages, "Packages", cx))
            .child(tab(Tab::Sources, "Sources", cx))
            .child(tab(Tab::Log, "Log", cx))
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let feed = self.feed.clone().unwrap_or_else(|| ALL_FEEDS.into());
        let prerelease = self.prerelease;
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .pb(px(6.))
            .child(
                div()
                    .id("nuget search")
                    .debug_selector(|| "nuget search".into())
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
                button("nuget feed", feed, true)
                    .debug_selector(|| "nuget feed".into())
                    .gap(px(4.))
                    .child(icon("chevron-down", 16.))
                    .children(self.choices(Popup::Feeds, cx))
                    .on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                        v.popup = if v.popup == Some(Popup::Feeds) { None } else { Some(Popup::Feeds) };
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("nuget prerelease")
                    .debug_selector(|| "nuget prerelease".into())
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .size(px(16.))
                            .rounded(px(3.))
                            .border_1()
                            .border_color(theme::control_border())
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(prerelease, |d| d.bg(theme::accent()).child(icon("check", 14.))),
                    )
                    .child("Prerelease")
                    .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.set_prerelease(!prerelease, cx))),
            )
    }

    fn list_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let chosen = self.selected == Some(i);
                let id = row.id.clone();
                div()
                    .id(("nuget package", i))
                    .debug_selector(move || format!("nuget package: {id}"))
                    .h(px(24.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(chosen, |d| d.bg(theme::selection()))
                    .when(!chosen, |d| d.hover(|d| d.bg(theme::row_hover())))
                    .child(icon("package", 16.))
                    .child(div().flex_1().truncate().child(row.id.clone()))
                    .child(div().flex_none().text_color(theme::text_secondary()).child(row.version.clone()))
                    .when(!self.advisories(&row.id).is_empty(), |d| d.child(icon("warning", 14.)))
                    .when(row.installed && !self.search.read(cx).text().trim().is_empty(), |d| {
                        d.child(icon("check", 14.))
                    })
                    .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.select(Some(i), cx)))
            })
            .collect();
        // What was found, or why a feed failed, under the list it speaks of.
        div()
            .w(px(280.))
            .flex_none()
            .min_h_0()
            .flex()
            .flex_col()
            .child(div().id("nuget packages").flex_1().min_h_0().overflow_y_scroll().px(px(4.)).children(rows))
            .child(
                div()
                    .flex_none()
                    .h(px(24.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .text_color(theme::text_secondary())
                    .debug_selector(|| "nuget status".into())
                    .child(div().truncate().child(self.status.clone())),
            )
    }

    fn details(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(row) = self.selected().cloned() else {
            return div()
                .flex_1()
                .p(px(12.))
                .text_color(theme::text_muted())
                .child("Choose a package")
                .into_any_element();
        };
        let version = self.version.clone();
        let versions_said = match &self.versions {
            Err(why) => Some(why.clone()),
            Ok(found) if found.is_empty() && self.sources.is_some() => Some("Reading versions…".into()),
            _ => None,
        };
        let projects: Vec<_> = self
            .projects
            .iter()
            .enumerate()
            .map(|(i, project)| {
                let has = project.version_of(&row.id);
                let current = has.map(|v| v.unwrap_or("").to_string());
                let vulnerable = has.flatten().is_some_and(|v| {
                    self.vulnerabilities.as_ref().is_some_and(|known| !known.of(&row.id, v).is_empty())
                });
                let can = !self.busy && version.is_some();
                let name = project.name.clone();
                div()
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon("project", 16.))
                    .child(div().flex_1().truncate().child(project.name.clone()))
                    .when(vulnerable, |d| d.child(icon("warning", 14.)))
                    .child(
                        div()
                            .flex_none()
                            .when(vulnerable, |d| d.text_color(theme::warning()))
                            .when(!vulnerable, |d| d.text_color(theme::text_secondary()))
                            .child(current.clone().unwrap_or_else(|| "—".into())),
                    )
                    .map(|d| match &current {
                        None => d.child(
                            button(("nuget install", i), "Install", can)
                                .debug_selector(move || format!("nuget install: {name}"))
                                .when(can, |b| {
                                    b.on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.install(i, cx)))
                                }),
                        ),
                        Some(installed) => {
                            let update = can && version.as_deref() != Some(installed.as_str());
                            let remove = name.clone();
                            d.child(
                                button(("nuget update", i), "Update", update)
                                    .debug_selector(move || format!("nuget update: {name}"))
                                    .when(update, |b| {
                                        b.on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.install(i, cx)))
                                    }),
                            )
                            .child(
                                button(("nuget remove", i), "Remove", !self.busy)
                                    .debug_selector(move || format!("nuget remove: {remove}"))
                                    .when(!self.busy, |b| {
                                        b.on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.remove(i, cx)))
                                    }),
                            )
                        }
                    })
            })
            .collect();
        div()
            .id("nuget details")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(12.))
            .flex()
            .flex_col()
            .gap(px(6.))
            // The package's name and its version on one line, so a short island still shows the projects.
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(row.id.clone()))
                    .child(div().flex_1())
                    .child(
                        button("nuget version", version.clone().unwrap_or_else(|| "—".into()), true)
                            .debug_selector(|| "nuget version".into())
                            .gap(px(4.))
                            .child(icon("chevron-down", 16.))
                            .children(self.choices(Popup::Versions, cx))
                            .on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                                v.popup = if v.popup == Some(Popup::Versions) { None } else { Some(Popup::Versions) };
                                cx.notify();
                            })),
                    )
                    .children(versions_said.map(|s| div().text_color(theme::text_muted()).child(s))),
            )
            .child(div().children(projects))
            .children(self.advisories(&row.id).into_iter().enumerate().map(|(i, advisory)| {
                let url = advisory.url.clone();
                div()
                    .id(("nuget advisory", i))
                    .debug_selector(move || format!("nuget advisory: {i}"))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon("warning", 14.))
                    .child(div().text_color(theme::warning()).child(format!(
                        "{} vulnerability in {}",
                        advisory.severity.name(),
                        advisory.versions
                    )))
                    .child(div().truncate().text_color(theme::accent()).child(advisory.url.clone()))
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.open_url(&url)))
            }))
            .when(!row.description.is_empty(), |d| d.child(div().child(row.description.clone())))
            .when(!row.authors.is_empty(), |d| {
                d.child(div().text_color(theme::text_secondary()).child(format!("by {}", row.authors)))
            })
            .when(!row.feeds.is_empty(), |d| {
                d.child(div().text_color(theme::text_secondary()).child(format!("From {}", row.feeds.join(", "))))
            })
            .into_any_element()
    }

    /// A chooser's list under its button, drawn over everything and kept inside the window.
    fn choices(&self, kind: Popup, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.popup != Some(kind) {
            return None;
        }
        let (choices, chosen) = match kind {
            Popup::Feeds => (self.feed_choices(), self.feed.clone().unwrap_or_else(|| ALL_FEEDS.into())),
            Popup::Versions => (self.versions.clone().unwrap_or_default(), self.version.clone().unwrap_or_default()),
        };
        let items: Vec<_> = choices
            .into_iter()
            .enumerate()
            .map(|(i, choice)| {
                let label = choice.clone();
                div()
                    .id(("nuget choice", i))
                    .debug_selector(move || format!("nuget choice: {label}"))
                    .h(px(24.))
                    .px(px(8.))
                    .mx(px(4.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .when(choice == chosen, |d| d.bg(theme::selection()))
                    .hover(|d| d.bg(theme::row_hover()))
                    .child(choice.clone())
                    .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        match kind {
                            Popup::Feeds => v.choose_feed(&choice, cx),
                            Popup::Versions => v.choose_version(&choice, cx),
                        }
                    }))
            })
            .collect();
        Some(
            deferred(
                anchored()
                    .position_mode(gpui::AnchoredPositionMode::Local)
                    .position(point(px(0.), px(26.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .id("nuget popup")
                            .min_w(px(180.))
                            .max_h(px(300.))
                            .overflow_y_scroll()
                            .py(px(4.))
                            .rounded(px(6.))
                            .bg(theme::popup())
                            .border_1()
                            .border_color(theme::popup_border())
                            .shadow_lg()
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
}

impl Render for NuGetView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.tab {
            Tab::Packages => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(self.toolbar(cx))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .child(self.list_view(cx))
                        .child(div().w(px(1.)).bg(theme::separator()))
                        .child(self.details(cx)),
                )
                .into_any_element(),
            Tab::Sources => self.sources_view(cx),
            Tab::Log => div()
                .id("nuget log")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(8.))
                .font_family("JetBrains Mono")
                .text_size(px(12.))
                .text_color(theme::text_secondary())
                .child(if self.log.is_empty() { "Nothing has run yet".to_string() } else { self.log.clone() })
                .into_any_element(),
        };
        div()
            .key_context("NuGet")
            .track_focus(&self.focus)
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .on_action(cx.listener(|v, _: &SelectNext, _, cx| v.step(1, cx)))
            .on_action(cx.listener(|v, _: &SelectPrevious, _, cx| v.step(-1, cx)))
            .child(self.tabs(cx))
            .child(body)
    }
}
