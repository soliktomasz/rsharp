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

#[gpui::test]
fn search_everywhere_finds_files_by_their_letters(cx: &mut TestAppContext) {
    let run = Run::new("everywhere", FILES);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| main.open_workspace(&run.folder, window, cx));
    cx.run_until_parked();
    // Rider's macOS keymap has it on Shift pressed twice, which gpui does not bind; it is run as the menu runs it.
    cx.update(|window, cx| window.dispatch_action(Box::new(rsharp_app::window::SearchEverywhere), cx));
    settle(cx);
    cx.simulate_input("gree");
    settle(cx);
    let listed = main.read_with(cx, |m, cx| m.go_to_listed(cx)).unwrap_or_default();
    assert_eq!(listed.first().map(String::as_str), Some("Greeter.cs"), "Search Everywhere listed {listed:?}");
}

#[gpui::test]
fn recent_locations_lists_the_places_left_with_their_lines_and_goes_back_to_one(cx: &mut TestAppContext) {
    let run = Run::new("recent-locations", FILES);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Greeter.cs"), Some((2, 4)), window, cx);
        main.open_file(&run.folder.join("Program.cs"), Some((0, 0)), window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(key("cmd-shift-e", "ctrl-shift-e"));
    settle(cx);
    let listed = main.read_with(cx, |m, cx| m.go_to_listed(cx)).unwrap_or_default();
    assert_eq!(listed, ["// Hello there Greeter.cs:3"], "Recent Locations did not list the place left");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(active(&main, cx), ("Greeter.cs".into(), 2), "Enter did not go back to the place");
}

#[gpui::test]
fn find_in_files_looks_only_in_its_scope_and_the_files_its_mask_fits(cx: &mut TestAppContext) {
    let run = Run::new("find-scope", FILES);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("src/Store.cs"), None, window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(key("cmd-shift-f", "ctrl-shift-f"));
    settle(cx);
    cx.simulate_input("hello");
    settle(cx);
    let count = |cx: &mut VisualTestContext| main.read_with(cx, |m, cx| m.go_to_listed(cx)).unwrap_or_default().len();
    assert_eq!(count(cx), 2, "the Solution scope did not find both lines");
    let popup = main.read_with(cx, |m, _| m.go_to_popup()).unwrap();
    let src = run.folder.join("src");
    popup.update(cx, |p, cx| p.set_scope(rsharp_app::goto::Scope::Directory(src), cx));
    settle(cx);
    assert_eq!(count(cx), 1, "the Directory scope looked outside its directory");
    popup.update(cx, |p, cx| {
        p.set_scope(rsharp_app::goto::Scope::Solution, cx);
        p.set_mask("*.xaml", cx);
    });
    settle(cx);
    assert_eq!(count(cx), 0, "the mask let in files it does not fit");
}

#[gpui::test]
fn replace_in_files_replaces_every_match_in_open_and_closed_files(cx: &mut TestAppContext) {
    let run = Run::new("replace-in-files", FILES);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Greeter.cs"), None, window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(key("cmd-shift-r", "ctrl-shift-h"));
    settle(cx);
    cx.simulate_input("hello");
    settle(cx);
    let popup = main.read_with(cx, |m, _| m.go_to_popup()).unwrap();
    popup.update(cx, |p, cx| {
        p.set_replacement("Bye", cx);
        p.replace_all(cx);
    });
    cx.run_until_parked();
    assert_eq!(main.read_with(cx, |m, _| m.said().map(str::to_string)), Some("Replaced 2 matches".into()));
    let open = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).text().to_string());
    assert!(open.contains("// Bye there"), "the open file was not replaced in its tab: {open:?}");
    let closed = std::fs::read_to_string(run.folder.join("src/Store.cs")).unwrap();
    assert!(closed.contains("// Bye again"), "the closed file was not written: {closed:?}");
}
