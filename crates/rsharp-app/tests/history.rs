//! Local History, as Rider's: each version the IDE saw kept, another program's change named as one, and a version
//! put back with Revert as one step Undo takes back.

mod support;

use gpui::{TestAppContext, VisualTestContext, WindowHandle};
use rsharp_app::history_window::HistoryWindow;
use rsharp_text::LineChange;
use support::Run;

fn history(cx: &mut VisualTestContext) -> WindowHandle<HistoryWindow> {
    cx.update(|_, cx| cx.windows().into_iter().find_map(|w| w.downcast::<HistoryWindow>()))
        .expect("Show Local History opened no window")
}

#[gpui::test]
fn each_version_is_kept_and_revert_puts_one_back_as_a_step_undo_takes_back(cx: &mut TestAppContext) {
    let run = Run::new("history", &[("Program.cs", "a;\n")]);
    let file = run.folder.join("Program.cs");
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&file, Some((0, 0)), window, cx);
    });
    cx.run_until_parked();
    for typed in ["b", "c"] {
        cx.simulate_input(typed);
        cx.simulate_keystrokes("secondary-s");
        cx.run_until_parked();
    }
    cx.simulate_keystrokes("secondary-w");
    cx.run_until_parked();
    std::fs::write(&file, "d;\n").unwrap();
    main.update_in(cx, |main, window, cx| main.open_file(&file, Some((0, 0)), window, cx));
    cx.run_until_parked();

    cx.dispatch_action(rsharp_app::window::ShowHistory);
    cx.run_until_parked();
    let window = history(cx);
    let listed = cx.update(|_, cx| window.read(cx).unwrap().listed());
    assert_eq!(listed.len(), 4, "not every version was kept: {listed:?}");
    assert!(listed[0].ends_with("External change"), "another program's change was not named as one: {listed:?}");

    let first = cx.update(|_, cx| window.read(cx).unwrap().lines());
    assert!(
        first.iter().any(|(change, _)| *change != LineChange::Same),
        "the window opened on a version no different from the text now"
    );
    cx.update(|_, cx| window.update(cx, |w, _, cx| w.choose(3, cx)).unwrap());
    let lines = cx.update(|_, cx| window.read(cx).unwrap().lines());
    assert_eq!(
        lines,
        [(LineChange::Removed, "a;".to_string()), (LineChange::Added, "d;".to_string())],
        "the oldest version's differences from the text now were not shown"
    );
    cx.update(|_, cx| window.update(cx, |w, _, cx| w.revert(cx)).unwrap());
    cx.run_until_parked();
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "a;\n", "Revert did not put the oldest version back");

    cx.simulate_keystrokes("secondary-z secondary-s");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "d;\n", "Undo did not take the revert back as one step");
}
