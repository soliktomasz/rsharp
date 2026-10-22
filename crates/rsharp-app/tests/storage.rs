//! How a file is stored, as Rider keeps it: its encoding and line separator read and written back, changed from
//! the status bar, its .editorconfig followed, its own indent detected, and a read-only file asked about first.

mod support;

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use rsharp_app::window::MainWindow;
use support::Run;

fn open<'a>(
    run: &Run,
    file: &str,
    caret: (usize, usize),
    cx: &'a mut TestAppContext,
) -> (Entity<MainWindow>, &'a mut VisualTestContext) {
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join(file), Some(caret), window, cx);
    });
    cx.run_until_parked();
    (main, cx)
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let at = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing on screen is {selector}")).center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

fn save(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
}

#[gpui::test]
fn an_eight_bit_file_is_written_back_in_its_own_encoding_and_refuses_what_it_cannot_hold(cx: &mut TestAppContext) {
    let run = Run::new("encoding", &[]);
    std::fs::write(run.folder.join("Old.cs"), b"caf\xE9;\n").unwrap();
    let (main, cx) = open(&run, "Old.cs", (0, 0), cx);

    cx.simulate_input("x");
    save(cx);
    assert_eq!(std::fs::read(run.folder.join("Old.cs")).unwrap(), b"xcaf\xE9;\n", "the file was not kept windows-1252");

    cx.simulate_input("😀");
    save(cx);
    assert_eq!(
        std::fs::read(run.folder.join("Old.cs")).unwrap(),
        b"xcaf\xE9;\n",
        "a character windows-1252 cannot hold was written"
    );
    let said = main.read_with(cx, |m, _| m.said().map(str::to_string)).unwrap_or_default();
    assert!(said.contains("windows-1252"), "the refused save did not say why: {said:?}");
}

#[gpui::test]
fn the_status_bar_changes_how_the_file_is_written(cx: &mut TestAppContext) {
    let run = Run::new("separators", &[("Program.cs", "a\nb\n")]);
    let (_main, cx) = open(&run, "Program.cs", (0, 0), cx);

    click(cx, "status: line separator");
    click(cx, "status menu: CRLF - Windows (\\r\\n)");
    click(cx, "status: encoding");
    click(cx, "status menu: UTF-8 BOM");
    save(cx);
    assert_eq!(
        std::fs::read(run.folder.join("Program.cs")).unwrap(),
        b"\xEF\xBB\xBFa\r\nb\r\n",
        "the line separator or encoding chosen in the status bar was not how the file was saved"
    );
}

#[gpui::test]
fn editorconfig_sets_the_indent_and_what_a_save_tidies(cx: &mut TestAppContext) {
    let config = "root = true\n[*.cs]\nindent_style = tab\nend_of_line = crlf\ninsert_final_newline = true\ntrim_trailing_whitespace = true\n";
    let run = Run::new("editorconfig", &[(".editorconfig", config), ("Program.cs", "class C {   ")]);
    let (_main, cx) = open(&run, "Program.cs", (0, 9), cx);

    cx.simulate_keystrokes("enter");
    cx.simulate_input("x;");
    save(cx);
    assert_eq!(
        std::fs::read_to_string(run.folder.join("Program.cs")).unwrap(),
        "class C {\r\n\tx;\r\n",
        "the .editorconfig's tab indent, trimming, final newline or CRLF was not followed"
    );
}

#[gpui::test]
fn a_files_own_indent_is_what_tab_inserts(cx: &mut TestAppContext) {
    let run = Run::new("detect", &[("Program.cs", "class C\n{\n  int a;\n  int b;\n}\n")]);
    let (_main, cx) = open(&run, "Program.cs", (3, 0), cx);
    cx.simulate_keystrokes("tab");
    cx.simulate_input("x");
    save(cx);
    assert_eq!(
        std::fs::read_to_string(run.folder.join("Program.cs")).unwrap(),
        "class C\n{\n  int a;\n  x  int b;\n}\n",
        "Tab did not insert the two spaces the file indents by"
    );
}

#[gpui::test]
fn a_read_only_file_asks_before_its_first_edit_and_takes_it_once_cleared(cx: &mut TestAppContext) {
    let run = Run::new("read-only", &[("Locked.cs", "int a;\n")]);
    let file = run.folder.join("Locked.cs");
    let mut permissions = std::fs::metadata(&file).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&file, permissions).unwrap();
    let (_main, cx) = open(&run, "Locked.cs", (0, 0), cx);

    cx.simulate_input("x");
    cx.run_until_parked();
    assert!(cx.has_pending_prompt(), "typing in a read-only file did not ask to clear its read-only status");
    cx.simulate_prompt_answer("Clear Read-Only Status");
    cx.run_until_parked();
    save(cx);
    assert!(!std::fs::metadata(&file).unwrap().permissions().readonly(), "the file was not made writable");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "xint a;\n", "the edit asked about was not made once cleared");
}
