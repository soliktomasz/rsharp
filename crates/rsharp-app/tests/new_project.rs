mod support;
#[path = "support/templates.rs"]
mod templates;

use gpui::{Modifiers, TestAppContext, VisualTestContext, WindowHandle};
use rsharp_app::{
    new_project_dialog::{self, NewProjectDialog},
    window::MainWindow,
};
use support::Run;

fn open(main: &mut VisualTestContext, run: &Run) -> (WindowHandle<NewProjectDialog>, VisualTestContext) {
    let owner = main.update(|window, _| window.window_handle().downcast::<MainWindow>().unwrap());
    let handle = main.update(|_, cx| new_project_dialog::open(owner, run.folder.clone(), None, cx).unwrap());
    handle.update(main, |d, _, cx| d.show_catalog(templates::fixture(), true, cx)).unwrap();
    let cx = VisualTestContext::from_window(handle.into(), main);
    cx.run_until_parked();
    (handle, cx)
}
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let b = cx.debug_bounds(selector).unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(b.center(), Modifiers::default());
    cx.run_until_parked();
}
#[gpui::test]
fn the_default_name_has_the_keys_on_open(cx: &mut TestAppContext) {
    let run = Run::new("new-project-default", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    cx.simulate_input("Greeter");
    cx.run_until_parked();
    let names = handle
        .read_with(&cx, |d, _| (d.draft().solution_name().to_string(), d.draft().project_name().to_string()))
        .unwrap();
    assert_eq!(names, ("Greeter".into(), "Greeter".into()));
    assert!(cx.debug_bounds("button: Create").is_some());
    assert!(cx.debug_bounds("button: Cancel").is_some());
}
#[gpui::test]
fn tab_and_arrows_reach_all_visible_fields(cx: &mut TestAppContext) {
    let run = Run::new("new-project-keys", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("Own.Project");
    cx.run_until_parked();
    assert_eq!(handle.read_with(&cx, |d, _| d.draft().project_name().to_string()).unwrap(), "Own.Project");
    click(&mut cx, "field: Search templates");
    cx.simulate_input("unit");
    cx.run_until_parked();
    assert_eq!(
        handle.read_with(&cx, |d, _| d.draft().families[d.draft().family.unwrap()].name.clone()).unwrap(),
        "Unit Test"
    );
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(
        handle.read_with(&cx, |d, _| d.draft().families[d.draft().family.unwrap()].name.clone()).unwrap(),
        "Aspire"
    );
}
#[gpui::test]
fn escape_clears_search_before_cancel(cx: &mut TestAppContext) {
    let run = Run::new("new-project-escape", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    click(&mut cx, "field: Search templates");
    cx.simulate_input("no matches");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(handle.read_with(&cx, |d, _| d.draft().query.is_empty()).unwrap());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(handle.read_with(&cx, |_, _| ()).is_err(), "Escape did not close the dialog");
}
#[gpui::test]
fn fields_follow_the_selected_type(cx: &mut TestAppContext) {
    let run = Run::new("new-project-types", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    click(&mut cx, "field: Search templates");
    cx.simulate_input("avalonia.xplat");
    cx.run_until_parked();
    assert!(!handle.read_with(&cx, |d, _| d.draft().asks_project_name()).unwrap());
    assert!(cx.debug_bounds("button: Install").is_some());
}
#[gpui::test]
fn errors_keep_inputs_and_actions_visible(cx: &mut TestAppContext) {
    let run = Run::new("new-project-errors", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_keystrokes("backspace");
    click(&mut cx, "button: Create");
    assert_eq!(
        handle.read_with(&cx, |d, _| d.problem().map(str::to_string)).unwrap().as_deref(),
        Some("Solution name cannot be empty")
    );
    assert!(cx.debug_bounds("field: Solution name").is_some());
    assert!(cx.debug_bounds("button: Cancel").is_some());
}
#[gpui::test]
fn closed_dialog_ignores_a_late_read(cx: &mut TestAppContext) {
    let run = Run::new("new-project-close", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    click(&mut cx, "button: Cancel");
    assert!(handle.update(&mut cx, |d, _, cx| d.show_catalog(templates::fixture(), true, cx)).is_err());
}

#[gpui::test]
fn choices_and_checkboxes_work_by_keyboard_without_stealing_field_spaces(cx: &mut TestAppContext) {
    let run = Run::new("new-project-controls", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    click(&mut cx, "choice: Framework");
    cx.simulate_keystrokes("down");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(handle.read_with(&cx, |d, _| d.draft().framework.clone()).unwrap().as_deref(), Some("net9.0"));
    click(&mut cx, "option: UseProgramMain");
    cx.simulate_keystrokes("space");
    cx.run_until_parked();
    assert_eq!(
        handle.read_with(&cx, |d, _| d.draft().option_value("UseProgramMain").map(str::to_string)).unwrap().as_deref(),
        Some("false")
    );
    click(&mut cx, "field: Solution name");
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("My Solution");
    cx.run_until_parked();
    assert_eq!(handle.read_with(&cx, |d, _| d.draft().solution_name().to_string()).unwrap(), "My Solution");
}

#[gpui::test]
fn a_small_window_keeps_the_footer_below_long_labels_and_errors(cx: &mut TestAppContext) {
    let run = Run::new("new-project-minimum", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    let mut catalog = (*templates::fixture()).clone();
    let console = &mut catalog.families.iter_mut().find(|f| f.name == "Console").unwrap().types[0];
    console.variants[0].options[0].label =
        "An unusually long localized template option that must wrap without hiding the buttons".repeat(3);
    handle.update(&mut cx, |d, _, cx| d.show_catalog(std::sync::Arc::new(catalog), true, cx)).unwrap();
    cx.simulate_resize(gpui::size(gpui::px(720.), gpui::px(480.)));
    cx.run_until_parked();
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_keystrokes("backspace");
    click(&mut cx, "button: Create");
    for selector in ["button: Create", "button: Cancel"] {
        let bounds = cx.debug_bounds(selector).unwrap();
        assert!(bounds.bottom() <= gpui::px(480.), "{selector} escaped the minimum window");
        assert!(bounds.left() >= gpui::px(0.) && bounds.right() <= gpui::px(720.));
    }
}
