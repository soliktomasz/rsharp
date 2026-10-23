//! The Solution view's file operations, as Rider's: New File, New Folder and a C# type where a row holds things,
//! Rename and Delete (to the Trash) of what is on disk, the open tabs following.

mod support;

use gpui::{Entity, TestAppContext, VisualContext, VisualTestContext};
use rsharp_app::window::MainWindow;
use std::path::PathBuf;
use support::Run;

const PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\" />\n";

fn files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Shop.slnx", "<Solution>\n  <Project Path=\"Shop.App/Shop.App.csproj\" />\n</Solution>\n"),
        ("Shop.App/Shop.App.csproj", PROJECT),
        ("Shop.App/Program.cs", "System.Console.WriteLine();\n"),
        ("Shop.App/Models/Order.cs", "namespace Shop.App.Models;\n\npublic class Order\n{\n    public int Total;\n}\n"),
    ]
}

fn open<'a>(run: &Run, cx: &'a mut TestAppContext) -> (Entity<MainWindow>, &'a mut VisualTestContext) {
    let (main, cx) = run.window(cx);
    let folder = run.folder.clone();
    main.update_in(cx, |main, window, cx| main.open_workspace(&folder, window, cx));
    cx.run_until_parked();
    for name in ["Shop.App", "Models"] {
        let at = row(&main, cx, name);
        main.update(cx, |main, cx| main.toggle_row(at, cx));
    }
    (main, cx)
}

fn rows(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> Vec<String> {
    main.read_with(cx, |main, _| main.solution_rows())
}

fn row(main: &Entity<MainWindow>, cx: &mut VisualTestContext, name: &str) -> usize {
    let rows = rows(main, cx);
    rows.iter().position(|r| r.trim_start() == name).unwrap_or_else(|| panic!("no row {name} in {rows:?}"))
}

fn choose(main: &Entity<MainWindow>, cx: &mut VisualTestContext, name: &str, item: &str) {
    let at = row(main, cx, name);
    main.update_in(cx, |main, window, cx| main.choose(at, item, window, cx));
    cx.run_until_parked();
}

fn answer(main: &Entity<MainWindow>, cx: &mut VisualTestContext, name: &str) {
    assert!(main.read_with(cx, |main, _| main.name_popup().is_some()), "no popup asked the name");
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-a" } else { "ctrl-a" });
    cx.simulate_input(name);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

fn active(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> Option<String> {
    main.read_with(cx, |main, cx| main.active_tab_name(cx))
}

fn text(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> String {
    main.read_with(cx, |m, cx| m.editor().map(|e| e.read(cx).text().to_string()).unwrap_or_default())
}

fn trash_for(run: &Run) -> PathBuf {
    let trash = run.folder.parent().unwrap().join("trash");
    rsharp_core::trash::set_folder(Some(trash.clone()));
    trash
}

#[gpui::test]
fn add_class_interface_writes_each_of_riders_kinds_of_type_in_its_folders_namespace(cx: &mut TestAppContext) {
    let run = Run::new("sf-types", &files());
    let models = run.folder.join("Shop.App/Models");
    let (main, cx) = open(&run, cx);
    let declarations = [
        "public class Invoice\n{\n}\n",
        "public interface IInvoice\n{\n}\n",
        "public record InvoiceLine();\n",
        "public struct Money\n{\n}\n",
        "public enum Status\n{\n}\n",
    ];
    let names = ["Invoice", "IInvoice", "InvoiceLine", "Money", "Status"];
    assert_eq!(rsharp_app::solution_menu::TYPE_KINDS, ["Class", "Interface", "Record", "Struct", "Enum"]);
    for (kind, name) in names.iter().enumerate() {
        choose(&main, cx, "Models", "Add > Class/Interface...");
        let popup = main.read_with(cx, |main, _| main.name_popup()).expect("no popup asked the type's name");
        assert_eq!(popup.read_with(cx, |p, _| p.kind()), Some("Class"), "the popup would not start at Class");
        for _ in 0..kind {
            cx.simulate_keystrokes("down");
        }
        assert_eq!(popup.read_with(cx, |p, _| p.kind()), Some(rsharp_app::solution_menu::TYPE_KINDS[kind]));
        cx.simulate_input(name);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        let written = models.join(format!("{name}.cs"));
        assert_eq!(
            std::fs::read_to_string(&written).unwrap_or_default(),
            format!("namespace Shop.App.Models;\n\n{}", declarations[kind]),
            "the {} would not be written as Rider writes one",
            rsharp_app::solution_menu::TYPE_KINDS[kind]
        );
        assert_eq!(active(&main, cx), Some(format!("{name}.cs")), "the new type's file would not open");
    }
}

#[gpui::test]
fn a_new_class_named_with_a_folder_goes_in_that_folder_in_its_namespace(cx: &mut TestAppContext) {
    let run = Run::new("sf-type-folder", &files());
    let (main, cx) = open(&run, cx);
    choose(&main, cx, "Shop.App", "Add > Class/Interface...");
    answer(&main, cx, "Domain/Sales/Invoice");
    let written = run.folder.join("Shop.App/Domain/Sales/Invoice.cs");
    assert_eq!(
        std::fs::read_to_string(written).unwrap_or_default(),
        "namespace Shop.App.Domain.Sales;\n\npublic class Invoice\n{\n}\n",
        "a type named with folders would not go in them, in their namespace"
    );
}

#[gpui::test]
fn new_file_and_new_folder_show_in_the_tree_and_a_new_file_opens(cx: &mut TestAppContext) {
    let run = Run::new("sf-new", &files());
    let (main, cx) = open(&run, cx);
    choose(&main, cx, "Shop.App", "Add > New Folder...");
    answer(&main, cx, "Drafts");
    assert!(run.folder.join("Shop.App/Drafts").is_dir(), "the folder would not be made");
    assert!(rows(&main, cx).contains(&"    Drafts".to_string()), "the folder would not show: {:?}", rows(&main, cx));
    choose(&main, cx, "Drafts", "Add > New File...");
    answer(&main, cx, "notes.md");
    assert!(run.folder.join("Shop.App/Drafts/notes.md").is_file(), "the file would not be made");
    assert_eq!(active(&main, cx).as_deref(), Some("notes.md"), "the new file would not open");
    assert!(rows(&main, cx).contains(&"      notes.md".to_string()), "the file would not show: {:?}", rows(&main, cx));
}

#[gpui::test]
fn a_name_taken_in_the_folder_is_refused_and_nothing_changes_on_disk(cx: &mut TestAppContext) {
    let run = Run::new("sf-taken", &files());
    let (main, cx) = open(&run, cx);
    for (name, item) in
        [("program.cs", "Add > New File..."), ("Models", "Add > New Folder..."), ("a:b", "Add > New File...")]
    {
        choose(&main, cx, "Shop.App", item);
        answer(&main, cx, name);
        let popup =
            main.read_with(cx, |main, _| main.name_popup()).unwrap_or_else(|| panic!("{name} closed the popup"));
        assert!(popup.read_with(cx, |p, _| p.problem().is_some()), "{name} would be refused without saying why");
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
    }
    assert_eq!(
        std::fs::read_to_string(run.folder.join("Shop.App/Program.cs")).unwrap(),
        "System.Console.WriteLine();\n"
    );
}

#[gpui::test]
fn a_file_renamed_keeps_its_tab_its_text_and_its_undo(cx: &mut TestAppContext) {
    let run = Run::new("sf-rename", &files());
    let order = run.folder.join("Shop.App/Models/Order.cs");
    let (main, cx) = open(&run, cx);
    main.update_in(cx, |main, window, cx| main.open_file(&order, Some((4, 21)), window, cx));
    cx.simulate_input(" // the sum");
    choose(&main, cx, "Order.cs", "Edit > Rename...");
    answer(&main, cx, "Purchase.cs");
    assert!(
        !order.exists() && run.folder.join("Shop.App/Models/Purchase.cs").is_file(),
        "the file would not be renamed"
    );
    assert_eq!(active(&main, cx).as_deref(), Some("Purchase.cs"), "the tab would not follow the file");
    assert!(text(&main, cx).contains("// the sum"), "the unsaved edit would be lost: {}", text(&main, cx));
    assert!(rows(&main, cx).contains(&"      Purchase.cs".to_string()), "the tree would not show the new name");
    let editor = main.read_with(cx, |m, _| m.editor()).unwrap();
    cx.focus(&editor);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-z" } else { "ctrl-z" });
    assert!(!text(&main, cx).contains("// the sum"), "undo would not take back the edit made before the rename");
}

#[gpui::test]
fn renaming_a_folder_with_two_open_files_keeps_the_one_in_front_in_front(cx: &mut TestAppContext) {
    let mut files = files();
    files.extend([("Shop.App/Models/Alpha.cs", "class Alpha { }\n"), ("Shop.App/Models/Beta.cs", "class Beta { }\n")]);
    let run = Run::new("sf-rename-folder", &files);
    let models = run.folder.join("Shop.App/Models");
    let (main, cx) = open(&run, cx);
    for file in ["Alpha.cs", "Beta.cs", "Alpha.cs"] {
        main.update_in(cx, |main, window, cx| main.open_file(&models.join(file), None, window, cx));
    }
    choose(&main, cx, "Models", "Edit > Rename...");
    answer(&main, cx, "Domain");
    let alpha = rsharp_core::paths::canonical(&run.folder.join("Shop.App/Domain/Alpha.cs"));
    let shown = main.read_with(cx, |m, cx| m.editor().map(|e| e.read(cx).path.clone()));
    assert_eq!(shown, Some(alpha), "the file in front would not stay in front under the folder's new name");
    assert_eq!(main.read_with(cx, |m, cx| m.groups(cx))[0].len(), 2, "a tab would be lost or doubled");
}

#[gpui::test]
fn a_file_deleted_with_unsaved_edits_goes_to_the_trash_with_them_and_local_history_keeps_them(cx: &mut TestAppContext) {
    let run = Run::new("sf-delete", &files());
    let trash = trash_for(&run);
    let order = run.folder.join("Shop.App/Models/Order.cs");
    let (main, cx) = open(&run, cx);
    main.update_in(cx, |main, window, cx| main.open_file(&order, Some((4, 21)), window, cx));
    cx.simulate_input(" // the sum");
    choose(&main, cx, "Order.cs", "Edit > Delete...");
    assert_eq!(
        main.read_with(cx, |m, _| m.asked().map(str::to_string)).as_deref(),
        Some("Move Order.cs to the Trash?")
    );
    cx.simulate_prompt_answer("Delete");
    cx.run_until_parked();
    assert!(!order.exists(), "the file would stay");
    let trashed = std::fs::read_to_string(trash.join("Order.cs")).unwrap_or_default();
    assert!(trashed.contains("// the sum"), "the Trash would not hold the unsaved edit: {trashed:?}");
    assert_eq!(active(&main, cx), None, "the deleted file's tab would stay open");
    let path = rsharp_core::paths::canonical(&order);
    let kept = cx.update(|_, cx| {
        let history = rsharp_app::history_window::local_history(cx);
        history
            .revisions(&path)
            .iter()
            .any(|r| history.content(r).is_ok_and(|b| String::from_utf8_lossy(&b).contains("// the sum")))
    });
    assert!(kept, "Local History would not keep the file as it was deleted");
    rsharp_core::trash::set_folder(None);
}

#[gpui::test]
fn a_delete_the_trash_refuses_keeps_the_open_file_and_its_edit(cx: &mut TestAppContext) {
    let run = Run::new("sf-refused", &files());
    // A file where the Trash's folder would be: the Trash cannot be made, so it refuses.
    let blocked = run.folder.parent().unwrap().join("blocked");
    std::fs::write(&blocked, "").unwrap();
    rsharp_core::trash::set_folder(Some(blocked.join("trash")));
    let order = run.folder.join("Shop.App/Models/Order.cs");
    let (main, cx) = open(&run, cx);
    main.update_in(cx, |main, window, cx| main.open_file(&order, Some((4, 21)), window, cx));
    cx.simulate_input(" // the sum");
    choose(&main, cx, "Order.cs", "Edit > Delete...");
    cx.simulate_prompt_answer("Delete");
    cx.run_until_parked();
    assert!(order.is_file(), "the file would go though the Trash refused it");
    assert_eq!(active(&main, cx).as_deref(), Some("Order.cs"), "its tab would close");
    assert!(text(&main, cx).contains("// the sum"), "its edit would be lost");
    assert!(main.read_with(cx, |m, _| m.said().is_some_and(|s| s.contains("Trash"))), "the refusal would not be said");
    rsharp_core::trash::set_folder(None);
}

/// Waits, in real time, for what Roslyn does on threads of its own; false after three minutes.
fn wait_for(cx: &mut VisualTestContext, until: impl Fn(&mut VisualTestContext) -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while std::time::Instant::now() < deadline {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        if until(cx) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

#[gpui::test]
fn a_file_renamed_after_its_type_offers_to_rename_the_type(cx: &mut TestAppContext) {
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let project = "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>";
    let run = Run::new(
        "sf-type-rename",
        &[
            ("Shop.slnx", "<Solution>\n  <Project Path=\"Shop.App/Shop.App.csproj\" />\n</Solution>\n"),
            ("Shop.App/Shop.App.csproj", project),
            ("Shop.App/Program.cs", "var order = new Shop.App.Order();\nSystem.Console.WriteLine(order);\n"),
            ("Shop.App/Order.cs", "namespace Shop.App;\n\npublic class Order\n{\n}\n"),
        ],
    );
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let folder = run.folder.clone();
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&folder, window, cx);
        main.open_file(&folder.join("Shop.App/Order.cs"), None, window, cx);
    });
    let loaded =
        |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.server_ready() || m.server_failed().is_some());
    assert!(wait_for(cx, loaded), "Roslyn never loaded the project");
    assert_eq!(main.read_with(cx, |m, _| m.server_failed().map(str::to_string)), None, "Roslyn failed");
    let at = row(&main, cx, "Shop.App (net10.0)");
    main.update(cx, |main, cx| main.toggle_row(at, cx));
    choose(&main, cx, "Order.cs", "Edit > Rename...");
    answer(&main, cx, "Purchase.cs");
    assert_eq!(
        main.read_with(cx, |m, _| m.asked().map(str::to_string)).as_deref(),
        Some("Rename the type Order to Purchase too?"),
        "a file named after its type would not offer the type's rename"
    );
    cx.simulate_prompt_answer("Rename Type");
    let program = run.folder.join("Shop.App/Program.cs");
    let renamed = |_: &mut VisualTestContext| {
        std::fs::read_to_string(&program).unwrap_or_default().contains("new Shop.App.Purchase()")
            || std::fs::read_to_string(run.folder.join("Shop.App/Purchase.cs")).is_ok()
    };
    assert!(wait_for(cx, renamed), "the type would not be renamed with its file");
    let text = main.read_with(cx, |m, cx| m.editor().map(|e| e.read(cx).text().to_string())).unwrap_or_default();
    assert!(text.contains("public class Purchase"), "the type's declaration would keep its old name: {text}");
    assert!(run.folder.join("Shop.App/Purchase.cs").is_file() && !run.folder.join("Shop.App/Order.cs").exists());
}

fn two_projects() -> Vec<(&'static str, &'static str)> {
    let mut files = files();
    files[0] = (
        "Shop.slnx",
        "<Solution>\n  <Project Path=\"Shop.App/Shop.App.csproj\" />\n  <Project Path=\"Shop.Core/Shop.Core.csproj\" />\n</Solution>\n",
    );
    files.extend([("Shop.Core/Shop.Core.csproj", PROJECT), ("Shop.Core/Catalog/Item.cs", "class Item { }\n")]);
    files
}

fn open_both<'a>(run: &Run, cx: &'a mut TestAppContext) -> (Entity<MainWindow>, &'a mut VisualTestContext) {
    let (main, cx) = open(run, cx);
    for name in ["Shop.Core", "Catalog"] {
        let at = row(&main, cx, name);
        main.update(cx, |main, cx| main.toggle_row(at, cx));
    }
    (main, cx)
}

#[gpui::test]
fn a_file_cut_and_pasted_into_another_projects_folder_moves_there_with_its_open_tab_and_its_unsaved_edit(
    cx: &mut TestAppContext,
) {
    let run = Run::new("sf-cut", &two_projects());
    let order = run.folder.join("Shop.App/Models/Order.cs");
    let (main, cx) = open_both(&run, cx);
    main.update_in(cx, |main, window, cx| main.open_file(&order, Some((4, 21)), window, cx));
    cx.simulate_input(" // the sum");
    choose(&main, cx, "Order.cs", "Edit > Cut");
    choose(&main, cx, "Catalog", "Edit > Paste");
    let moved = run.folder.join("Shop.Core/Catalog/Order.cs");
    assert!(!order.exists() && moved.is_file(), "the file would not move");
    let shown = main.read_with(cx, |m, cx| m.editor().map(|e| e.read(cx).path.clone()));
    assert_eq!(shown, Some(rsharp_core::paths::canonical(&moved)), "the tab would not follow the file");
    assert!(text(&main, cx).contains("// the sum"), "the unsaved edit would be lost");
    assert!(
        rows(&main, cx).contains(&"      Order.cs".to_string()),
        "the file would not show in its new folder: {:?}",
        rows(&main, cx)
    );
}

#[gpui::test]
fn a_folder_copied_and_pasted_copies_all_it_holds_and_a_copy_into_its_own_folder_asks_its_name(
    cx: &mut TestAppContext,
) {
    let run = Run::new("sf-copy", &two_projects());
    let (main, cx) = open_both(&run, cx);
    choose(&main, cx, "Models", "Edit > Copy");
    choose(&main, cx, "Shop.Core", "Edit > Paste");
    assert!(run.folder.join("Shop.Core/Models/Order.cs").is_file(), "the folder's file would not be copied");
    assert!(run.folder.join("Shop.App/Models/Order.cs").is_file(), "the copy would take the original");
    choose(&main, cx, "Shop.App", "Edit > Paste");
    let popup = main.read_with(cx, |m, _| m.name_popup()).expect("a copy into its own folder would not ask its name");
    assert_eq!(popup.read_with(cx, |p, cx| p.text(cx)), "Models");
    answer(&main, cx, "Models2");
    assert!(run.folder.join("Shop.App/Models2/Order.cs").is_file(), "the duplicate would not be made under its name");
}

#[gpui::test]
fn a_file_dragged_onto_another_folder_moves_and_with_the_copy_key_held_is_copied(cx: &mut TestAppContext) {
    let run = Run::new("sf-drag", &two_projects());
    let order = run.folder.join("Shop.App/Models/Order.cs");
    let (_main, cx) = open_both(&run, cx);
    let drag = |cx: &mut VisualTestContext, from: &str, to: &str, modifiers: gpui::Modifiers| {
        cx.run_until_parked();
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        let start = cx
            .debug_bounds(leak(format!("solution row: {from}")))
            .unwrap_or_else(|| panic!("no row {from} on screen"))
            .center();
        let end = cx
            .debug_bounds(leak(format!("solution row: {to}")))
            .unwrap_or_else(|| panic!("no row {to} on screen"))
            .center();
        cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
        for step in 1..=8 {
            cx.simulate_mouse_move(
                start + (end - start) * (step as f32 / 8.),
                Some(gpui::MouseButton::Left),
                modifiers,
            );
        }
        cx.simulate_mouse_up(end, gpui::MouseButton::Left, modifiers);
        cx.run_until_parked();
    };
    drag(cx, "Order.cs", "Catalog", gpui::Modifiers::default());
    let moved = run.folder.join("Shop.Core/Catalog/Order.cs");
    assert!(!order.exists() && moved.is_file(), "a file dragged onto a folder would not move there");
    let copy = if cfg!(target_os = "macos") { gpui::Modifiers::alt() } else { gpui::Modifiers::control() };
    drag(cx, "Item.cs", "Shop.App", copy);
    assert!(run.folder.join("Shop.App/Item.cs").is_file(), "a file dragged with the copy key would not be copied");
    assert!(run.folder.join("Shop.Core/Catalog/Item.cs").is_file(), "a copy by drag would take the original");
}

#[gpui::test]
fn a_parent_moved_takes_its_dependents_and_its_projects_dependent_upon_follows(cx: &mut TestAppContext) {
    let mut files = two_projects();
    files[1] = (
        "Shop.App/Shop.App.csproj",
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <ItemGroup>\n    <Compile Update=\"Form.Designer.cs\">\n      <DependentUpon>Form.cs</DependentUpon>\n    </Compile>\n  </ItemGroup>\n</Project>\n",
    );
    files.extend([
        ("Shop.App/Form.cs", "partial class Form { }\n"),
        ("Shop.App/Form.Designer.cs", "partial class Form { }\n"),
    ]);
    let run = Run::new("sf-dependents", &files);
    let (main, cx) = open_both(&run, cx);
    choose(&main, cx, "Form.cs", "Edit > Cut");
    choose(&main, cx, "Models", "Edit > Paste");
    assert!(run.folder.join("Shop.App/Models/Form.Designer.cs").is_file(), "a file's dependent would not go with it");
    let project = std::fs::read_to_string(run.folder.join("Shop.App/Shop.App.csproj")).unwrap();
    assert!(
        project.contains("<Compile Update=\"Models/Form.Designer.cs\">"),
        "the project would lose its item:\n{project}"
    );
}

#[gpui::test]
fn a_folder_pasted_into_itself_is_refused(cx: &mut TestAppContext) {
    let run = Run::new("sf-into-itself", &two_projects());
    let (main, cx) = open_both(&run, cx);
    choose(&main, cx, "Models", "Edit > Copy");
    choose(&main, cx, "Models", "Edit > Paste");
    assert!(
        main.read_with(cx, |m, _| m.said().is_some_and(|s| s.contains("inside itself"))),
        "the refusal would not be said"
    );
    assert!(!run.folder.join("Shop.App/Models/Models").exists(), "a folder would be copied into itself");
}

/// Waits in real time for the folder watcher, whose changes come on a thread of its own; false after ten seconds.
fn heard(cx: &mut VisualTestContext, until: impl Fn(&mut VisualTestContext) -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        cx.run_until_parked();
        if until(cx) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

/// What another program writes, once the watcher has had a moment to start.
fn elsewhere(path: &std::path::Path, text: &str) {
    std::thread::sleep(std::time::Duration::from_millis(300));
    std::fs::write(path, text).unwrap();
}

#[gpui::test]
fn a_file_made_by_another_program_shows_in_the_tree(cx: &mut TestAppContext) {
    let run = Run::new("sf-watch-made", &files());
    let (main, cx) = open(&run, cx);
    elsewhere(&run.folder.join("Shop.App/Models/Invoice.cs"), "class Invoice { }\n");
    assert!(
        heard(cx, |cx| rows(&main, cx).contains(&"      Invoice.cs".to_string())),
        "a file another program made would not show: {:?}",
        rows(&main, cx)
    );
}

#[gpui::test]
fn an_open_file_changed_by_another_program_shows_its_new_text_and_local_history_keeps_it(cx: &mut TestAppContext) {
    let run = Run::new("sf-watch-open", &files());
    let order = run.folder.join("Shop.App/Models/Order.cs");
    let (main, cx) = open(&run, cx);
    main.update_in(cx, |main, window, cx| main.open_file(&order, None, window, cx));
    cx.run_until_parked();
    elsewhere(&order, "class Order { int Pulled; }\n");
    assert!(
        heard(cx, |cx| text(&main, cx).contains("Pulled")),
        "the open tab would not show the new text: {}",
        text(&main, cx)
    );
    let path = rsharp_core::paths::canonical(&order);
    let kept = cx.update(|_, cx| {
        let history = rsharp_app::history_window::local_history(cx);
        history.revisions(&path).iter().any(|r| r.label.as_deref() == Some("External change"))
    });
    assert!(kept, "Local History would not keep the change as an External change");
}

#[gpui::test]
fn a_file_not_open_changed_by_another_program_is_kept_in_local_history(cx: &mut TestAppContext) {
    let run = Run::new("sf-watch-closed", &files());
    let program = run.folder.join("Shop.App/Program.cs");
    let (_main, cx) = open(&run, cx);
    elsewhere(&program, "System.Console.WriteLine(\"pulled\");\n");
    let path = rsharp_core::paths::canonical(&program);
    let kept = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let history = rsharp_app::history_window::local_history(cx);
            history
                .revisions(&path)
                .iter()
                .any(|r| history.content(r).is_ok_and(|b| String::from_utf8_lossy(&b).contains("pulled")))
        })
    };
    assert!(heard(cx, kept), "Local History would not keep a change made by another program");
}

#[gpui::test]
fn a_project_file_changed_by_another_program_is_evaluated_again(cx: &mut TestAppContext) {
    let run = Run::new("sf-watch-project", &files());
    if rsharp_lsp::SharedHost::carried(&run.folder).is_none() {
        eprintln!("skipped: Passenger's sidecars were not built (no dotnet or no vendor/passenger at build time)");
        return;
    }
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let folder = run.folder.clone();
    main.update_in(cx, |main, window, cx| main.open_workspace(&folder, window, cx));
    let evaluated = |cx: &mut VisualTestContext| !main.read_with(cx, |m, _| m.evaluating());
    assert!(heard(cx, evaluated), "the projects were never evaluated");
    elsewhere(
        &run.folder.join("Shop.App/Shop.App.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>\n",
    );
    let shown = |cx: &mut VisualTestContext| rows(&main, cx).iter().any(|r| r.contains("Shop.App (net10.0)"));
    assert!(
        heard(cx, shown),
        "a project file another program changed would not be evaluated again: {:?}",
        rows(&main, cx)
    );
}

#[gpui::test]
fn a_change_from_elsewhere_to_a_file_with_unsaved_edits_is_kept_in_local_history(cx: &mut TestAppContext) {
    let run = Run::new("sf-fix-unsaved", &files());
    let order = run.folder.join("Shop.App/Models/Order.cs");
    let (main, cx) = open(&run, cx);
    main.update_in(cx, |main, window, cx| main.open_file(&order, Some((4, 21)), window, cx));
    cx.simulate_input(" // mine");
    elsewhere(&order, "class Order { int Pulled; }\n");
    let path = rsharp_core::paths::canonical(&order);
    let kept = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let history = rsharp_app::history_window::local_history(cx);
            history
                .revisions(&path)
                .iter()
                .any(|r| history.content(r).is_ok_and(|b| String::from_utf8_lossy(&b).contains("Pulled")))
        })
    };
    assert!(heard(cx, kept), "a pulled change to a file with unsaved edits would be lost to the next save");
    assert!(text(&main, cx).contains("// mine"), "the unsaved edit would be lost");
}

#[gpui::test]
fn a_duplicate_whose_dependent_would_land_on_a_file_is_refused(cx: &mut TestAppContext) {
    let mut files = files();
    files.extend([
        ("Shop.App/Form1.cs", "partial class Form1 { }\n"),
        ("Shop.App/Form1.resx", "<root>one</root>\n"),
        ("Shop.App/Form2.resx", "<root>two</root>\n"),
    ]);
    let run = Run::new("sf-fix-dependent", &files);
    let (main, cx) = open(&run, cx);
    choose(&main, cx, "Form1.cs", "Edit > Copy");
    choose(&main, cx, "Shop.App", "Edit > Paste");
    answer(&main, cx, "Form2.cs");
    assert_eq!(
        std::fs::read_to_string(run.folder.join("Shop.App/Form2.resx")).unwrap(),
        "<root>two</root>\n",
        "a copy's dependent would overwrite a file already there"
    );
    assert!(!run.folder.join("Shop.App/Form2.cs").exists(), "a copy whose dependent is refused would go half way");
}

#[gpui::test]
fn a_solution_item_cannot_be_dragged_out_of_its_place(cx: &mut TestAppContext) {
    let mut files = files();
    files[0] = (
        "Shop.slnx",
        "<Solution>\n  <Folder Name=\"/Items/\">\n    <File Path=\"README.md\" />\n  </Folder>\n  <Project Path=\"Shop.App/Shop.App.csproj\" />\n</Solution>\n",
    );
    files.push(("README.md", "# Shop\n"));
    let run = Run::new("sf-fix-item-drag", &files);
    let (main, cx) = open(&run, cx);
    let at = row(&main, cx, "Items");
    main.update(cx, |m, cx| m.toggle_row(at, cx));
    cx.run_until_parked();
    let leak = |s: &str| -> &'static str { Box::leak(s.to_string().into_boxed_str()) };
    let start = cx.debug_bounds(leak("solution row: README.md")).expect("no README.md row").center();
    let end = cx.debug_bounds(leak("solution row: Models")).expect("no Models row").center();
    cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
    for step in 1..=8 {
        cx.simulate_mouse_move(
            start + (end - start) * (step as f32 / 8.),
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
    }
    cx.simulate_mouse_up(end, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(run.folder.join("README.md").is_file(), "a solution item would move, leaving the solution naming nothing");
}

#[gpui::test]
fn git_s_own_files_are_never_kept_in_local_history(cx: &mut TestAppContext) {
    let mut files = files();
    files.push((".git/HEAD", "ref: refs/heads/main\n"));
    let run = Run::new("sf-fix-git", &files);
    let (_main, cx) = open(&run, cx);
    elsewhere(&run.folder.join(".git/index"), "DIRC binary");
    std::fs::write(run.folder.join("Shop.App/Program.cs"), "System.Console.WriteLine(1);\n").unwrap();
    let program = rsharp_core::paths::canonical(&run.folder.join("Shop.App/Program.cs"));
    let seen = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| !rsharp_app::history_window::local_history(cx).revisions(&program).is_empty())
    };
    assert!(heard(cx, seen), "the change beside it would not be heard");
    let index = rsharp_core::paths::canonical(&run.folder.join(".git/index"));
    let kept = cx.update(|_, cx| rsharp_app::history_window::local_history(cx).revisions(&index).len());
    assert_eq!(kept, 0, "Git's index would fill Local History with every git command");
}

#[gpui::test]
fn a_file_renamed_by_letter_case_alone_is_renamed(cx: &mut TestAppContext) {
    let run = Run::new("sf-fix-case", &files());
    let (main, cx) = open(&run, cx);
    choose(&main, cx, "Order.cs", "Edit > Rename...");
    answer(&main, cx, "order.cs");
    assert!(main.read_with(cx, |m, _| m.name_popup().is_none()), "a case-only rename would be refused");
    let names: Vec<String> = std::fs::read_dir(run.folder.join("Shop.App/Models"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["order.cs"], "the file would keep its old letter case");
}
