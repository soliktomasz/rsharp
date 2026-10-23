//! The C# language service as a person uses it, against the real Roslyn on a fresh project: Go to Declaration into
//! another file, completion after a dot, Rename across files, and Find Usages. Skips, saying why, when Roslyn's
//! server is not installed.

mod support;

use gpui::{Entity, TestAppContext, VisualTestContext};
use rsharp_app::window::MainWindow;
use std::time::{Duration, Instant};
use support::Run;

const PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework><OutputType>Exe</OutputType></PropertyGroup></Project>"#;
const GREETER: &str =
    "namespace Demo;\n\npublic class Greeter\n{\n    public string Greet(string name) => \"Hello \" + name;\n}\n";
const PROGRAM: &str = "namespace Demo;\n\npublic static class Program\n{\n    public static void Main()\n    {\n        var greeter = new Greeter();\n        System.Console.WriteLine(greeter.Greet(\"you\"));\n    }\n}\n";

/// Waits, in real time, for what Roslyn does on threads of its own to reach the window; false after three minutes.
fn wait_for(cx: &mut VisualTestContext, until: impl Fn(&mut VisualTestContext) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        cx.run_until_parked();
        if until(cx) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn wait(cx: &mut VisualTestContext, what: &str, until: impl Fn(&mut VisualTestContext) -> bool) {
    assert!(wait_for(cx, until), "{what} never happened");
}

fn text(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> String {
    main.read_with(cx, |m, cx| m.editor().map(|e| e.read(cx).text().to_string()).unwrap_or_default())
}

#[gpui::test]
fn declaration_completion_rename_and_usages_work_against_roslyn(cx: &mut TestAppContext) {
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let run = Run::new("language", &[("Demo.csproj", PROJECT), ("Greeter.cs", GREETER), ("Program.cs", PROGRAM)]);
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    // The caret on `Greeter` in `new Greeter()`.
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Program.cs"), Some((6, 28)), window, cx);
    });
    let loaded =
        |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.server_ready() || m.server_failed().is_some());
    if !wait_for(cx, loaded) {
        panic!(
            "Roslyn never loaded the project; the status bar says {:?}",
            main.read_with(cx, |m, _| m.server_status())
        );
    }
    assert_eq!(main.read_with(cx, |m, _| m.server_failed().map(str::to_string)), None, "Roslyn failed");

    let declaration = if cfg!(target_os = "macos") { "cmd-b" } else { "f12" };
    cx.simulate_keystrokes(declaration);
    wait(cx, "Go to Declaration opening Greeter.cs", |cx| {
        main.read_with(cx, |m, cx| m.active_tab_name(cx).as_deref() == Some("Greeter.cs"))
    });
    let caret = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).caret());
    assert_eq!((caret.line, caret.column), (2, 13), "Go to Declaration did not land on the class's name");

    // Back in Program.cs, completion after `greeter.` offers Greet.
    main.update_in(cx, |main, window, cx| main.open_file(&run.folder.join("Program.cs"), Some((7, 41)), window, cx));
    cx.simulate_keystrokes("ctrl-space");
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    wait(cx, "completion after `greeter.`", |cx| editor.read_with(cx, |e, _| e.completion_shown().is_some()));
    cx.simulate_input("Gre");
    cx.run_until_parked();
    let shown = editor.read_with(cx, |e, _| e.completion_shown()).unwrap_or_default();
    assert_eq!(shown.first().map(String::as_str), Some("Greet"), "completion for `Gre` showed {shown:?}");
    cx.simulate_keystrokes("escape secondary-z");
    cx.run_until_parked();
    assert_eq!(text(&main, cx), PROGRAM, "Undo did not take back what was typed");

    // Rename of the method, from its call, changes both files.
    main.update_in(cx, |main, window, cx| main.open_file(&run.folder.join("Program.cs"), Some((7, 44)), window, cx));
    let rename = if cfg!(target_os = "macos") { "shift-f6" } else { "ctrl-r ctrl-r" };
    cx.simulate_keystrokes(rename);
    cx.run_until_parked();
    cx.simulate_input("Welcome");
    cx.simulate_keystrokes("enter");
    wait(cx, "Rename changing the call", |cx| text(&main, cx).contains("greeter.Welcome(\"you\")"));
    // Greeter.cs is open since Go to Declaration: its tab has the change, unsaved, as Rider leaves it.
    main.update_in(cx, |main, window, cx| main.open_file(&run.folder.join("Greeter.cs"), Some((2, 15)), window, cx));
    let greeter = text(&main, cx);
    assert!(greeter.contains("public string Welcome(string name)"), "Rename did not reach the other file:\n{greeter}");

    // Find Usages of the class lists its use in Program.cs.
    let usages = if cfg!(target_os = "macos") { "alt-f7" } else { "shift-f12" };
    cx.simulate_keystrokes(usages);
    wait(cx, "Find Usages listing", |cx| main.read_with(cx, |m, _| m.found().is_some()));
    assert_eq!(
        main.read_with(cx, |m, _| m.found()),
        Some(vec!["Program.cs:7".to_string()]),
        "Find Usages listed the wrong places"
    );
}
