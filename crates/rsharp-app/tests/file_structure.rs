//! File Structure (Alt+\): the shown C# file's types and members from Roslyn, filtered as typed, Enter going to one.

mod support;

use gpui::{TestAppContext, VisualTestContext};
use std::time::{Duration, Instant};
use support::Run;

const PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>"#;
/// `Pay` is declared on line 10 (from 0), its name at column 16; the caret starts inside its body.
const SHOP: &str = "namespace Shop;\n\npublic class Cart\n{\n    public void Add() { }\n    public void Remove() { }\n}\n\npublic class Order\n{\n    public void Pay()\n    {\n        int total = 1;\n    }\n}\n";

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
fn file_structure_lists_members_and_goes_to_one(cx: &mut TestAppContext) {
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let run = Run::new("file-structure", &[("Shop.csproj", PROJECT), ("Shop.cs", SHOP)]);
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Shop.cs"), Some((12, 8)), window, cx);
    });
    let loaded =
        |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.server_ready() || m.server_failed().is_some());
    assert!(wait_for(cx, loaded), "Roslyn never loaded: {:?}", main.read_with(cx, |m, _| m.server_status()));

    // Roslyn may answer before the file has joined the project; the popup is asked again until it lists members.
    let listed = |cx: &mut VisualTestContext| {
        if main.read_with(cx, |m, _| m.go_to_popup().is_none()) {
            cx.simulate_keystrokes("alt-\\");
        }
        main.read_with(cx, |m, cx| m.go_to_listed(cx).is_some_and(|rows| rows.iter().any(|r| r == "Pay() void")))
    };
    assert!(
        wait_for(cx, listed),
        "File Structure never listed the file's members: {:?}, the status bar says {:?}",
        main.read_with(cx, |m, cx| m.go_to_listed(cx)),
        main.read_with(cx, |m, _| m.said().map(str::to_string))
    );
    let popup = main.read_with(cx, |m, _| m.go_to_popup().unwrap());
    assert_eq!(popup.read_with(cx, |p, _| p.title()), Some("File Structure: Shop.cs".into()));
    assert_eq!(
        popup.read_with(cx, |p, _| p.chosen_name()),
        Some("Pay()".into()),
        "the popup did not open on the member the caret is in"
    );

    cx.simulate_input("pay");
    cx.run_until_parked();
    let rows = main.read_with(cx, |m, cx| m.go_to_listed(cx)).unwrap();
    let members: Vec<_> = rows.iter().filter(|r| *r != "Shop").map(String::as_str).collect();
    assert_eq!(members, ["Order", "Pay() void"], "typing \"pay\" did not leave Pay in its class: {rows:?}");

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let caret = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).caret());
    assert_eq!((caret.line, caret.column), (10, 16), "Enter did not put the caret on Pay's name");
    assert!(main.read_with(cx, |m, _| m.go_to_popup().is_none()), "the popup stayed open after going");
}

#[gpui::test]
fn file_structure_without_roslyn_says_so(cx: &mut TestAppContext) {
    let run = Run::new("file-structure-no-server", &[("Shop.csproj", PROJECT), ("Shop.cs", SHOP)]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Shop.cs"), Some((12, 8)), window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("alt-\\");
    cx.run_until_parked();
    assert!(main.read_with(cx, |m, _| m.go_to_popup().is_none()), "an empty File Structure opened with no server");
    assert_eq!(
        main.read_with(cx, |m, _| m.said().map(str::to_string)).as_deref(),
        Some("File Structure needs Roslyn, which is not running")
    );
}
