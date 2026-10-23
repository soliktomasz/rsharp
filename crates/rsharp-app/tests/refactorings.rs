//! The refactorings as a person runs them: Surround With and Complete Current Statement, which the editor makes
//! itself; and Optimize Usings, Encapsulate Field and Safe Delete, which Roslyn makes, against the real server
//! (skipped, saying why, when it is not installed).

mod support;

use gpui::{Entity, TestAppContext, VisualTestContext};
use rsharp_app::window::MainWindow;
use std::time::{Duration, Instant};
use support::Run;

fn key(mac: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") { mac } else { other }
}

fn text(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> String {
    main.read_with(cx, |m, cx| m.editor().map(|e| e.read(cx).text().to_string()).unwrap_or_default())
}

#[gpui::test]
fn surround_with_wraps_the_lines_in_the_construct_chosen_with_its_condition_selected(cx: &mut TestAppContext) {
    let run = Run::new("surround", &[("A.cs", "class A\n{\n    void M()\n    {\n        Work();\n    }\n}\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("A.cs"), Some((4, 8)), window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(key("cmd-alt-t", "ctrl-k ctrl-s"));
    cx.simulate_keystrokes("enter");
    cx.simulate_input("ready");
    cx.run_until_parked();
    assert_eq!(
        text(&main, cx),
        "class A\n{\n    void M()\n    {\n        if (ready)\n        {\n            Work();\n        }\n    }\n}\n",
        "Surround With did not wrap the line in `if` with its condition typed over the placeholder"
    );
}

#[gpui::test]
fn complete_current_statement_closes_the_line_and_goes_to_the_next(cx: &mut TestAppContext) {
    let run =
        Run::new("complete-statement", &[("A.cs", "class A\n{\n    void M()\n    {\n        var x = F(1\n    }\n}\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("A.cs"), Some((4, 12)), window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(key("cmd-shift-enter", "ctrl-shift-enter"));
    cx.simulate_input("Next();");
    cx.run_until_parked();
    assert_eq!(
        text(&main, cx),
        "class A\n{\n    void M()\n    {\n        var x = F(1);\n        Next();\n    }\n}\n",
        "Complete Current Statement did not close the call and the statement and go to the next line"
    );
}

const SHOP: &str = "using System.Text;\nusing System;\n\nnamespace Demo;\n\npublic class Shop\n{\n    private int count;\n\n    private int Unused() => 1;\n\n    private int Used() => 2;\n\n    public int Total(int price) => price + count + Used();\n\n    public void Log()\n    {\n        \n    }\n}\n";

/// Waits, in real time, for what Roslyn does on threads of its own; false after three minutes.
fn wait_for(cx: &mut VisualTestContext, until: impl Fn(&mut VisualTestContext) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        cx.executor().advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        if until(cx) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[gpui::test]
fn optimize_usings_encapsulate_field_and_safe_delete_change_the_code_as_roslyn_says(cx: &mut TestAppContext) {
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let project = r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>"#;
    let run = Run::new("refactor-roslyn", &[("Demo.csproj", project), ("Shop.cs", SHOP)]);
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Shop.cs"), Some((0, 0)), window, cx);
    });
    let said = |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.roslyn_said().join("\n"));
    if !wait_for(cx, |cx| main.read_with(cx, |m, _| m.server_ready())) {
        panic!("Roslyn never loaded the project; it said:\n{}", said(cx));
    }

    // Optimize Usings: the unused usings go. Roslyn may need a moment after its load to see them unused, so the key is
    // pressed again only once the last press is answered as not available.
    cx.simulate_keystrokes("ctrl-alt-o");
    let optimized = wait_for(cx, |cx| {
        if main.read_with(cx, |m, _| m.said() == Some("Optimize Usings is not available here")) {
            std::thread::sleep(Duration::from_millis(500));
            cx.simulate_keystrokes("ctrl-alt-o");
        }
        text(&main, cx).starts_with("namespace")
    });
    assert!(optimized, "Optimize Usings left {:?}; Roslyn said:\n{}", text(&main, cx).lines().next(), said(cx));

    // Safe Delete of a method nothing calls deletes it.
    let line = |cx: &mut VisualTestContext, of: &str| text(&main, cx).lines().position(|l| l.contains(of)).unwrap();
    let unused = line(cx, "Unused()");
    main.update_in(cx, |main, window, cx| main.open_file(&run.folder.join("Shop.cs"), Some((unused, 17)), window, cx));
    cx.simulate_keystrokes(key("cmd-delete", "alt-delete"));
    assert!(wait_for(cx, |cx| !text(&main, cx).contains("Unused")), "Safe Delete kept the method nothing uses");

    // Encapsulate Field makes the field a property and uses it.
    let field = line(cx, "private int count;");
    main.update_in(cx, |main, window, cx| main.open_file(&run.folder.join("Shop.cs"), Some((field, 17)), window, cx));
    cx.update(|window, cx| window.dispatch_action(Box::new(rsharp_app::editor::EncapsulateField), cx));
    // Two actions are offered (use the property, or still the field); the first, using it, is chosen.
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    let listed = wait_for(cx, |cx| editor.read_with(cx, |e, _| e.actions_shown()).is_some());
    assert!(listed, "Encapsulate Field offered nothing; Roslyn said:\n{}", said(cx));
    let first = editor.read_with(cx, |e, _| e.actions_shown()).unwrap_or_default();
    assert!(first[0].contains("and use property"), "Encapsulate Field listed {first:?}");
    cx.simulate_keystrokes("enter");
    let encapsulated = wait_for(cx, |cx| text(&main, cx).contains(" Count { get => count; set => count = value; }"));
    assert!(encapsulated, "Encapsulate Field made no property:\n{}", text(&main, cx));

    // Safe Delete from a use of a method still used: asked first; Cancel keeps it, Delete Anyway deletes it.
    let call = line(cx, "+ Used()");
    let column = text(&main, cx).lines().nth(call).unwrap().find("Used()").unwrap();
    for (answer, kept) in [("Cancel", true), ("Delete Anyway", false)] {
        main.update_in(cx, |main, window, cx| {
            main.open_file(&run.folder.join("Shop.cs"), Some((call, column)), window, cx)
        });
        cx.simulate_keystrokes(key("cmd-delete", "alt-delete"));
        assert!(wait_for(cx, |cx| cx.has_pending_prompt()), "Safe Delete did not ask before deleting what is used");
        cx.simulate_prompt_answer(answer);
        let done = wait_for(cx, |cx| {
            text(&main, cx).contains("int Used()") == kept
                && main.read_with(cx, |m, _| m.said().is_some_and(|s| s.contains("Used")))
        });
        assert!(done, "{answer} did not {} the method", if kept { "keep" } else { "delete" });
    }

    // Completion lists the live templates where a statement begins: cw written out as Console.WriteLine.
    let blank = line(cx, "void Log()") + 2;
    main.update_in(cx, |main, window, cx| main.open_file(&run.folder.join("Shop.cs"), Some((blank, 8)), window, cx));
    cx.simulate_input("c");
    cx.simulate_input("w");
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    let listed = wait_for(cx, |cx| {
        editor.read_with(cx, |e, _| e.completion_shown()).is_some_and(|l| l.first().is_some_and(|f| f == "cw"))
    });
    assert!(
        listed,
        "completion did not list the cw template first: {:?}",
        editor.read_with(cx, |e, _| e.completion_shown())
    );
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(
        text(&main, cx).contains("        Console.WriteLine();\n"),
        "the cw template was not written out:\n{}",
        text(&main, cx)
    );
}

#[gpui::test]
fn a_live_template_is_written_on_tab_and_tab_goes_through_its_variables(cx: &mut TestAppContext) {
    let run = Run::new("live-template", &[("A.cs", "class A\n{\n    void M()\n    {\n        \n    }\n}\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("A.cs"), Some((4, 8)), window, cx);
    });
    cx.run_until_parked();
    cx.simulate_input("for");
    cx.simulate_keystrokes("tab");
    // The index is selected at its three places: typing writes all three.
    cx.simulate_input("n");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("count");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("Work();");
    cx.run_until_parked();
    assert_eq!(
        text(&main, cx),
        "class A\n{\n    void M()\n    {\n        for (int n = 0; n < count; n++)\n        {\n            Work();\n        }\n    }\n}\n",
        "the `for` template was not written at its indent, its variables typed over, and its body reached"
    );
}
