//! Bookmarks from the window: toggled on the caret's line, marked in the gutter, walked with Next, listed in the
//! Bookmarks window, carried through edits and kept with the folder.

mod support;

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use rsharp_app::window::{MainWindow, NextBookmark};
use support::Run;

const SHOP: &str = "namespace Shop;\n\npublic class Cart\n{\n    public void Pay() { }\n}\n";

fn toggle(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "f3" } else { "ctrl-k k" });
    cx.run_until_parked();
}

fn marked(main: &gpui::Entity<MainWindow>, path: &std::path::Path, cx: &mut VisualTestContext) -> Vec<usize> {
    main.read_with(cx, |_, cx| {
        rsharp_app::buffer::find(&path.to_path_buf(), cx).map(|b| b.read(cx).bookmarks.clone()).unwrap_or_default()
    })
}

#[gpui::test]
fn bookmarks_toggle_walk_list_follow_edits_and_are_kept(cx: &mut TestAppContext) {
    let run = Run::new("bookmarks", &[("Shop.cs", SHOP)]);
    let source = rsharp_core::paths::canonical(&run.folder.join("Shop.cs"));
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&source, Some((4, 4)), window, cx);
    });
    cx.run_until_parked();

    toggle(cx);
    assert_eq!(marked(&main, &source, cx), [4], "the caret's line was not bookmarked in the gutter");

    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((0, 0)), window, cx));
    cx.run_until_parked();
    cx.dispatch_action(NextBookmark);
    cx.run_until_parked();
    let caret = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).caret());
    assert_eq!(caret.line, 4, "Next Bookmark did not go to the bookmarked line");

    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-f3" } else { "ctrl-k w" });
    cx.run_until_parked();
    assert_eq!(
        main.read_with(cx, |m, cx| m.bookmark_rows(cx)),
        ["Shop.cs:5  public void Pay() { }"],
        "the Bookmarks window does not list the bookmark with its line"
    );

    // A line typed above moves the bookmark down with its code.
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((0, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_input("// note\n");
    cx.run_until_parked();
    assert_eq!(marked(&main, &source, cx), [5], "the bookmark did not follow its line down");

    // The folder closed and opened again keeps it.
    main.update_in(cx, |main, window, cx| main.close_solution(window, cx));
    cx.run_until_parked();
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&source, Some((0, 0)), window, cx);
    });
    cx.run_until_parked();
    assert_eq!(marked(&main, &source, cx), [5], "the bookmark was not kept with the folder");

    click(cx, "bookmark: 0");
    let caret = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).caret());
    assert_eq!(caret.line, 5, "a click on the bookmark's row did not go to its line");
    toggle(cx);
    assert!(marked(&main, &source, cx).is_empty(), "toggling again did not take the bookmark away");
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let at = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing on screen is {selector}")).center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

#[gpui::test]
fn a_bookmark_follows_its_line_through_a_change_on_disk(cx: &mut TestAppContext) {
    let run = Run::new("bookmarks-external", &[("Shop.cs", SHOP)]);
    let source = rsharp_core::paths::canonical(&run.folder.join("Shop.cs"));
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&source, Some((4, 4)), window, cx);
    });
    cx.run_until_parked();
    toggle(cx);
    assert_eq!(marked(&main, &source, cx), [4]);
    // Another program (a git checkout, an agent) adds a line above it: the open file reloads.
    std::fs::write(&source, format!("using System;\n{SHOP}")).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while marked(&main, &source, cx) != [5] && std::time::Instant::now() < deadline {
        cx.executor().advance_clock(std::time::Duration::from_millis(200));
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(marked(&main, &source, cx), [5], "a change on disk lost the bookmark rather than moving it");
}
