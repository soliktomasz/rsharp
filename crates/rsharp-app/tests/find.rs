//! The find bar over the editor, driven by Rider's keys: it finds the word at the caret, steps through the
//! matches, replaces them, and gives the keys back to the editor when it closes.

mod support;

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use support::Run;

const REPLACE: &str = if cfg!(target_os = "macos") { "cmd-r" } else { "ctrl-h" };

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let at = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing on screen is {selector}")).center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

#[gpui::test]
fn find_starts_at_the_word_under_the_caret_and_enter_steps_round_the_matches(cx: &mut TestAppContext) {
    let run = Run::new("find", &[("Program.cs", "int count = count + 1;\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Program.cs"), Some((0, 6)), window, cx);
    });
    cx.run_until_parked();
    let count = |cx: &mut VisualTestContext| main.read_with(cx, |m, cx| m.find_count(cx));

    cx.simulate_keystrokes("secondary-f");
    cx.run_until_parked();
    assert_eq!(count(cx).as_deref(), Some("1/2"), "Find did not start at the word under the caret");

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(count(cx).as_deref(), Some("2/2"), "Enter did not go on to the next match");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(count(cx).as_deref(), Some("1/2"), "Enter past the last match did not wrap to the first");

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(count(cx), None, "Escape did not close the find bar");
    cx.simulate_keystrokes("right");
    cx.simulate_input("X");
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(run.folder.join("Program.cs")).unwrap(),
        "int countX = count + 1;\n",
        "the editor did not get the keys back where the find bar left its selection"
    );

    cx.simulate_keystrokes("secondary-f");
    cx.simulate_input("nothing");
    cx.run_until_parked();
    assert_eq!(count(cx).as_deref(), Some("0 results"), "a query typed over the old one was not searched");
}

#[gpui::test]
fn replace_takes_one_match_at_a_time_and_replace_all_the_rest_as_one_undo(cx: &mut TestAppContext) {
    let run = Run::new("replace", &[("Program.cs", "a = a + a;\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Program.cs"), Some((0, 0)), window, cx);
    });
    cx.run_until_parked();
    let file = || std::fs::read_to_string(run.folder.join("Program.cs")).unwrap();

    cx.simulate_keystrokes(REPLACE);
    cx.run_until_parked();
    click(cx, "replace-field");
    cx.simulate_input("b");
    cx.simulate_keystrokes("enter secondary-s");
    cx.run_until_parked();
    assert_eq!(file(), "b = a + a;\n", "Enter in the replace field did not replace the current match");

    click(cx, "replace-all");
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert_eq!(file(), "b = b + b;\n", "Replace All left a match");
    assert_eq!(main.read_with(cx, |m, cx| m.find_count(cx)).as_deref(), Some("Replaced 2"));

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.simulate_keystrokes("secondary-z secondary-s");
    cx.run_until_parked();
    assert_eq!(file(), "b = a + a;\n", "one Undo did not take back Replace All whole");
}

#[gpui::test]
fn a_regex_that_does_not_parse_says_why_in_the_count(cx: &mut TestAppContext) {
    let run = Run::new("find-regex", &[("Program.cs", "x\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Program.cs"), Some((1, 0)), window, cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("secondary-f");
    cx.run_until_parked();
    click(cx, "regex");
    cx.simulate_input("(a");
    cx.run_until_parked();
    let count = main.read_with(cx, |m, cx| m.find_count(cx)).expect("the find bar is open");
    assert!(count.to_lowercase().contains("unclosed"), "a broken pattern did not say what is wrong: {count:?}");
}
