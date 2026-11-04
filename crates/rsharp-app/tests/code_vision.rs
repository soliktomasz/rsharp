//! Code Vision: Roslyn's usage counts at the end of each member's line, a click on one listing the usages.

mod support;

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use std::time::{Duration, Instant};
use support::Run;

const PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>"#;
/// `Add` is declared on line 4 (from 0) and called twice; `Unused` never.
const SHOP: &str = "namespace Shop;\n\npublic class Cart\n{\n    public void Add() { }\n    public void Unused() { }\n\n    public void Fill()\n    {\n        Add();\n        Add();\n    }\n}\n";

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
fn code_vision_counts_usages_and_opens_them(cx: &mut TestAppContext) {
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let run = Run::new("code-vision", &[("Shop.csproj", PROJECT), ("Shop.cs", SHOP)]);
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Shop.cs"), Some((0, 0)), window, cx);
    });
    let loaded =
        |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.server_ready() || m.server_failed().is_some());
    assert!(wait_for(cx, loaded), "Roslyn never loaded: {:?}", main.read_with(cx, |m, _| m.server_status()));
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    // Roslyn counts once the file has joined the project; until then a lens may say 0 or be missing.
    let counted = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.code_vision().contains(&(4, "2 usages".to_string())))
    };
    assert!(
        wait_for(cx, counted),
        "Code Vision never said Add's 2 usages: {:?}",
        editor.read_with(cx, |e, _| e.code_vision())
    );
    assert!(
        editor.read_with(cx, |e, _| e.code_vision().contains(&(5, "no usages".to_string()))),
        "an unused member did not say so: {:?}",
        editor.read_with(cx, |e, _| e.code_vision())
    );

    let at = editor.read_with(cx, |e, _| e.code_vision_bounds(4)).expect("Add's lens was not drawn").center();
    cx.simulate_click(at, Modifiers::default());
    let listed = |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.found().is_some_and(|f| f.len() == 2));
    assert!(
        wait_for(cx, listed),
        "a click on the lens did not list Add's usages: {:?}",
        main.read_with(cx, |m, _| m.found())
    );
    let mut found = main.read_with(cx, |m, _| m.found()).unwrap();
    found.sort();
    assert_eq!(found, ["Shop.cs:10", "Shop.cs:11"], "the lens listed other places than Add's calls");

    // Settings | Editor | Inlay Hints | Code Vision off: no lens is drawn.
    cx.update(|_, cx| {
        let mut settings = rsharp_app::settings::settings(cx).clone();
        settings.code_vision = false;
        rsharp_app::settings::apply(settings, cx);
    });
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.code_vision_bounds(4)).is_none(), "Code Vision drew a lens while off");
}

#[gpui::test]
fn code_vision_without_roslyn_draws_nothing(cx: &mut TestAppContext) {
    let run = Run::new("code-vision-no-server", &[("Shop.csproj", PROJECT), ("Shop.cs", SHOP)]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("Shop.cs"), Some((0, 0)), window, cx);
    });
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    assert!(editor.read_with(cx, |e, _| e.code_vision().is_empty()), "lenses drawn with no server to count them");
}
