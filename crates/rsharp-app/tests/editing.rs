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
