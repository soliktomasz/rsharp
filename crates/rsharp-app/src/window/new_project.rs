use super::*;
use crate::new_project_dialog::{self, Ended, NewProjectDialog};
use rsharp_core::solution_edits::{self, SolutionEdit};
use rsharp_core::templates::ProjectTarget;

impl MainWindow {
    pub fn new_project_dialog(&self) -> Option<gpui::WindowHandle<NewProjectDialog>> {
        self.creation_dialog
    }
    pub fn show_new_solution(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_creation(None, window, cx);
    }
    pub fn show_new_project(&mut self, into: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(solution) = self.tree.as_ref().map(|t| t.solution()) else {
            self.said = Some("New Project requires an open solution".into());
            return;
        };
        let target = ProjectTarget {
            solution: solution.path.clone(),
            folder: into,
            projects: solution.projects().iter().map(|p| p.name.clone()).collect(),
        };
        self.open_creation(Some(target), window, cx);
    }
    fn open_creation(&mut self, target: Option<ProjectTarget>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dialog) = self.creation_dialog
            && dialog.update(cx, |_, w, _| w.activate_window()).is_ok()
        {
            return;
        }
        if self.creation_opening {
            return;
        }
        self.creation_opening = true;
        self.project_menu = false;
        let last = store(cx).read::<RecentSolutions>().entries.first().map(|e| e.path.clone());
        let selected = target.clone();
        let original = self.opened_as.clone();
        let owner = window.window_handle().downcast::<MainWindow>().unwrap();
        let location = cx.background_executor().spawn(async move {
            if let Some(target) = selected {
                let parent = target.solution.parent().unwrap_or(Path::new("/")).to_path_buf();
                let physical = target.folder.map(|f| parent.join(f.trim_matches('/'))).filter(|p| p.is_dir());
                physical.unwrap_or(parent)
            } else {
                crate::templates::beside_last(last.as_deref())
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let directory = location.await;
            let _ = this.update_in(cx, |m, window, cx| {
                m.creation_opening = false;
                if m.opened_as != original {
                    return;
                }
                match new_project_dialog::open(owner, directory, target.clone(), cx) {
                    Ok(handle) => {
                        m.creation_dialog = Some(handle);
                        let dialog = handle.entity(cx).unwrap();
                        let owned = cx.entity();
                        handle.update(cx, |d, w, cx| d.watch_owner(&owned, original.clone(), w, cx)).unwrap();
                        m._creation_subscription =
                            Some(cx.subscribe_in(&dialog, window, move |m, _, event: &Ended, window, cx| {
                                m.creation_dialog = None;
                                m._creation_subscription = None;
                                match event {
                                    Ended::Cancelled => {
                                        window.focus(&m.root_focus);
                                        window.activate_window();
                                    }
                                    Ended::Created(created) => {
                                        created.keep();
                                        let result = if m.opened_as != original {
                                            Err("The originating window changed workspace".into())
                                        } else {
                                            m.attach_generated(
                                                target.clone(),
                                                created.solution.clone(),
                                                created.projects.clone(),
                                                cx,
                                            )
                                        };
                                        if let Err(why) = result {
                                            let paths = created
                                                .projects
                                                .iter()
                                                .map(|p| p.display().to_string())
                                                .collect::<Vec<_>>()
                                                .join(", ");
                                            m.said = Some(format!("{why}. Created files kept at {paths}").into());
                                            log::error!("{}", m.said.as_ref().unwrap());
                                        } else if !created.warnings.is_empty() {
                                            let warnings = created.warnings.join("\n");
                                            log::warn!("{warnings}");
                                            m.said = Some(warnings.clone().into());
                                            if let Some(solution) = created.solution.clone() {
                                                cx.defer(move |cx| {
                                                    let handle = crate::projects::open_project(&solution, cx);
                                                    let _ = handle.update(cx, |m, _, cx| {
                                                        m.said = Some(warnings.into());
                                                        cx.notify();
                                                    });
                                                });
                                            }
                                        }
                                    }
                                }
                                cx.notify();
                            }));
                    }
                    Err(why) => m.said = Some(why.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Accepts generated paths through the same preserving write used by Add Existing Project.
    pub fn attach_generated(
        &mut self,
        target: Option<ProjectTarget>,
        solution: Option<PathBuf>,
        projects: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if let Some(target) = target {
            if self.tree.as_ref().is_none_or(|t| t.solution().path != target.solution) {
                return Err("The originating window no longer has the target solution open".into());
            }
            self.add_existing(target.folder, projects, true, cx)
        } else {
            let solution = solution.ok_or("The template produced no solution")?;
            let read = rsharp_core::solution::read(&solution).ok_or("The generated solution could not be read")?;
            let existing: HashSet<_> = read.projects().iter().map(|p| p.path.clone()).collect();
            let missing: Vec<_> = projects.iter().filter(|p| !existing.contains(*p)).collect();
            if !missing.is_empty() {
                self.write_edited(
                    &solution,
                    |text| {
                        let mut text = text.to_string();
                        for project in missing {
                            text = solution_edits::edit(
                                &solution,
                                &text,
                                SolutionEdit::AddProject { folder: None, project },
                            )?;
                        }
                        Ok(text)
                    },
                    cx,
                )?;
            }
            cx.defer(move |cx| {
                crate::projects::open_project(&solution, cx);
            });
            Ok(())
        }
    }
    pub(super) fn project_widget_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .left(px(if cfg!(target_os = "macos") { 84. } else { 48. }))
            .top(px(TOOLBAR_HEIGHT))
            .w(px(220.))
            .py(px(6.))
            .rounded(px(6.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .on_mouse_down_out(cx.listener(|m, _, _, cx| {
                m.project_menu = false;
                cx.notify();
            }))
            .children([("New Solution", true), ("Open…", false)].into_iter().map(|(title, new)| {
                div()
                    .id(title)
                    .debug_selector(move || format!("project widget: {title}"))
                    .h(px(28.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .hover(|d| d.bg(theme::selection()))
                    .child(title)
                    .on_click(cx.listener(move |m, _: &ClickEvent, w, cx| {
                        m.project_menu = false;
                        if new {
                            m.show_new_solution(w, cx);
                        } else {
                            m.choose_folder(&OpenFolder, w, cx);
                        }
                    }))
            }))
    }
}
