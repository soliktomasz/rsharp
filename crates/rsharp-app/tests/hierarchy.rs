//! The Hierarchy window: Call Hierarchy's callers and callees, and Type Hierarchy's supertypes, from Roslyn.

mod support;

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use std::time::{Duration, Instant};
use support::Run;

const PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>"#;
/// `A` (line 5) and `C` (line 7) call `B` (line 6), counted from 0.
const CALLS: &str = "namespace Shop;\n\npublic class Base { }\n\npublic class Calls : Base\n{\n    void A() { B(); }\n    void B() { }\n    void C() { B(); }\n}\n";

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

fn click(cx: &mut VisualTestContext, selector: &'static str, count: usize) {
    cx.run_until_parked();
    let position = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing on screen is {selector}")).center();
    let (button, modifiers) = (gpui::MouseButton::Left, Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent { button, position, modifiers, click_count: count, first_mouse: false });
    cx.simulate_event(gpui::MouseUpEvent { button, position, modifiers, click_count: count });
    cx.run_until_parked();
}

#[gpui::test]
fn call_and_type_hierarchies_list_from_roslyn(cx: &mut TestAppContext) {
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let run = Run::new("hierarchy", &[("Shop.csproj", PROJECT), ("Calls.cs", CALLS)]);
    let source = run.folder.join("Calls.cs");
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&source, Some((7, 9)), window, cx);
    });
    let loaded =
        |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.server_ready() || m.server_failed().is_some());
    assert!(wait_for(cx, loaded), "Roslyn never loaded: {:?}", main.read_with(cx, |m, _| m.server_status()));
    let rows = |cx: &mut VisualTestContext| main.read_with(cx, |m, cx| m.hierarchy_rows(cx));

    // Roslyn may answer before the file has joined the project; the key is pressed again until callers come.
    let callers = |cx: &mut VisualTestContext| {
        if rows(cx).len() < 3 {
            cx.simulate_keystrokes("ctrl-alt-h");
        }
        rows(cx).len() == 3
    };
    assert!(wait_for(cx, callers), "Call Hierarchy never listed B's callers: {:?}", rows(cx));
    let listed = rows(cx);
    assert!(listed[0].starts_with('B'), "the root is not B: {listed:?}");
    assert!(listed[1].trim_start().starts_with('A') && listed[2].trim_start().starts_with('C'), "{listed:?}");
    assert!(listed[1].starts_with("  "), "a caller is not drawn under its callee: {listed:?}");
    assert_eq!(main.read_with(cx, |m, cx| m.hierarchy_title(cx)), Some("Calls of B()".to_string()));

    click(cx, "hierarchy row: 2", 2);
    let caret = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).caret());
    assert_eq!((caret.line, caret.column), (8, 15), "a double-click did not go to C's call of B");

    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((6, 9)), window, cx));
    cx.simulate_keystrokes("ctrl-alt-h");
    assert!(wait_for(cx, |cx| rows(cx).first().is_some_and(|r| r.starts_with('A'))), "{:?}", rows(cx));
    click(cx, "hierarchy: Callees", 1);
    assert!(
        wait_for(cx, |cx| rows(cx).iter().any(|r| r.trim_start().starts_with('B') && r.starts_with("  "))),
        "Callees of A did not list B: {:?}",
        rows(cx)
    );

    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((4, 14)), window, cx));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "ctrl-h" } else { "ctrl-e h" });
    assert!(
        wait_for(cx, |cx| rows(cx).iter().any(|r| r.trim_start().starts_with("Base") && r.starts_with("  "))),
        "Type Hierarchy did not list Calls' base: {:?}",
        rows(cx)
    );
    assert_eq!(main.read_with(cx, |m, cx| m.hierarchy_title(cx)), Some("Hierarchy of Calls".to_string()));

    // Closing the solution lets its Roslyn go: the window holds neither its rows nor its server.
    main.update_in(cx, |main, window, cx| main.close_solution(window, cx));
    cx.run_until_parked();
    assert!(rows(cx).is_empty(), "the closed solution's hierarchy stayed listed");
    assert!(
        !main.read_with(cx, |m, cx| m.hierarchy_holds_server(cx)),
        "the Hierarchy window kept the closed solution's Roslyn alive"
    );
}

#[gpui::test]
fn call_hierarchy_without_roslyn_says_so(cx: &mut TestAppContext) {
    let run = Run::new("hierarchy-no-server", &[("Shop.csproj", PROJECT), ("Calls.cs", CALLS)]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Calls.cs"), Some((7, 9)), window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-alt-h");
    cx.run_until_parked();
    assert_eq!(
        main.read_with(cx, |m, _| m.said().map(str::to_string)).as_deref(),
        Some("Call Hierarchy needs Roslyn, which is not running")
    );
}
