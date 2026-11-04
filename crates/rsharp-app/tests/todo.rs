//! The TODO window: every TODO and FIXME comment in the folder, kept fresh as files change on disk.

mod support;

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use rsharp_app::window::ShowTodo;
use std::time::{Duration, Instant};
use support::Run;

fn wait_for(cx: &mut VisualTestContext, until: impl Fn(&mut VisualTestContext) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        if until(cx) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[gpui::test]
fn todo_window_lists_and_refreshes(cx: &mut TestAppContext) {
    let run = Run::new(
        "todo",
        &[
            ("A.cs", "// TODO: one\nclass A { }\n"),
            ("Sub/B.cs", "class B\n{\n    // FIXME two\n    string s = \"TODO not this\";\n}\n"),
            ("C.cs", "class C { }\n"),
        ],
    );
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| main.open_workspace(&run.folder, window, cx));
    cx.run_until_parked();
    cx.dispatch_action(ShowTodo);
    let rows = |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.todo_rows());
    let both = ["A.cs:1  TODO: one", "B.cs:3  FIXME two"];
    assert!(wait_for(cx, |cx| rows(cx) == both), "the TODO window listed {:?}", rows(cx));

    std::fs::write(run.folder.join("A.cs"), "class A { }\n").unwrap();
    assert!(
        wait_for(cx, |cx| rows(cx) == ["B.cs:3  FIXME two"]),
        "a TODO removed on disk stayed listed: {:?}",
        rows(cx)
    );

    let at = cx.debug_bounds("todo: 0").expect("the FIXME's row").center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
    let (tab, line) = main.read_with(cx, |m, cx| (m.active_tab_name(cx), m.editor().unwrap().read(cx).caret().line));
    assert_eq!((tab.as_deref(), line), (Some("B.cs"), 2), "a click on the row did not open its line");
}
