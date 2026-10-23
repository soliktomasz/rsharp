mod support;
#[path = "support/templates.rs"]
mod templates;

use gpui::{Modifiers, TestAppContext, VisualTestContext, WindowHandle};
use rsharp_app::window::NewSolution;
use rsharp_app::{
    new_project_dialog::{self, NewProjectDialog},
    window::MainWindow,
};
use rsharp_core::templates::ProjectTarget;
use support::Run;

fn open(main: &mut VisualTestContext, run: &Run) -> (WindowHandle<NewProjectDialog>, VisualTestContext) {
    let owner = main.update(|window, _| window.window_handle().downcast::<MainWindow>().unwrap());
    let handle = main.update(|_, cx| new_project_dialog::open(owner, run.folder.clone(), None, cx).unwrap());
    handle.update(main, |d, _, cx| d.show_catalog(templates::fixture(), true, cx)).unwrap();
    let cx = VisualTestContext::from_window(handle.into(), main);
    cx.run_until_parked();
    (handle, cx)
}
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let b = cx.debug_bounds(selector).unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(b.center(), Modifiers::default());
    cx.run_until_parked();
}
#[gpui::test]
fn the_default_name_has_the_keys_on_open(cx: &mut TestAppContext) {
    let run = Run::new("new-project-default", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    cx.simulate_input("Greeter");
    cx.run_until_parked();
    let names = handle
        .read_with(&cx, |d, _| (d.draft().solution_name().to_string(), d.draft().project_name().to_string()))
        .unwrap();
    assert_eq!(names, ("Greeter".into(), "Greeter".into()));
    assert!(cx.debug_bounds("button: Create").is_some());
    assert!(cx.debug_bounds("button: Cancel").is_some());
}
#[gpui::test]
fn tab_and_arrows_reach_all_visible_fields(cx: &mut TestAppContext) {
    let run = Run::new("new-project-keys", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("Own.Project");
    cx.run_until_parked();
    assert_eq!(handle.read_with(&cx, |d, _| d.draft().project_name().to_string()).unwrap(), "Own.Project");
    click(&mut cx, "field: Search templates");
    cx.simulate_input("unit");
    cx.run_until_parked();
    assert_eq!(
        handle.read_with(&cx, |d, _| d.draft().families[d.draft().family.unwrap()].name.clone()).unwrap(),
        "Unit Test"
    );
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(
        handle.read_with(&cx, |d, _| d.draft().families[d.draft().family.unwrap()].name.clone()).unwrap(),
        "Aspire"
    );
}
#[gpui::test]
fn escape_clears_search_before_cancel(cx: &mut TestAppContext) {
    let run = Run::new("new-project-escape", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    click(&mut cx, "field: Search templates");
    cx.simulate_input("no matches");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(handle.read_with(&cx, |d, _| d.draft().query.is_empty()).unwrap());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(handle.read_with(&cx, |_, _| ()).is_err(), "Escape did not close the dialog");
}
#[gpui::test]
fn fields_follow_the_selected_type(cx: &mut TestAppContext) {
    let run = Run::new("new-project-types", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    click(&mut cx, "field: Search templates");
    cx.simulate_input("avalonia.xplat");
    cx.run_until_parked();
    assert!(!handle.read_with(&cx, |d, _| d.draft().asks_project_name()).unwrap());
    assert!(cx.debug_bounds("button: Install").is_some());
}
#[gpui::test]
fn errors_keep_inputs_and_actions_visible(cx: &mut TestAppContext) {
    let run = Run::new("new-project-errors", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_keystrokes("backspace");
    click(&mut cx, "button: Create");
    assert_eq!(
        handle.read_with(&cx, |d, _| d.problem().map(str::to_string)).unwrap().as_deref(),
        Some("Solution name cannot be empty")
    );
    assert!(cx.debug_bounds("field: Solution name").is_some());
    assert!(cx.debug_bounds("button: Cancel").is_some());
}
#[gpui::test]
fn closed_dialog_ignores_a_late_read(cx: &mut TestAppContext) {
    let run = Run::new("new-project-close", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    click(&mut cx, "button: Cancel");
    assert!(handle.update(&mut cx, |d, _, cx| d.show_catalog(templates::fixture(), true, cx)).is_err());
}

#[gpui::test]
fn choices_and_checkboxes_work_by_keyboard_without_stealing_field_spaces(cx: &mut TestAppContext) {
    let run = Run::new("new-project-controls", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    click(&mut cx, "choice: Framework");
    cx.simulate_keystrokes("down");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(handle.read_with(&cx, |d, _| d.draft().framework.clone()).unwrap().as_deref(), Some("net9.0"));
    click(&mut cx, "option: UseProgramMain");
    cx.simulate_keystrokes("space");
    cx.run_until_parked();
    assert_eq!(
        handle.read_with(&cx, |d, _| d.draft().option_value("UseProgramMain").map(str::to_string)).unwrap().as_deref(),
        Some("false")
    );
    click(&mut cx, "field: Solution name");
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("My Solution");
    cx.run_until_parked();
    assert_eq!(handle.read_with(&cx, |d, _| d.draft().solution_name().to_string()).unwrap(), "My Solution");
}

#[gpui::test]
fn a_small_window_keeps_the_footer_below_long_labels_and_errors(cx: &mut TestAppContext) {
    let run = Run::new("new-project-minimum", &[]);
    let (_, main) = run.window(cx);
    let (handle, mut cx) = open(main, &run);
    let mut catalog = (*templates::fixture()).clone();
    let console = &mut catalog.families.iter_mut().find(|f| f.name == "Console").unwrap().types[0];
    console.variants[0].options[0].label =
        "An unusually long localized template option that must wrap without hiding the buttons".repeat(3);
    handle.update(&mut cx, |d, _, cx| d.show_catalog(std::sync::Arc::new(catalog), true, cx)).unwrap();
    cx.simulate_resize(gpui::size(gpui::px(720.), gpui::px(480.)));
    cx.run_until_parked();
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_keystrokes("backspace");
    click(&mut cx, "button: Create");
    for selector in ["button: Create", "button: Cancel"] {
        let bounds = cx.debug_bounds(selector).unwrap();
        assert!(bounds.bottom() <= gpui::px(480.), "{selector} escaped the minimum window");
        assert!(bounds.left() >= gpui::px(0.) && bounds.right() <= gpui::px(720.));
    }
}

#[gpui::test]
fn welcome_file_find_action_and_widget_open_new_solution(cx: &mut TestAppContext) {
    let run = Run::new("new-solution-entry", &[("Shop.slnx", "<Solution />\n")]);
    let (main, cx) = run.window(cx);
    cx.run_until_parked();
    click(cx, "button: New Solution");
    let first = main.read_with(cx, |m, _| m.new_project_dialog()).expect("Welcome did not open New Solution");
    cx.dispatch_action(NewSolution);
    cx.run_until_parked();
    assert_eq!(
        main.read_with(cx, |m, _| m.new_project_dialog()),
        Some(first),
        "repeating the action opened another dialog"
    );
    first.update(cx, |_, w, _| w.remove_window()).unwrap();
    cx.simulate_keystrokes("secondary-shift-a");
    cx.simulate_input("new solution");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let second = main.read_with(cx, |m, _| m.new_project_dialog()).expect("Find Action did not open New Solution");
    assert_ne!(first, second);
    second.update(cx, |_, w, _| w.remove_window()).unwrap();
    main.update_in(cx, |m, w, cx| m.open_workspace(&run.folder, w, cx));
    cx.run_until_parked();
    click(cx, "project widget");
    click(cx, "project widget: New Solution");
    assert!(main.read_with(cx, |m, _| m.new_project_dialog()).is_some());
    assert!(rsharp_app::actions::ALL.iter().any(|a| a.title == "New Solution…" && a.menu == "File"));
}

#[gpui::test]
fn new_project_shortcut_and_add_menu_capture_the_solution_folder(cx: &mut TestAppContext) {
    let run = Run::new(
        "new-project-entry",
        &[("Shop.slnx", "<Solution>\n  <Folder Name=\"/src/tools/\" />\n</Solution>\n"), ("src/tools/.keep", "")],
    );
    let (main, cx) = run.window(cx);
    main.update_in(cx, |m, w, cx| m.open_workspace(&run.folder, w, cx));
    cx.run_until_parked();
    let src = main.read_with(cx, |m, _| m.solution_rows().iter().position(|s| s.trim() == "src").unwrap());
    main.update(cx, |m, cx| m.toggle_row(src, cx));
    cx.run_until_parked();
    let at = main.read_with(cx, |m, _| m.solution_rows().iter().position(|s| s.trim() == "tools").unwrap());
    main.update_in(cx, |m, w, cx| m.choose(at, "Add > New Project...", w, cx));
    cx.run_until_parked();
    let handle = main.read_with(cx, |m, _| m.new_project_dialog()).unwrap();
    let target = handle.read_with(cx, |d, _| (d.draft().directory.clone(), d.draft().target.clone().unwrap())).unwrap();
    assert_eq!(target.0, run.folder.join("src/tools"));
    assert_eq!(target.1.folder.as_deref(), Some("/src/tools/"));
    handle.update(cx, |_, w, _| w.remove_window()).unwrap();
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-shift-n" } else { "ctrl-shift-n" });
    cx.run_until_parked();
    assert!(main.read_with(cx, |m, _| m.new_project_dialog()).is_some(), "New Project shortcut did not open a dialog");
}

#[gpui::test]
fn a_multi_project_attachment_is_one_preserving_write(cx: &mut TestAppContext) {
    let before = "\u{feff}<Solution>\r\n  <!-- keep exactly -->\r\n  <Folder Name=\"/src/\" />\r\n</Solution>\r\n";
    let run = Run::new(
        "new-project-batch",
        &[("Shop.slnx", before), ("A/A.csproj", "<Project />\n"), ("B/B.csproj", "<Project />\n")],
    );
    let (main, cx) = run.window(cx);
    main.update_in(cx, |m, w, cx| m.open_workspace(&run.folder, w, cx));
    cx.run_until_parked();
    let target =
        ProjectTarget { solution: run.folder.join("Shop.slnx"), folder: Some("/src/".into()), projects: vec![] };
    main.update(cx, |m, cx| {
        m.attach_generated(
            Some(target.clone()),
            None,
            vec![run.folder.join("A/A.csproj"), run.folder.join("B/B.csproj")],
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    let after = std::fs::read_to_string(run.folder.join("Shop.slnx")).unwrap();
    assert!(after.contains("<!-- keep exactly -->\r\n"));
    assert!(after.starts_with('\u{feff}'));
    assert_eq!(main.read_with(cx, |m, _| m.selected_row()).as_deref(), Some("    B"));
    let saved = after.clone();
    std::fs::write(run.folder.join("C.csproj"), "<Project />").unwrap();
    assert!(
        main.update(cx, |m, cx| m.attach_generated(
            Some(target),
            None,
            vec![run.folder.join("C.csproj"), run.folder.join("missing.txt")],
            cx
        ))
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(run.folder.join("Shop.slnx")).unwrap(),
        saved,
        "a failed member left a partial attachment"
    );
}

#[gpui::test]
fn a_changed_owner_keeps_output_without_attachment(cx: &mut TestAppContext) {
    let run = Run::new(
        "new-project-owner",
        &[("One.slnx", "<Solution />\n"), ("two/Two.slnx", "<Solution />\n"), ("Made/Made.csproj", "<Project />\n")],
    );
    let (main, cx) = run.window(cx);
    main.update_in(cx, |m, w, cx| m.open_workspace(&run.folder.join("two/Two.slnx"), w, cx));
    cx.run_until_parked();
    let target = ProjectTarget { solution: run.folder.join("One.slnx"), folder: None, projects: vec![] };
    assert!(
        main.update(cx, |m, cx| m.attach_generated(Some(target), None, vec![run.folder.join("Made/Made.csproj")], cx))
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(run.folder.join("two/Two.slnx")).unwrap(), "<Solution />\n");
    assert!(run.folder.join("Made/Made.csproj").exists());
}

#[gpui::test]
fn a_new_solution_uses_welcome_or_another_window(cx: &mut TestAppContext) {
    let run = Run::new(
        "new-solution-open",
        &[("One.slnx", "<Solution />\n"), ("new/Two.slnx", "<Solution />\n"), ("new/App/App.csproj", "<Project />\n")],
    );
    let (main, cx) = run.window(cx);
    main.update(cx, |m, cx| m.attach_generated(None, Some(run.folder.join("One.slnx")), vec![], cx)).unwrap();
    cx.run_until_parked();
    assert_eq!(
        main.read_with(cx, |m, _| m.opened_as().map(std::path::Path::to_path_buf)),
        Some(run.folder.join("One.slnx"))
    );
    main.update(cx, |m, cx| {
        m.attach_generated(None, Some(run.folder.join("new/Two.slnx")), vec![run.folder.join("new/App/App.csproj")], cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| rsharp_app::projects::windows(cx).len()), 2);
    assert_eq!(
        main.read_with(cx, |m, _| m.opened_as().map(std::path::Path::to_path_buf)),
        Some(run.folder.join("One.slnx"))
    );
    assert!(std::fs::read_to_string(run.folder.join("new/Two.slnx")).unwrap().contains("App/App.csproj"));
    let recent = cx.update(|_, cx| {
        rsharp_app::state::store(cx).read::<rsharp_app::state::RecentSolutions>().entries[0].path.clone()
    });
    assert_eq!(recent, run.folder.join("new/Two.slnx"));
}

#[gpui::test]
fn new_projects_save_the_open_solution_then_add_only_their_lines(cx: &mut TestAppContext) {
    let before = "\u{feff}Microsoft Visual Studio Solution File, Format Version 12.00\r\n# keep exactly\r\nGlobal\r\nEndGlobal\r\n";
    let run = Run::new("new-project-open-buffer", &[("Shop.sln", before), ("Data/Data.csproj", "<Project />\n")]);
    let (main, cx) = run.window(cx);
    let solution = run.folder.join("Shop.sln");
    main.update_in(cx, |m, w, cx| {
        m.open_workspace(&solution, w, cx);
        m.open_file(&solution, None, w, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-down" } else { "ctrl-end" });
    cx.simulate_input("# unsaved comment\n");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&solution).unwrap(), before, "the buffer was saved before attachment");
    let target = ProjectTarget { solution: solution.clone(), folder: None, projects: vec![] };
    main.update(cx, |m, cx| m.attach_generated(Some(target), None, vec![run.folder.join("Data/Data.csproj")], cx))
        .unwrap();
    cx.run_until_parked();
    let bytes = std::fs::read(&solution).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(
        text.starts_with("\u{feff}Microsoft Visual Studio Solution File, Format Version 12.00\r\n# keep exactly\r\n")
    );
    assert!(text.ends_with("EndGlobal\r\n# unsaved comment\r\n"));
    assert!(text.contains("Data\\Data.csproj"));
    let saved = format!("{before}# unsaved comment\r\n").into_bytes();
    let kept = cx.update(|_, cx| {
        let history = rsharp_app::history_window::local_history(cx);
        history.revisions(&solution).iter().any(|r| history.content(r).is_ok_and(|b| b == saved))
    });
    assert!(kept, "Local History lost the saved open solution before its project was added");
}

#[gpui::test]
fn a_deleted_logical_folder_refuses_attachment_and_missing_physical_folder_uses_root(cx: &mut TestAppContext) {
    let run = Run::new(
        "new-project-folder-gone",
        &[
            ("Shop.slnx", "<Solution>\n  <Folder Name=\"/src/\" />\n</Solution>\n"),
            ("Made/Made.csproj", "<Project />\n"),
        ],
    );
    let (main, cx) = run.window(cx);
    main.update_in(cx, |m, w, cx| {
        m.open_workspace(&run.folder, w, cx);
        m.show_new_project(Some("/src/".into()), w, cx);
    });
    cx.run_until_parked();
    let dialog = main.read_with(cx, |m, _| m.new_project_dialog()).unwrap();
    assert_eq!(dialog.read_with(cx, |d, _| d.draft().directory.clone()).unwrap(), run.folder);
    std::fs::write(run.folder.join("Shop.slnx"), "<Solution />\n").unwrap();
    let target =
        ProjectTarget { solution: run.folder.join("Shop.slnx"), folder: Some("/src/".into()), projects: vec![] };
    assert!(
        main.update(cx, |m, cx| m.attach_generated(Some(target), None, vec![run.folder.join("Made/Made.csproj")], cx))
            .is_err()
    );
    assert!(run.folder.join("Made/Made.csproj").exists());
}
