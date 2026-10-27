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

fn pump(cx: &mut VisualTestContext, duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
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

fn restored(name: &str, files: &[(&str, &str)]) -> Option<Run> {
    if rsharp_lsp::xaml::XamlHost::carried().is_none() {
        eprintln!("skipped: the XAML host was not built");
        return None;
    }
    let run = Run::new(name, files);
    let dotnet = rsharp_core::programs::dotnet().unwrap();
    let restore = rsharp_core::dotnet::command(&dotnet, ["restore", "-v", "q"], &run.folder).output().unwrap();
    if !restore.status.success() {
        eprintln!("skipped: Avalonia could not be restored: {}", String::from_utf8_lossy(&restore.stdout));
        return None;
    }
    Some(run)
}

#[gpui::test]
fn a_dot_requests_the_next_binding_members(cx: &mut TestAppContext) {
    let text = WINDOW.replace("Greeting", "");
    let model = "namespace Demo; public class MainViewModel { public Person Person { get; set; } } public class Person { public string Name { get; set; } }";
    let Some(run) = restored(
        "xaml-dot",
        &[
            ("Demo.csproj", PROJECT),
            ("MainWindow.axaml", &text),
            ("MainWindow.axaml.cs", CODE),
            ("MainViewModel.cs", model),
        ],
    ) else {
        return;
    };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |m, window, cx| {
        m.open_workspace(&run.folder, window, cx);
        m.open_file(&run.folder.join("MainWindow.axaml"), Some((6, 30)), window, cx);
    });
    cx.run_until_parked();
    cx.simulate_input("P");
    assert!(caret_line(&main, cx).contains("{Binding P|}"), "the binding caret is wrong: {}", caret_line(&main, cx));
    offered(&main, cx, "the view model's Person property", "Person");
    for c in "erson".chars() {
        cx.simulate_input(&c.to_string());
    }
    cx.simulate_input(".");
    offered(&main, cx, "Person's Name after the binding dot", "Name");
    cx.simulate_keystrokes("enter");
    assert!(caret_line(&main, cx).contains("{Binding Person.Name|}"), "{}", caret_line(&main, cx));
}

#[gpui::test]
fn a_binding_dot_closes_the_previous_list_even_without_members(cx: &mut TestAppContext) {
    let text = WINDOW.replace("Greeting", "");
    let model =
        "namespace Demo; public class MainViewModel { public Person Person { get; set; } } public class Person { }";
    let Some(run) = restored(
        "xaml-dot-empty",
        &[
            ("Demo.csproj", PROJECT),
            ("MainWindow.axaml", &text),
            ("MainWindow.axaml.cs", CODE),
            ("MainViewModel.cs", model),
        ],
    ) else {
        return;
    };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |m, window, cx| {
        m.open_workspace(&run.folder, window, cx);
        m.open_file(&run.folder.join("MainWindow.axaml"), Some((6, 30)), window, cx);
    });
    cx.run_until_parked();
    cx.simulate_input("P");
    offered(&main, cx, "the Person property before its empty member list", "Person");
    for c in "erson".chars() {
        cx.simulate_input(&c.to_string());
    }
    cx.simulate_input(".");
    assert!(shown(&main, cx).is_empty(), "the previous Person choice can still replace Person. after typing its dot");
    cx.simulate_keystrokes("enter");
    let text = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).text().to_string());
    assert!(text.contains("{Binding Person."), "Enter accepted an old completion and removed the binding dot: {text}");
}

#[gpui::test]
fn a_saved_source_change_rechecks_open_xaml(cx: &mut TestAppContext) {
    let text = WINDOW.replace("Greeting", "Added");
    let Some(run) = restored(
        "xaml-source-change",
        &[
            ("Demo.csproj", PROJECT),
            ("MainWindow.axaml", &text),
            ("MainWindow.axaml.cs", CODE),
            ("MainViewModel.cs", VIEW_MODEL),
        ],
    ) else {
        return;
    };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |m, window, cx| {
        m.open_workspace(&run.folder, window, cx);
        m.open_file(&run.folder.join("MainWindow.axaml"), None, window, cx);
    });
    let buffer = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).buffer.clone());
    wait(cx, "the missing Added property marked", |cx| {
        buffer.read_with(cx, |b, _| b.diagnostics.iter().any(|d| d.message.contains("'Added'")))
    });
    std::fs::write(run.folder.join("MainViewModel.cs"), VIEW_MODEL.replace("Greeting", "Added")).unwrap();
    wait(cx, "the binding error cleared after saving its C# property", |cx| {
        buffer.read_with(cx, |b, _| b.diagnostics.is_empty())
    });
}

#[gpui::test]
fn resources_follow_unsaved_edits_until_the_last_editor_closes(cx: &mut TestAppContext) {
    let window_text = "<Window xmlns=\"https://github.com/avaloniaui\" Background=\"{StaticResource Ghost}\" />";
    let app = "<Application xmlns=\"https://github.com/avaloniaui\" xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"></Application>";
    let typed = app.replace("</Application>", "<Application.Resources><SolidColorBrush x:Key=\"Ghost\" Color=\"Red\" /></Application.Resources></Application>");
    let Some(run) = restored(
        "xaml-resources-close",
        &[("Demo.csproj", PROJECT), ("MainWindow.axaml", window_text), ("App.axaml", app)],
    ) else {
        return;
    };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |m, window, cx| {
        m.open_workspace(&run.folder, window, cx);
        m.open_file(&run.folder.join("MainWindow.axaml"), None, window, cx);
    });
    let buffer = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).buffer.clone());
    wait(cx, "the missing Ghost resource marked", |cx| {
        buffer.read_with(cx, |b, _| b.diagnostics.iter().any(|d| d.message.contains("'Ghost'")))
    });
    main.update_in(cx, |m, window, cx| m.open_file(&run.folder.join("App.axaml"), None, window, cx));
    main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).buffer.clone())
        .update(cx, |b, cx| b.replace_all(typed, cx));
    wait(cx, "the other buffer sees the unsaved resource", |cx| buffer.read_with(cx, |b, _| b.diagnostics.is_empty()));
    cx.dispatch_action(rsharp_app::window::SplitRight);
    cx.run_until_parked();
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-w" } else { "ctrl-f4" });
    cx.run_until_parked();
    buffer.update(cx, |b, cx| b.replace_all(window_text.replace("Ghost", "Missing"), cx));
    wait(cx, "the surviving editor still checks dependent resources", |cx| {
        buffer.read_with(cx, |b, _| b.diagnostics.iter().any(|d| d.message.contains("'Missing'")))
    });
    buffer.update(cx, |b, cx| b.replace_all(window_text.to_string(), cx));
    wait(cx, "closing one split keeps the unsaved resource", |cx| {
        buffer.read_with(cx, |b, _| b.diagnostics.is_empty())
    });
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-w" } else { "ctrl-f4" });
    wait(cx, "the discarded resource is missing again", |cx| {
        buffer.read_with(cx, |b, _| b.diagnostics.iter().any(|d| d.message.contains("'Ghost'")))
    });
}

#[gpui::test]
fn reopening_a_solution_reads_changes_made_while_it_was_closed(cx: &mut TestAppContext) {
    let text = WINDOW.replace("Greeting", "Added");
    let Some(run) = restored(
        "xaml-reopen",
        &[
            ("Demo.csproj", PROJECT),
            ("MainWindow.axaml", &text),
            ("MainWindow.axaml.cs", CODE),
            ("MainViewModel.cs", VIEW_MODEL),
        ],
    ) else {
        return;
    };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let path = run.folder.join("MainWindow.axaml");
    main.update_in(cx, |m, window, cx| {
        m.open_workspace(&run.folder, window, cx);
        m.open_file(&path, None, window, cx);
    });
    wait(cx, "the first session's missing property marked", |cx| {
        main.read_with(cx, |m, cx| !m.editor().unwrap().read(cx).buffer.read(cx).diagnostics.is_empty())
    });
    main.update_in(cx, |m, window, cx| m.close_solution(window, cx));
    cx.run_until_parked();
    std::fs::write(run.folder.join("MainViewModel.cs"), VIEW_MODEL.replace("Greeting", "Added")).unwrap();
    main.update_in(cx, |m, window, cx| {
        m.open_workspace(&run.folder, window, cx);
        m.open_file(&path, Some((6, 32)), window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "f1" } else { "ctrl-shift-f1" });
    wait(cx, "the reopened session reads the new Added declaration", |cx| {
        main.read_with(cx, |m, cx| {
            m.editor()
                .unwrap()
                .read(cx)
                .popup_text()
                .is_some_and(|t| t.contains("Added") && !t.contains("Cannot resolve"))
        })
    });
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

#[gpui::test]
fn after_a_build_the_xaml_compilers_error_is_marked_on_its_line_until_that_line_is_edited(cx: &mut TestAppContext) {
    // A library, so the build reaches Avalonia's XAML compiler with no entry point to write.
    let project = PROJECT.replace("<OutputType>WinExe</OutputType>", "<OutputType>Library</OutputType>");
    let text = WINDOW.replace("<TextBlock Text", "<TextBlock Width=\"wide\" Text");
    let Some(run) = restored(
        "xaml-built",
        &[
            ("Demo.csproj", &project),
            ("MainWindow.axaml", &text),
            ("MainWindow.axaml.cs", CODE),
            ("MainViewModel.cs", VIEW_MODEL),
        ],
    ) else {
        return;
    };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let window = run.folder.join("MainWindow.axaml");
    main.update_in(cx, |m, window_, cx| {
        m.open_workspace(&run.folder, window_, cx);
        m.open_file(&window, Some((6, 23)), window_, cx);
    });
    let buffer = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).buffer.clone());

    // A number only the compiler reads: marked once the build says so, on the attribute's name.
    cx.simulate_keystrokes("secondary-shift-b");
    wait(cx, "the build's AVLN1000 marked", |cx| {
        buffer.read_with(cx, |b, _| b.diagnostics.iter().any(|d| d.message.starts_with("AVLN1000")))
    });
    let (marked, word) = buffer.read_with(cx, |b, _| {
        let d = b.diagnostics.iter().find(|d| d.message.starts_with("AVLN1000")).unwrap().clone();
        let word = b.document.text().byte_slice(d.range.clone()).to_string();
        (d, word)
    });
    assert!(marked.error, "the compiler's error is not marked as an error");
    assert_eq!(
        marked.message, "AVLN1000: The input string 'wide' was not in a correct format.",
        "the editor displayed the wrong build error text"
    );
    assert_eq!(word, "Width", "the build's problem is not on the attribute its place points at");

    // The line edited, the build's word on it is the build's, no longer this text's.
    cx.simulate_input("x");
    wait(cx, "the build's problem gone from the edited line", |cx| {
        buffer.read_with(cx, |b, _| b.diagnostics.iter().all(|d| !d.message.starts_with("AVLN")))
    });
}

#[gpui::test]
fn an_elements_name_renames_with_its_code_behinds_uses_and_the_code_goes_to_it(cx: &mut TestAppContext) {
    let project = PROJECT.replace("<OutputType>WinExe</OutputType>", "<OutputType>Library</OutputType>");
    let window_text = "<Window xmlns=\"https://github.com/avaloniaui\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n        x:Class=\"Demo.MainWindow\">\n  <StackPanel>\n    <Button Name=\"Go\" Content=\"Go\" />\n    <TextBlock Text=\"{Binding #Go.Content}\" />\n  </StackPanel>\n</Window>\n";
    let code = "namespace Demo;\n\npublic partial class MainWindow : Avalonia.Controls.Window // Go Go Go Go Go Go\n{\n    public MainWindow()\n    {\n        InitializeComponent();\n        this.Go.Content = \"Start\";\n    }\n}\n";
    let Some(run) = restored(
        "xaml-name",
        &[("Demo.csproj", &project), ("MainWindow.axaml", window_text), ("MainWindow.axaml.cs", code)],
    ) else {
        return;
    };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let (xaml, behind) = (run.folder.join("MainWindow.axaml"), run.folder.join("MainWindow.axaml.cs"));
    main.update_in(cx, |m, window, cx| m.open_workspace(&run.folder, window, cx));
    let active = |main: &Entity<MainWindow>, cx: &mut VisualTestContext| {
        main.read_with(cx, |m, cx| m.editor().map(|e| e.read(cx).path.clone()))
    };

    // The field Avalonia's generator makes for the button leads to its name in the XAML, once Roslyn has the project.
    let deadline = Instant::now() + Duration::from_secs(120);
    while active(&main, cx).as_ref() != Some(&xaml) {
        assert!(Instant::now() < deadline, "Go to Declaration on the field never reached the XAML's name");
        main.update_in(cx, |m, window, cx| m.open_file(&behind, Some((7, 14)), window, cx));
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-b" } else { "f12" });
        for _ in 0..20 {
            cx.executor().advance_clock(Duration::from_millis(100));
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    assert!(caret_line(&main, cx).contains("<Button Name=\"|Go\""), "not on the name: {}", caret_line(&main, cx));

    // Find Usages lists where the XAML names it and where the code-behind uses it.
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "alt-f7" } else { "shift-f12" });
    wait(cx, "Find Usages listing", |cx| main.read_with(cx, |m, _| m.found().is_some()));
    let found = main.read_with(cx, |m, _| m.found()).unwrap();
    for place in ["MainWindow.axaml:5", "MainWindow.axaml:6", "MainWindow.axaml.cs:8"] {
        assert!(found.iter().any(|f| f == place), "Find Usages did not list {place}: {found:?}");
    }

    // Rename changes the name, the binding that names it and the code-behind's use.
    main.update_in(cx, |m, window, cx| m.open_file(&xaml, Some((4, 19)), window, cx));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "shift-f6" } else { "ctrl-r ctrl-r" });
    cx.run_until_parked();
    cx.simulate_input("Start");
    cx.simulate_keystrokes("enter");
    let text_of = |path: &std::path::PathBuf, cx: &mut VisualTestContext| {
        cx.update(|_, cx| rsharp_app::buffer::find(path, cx).map(|b| b.read(cx).document.text().to_string()))
            .unwrap_or_default()
    };
    wait(cx, "the code-behind's use renamed", |cx| text_of(&behind, cx).contains("Start.Content = \"Start\";"));
    let renamed = text_of(&xaml, cx);
    assert!(renamed.contains("<Button Name=\"Start\""), "the element's name was not renamed:\n{renamed}");
    assert!(renamed.contains("{Binding #Start.Content}"), "the binding naming it was not renamed:\n{renamed}");
}

const PALETTE: &str = "<ResourceDictionary xmlns=\"https://github.com/avaloniaui\"\n                    xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\">\n    <ResourceDictionary.ThemeDictionaries>\n        <ResourceDictionary x:Key=\"Dark\">\n            <SolidColorBrush x:Key=\"Accent\" Color=\"#3D7BF5\" />\n            <SolidColorBrush x:Key=\"Muted\" Color=\"#868A91\" />\n        </ResourceDictionary>\n        <ResourceDictionary x:Key=\"Light\">\n            <SolidColorBrush x:Key=\"Accent\" Color=\"#3574F0\" />\n            <SolidColorBrush x:Key=\"Muted\" Color=\"#6C707E\" />\n        </ResourceDictionary>\n    </ResourceDictionary.ThemeDictionaries>\n</ResourceDictionary>\n";
const APP: &str = "<Application xmlns=\"https://github.com/avaloniaui\"\n             xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n             x:Class=\"Demo.App\">\n  <Application.Resources>\n    <ResourceDictionary>\n      <ResourceDictionary.MergedDictionaries>\n        <ResourceInclude Source=\"avares://Demo/Styles/Palette.axaml\" />\n      </ResourceDictionary.MergedDictionaries>\n    </ResourceDictionary>\n  </Application.Resources>\n</Application>\n";

#[test]
fn a_fix_that_changes_csharp_makes_its_edit_from_the_editors_unsaved_text() {
    let Some(run) = restored(
        "xaml-fixes-csharp",
        &[
            ("Demo.csproj", PROJECT),
            ("MainWindow.axaml", WINDOW),
            ("MainWindow.axaml.cs", CODE),
            ("MainViewModel.cs", VIEW_MODEL),
        ],
    ) else {
        return;
    };
    let host = rsharp_lsp::xaml::XamlHost::carried().unwrap();
    let (project, path) = (run.folder.join("Demo.csproj"), run.folder.join("MainWindow.axaml"));
    let model = run.folder.join("MainViewModel.cs");
    let text = WINDOW.replace("{Binding Greeting}", "{Binding Missing}");
    let problems = host.check(&project, &path, &text).unwrap().unwrap();
    assert!(problems.iter().any(|p| p.message == "Cannot resolve property 'Missing' on MainViewModel"));
    // The view model as the editor has it, two lines longer than on disk.
    let unsaved = VIEW_MODEL.replace("{\n", "{\n    public int Count { get; set; }\n\n");
    let texts = std::collections::HashMap::from([(model.clone(), unsaved.clone())]);
    let actions = host.fixes(&project, &path, &text, 6, &texts).unwrap();
    let create = actions
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["title"] == "Create property 'Missing'")
        .expect("the property the binding names is offered to create");
    let edit = rsharp_lsp::workspace_edit(&create["edit"]);
    let (_, edits) = edit.files.iter().find(|(p, _)| *p == model).expect("the fix changes the view model");
    // Made from the editor's text, the edits leave it whole with the new property before its closing brace.
    let offset =
        |p: rsharp_lsp::Point| unsaved.split_inclusive('\n').take(p.line).map(str::len).sum::<usize>() + p.character;
    let mut made = unsaved.clone();
    let mut edits = edits.clone();
    edits.sort_by_key(|e| std::cmp::Reverse(offset(e.span.start)));
    for e in edits {
        made.replace_range(offset(e.span.start)..offset(e.span.end), &e.text);
    }
    assert!(
        made.contains("    public int Count { get; set; }\n")
            && made.contains("Greeting { get; set; } = \"Hello\";\n")
            && made.trim_end().ends_with("Missing { get; set; }\n}"),
        "the fix's edit was placed by the file on disk, not the editor's text:\n{made}"
    );
}

#[gpui::test]
fn a_key_one_theme_variant_lacks_is_warned_of_and_the_fix_adds_it_to_that_variant(cx: &mut TestAppContext) {
    let window_text = "<Window xmlns=\"https://github.com/avaloniaui\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n        x:Class=\"Demo.MainWindow\">\n  <Button Content=\"Go\" Foreground=\"{DynamicResource Danger}\" />\n</Window>\n";
    let Some(run) = restored(
        "xaml-variants",
        &[
            ("Demo.csproj", PROJECT),
            ("App.axaml", APP),
            ("App.axaml.cs", "namespace Demo;\n\npublic partial class App : Avalonia.Application { }\n"),
            ("Styles/Palette.axaml", PALETTE),
            ("MainWindow.axaml", window_text),
            ("MainWindow.axaml.cs", CODE),
        ],
    ) else {
        return;
    };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let (view, palette) = (run.folder.join("MainWindow.axaml"), run.folder.join("Styles/Palette.axaml"));
    main.update_in(cx, |m, window, cx| {
        m.open_workspace(&run.folder, window, cx);
        m.open_file(&view, None, window, cx);
        m.open_file(&palette, None, window, cx);
    });
    let buffer_of = |path: &std::path::PathBuf, cx: &mut VisualTestContext| {
        cx.update(|_, cx| rsharp_app::buffer::find(path, cx)).unwrap()
    };
    let messages = |path: &std::path::PathBuf, cx: &mut VisualTestContext| {
        let buffer = buffer_of(path, cx);
        buffer.read_with(cx, |b, _| b.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>())
    };

    // Dark gets a key Light lacks, as typed: warned where it is defined and, from the palette as typed, where it is used.
    let added = "\n            <SolidColorBrush x:Key=\"Danger\" Color=\"#E55765\" />";
    let dark_muted = "<SolidColorBrush x:Key=\"Muted\" Color=\"#868A91\" />";
    buffer_of(&palette, cx)
        .update(cx, |b, cx| b.replace_all(PALETTE.replace(dark_muted, &format!("{dark_muted}{added}")), cx));
    wait(cx, "the warning on the key only Dark defines", |cx| !messages(&palette, cx).is_empty());
    assert_eq!(messages(&palette, cx), ["'Danger' is defined for Dark but not for Light"]);
    wait(cx, "the warning on the window's use of it", |cx| !messages(&view, cx).is_empty());
    assert_eq!(messages(&view, cx), ["'Danger' is not defined for Light"]);

    // Alt+Enter on the key's line offers the fix, which adds it to Light; both warnings go.
    main.update_in(cx, |m, window, cx| m.open_file(&palette, Some((6, 40)), window, cx));
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "alt-enter" } else { "ctrl-." });
    wait(cx, "the fix offered", |cx| editor.read_with(cx, |e, _| e.actions_shown().is_some()));
    assert_eq!(editor.read_with(cx, |e, _| e.actions_shown()).unwrap(), ["Add 'Danger' to Light"]);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let fixed = buffer_of(&palette, cx).read_with(cx, |b, _| b.document.text().to_string());
    assert!(
        fixed.contains("<SolidColorBrush x:Key=\"Muted\" Color=\"#6C707E\" />\n            <SolidColorBrush x:Key=\"Danger\" Color=\"#E55765\" />\n        </ResourceDictionary>"),
        "the fix did not add the key to Light:\n{fixed}"
    );
    wait(cx, "both warnings gone", |cx| messages(&palette, cx).is_empty() && messages(&view, cx).is_empty());
}

#[gpui::test]
fn an_avares_path_completes_from_the_projects_assets_is_warned_of_when_missing_and_goes_to_its_file(
    cx: &mut TestAppContext,
) {
    let project = PROJECT.replace(
        "</ItemGroup>",
        "</ItemGroup>\n  <ItemGroup>\n    <AvaloniaResource Include=\"Assets\\**\" />\n  </ItemGroup>",
    );
    let Some(run) = restored(
        "xaml-assets",
        &[
            ("Demo.csproj", &project),
            ("App.axaml", APP),
            ("App.axaml.cs", "namespace Demo;\n\npublic partial class App : Avalonia.Application { }\n"),
            ("Styles/Palette.axaml", PALETTE),
            ("MainWindow.axaml", WINDOW),
            ("MainWindow.axaml.cs", CODE),
            ("MainViewModel.cs", VIEW_MODEL),
            ("Assets/logo.png", "\u{89}PNG\r\n\u{1a}\n"),
        ],
    ) else {
        return;
    };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let window = run.folder.join("MainWindow.axaml");
    main.update_in(cx, |m, window_, cx| {
        m.open_workspace(&run.folder, window_, cx);
        m.open_file(&window, Some((5, 38)), window_, cx);
    });

    // A source's quote offers the project's assets as avares:// paths, narrowed as typed.
    cx.simulate_keystrokes("enter");
    cx.simulate_input("<Image Source=");
    cx.simulate_input("\"");
    offered(&main, cx, "the project's asset as an avares:// path", "avares://Demo/Assets/logo.png");
    assert!(shown(&main, cx).iter().any(|l| l == "avares://Demo/Styles/Palette.axaml"), "{:?}", shown(&main, cx));
    for c in "logo".chars() {
        cx.simulate_input(&c.to_string());
    }
    cx.run_until_parked();
    assert_eq!(shown(&main, cx).first().map(String::as_str), Some("avares://Demo/Assets/logo.png"));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    // shortcut: RSharp's editor pairs no quotes (Passenger's closes a typed one), so the closing quote is typed here.
    assert!(
        caret_line(&main, cx).ends_with("<Image Source=\"avares://Demo/Assets/logo.png|"),
        "the path did not go in: {}",
        caret_line(&main, cx)
    );
    cx.simulate_input("\"");
    cx.simulate_input(" />");
    let buffer = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).buffer.clone());
    let no_problem = |cx: &mut VisualTestContext| buffer.read_with(cx, |b, _| b.diagnostics.is_empty());
    pump(cx, Duration::from_secs(2));
    assert!(no_problem(cx), "an asset that is there was marked");

    // An asset that is not there is warned of.
    let text = buffer.read_with(cx, |b, _| b.document.text().to_string());
    let text = text.replace("logo.png\"", "logo2.png\"");
    buffer.update(cx, |b, cx| b.replace_all(text, cx));
    wait(cx, "the warning on the asset that is not there", |cx| buffer.read_with(cx, |b, _| !b.diagnostics.is_empty()));
    let marked =
        buffer.read_with(cx, |b, _| b.diagnostics.iter().map(|d| (d.message.clone(), d.error)).collect::<Vec<_>>());
    assert_eq!(marked, [("'avares://Demo/Assets/logo2.png' is not found".to_string(), false)]);

    // Go to Declaration on an include's path opens the file it names.
    main.update_in(cx, |m, window_, cx| m.open_file(&run.folder.join("App.axaml"), Some((6, 50)), window_, cx));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-b" } else { "f12" });
    wait(cx, "Palette.axaml opened", |cx| {
        main.read_with(cx, |m, cx| m.editor().is_some_and(|e| e.read(cx).path.ends_with("Styles/Palette.axaml")))
    });
}

/// The window's StackPanel written badly: indents astray, attributes over lines, blank lines, a comment, a character
/// reference, single quotes and an empty element.
const UNTIDY: &str = "<StackPanel   Margin=\"12\">\n<!-- the\n     greeting -->\n      <TextBlock Text='Fish &amp; chips' />\n<Button Grid.Row=\"1\" Content=\"Go\"\n  x:Name=\"Pay\"\n     HorizontalAlignment=\"Center\"/>\n\n\n\n  <Border></Border>\n</StackPanel >";

#[gpui::test]
fn reformat_code_lays_xaml_out_by_riders_code_style_and_the_editorconfig_changing_nothing_but_whitespace(
    cx: &mut TestAppContext,
) {
    let tidy = "<Window xmlns=\"https://github.com/avaloniaui\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n        x:Class=\"Demo.MainWindow\">\n    <StackPanel Margin=\"12\">\n        <TextBlock Text=\"Hi\" />\n    </StackPanel>\n</Window>\n";
    let Some(run) =
        restored("xaml-format", &[("Demo.csproj", PROJECT), ("MainWindow.axaml", tidy), ("MainWindow.axaml.cs", CODE)])
    else {
        return;
    };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let window = run.folder.join("MainWindow.axaml");
    main.update_in(cx, |m, window_, cx| {
        m.open_workspace(&run.folder, window_, cx);
        m.open_file(&window, None, window_, cx);
    });
    let reformat = if cfg!(target_os = "macos") { "cmd-alt-l" } else { "ctrl-alt-enter" };
    let said = |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.said().map(str::to_string));
    let buffer = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).buffer.clone());
    let text = |cx: &mut VisualTestContext| buffer.read_with(cx, |b, _| b.document.text().to_string());
    // A file laid out as Rider lays it out is left as it is, once the host has read the project.
    wait(cx, "the window checked", |cx| {
        cx.simulate_keystrokes(reformat);
        said(cx).as_deref() == Some("Already formatted")
    });

    let (start, end) = (tidy.find("<StackPanel").unwrap(), tidy.find("</StackPanel>").unwrap() + "</StackPanel>".len());
    let untidy = format!("{}{UNTIDY}{}", &tidy[..start], &tidy[end..]);
    buffer.update(cx, |b, cx| b.replace_all(untidy.clone(), cx));
    main.update_in(cx, |m, window_, cx| m.open_file(&window, Some((0, 0)), window_, cx));
    cx.simulate_keystrokes(reformat);
    wait(cx, "the reformat", |cx| said(cx).as_deref() == Some("Reformatted"));
    // Rider's defaults: attributes as they were broken over lines, aligned with the first; every child a line of its
    // own, indented from its parent, at most two blank lines; the comment moved whole; values and quotes as written.
    let expected = format!(
        "{}{}{}",
        &tidy[..start],
        "<StackPanel Margin=\"12\">\n        <!-- the\n             greeting -->\n        <TextBlock Text='Fish &amp; chips' />\n        <Button Grid.Row=\"1\" Content=\"Go\"\n                x:Name=\"Pay\"\n                HorizontalAlignment=\"Center\" />\n\n\n        <Border></Border>\n    </StackPanel>",
        &tidy[end..]
    );
    assert_eq!(text(cx), expected, "Reformat Code did not lay the panel out as Rider does");
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert_eq!(text(cx), untidy, "one Undo did not take the whole reformat back");

    // The Button's lines alone: the comment above them keeps its column.
    let button = untidy[..untidy.find("<Button").unwrap()].matches('\n').count();
    main.update_in(cx, |m, window_, cx| m.open_file(&window, Some((button, 0)), window_, cx));
    cx.simulate_keystrokes("shift-down shift-down shift-end");
    cx.simulate_keystrokes(reformat);
    wait(cx, "the selection reformatted", |cx| text(cx) != untidy);
    let selected = text(cx);
    assert!(
        selected.contains("<StackPanel   Margin=\"12\">\n<!-- the\n"),
        "lines outside the selection moved:\n{selected}"
    );
    assert!(
        selected.contains("\n        <Button Grid.Row=\"1\" Content=\"Go\"\n                x:Name=\"Pay\"\n                HorizontalAlignment=\"Center\" />\n"),
        "the selected lines were not laid out:\n{selected}"
    );
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();

    // The .editorconfig, by Rider's keys and XAML Styler's: each attribute on a line of its own, one indent in, in XAML
    // Styler's order, and an empty element closed.
    std::fs::write(
        run.folder.join(".editorconfig"),
        "root = true\n\n[*.{axaml,xaml}]\nxml_attribute_style = on_different_lines\nxml_attribute_indent = single_indent\nxaml_enable_attribute_reordering = true\nxaml_remove_ending_tag_of_empty_element = true\n",
    )
    .unwrap();
    main.update_in(cx, |m, window_, cx| m.open_file(&window, Some((0, 0)), window_, cx));
    cx.simulate_keystrokes(reformat);
    wait(cx, "the reformat by the .editorconfig", |cx| text(cx) != untidy);
    let configured = text(cx);
    let panel = &configured[configured.find("<StackPanel").unwrap()..];
    assert_eq!(
        panel,
        format!(
            "{}{}",
            "<StackPanel Margin=\"12\">\n        <!-- the\n             greeting -->\n        <TextBlock Text='Fish &amp; chips' />\n        <Button\n            x:Name=\"Pay\"\n            Grid.Row=\"1\"\n            HorizontalAlignment=\"Center\"\n            Content=\"Go\" />\n\n\n        <Border />\n    </StackPanel>",
            &tidy[end..]
        ),
        "the .editorconfig's style was not followed"
    );
    assert!(
        configured.starts_with("<Window\n    x:Class=\"Demo.MainWindow\"\n    xmlns=\""),
        "the root's attributes:\n{configured}"
    );
}

#[gpui::test]
fn surround_with_wraps_the_element_at_the_caret_or_the_elements_chosen_in_a_layout_element(cx: &mut TestAppContext) {
    let (title, button) = ("<TextBlock Name=\"Title\" Text=\"Hello\" />", "<Button Name=\"Go\" Content=\"Go\" />");
    let window_text = format!(
        "<Window xmlns=\"https://github.com/avaloniaui\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n        x:Class=\"Demo.MainWindow\">\n    <StackPanel Margin=\"12\" Spacing=\"8\">\n        {title}\n        {button}\n    </StackPanel>\n</Window>\n"
    );
    let Some(run) = restored(
        "xaml-surround",
        &[("Demo.csproj", PROJECT), ("MainWindow.axaml", &window_text), ("MainWindow.axaml.cs", CODE)],
    ) else {
        return;
    };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let window = run.folder.join("MainWindow.axaml");
    main.update_in(cx, |m, window_, cx| {
        m.open_workspace(&run.folder, window_, cx);
        m.open_file(&window, Some((5, 13)), window_, cx);
    });
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    let surround = if cfg!(target_os = "macos") { "cmd-alt-t" } else { "ctrl-k ctrl-s" };
    let listed = |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.surrounds_shown());
    let text = |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.text().to_string());

    // The element at the caret: Avalonia's layout elements, each chosen by its digit; 4 is Border, the caret then in
    // its start tag.
    cx.simulate_keystrokes(surround);
    wait(cx, "Surround With's templates", |cx| listed(cx).is_some());
    assert_eq!(
        listed(cx).unwrap(),
        ["tag", "Grid", "StackPanel", "Border", "ScrollViewer", "DockPanel", "WrapPanel", "Canvas", "Viewbox", "Panel"]
    );
    cx.simulate_input("4");
    cx.run_until_parked();
    assert!(
        text(cx).contains(&format!("        <Border>\n            {button}\n        </Border>\n")),
        "the Button was not wrapped in a Border:\n{}",
        text(cx)
    );
    let caret = editor.read_with(cx, |e, _| e.caret());
    assert_eq!((caret.line, caret.column), (5, 15), "the caret is not in the Border's start tag");
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert_eq!(text(cx), window_text, "one Undo did not take the surround back");

    // Two elements chosen: what holds one child alone is not offered; Rider's tag, its name typed in both tags.
    main.update_in(cx, |m, window_, cx| m.open_file(&window, Some((4, 11)), window_, cx));
    cx.simulate_keystrokes("shift-down shift-right");
    cx.simulate_keystrokes(surround);
    wait(cx, "Surround With's templates for two elements", |cx| listed(cx).is_some());
    let offered = listed(cx).unwrap();
    assert!(!offered.iter().any(|o| o == "Border" || o == "ScrollViewer"), "one-child elements offered: {offered:?}");
    cx.simulate_input("1");
    cx.run_until_parked();
    cx.simulate_input("DockPanel");
    cx.run_until_parked();
    assert!(
        text(cx).contains(&format!(
            "        <DockPanel>\n            {title}\n            {button}\n        </DockPanel>\n"
        )),
        "the tag's name was not typed in both tags:\n{}",
        text(cx)
    );
}

#[gpui::test]
fn extract_resource_moves_a_value_into_this_files_or_the_applications_resources_and_extract_style_its_properties(
    cx: &mut TestAppContext,
) {
    use rsharp_app::editor::{ExtractResource, ExtractStyle};
    let window_text = "<Window xmlns=\"https://github.com/avaloniaui\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n        x:Class=\"Demo.MainWindow\"\n        Title=\"Gallery\">\n    <StackPanel Margin=\"12\" Spacing=\"8\">\n        <TextBlock Name=\"Title\" Text=\"Hello\" />\n        <Button Name=\"Go\" Content=\"Go\" Margin=\"12\" Background=\"{StaticResource Accent}\" />\n    </StackPanel>\n</Window>\n";
    let Some(run) = restored(
        "xaml-extract",
        &[
            ("Demo.csproj", PROJECT),
            ("App.axaml", APP),
            ("App.axaml.cs", "namespace Demo;\n\npublic partial class App : Avalonia.Application { }\n"),
            ("Styles/Palette.axaml", PALETTE),
            ("MainWindow.axaml", window_text),
            ("MainWindow.axaml.cs", CODE),
        ],
    ) else {
        return;
    };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let (window, app) = (run.folder.join("MainWindow.axaml"), run.folder.join("App.axaml"));
    main.update_in(cx, |m, window_, cx| {
        m.open_workspace(&run.folder, window_, cx);
        m.open_file(&window, Some((4, 24)), window_, cx);
    });
    let buffer = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).buffer.clone());
    let text = |cx: &mut VisualTestContext| buffer.read_with(cx, |b, _| b.document.text().to_string());
    let said = |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.said().map(str::to_string));
    let dialog = |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.extracting());
    let clean = |cx: &mut VisualTestContext| buffer.read_with(cx, |b, _| b.diagnostics.is_empty());

    // The panel's margin: the name suggested, the places the window reaches, its other use offered too.
    wait(cx, "Extract Resource's dialog", |cx| {
        if dialog(cx).is_none() {
            cx.dispatch_action(ExtractResource);
        }
        dialog(cx).is_some()
    });
    let resource = dialog(cx).unwrap();
    assert_eq!(resource.read_with(cx, |d, cx| d.name(cx)), "StackPanelMargin");
    assert_eq!(
        resource.read_with(cx, |d, _| d.places().to_vec()),
        [
            "MainWindow.axaml <Window.Resources>",
            "App.axaml <Application.Resources>",
            "Palette.axaml <ResourceDictionary>"
        ]
    );
    assert_eq!(resource.read_with(cx, |d, _| d.every_use()), Some(true), "the value's other use was not offered");
    resource.update(cx, |d, cx| {
        d.set_name("Accent", cx);
        d.accept(cx);
    });
    wait(cx, "the taken key refused", |cx| resource.read_with(cx, |d, _| d.problem().is_some()));
    assert_eq!(
        resource.read_with(cx, |d, _| d.problem().map(str::to_string)).unwrap(),
        "Resource with the same name already exists"
    );
    resource.update(cx, |d, cx| {
        d.set_name("PanelMargin", cx);
        d.accept(cx);
    });
    wait(cx, "the resource extracted", |cx| dialog(cx).is_none());
    assert!(
        text(cx).contains("Title=\"Gallery\">\n    <Window.Resources>\n        <Thickness x:Key=\"PanelMargin\">12</Thickness>\n    </Window.Resources>\n    <StackPanel Margin=\"{StaticResource PanelMargin}\""),
        "the resource was not made in the window's resources:\n{}",
        text(cx)
    );
    assert!(
        text(cx).contains("<Button Name=\"Go\" Content=\"Go\" Margin=\"{StaticResource PanelMargin}\""),
        "{}",
        text(cx)
    );
    assert_eq!(
        said(cx).as_deref(),
        Some("Extracted resource 'PanelMargin' to MainWindow.axaml <Window.Resources>, used in 2 places")
    );
    wait(cx, "the file, its new resource resolved, marked with nothing", clean);
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert!(!text(cx).contains("PanelMargin"), "Undo did not take the extraction back:\n{}", text(cx));

    // A brush to the application's resources.
    let with_brush = text(cx).replace("<TextBlock Name=\"Title\"", "<TextBlock Name=\"Title\" Foreground=\"#E2A33F\"");
    buffer.update(cx, |b, cx| b.replace_all(with_brush, cx));
    main.update_in(cx, |m, window_, cx| m.open_file(&window, Some((5, 45)), window_, cx));
    cx.dispatch_action(ExtractResource);
    wait(cx, "Extract Resource's dialog on the brush", |cx| dialog(cx).is_some());
    let resource = dialog(cx).unwrap();
    assert_eq!(resource.read_with(cx, |d, cx| d.name(cx)), "ForegroundBrush");
    resource.update(cx, |d, cx| {
        d.choose_place(1, cx);
        d.accept(cx);
    });
    wait(cx, "the brush extracted", |cx| dialog(cx).is_none());
    let app_text = std::fs::read_to_string(&app).unwrap();
    assert!(
        app_text.contains("</ResourceDictionary.MergedDictionaries>\n")
            && app_text.contains("<SolidColorBrush x:Key=\"ForegroundBrush\" Color=\"#E2A33F\" />"),
        "the brush is not among the application's resources:\n{app_text}"
    );
    assert!(text(cx).contains("Foreground=\"{StaticResource ForegroundBrush}\""), "{}", text(cx));
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();

    // The button's margin and background to a style selecting a class it takes.
    main.update_in(cx, |m, window_, cx| m.open_file(&window, Some((6, 13)), window_, cx));
    cx.dispatch_action(ExtractStyle);
    wait(cx, "Extract Style's dialog", |cx| dialog(cx).is_some());
    let style = dialog(cx).unwrap();
    assert_eq!(style.read_with(cx, |d, cx| d.name(cx)), "buttonStyle");
    let properties: Vec<String> = style.read_with(cx, |d, _| d.properties()).into_iter().map(|(p, _)| p).collect();
    assert_eq!(properties, ["Content = \"Go\"", "Margin = \"12\"", "Background = \"{StaticResource Accent}\""]);
    style.update(cx, |d, cx| {
        for i in 0..3 {
            d.toggle_property(i, cx);
        }
        d.accept(cx);
    });
    assert_eq!(
        style.read_with(cx, |d, _| d.problem().map(str::to_string)).as_deref(),
        Some("At least one property must be selected")
    );
    style.update(cx, |d, cx| {
        d.toggle_property(1, cx);
        d.toggle_property(2, cx);
        d.accept(cx);
    });
    wait(cx, "the style extracted", |cx| dialog(cx).is_none());
    let made = text(cx);
    assert!(
        made.contains("Title=\"Gallery\">\n    <Window.Styles>\n        <Style Selector=\"Button.buttonStyle\">\n            <Setter Property=\"Margin\" Value=\"12\" />\n            <Setter Property=\"Background\" Value=\"{StaticResource Accent}\" />\n        </Style>\n    </Window.Styles>\n"),
        "the style was not made in the window's styles:\n{made}"
    );
    assert!(made.contains("<Button Name=\"Go\" Content=\"Go\" Classes=\"buttonStyle\" />"), "{made}");
    wait(cx, "the style and the button that takes it marked with nothing", clean);
}

const MVVM: &str = "using System.ComponentModel;\nusing System.Runtime.CompilerServices;\n\nnamespace CommunityToolkit.Mvvm.ComponentModel\n{\n    [System.AttributeUsage(System.AttributeTargets.Field)]\n    public sealed class ObservablePropertyAttribute : System.Attribute\n    {\n    }\n\n    public class ObservableObject : INotifyPropertyChanged\n    {\n        public event PropertyChangedEventHandler? PropertyChanged;\n\n        protected bool SetProperty<T>(ref T field, T value, [CallerMemberName] string? name = null)\n        {\n            field = value;\n            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));\n            return true;\n        }\n    }\n}\n\nnamespace CommunityToolkit.Mvvm.Input\n{\n    [System.AttributeUsage(System.AttributeTargets.Method)]\n    public sealed class RelayCommandAttribute : System.Attribute\n    {\n    }\n}\n";

/// Where `needle` ends in `text`, as the line and column a file is opened at.
fn after(text: &str, needle: &str) -> (usize, usize) {
    let end = text.find(needle).unwrap_or_else(|| panic!("'{needle}' is not in the file")) + needle.len();
    let line = text[..end].matches('\n').count();
    (line, end - text[..end].rfind('\n').map_or(0, |i| i + 1))
}

#[gpui::test]
fn a_binding_path_and_a_converter_nothing_has_offer_to_create_them_in_the_view_model_and_a_converter_class(
    cx: &mut TestAppContext,
) {
    let window_text = "<Window xmlns=\"https://github.com/avaloniaui\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n        xmlns:vm=\"using:Demo.ViewModels\"\n        x:Class=\"Demo.MainWindow\"\n        x:DataType=\"vm:MainViewModel\">\n    <StackPanel Margin=\"12\" Spacing=\"8\">\n        <TextBlock Text=\"{Binding Greeting}\" />\n        <Button Name=\"Go\" Content=\"Go\" />\n        <TextBlock Text=\"{Binding #Go.Content}\" />\n    </StackPanel>\n</Window>\n";
    let model_text = "namespace Demo.ViewModels;\n\npublic class MainViewModel\n{\n    public string Greeting { get; set; } = \"Hello\";\n\n    public int Count { get; set; }\n}\n";
    let shop_view = "<UserControl xmlns=\"https://github.com/avaloniaui\"\n             xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n             xmlns:vm=\"using:Demo.ViewModels\"\n             x:Class=\"Demo.ShopView\"\n             x:DataType=\"vm:ShopViewModel\">\n    <StackPanel>\n        <TextBlock Text=\"{Binding Name}\" />\n    </StackPanel>\n</UserControl>\n";
    let shop_model = "using CommunityToolkit.Mvvm.ComponentModel;\n\nnamespace Demo.ViewModels;\n\npublic class ShopViewModel : ObservableObject\n{\n    [ObservableProperty]\n    private string? _name;\n}\n";
    let Some(run) = restored(
        "xaml-create",
        &[
            ("Demo.csproj", PROJECT),
            ("MainWindow.axaml", window_text),
            ("MainWindow.axaml.cs", CODE),
            ("ViewModels/MainViewModel.cs", model_text),
            ("ViewModels/ShopViewModel.cs", shop_model),
            ("Mvvm.cs", MVVM),
            ("ShopView.axaml", shop_view),
            (
                "ShopView.axaml.cs",
                "namespace Demo;\n\npublic partial class ShopView : Avalonia.Controls.UserControl { }\n",
            ),
        ],
    ) else {
        return;
    };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let (window, model) = (run.folder.join("MainWindow.axaml"), run.folder.join("ViewModels/MainViewModel.cs"));
    main.update_in(cx, |m, window_, cx| {
        m.open_workspace(&run.folder, window_, cx);
        m.open_file(&window, None, window_, cx);
    });
    let quick_fix = if cfg!(target_os = "macos") { "alt-enter" } else { "ctrl-." };
    // The fix offered first on the line after `needle`, chosen, in the file `path` open in the editor.
    let fix = |path: &std::path::PathBuf, needle: &str, offered: &str, cx: &mut VisualTestContext| {
        let buffer = cx.update(|_, cx| rsharp_app::buffer::find(path, cx)).unwrap();
        let place = after(&buffer.read_with(cx, |b, _| b.document.text().to_string()), needle);
        main.update_in(cx, |m, window_, cx| m.open_file(path, Some(place), window_, cx));
        let editor = main.read_with(cx, |m, _| m.editor().unwrap());
        cx.simulate_keystrokes(quick_fix);
        wait(cx, &format!("the fixes offered after {needle}"), |cx| {
            editor.read_with(cx, |e, _| e.actions_shown().is_some())
        });
        let first = editor.read_with(cx, |e, _| e.actions_shown()).unwrap().first().cloned();
        assert_eq!(first.as_deref(), Some(offered), "the first fix offered after {needle}");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
    };
    let buffer = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).buffer.clone());
    let marked = |text: &str, cx: &mut VisualTestContext| {
        buffer.read_with(cx, |b, _| b.diagnostics.iter().any(|d| d.message.contains(text)))
    };
    let edit = |from: &str, to: &str, cx: &mut VisualTestContext| {
        let text = buffer.read_with(cx, |b, _| b.document.text().to_string()).replace(from, to);
        buffer.update(cx, |b, cx| b.replace_all(text, cx));
    };
    let disk = |path: &std::path::PathBuf| std::fs::read_to_string(path).unwrap_or_default();

    // A property the view model lacks, typed as the TextBlock's Text reads it.
    edit("{Binding Greeting}", "{Binding Subtitle}", cx);
    wait(cx, "the binding path the view model lacks marked", |cx| marked("'Subtitle'", cx));
    fix(&window, "{Binding Subt", "Create property 'Subtitle'", cx);
    wait(cx, "the property written", |_| {
        disk(&model).contains("    public int Count { get; set; }\n\n    public string? Subtitle { get; set; }\n}")
    });
    wait(cx, "the binding resolved once the property is there", |cx| !marked("'Subtitle'", cx));

    // A command for a button's Command: an ICommand property, its using added.
    edit(
        "<Button Name=\"Go\" Content=\"Go\"",
        "<Button Name=\"Go\" Content=\"Go\" Command=\"{Binding SaveCommand}\"",
        cx,
    );
    wait(cx, "the command the view model lacks marked", |cx| marked("'SaveCommand'", cx));
    fix(&window, "{Binding Save", "Create property 'SaveCommand'", cx);
    wait(cx, "the command written", |_| {
        disk(&model)
            .contains("    public string? Subtitle { get; set; }\n\n    public ICommand? SaveCommand { get; set; }\n}")
    });
    assert!(disk(&model).contains("using System.Windows.Input;\n\nnamespace Demo.ViewModels;"), "{}", disk(&model));
    wait(cx, "the command resolved once it is there", |cx| !marked("'SaveCommand'", cx));

    // A converter nothing defines: its class in a file of its own and its resource.
    edit(
        "<TextBlock Text=\"{Binding #Go.Content}\" />",
        "<TextBlock Text=\"{Binding #Go.Content}\" />\n        <TextBlock Text=\"{Binding Greeting, Converter={StaticResource ShoutConverter}}\" />",
        cx,
    );
    wait(cx, "the converter nothing defines marked", |cx| marked("Resource 'ShoutConverter' is not found", cx));
    fix(&window, "{StaticResource Shout", "Create converter 'ShoutConverter'", cx);
    let converter = run.folder.join("ShoutConverter.cs");
    wait(cx, "ShoutConverter.cs made", |_| converter.exists());
    let made = disk(&converter);
    assert!(made.contains("namespace Demo;\n\npublic class ShoutConverter : IValueConverter\n{\n"), "{made}");
    assert!(made.contains("using Avalonia.Data.Converters;"), "{made}");
    assert!(
        made.contains("    public object? Convert(object? value, Type targetType, object? parameter, CultureInfo culture)\n    {\n        throw new NotImplementedException();\n    }\n"),
        "{made}"
    );
    let view = buffer.read_with(cx, |b, _| b.document.text().to_string());
    assert!(view.contains("xmlns:vm=\"using:Demo.ViewModels\"\n        xmlns:local=\"using:Demo\""), "{view}");
    assert!(view.contains("<local:ShoutConverter x:Key=\"ShoutConverter\" />"), "{view}");
    wait(cx, "the converter, its class read, resolved and the window marked with nothing", |cx| {
        buffer.read_with(cx, |b, _| b.diagnostics.is_empty())
    });

    // With the MVVM Toolkit, an [ObservableProperty] field and a [RelayCommand] method, the class made partial.
    let (shop, shop_model_path) = (run.folder.join("ShopView.axaml"), run.folder.join("ViewModels/ShopViewModel.cs"));
    main.update_in(cx, |m, window_, cx| m.open_file(&shop, None, window_, cx));
    let shop_buffer = cx.update(|_, cx| rsharp_app::buffer::find(&shop, cx)).unwrap();
    let text = shop_buffer.read_with(cx, |b, _| b.document.text().to_string()).replace(
        "<TextBlock Text=\"{Binding Name}\" />",
        "<TextBlock Text=\"{Binding Name}\" />\n        <TextBlock Text=\"{Binding Title}\" />\n        <Button Command=\"{Binding BuyCommand}\" />",
    );
    shop_buffer.update(cx, |b, cx| b.replace_all(text, cx));
    wait(cx, "the property and the command the shop's view model lacks marked", |cx| {
        shop_buffer.read_with(cx, |b, _| b.diagnostics.len() == 2)
    });
    fix(&shop, "{Binding Tit", "Create property 'Title'", cx);
    wait(cx, "the property field written", |_| disk(&shop_model_path).contains("_title"));
    fix(&shop, "{Binding Buy", "Create property 'BuyCommand'", cx);
    wait(cx, "the command method written", |_| disk(&shop_model_path).contains("Buy()"));
    assert_eq!(
        disk(&shop_model_path),
        "using CommunityToolkit.Mvvm.ComponentModel;\nusing CommunityToolkit.Mvvm.Input;\n\nnamespace Demo.ViewModels;\n\npublic partial class ShopViewModel : ObservableObject\n{\n    [ObservableProperty]\n    private string? _name;\n\n    [ObservableProperty]\n    private string? _title;\n\n    [RelayCommand]\n    private void Buy()\n    {\n    }\n}\n"
    );
    wait(cx, "the generated property and command resolved", |cx| {
        shop_buffer.read_with(cx, |b, _| b.diagnostics.is_empty())
    });
}
