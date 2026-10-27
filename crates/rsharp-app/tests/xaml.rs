//! Avalonia XAML as a person types it, against the real XAML host on a restored Avalonia app (Passenger's XamlTests):
//! a tag offers the controls, then a control's properties, then the chosen property's values; a misspelt property and
//! binding path are marked once typing pauses; a tag's card names its type; and Go to Declaration from a binding opens the view model's property.
//! Skips, saying why, without the XAML host or when Avalonia cannot be restored.

mod support;

use gpui::{Entity, TestAppContext, VisualTestContext};
use rsharp_app::window::MainWindow;
use std::time::{Duration, Instant};
use support::Run;

const PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <OutputType>WinExe</OutputType>\n    <TargetFramework>net10.0</TargetFramework>\n    <Nullable>enable</Nullable>\n  </PropertyGroup>\n  <ItemGroup>\n    <PackageReference Include=\"Avalonia\" Version=\"12.1.4\" />\n  </ItemGroup>\n</Project>\n";
const WINDOW: &str = "<Window xmlns=\"https://github.com/avaloniaui\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n        xmlns:vm=\"using:Demo\"\n        x:Class=\"Demo.MainWindow\"\n        x:DataType=\"vm:MainViewModel\">\n  <StackPanel Margin=\"12\" Spacing=\"8\">\n    <TextBlock Text=\"{Binding Greeting}\" />\n  </StackPanel>\n</Window>\n";
const CODE: &str = "namespace Demo;\n\npublic partial class MainWindow : Avalonia.Controls.Window { }\n";
const VIEW_MODEL: &str =
    "namespace Demo;\n\npublic class MainViewModel\n{\n    public string Greeting { get; set; } = \"Hello\";\n}\n";

fn wait(cx: &mut VisualTestContext, what: &str, done: impl Fn(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !done(cx) {
        assert!(Instant::now() < deadline, "{what} never happened");
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn shown(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> Vec<String> {
    main.read_with(cx, |m, cx| m.editor().and_then(|e| e.read(cx).completion_shown()).unwrap_or_default())
}

fn offered(main: &Entity<MainWindow>, cx: &mut VisualTestContext, what: &str, label: &str) {
    wait(cx, what, |cx| shown(main, cx).iter().any(|l| l == label));
}

fn caret_line(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> String {
    main.read_with(cx, |m, cx| {
        let editor = m.editor().unwrap().read(cx);
        let caret = editor.caret();
        let line = editor.text().to_string().lines().nth(caret.line).unwrap_or_default().to_string();
        let at = line.char_indices().nth(caret.column).map_or(line.len(), |(i, _)| i);
        format!("{}|{}", &line[..at], &line[at..])
    })
}

#[gpui::test]
fn avalonia_xaml_completes_marks_and_goes_to_what_its_project_declares(cx: &mut TestAppContext) {
    if rsharp_lsp::xaml::XamlHost::carried().is_none() {
        eprintln!("skipped: the XAML host was not built (no dotnet or no vendor/passenger at build time)");
        return;
    }
    let run = Run::new(
        "xaml",
        &[
            ("Demo.csproj", PROJECT),
            ("MainWindow.axaml", WINDOW),
            ("MainWindow.axaml.cs", CODE),
            ("MainViewModel.cs", VIEW_MODEL),
        ],
    );
    let dotnet = rsharp_core::programs::dotnet().unwrap();
    let restore = rsharp_core::dotnet::command(&dotnet, ["restore", "-v", "q"], &run.folder).output().unwrap();
    if !restore.status.success() {
        eprintln!("skipped: Avalonia 12.1.4 could not be restored: {}", String::from_utf8_lossy(&restore.stdout));
        return;
    }
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let window = run.folder.join("MainWindow.axaml");
    main.update_in(cx, |m, window_, cx| {
        m.open_workspace(&run.folder, window_, cx);
        m.open_file(&window, Some((5, 38)), window_, cx);
    });

    // A tag's '<' offers Avalonia's controls, narrowed as the name is typed.
    cx.simulate_keystrokes("enter");
    cx.simulate_input("<");
    offered(&main, cx, "Avalonia's controls after a tag's '<'", "Button");
    assert!(shown(&main, cx).iter().any(|l| l == "Border"), "Border is not among the controls: {:?}", shown(&main, cx));
    cx.simulate_input("Butt");
    cx.run_until_parked();
    assert_eq!(shown(&main, cx).first().map(String::as_str), Some("Button"), "Button is not first for \"Butt\"");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(caret_line(&main, cx).ends_with("<Button|"), "choosing Button did not write it: {}", caret_line(&main, cx));

    // Its properties, its bases' included; the one chosen goes in with its quotes and its values offered there.
    cx.simulate_input(" ");
    cx.simulate_input("H");
    offered(&main, cx, "a Button's properties, its bases' included", "HorizontalAlignment");
    cx.simulate_input("orizontalA");
    cx.run_until_parked();
    assert_eq!(shown(&main, cx).first().map(String::as_str), Some("HorizontalAlignment"));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(
        caret_line(&main, cx).ends_with("<Button HorizontalAlignment=\"|\""),
        "the property did not go in with its quotes and the caret between them: {}",
        caret_line(&main, cx)
    );
    offered(&main, cx, "the enum's members inside the new value's quotes", "Center");
    let mut values = shown(&main, cx);
    values.sort();
    assert_eq!(values, ["Center", "Left", "Right", "Stretch"], "HorizontalAlignment's values");
    cx.simulate_input("Ce");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(caret_line(&main, cx).ends_with("<Button HorizontalAlignment=\"Center|\""), "{}", caret_line(&main, cx));

    // A misspelt property and binding path are marked once typing pauses.
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    let mistaken = WINDOW
        .replace("<TextBlock Text=\"{Binding Greeting}\"", "<TextBlock Foregrund=\"Red\" Text=\"{Binding Greting}\"");
    let buffer = editor.read_with(cx, |e, _| e.buffer.clone());
    buffer.update(cx, |b, cx| b.replace_all(mistaken, cx));
    wait(cx, "the two mistakes marked", |cx| editor.read_with(cx, |e, cx| e.buffer.read(cx).diagnostics.len() == 2));
    let mut messages: Vec<String> =
        editor.read_with(cx, |e, cx| e.buffer.read(cx).diagnostics.iter().map(|d| d.message.clone()).collect());
    messages.sort();
    assert_eq!(
        messages,
        ["Cannot resolve property 'Foregrund' on TextBlock", "Cannot resolve property 'Greting' on MainViewModel"],
        "the mistakes were not marked as Passenger marks them"
    );

    // The card on a tag names its type and base.
    buffer.update(cx, |b, cx| b.replace_all(WINDOW.to_string(), cx));
    main.update_in(cx, |m, window_, cx| m.open_file(&window, Some((6, 7)), window_, cx));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "f1" } else { "ctrl-shift-f1" });
    wait(cx, "the hover card", |cx| editor.read_with(cx, |e, _| e.popup_text().is_some()));
    let card = editor.read_with(cx, |e, _| e.popup_text()).unwrap();
    assert!(card.contains("class Avalonia.Controls.TextBlock"), "the card did not name the tag's type: {card}");
    cx.simulate_keystrokes("escape");

    // Go to Declaration from the binding's path opens the view model's property.
    main.update_in(cx, |m, window_, cx| m.open_file(&window, Some((6, 32)), window_, cx));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-b" } else { "f12" });
    wait(cx, "the view model opened", |cx| {
        main.read_with(cx, |m, cx| m.editor().is_some_and(|e| e.read(cx).path.ends_with("MainViewModel.cs")))
    });
    let caret = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).caret());
    assert_eq!(caret.line, 4, "Go to Declaration did not land on the Greeting property");
}
