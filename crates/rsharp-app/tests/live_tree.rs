//! The Live Visual Tree of an Avalonia app debugged from the window (Passenger's LiveTreeTests and BindingErrorsTests):
//! Passenger's LiveTreeApp, headless, with a binding that fails; its tree as the inspector inside it says, an element's
//! XAML place, and the failed binding written in the Debug console where its place links to the XAML.

mod support;

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use std::path::Path;
use std::time::{Duration, Instant};
use support::Run;

const VERSION: &str = "12.1.4";

/// LiveTreeApp's window with a caption bound to a name its data context lacks, which fails as the app shows it.
const WINDOW: &str = "<Window xmlns=\"https://github.com/avaloniaui\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n        x:Class=\"LiveTreeApp.MainWindow\"\n        x:CompileBindings=\"False\"\n        Title=\"Live Tree App\" Width=\"480\" Height=\"320\">\n    <StackPanel x:Name=\"Panel\" Margin=\"12\" Spacing=\"8\">\n        <TextBlock x:Name=\"Greeting\" Classes=\"title\" Text=\"Hello\" />\n        <Button x:Name=\"Save\" Content=\"Save\" />\n        <TextBlock x:Name=\"Caption\" DataContext=\"Ada\" Text=\"{Binding Nmae}\" />\n    </StackPanel>\n</Window>\n";

/// Passenger's headless app project, at the Avalonia the tests carry.
fn project() -> String {
    let packages: String = [
        "Avalonia",
        "Avalonia.Themes.Fluent",
        "Avalonia.Fonts.Inter",
        "Avalonia.Headless",
        "Avalonia.HarfBuzz",
        "Avalonia.Skia",
    ]
    .iter()
    .map(|p| format!("    <PackageReference Include=\"{p}\" Version=\"{VERSION}\" />\n"))
    .collect();
    format!(
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <OutputType>Exe</OutputType>\n    <TargetFramework>net10.0</TargetFramework>\n    <Nullable>enable</Nullable>\n    <ImplicitUsings>enable</ImplicitUsings>\n  </PropertyGroup>\n  <ItemGroup>\n{packages}  </ItemGroup>\n</Project>\n"
    )
}

/// The app in a folder of the test's own, restored; none, saying why, without what it needs.
fn app(name: &str) -> Option<Run> {
    if rsharp_dap::adapter().is_none() || rsharp_dap::inspector::carried().is_none() {
        eprintln!("skipped: Passenger's debug adapter or inspector is not built");
        return None;
    }
    let debuggee = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/passenger/tests/Debuggees/LiveTreeApp");
    let files: Vec<(String, String)> = ["App.axaml", "App.axaml.cs", "MainWindow.axaml.cs", "Program.cs"]
        .iter()
        .map(|f| (f.to_string(), std::fs::read_to_string(debuggee.join(f)).expect("the submodule's LiveTreeApp")))
        .chain([("MainWindow.axaml".into(), WINDOW.into()), ("LiveTreeApp.csproj".into(), project())])
        .collect();
    let named: Vec<(&str, &str)> = files.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
    let run = Run::new(name, &named);
    let dotnet = rsharp_core::programs::dotnet()?;
    let restore = rsharp_core::dotnet::command(&dotnet, ["restore", "-v", "q"], &run.folder).output().unwrap();
    if !restore.status.success() {
        eprintln!("skipped: Avalonia could not be restored: {}", String::from_utf8_lossy(&restore.stdout));
        return None;
    }
    Some(run)
}

fn pump(cx: &mut VisualTestContext, what: &str, done: impl Fn(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(180);
    while !done(cx) {
        assert!(Instant::now() < deadline, "{what} never happened");
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[gpui::test]
fn a_debugged_avalonia_apps_tree_its_elements_xaml_and_its_failed_binding_are_shown(cx: &mut TestAppContext) {
    let Some(run) = app("live-tree") else { return };
    let (main, cx) = run.window(cx);
    let folder = run.folder.clone();
    main.update_in(cx, |m, window, cx| m.open_workspace(&folder, window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f5");
    let status = |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.live_status()).unwrap_or_default();
    pump(cx, "the app's inspector saying hello", |cx| status(cx).starts_with("Avalonia"));
    assert!(status(cx).contains(VERSION), "the window does not say the app's Avalonia: {}", status(cx));

    // Its tree, as the inspector inside it says: the window, the panel, the named controls.
    cx.dispatch_action(rsharp_app::window::LiveVisualTree);
    let rows =
        |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.live_tree()).map(|(r, _)| r).unwrap_or_default();
    pump(cx, "the app's tree", |cx| rows(cx).iter().any(|r| r.contains("MainWindow")));
    // Open down to the app's own elements.
    pump(cx, "the Save button among the rows", |cx| rows(cx).iter().any(|r| r.trim() == "Button#Save"));
    assert!(rows(cx).iter().any(|r| r.trim() == "Button#Save"), "the tree has no Save button: {:?}", rows(cx));

    // The button chosen: its XAML place, where the app's Debug build recorded it.
    let save = cx.debug_bounds("live element Button#Save").expect("the Save button's row");
    cx.simulate_click(save.center(), Modifiers::default());
    pump(cx, "the button's details", |cx| main.read_with(cx, |m, _| m.live_tree().and_then(|(_, d)| d).is_some()));
    let details = main.read_with(cx, |m, _| m.live_tree().and_then(|(_, d)| d)).unwrap();
    assert!(
        details["source"]["file"].as_str().is_some_and(|f| f.ends_with("MainWindow.axaml")),
        "the button's XAML place is not its window's file: {details}"
    );
    assert_eq!(details["source"]["line"], 8, "the button's XAML line");

    // The caption's binding failed as the app showed it: a console line led by its place, as a compiler message's.
    let console = |cx: &mut VisualTestContext| main.read_with(cx, |m, cx| m.debug_console().read(cx).text());
    pump(cx, "the failed binding in the Debug console", |cx| console(cx).contains("binding error"));
    let said = console(cx);
    assert!(said.contains("MainWindow.axaml(9,") && said.contains("Nmae"), "the binding error's line: {said}");

    // The window's XAML in the editor and the caption chosen: its binding's value shows after its line, failed.
    let xaml = rsharp_core::paths::canonical(&run.folder.join("MainWindow.axaml"));
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |m, window, cx| m.open_file(&xaml, None, window, cx));
    let caption = cx.debug_bounds("live element TextBlock#Caption").expect("the caption's row");
    cx.simulate_click(caption.center(), Modifiers::default());
    let values = |cx: &mut VisualTestContext| {
        main.read_with(cx, |_, cx| {
            rsharp_app::buffer::find(&xaml, cx).map(|b| b.read(cx).binding_values.clone()).unwrap_or_default()
        })
    };
    pump(cx, "the caption's binding value after its line", |cx| !values(cx).is_empty());
    assert!(
        values(cx).iter().any(|(line, text)| *line == 8 && text.starts_with("Text: binding failed")),
        "the caption's failed binding is not after its line: {:?}",
        values(cx)
    );
    cx.simulate_keystrokes("shift-f5");
    cx.run_until_parked();
}
