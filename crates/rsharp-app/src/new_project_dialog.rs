//! Passenger's template choices in Rider's native New Solution / New Project window.

mod render;
pub mod state;

use crate::field::{Changed, TextField};
use crate::templates;
use crate::window::MainWindow;
use futures::{FutureExt, StreamExt};
use gpui::{
    App, AppContext, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable, PathPromptOptions, ScrollHandle,
    SharedString, Subscription, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions, actions, px, size,
};
use rsharp_core::templates::{
    ProjectTarget, TemplateCatalog, TemplateOptionKind,
    cache::CatalogSubscription,
    create::{self, CreatedProjects, CreationProgress},
};
use state::Draft;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

actions!(new_project_dialog, [NextField, PreviousField, ActivateControl]);

pub enum Ended {
    Created(CreatedProjects),
    Cancelled,
}

struct Popup {
    key: String,
    choices: Vec<(String, String)>,
    selected: usize,
}

#[derive(Default)]
struct Progress {
    message: String,
    output: String,
    truncated: bool,
}
impl Progress {
    fn append(&mut self, value: &str) {
        self.output.push_str(value);
        if self.output.len() > 256 * 1024 {
            let mut at = self.output.len() - 256 * 1024;
            while !self.output.is_char_boundary(at) {
                at += 1;
            }
            self.output.drain(..at);
            self.truncated = true;
        }
    }
}

pub struct NewProjectDialog {
    pub(crate) owner: WindowHandle<MainWindow>,
    draft: Draft,
    fields: BTreeMap<String, Entity<TextField>>,
    controls: BTreeMap<String, FocusHandle>,
    order: Vec<String>,
    focus: FocusHandle,
    list_focus: FocusHandle,
    list_scroll: ScrollHandle,
    form_scroll: ScrollHandle,
    form_rows: BTreeMap<String, usize>,
    popup: Option<Popup>,
    problem: Option<String>,
    busy: bool,
    loading: bool,
    closing: bool,
    cancel: Arc<AtomicBool>,
    revision: u64,
    name_revision: u64,
    progress: Arc<Mutex<Progress>>,
    _catalog_subscription: Option<CatalogSubscription>,
    _owner_subscription: Option<Subscription>,
    installed_choice: Option<String>,
    origin: Option<Option<PathBuf>>,
}

impl EventEmitter<Ended> for NewProjectDialog {}
impl Focusable for NewProjectDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Drop for NewProjectDialog {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

pub fn open(
    owner: WindowHandle<MainWindow>,
    directory: PathBuf,
    target: Option<ProjectTarget>,
    cx: &mut App,
) -> Result<WindowHandle<NewProjectDialog>, String> {
    for window in cx.windows() {
        if let Some(handle) = window.downcast::<NewProjectDialog>()
            && handle.read_with(cx, |d, _| d.owner == owner).unwrap_or(false)
        {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            return Ok(handle);
        }
    }
    let title = if target.is_some() { "New Project" } else { "New Solution" };
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(920.), px(640.)), cx))),
            window_min_size: Some(size(px(720.), px(480.))),
            titlebar: Some(TitlebarOptions { title: Some(title.into()), ..Default::default() }),
            ..Default::default()
        },
        |window, cx| {
            let dialog = cx.new(|cx| NewProjectDialog::new(owner, directory, target, cx));
            dialog.update(cx, |d, cx| {
                d.sync_fields(window, cx);
                d.initial_focus(window, cx);
                d.load(cx);
            });
            let weak = dialog.downgrade();
            window.on_window_should_close(cx, move |window, cx| {
                weak.update(cx, |d, cx| {
                    d.cancel(window, cx);
                    false
                })
                .unwrap_or(true)
            });
            dialog
        },
    )
    .map_err(|e| e.to_string())
}

impl NewProjectDialog {
    fn new(
        owner: WindowHandle<MainWindow>,
        directory: PathBuf,
        target: Option<ProjectTarget>,
        cx: &mut Context<NewProjectDialog>,
    ) -> NewProjectDialog {
        NewProjectDialog {
            owner,
            draft: Draft::new(directory, target),
            fields: BTreeMap::new(),
            controls: BTreeMap::new(),
            order: vec![],
            focus: cx.focus_handle(),
            list_focus: cx.focus_handle(),
            list_scroll: ScrollHandle::new(),
            form_scroll: ScrollHandle::new(),
            form_rows: BTreeMap::new(),
            popup: None,
            problem: None,
            busy: false,
            loading: false,
            closing: false,
            cancel: Arc::new(AtomicBool::new(false)),
            revision: 0,
            name_revision: 0,
            progress: Arc::new(Mutex::new(Progress::default())),
            _catalog_subscription: None,
            _owner_subscription: None,
            installed_choice: None,
            origin: None,
        }
    }
}

impl NewProjectDialog {
    pub(crate) fn watch_owner(
        &mut self,
        owner: &Entity<MainWindow>,
        origin: Option<PathBuf>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.origin = Some(origin);
        let handle = window.window_handle().downcast::<Self>().unwrap();
        self._owner_subscription = Some(cx.observe_release(owner, move |d, _, cx| {
            d.cancel.store(true, Ordering::Release);
            d.closing = true;
            cx.defer(move |cx| {
                let _ = handle.update(cx, |d, w, cx| d.cancel(w, cx));
            });
        }));
    }
    pub fn draft(&self) -> &Draft {
        &self.draft
    }
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }
    pub fn is_busy(&self) -> bool {
        self.busy
    }
    pub fn show_catalog(&mut self, catalog: Arc<TemplateCatalog>, authoritative: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.draft.show_catalog(catalog, authoritative);
        self.loading = !authoritative;
        self.problem = None;
        if authoritative
            && let Some(short) = self.installed_choice.take()
            && !self.draft.families.iter().flat_map(|f| &f.types).any(|t| t.short_name() == short && t.pack.is_none())
        {
            self.problem = Some(format!("The installed package did not provide {short}. Retry template discovery."));
        }
        self.names(cx);
        cx.notify();
    }
    fn context(&self) -> PathBuf {
        self.draft
            .target
            .as_ref()
            .and_then(|t| t.solution.parent().map(PathBuf::from))
            .unwrap_or_else(|| self.draft.directory.clone())
    }
    fn load(&mut self, cx: &mut Context<Self>) {
        if self.busy || !cx.global::<templates::Templates>().enabled {
            return;
        }
        self.revision += 1;
        let revision = self.revision;
        self.loading = true;
        self.draft.authoritative = false;
        self.problem = None;
        let shared = templates::shared(cx);
        let folder = self.context();
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        cx.background_executor()
            .spawn(async move {
                let result = templates::service(&shared).and_then(|service| {
                    let folder = existing_parent(folder);
                    if let Some(ready) = service.ready(&folder).or_else(|| service.last_listed(&folder).ok().flatten())
                    {
                        let _ = tx.unbounded_send(Ok((service.clone(), ready, false)));
                    }
                    service.load(&folder).map(|catalog| (service, catalog, true))
                });
                let _ = tx.unbounded_send(result);
            })
            .detach();
        cx.spawn(async move |this, cx| {
            while let Some(result) = rx.next().await {
                if this
                    .update(cx, |d, cx| {
                        if d.revision != revision || d.busy || d.closing {
                            return;
                        }
                        match result {
                            Ok((service, catalog, authoritative)) => {
                                d.show_catalog(catalog, authoritative, cx);
                                if d._catalog_subscription.is_none() {
                                    let (tx, mut rx) = futures::channel::mpsc::channel(1);
                                    let tx = Mutex::new(tx);
                                    d._catalog_subscription = Some(service.subscribe(move || {
                                        let _ = tx.lock().unwrap().try_send(());
                                    }));
                                    cx.spawn(async move |this, cx| {
                                        while rx.next().await.is_some() {
                                            if this.update(cx, |d, cx| d.load(cx)).is_err() {
                                                break;
                                            }
                                        }
                                    })
                                    .detach();
                                }
                            }
                            Err(why) => {
                                d.loading = false;
                                d.problem = Some(why);
                                cx.notify();
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }
    fn names(&mut self, cx: &mut Context<Self>) {
        self.name_revision += 1;
        let revision = self.name_revision;
        let directory = self.draft.directory.clone();
        let bases: Vec<_> = self.draft.families.iter().flat_map(|f| &f.types).map(|t| t.default_name.clone()).collect();
        let work = cx.background_executor().spawn(async move {
            let mut occupied = vec![];
            for base in bases {
                for n in 1.. {
                    let name = format!("{base}{n}");
                    if directory.join(&name).exists() {
                        occupied.push(name);
                    } else {
                        break;
                    }
                }
            }
            occupied
        });
        cx.spawn(async move |this, cx| {
            let occupied = work.await;
            let _ = this.update(cx, |d, cx| {
                if d.name_revision == revision && !d.busy {
                    d.draft.set_occupied_names(occupied);
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn initial_focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        let key = if self.draft.target.is_some() { "Project name" } else { "Solution name" };
        if let Some(field) = self.fields.get(key) {
            field.update(cx, |f, cx| f.select_all(cx));
            window.focus(&field.focus_handle(cx));
        }
    }
    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut values = vec![
            ("Search templates".into(), self.draft.query.clone()),
            ("Solution name".into(), self.draft.solution_text().into()),
            ("Project name".into(), self.draft.project_text().into()),
            ("Location".into(), self.draft.directory.to_string_lossy().into_owned()),
        ];
        if let Some(variant) = self.draft.variant() {
            for option in &variant.options {
                if matches!(
                    option.kind,
                    TemplateOptionKind::Text | TemplateOptionKind::Integer | TemplateOptionKind::Float
                ) {
                    values.push((
                        format!("option: {}", option.name),
                        self.draft.option_value(&option.name).unwrap_or("").to_string(),
                    ));
                }
            }
        }
        for (key, value) in values {
            if !self.fields.contains_key(&key) {
                let field =
                    cx.new(|cx| TextField::new(if key == "Search templates" { "Search templates" } else { "" }, cx));
                let name = key.clone();
                cx.subscribe(&field, move |d, field, _: &Changed, cx| {
                    if !d.busy {
                        d.field_changed(&name, field.read(cx).text(), cx);
                    }
                })
                .detach();
                self.fields.insert(key.clone(), field);
            }
            let field = &self.fields[&key];
            if field.read(cx).text() != value {
                let focused = field.focus_handle(cx).is_focused(window);
                field.update(cx, |f, cx| {
                    f.set_text(&value, cx);
                    if focused {
                        f.select_all(cx);
                    }
                });
            }
        }
    }
    fn field_changed(&mut self, key: &str, value: String, cx: &mut Context<Self>) {
        let changed = match key {
            "Search templates" if self.draft.query != value => {
                self.draft.search(&value);
                self.list_scroll.scroll_to_item(0);
                true
            }
            "Solution name" if self.draft.solution_text() != value => {
                self.draft.set_solution_name(value);
                true
            }
            "Project name" if self.draft.project_text() != value => {
                self.draft.set_project_name(value);
                true
            }
            "Location" if self.draft.directory.to_string_lossy() != value => {
                let before = existing_parent(self.context());
                self.draft.directory = value.into();
                self.names(cx);
                // Typing a path reloads only once it reaches another existing folder.
                if existing_parent(self.context()) != before {
                    self.load(cx);
                }
                true
            }
            key if key.starts_with("option: ") => {
                let key = &key[8..];
                if self.draft.option_value(key) != Some(&value) {
                    self.draft.set_option(key, value);
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if changed {
            self.problem = None;
            self.popup = None;
            cx.notify();
        }
    }
    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            directories: true,
            files: false,
            multiple: false,
            prompt: Some("Select".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picker.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |d, cx| {
                    if !d.busy {
                        d.field_changed("Location", path.to_string_lossy().into_owned(), cx);
                    }
                });
            }
        })
        .detach();
    }
    pub(crate) fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Release);
        self.revision += 1;
        if self.busy {
            self.closing = true;
            self.progress.lock().unwrap().message = "Cancelling…".into();
            cx.notify();
        } else {
            cx.emit(Ended::Cancelled);
            window.remove_window();
            let _ = self.owner.update(cx, |_, w, _| w.activate_window());
        }
    }
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.popup.take().is_some() {
            cx.notify();
            return;
        }
        if self.fields["Search templates"].focus_handle(cx).is_focused(window) && !self.draft.query.is_empty() {
            self.draft.search("");
            cx.notify();
            return;
        }
        self.cancel(window, cx);
    }
    fn traverse(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.popup = None;
        let handles: Vec<_> = self
            .order
            .iter()
            .filter_map(|k| self.fields.get(k).map(|f| f.focus_handle(cx)).or_else(|| self.controls.get(k).cloned()))
            .collect();
        if handles.is_empty() {
            return;
        }
        let at = handles.iter().position(|h| h.is_focused(window));
        let next = if backwards {
            at.unwrap_or(0).checked_sub(1).unwrap_or(handles.len() - 1)
        } else {
            at.map_or(0, |i| (i + 1) % handles.len())
        };
        window.focus(&handles[next]);
        if let Some(row) = self.form_rows.get(&self.order[next]) {
            self.form_scroll.scroll_to_item(*row);
        }
        for field in self.fields.values() {
            if field.focus_handle(cx) == handles[next] {
                field.update(cx, |f, cx| f.select_all(cx));
            }
        }
        cx.notify();
    }
    fn arrow(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if let Some(popup) = &mut self.popup {
            popup.selected = if backwards {
                popup.selected.saturating_sub(1)
            } else {
                (popup.selected + 1).min(popup.choices.len().saturating_sub(1))
            };
        } else if self.list_focus.is_focused(window)
            || self.fields["Search templates"].focus_handle(cx).is_focused(window)
        {
            let at = self.draft.listed.iter().position(|i| Some(*i) == self.draft.family).unwrap_or(0);
            let next =
                if backwards { at.saturating_sub(1) } else { (at + 1).min(self.draft.listed.len().saturating_sub(1)) };
            if let Some(i) = self.draft.listed.get(next) {
                self.draft.choose_family(*i);
                self.list_scroll.scroll_to_item(next);
            }
        } else if let Some(key) = self.focused_control(window).filter(|key| !self.choices(key).is_empty()) {
            // Arrows open a choice's list; buttons and checkboxes answer only Space and Enter.
            self.activate(&key, window, cx);
        }
        cx.notify();
    }
    fn focused_control(&self, window: &Window) -> Option<String> {
        self.controls.iter().find(|(_, h)| h.is_focused(window)).map(|(k, _)| k.clone())
    }
    fn activate(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if key == "Cancel" {
            self.cancel(window, cx);
            return;
        }
        if self.busy {
            return;
        }
        match key {
            "Create" => self.create(window, cx),
            "Install" => self.install(window, cx),
            "Retry" => self.load(cx),
            "Browse" => self.browse(window, cx),
            "Same directory" => {
                self.draft.same_directory = !self.draft.same_directory;
                cx.notify();
            }
            key if key.starts_with("check: ") => {
                let key = &key[7..];
                let value = self.draft.option_value(key) != Some("true");
                self.draft.set_option(key, value.to_string());
                cx.notify();
            }
            _ => {
                let choices = self.choices(key);
                if !choices.is_empty() {
                    let value = self.choice_value(key);
                    let selected = choices.iter().position(|(v, _)| *v == value).unwrap_or(0);
                    self.popup = Some(Popup { key: key.into(), choices, selected });
                    cx.notify();
                }
            }
        }
    }
    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(popup) = self.popup.take() {
            if let Some((value, _)) = popup.choices.get(popup.selected) {
                self.set_choice(&popup.key, value);
                cx.notify();
            }
        } else if let Some(key) = self.focused_control(window).filter(|key| key != "Templates") {
            self.activate(&key, window, cx);
        } else {
            self.create(window, cx);
        }
    }
    fn choices(&self, key: &str) -> Vec<(String, String)> {
        let values = match key {
            "Type" => {
                return self
                    .draft
                    .family
                    .map(|i| {
                        self.draft.families[i]
                            .types
                            .iter()
                            .map(|t| (t.short_name().into(), t.type_name.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
            }
            "Language" => self.draft.chosen().map(|t| t.languages()).unwrap_or_default(),
            "Framework" => {
                self.draft.chosen().map(|t| t.frameworks(self.draft.language.as_deref())).unwrap_or_default()
            }
            "Solution format" => self.draft.catalog.as_ref().map(|c| c.solution_formats.clone()).unwrap_or_default(),
            key => {
                return self
                    .draft
                    .variant()
                    .and_then(|v| v.options.iter().find(|o| o.name == key))
                    .map(|o| o.choices.iter().map(|c| (c.value.clone(), c.label.clone())).collect())
                    .unwrap_or_default();
            }
        };
        values.into_iter().map(|v| (v.clone(), v)).collect()
    }
    fn choice_value(&self, key: &str) -> String {
        match key {
            "Type" => self.draft.chosen().map(|t| t.short_name().into()).unwrap_or_default(),
            "Language" => self.draft.language.clone().unwrap_or_default(),
            "Framework" => self.draft.framework.clone().unwrap_or_default(),
            "Solution format" => self.draft.solution_format.clone(),
            key => self.draft.option_value(key).unwrap_or("").into(),
        }
    }
    fn set_choice(&mut self, key: &str, value: &str) {
        match key {
            "Type" => self.draft.choose_type(value),
            "Language" => self.draft.set_language(value),
            "Framework" => self.draft.set_framework(value),
            "Solution format" => self.draft.solution_format = value.into(),
            key => self.draft.set_option(key, value.into()),
        }
        self.problem = None;
    }
    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if let Some(origin) = &self.origin
            && !self.owner.read_with(cx, |m, _| m.opened_as() == origin.as_deref()).unwrap_or(false)
        {
            self.problem = Some("The originating window changed workspace".into());
            cx.notify();
            return;
        }
        let request = match self.draft.request() {
            Ok(r) => r,
            Err(e) => {
                self.problem = Some(e);
                cx.notify();
                return;
            }
        };
        let problem =
            if request.target.is_none() { create::name_problem(&request.solution_name, "Solution") } else { None }
                .or_else(|| {
                    if !request.template.is_solution {
                        create::name_problem(&request.project_name, "Project")
                    } else {
                        None
                    }
                });
        if let Some(why) = problem {
            self.problem = Some(why);
            cx.notify();
            return;
        }
        if !cx.global::<templates::Templates>().enabled {
            self.problem = Some("Template commands are disabled in this test".into());
            cx.notify();
            return;
        }
        let catalog = self.draft.catalog.clone().unwrap();
        let shared = templates::shared(cx);
        self.begin();
        let cancel = self.cancel.clone();
        let progress = self.progress.clone();
        let work = cx.background_executor().spawn(async move {
            let result = templates::service(&shared).and_then(|s| {
                create::create(s.environment(), &catalog, &request, &cancel, &mut |p| {
                    let mut status = progress.lock().unwrap();
                    match p {
                        CreationProgress::Message(s) => status.message = s,
                        CreationProgress::Output(s) => status.append(&s),
                    }
                })
            });
            if cancel.load(Ordering::Acquire) {
                // A failed run's own error carries any cleanup it could not finish.
                result?.discard()?;
                return Err("Cancelled".into());
            }
            result.map(Some)
        });
        self.finish(work, window, cx);
    }
    fn install(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !cx.global::<templates::Templates>().enabled {
            return;
        }
        let Some(pack) = self.draft.chosen().and_then(|t| t.pack.clone()) else {
            return;
        };
        let package = pack.id;
        self.installed_choice = self.draft.chosen().map(|t| t.short_name().to_string());
        let folder = self.context();
        let shared = templates::shared(cx);
        self.begin();
        self.draft.authoritative = false;
        let cancel = self.cancel.clone();
        let progress = self.progress.clone();
        progress.lock().unwrap().message = format!("Installing {package}…");
        let work = cx.background_executor().spawn(async move {
            templates::service(&shared)?
                .install(&package, &existing_parent(folder), &cancel, &mut |s| progress.lock().unwrap().append(s))?;
            Ok(None)
        });
        self.finish(work, window, cx);
    }
    fn begin(&mut self) {
        self.busy = true;
        self.problem = None;
        self.popup = None;
        self.revision += 1;
        self.cancel = Arc::new(AtomicBool::new(false));
        *self.progress.lock().unwrap() = Progress::default();
    }
    fn finish(
        &mut self,
        work: gpui::Task<Result<Option<CreatedProjects>, String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(self.controls.get("Cancel").unwrap_or(&self.focus));
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let mut work = work.fuse();
            let result = loop {
                futures::select! {
                    result = work => break result,
                    _ = cx.background_executor().timer(Duration::from_millis(50)).fuse() => {
                        let _ = this.update(cx, |_, cx| cx.notify());
                    }
                }
            };
            let mut result = Some(result);
            let accepted = this.update_in(cx, |d, window, cx| {
                d.busy = false;
                match result.take().unwrap() {
                    Ok(Some(created)) if !d.closing => {
                        cx.emit(Ended::Created(created));
                        window.remove_window();
                    }
                    Ok(Some(created)) => {
                        // Cancel may arrive after generation finished but before this UI handoff.
                        d.busy = true;
                        let cleanup = cx.background_executor().spawn(async move {
                            created.discard()?;
                            Err("Cancelled".into())
                        });
                        d.finish(cleanup, window, cx);
                    }
                    Ok(None) if !d.closing => d.load(cx),
                    Ok(None) => d.cancel(window, cx),
                    Err(why) if d.closing && why == "Cancelled" => d.cancel(window, cx),
                    Err(why) => {
                        d.closing = false;
                        d.problem = Some(why);
                        cx.notify();
                    }
                }
            });
            if accepted.is_err()
                && let Some(Ok(Some(created))) = result.take()
            {
                cx.background_executor()
                    .spawn(async move {
                        if let Err(why) = created.discard() {
                            log::error!("{why}");
                        }
                    })
                    .await;
            }
        })
        .detach();
    }
}

fn existing_parent(path: PathBuf) -> PathBuf {
    path.ancestors().find(|p| p.is_dir()).map(PathBuf::from).unwrap_or_else(rsharp_core::programs::home)
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    #[gpui::test]
    fn refresh_does_not_replace_an_active_creation(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| crate::init(std::env::temp_dir().join("rsharp-busy-dialog-state"), cx));
        let (_, main) = cx.add_window_view(MainWindow::new);
        let owner = main.update(|w, _| w.window_handle().downcast::<MainWindow>().unwrap());
        let dialog = main.update(|_, cx| cx.new(|cx| NewProjectDialog::new(owner, std::env::temp_dir(), None, cx)));
        dialog.update(main, |d, cx| {
            let catalog = |sdk: &str| {
                Arc::new(TemplateCatalog {
                    sdk: sdk.into(),
                    families: vec![],
                    solution_formats: vec!["slnx".into()],
                    solution_format: "slnx".into(),
                })
            };
            d.show_catalog(catalog("old"), true, cx);
            d.begin();
            d.show_catalog(catalog("new"), true, cx);
            assert_eq!(d.draft.catalog.as_ref().unwrap().sdk, "old", "refresh replaced the request being created");
            assert!(d.is_busy());
            d.busy = false;
            d.installed_choice = Some("absent-template".into());
            d.show_catalog(catalog("installed"), true, cx);
            assert!(
                d.problem().is_some_and(|p| p.contains("did not provide absent-template")),
                "an install that yielded no matching template claimed success"
            );
        });
    }

    #[gpui::test]
    fn cancel_keeps_the_busy_window_until_its_worker_settles_and_never_emits_created(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::init(std::env::temp_dir().join(format!("rsharp-cancel-dialog-{}", std::process::id())), cx);
            cx.global_mut::<templates::Templates>().enabled = false;
        });
        let (_, main) = cx.add_window_view(MainWindow::new);
        let owner = main.update(|w, _| w.window_handle().downcast::<MainWindow>().unwrap());
        let handle = main.update(|_, cx| open(owner, std::env::temp_dir(), None, cx).unwrap());
        let (tx, rx) = futures::channel::oneshot::channel();
        handle
            .update(main, |d, w, cx| {
                d.begin();
                let work = cx.background_executor().spawn(async move { rx.await.unwrap() });
                d.finish(work, w, cx);
            })
            .unwrap();
        let mut dialog = gpui::VisualTestContext::from_window(handle.into(), main);
        dialog.run_until_parked();
        dialog.simulate_keystrokes("escape");
        dialog.run_until_parked();
        assert!(
            handle.read_with(&dialog, |d, _| d.busy && d.cancel.load(Ordering::Acquire) && d.closing).unwrap(),
            "Cancel abandoned its active worker"
        );
        tx.send(Err("Cancelled".into())).unwrap();
        dialog.run_until_parked();
        assert!(
            handle.read_with(&dialog, |_, _| ()).is_err(),
            "the cancelled worker settled but the dialog stayed open"
        );
    }

    #[gpui::test]
    fn a_cancelled_late_live_result_reports_cleanup_refusal_without_closing(cx: &mut gpui::TestAppContext) {
        let mut environment = match rsharp_core::templates::TemplateEnvironment::discover() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("SKIP live late-result cleanup: {e}");
                return;
            }
        };
        let base = std::env::temp_dir().join(format!("rsharp-late-result-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        environment.cli_home = base.join("home");
        environment.hive = Some(base.join("hive"));
        environment.nuget_packages = Some(base.join("packages"));
        let catalogs = rsharp_core::templates::cache::Catalogs::new(environment.clone());
        let catalog = catalogs.load(&base).unwrap();
        let mut draft = Draft::new(base.clone(), None);
        draft.show_catalog(catalog.clone(), true);
        draft.set_solution_name("Late".into());
        let created =
            create::create(&environment, &catalog, &draft.request().unwrap(), &AtomicBool::new(false), &mut |_| {})
                .unwrap();
        std::fs::write(base.join("Late/unrelated.txt"), "keep").unwrap();
        cx.update(|cx| {
            crate::init(base.join("state"), cx);
            cx.global_mut::<templates::Templates>().enabled = false;
        });
        let (_, main) = cx.add_window_view(MainWindow::new);
        let owner = main.update(|w, _| w.window_handle().downcast::<MainWindow>().unwrap());
        let handle = main.update(|_, cx| open(owner, base.clone(), None, cx).unwrap());
        handle
            .update(main, |d, w, cx| {
                d.begin();
                d.closing = true;
                let work = cx.background_executor().spawn(async move { Ok(Some(created)) });
                d.finish(work, w, cx);
            })
            .unwrap();
        let mut dialog = gpui::VisualTestContext::from_window(handle.into(), main);
        dialog.run_until_parked();
        assert!(
            handle.read_with(&dialog, |d, _| d.problem().is_some_and(|p| p.contains("Cleanup left"))).unwrap_or(false),
            "a late result closed the dialog without showing what cleanup retained"
        );
        assert_eq!(std::fs::read_to_string(base.join("Late/unrelated.txt")).unwrap(), "keep");
        handle.update(&mut dialog, |_, w, _| w.remove_window()).unwrap();
        catalogs.shutdown();
        let _ = std::fs::remove_dir_all(base);
    }
}
