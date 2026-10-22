//! What the IDE keeps between runs: a folder reopens as it was left, and the Welcome screen lists it.

mod support;

use gpui::{AppContext, TestAppContext};
use rsharp_app::state::{RecentSolutions, store};
use rsharp_app::window::MainWindow;
use support::Run;

#[gpui::test]
fn a_folder_reopens_with_its_tabs_carets_and_open_folders(cx: &mut TestAppContext) {
    let run = Run::new(
        "reopen",
        &[("App.slnx", "<Solution />"), ("src/A.cs", "class A\n{\n}\n"), ("src/B.cs", "class B {}\n")],
    );
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("src/B.cs"), Some((0, 6)), window, cx);
        main.open_file(&run.folder.join("src/A.cs"), Some((2, 0)), window, cx);
    });
    cx.run_until_parked();

    let again = cx.update(|window, cx| cx.new(|cx| MainWindow::new(window, cx)));
    again.update_in(cx, |main, window, cx| main.open_workspace(&run.folder, window, cx));
    cx.run_until_parked();

    again.read_with(cx, |main, cx| {
        assert_eq!(
            main.tab_carets(cx),
            [("B.cs".into(), (0, 6)), ("A.cs".into(), (2, 0))],
            "the tabs or their carets were not kept"
        );
        assert_eq!(
            main.active_tab_name(cx).as_deref(),
            Some("A.cs"),
            "the tab that was in front is not in front again"
        );
    });
    let recent = cx.update(|_, cx| store(cx).read::<RecentSolutions>());
    assert_eq!(recent.entries.len(), 1, "opening the same folder twice listed it twice");
    assert_eq!(recent.entries[0].name, "App", "the folder is listed by its solution's name");
}
