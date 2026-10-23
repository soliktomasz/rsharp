//! The editor tabs as Rider's: splits that show one file over one text, pinned tabs first, the closes, Reopen
//! Closed Tab and the tab's own menu.

mod support;

use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext};
use rsharp_app::window::*;
use support::Run;

fn click(cx: &mut VisualTestContext, selector: &str) {
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    let at = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing on screen is {selector}")).center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

fn groups(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> Vec<Vec<String>> {
    main.read_with(cx, |m, cx| m.groups(cx))
}

#[gpui::test]
fn a_split_shows_the_same_text_and_undo_in_one_takes_back_typing_in_the_other(cx: &mut TestAppContext) {
    let run = Run::new("split", &[("Program.cs", "int x;\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Program.cs"), Some((0, 0)), window, cx);
    });
    cx.run_until_parked();

    cx.dispatch_action(SplitRight);
    cx.run_until_parked();
    assert_eq!(groups(&main, cx), [["Program.cs"], ["Program.cs"]], "Split Right did not show the file beside itself");
    assert_eq!(main.read_with(cx, |m, _| m.current_group()), 1, "the new split did not take the keys");

    cx.simulate_input("var");
    cx.run_until_parked();
    click(cx, "tab 0 Program.cs");
    cx.simulate_keystrokes("secondary-z secondary-s");
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(run.folder.join("Program.cs")).unwrap(),
        "int x;\n",
        "Undo in one split did not take back what was typed in the other, so they hold two texts"
    );

    cx.dispatch_action(SplitDown);
    cx.run_until_parked();
    assert_eq!(groups(&main, cx).len(), 3);
    cx.simulate_keystrokes("secondary-w");
    cx.run_until_parked();
    assert_eq!(groups(&main, cx).len(), 2, "closing a split's last tab did not close the split");
    cx.dispatch_action(UnsplitAll);
    cx.run_until_parked();
    assert_eq!(groups(&main, cx), [["Program.cs"]], "Unsplit All left a split, or the file twice");
}

#[gpui::test]
fn pinned_tabs_stay_first_and_through_the_closes_and_a_closed_tab_comes_back(cx: &mut TestAppContext) {
    let files = [("a.cs", ""), ("b.cs", ""), ("c.cs", ""), ("d.cs", "")];
    let run = Run::new("tabs", &files);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        for (file, _) in files {
            main.open_file(&run.folder.join(file), None, window, cx);
        }
    });
    cx.run_until_parked();
    let tabs = |cx: &mut VisualTestContext| groups(&main, cx).remove(0);

    click(cx, "tab 0 c.cs");
    cx.dispatch_action(PinTab);
    cx.run_until_parked();
    assert_eq!(tabs(cx), ["*c.cs", "a.cs", "b.cs", "d.cs"], "a pinned tab did not go first");

    click(cx, "tab 0 b.cs");
    cx.dispatch_action(CloseOtherTabs);
    cx.run_until_parked();
    assert_eq!(tabs(cx), ["*c.cs", "b.cs"], "Close Other Tabs closed a pinned tab, or left an unpinned one");

    cx.dispatch_action(ReopenClosedTab);
    cx.run_until_parked();
    assert_eq!(tabs(cx), ["*c.cs", "b.cs", "a.cs"], "Reopen Closed Tab did not bring back the tab closed last");

    cx.dispatch_action(CloseTabsToLeft);
    cx.run_until_parked();
    assert_eq!(tabs(cx), ["*c.cs", "a.cs"], "Close Tabs to the Left closed the wrong tabs");

    cx.dispatch_action(CloseAllTabs);
    cx.run_until_parked();
    assert_eq!(tabs(cx), ["*c.cs"], "Close All Tabs closed the pinned tab, or left an unpinned one");
}

#[gpui::test]
fn the_tab_menu_splits_the_tab_it_was_opened_on(cx: &mut TestAppContext) {
    let run = Run::new("tab-menu", &[("a.cs", ""), ("b.cs", "")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("a.cs"), None, window, cx);
        main.open_file(&run.folder.join("b.cs"), None, window, cx);
    });
    cx.run_until_parked();

    let at = cx.debug_bounds("tab 0 a.cs").expect("a.cs has a tab").center();
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    click(cx, "tab menu: Split Down");
    assert_eq!(groups(&main, cx), [vec!["a.cs", "b.cs"], vec!["a.cs"]], "the menu did not split the tab it opened on");
}

#[gpui::test]
fn a_tab_dragged_out_of_the_window_opens_in_a_window_of_its_own_over_the_same_text(cx: &mut TestAppContext) {
    let run = Run::new("drag-out", &[("a.cs", "int a;\n"), ("b.cs", "")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("a.cs"), None, window, cx);
        main.open_file(&run.folder.join("b.cs"), None, window, cx);
    });
    cx.run_until_parked();

    let from = cx.debug_bounds("tab 0 a.cs").expect("a.cs has a tab").center();
    let to = gpui::point(gpui::px(-40.), gpui::px(-40.));
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    for step in 1..=8 {
        cx.simulate_mouse_move(from + (to - from) * (step as f32 / 8.), Some(MouseButton::Left), Modifiers::default());
    }
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();

    assert_eq!(groups(&main, cx), [["b.cs"]], "the tab dragged out stayed in the main window");
    let opened = cx
        .update(|_, cx| cx.windows().into_iter().find_map(|w| w.downcast::<rsharp_app::editor_window::EditorWindow>()));
    let opened = opened.expect("no editor window opened for the tab let go outside");
    let tabs = cx.update(|_, cx| opened.read(cx).map(|w| w.tabs(cx)).unwrap());
    assert_eq!(tabs, ["a.cs"]);

    main.update_in(cx, |main, window, cx| main.open_file(&run.folder.join("a.cs"), Some((0, 0)), window, cx));
    cx.simulate_input("var ");
    cx.run_until_parked();
    let texts = cx.update(|_, cx| opened.read(cx).map(|w| w.texts(cx)).unwrap());
    assert_eq!(texts, ["var int a;\n"], "the editor window did not show what the main window typed in the same file");
}
