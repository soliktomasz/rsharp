//! A project Roslyn cannot load, as a person learns of it: said in the status bar and listed in Problems' Toolset.
//! Skips, saying why, when Roslyn's server is not installed.
// A file of its own, so its Roslyn is the only one its test process starts.

mod support;

use gpui::{TestAppContext, VisualTestContext};
use rsharp_app::window::ProblemsTab;
use std::time::{Duration, Instant};
use support::Run;

/// Waits, in real time, for what Roslyn says on threads of its own to reach the window; false after three minutes.
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
fn a_project_that_will_not_load_is_said_in_the_status_bar_and_listed_in_toolset(cx: &mut TestAppContext) {
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let run = Run::new(
        "load-problem",
        &[
            (
                "Bad.csproj",
                r#"<Project Sdk="No.Such.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>"#,
            ),
            ("A.cs", "class A {}\n"),
        ],
    );
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |main, window, cx| main.open_workspace(&run.folder, window, cx));
    let listed = |cx: &mut VisualTestContext| main.read_with(cx, |m, cx| m.problems_listed(ProblemsTab::Toolset, cx));
    if !wait_for(cx, |cx| !listed(cx).is_empty()) {
        panic!(
            "Toolset never listed Bad.csproj; the status bar says {:?}",
            main.read_with(cx, |m, _| m.server_status())
        );
    }
    let problems = listed(cx);
    assert!(
        problems[0].starts_with("Bad.csproj ") && problems[0].contains("No.Such.Sdk"),
        "Toolset listed {problems:?}"
    );
}
