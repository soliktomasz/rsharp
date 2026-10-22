//! The window driven as a person drives it: a folder opened, a file opened at a caret, keys pressed.

mod support;

use gpui::TestAppContext;
use support::Run;

#[gpui::test]
fn what_is_typed_is_saved_with_the_files_own_line_endings(cx: &mut TestAppContext) {
    let run = Run::new("typing", &[("Program.cs", "class C\r\n{\r\n}\r\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Program.cs"), Some((1, 1)), window, cx);
    });
    cx.run_until_parked();

    cx.simulate_input("int x;");
    cx.simulate_keystrokes("enter secondary-s");
    cx.run_until_parked();

    assert_eq!(
        std::fs::read_to_string(run.folder.join("Program.cs")).unwrap(),
        "class C\r\n{int x;\r\n\r\n}\r\n",
        "the typed text was not saved, or the file lost its CRLF line endings"
    );
}

#[gpui::test]
fn carets_cloned_below_type_on_every_line_and_undo_as_one(cx: &mut TestAppContext) {
    let run = Run::new("carets", &[("Program.cs", "a\nb\nc\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Program.cs"), Some((0, 0)), window, cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("alt-shift-down alt-shift-down");
    cx.simulate_input("//");
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    let file = || std::fs::read_to_string(run.folder.join("Program.cs")).unwrap();
    assert_eq!(file(), "//a\n//b\n//c\n", "typing with carets cloned below did not type on every line");

    cx.simulate_keystrokes("secondary-z secondary-s");
    cx.run_until_parked();
    assert_eq!(file(), "a\nb\nc\n", "one Undo did not take back the run typed at every caret");

    cx.simulate_keystrokes("escape");
    cx.simulate_input("x");
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert_eq!(file(), "a\nb\nxc\n", "Escape did not leave only the caret added last");
}

#[gpui::test]
fn every_occurrence_selected_is_replaced_by_what_is_typed(cx: &mut TestAppContext) {
    let run = Run::new("occurrences", &[("Program.cs", "int a = a + a;\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Program.cs"), Some((0, 4)), window, cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "ctrl-cmd-g" } else { "alt-shift-;" });
    cx.simulate_input("b");
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(run.folder.join("Program.cs")).unwrap(),
        "int b = b + b;\n",
        "Select All Occurrences did not put a caret on every copy of the word"
    );
}

#[gpui::test]
fn shift_and_the_arrows_select_a_column_in_column_selection_mode(cx: &mut TestAppContext) {
    let run = Run::new("column-mode", &[("Program.cs", "abcd\nab\nabcd\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Program.cs"), Some((0, 1)), window, cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-shift-8" } else { "alt-shift-insert" });
    cx.simulate_keystrokes("shift-down shift-down shift-right shift-right");
    cx.simulate_input("X");
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(run.folder.join("Program.cs")).unwrap(),
        "aXd\naX\naXd\n",
        "Shift and the arrows in Column Selection Mode did not select the same columns on every line"
    );
}
