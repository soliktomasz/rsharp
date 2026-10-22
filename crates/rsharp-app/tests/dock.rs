//! The tool windows' frame, driven by the mouse: stripe buttons open and close windows, a button dragged to
//! another stripe moves its window, a gap dragged resizes, and the folder keeps it all.

mod support;

use gpui::{AppContext, Modifiers, MouseButton, Point, TestAppContext, VisualTestContext};
use rsharp_app::dock::{Place, Tool};
use rsharp_app::window::MainWindow;
use support::Run;

fn centre(cx: &mut VisualTestContext, selector: &'static str) -> Point<gpui::Pixels> {
    cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing on screen is {selector}")).center()
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let at = centre(cx, selector);
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

/// Presses on one place, moves in steps to another and lets go there, as a hand drags.
fn drag(cx: &mut VisualTestContext, from: Point<gpui::Pixels>, to: Point<gpui::Pixels>) {
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    for step in 1..=8 {
        let at = from + (to - from) * (step as f32 / 8.);
        cx.simulate_mouse_move(at, Some(MouseButton::Left), Modifiers::default());
    }
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
}

#[gpui::test]
fn a_stripe_button_opens_and_closes_its_window_and_shift_escape_hides_it(cx: &mut TestAppContext) {
    let run = Run::new("dock-toggle", &[("a.cs", "class A {}\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| main.open_workspace(&run.folder, window, cx));
    cx.run_until_parked();

    click(cx, "stripe: Problems");
    main.read_with(cx, |main, _| {
        assert!(main.dock().is_open(Tool::Problems), "a click on Problems' button did not open it");
        assert_eq!(
            main.dock().open_in(Place::BottomLeft),
            Some(Tool::Problems),
            "Problems opened away from Rider's place for it"
        );
    });
    cx.simulate_keystrokes("shift-escape");
    cx.run_until_parked();
    assert!(
        !main.read_with(cx, |main, _| main.dock().is_open(Tool::Problems)),
        "Shift+Escape did not hide the window that had the keys"
    );
    assert!(main.read_with(cx, |main, _| main.dock().is_open(Tool::Solution)), "hiding Problems closed Solution too");
}

#[gpui::test]
fn a_window_dragged_to_the_right_stripe_moves_there_and_the_folder_keeps_it(cx: &mut TestAppContext) {
    let run = Run::new("dock-move", &[("a.cs", "class A {}\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| main.open_workspace(&run.folder, window, cx));
    cx.run_until_parked();

    let from = centre(cx, "stripe: Problems");
    let to = centre(cx, "stripe group: Right Top");
    drag(cx, from, to);
    assert_eq!(
        main.read_with(cx, |main, _| main.dock().place_of(Tool::Problems)),
        Place::RightTop,
        "the dropped button did not move Problems"
    );

    let again = cx.update(|window, cx| cx.new(|cx| MainWindow::new(window, cx)));
    again.update_in(cx, |main, window, cx| main.open_workspace(&run.folder, window, cx));
    assert_eq!(
        again.read_with(cx, |main, _| main.dock().place_of(Tool::Problems)),
        Place::RightTop,
        "the folder forgot where Problems was moved"
    );
}

#[gpui::test]
fn dragging_the_gap_beside_solution_resizes_it(cx: &mut TestAppContext) {
    let run = Run::new("dock-resize", &[]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| main.open_workspace(&run.folder, window, cx));
    cx.run_until_parked();

    let before = main.read_with(cx, |main, _| main.dock().left_width);
    let from = centre(cx, "splitter: left");
    drag(cx, from, from + gpui::point(gpui::px(100.), gpui::px(0.)));
    let after = main.read_with(cx, |main, _| main.dock().left_width);
    assert!(
        (after - before - 100.).abs() < 4.,
        "dragging the gap 100 px right made Solution {} wide, from {before}",
        after
    );
}
