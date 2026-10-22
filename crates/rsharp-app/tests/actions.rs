//! Every action by name: Find Action finds it, runs it where the person was, and says why when it cannot.

mod support;

use gpui::TestAppContext;
use support::Run;

#[gpui::test]
fn find_action_runs_the_action_chosen_by_name_where_the_person_was(cx: &mut TestAppContext) {
    let run = Run::new("find-action", &[("Program.cs", "class C {}\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Program.cs"), None, window, cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("secondary-shift-a");
    cx.simulate_input("cltab");
    cx.run_until_parked();
    let listed = main.read_with(cx, |main, cx| main.find_action_listed(cx)).expect("Find Action opened");
    assert_eq!(listed.first(), Some(&"Close Tab"), "typing 'cltab' did not put Close Tab first: {listed:?}");

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    main.read_with(cx, |main, cx| {
        assert!(main.find_action_listed(cx).is_none(), "Find Action stayed open after running an action");
        assert!(main.tab_carets(cx).is_empty(), "Close Tab chosen in Find Action did not close the tab");
    });
}

#[gpui::test]
fn an_action_that_cannot_run_says_why_instead_of_doing_nothing(cx: &mut TestAppContext) {
    let run = Run::new("unavailable", &[]);
    let (main, cx) = run.window(cx);
    cx.run_until_parked();

    cx.simulate_keystrokes("secondary-shift-a");
    cx.simulate_input("close solution");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    main.read_with(cx, |main, _| {
        assert_eq!(
            main.said(),
            Some("Close Solution is not available here"),
            "with nothing open, Close Solution said nothing"
        );
    });
}

#[gpui::test]
fn escape_closes_find_action_and_typing_goes_back_to_the_editor(cx: &mut TestAppContext) {
    let run = Run::new("escape", &[("a.txt", "")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("a.txt"), None, window, cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("secondary-shift-a");
    cx.simulate_input("x");
    cx.simulate_keystrokes("escape");
    cx.simulate_input("typed");
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();

    assert!(main.read_with(cx, |main, cx| main.find_action_listed(cx)).is_none(), "Escape did not close Find Action");
    assert_eq!(
        std::fs::read_to_string(run.folder.join("a.txt")).unwrap(),
        "typed",
        "after Escape the keys did not reach the editor"
    );
}
