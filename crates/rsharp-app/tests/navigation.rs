//! Getting around the folder as a person does: Back and Forward, Last Edit Location, Recent Files, Go to File and
//! Find in Files, each by its keys.

mod support;

use gpui::{Entity, TestAppContext, VisualTestContext};
use rsharp_app::window::MainWindow;
use std::time::Duration;
use support::Run;

/// Rider's key for an action: its macOS keymap's on macOS, its Visual Studio keymap's elsewhere.
fn key(mac: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") { mac } else { other }
}

const FILES: &[(&str, &str)] = &[
    ("Greeter.cs", "class Greeter\n{\n    // Hello there\n}\n"),
    ("Program.cs", "class Program\n{\n    static void Main() { }\n}\n"),
    ("src/Store.cs", "class Store\n{\n    // hello again\n}\n"),
];

fn active(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> (String, usize) {
    main.read_with(cx, |m, cx| {
        let editor = m.editor().expect("a file is open").read(cx);
        (editor.path.file_name().unwrap().to_string_lossy().into_owned(), editor.caret().line)
    })
}

/// Lets the popup's pause before it looks pass, and what it looked for land.
fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
}

#[gpui::test]
fn back_and_forward_go_between_the_places_left(cx: &mut TestAppContext) {
    let run = Run::new("back-forward", FILES);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Greeter.cs"), Some((2, 0)), window, cx);
        main.open_file(&run.folder.join("Program.cs"), Some((3, 0)), window, cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes(key("cmd-[", "ctrl--"));
    cx.run_until_parked();
    assert_eq!(active(&main, cx), ("Greeter.cs".into(), 2), "Back did not return to the place left");
    cx.simulate_keystrokes(key("cmd-]", "ctrl-shift--"));
    cx.run_until_parked();
    assert_eq!(active(&main, cx), ("Program.cs".into(), 3), "Forward did not go to the place Back left");
}

#[gpui::test]
fn last_edit_location_returns_to_where_the_text_was_changed(cx: &mut TestAppContext) {
    let run = Run::new("last-edit", FILES);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Greeter.cs"), Some((2, 4)), window, cx);
    });
    cx.run_until_parked();
    cx.simulate_input("x");
    main.update_in(cx, |main, window, cx| main.open_file(&run.folder.join("Program.cs"), Some((0, 0)), window, cx));
    cx.run_until_parked();

    cx.simulate_keystrokes(key("cmd-shift-backspace", "ctrl-shift-backspace"));
    cx.run_until_parked();
    assert_eq!(active(&main, cx), ("Greeter.cs".into(), 2), "Last Edit Location did not go to the edit");
}

#[gpui::test]
fn recent_files_lists_the_files_shown_the_last_first_and_goes_to_the_one_chosen(cx: &mut TestAppContext) {
    let run = Run::new("recent", FILES);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        for file in ["Greeter.cs", "src/Store.cs", "Program.cs"] {
            main.open_file(&run.folder.join(file), None, window, cx);
        }
    });
    cx.run_until_parked();

    cx.simulate_keystrokes(key("cmd-e", "ctrl-,"));
    settle(cx);
    let listed = main.read_with(cx, |m, cx| m.go_to_listed(cx)).unwrap_or_default();
    assert_eq!(listed, ["Store.cs src", "Greeter.cs"], "Recent Files did not list the others, the last shown first");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(active(&main, cx).0, "Store.cs", "Enter did not go to the file shown before");
}

#[gpui::test]
fn go_to_file_finds_a_file_by_the_letters_of_its_name(cx: &mut TestAppContext) {
    let run = Run::new("go-to-file", FILES);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| main.open_workspace(&run.folder, window, cx));
    cx.run_until_parked();

    cx.simulate_keystrokes(key("cmd-shift-o", "ctrl-shift-t"));
    settle(cx);
    cx.simulate_input("sto");
    settle(cx);
    let listed = main.read_with(cx, |m, cx| m.go_to_listed(cx)).unwrap_or_default();
    assert_eq!(listed.first().map(String::as_str), Some("Store.cs src"), "Go to File listed {listed:?}");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(active(&main, cx).0, "Store.cs", "Enter did not open the file found");
}

#[gpui::test]
fn find_in_files_lists_every_line_holding_the_text_and_goes_to_one(cx: &mut TestAppContext) {
    let run = Run::new("find-in-files", FILES);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| main.open_workspace(&run.folder, window, cx));
    cx.run_until_parked();

    cx.simulate_keystrokes(key("cmd-shift-f", "ctrl-shift-f"));
    settle(cx);
    cx.simulate_input("hello");
    settle(cx);
    let listed = main.read_with(cx, |m, cx| m.go_to_listed(cx)).unwrap_or_default();
    assert_eq!(
        listed,
        ["// Hello there Greeter.cs:3", "// hello again Store.cs:3"],
        "Find in Files did not find both lines, case ignored"
    );
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    assert_eq!(active(&main, cx), ("Store.cs".into(), 2), "Enter did not go to the line found");
}
