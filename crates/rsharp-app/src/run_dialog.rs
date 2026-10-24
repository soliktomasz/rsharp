//! Rider's Run/Debug Configurations dialog (Passenger's `RunConfigurationsDialog`): the configurations down the left
//! under Add, Copy and Remove, the chosen one's fields on the right, OK, Cancel and Apply below. The generated ones
//! change only by copying them, as Rider shows launch profiles; the folder's own change in place. It edits a copy of
//! the folder's own, which OK and Apply hand back with the one chosen.

use crate::assets::icon;
use crate::field::{Changed, TextField};
use crate::find_action::{Choose, Dismiss};
use crate::settings_dialog::{button, checkbox};
use crate::theme;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, SharedString, Window, div,
    prelude::*, px,
};
use rsharp_core::project::EvaluatedProject;
use rsharp_core::run::configuration::{RunConfiguration, project_folder, project_name};
use rsharp_core::run::profile::LaunchProfile;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What the dialog hands back: the folder's own configurations as edited, and the one to keep chosen.
pub enum Ended {
    Applied { own: Vec<RunConfiguration>, chosen: Option<String> },
    Dismissed,
}

/// A row of the list: one of the folder's own, which changes here, or a generated one, which changes only by copying.
struct Row {
    configuration: RunConfiguration,
    generated: bool,
}

/// An environment variable's two fields, in the order the person put them.
struct Variable {
    name: Entity<TextField>,
    value: Entity<TextField>,
}

/// One of the form's text fields, as a test fills it without typing.
pub enum Field {
    Name,
    Program,
    Arguments,
    WorkingDirectory,
    BrowserUrl,
}

/// Which of the row's menus is open: its project, the project's frameworks, or its launch profiles.
// A shortcut: a menu opens under its row rather than as a popup over the dialog; make it a popup once a person
// misses the arrow keys it would take.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
    Project,
    Framework,
    Profile,
}

pub struct RunDialog {
    folder: PathBuf,
    rows: Vec<Row>,
    selected: usize,
    /// The folder's projects, as the window evaluated them: their names, frameworks and launch profiles fill the form.
    projects: Vec<EvaluatedProject>,
    name: Entity<TextField>,
    program: Entity<TextField>,
    arguments: Entity<TextField>,
    working_directory: Entity<TextField>,
    browser_url: Entity<TextField>,
    variables: Vec<Variable>,
    project: PathBuf,
    framework: Option<String>,
    profile: Option<String>,
    build_before_launch: bool,
    launch_browser: bool,
    /// The chosen project's launch profiles, read when the row changes rather than on every drawing of the form.
    profiles: Vec<LaunchProfile>,
    menu: Option<Menu>,
    /// Set while the fields show a row, so what they hold then is no edit of it: a generated row is shown, never edited.
    showing: bool,
    /// Why the last Apply or OK was refused, shown over the buttons.
    problem: Option<SharedString>,
    focus: FocusHandle,
}

impl EventEmitter<Ended> for RunDialog {}

impl Focusable for RunDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn field(text: &str, cx: &mut App) -> Entity<TextField> {
    cx.new(|cx| {
        let mut field = TextField::new("", cx);
        field.set_text(text, cx);
        field
    })
}

impl RunDialog {
    pub fn new(
        folder: &Path,
        own: &[RunConfiguration],
        generated: &[RunConfiguration],
        projects: Vec<EvaluatedProject>,
        chosen: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Self {
        let rows: Vec<Row> = own
            .iter()
            .map(|configuration| Row { configuration: configuration.clone(), generated: false })
            .chain(generated.iter().map(|configuration| Row { configuration: configuration.clone(), generated: true }))
            .collect();
        let selected = chosen.and_then(|name| rows.iter().position(|row| row.configuration.name == name)).unwrap_or(0);
        let mut dialog = Self {
            folder: folder.to_path_buf(),
            rows,
            selected,
            projects,
            name: field("", cx),
            program: field("", cx),
            arguments: field("", cx),
            working_directory: field("", cx),
            browser_url: field("", cx),
            variables: Vec::new(),
            project: PathBuf::new(),
            framework: None,
            profile: None,
            build_before_launch: true,
            launch_browser: false,
            profiles: Vec::new(),
            menu: None,
            showing: false,
            problem: None,
            focus: cx.focus_handle(),
        };
        dialog.subscribe_fields(cx);
        dialog.show(cx);
        dialog
    }

    /// Every row, its name and whether it is one the projects made, for a test.
    pub fn rows(&self) -> Vec<(String, bool)> {
        self.rows.iter().map(|row| (row.configuration.name.clone(), row.generated)).collect()
    }

    /// The configuration the form shows, as it stands.
    pub fn chosen(&self) -> Option<RunConfiguration> {
        self.rows.get(self.selected).map(|row| row.configuration.clone())
    }

    /// Why the last Apply or OK was refused, or nothing.
    pub fn problem(&self) -> String {
        self.problem.clone().unwrap_or_default().to_string()
    }

    /// Chooses a row by its name, as a click does; false when none has it.
    pub fn choose(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        let Some(at) = self.rows.iter().position(|row| row.configuration.name == name) else { return false };
        self.selected = at;
        self.show(cx);
        cx.notify();
        true
    }

    /// Add: a configuration of the folder's own, of the first project a person can run, named "Unnamed" unless that
    /// name is taken.
    pub fn add(&mut self, cx: &mut Context<Self>) {
        let name = RunConfiguration::unique_name("Unnamed", self.rows.iter().map(|row| row.configuration.name.clone()));
        let project = self.projects.iter().find(|project| project.is_runnable()).or(self.projects.first());
        let configuration = RunConfiguration {
            name,
            project: project.map(|project| project.path.clone()).unwrap_or_default(),
            ..RunConfiguration::default()
        };
        let at = self.own_count();
        self.rows.insert(at, Row { configuration, generated: false });
        self.selected = at;
        self.show(cx);
        cx.notify();
    }

    /// Copy: the chosen row — a generated one too — as one of the folder's own, under a name no other has.
    pub fn copy(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(self.selected) else { return };
        let name = RunConfiguration::unique_name(
            &row.configuration.name,
            self.rows.iter().map(|row| row.configuration.name.clone()),
        );
        let copy = Row { configuration: RunConfiguration { name, ..row.configuration.clone() }, generated: false };
        // Beside the row it came from when that is one of the folder's own, else at the end of them.
        let at = if row.generated { self.own_count() } else { self.selected + 1 };
        self.rows.insert(at, copy);
        self.selected = at;
        self.show(cx);
        cx.notify();
    }

    /// Remove: the chosen one, when it is the folder's own; a generated one stays, its project's to change.
    pub fn remove(&mut self, cx: &mut Context<Self>) {
        if self.rows.get(self.selected).is_none_or(|row| row.generated) {
            return;
        }
        self.rows.remove(self.selected);
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        self.show(cx);
        cx.notify();
    }

    /// Add variable: an empty row at the environment's end.
    pub fn add_variable(&mut self, cx: &mut Context<Self>) {
        self.variables.push(Variable { name: field("", cx), value: field("", cx) });
        self.write_back(cx);
        cx.notify();
    }

    /// Remove variable: the row at `index`, keeping the rest in their order.
    pub fn remove_variable(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.variables.len() {
            self.variables.remove(index);
            self.write_back(cx);
            cx.notify();
        }
    }

    /// Sets one of the form's fields as typing into it would; the chosen row is the folder's own, so the form's value
    /// becomes the configuration's. A test fills the dialog with these rather than with keys.
    pub fn set_text(&mut self, field: Field, text: &str, cx: &mut Context<Self>) {
        let entity = match field {
            Field::Name => &self.name,
            Field::Program => &self.program,
            Field::Arguments => &self.arguments,
            Field::WorkingDirectory => &self.working_directory,
            Field::BrowserUrl => &self.browser_url,
        };
        entity.clone().update(cx, |field, cx| field.set_text(text, cx));
        self.write_back(cx);
        cx.notify();
    }

    /// Sets an environment variable's name and value, adding rows until it exists.
    pub fn set_variable(&mut self, index: usize, name: &str, value: &str, cx: &mut Context<Self>) {
        while self.variables.len() <= index {
            self.variables.push(Variable { name: field("", cx), value: field("", cx) });
        }
        let variable = &self.variables[index];
        let (name_field, value_field) = (variable.name.clone(), variable.value.clone());
        name_field.update(cx, |field, cx| field.set_text(name, cx));
        value_field.update(cx, |field, cx| field.set_text(value, cx));
        self.write_back(cx);
        cx.notify();
    }

    /// The folder's own as they stand, for a test: each name with whether it builds first.
    pub fn own(&self) -> Vec<(String, bool)> {
        self.rows
            .iter()
            .filter(|row| !row.generated)
            .map(|row| (row.configuration.name.clone(), row.configuration.build_before_launch))
            .collect()
    }

    fn own_count(&self) -> usize {
        self.rows.iter().take_while(|row| !row.generated).count()
    }

    fn shown_row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    /// Fills the form from the chosen row, the menus' values too.
    fn show(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(self.selected) else {
            self.showing = true;
            for field in [&self.name, &self.program, &self.arguments, &self.working_directory, &self.browser_url] {
                field.clone().update(cx, |field, cx| field.set_text("", cx));
            }
            self.showing = false;
            return;
        };
        let configuration = row.configuration.clone();
        let editable = !row.generated;
        self.showing = true;
        self.name.update(cx, |field, cx| field.set_text(&configuration.name, cx));
        self.program.update(cx, |field, cx| field.set_text(configuration.program.as_deref().unwrap_or_default(), cx));
        self.arguments.update(cx, |field, cx| field.set_text(&configuration.arguments, cx));
        self.working_directory
            .update(cx, |field, cx| field.set_text(configuration.working_directory.as_deref().unwrap_or_default(), cx));
        self.browser_url.update(cx, |field, cx| field.set_text(&configuration.browser_url, cx));
        self.variables = configuration
            .environment
            .iter()
            .map(|(name, value)| Variable { name: field(name, cx), value: field(value, cx) })
            .collect();
        self.project = configuration.project.clone();
        self.profiles = LaunchProfile::read(&project_folder(&self.project));
        self.framework = configuration.framework.clone();
        self.profile = configuration.launch_profile.clone();
        self.build_before_launch = configuration.build_before_launch;
        self.launch_browser = configuration.launch_browser;
        // A generated row is shown but not edited: its fields take no change.
        self.showing = !editable;
        self.menu = None;
    }

    /// Writes the form into the chosen row, when it is the folder's own and the form is not only showing it.
    fn write_back(&mut self, cx: &App) {
        if self.showing {
            return;
        }
        let text = |field: &Entity<TextField>| field.read(cx).text().to_string();
        let trimmed = |field: &Entity<TextField>| {
            let text = text(field).trim().to_string();
            (!text.is_empty()).then_some(text)
        };
        let environment: BTreeMap<String, String> = self
            .variables
            .iter()
            .filter_map(|variable| {
                let name = text(&variable.name).trim().to_string();
                (!name.is_empty()).then(|| (name, text(&variable.value)))
            })
            .collect();
        let Some(row) = self.rows.get_mut(self.selected).filter(|row| !row.generated) else { return };
        row.configuration = RunConfiguration {
            name: text(&self.name).trim().to_string(),
            project: self.project.clone(),
            framework: self.framework.clone(),
            launch_profile: self.profile.clone(),
            program: trimmed(&self.program),
            arguments: text(&self.arguments),
            environment,
            working_directory: trimmed(&self.working_directory),
            build_before_launch: self.build_before_launch,
            launch_browser: self.launch_browser,
            browser_url: text(&self.browser_url).trim().to_string(),
        };
    }

    /// Apply: the list is kept with the folder when every one of its own can be; the dialog stays open either way.
    pub fn apply(&mut self, cx: &mut Context<Self>) -> bool {
        let mut taken: Vec<String> =
            self.rows.iter().filter(|row| row.generated).map(|row| row.configuration.name.clone()).collect();
        for row in self.rows.iter().filter(|row| !row.generated) {
            if let Some(problem) = row.configuration.problem(&taken, &self.folder) {
                self.problem = Some(problem.clone().into());
                if let Some(at) = self.rows.iter().position(|other| other.configuration.name == row.configuration.name)
                {
                    self.selected = at;
                    self.show(cx);
                }
                cx.notify();
                return false;
            }
            taken.push(row.configuration.name.clone());
        }
        self.problem = None;
        let own: Vec<RunConfiguration> =
            self.rows.iter().filter(|row| !row.generated).map(|row| row.configuration.clone()).collect();
        let chosen = self.shown_row().map(|row| row.configuration.name.clone());
        cx.emit(Ended::Applied { own, chosen });
        cx.notify();
        true
    }

    /// OK: the list applied, and the dialog closed.
    pub fn ok(&mut self, cx: &mut Context<Self>) {
        if self.apply(cx) {
            cx.emit(Ended::Dismissed);
        }
    }

    /// The chosen project's launch profiles, as the row's menu offers them.
    fn profiles(&self) -> &[LaunchProfile] {
        &self.profiles
    }

    fn frameworks(&self) -> Vec<String> {
        self.projects
            .iter()
            .find(|project| rsharp_core::paths::same(&project.path, &self.project))
            .map(|project| project.target_frameworks.clone())
            .unwrap_or_default()
    }

    fn choose_project(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.project = path;
        self.profiles = LaunchProfile::read(&project_folder(&self.project));
        // A framework and a launch profile belong to the project they came from.
        self.framework = None;
        self.profile = None;
        self.menu = None;
        self.write_back(cx);
        cx.notify();
    }

    /// Rider's Launch profile: its values fill the form, under the name the configuration already has.
    fn choose_profile(&mut self, name: &str, cx: &mut Context<Self>) {
        let profile = self.profiles().iter().find(|profile| profile.name == name).cloned();
        if let Some(profile) = profile
            && let Some(project) =
                self.projects.iter().find(|project| rsharp_core::paths::same(&project.path, &self.project))
        {
            let filled = RunConfiguration::from_profile(project, &profile);
            self.profile = Some(profile.name.clone());
            self.program.update(cx, |field, cx| field.set_text(filled.program.as_deref().unwrap_or_default(), cx));
            self.arguments.update(cx, |field, cx| field.set_text(&filled.arguments, cx));
            self.working_directory
                .update(cx, |field, cx| field.set_text(filled.working_directory.as_deref().unwrap_or_default(), cx));
            self.browser_url.update(cx, |field, cx| field.set_text(&filled.browser_url, cx));
            self.launch_browser = filled.launch_browser;
            self.variables = filled
                .environment
                .iter()
                .map(|(name, value)| Variable { name: field(name, cx), value: field(value, cx) })
                .collect();
        }
        self.menu = None;
        self.write_back(cx);
        cx.notify();
    }

    fn subscribe_fields(&mut self, cx: &mut Context<Self>) {
        for field in [&self.name, &self.program, &self.arguments, &self.working_directory, &self.browser_url] {
            cx.subscribe(field, |dialog, _, _: &Changed, cx| dialog.write_back(cx)).detach();
        }
    }
}

/// A row of the form: its label, and what fills it.
fn labelled(label: &str, content: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().w(px(150.)).flex_none().text_color(theme::text_secondary()).child(label.to_string()))
        .child(content)
}

fn boxed(field: &Entity<TextField>, width: f32, enabled: bool) -> gpui::Div {
    boxed_named(field, width, enabled, None)
}

/// A field in its box, tagged for a test that clicks it and types.
fn boxed_named(field: &Entity<TextField>, width: f32, enabled: bool, name: Option<&'static str>) -> gpui::Div {
    div()
        .when_some(name, |d, name| d.debug_selector(move || format!("run field: {name}")))
        .w(px(width))
        .h(px(26.))
        .px(px(6.))
        .rounded(px(4.))
        .border_1()
        .border_color(theme::control_border())
        .bg(theme::search_field())
        .when(!enabled, |d| d.opacity(0.5))
        .child(field.clone())
}

fn small_button(id: impl Into<gpui::ElementId>, text: &str) -> gpui::Stateful<gpui::Div> {
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

impl Render for RunDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let generated = self.shown_row().is_some_and(|row| row.generated);
        let editable = !generated;
        let rows: Vec<gpui::AnyElement> = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let name = row.configuration.name.clone();
                let generated = row.generated;
                div()
                    .id(("run-row", i))
                    .debug_selector({
                        let selector = format!("run row: {name}");
                        move || selector.clone()
                    })
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(i == self.selected, |d| d.bg(theme::selection()))
                    .when(i != self.selected, |d| d.hover(|d| d.bg(theme::row_hover())))
                    .child(div().flex_1().truncate().child(name))
                    .children(
                        generated.then(|| div().text_color(theme::text_muted()).text_size(px(11.)).child("generated")),
                    )
                    .on_click(cx.listener(move |dialog, _: &ClickEvent, _, cx| {
                        dialog.selected = i;
                        dialog.show(cx);
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        let menu_rows: Vec<gpui::AnyElement> = match self.menu {
            None => Vec::new(),
            Some(Menu::Project) => self
                .projects
                .iter()
                .enumerate()
                .map(|(i, project)| {
                    let path = project.path.clone();
                    let name = project_name(&project.path);
                    small_button(("project", i), &name)
                        .debug_selector(move || format!("run project: {name}"))
                        .on_click(
                            cx.listener(move |dialog, _: &ClickEvent, _, cx| dialog.choose_project(path.clone(), cx)),
                        )
                        .into_any_element()
                })
                .collect(),
            Some(Menu::Framework) => self
                .frameworks()
                .into_iter()
                .enumerate()
                .map(|(i, framework)| {
                    let text = framework.clone();
                    small_button(("framework", i), &framework)
                        .debug_selector(move || format!("run framework: {text}"))
                        .on_click(cx.listener(move |dialog, _: &ClickEvent, _, cx| {
                            dialog.framework = Some(framework.clone());
                            dialog.menu = None;
                            dialog.write_back(cx);
                            cx.notify();
                        }))
                        .into_any_element()
                })
                .collect(),
            Some(Menu::Profile) => self
                .profiles()
                .iter()
                .enumerate()
                .map(|(i, profile)| {
                    let name = profile.name.clone();
                    small_button(("profile", i), &name)
                        .debug_selector({
                            let selector = format!("run profile: {name}");
                            move || selector.clone()
                        })
                        .on_click(cx.listener(move |dialog, _: &ClickEvent, _, cx| dialog.choose_profile(&name, cx)))
                        .into_any_element()
                })
                .collect(),
        };
        let variables: Vec<gpui::AnyElement> = self
            .variables
            .iter()
            .enumerate()
            .map(|(i, variable)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(boxed(&variable.name, 150., editable))
                    .child(boxed(&variable.value, 210., editable))
                    .child(
                        small_button(("remove-variable", i), "Remove")
                            .when(editable, |d| {
                                d.on_click(cx.listener(move |dialog, _, _, cx| dialog.remove_variable(i, cx)))
                            })
                            .when(!editable, |d| d.opacity(0.5)),
                    )
                    .into_any_element()
            })
            .collect();
        let menu_button = |id: &'static str, text: String, menu: Menu, cx: &mut Context<Self>| {
            let selector = format!("run menu: {id}");
            small_button(id, &text)
                .debug_selector(move || selector.clone())
                .gap(px(6.))
                .child(icon("chevron-down", 12.))
                .when(editable, |d| {
                    d.on_click(cx.listener(move |dialog, _: &ClickEvent, _, cx| {
                        dialog.menu = (dialog.menu != Some(menu)).then_some(menu);
                        cx.notify();
                    }))
                })
                .when(!editable, |d| d.opacity(0.5))
        };
        let project = self.project.clone();
        let project_label =
            if project.as_os_str().is_empty() { "Choose a project".to_string() } else { project_name(&project) };
        let framework_label = self.framework.clone().unwrap_or_else(|| "The project's first".into());
        let profile_label = self.profile.clone().unwrap_or_else(|| "None".into());
        div()
            .key_context("RunDialog")
            .track_focus(&self.focus)
            .w(px(880.))
            .h(px(560.))
            .flex()
            .flex_col()
            .rounded(px(10.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .shadow_lg()
            .on_action(cx.listener(|dialog, _: &Choose, _, cx| dialog.ok(cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(Ended::Dismissed)))
            .child(div().px(px(16.)).py(px(10.)).font_weight(FontWeight::SEMIBOLD).child("Run/Debug Configurations"))
            .child(
                div()
                    .id("run-dialog")
                    .debug_selector(|| "run dialog".to_string())
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .w(px(280.))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .px(px(10.))
                            .border_r_1()
                            .border_color(theme::popup_border())
                            .child(
                                div()
                                    .flex()
                                    .gap(px(6.))
                                    .child(
                                        small_button("run add", "Add")
                                            .debug_selector(|| "run: Add".into())
                                            .on_click(cx.listener(|dialog, _: &ClickEvent, _, cx| dialog.add(cx))),
                                    )
                                    .child(
                                        small_button("run copy", "Copy")
                                            .debug_selector(|| "run: Copy".into())
                                            .on_click(cx.listener(|dialog, _: &ClickEvent, _, cx| dialog.copy(cx))),
                                    )
                                    .child(
                                        small_button("run remove", "Remove")
                                            .debug_selector(|| "run: Remove".into())
                                            .on_click(cx.listener(|dialog, _: &ClickEvent, _, cx| dialog.remove(cx))),
                                    ),
                            )
                            .children(rows),
                    )
                    .child(
                        div()
                            .id("run form")
                            .flex_1()
                            .min_w_0()
                            .overflow_y_scroll()
                            .px(px(16.))
                            .py(px(10.))
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(labelled("Name", boxed_named(&self.name, 300., editable, Some("Name"))))
                            .child(labelled("Project", menu_button("project", project_label, Menu::Project, cx)))
                            .children((self.frameworks().len() > 1).then(|| {
                                labelled(
                                    "Target framework",
                                    menu_button("framework", framework_label, Menu::Framework, cx),
                                )
                            }))
                            .children((!self.profiles().is_empty()).then(|| {
                                labelled("Launch profile", menu_button("profile", profile_label, Menu::Profile, cx))
                            }))
                            .children((!menu_rows.is_empty()).then(|| div().flex().gap(px(6.)).children(menu_rows)))
                            .child(labelled("Exe path", boxed(&self.program, 300., editable)))
                            .child(labelled(
                                "Program arguments",
                                boxed_named(&self.arguments, 400., editable, Some("Program arguments")),
                            ))
                            .child(labelled("Working directory", boxed(&self.working_directory, 400., editable)))
                            .child(div().text_color(theme::text_secondary()).child("Environment variables"))
                            .children(variables)
                            .child(div().flex().child(
                                checkbox("Build the project first", self.build_before_launch).on_click(cx.listener(
                                    |dialog, _: &ClickEvent, _, cx| {
                                        if !dialog.showing {
                                            dialog.build_before_launch = !dialog.build_before_launch;
                                            dialog.write_back(cx);
                                        }
                                        cx.notify();
                                    },
                                )),
                            ))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(12.))
                                    .child(checkbox("Open browser", self.launch_browser).on_click(cx.listener(
                                        |dialog, _: &ClickEvent, _, cx| {
                                            if !dialog.showing {
                                                dialog.launch_browser = !dialog.launch_browser;
                                                dialog.write_back(cx);
                                            }
                                            cx.notify();
                                        },
                                    )))
                                    .child(boxed(&self.browser_url, 260., editable)),
                            )
                            .child(
                                div().flex().child(
                                    small_button("run add variable", "Add variable")
                                        .debug_selector(|| "run: Add variable".into())
                                        .when(editable, |d| {
                                            d.on_click(
                                                cx.listener(|dialog, _: &ClickEvent, _, cx| dialog.add_variable(cx)),
                                            )
                                        }),
                                ),
                            ),
                    ),
            )
            .child(
                div()
                    .h(px(56.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(16.))
                    .border_t_1()
                    .border_color(theme::frame())
                    .child(
                        div()
                            .flex_1()
                            .text_color(theme::error())
                            .debug_selector(|| "run problem".to_string())
                            .children(self.problem.clone()),
                    )
                    .child(
                        button("Cancel", false, true)
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(Ended::Dismissed))),
                    )
                    .child(button("Apply", false, true).on_click(cx.listener(|dialog, _: &ClickEvent, _, cx| {
                        dialog.apply(cx);
                    })))
                    .child(
                        button("OK", true, true).on_click(cx.listener(|dialog, _: &ClickEvent, _, cx| dialog.ok(cx))),
                    ),
            )
    }
}
