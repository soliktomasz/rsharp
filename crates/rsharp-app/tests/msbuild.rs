//! Code help in a project file as a person uses it, against the real Host on a fresh SDK project: completion inside a
//! PropertyGroup and after `$(`, the hover card's evaluated value, Go to Declaration of a property to where it is set,
//! and the evaluation's error as the file's problem. Skips, saying why, without Passenger's sidecars.

mod support;

use gpui::{Entity, TestAppContext, VisualTestContext};
use rsharp_app::window::MainWindow;
use std::time::{Duration, Instant};
use support::Run;

const PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <TargetFramework>net10.0</TargetFramework>\n    <Greeting>hello</Greeting>\n    <Shown>$(Greeting)</Shown>\n  </PropertyGroup>\n</Project>\n";

fn wait(cx: &mut VisualTestContext, what: &str, done: impl Fn(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !done(cx) {
        assert!(Instant::now() < deadline, "{what} never happened");
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn evaluated(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> bool {
    main.read_with(cx, |m, cx| m.editor().is_some_and(|e| e.read(cx).buffer.read(cx).insight.is_some()))
}

#[gpui::test]
fn a_project_file_completes_shows_and_goes_to_what_its_evaluation_knows(cx: &mut TestAppContext) {
    if rsharp_lsp::SharedHost::carried(std::path::Path::new(".")).is_none() {
        eprintln!("skipped: Passenger's sidecars were not built (no dotnet or no vendor/passenger at build time)");
        return;
    }
    let run = Run::new("msbuild-help", &[("Demo.csproj", PROJECT), ("Program.cs", "System.Console.WriteLine();\n")]);
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |m, window, cx| {
        m.open_workspace(&run.folder, window, cx);
        m.open_file(&run.folder.join("Demo.csproj"), Some((4, 20)), window, cx);
    });
    wait(cx, "the project's evaluation", |cx| evaluated(&main, cx));
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());

    // The hover card on $(Greeting) names its value and where it is set.
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "f1" } else { "ctrl-shift-f1" });
    wait(cx, "the hover card", |cx| editor.read_with(cx, |e, _| e.popup_text().is_some()));
    let card = editor.read_with(cx, |e, _| e.popup_text()).unwrap();
    assert!(card.contains("Greeting = hello"), "the card did not give the evaluated value: {card}");
    cx.simulate_keystrokes("escape");

    // Go to Declaration lands on the element that sets it.
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-b" } else { "f12" });
    cx.run_until_parked();
    let caret = editor.read_with(cx, |e, _| e.caret());
    assert_eq!((caret.line, caret.column), (3, 4), "Go to Declaration did not land where Greeting is set");

    // A new element inside the PropertyGroup offers the schema's properties and the project's own.
    main.update_in(cx, |m, window, cx| m.open_file(&run.folder.join("Demo.csproj"), Some((4, 35)), window, cx));
    cx.simulate_keystrokes("enter");
    cx.simulate_input("<");
    cx.simulate_input("Nulla");
    cx.run_until_parked();
    let shown = editor.read_with(cx, |e, _| e.completion_shown()).unwrap_or_default();
    assert_eq!(shown.first().map(String::as_str), Some("Nullable"), "completion inside PropertyGroup: {shown:?}");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let line = editor.read_with(cx, |e, _| e.text().to_string()).lines().nth(5).unwrap_or_default().to_string();
    assert_eq!(line.trim(), "<Nullable", "choosing it did not write the name: {line:?}");

    // An unknown SDK is the file's problem once the evaluation says so.
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-a" } else { "ctrl-a" });
    cx.simulate_input("<Project Sdk=\"No.Such.Sdk\">\n</Project>\n");
    wait(cx, "the unknown SDK's problem", |cx| {
        editor.read_with(cx, |e, cx| e.buffer.read(cx).diagnostics.iter().any(|d| d.error))
    });
}
