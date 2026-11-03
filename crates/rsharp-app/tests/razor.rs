//! Razor as a person uses it, against the real Roslyn, which cohosts Razor, on a fresh Razor class library: a
//! component's C# problem marked, completion at a tag's `<`, and Reformat Code. Skips, saying why, when Roslyn's
//! server is not installed.

mod support;

use gpui::{Entity, TestAppContext, VisualTestContext};
use rsharp_app::window::MainWindow;
use std::time::{Duration, Instant};
use support::Run;

const PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk.Razor\">\n  <PropertyGroup>\n    <TargetFramework>net10.0</TargetFramework>\n  </PropertyGroup>\n  <ItemGroup>\n    <FrameworkReference Include=\"Microsoft.AspNetCore.App\" />\n  </ItemGroup>\n</Project>\n";
const COUNTER: &str = "<h1>Count: @count</h1>\n\n<button @onclick=\"Increment\">Click</button>\n\n@code {\nprivate int count = \"zero\";\n    private void Increment() => count++;\n}\n";

/// Waits, in real time, for what Roslyn does on threads of its own to reach the window; false after three minutes.
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

fn text(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> String {
    main.read_with(cx, |m, cx| m.editor().map(|e| e.read(cx).text().to_string()).unwrap_or_default())
}

#[gpui::test]
fn a_razor_component_is_checked_completed_and_reformatted_through_roslyn(cx: &mut TestAppContext) {
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let run = Run::new("razor", &[("Shop.csproj", PROJECT), ("Counter.razor", COUNTER)]);
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Counter.razor"), Some((1, 0)), window, cx);
    });
    let loaded =
        |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.server_ready() || m.server_failed().is_some());
    assert!(
        wait_for(cx, loaded),
        "Roslyn never loaded the project; the status bar says {:?}",
        main.read_with(cx, |m, _| m.server_status())
    );
    assert_eq!(main.read_with(cx, |m, _| m.server_failed().map(str::to_string)), None, "Roslyn failed");

    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    let problems = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, cx| e.buffer.read(cx).diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>())
    };
    // Roslyn may say it has loaded before the file has moved into the project: its problems are asked again until then.
    let marked = wait_for(cx, |cx| {
        let found = problems(cx).iter().any(|p| p.contains("Cannot implicitly convert type 'string' to 'int'"));
        if !found {
            editor.update(cx, |e, cx| e.buffer.update(cx, |b, cx| b.check(Duration::ZERO, cx)));
        }
        found
    });
    assert!(marked, "the component's string given to an int was never marked; the file shows {:?}", problems(cx));

    // A tag's `<` on the empty line lists the elements.
    cx.simulate_input("<");
    assert!(
        wait_for(cx, |cx| editor.read_with(cx, |e, _| e.completion_shown().is_some_and(|s| !s.is_empty()))),
        "completion at a Razor file's `<` never showed"
    );
    let shown = editor.read_with(cx, |e, _| e.completion_shown()).unwrap_or_default();
    assert!(shown.iter().any(|s| s == "button" || s == "div"), "`<` did not offer HTML's elements: {shown:?}");
    cx.simulate_keystrokes("escape secondary-z");
    cx.run_until_parked();
    assert_eq!(text(&main, cx), COUNTER, "Undo did not take back what was typed");

    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-alt-l" } else { "ctrl-alt-enter" });
    assert!(
        wait_for(cx, |cx| text(&main, cx).contains("\n    private int count = \"zero\";\n")),
        "Reformat Code did not indent the @code block's member:\n{}",
        text(&main, cx)
    );
}
