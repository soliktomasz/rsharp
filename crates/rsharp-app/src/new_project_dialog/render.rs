use super::*;
use crate::assets::icon;
use crate::find_action::{Choose, Dismiss, SelectNext, SelectPrevious};
use crate::settings_dialog::{button, checkbox};
use crate::theme;
use gpui::{AnyElement, ClickEvent, Div, FontWeight, Render, Stateful, anchored, deferred, div, point, prelude::*};

impl NewProjectDialog {
    fn control(&mut self, key: &str, enabled: bool, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let focus = self.controls.entry(key.into()).or_insert_with(|| cx.focus_handle()).clone();
        if enabled {
            self.order.push(key.into());
        }
        div()
            .id(SharedString::from(key.to_string()))
            .key_context("ProjectControl")
            .track_focus(&focus)
            .when(focus.is_focused(window), |d| d.border_1().rounded(px(4.)).border_color(theme::accent()))
            .when(!enabled, |d| d.opacity(0.5))
            .on_mouse_down(gpui::MouseButton::Left, move |_, window, _| {
                if enabled {
                    window.focus(&focus);
                }
            })
    }
    fn field(&mut self, key: &str, enabled: bool, window: &Window, cx: &App) -> AnyElement {
        if enabled {
            self.order.push(key.into());
        }
        let field = self.fields[key].clone();
        let focus = field.focus_handle(cx);
        let label = key.to_string();
        div()
            .id(SharedString::from(format!("field-{key}")))
            .debug_selector(move || format!("field: {label}"))
            .h(px(28.))
            .w_full()
            .min_w_0()
            .px(px(8.))
            .flex()
            .items_center()
            .rounded(px(4.))
            .bg(theme::frame())
            .border_1()
            .border_color(theme::control_border())
            .when(focus.is_focused(window), |d| d.border_color(theme::accent()))
            .when(enabled, |d| d.child(field).on_mouse_down(gpui::MouseButton::Left, move |_, w, _| w.focus(&focus)))
            .when(!enabled, |d| d.text_color(theme::text_secondary()).child(self.fields[key].read(cx).text()))
            .into_any_element()
    }
    fn row(label: impl Into<SharedString>, control: AnyElement) -> AnyElement {
        div()
            .flex()
            .items_start()
            .gap(px(12.))
            .child(div().w(px(112.)).flex_none().pt(px(5.)).child(label.into()))
            .child(div().flex_1().min_w_0().child(control))
            .into_any_element()
    }
    fn choice(&mut self, key: &str, enabled: bool, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let value = self.choice_value(key);
        let choices = self.choices(key);
        let label = choices.iter().find(|(v, _)| *v == value).map(|(_, l)| l.clone()).unwrap_or(value);
        let action = key.to_string();
        let selector = key.to_string();
        let mut control = self
            .control(key, enabled, window, cx)
            .debug_selector(move || format!("choice: {selector}"))
            .relative()
            .w_full()
            .min_w_0()
            .h(px(28.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(4.))
            .border_1()
            .border_color(theme::control_border())
            .bg(theme::frame())
            .when(self.controls[key].is_focused(window), |d| d.border_color(theme::accent()))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .child(icon("chevron-down", 12.))
            .on_click(cx.listener(move |d, _: &ClickEvent, w, cx| {
                if enabled {
                    d.activate(&action, w, cx)
                }
            }));
        if let Some(popup) = &self.popup
            && popup.key == key
        {
            let selected = popup.selected;
            let rows: Vec<_> = popup
                .choices
                .iter()
                .enumerate()
                .map(|(i, (_, label))| {
                    div()
                        .id(("choice-item", i))
                        .px(px(10.))
                        .py(px(5.))
                        .w_full()
                        .when(i == selected, |d| d.bg(theme::selection()))
                        .hover(|d| d.bg(theme::row_hover()))
                        .child(label.clone())
                        .on_click(cx.listener(move |d, _: &ClickEvent, w, cx| {
                            cx.stop_propagation();
                            if let Some(p) = &mut d.popup {
                                p.selected = i;
                            }
                            d.choose(w, cx);
                        }))
                        .into_any_element()
                })
                .collect();
            control = control.child(deferred(
                anchored()
                    .position_mode(gpui::AnchoredPositionMode::Local)
                    .position(point(px(0.), px(28.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .id("template-choices")
                            .w(px(300.))
                            .max_h(px(240.))
                            .overflow_y_scroll()
                            .py(px(4.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(theme::popup_border())
                            .bg(theme::popup())
                            .shadow_lg()
                            .children(rows)
                            .on_mouse_down_out(cx.listener(|d, _, _, cx| {
                                d.popup = None;
                                cx.notify();
                            })),
                    ),
            ));
        }
        control.into_any_element()
    }
    fn dialog_button(
        &mut self,
        key: &'static str,
        default: bool,
        enabled: bool,
        w: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let focus = self.control(key, enabled, w, cx);
        focus
            .child(button(key, default, enabled))
            .on_click(cx.listener(move |d, _: &ClickEvent, w, cx| {
                if enabled {
                    d.activate(key, w, cx)
                }
            }))
            .into_any_element()
    }
}

impl Render for NewProjectDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_fields(window, cx);
        self.order.clear();
        let enabled = !self.busy;
        let chosen = self.draft.chosen().cloned();
        let new_solution = self.draft.target.is_none();
        let mut form = vec![];
        // Names lead the form and the tab order, as Rider does on opening.
        if new_solution {
            let field = self.field("Solution name", enabled, window, cx);
            form.push(Self::row("Solution name", field));
        }
        if self.draft.asks_project_name() {
            let field = self.field("Project name", enabled, window, cx);
            form.push(Self::row("Project name", field));
        }
        let location = self.field("Location", enabled, window, cx);
        let browse = self.dialog_button("Browse", false, enabled, window, cx);
        form.push(Self::row(
            "Location",
            div().flex().gap(px(8.)).child(div().flex_1().min_w_0().child(location)).child(browse).into_any_element(),
        ));
        if new_solution && self.draft.asks_project_name() {
            let checked = self.draft.same_directory;
            let check = self
                .control("Same directory", enabled, window, cx)
                .child(checkbox("Put solution and project in the same directory", checked))
                .on_click(cx.listener(move |d, _: &ClickEvent, w, cx| {
                    if enabled {
                        d.activate("Same directory", w, cx)
                    }
                }));
            form.push(Self::row("", check.into_any_element()));
            let choice = self.choice("Solution format", enabled, window, cx);
            form.push(Self::row("Solution format", choice));
        }
        if let Some(template) = &chosen {
            let preview = self.draft.directory.join(if new_solution {
                self.draft.solution_name()
            } else {
                self.draft.project_name()
            });
            form.push(
                div()
                    .text_color(theme::text_secondary())
                    .child(format!("Will be created in {}", preview.display()))
                    .into_any_element(),
            );
            form.push(
                div().mt(px(6.)).font_weight(FontWeight::SEMIBOLD).child(template.name.clone()).into_any_element(),
            );
            let variant = self.draft.variant().unwrap().clone();
            if !variant.description.is_empty() {
                form.push(div().text_color(theme::text_secondary()).child(variant.description).into_any_element());
            }
            if self.draft.family.is_some_and(|i| self.draft.families[i].types.len() > 1) {
                let choice = self.choice("Type", enabled, window, cx);
                form.push(Self::row("Type", choice));
            }
            if template.languages().len() > 1 {
                let choice = self.choice("Language", enabled, window, cx);
                form.push(Self::row("Language", choice));
            }
            if !template.frameworks(self.draft.language.as_deref()).is_empty() {
                let choice = self.choice("Framework", enabled, window, cx);
                form.push(Self::row("Framework", choice));
            }
            for option in variant.options {
                let allowed = enabled && self.draft.option_enabled(&option.name);
                let symbol = option.name.clone();
                let selector = symbol.clone();
                let control = match option.kind {
                    TemplateOptionKind::Bool => {
                        let key = format!("check: {symbol}");
                        let checked = self.draft.option_value(&symbol) == Some("true");
                        let checked = if option.inverted { !checked } else { checked };
                        self.control(&key, allowed, window, cx)
                            .debug_selector(move || format!("option: {selector}"))
                            .child(checkbox(option.label, checked))
                            .on_click(cx.listener(move |d, _: &ClickEvent, w, cx| {
                                if allowed {
                                    d.activate(&key, w, cx)
                                }
                            }))
                            .into_any_element()
                    }
                    TemplateOptionKind::Choice => {
                        let choice = self.choice(&symbol, allowed, window, cx);
                        Self::row(option.label, choice)
                    }
                    _ => {
                        let field = self.field(&format!("option: {symbol}"), allowed, window, cx);
                        Self::row(option.label, field)
                    }
                };
                form.push(
                    div()
                        .id(SharedString::from(format!("option-row-{symbol}")))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(control)
                        .children(
                            option
                                .description
                                .filter(|d| !d.is_empty())
                                .map(|d| div().text_color(theme::text_secondary()).child(d)),
                        )
                        .into_any_element(),
                );
            }
            if let Some(pack) = &template.pack {
                form.push(
                    div()
                        .text_color(theme::text_secondary())
                        .child(format!("This template requires {}.", pack.id))
                        .into_any_element(),
                );
                let install = self.dialog_button("Install", true, enabled && self.draft.authoritative, window, cx);
                form.push(div().flex().child(install).into_any_element());
            } else if let Ok(request) = self.draft.request()
                && let Ok(args) = create::arguments(&request, None)
            {
                let command = args
                    .iter()
                    .map(|a| {
                        let s = a.to_string_lossy();
                        if s.contains(char::is_whitespace) {
                            format!("\"{}\"", s.replace('"', "\\\""))
                        } else {
                            s.into_owned()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                form.push(
                    div()
                        .mt(px(6.))
                        .text_color(theme::text_secondary())
                        .child(format!("dotnet new {command}"))
                        .into_any_element(),
                );
            }
        }
        let progress = self.progress.lock().unwrap();
        if self.busy {
            form.push(div().font_weight(FontWeight::SEMIBOLD).child(progress.message.clone()).into_any_element());
        }
        if !progress.output.is_empty() {
            let lines: Arc<Vec<String>> = Arc::new(progress.output.lines().map(str::to_owned).collect());
            let count = lines.len();
            form.push(
                div()
                    .flex_none()
                    .h(px(160.))
                    .flex()
                    .flex_col()
                    .when(progress.truncated, |d| d.child("Earlier output truncated."))
                    .child(
                        gpui::uniform_list("template-output", count, move |range, _, _| {
                            range.map(|i| div().h(px(20.)).text_size(px(12.)).child(lines[i].clone())).collect()
                        })
                        .flex_1()
                        .min_h_0(),
                    )
                    .into_any_element(),
            );
        }
        drop(progress);
        let search = self.field("Search templates", enabled, window, cx);
        self.controls.insert("Templates".into(), self.list_focus.clone());
        if enabled {
            self.order.push("Templates".into());
        }
        let rows: Vec<_> = self
            .draft
            .listed
            .iter()
            .map(|&i| {
                let name = self.draft.families[i].name.clone();
                let label = name.clone();
                let missing = self.draft.families[i].not_installed();
                div()
                    .id(("template-family", i))
                    .debug_selector(move || format!("template family: {name}"))
                    .h(px(28.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(4.))
                    .when(self.draft.family == Some(i), |d| d.bg(theme::selection()))
                    .when(self.draft.family != Some(i), |d| d.hover(|d| d.bg(theme::row_hover())))
                    .child(icon(if missing { "package" } else { "project" }, 16.))
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .when(missing, |d| d.text_color(theme::text_secondary()))
                    .on_click(cx.listener(move |d, _: &ClickEvent, w, cx| {
                        if !d.busy {
                            d.draft.choose_family(i);
                            d.problem = None;
                            d.popup = None;
                            w.focus(&d.list_focus);
                            cx.notify();
                        }
                    }))
                    .into_any_element()
            })
            .collect();
        let ready = enabled && self.draft.authoritative && chosen.as_ref().is_some_and(|t| t.pack.is_none());
        let create = self.dialog_button("Create", true, ready, window, cx);
        let cancel = self.dialog_button("Cancel", false, true, window, cx);
        let buttons = if cfg!(target_os = "macos") { vec![cancel, create] } else { vec![create, cancel] };
        let mut footer = div()
            .flex_none()
            .border_t_1()
            .border_color(theme::control_border())
            .px(px(16.))
            .py(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.));
        if let Some(problem) = &self.problem {
            footer = footer.child(div().text_color(theme::error()).child(problem.clone()));
        }
        let retry = if self.problem.is_some() && !self.draft.authoritative {
            Some(self.dialog_button("Retry", false, enabled, window, cx))
        } else {
            None
        };
        footer = footer.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .children(retry)
                .child(div().flex_1().text_color(theme::text_secondary()).children(if self.busy {
                    Some(self.progress.lock().unwrap().message.clone())
                } else {
                    self.loading.then(|| "Loading templates…".into())
                }))
                .children(buttons),
        );
        div()
            .key_context("NewProjectDialog")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::island())
            .font_family(theme::UI_FONT)
            .text_size(px(13.))
            .text_color(theme::text())
            .on_action(cx.listener(|d, _: &Dismiss, w, cx| d.escape(w, cx)))
            .on_action(cx.listener(|d, _: &Choose, w, cx| d.choose(w, cx)))
            .on_action(cx.listener(|d, _: &NextField, w, cx| d.traverse(false, w, cx)))
            .on_action(cx.listener(|d, _: &PreviousField, w, cx| d.traverse(true, w, cx)))
            .on_action(cx.listener(|d, _: &SelectNext, w, cx| d.arrow(false, w, cx)))
            .on_action(cx.listener(|d, _: &SelectPrevious, w, cx| d.arrow(true, w, cx)))
            .on_action(cx.listener(|d, _: &ActivateControl, w, cx| {
                if let Some(k) = d.focused_control(w) {
                    d.activate(&k, w, cx)
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .w(px(260.))
                            .flex_none()
                            .p(px(8.))
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .bg(theme::frame())
                            .child(search)
                            .child(
                                div()
                                    .id("template-families")
                                    .track_focus(&self.list_focus)
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.list_scroll)
                                    .children(rows)
                                    .when(self.draft.listed.is_empty() && !self.loading, |d| {
                                        d.child(
                                            div()
                                                .p(px(8.))
                                                .text_color(theme::text_secondary())
                                                .child("No templates match"),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .id("template-form")
                            .flex_1()
                            .min_w_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.form_scroll)
                            .p(px(20.))
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .children(form),
                    ),
            )
            .child(footer)
    }
}
