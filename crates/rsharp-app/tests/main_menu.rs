//! The main menu on Windows and Linux, which have no menu bar: the toolbar's button opens every menu as a
//! section, and a choice runs where the person was.
#![cfg(not(target_os = "macos"))]

mod support;

use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext, point, px};
use support::Run;

/// Scrolls the main menu until `selector` is inside the window, as a person would to reach a later menu.
fn scroll_to(cx: &mut VisualTestContext, selector: &'static str) {
    let height = cx.update(|w, _| w.viewport_size().height);
    let menu = cx.debug_bounds("main menu dropdown").expect("the main menu did not open");
    for _ in 0..100 {
        if cx.debug_bounds(selector).is_some_and(|b| b.bottom() <= height) {
            return;
        }
        cx.simulate_event(ScrollWheelEvent {
            position: menu.center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(-200.))),
            ..Default::default()
        });
        cx.run_until_parked();
    }
    panic!("{selector} cannot be scrolled into the window");
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let at = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing on screen is {selector}")).center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

#[gpui::test]
fn a_menu_choice_runs_where_the_person_was(cx: &mut TestAppContext) {
    let run = Run::new("main-menu", &[("Program.cs", "class C {}\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Program.cs"), None, window, cx);
    });
    cx.run_until_parked();

    click(cx, "main menu");
    let height = cx.update(|w, _| w.viewport_size().height);
    assert!(
        cx.debug_bounds("main menu dropdown").is_some_and(|b| b.bottom() <= height),
        "the main menu runs past the bottom of the window"
    );
    scroll_to(cx, "menu: Close Tab");
    click(cx, "menu: Close Tab");

    assert!(
        main.read_with(cx, |main, cx| main.tab_carets(cx)).is_empty(),
        "Close Tab from the main menu did not close the tab"
    );
}

#[gpui::test]
fn a_menu_choice_that_cannot_run_says_why(cx: &mut TestAppContext) {
    let run = Run::new("main-menu-greyed", &[]);
    let (main, cx) = run.window(cx);
    cx.run_until_parked();

    click(cx, "main menu");
    click(cx, "menu: Close Solution");

    assert_eq!(
        main.read_with(cx, |main, _| main.said().map(str::to_string)).as_deref(),
        Some("Close Solution is not available here")
    );
}
