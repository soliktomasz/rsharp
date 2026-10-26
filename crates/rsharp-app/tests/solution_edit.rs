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
        // Whether the test's own folder sits in a worktree is the system's; the Git submenu is not what this test
        // is about, so it is left out of the comparison.
        main.read_with(cx, |main, _| {
            main.solution_menu(at).into_iter().filter(|item| !item.starts_with("Git >")).collect::<Vec<_>>()
        })
    };
    assert_eq!(
        menu(cx, "Shop"),
        [
            "Build".to_string(),
            "Rebuild".into(),
            "Clean".into(),
            format!("Add > New Project... ({})", key("⇧⌘N", "Ctrl+Shift+N")),
            "Add > New Solution Folder...".into(),
            "Add > Existing Project...".into(),
            "Copy Path".into(),
            "Open In > Terminal".into(),
        ]
    );
    assert_eq!(
        menu(cx, "src"),
        [
            "Build".to_string(),
            "Rebuild".into(),
            "Clean".into(),
            format!("Add > New Project... ({})", key("⇧⌘N", "Ctrl+Shift+N")),
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
            "Build".to_string(),
            "Rebuild".into(),
            "Clean".into(),
            "Add > New File...".to_string(),
            "Add > New Folder...".into(),
            "Add > Class/Interface...".into(),
            "Add > Reference...".into(),
            format!("Edit > Paste ({paste})"),
            format!("Edit > Remove... ({delete})"),
            "Copy Path".into(),
            "Open In > Terminal".into(),
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
            "Open In > Terminal".into(),
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
            "Open In > Terminal".into(),
        ],
        "a file's menu would not be Rider's"
    );
    assert_eq!(
        menu(cx, "README.md"),
        [format!("Edit > Remove... ({delete})"), "Copy Path".into(), "Open In > Terminal".into()]
    );
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
        Some(vec![
            format!("New Project... ({})", key("⇧⌘N", "Ctrl+Shift+N")),
            "New Solution Folder...".to_string(),
            "Existing Project...".into(),
            "Existing Item...".into()
        ]),
        "the New key would not offer the row's Add items"
    );
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "shift-f6" } else { "f2" });
    let popup = main.read_with(cx, |main, _| main.name_popup()).expect("the rename key would not ask the name");
    assert_eq!(popup.read_with(cx, |popup, cx| popup.text(cx)), "src", "the popup would not start from the name");
}

#[gpui::test]
fn an_existing_project_added_to_a_folder_shows_there_and_in_the_slnx(cx: &mut TestAppContext) {
    let mut files = files();
    files.push(("tools/Importer/Importer.csproj", PROJECT));
    let run = Run::new("se-existing-project", &files);
    let solution = run.folder.join("Shop.slnx");
    let (main, cx) = open(&run, cx);
    let importer = run.folder.join("tools/Importer/Importer.csproj");
    let added = main.update(cx, |main, cx| main.add_existing(Some("/src/".into()), vec![importer], true, cx));
    assert_eq!(added, Ok(()), "the project would not be added");
    assert!(
        on_disk(&solution).contains("  <Folder Name=\"/src/\">\n    <Project Path=\"src/Shop.Core/Shop.Core.csproj\" />\n    <Project Path=\"tools/Importer/Importer.csproj\" />\n  </Folder>\n"),
        "the project would not be written into its folder:\n{}",
        on_disk(&solution)
    );
    assert!(
        rows(&main, cx).contains(&"    Importer".to_string()),
        "the project would not show in its folder: {:?}",
        rows(&main, cx)
    );
    assert_eq!(main.read_with(cx, |main, _| main.selected_row()).as_deref(), Some("    Importer"));
}

#[gpui::test]
fn an_existing_item_added_to_a_solution_folder_shows_there(cx: &mut TestAppContext) {
    let mut files = files();
    files.push(("docs/notes.md", "# Notes\n"));
    let run = Run::new("se-existing-item", &files);
    let solution = run.folder.join("Shop.slnx");
    let (main, cx) = open(&run, cx);
    let notes = run.folder.join("docs/notes.md");
    let added = main.update(cx, |main, cx| main.add_existing(Some("/Solution Items/".into()), vec![notes], false, cx));
    assert_eq!(added, Ok(()));
    assert!(
        on_disk(&solution).contains("    <File Path=\"README.md\" />\n    <File Path=\"docs/notes.md\" />\n"),
        "{}",
        on_disk(&solution)
    );
    assert!(
        rows(&main, cx).contains(&"    notes.md".to_string()),
        "the file would not show in its folder: {:?}",
        rows(&main, cx)
    );
}

fn trash_for(run: &Run) -> std::path::PathBuf {
    let trash = run.folder.parent().unwrap().join("trash");
    rsharp_core::trash::set_folder(Some(trash.clone()));
    trash
}

#[gpui::test]
fn delete_on_a_project_beside_its_solution_takes_it_out_and_leaves_the_folder(cx: &mut TestAppContext) {
    let run = Run::new(
        "se-remove-beside",
        &[
            ("Shop.slnx", "<Solution>\n  <Project Path=\"Shop.csproj\" />\n</Solution>\n"),
            ("Shop.csproj", PROJECT),
            ("Models/Order.cs", "class Order { }\n"),
        ],
    );
    let trash = trash_for(&run);
    let (main, cx) = open(&run, cx);
    // The solution is named Shop too: its project is the row under it.
    let project = rows(&main, cx).iter().position(|r| r == "  Shop").expect("no project row");
    main.update_in(cx, |main, window, cx| main.choose(project, "Edit > Remove...", window, cx));
    cx.run_until_parked();
    assert_eq!(
        main.read_with(cx, |m, _| m.asked().map(str::to_string)).as_deref(),
        Some("Remove Shop from the solution? Its folder and files stay on disk.")
    );
    cx.simulate_prompt_answer("Remove");
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt(), "a project beside its solution would be offered to the Trash");
    assert_eq!(
        on_disk(&run.folder.join("Shop.slnx")),
        "<Solution>\n</Solution>\n",
        "the project would stay in the solution"
    );
    assert!(
        run.folder.join("Shop.csproj").is_file() && run.folder.join("Models").is_dir(),
        "the project's files would go"
    );
    assert!(!trash.exists(), "something would go to the Trash");
    rsharp_core::trash::set_folder(None);
}

#[gpui::test]
fn delete_on_a_project_in_its_own_folder_moves_that_folder_to_the_trash_only_once_asked_again(cx: &mut TestAppContext) {
    let run = Run::new(
        "se-remove-own",
        &[
            (
                "Shop.slnx",
                "<Solution>\n  <Project Path=\"Shop.csproj\" />\n  <Project Path=\"Lib/Lib.csproj\" />\n</Solution>\n",
            ),
            ("Shop.csproj", PROJECT),
            ("Lib/Lib.csproj", PROJECT),
            ("Lib/Thing.cs", "class Thing { }\n"),
        ],
    );
    let trash = trash_for(&run);
    let (main, cx) = open(&run, cx);
    choose(&main, cx, "Lib", "Edit > Remove...");
    cx.simulate_prompt_answer("Remove");
    cx.run_until_parked();
    assert_eq!(
        main.read_with(cx, |m, _| m.asked().map(str::to_string)).as_deref(),
        Some("Also move the folder Lib and everything in it to the Trash?"),
        "a project in its own folder would not be offered to the Trash"
    );
    cx.simulate_prompt_answer("Move to Trash");
    cx.run_until_parked();
    assert!(!run.folder.join("Lib").exists(), "the project's folder would stay");
    assert!(trash.join("Lib/Lib.csproj").is_file(), "the project's folder would not be in the Trash");
    assert_eq!(on_disk(&run.folder.join("Shop.slnx")), "<Solution>\n  <Project Path=\"Shop.csproj\" />\n</Solution>\n");
    rsharp_core::trash::set_folder(None);
}

const EXECUTABLE: &str = "\u{feff}<Project Sdk=\"Microsoft.NET.Sdk\">\r\n\r\n  <PropertyGroup>\r\n    <OutputType>Exe</OutputType>\r\n    <TargetFramework>net10.0</TargetFramework>\r\n  </PropertyGroup>\r\n\r\n</Project>\r\n";

fn referencing() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "Shop.slnx",
            "<Solution>\n  <Project Path=\"Shop.App/Shop.App.csproj\" />\n  <Project Path=\"Shop.Core/Shop.Core.csproj\" />\n</Solution>\n",
        ),
        ("Shop.App/Shop.App.csproj", EXECUTABLE),
        ("Shop.Core/Shop.Core.csproj", PROJECT),
    ]
}

#[gpui::test]
fn add_reference_adds_one_and_remove_takes_it_out_leaving_the_project_file_as_it_was(cx: &mut TestAppContext) {
    let run = Run::new("se-reference", &referencing());
    let (app, core) = (run.folder.join("Shop.App/Shop.App.csproj"), run.folder.join("Shop.Core/Shop.Core.csproj"));
    let original = std::fs::read(&app).unwrap();
    let (main, cx) = open(&run, cx);
    choose(&main, cx, "Shop.App", "Add > Reference...");
    let dialog = main.read_with(cx, |m, _| m.reference_dialog()).expect("Add Reference would open no dialog");
    let offered = dialog.read_with(cx, |d, _| d.choices());
    assert_eq!(
        offered,
        [("Shop.Core".to_string(), true, false)],
        "the dialog would not offer the other project, unticked"
    );
    dialog.update(cx, |d, cx| d.toggle(0, cx));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(main.read_with(cx, |m, _| m.reference_dialog().is_none()), "OK would leave the dialog open");
    assert_eq!(
        on_disk(&app),
        "\u{feff}<Project Sdk=\"Microsoft.NET.Sdk\">\r\n\r\n  <PropertyGroup>\r\n    <OutputType>Exe</OutputType>\r\n    <TargetFramework>net10.0</TargetFramework>\r\n  </PropertyGroup>\r\n\r\n  <ItemGroup>\r\n    <ProjectReference Include=\"..\\Shop.Core\\Shop.Core.csproj\" />\r\n  </ItemGroup>\r\n\r\n</Project>\r\n",
        "the reference would not be written as Rider writes it"
    );
    // Its Dependencies list it once the project is evaluated, which with servers off a test gives the window itself.
    let evaluated = rsharp_core::project::EvaluatedProject {
        path: rsharp_core::paths::canonical(&app),
        complete: true,
        target_frameworks: vec!["net10.0".into()],
        dependencies: vec![rsharp_core::project::FrameworkDependencies {
            projects: vec![rsharp_core::paths::canonical(&core)],
            ..Default::default()
        }],
        ..Default::default()
    };
    main.update(cx, |m, cx| m.set_evaluated(evaluated, cx));
    open_row(&main, cx, "Shop.App (net10.0)");
    open_row(&main, cx, "Dependencies");
    choose(&main, cx, "Shop.Core", "Remove...");
    assert_eq!(
        main.read_with(cx, |m, _| m.asked().map(str::to_string)).as_deref(),
        Some("Remove the reference to Shop.Core from Shop.App?")
    );
    cx.simulate_prompt_answer("Remove");
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(&app).unwrap(),
        original,
        "with the reference removed, the project file would not be as it was"
    );
}

#[gpui::test]
fn a_project_that_depends_on_this_one_cannot_be_ticked(cx: &mut TestAppContext) {
    let mut files = referencing();
    files[2] = (
        "Shop.Core/Shop.Core.csproj",
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <ItemGroup>\n    <ProjectReference Include=\"..\\Shop.App\\Shop.App.csproj\" />\n  </ItemGroup>\n</Project>\n",
    );
    let run = Run::new("se-reference-cycle", &files);
    let (main, cx) = open(&run, cx);
    choose(&main, cx, "Shop.App", "Add > Reference...");
    let dialog = main.read_with(cx, |m, _| m.reference_dialog()).expect("no dialog");
    assert_eq!(
        dialog.read_with(cx, |d, _| d.choices()),
        [("Shop.Core".to_string(), false, false)],
        "a project that references this one would be offered, which would make a cycle"
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(main.read_with(cx, |m, _| m.reference_dialog().is_none()), "Escape would leave the dialog open");
}

#[gpui::test]
fn add_reference_to_a_windows_1252_project_keeps_its_other_bytes_and_local_history_keeps_it_as_it_was(
    cx: &mut TestAppContext,
) {
    let mut files = referencing();
    files.remove(1);
    let run = Run::new("se-reference-1252", &files);
    let (app, core) = (run.folder.join("Shop.App/Shop.App.csproj"), run.folder.join("Shop.Core/Shop.Core.csproj"));
    let text = "<Project Sdk=\"Microsoft.NET.Sdk\">\r\n  <!-- Caf\u{e9} \u{2013} -->\r\n  <PropertyGroup>\r\n    <OutputType>Exe</OutputType>\r\n  </PropertyGroup>\r\n</Project>\r\n";
    let original = encoding_rs::WINDOWS_1252.encode(text).0.into_owned();
    std::fs::create_dir_all(app.parent().unwrap()).unwrap();
    std::fs::write(&app, &original).unwrap();
    let (main, cx) = open(&run, cx);
    let set = main.update(cx, |m, cx| m.set_references(&app, vec![core.clone()], cx));
    assert_eq!(set, Ok(()));
    cx.run_until_parked();
    let bytes = std::fs::read(&app).unwrap();
    let (written, _, _) = encoding_rs::WINDOWS_1252.decode(&bytes);
    let without = written.replace(
        "  <ItemGroup>\r\n    <ProjectReference Include=\"..\\Shop.Core\\Shop.Core.csproj\" />\r\n  </ItemGroup>\r\n",
        "",
    );
    assert_eq!(without, text, "another byte of the Windows-1252 project would change");
    let path = rsharp_core::paths::canonical(&app);
    let kept = cx.update(|_, cx| {
        let history = rsharp_app::history_window::local_history(cx);
        history.revisions(&path).iter().any(|r| history.content(r).is_ok_and(|b| b == original))
    });
    assert!(kept, "Local History would not keep the project as it was");
}

#[gpui::test]
fn local_history_lists_what_rsharp_wrote_as_the_newest(cx: &mut TestAppContext) {
    let run = Run::new("se-history-order", &files());
    let solution = run.folder.join("Shop.slnx");
    let (main, cx) = open(&run, cx);
    for name in ["One", "Two", "Three"] {
        choose(&main, cx, "Shop", "Add > New Solution Folder...");
        answer(&main, cx, name);
    }
    let path = rsharp_core::paths::canonical(&solution);
    let newest = cx.update(|_, cx| {
        let history = rsharp_app::history_window::local_history(cx);
        history.revisions(&path).last().and_then(|r| history.content(r).ok())
    });
    assert_eq!(
        newest,
        Some(std::fs::read(&solution).unwrap()),
        "Local History's newest would not be the file as written"
    );
}
