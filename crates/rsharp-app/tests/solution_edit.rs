//! Changing the solution from the Solution view, as Rider's: the row's menu and its keys, the name popup, and solution
//! folders added, renamed and removed, each changing the .sln or .slnx on disk and nothing else in it.

mod support;

use gpui::{Entity, TestAppContext, VisualTestContext};
use rsharp_app::window::MainWindow;
use std::path::Path;
use support::Run;

const PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\" />\n";

const SLNX: &str = concat!(
    "<Solution>\n  <Project Path=\"src/Shop.Web/Shop.Web.csproj\" />\n",
    "  <Folder Name=\"/Solution Items/\">\n    <File Path=\"README.md\" />\n  </Folder>\n",
    "  <Folder Name=\"/src/\">\n    <Project Path=\"src/Shop.Core/Shop.Core.csproj\" />\n  </Folder>\n</Solution>\n",
);

fn files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Shop.slnx", SLNX),
        ("src/Shop.Web/Shop.Web.csproj", PROJECT),
        ("src/Shop.Web/Program.cs", "class Program { }\n"),
        ("src/Shop.Web/Models/Order.cs", "class Order { }\n"),
        ("src/Shop.Core/Shop.Core.csproj", PROJECT),
        ("README.md", "# Shop\n"),
    ]
}

fn open<'a>(run: &Run, cx: &'a mut TestAppContext) -> (Entity<MainWindow>, &'a mut VisualTestContext) {
    let (main, cx) = run.window(cx);
    let folder = run.folder.clone();
    main.update_in(cx, |main, window, cx| main.open_workspace(&folder, window, cx));
    cx.run_until_parked();
    (main, cx)
}

fn rows(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> Vec<String> {
    main.read_with(cx, |main, _| main.solution_rows())
}

fn row(main: &Entity<MainWindow>, cx: &mut VisualTestContext, name: &str) -> usize {
    let rows = rows(main, cx);
    rows.iter().position(|r| r.trim_start() == name).unwrap_or_else(|| panic!("no row {name} in {rows:?}"))
}

fn open_row(main: &Entity<MainWindow>, cx: &mut VisualTestContext, name: &str) {
    let at = row(main, cx, name);
    main.update(cx, |main, cx| main.toggle_row(at, cx));
}

fn choose(main: &Entity<MainWindow>, cx: &mut VisualTestContext, name: &str, item: &str) {
    let at = row(main, cx, name);
    main.update_in(cx, |main, window, cx| main.choose(at, item, window, cx));
    cx.run_until_parked();
}

/// Types a name into the popup that asks one, and presses Enter.
fn answer(main: &Entity<MainWindow>, cx: &mut VisualTestContext, name: &str) {
    assert!(main.read_with(cx, |main, _| main.name_popup().is_some()), "no popup asked the name");
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-a" } else { "ctrl-a" });
    cx.simulate_input(name);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

fn on_disk(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// A key as the menu shows it: Rider's macOS keymap on macOS, its Visual Studio keymap elsewhere.
fn key(mac: &str, other: &str) -> String {
    if cfg!(target_os = "macos") { mac.into() } else { other.into() }
}

#[gpui::test]
fn the_solution_view_menus_offer_riders_items_with_their_keys(cx: &mut TestAppContext) {
    let run = Run::new("se-menus", &files());
    let (main, cx) = open(&run, cx);
    for name in ["Solution Items", "src", "Shop.Web"] {
        open_row(&main, cx, name);
    }
    let (rename, delete) = (key("⇧F6", "F2"), key("⌫", "Delete"));
    let (cut, copy, paste) = (key("⌘X", "Ctrl+X"), key("⌘C", "Ctrl+C"), key("⌘V", "Ctrl+V"));
    let menu = |cx: &mut VisualTestContext, name: &str| {
        let at = row(&main, cx, name);
        main.read_with(cx, |main, _| main.solution_menu(at))
    };
    assert_eq!(menu(cx, "Shop"), ["Add > New Solution Folder...", "Add > Existing Project...", "Copy Path"]);
    assert_eq!(
        menu(cx, "src"),
        [
            "Add > New Solution Folder...".to_string(),
            "Add > Existing Project...".into(),
            "Add > Existing Item...".into(),
            format!("Edit > Remove... ({delete})"),
            format!("Edit > Rename... ({rename})"),
        ],
        "a solution folder's menu would not be Rider's"
    );
    assert_eq!(
        menu(cx, "Shop.Web"),
        [
            "Add > New File...".to_string(),
            "Add > New Folder...".into(),
            "Add > Class/Interface...".into(),
            "Add > Reference...".into(),
            format!("Edit > Paste ({paste})"),
            format!("Edit > Remove... ({delete})"),
            "Copy Path".into(),
        ],
        "a project's menu would not be Rider's"
    );
    assert_eq!(menu(cx, "Dependencies"), ["Add Reference..."]);
    assert_eq!(
        menu(cx, "Models"),
        [
            "Add > New File...".to_string(),
            "Add > New Folder...".into(),
            "Add > Class/Interface...".into(),
            format!("Edit > Cut ({cut})"),
            format!("Edit > Copy ({copy})"),
            format!("Edit > Paste ({paste})"),
            format!("Edit > Delete... ({delete})"),
            format!("Edit > Rename... ({rename})"),
            "Copy Path".into(),
        ],
        "a folder's menu would not be Rider's"
    );
    assert_eq!(
        menu(cx, "Program.cs"),
        [
            format!("Edit > Cut ({cut})"),
            format!("Edit > Copy ({copy})"),
            format!("Edit > Paste ({paste})"),
            format!("Edit > Delete... ({delete})"),
            format!("Edit > Rename... ({rename})"),
            "Copy Path".into(),
        ],
        "a file's menu would not be Rider's"
    );
    assert_eq!(menu(cx, "README.md"), [format!("Edit > Remove... ({delete})"), "Copy Path".into()]);
}

#[gpui::test]
fn a_solution_folder_added_renamed_and_removed_changes_only_its_lines(cx: &mut TestAppContext) {
    let run = Run::new("se-folders", &files());
    let solution = run.folder.join("Shop.slnx");
    let (main, cx) = open(&run, cx);
    choose(&main, cx, "Shop", "Add > New Solution Folder...");
    answer(&main, cx, "Tools");
    assert!(
        on_disk(&solution).ends_with("  </Folder>\n  <Folder Name=\"/Tools/\" />\n</Solution>\n"),
        "the folder would not be written as Visual Studio writes it:\n{}",
        on_disk(&solution)
    );
    assert_eq!(
        main.read_with(cx, |main, _| main.selected_row()).as_deref(),
        Some("  Tools"),
        "the folder added would not be selected"
    );
    choose(&main, cx, "Tools", "Edit > Rename...");
    answer(&main, cx, "Utilities");
    assert!(
        rows(&main, cx).contains(&"  Utilities".to_string()),
        "the renamed folder would not show: {:?}",
        rows(&main, cx)
    );
    assert!(on_disk(&solution).contains("<Folder Name=\"/Utilities/\" />"));
    choose(&main, cx, "Utilities", "Edit > Remove...");
    assert!(cx.has_pending_prompt(), "removing a solution folder would not ask first");
    cx.simulate_prompt_answer("Remove");
    cx.run_until_parked();
    assert_eq!(on_disk(&solution), SLNX, "with the folder removed, the .slnx would not be as it was");
    assert!(!rows(&main, cx).iter().any(|r| r.contains("Utilities")), "a removed folder would still show");
}

#[gpui::test]
fn removing_a_solution_folder_names_what_goes_with_it_and_cancelled_changes_nothing(cx: &mut TestAppContext) {
    let run = Run::new("se-remove-asks", &files());
    let solution = run.folder.join("Shop.slnx");
    let (main, cx) = open(&run, cx);
    choose(&main, cx, "src", "Edit > Remove...");
    assert_eq!(
        main.read_with(cx, |main, _| main.asked().map(str::to_string)).as_deref(),
        Some("Remove the solution folder src from Shop, and with it the project Shop.Core? Their files stay on disk."),
        "the question would not name what goes with the folder"
    );
    assert!(cx.has_pending_prompt(), "removing a solution folder would not ask first");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(on_disk(&solution), SLNX, "a cancelled Remove would change the solution");
}

#[gpui::test]
fn a_name_taken_or_invalid_keeps_the_popup_open_saying_why(cx: &mut TestAppContext) {
    let run = Run::new("se-names", &files());
    let solution = run.folder.join("Shop.slnx");
    let (main, cx) = open(&run, cx);
    for refused in ["SRC", "a/b", " "] {
        choose(&main, cx, "Shop", "Add > New Solution Folder...");
        answer(&main, cx, refused);
        let popup =
            main.read_with(cx, |main, _| main.name_popup()).unwrap_or_else(|| panic!("{refused:?} closed the popup"));
        let problem = popup.read_with(cx, |popup, _| popup.problem().map(str::to_string));
        assert!(problem.is_some(), "{refused:?} would be refused without saying why");
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(main.read_with(cx, |main, _| main.name_popup().is_none()), "Escape would leave the popup open");
    }
    assert_eq!(on_disk(&solution), SLNX, "a refused name would change the solution");
}

#[gpui::test]
fn a_solution_open_with_unsaved_edits_is_saved_before_a_folder_is_added_and_keeps_them(cx: &mut TestAppContext) {
    let run = Run::new("se-unsaved", &files());
    let solution = run.folder.join("Shop.slnx");
    let (main, cx) = open(&run, cx);
    main.update_in(cx, |main, window, cx| main.open_file(&solution, Some((0, 10)), window, cx));
    cx.simulate_input("<!-- kept -->");
    choose(&main, cx, "Shop", "Add > New Solution Folder...");
    answer(&main, cx, "Tools");
    let text = on_disk(&solution);
    assert!(text.starts_with("<Solution><!-- kept -->\n"), "the unsaved edit would be lost:\n{text}");
    assert!(text.contains("<Folder Name=\"/Tools/\" />"), "the folder would not be added:\n{text}");
    let shown = main.read_with(cx, |main, cx| main.editor().map(|e| e.read(cx).text().to_string())).unwrap();
    assert_eq!(shown, text, "the open tab would not show what was written");
}

#[gpui::test]
fn a_folder_added_to_a_windows_1252_sln_keeps_every_other_byte_and_local_history_keeps_it_as_it_was(
    cx: &mut TestAppContext,
) {
    let sln = "Microsoft Visual Studio Solution File, Format Version 12.00\r\n# Caf\u{e9} \u{2013}\r\n\
        Project(\"{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}\") = \"Shop.Web\", \"src\\Shop.Web\\Shop.Web.csproj\", \"{A1000000-0000-0000-0000-000000000001}\"\r\nEndProject\r\n\
        Global\r\nEndGlobal\r\n";
    let (original, _, _) = encoding_rs::WINDOWS_1252.encode(sln);
    let run = Run::new("se-1252", &[("src/Shop.Web/Shop.Web.csproj", PROJECT)]);
    let solution = run.folder.join("Shop.sln");
    std::fs::write(&solution, &original).unwrap();
    let (main, cx) = open(&run, cx);
    choose(&main, cx, "Shop", "Add > New Solution Folder...");
    answer(&main, cx, "Tools");
    let written = std::fs::read(&solution).unwrap();
    let (text, _, _) = encoding_rs::WINDOWS_1252.decode(&written);
    assert!(text.contains("# Caf\u{e9} \u{2013}\r\n"), "the .sln's Windows-1252 comment would be rewritten:\n{text}");
    let tools =
        text.split('\n').find(|l| l.contains("\"Tools\", \"Tools\"")).expect("the folder was not added").to_string();
    assert_eq!(text.replace(&format!("{tools}\nEndProject\r\n"), ""), sln, "another byte of the .sln would change");
    let path = rsharp_core::paths::canonical(&solution);
    let kept = cx.update(|_, cx| {
        let history = rsharp_app::history_window::local_history(cx);
        history.revisions(&path).iter().any(|r| history.content(r).is_ok_and(|bytes| bytes == original.as_ref()))
    });
    assert!(kept, "Local History would not keep the .sln as it was before the folder went in");
}

#[gpui::test]
fn the_new_key_offers_the_add_items_and_the_rename_key_asks_the_name(cx: &mut TestAppContext) {
    let run = Run::new("se-keys", &files());
    let (main, cx) = open(&run, cx);
    let at = row(&main, cx, "src");
    main.update(cx, |main, cx| main.select_row(at, cx));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-n" } else { "alt-insert" });
    let shown = main.read_with(cx, |main, _| main.shown_menu());
    assert_eq!(
        shown,
        Some(vec!["New Solution Folder...".to_string(), "Existing Project...".into(), "Existing Item...".into()]),
        "the New key would not offer the row's Add items"
    );
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "shift-f6" } else { "f2" });
    let popup = main.read_with(cx, |main, _| main.name_popup()).expect("the rename key would not ask the name");
    assert_eq!(popup.read_with(cx, |popup, cx| popup.text(cx)), "src", "the popup would not start from the name");
}
