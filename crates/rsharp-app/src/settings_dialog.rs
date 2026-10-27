//! Rider's Settings dialog: its pages listed on the left under a search, the chosen page on the right, and
//! OK, Cancel and Apply in Rider's order for the platform. Nothing is in force until OK or Apply.

use crate::assets::icon;
use crate::field::{Changed, TextField};
use crate::find_action::{Choose, Dismiss};
use crate::settings::{
    AppSettings, AttributeIndent, AttributeStyle, BuildResults, ContentIndent, XamlFormat, apply, settings,
};
use crate::theme;
use gpui::{
    AnyWindowHandle, App, Bounds, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, prelude::*, px, size, svg,
};

enum Opt {
    Check {
        label: &'static str,
        get: fn(&AppSettings) -> bool,
        set: fn(&mut AppSettings, bool),
    },
    Number {
        label: &'static str,
        number: Number,
    },
    /// One of a few, by its place among `names`.
    Choice {
        label: &'static str,
        names: &'static [&'static str],
        get: fn(&AppSettings) -> usize,
        set: fn(&mut AppSettings, usize),
    },
    /// A line of text, in a field as wide as the page.
    Text {
        label: &'static str,
        get: fn(&AppSettings) -> &str,
        set: fn(&mut AppSettings, String),
    },
}

impl Opt {
    fn label(&self) -> &'static str {
        match self {
            Opt::Check { label, .. }
            | Opt::Number { label, .. }
            | Opt::Choice { label, .. }
            | Opt::Text { label, .. } => label,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Number {
    FontSize,
    LineSpacing,
    XamlBlankLines,
    XamlMargin,
}

impl Number {
    const ALL: [Number; 4] = [Number::FontSize, Number::LineSpacing, Number::XamlBlankLines, Number::XamlMargin];

    fn get(self, s: &AppSettings) -> f32 {
        match self {
            Number::FontSize => s.editor_font_size,
            Number::LineSpacing => s.editor_line_spacing,
            Number::XamlBlankLines => s.xaml_format.max_blank_lines as f32,
            Number::XamlMargin => s.xaml_format.right_margin as f32,
        }
    }

    fn set(self, s: &mut AppSettings, value: f32) {
        match self {
            Number::FontSize => s.editor_font_size = value,
            Number::LineSpacing => s.editor_line_spacing = value,
            Number::XamlBlankLines => s.xaml_format.max_blank_lines = value as u32,
            Number::XamlMargin => s.xaml_format.right_margin = value as u32,
        }
    }

    /// Its range, and whether it takes whole numbers alone.
    fn range(self) -> (std::ops::RangeInclusive<f32>, bool) {
        match self {
            Number::FontSize => (AppSettings::FONT_SIZES, false),
            Number::LineSpacing => (AppSettings::LINE_SPACINGS, false),
            Number::XamlBlankLines => (XamlFormat::BLANK_LINES, true),
            Number::XamlMargin => (XamlFormat::MARGINS, true),
        }
    }
}

const RESULTS: &[(BuildResults, &str)] = &[
    (BuildResults::Always, "Always"),
    (BuildResults::WarningsOrErrors, "Warnings or errors"),
    (BuildResults::Errors, "Errors"),
    (BuildResults::Never, "Never"),
];
const ATTRIBUTE_STYLES: [AttributeStyle; 4] = [
    AttributeStyle::OnSingleLine,
    AttributeStyle::FirstAttributeOnSingleLine,
    AttributeStyle::OnDifferentLines,
    AttributeStyle::DoNotTouch,
];
const ATTRIBUTE_INDENTS: [AttributeIndent; 3] =
    [AttributeIndent::SingleIndent, AttributeIndent::DoubleIndent, AttributeIndent::AlignByFirstAttribute];
const CONTENT_INDENTS: [ContentIndent; 4] =
    [ContentIndent::DoNotTouch, ContentIndent::OneIndent, ContentIndent::ZeroIndent, ContentIndent::RemoveIndent];
/// Rider's names for the choices above, in their order.
const CONTENT_INDENT_NAMES: &[&str] =
    &["Do not change indenting", "One indent from parent element", "No indent from parent element", "No indent at all"];

fn place<T: PartialEq>(all: &[T], value: &T) -> usize {
    all.iter().position(|v| v == value).unwrap_or(0)
}

/// A check of the XAML code style, by its field.
macro_rules! xaml_check {
    ($label:literal, $field:ident) => {
        Opt::Check { label: $label, get: |s| s.xaml_format.$field, set: |s, value| s.xaml_format.$field = value }
    };
}

struct Page {
    group: &'static str,
    name: &'static str,
    options: &'static [Opt],
}

static PAGES: &[Page] = &[
    Page {
        group: "Appearance & Behavior",
        name: "System Settings",
        options: &[Opt::Check {
            label: "Reopen projects on startup",
            get: |s| s.reopen_projects_on_startup,
            set: |s, v| s.reopen_projects_on_startup = v,
        }],
    },
    Page {
        group: "Editor",
        name: "General",
        options: &[
            Opt::Check {
                label: "Show line numbers",
                get: |s| s.show_line_numbers,
                set: |s, v| s.show_line_numbers = v,
            },
            Opt::Check { label: "Show whitespaces", get: |s| s.show_whitespace, set: |s, v| s.show_whitespace = v },
        ],
    },
    Page {
        group: "Editor",
        name: "Font",
        options: &[
            Opt::Number { label: "Size", number: Number::FontSize },
            Opt::Number { label: "Line height", number: Number::LineSpacing },
        ],
    },
    Page {
        group: "Build, Execution, Deployment",
        name: "Build",
        options: &[
            Opt::Check {
                label: "Open the Build window when a build is explicitly invoked",
                get: |s| s.open_build_window,
                set: |s, value| s.open_build_window = value,
            },
            Opt::Choice {
                label: "Showing build results after build completed",
                names: &["Always", "Warnings or errors", "Errors", "Never"],
                get: |s| RESULTS.iter().position(|(mode, _)| *mode == s.build_results).unwrap_or(2),
                set: |s, i| s.build_results = RESULTS[i].0,
            },
        ],
    },
    Page {
        group: "Build, Execution, Deployment",
        name: "Hot Reload",
        options: &[
            Opt::Check {
                label: "Run with Hot Reload (dotnet watch)",
                get: |s| s.hot_reload,
                set: |s, value| s.hot_reload = value,
            },
            Opt::Check {
                label: "Always restart for a change it cannot apply",
                get: |s| s.hot_reload_restart_on_rude_edit,
                set: |s, value| s.hot_reload_restart_on_rude_edit = value,
            },
        ],
    },
    Page {
        group: "Editor",
        name: "Code Style: XAML",
        options: &[
            Opt::Choice {
                label: "Attributes format",
                names: &[
                    "On single line",
                    "First attribute on single line",
                    "Each attribute on separate lines",
                    "Do not touch",
                ],
                get: |s| place(&ATTRIBUTE_STYLES, &s.xaml_format.attribute_style),
                set: |s, i| s.xaml_format.attribute_style = ATTRIBUTE_STYLES[i],
            },
            Opt::Choice {
                label: "Attributes indenting",
                names: &["Single indent", "Double indent", "Align by first attribute"],
                get: |s| place(&ATTRIBUTE_INDENTS, &s.xaml_format.attribute_indent),
                set: |s, i| s.xaml_format.attribute_indent = ATTRIBUTE_INDENTS[i],
            },
            xaml_check!("Spaces around '=' in attribute", spaces_around_equals),
            xaml_check!("Space after last attribute", space_after_last_attribute),
            xaml_check!("Space before '/>'", space_before_self_closing),
            xaml_check!("Keep existing line breaks", keep_line_breaks),
            Opt::Number { label: "Maximum blank lines between tags", number: Number::XamlBlankLines },
            xaml_check!("Place multiline elements on a new line", multiline_elements_on_new_line),
            xaml_check!("Place single-line elements on a new line", singleline_elements_on_new_line),
            xaml_check!(
                "Line breaks after start-tag and before end-tag in elements that contain other elements and no text",
                breaks_inside_elements_with_children
            ),
            xaml_check!(
                "Line breaks after start-tag and before end-tag in multiline elements",
                breaks_inside_multiline_elements
            ),
            Opt::Choice {
                label: "Indent inside element that does not contain text",
                names: CONTENT_INDENT_NAMES,
                get: |s| place(&CONTENT_INDENTS, &s.xaml_format.child_indent),
                set: |s, i| s.xaml_format.child_indent = CONTENT_INDENTS[i],
            },
            Opt::Choice {
                label: "Indent inside element that contains text",
                names: CONTENT_INDENT_NAMES,
                get: |s| place(&CONTENT_INDENTS, &s.xaml_format.text_indent),
                set: |s, i| s.xaml_format.text_indent = CONTENT_INDENTS[i],
            },
            xaml_check!("Blank line after processing instructions", blank_line_after_processing_instructions),
            xaml_check!("Wrap tags and processing instructions", wrap_tags),
            xaml_check!("Wrap before and after elements", wrap_around_elements),
            Opt::Number { label: "Right margin (columns)", number: Number::XamlMargin },
            // XAML Styler's, which Rider has from that plugin.
            xaml_check!("Reorder attributes", reorder_attributes),
            Opt::Text {
                label: "Attribute ordering rule groups",
                get: |s| &s.xaml_format.attribute_order,
                set: |s, groups| s.xaml_format.attribute_order = groups,
            },
            xaml_check!("Remove ending tag of empty element", collapse_empty_elements),
        ],
    },
];

pub struct SettingsDialog {
    draft: AppSettings,
    search: Entity<TextField>,
    numbers: Vec<(Number, Entity<TextField>)>,
    texts: Vec<(&'static str, Entity<TextField>)>,
    page: usize,
    problem: Option<SharedString>,
    focus: FocusHandle,
    /// The choice whose list is open, by its label.
    choosing: Option<&'static str>,
}

/// Opens Settings, or brings it forward when it is open already.
pub fn open(cx: &mut App) {
    if let Some(window) = cx.windows().into_iter().find(|w| w.downcast::<SettingsDialog>().is_some()) {
        let _ = window.update(cx, |_, window, _| window.activate_window());
        return;
    }
    let bounds = Bounds::centered(None, size(px(880.), px(620.)), cx);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("Settings".into()), ..Default::default() }),
            window_min_size: Some(size(px(640.), px(420.))),
            ..Default::default()
        },
        |window, cx| {
            let dialog = cx.new(|cx| SettingsDialog::new(cx));
            window.focus(&dialog.read(cx).search.focus_handle(cx));
            dialog
        },
    );
    if let Err(error) = opened {
        log::error!("Settings did not open: {error}");
    }
}

impl SettingsDialog {
    fn new(cx: &mut Context<Self>) -> Self {
        let draft = settings(cx).clone();
        let search = cx.new(|cx| TextField::new("Search settings", cx));
        cx.subscribe(&search, |d, _, _: &Changed, cx| {
            if let Some(first) = d.shown(cx).first() {
                d.page = *first;
            }
            cx.notify()
        })
        .detach();
        let field = |value: &str, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut f = TextField::new("", cx);
                f.set_text(value, cx);
                f
            })
        };
        let numbers = Number::ALL
            .into_iter()
            .map(|number| {
                let shown = field(&format_number(number.get(&draft)), cx);
                cx.subscribe(&shown, |d, _, _: &Changed, cx| d.read_numbers(cx)).detach();
                (number, shown)
            })
            .collect();
        let texts = PAGES
            .iter()
            .flat_map(|p| p.options)
            .filter_map(|o| match o {
                Opt::Text { label, get, set } => {
                    let (label, set) = (*label, *set);
                    let shown = field(get(&draft), cx);
                    cx.subscribe(&shown, move |d, shown, _: &Changed, cx| {
                        set(&mut d.draft, shown.read(cx).text().to_string());
                        cx.notify()
                    })
                    .detach();
                    Some((label, shown))
                }
                _ => None,
            })
            .collect();
        Self { draft, search, numbers, texts, page: 0, problem: None, focus: cx.focus_handle(), choosing: None }
    }

    /// The pages the search leaves, by index: a page stays when its name, its group or an option on it matches.
    fn shown(&self, cx: &App) -> Vec<usize> {
        let query = self.search.read(cx).text().to_lowercase();
        let matches = |text: &str| text.to_lowercase().contains(query.trim());
        PAGES
            .iter()
            .enumerate()
            .filter(|(_, p)| matches(p.name) || matches(p.group) || p.options.iter().any(|o| matches(o.label())))
            .map(|(i, _)| i)
            .collect()
    }

    /// Reads the number fields into the draft, or says which is out of range.
    fn read_numbers(&mut self, cx: &mut Context<Self>) {
        self.problem = None;
        for (number, field) in &self.numbers {
            let (number, text) = (*number, field.read(cx).text().to_string());
            let label = PAGES
                .iter()
                .flat_map(|p| p.options)
                .find_map(|o| matches!(o, Opt::Number { number: n, .. } if *n == number).then(|| o.label()))
                .unwrap_or_default();
            let (range, whole) = number.range();
            match text.trim().parse::<f32>() {
                Ok(value) if range.contains(&value) && (!whole || value.fract() == 0.) => {
                    number.set(&mut self.draft, value)
                }
                _ => {
                    self.problem = Some(
                        format!(
                            "{label} must be a number from {} to {}",
                            format_number(*range.start()),
                            format_number(*range.end())
                        )
                        .into(),
                    );
                }
            }
        }
        cx.notify();
    }

    fn changed(&self, cx: &App) -> bool {
        self.problem.is_none() && self.draft != *settings(cx)
    }

    fn apply_draft(&mut self, cx: &mut Context<Self>) {
        if self.problem.is_none() {
            apply(self.draft.clone(), cx);
            cx.notify();
        }
    }

    fn ok(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.problem.is_none() {
            self.apply_draft(cx);
            window.remove_window();
        }
    }

    /// The setting a test or a person reads off the dialog's page.
    pub fn draft(&self) -> &AppSettings {
        &self.draft
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_ref().map(|p| p.as_ref())
    }

    pub fn page_names(&self, cx: &App) -> Vec<&'static str> {
        self.shown(cx).into_iter().map(|i| PAGES[i].name).collect()
    }
}

impl Focusable for SettingsDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.shown(cx);
        let page = &PAGES[self.page];
        let changed = self.changed(cx);
        let mut list = Vec::new();
        let mut group = "";
        for &i in &shown {
            if PAGES[i].group != group {
                group = PAGES[i].group;
                list.push(
                    div()
                        .h(px(24.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .text_color(theme::text_muted())
                        .child(group)
                        .into_any_element(),
                );
            }
            let selected = i == self.page;
            list.push(
                div()
                    .id(("page", i))
                    .debug_selector(move || format!("page: {}", PAGES[i].name))
                    .h(px(24.))
                    .pl(px(26.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .when(selected, |d| d.bg(theme::selection()))
                    .when(!selected, |d| d.hover(|d| d.bg(theme::row_hover())))
                    .child(PAGES[i].name)
                    .on_click(cx.listener(move |d, _: &ClickEvent, _, cx| {
                        d.page = i;
                        cx.notify()
                    }))
                    .into_any_element(),
            );
        }
        let options: Vec<_> = page
            .options
            .iter()
            .map(|option| match option {
                Opt::Check { label, get, set } => {
                    let checked = get(&self.draft);
                    let set = *set;
                    checkbox(*label, checked)
                        .on_click(cx.listener(move |d, _: &ClickEvent, _, cx| {
                            set(&mut d.draft, !checked);
                            cx.notify()
                        }))
                        .into_any_element()
                }
                Opt::Number { label, number } => div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().min_w(px(90.)).child(*label))
                    .child(
                        div()
                            .debug_selector({
                                let label = *label;
                                move || format!("field: {label}")
                            })
                            .w(px(80.))
                            .h(px(28.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .rounded(px(4.))
                            .bg(theme::island())
                            .border_1()
                            .border_color(theme::control_border())
                            .children(self.numbers.iter().find(|(n, _)| n == number).map(|(_, f)| f.clone())),
                    )
                    .into_any_element(),
                Opt::Choice { label, names, get, set } => {
                    let (label, names, set) = (*label, *names, *set);
                    let chosen = names.get(get(&self.draft)).copied().unwrap_or_default();
                    let open = self.choosing == Some(label);
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(label)
                        .child(
                            div()
                                .id(label)
                                .debug_selector(move || format!("choice: {label}"))
                                .w(px(280.))
                                .h(px(28.))
                                .px(px(8.))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(theme::control_border())
                                .flex()
                                .items_center()
                                .child(div().flex_1().child(chosen))
                                .child(icon("chevron-down", 16.))
                                .on_click(cx.listener(move |d, _, _, cx| {
                                    d.choosing = if open { None } else { Some(label) };
                                    cx.notify();
                                })),
                        )
                        .when(open, |d| {
                            d.child(div().w(px(280.)).flex().flex_col().children(names.iter().enumerate().map(
                                |(i, name)| {
                                    div()
                                        .id(("choice", i))
                                        .debug_selector(move || format!("{label}: {name}"))
                                        .h(px(24.))
                                        .px(px(8.))
                                        .flex()
                                        .items_center()
                                        .rounded(px(4.))
                                        .hover(|d| d.bg(theme::row_hover()))
                                        .child(*name)
                                        .on_click(cx.listener(move |d, _, _, cx| {
                                            set(&mut d.draft, i);
                                            d.choosing = None;
                                            cx.notify();
                                        }))
                                },
                            )))
                        })
                        .into_any_element()
                }
                Opt::Text { label, .. } => div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(*label)
                    .child(
                        div()
                            .debug_selector({
                                let label = *label;
                                move || format!("field: {label}")
                            })
                            .h(px(28.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .rounded(px(4.))
                            .bg(theme::island())
                            .border_1()
                            .border_color(theme::control_border())
                            .overflow_hidden()
                            .children(self.texts.iter().find(|(l, _)| l == label).map(|(_, f)| f.clone())),
                    )
                    .into_any_element(),
            })
            .collect();
        let mac = cfg!(target_os = "macos");
        let ok = button("OK", true, self.problem.is_none())
            .on_click(cx.listener(|d, _: &ClickEvent, window, cx| d.ok(window, cx)));
        let cancel =
            button("Cancel", false, true).on_click(cx.listener(|_, _: &ClickEvent, window, _| window.remove_window()));
        let apply_button = button("Apply", false, changed).on_click(cx.listener(move |d, _: &ClickEvent, _, cx| {
            if changed {
                d.apply_draft(cx)
            }
        }));
        let buttons = if mac { vec![cancel, apply_button, ok] } else { vec![ok, cancel, apply_button] };

        div()
            .key_context("SettingsDialog")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::island())
            .text_color(theme::text())
            .font_family(theme::UI_FONT)
            .text_size(px(13.))
            .on_action(cx.listener(|_, _: &Dismiss, window, _| window.remove_window()))
            .on_action(cx.listener(|d, _: &Choose, window, cx| d.ok(window, cx)))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .w(px(250.))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .p(px(8.))
                            .bg(theme::frame())
                            .child(
                                div()
                                    .h(px(30.))
                                    .mb(px(6.))
                                    .px(px(8.))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .rounded(px(6.))
                                    .bg(theme::island())
                                    .border_1()
                                    .border_color(theme::control_border())
                                    .child(icon("search", 16.))
                                    .child(self.search.clone()),
                            )
                            .children(list)
                            .when(shown.is_empty(), |d| {
                                d.child(div().p(px(10.)).text_color(theme::text_secondary()).child("No settings match"))
                            }),
                    )
                    .child(
                        div()
                            .id("settings-page")
                            .flex_1()
                            .min_w_0()
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .p(px(20.))
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("{} › {}", page.group, page.name)),
                            )
                            .children(options),
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
                    .child(div().flex_1().text_color(theme::error()).children(self.problem.clone()))
                    .children(buttons),
            )
    }
}

pub(crate) fn checkbox(label: impl Into<SharedString>, checked: bool) -> gpui::Stateful<gpui::Div> {
    let label = label.into();
    div()
        .id(label.clone())
        .debug_selector({
            let label = label.clone();
            move || format!("check: {label}")
        })
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex_none()
                .size(px(16.))
                .rounded(px(3.))
                .flex()
                .items_center()
                .justify_center()
                .when(checked, |d| {
                    d.bg(theme::accent())
                        .child(svg().path("icons/check.svg").size(px(14.)).text_color(theme::on_accent()))
                })
                .when(!checked, |d| d.border_1().border_color(theme::control_border())),
        )
        // A label longer than the page wraps rather than running past its edge.
        .child(div().flex_1().min_w_0().child(label))
}

/// A dialog button as Rider's: the default one filled with the accent, the rest outlined; a disabled one dimmed.
pub(crate) fn button(label: &'static str, default: bool, enabled: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(label)
        .debug_selector(move || format!("button: {label}"))
        .h(px(28.))
        .min_w(px(72.))
        .px(px(14.))
        .rounded(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .when(default && enabled, |d| d.bg(theme::accent()).text_color(theme::on_accent()))
        .when(!default || !enabled, |d| d.border_1().border_color(theme::control_border()))
        .when(!enabled, |d| d.text_color(theme::text_secondary()))
        .when(enabled && !default, |d| d.hover(|d| d.bg(theme::button_hover())))
        .child(label)
}

/// A number as a field shows it: `13`, not `13.0`.
fn format_number(value: f32) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The open Settings window, for a test to drive.
pub fn window(cx: &App) -> Option<AnyWindowHandle> {
    cx.windows().into_iter().find(|w| w.downcast::<SettingsDialog>().is_some())
}
