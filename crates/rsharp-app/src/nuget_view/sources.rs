//! Rider's Sources tab of the NuGet window (Passenger's `NuGetSourcesView` and `FeedDialog`): the nuget.config files in
//! effect, NuGet's order, and for the one chosen its feeds, each enabled or not, added, edited and deleted in that file.
//! A feed disabled is disabled in the file that names it; one enabled is enabled wherever a file had disabled it, as
//! `dotnet nuget enable source` does. The machine's files are read, never written.

use super::{NuGetView, button};
use crate::assets::icon;
use crate::field::TextField;
use crate::theme;
use gpui::{ClickEvent, Context, Entity, Focusable, Window, div, prelude::*, px};
use rsharp_core::nuget::config_edit::{self, FeedEdit};
use rsharp_core::nuget::sources::ConfigFile;
use std::path::{Path, PathBuf};

/// The feed form: a feed of the chosen file being edited (its old name), or a new one.
pub struct FeedForm {
    editing: Option<String>,
    had_password: bool,
    pub name: Entity<TextField>,
    pub url: Entity<TextField>,
    pub user: Entity<TextField>,
    pub password: Entity<TextField>,
    pub enabled: bool,
    pub allow_insecure: bool,
    problem: Option<String>,
}

impl NuGetView {
    /// The file whose feeds are shown: the one chosen while still in effect, else the folder's own, else the first
    /// that can be written.
    pub fn chosen_file(&self) -> Option<&ConfigFile> {
        let files = &self.sources.as_ref()?.files;
        files
            .iter()
            .find(|f| Some(&f.path) == self.chosen_file.as_ref())
            .or_else(|| files.iter().find(|f| !f.read_only()))
            .or(files.first())
    }

    pub fn choose_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.chosen_file = Some(path.to_path_buf());
        self.form = None;
        cx.notify();
    }

    pub fn sources_problem(&self) -> Option<&str> {
        self.sources_problem.as_deref()
    }

    pub fn form(&self) -> Option<&FeedForm> {
        self.form.as_ref()
    }

    /// Opens the form on a feed of the chosen file, or empty for Add Feed. Its password is never shown: left empty,
    /// the file keeps the one it has.
    pub fn open_form(&mut self, feed: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.chosen_file().filter(|f| !f.read_only()) else { return };
        let known = feed.and_then(|n| file.feeds.iter().find(|f| f.name == n)).cloned();
        let enabled = known.as_ref().is_none_or(|k| !file.disabled.iter().any(|d| d.eq_ignore_ascii_case(&k.name)));
        let field = |placeholder: &'static str, text: Option<&str>, masked: bool, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut field = TextField::new(placeholder, cx);
                if masked {
                    field = field.masked();
                }
                field.set_text(text.unwrap_or(""), cx);
                field
            })
        };
        let form = FeedForm {
            name: field("Name", known.as_ref().map(|k| k.name.as_str()), false, cx),
            url: field("URL or folder", known.as_ref().map(|k| k.value.as_str()), false, cx),
            user: field("User", known.as_ref().and_then(|k| k.username.as_deref()), false, cx),
            password: field(
                if known.as_ref().is_some_and(|k| k.has_password) { "Kept as it is" } else { "Password" },
                None,
                true,
                cx,
            ),
            enabled,
            allow_insecure: known.as_ref().is_some_and(|k| k.allow_insecure),
            had_password: known.as_ref().is_some_and(|k| k.has_password),
            editing: known.map(|k| k.name),
            problem: None,
        };
        window.focus(&form.name.focus_handle(cx));
        self.form = Some(form);
        cx.notify();
    }

    pub fn cancel_form(&mut self, cx: &mut Context<Self>) {
        self.form = None;
        cx.notify();
    }

    pub fn set_form_switches(&mut self, enabled: bool, allow_insecure: bool, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.form {
            (form.enabled, form.allow_insecure) = (enabled, allow_insecure);
            cx.notify();
        }
    }

    /// OK: the feed written to the chosen file, unless its name or URL will not do.
    pub fn accept_form(&mut self, cx: &mut Context<Self>) {
        let (Some(form), Some(file)) = (&self.form, self.chosen_file().cloned()) else { return };
        let text = |f: &Entity<TextField>| f.read(cx).text().trim().to_string();
        let (name, url, user, password) =
            (text(&form.name), text(&form.url), text(&form.user), form.password.read(cx).text());
        let why = if name.is_empty() {
            Some("A feed needs a name".to_string())
        } else if url.is_empty() {
            Some("A feed needs a URL or a folder".to_string())
        } else if file.feeds.iter().any(|f| {
            f.name.eq_ignore_ascii_case(&name) && form.editing.as_ref().is_none_or(|e| !e.eq_ignore_ascii_case(&f.name))
        }) {
            Some(format!("{} has a feed named {name} already", file_name(&file.path)))
        } else if !user.is_empty() && password.is_empty() && !form.had_password {
            // NuGet refuses to read a file whose credentials name a user without a password.
            Some("A user needs a password".to_string())
        } else {
            None
        };
        if let Some(why) = why {
            self.form.as_mut().unwrap().problem = Some(why);
            cx.notify();
            return;
        }
        let edit = FeedEdit {
            name,
            url,
            enabled: form.enabled,
            allow_insecure: form.allow_insecure,
            username: (!user.is_empty()).then_some(user),
            password: (!password.is_empty()).then_some(password),
        };
        let editing = form.editing.clone();
        self.form = None;
        self.write(
            vec![(
                file.path,
                Box::new(move |text: &str| match &editing {
                    Some(old) => config_edit::update(text, old, &edit),
                    None => config_edit::add(text, &edit),
                }),
            )],
            cx,
        );
    }

    /// A feed enabled wherever a file disables it, or disabled in the file that names it.
    pub fn set_feed_enabled(&mut self, name: &str, enabled: bool, cx: &mut Context<Self>) {
        let Some(sources) = self.sources.clone() else { return };
        let mut changes: Vec<(PathBuf, Change)> = Vec::new();
        if enabled {
            for file in sources.files.iter().filter(|f| !f.read_only()) {
                if file.disabled.iter().any(|d| d.eq_ignore_ascii_case(name)) {
                    let name = name.to_string();
                    changes
                        .push((file.path.clone(), Box::new(move |t: &str| config_edit::set_enabled(t, &name, true))));
                }
            }
        } else if let Some(file) = self.chosen_file().filter(|f| !f.read_only()) {
            let name = name.to_string();
            changes.push((file.path.clone(), Box::new(move |t: &str| config_edit::set_enabled(t, &name, false))));
        }
        self.write(changes, cx);
    }

    pub fn delete_feed(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(file) = self.chosen_file().filter(|f| !f.read_only()).map(|f| f.path.clone()) else { return };
        let name = name.to_string();
        self.write(vec![(file, Box::new(move |t: &str| config_edit::remove(t, &name)))], cx);
    }

    /// Writes changes off the UI thread: an open copy of each file saved first, the bytes it replaces kept in Local
    /// History, then the sources read again. A file that will not take its change says why under the tab.
    fn write(&mut self, changes: Vec<(PathBuf, Change)>, cx: &mut Context<Self>) {
        if changes.is_empty() {
            return;
        }
        for (path, _) in &changes {
            if let Some(buffer) = crate::buffer::find(path, cx).filter(|b| b.read(cx).is_modified())
                && let Err(e) = buffer.update(cx, |b, cx| b.save(cx))
            {
                self.sources_problem = Some(format!("{} could not be saved: {e}", file_name(path)));
                cx.notify();
                return;
            }
        }
        self.sources_problem = None;
        cx.spawn(async move |this, cx| {
            let written = cx
                .background_executor()
                .spawn(async move {
                    changes
                        .into_iter()
                        .map(|(path, change)| (path.clone(), config_edit::change_file(&path, change)))
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                let mut kept = Vec::new();
                for (path, result) in written {
                    match result {
                        Ok((before, after)) => {
                            kept.extend(before.map(|b| (path.clone(), b, None)));
                            kept.push((path.clone(), after, None));
                            // Edits made since it was saved are kept, and the tab says it is not what was written.
                            match crate::buffer::find(&path, cx) {
                                Some(buffer) if buffer.read(cx).is_modified() => {
                                    v.sources_problem = Some(format!(
                                        "{} has unsaved edits, so its tab was not reloaded with the change",
                                        file_name(&path)
                                    ))
                                }
                                Some(buffer) => buffer.update(cx, |b, cx| b.reload(cx)),
                                None => {}
                            }
                        }
                        Err(why) => v.sources_problem = Some(format!("{}: {why}", file_name(&path))),
                    }
                }
                crate::history_window::record_all(kept, cx);
                v.reload_sources(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn sources_view(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(sources) = self.sources.clone() else {
            return div()
                .p(px(12.))
                .text_color(theme::text_muted())
                .child("Reading package sources…")
                .into_any_element();
        };
        let chosen = self.chosen_file().map(|f| f.path.clone());
        let files: Vec<_> = sources
            .files
            .iter()
            .map(|file| {
                let path = file.path.clone();
                let label = format!("{}  {}", file.place.name(), file_name(&file.path));
                let is_chosen = chosen.as_ref() == Some(&file.path);
                div()
                    .id(gpui::ElementId::Name(format!("nuget file: {}", file.path.display()).into()))
                    .debug_selector(move || format!("nuget file: {label}"))
                    .h(px(24.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(is_chosen, |d| d.bg(theme::selection()))
                    .when(!is_chosen, |d| d.hover(|d| d.bg(theme::row_hover())))
                    .child(icon("file-config", 16.))
                    .child(div().flex_none().child(file.place.name()))
                    .child(div().truncate().text_color(theme::text_secondary()).child(file_name(&file.path)))
                    .when(file.problem.is_some(), |d| d.child(icon("error", 14.)))
                    .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.choose_file(&path, cx)))
            })
            .collect();
        let detail = match self.chosen_file().cloned() {
            None => div()
                .p(px(12.))
                .text_color(theme::text_muted())
                .child("No nuget.config is in effect")
                .into_any_element(),
            Some(file) => self.file_view(&file, cx),
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .child(
                div()
                    .id("nuget files")
                    .w(px(240.))
                    .flex_none()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(4.))
                    .children(files)
                    .when(sources.feeds.iter().any(|f| f.defined_in.is_none()), |d| {
                        d.child(
                            div()
                                .px(px(8.))
                                .pt(px(6.))
                                .text_color(theme::text_muted())
                                .child("nuget.org, as NuGet assumes without a user nuget.config"),
                        )
                    }),
            )
            .child(div().w(px(1.)).bg(theme::separator()))
            .child(detail)
            .into_any_element()
    }

    fn file_view(&self, file: &ConfigFile, cx: &mut Context<Self>) -> gpui::AnyElement {
        let writable = !file.read_only();
        let mut body: Vec<gpui::AnyElement> = vec![
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_color(theme::text_secondary())
                        .child(file.path.display().to_string()),
                )
                .when(!writable, |d| d.child(div().text_color(theme::text_muted()).child("Read-only")))
                .when(writable && self.form.is_none(), |d| {
                    d.child(
                        button("nuget add feed", "Add Feed", true)
                            .debug_selector(|| "nuget add feed".into())
                            .on_click(cx.listener(|v, _: &ClickEvent, w, cx| v.open_form(None, w, cx))),
                    )
                })
                .into_any_element(),
        ];
        if let Some(problem) = &file.problem {
            body.push(div().text_color(theme::error()).child(problem.clone()).into_any_element());
        }
        if let Some(problem) = &self.sources_problem {
            body.push(
                div()
                    .text_color(theme::error())
                    .debug_selector(|| "nuget sources problem".into())
                    .child(problem.clone())
                    .into_any_element(),
            );
        }
        if let Some(form) = &self.form {
            body.push(self.form_view(form, cx));
        }
        let in_effect = |name: &str| {
            self.sources
                .as_ref()
                .is_some_and(|s| s.feeds.iter().any(|f| f.name.eq_ignore_ascii_case(name) && f.enabled))
        };
        for (i, feed) in file.feeds.iter().enumerate() {
            let enabled = in_effect(&feed.name);
            let (name, toggle, edit, delete) =
                (feed.name.clone(), feed.name.clone(), feed.name.clone(), feed.name.clone());
            body.push(
                div()
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .id(("nuget feed enabled", i))
                            .debug_selector(move || format!("nuget feed enabled: {name}"))
                            .size(px(16.))
                            .rounded(px(3.))
                            .border_1()
                            .border_color(theme::control_border())
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(enabled, |d| d.bg(theme::accent()).child(icon("check", 14.)))
                            .when(writable, |d| {
                                d.on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                                    v.set_feed_enabled(&toggle, !enabled, cx)
                                }))
                            }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .when(!enabled, |d| d.text_color(theme::text_muted()))
                            .child(feed.name.clone()),
                    )
                    .child(div().flex_1().truncate().text_color(theme::text_secondary()).child(feed.value.clone()))
                    .when(feed.username.is_some(), |d| d.child(icon("locked", 14.)))
                    .when(writable, |d| {
                        d.child(
                            button(("nuget edit feed", i), "Edit", true)
                                .debug_selector({
                                    let name = edit.clone();
                                    move || format!("nuget edit feed: {name}")
                                })
                                .on_click(cx.listener(move |v, _: &ClickEvent, w, cx| v.open_form(Some(&edit), w, cx))),
                        )
                        .child(
                            button(("nuget delete feed", i), "Delete", true)
                                .debug_selector({
                                    let name = delete.clone();
                                    move || format!("nuget delete feed: {name}")
                                })
                                .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.delete_feed(&delete, cx))),
                        )
                    })
                    .into_any_element(),
            );
        }
        if file.feeds.is_empty() && self.form.is_none() {
            body.push(div().text_color(theme::text_muted()).child("This file names no feeds").into_any_element());
        }
        div()
            .id("nuget file")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(12.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .children(body)
            .into_any_element()
    }

    fn form_view(&self, form: &FeedForm, cx: &mut Context<Self>) -> gpui::AnyElement {
        let row = |label: &'static str, field: &Entity<TextField>| {
            div().flex_1().min_w_0().flex().items_center().gap(px(8.)).child(div().flex_none().child(label)).child(
                div()
                    .debug_selector(move || format!("nuget form: {label}"))
                    .flex_1()
                    .h(px(26.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(theme::control_border())
                    .child(field.clone()),
            )
        };
        let check =
            |id: &'static str, label: &'static str, on: bool, cx: &mut Context<Self>, flip: fn(&mut FeedForm)| {
                div()
                    .id(id)
                    .debug_selector(move || format!("nuget form: {label}"))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .size(px(16.))
                            .rounded(px(3.))
                            .border_1()
                            .border_color(theme::control_border())
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(on, |d| d.bg(theme::accent()).child(icon("check", 14.))),
                    )
                    .child(label)
                    .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                        if let Some(form) = &mut v.form {
                            flip(form);
                            cx.notify();
                        }
                    }))
            };
        div()
            .p(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(theme::control_border())
            .flex()
            .flex_col()
            .gap(px(4.))
            // Three rows, so a short bottom island still shows OK.
            .child(div().flex().gap(px(12.)).child(row("Name", &form.name)).child(row("URL", &form.url)))
            .child(div().flex().gap(px(12.)).child(row("User", &form.user)).child(row("Password", &form.password)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(check("nuget form enabled", "Enabled", form.enabled, cx, |f| f.enabled = !f.enabled))
                    .child(check("nuget form insecure", "Allow insecure connections", form.allow_insecure, cx, |f| {
                        f.allow_insecure = !f.allow_insecure
                    }))
                    .child(div().flex_1().truncate().text_color(theme::error()).children(form.problem.clone()))
                    .child(
                        button("nuget form cancel", "Cancel", true)
                            .debug_selector(|| "nuget form cancel".into())
                            .on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.cancel_form(cx))),
                    )
                    .child(
                        button("nuget form ok", if form.editing.is_some() { "Save" } else { "Add" }, true)
                            .debug_selector(|| "nuget form ok".into())
                            .on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.accept_form(cx))),
                    ),
            )
            .into_any_element()
    }
}

type Change = Box<dyn FnOnce(&str) -> Result<String, String> + Send>;

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}
