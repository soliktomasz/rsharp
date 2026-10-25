mod support;

use gpui::{Modifiers, TestAppContext, point, px};
use rsharp_app::editor;
use std::sync::Arc;
use support::Run;

#[gpui::test]
fn clicking_a_change_shows_both_texts_and_rollback_is_one_undoable_edit(cx: &mut TestAppContext) {
    let before = "old\nkeep\ntail";
    let after = "new\nkeep\nTAIL\n";
    let run = Run::new("git-gutter", &[("file.txt", after)]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("file.txt"), None, window, cx);
    });
    cx.run_until_parked();
    let editor = main.read_with(cx, |main, _| main.editor().unwrap());
    editor.update(cx, |editor, cx| {
        editor.buffer.update(cx, |buffer, cx| {
            buffer.git_base = Some(Arc::from(before));
            buffer.git_hunks = Arc::new(rsharp_text::diff_hunks(before, after));
            cx.notify();
        });
    });
    cx.run_until_parked();
    let surface = cx.debug_bounds("editor surface").expect("the editor is visible");
    cx.simulate_click(surface.origin + point(px(4.), px(8.)), Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("git before: old").is_some(), "the change popup did not show the committed line");
    assert!(cx.debug_bounds("git after: new").is_some(), "the change popup did not show the working line");
    let rollback = cx.debug_bounds("git rollback lines").expect("the popup offers Rollback Lines").center();
    cx.simulate_click(rollback, Modifiers::default());
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |editor, _| editor.text().to_string()), "old\nkeep\nTAIL\n");
    cx.dispatch_action(editor::Undo);
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.text().to_string()),
        after,
        "one Undo did not restore the whole hunk"
    );
}

#[gpui::test]
fn a_wrapped_change_opens_the_same_hunk_from_its_continuation_row(cx: &mut TestAppContext) {
    let before = format!("old {}\n", "word ".repeat(250));
    let after = format!("new {}\n", "word ".repeat(250));
    let run = Run::new("git-gutter-wrap", &[("file.txt", &after)]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("file.txt"), None, window, cx);
    });
    cx.run_until_parked();
    let editor = main.read_with(cx, |main, _| main.editor().unwrap());
    editor.update(cx, |editor, cx| {
        editor.buffer.update(cx, |buffer, cx| {
            buffer.git_base = Some(Arc::from(before.as_str()));
            buffer.git_hunks = Arc::new(rsharp_text::diff_hunks(&before, &after));
            cx.notify();
        });
    });
    cx.dispatch_action(editor::ToggleSoftWrap);
    cx.run_until_parked();
    let line_height = cx.update(|_, cx| rsharp_app::settings::settings(cx).line_height());
    let surface = cx.debug_bounds("editor surface").unwrap();
    cx.simulate_click(surface.origin + point(px(4.), px(line_height * 1.5)), Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("git change popup").is_some(), "a wrapped row lost the change bar's hunk");
    let rollback = cx.debug_bounds("git rollback lines").unwrap().center();
    cx.dispatch_action(editor::Escape);
    cx.run_until_parked();
    cx.simulate_click(rollback, Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.text().to_string()),
        after,
        "Escape left the popup's rollback actionable"
    );
}

#[gpui::test]
fn a_folded_body_keeps_its_changed_lines_reachable_from_the_fold_header(cx: &mut TestAppContext) {
    let before = "class C\n{\n    int old;\n}\n";
    let after = "class C\n{\n    int changed;\n}\n";
    let run = Run::new("git-gutter-fold", &[("Program.cs", after)]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Program.cs"), None, window, cx);
    });
    cx.run_until_parked();
    let editor = main.read_with(cx, |main, _| main.editor().unwrap());
    editor.update(cx, |editor, cx| {
        editor.buffer.update(cx, |buffer, cx| {
            buffer.git_base = Some(Arc::from(before));
            buffer.git_hunks = Arc::new(rsharp_text::diff_hunks(before, after));
            cx.notify();
        });
    });
    cx.dispatch_action(editor::CollapseAll);
    cx.run_until_parked();
    let line_height = cx.update(|_, cx| rsharp_app::settings::settings(cx).line_height());
    let surface = cx.debug_bounds("editor surface").unwrap();
    cx.simulate_click(surface.origin + point(px(4.), px(line_height * 1.5)), Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("git before:     int old;").is_some(), "the folded body hid its change popup");
    let rollback = cx.debug_bounds("git rollback lines").unwrap().center();
    editor.update(cx, |editor, cx| {
        editor.buffer.update(cx, |buffer, cx| {
            buffer.git_base = Some(Arc::from("different base\n"));
            cx.notify();
        });
    });
    cx.run_until_parked();
    cx.simulate_click(rollback, Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.text().to_string()),
        after,
        "a stale popup rolled back against a changed baseline"
    );
}
