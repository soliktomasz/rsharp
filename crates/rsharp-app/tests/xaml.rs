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
