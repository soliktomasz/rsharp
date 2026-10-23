//! The target framework switcher of a multi-targeted project, against the real Roslyn: the status bar names the
//! framework the file is seen in, and choosing another shows the file as that framework compiles it. Skips, saying why,
//! when Roslyn's server is not there.

mod support;

use gpui::{TestAppContext, VisualTestContext};
use std::time::{Duration, Instant};
use support::Run;

const PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFrameworks>net10.0;netstandard2.1</TargetFrameworks></PropertyGroup></Project>"#;
/// An error only netstandard2.1 compiles.
const CODE: &str = "namespace Demo;\n\npublic class A\n{\n#if NETSTANDARD2_1\n    int broken = \"text\";\n#endif\n}\n";

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
fn the_switcher_shows_the_file_as_the_chosen_framework_compiles_it(cx: &mut TestAppContext) {
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let run = Run::new("frameworks", &[("Demo.csproj", PROJECT), ("A.cs", CODE)]);
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |m, window, cx| {
        m.open_workspace(&run.folder, window, cx);
        m.open_file(&run.folder.join("A.cs"), Some((0, 0)), window, cx);
    });
    let buffer = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).buffer.clone());
    let frameworks = |cx: &mut VisualTestContext| {
        buffer.read_with(cx, |b, _| {
            b.contexts.iter().map(|c| c["_vs_label"].as_str().unwrap_or_default().to_string()).collect::<Vec<_>>()
        })
    };
    assert!(
        wait_for(cx, |cx| frameworks(cx).len() == 2),
        "the file was never seen in both frameworks: {:?}; Roslyn said:\n{}",
        frameworks(cx),
        main.read_with(cx, |m, _| m.roslyn_said().join("\n"))
    );
    let standard = frameworks(cx).iter().position(|f| f.contains("netstandard2.1")).expect("netstandard2.1 is offered");
    buffer.update(cx, |b, cx| b.choose_context(standard, cx));
    let errors =
        |cx: &mut VisualTestContext| buffer.read_with(cx, |b, _| b.diagnostics.iter().filter(|d| d.error).count());
    assert!(wait_for(cx, |cx| errors(cx) > 0), "netstandard2.1's error was not shown once it was chosen");
    assert_eq!(
        buffer.read_with(cx, |b, _| b.context_label()),
        Some(frameworks(cx)[standard].clone()),
        "the status bar names another framework"
    );
    let other = 1 - standard;
    buffer.update(cx, |b, cx| b.choose_context(other, cx));
    assert!(wait_for(cx, |cx| errors(cx) == 0), "net10.0 still showed netstandard2.1's error");
}
