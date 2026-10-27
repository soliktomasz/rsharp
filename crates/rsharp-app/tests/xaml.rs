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
fn a_package_restored_while_its_xaml_is_open_has_its_types_read_once_the_restore_lands(cx: &mut TestAppContext) {
    let text = "<Window xmlns=\"https://github.com/avaloniaui\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n        xmlns:h=\"using:Avalonia.Headless\"\n        x:Class=\"Demo.MainWindow\">\n    <Window.Resources>\n        <h:AvaloniaHeadlessPlatformOptions x:Key=\"Headless\" />\n    </Window.Resources>\n    <TextBlock Text=\"Welcome\" />\n</Window>\n";
    let Some(run) = restored(
        "xaml-restore",
        &[("Demo.csproj", PROJECT), ("MainWindow.axaml", text), ("MainWindow.axaml.cs", CODE)],
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
    wait(cx, "the type of a package the app does not reference marked", |cx| {
        buffer.read_with(cx, |b, _| b.diagnostics.iter().any(|d| d.message.contains("AvaloniaHeadlessPlatformOptions")))
    });

    // The package added where the project file does not name it, so only its restore says so, and restored as a
    // terminal's dotnet restore would: the restore's assets, under obj/, are what the XAML reads it from.
    std::fs::write(
        run.folder.join("Directory.Build.props"),
        "<Project>\n  <ItemGroup>\n    <PackageReference Include=\"Avalonia.Headless\" Version=\"12.1.4\" />\n  </ItemGroup>\n</Project>\n",
    )
    .unwrap();
    pump(cx, Duration::from_secs(1));
    let dotnet = rsharp_core::programs::dotnet().unwrap();
    let restore = rsharp_core::dotnet::command(&dotnet, ["restore", "-v", "q"], &run.folder).output().unwrap();
    assert!(restore.status.success(), "{}", String::from_utf8_lossy(&restore.stdout));
    wait(cx, "the package's type resolved once its restore landed", |cx| {
        buffer.read_with(cx, |b, _| b.diagnostics.is_empty())
    });
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
    // The quote typed came with its closer: the path goes in before it, and the quote typed next steps over it.
    assert!(
        caret_line(&main, cx).ends_with("<Image Source=\"avares://Demo/Assets/logo.png|\""),
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

    // Settings' Code Style: XAML, where no .editorconfig says otherwise: each attribute on a line of its own, one
    // indent in, in the order written.
    std::fs::remove_file(run.folder.join(".editorconfig")).unwrap();
    buffer.update(cx, |b, cx| b.replace_all(untidy.clone(), cx));
    cx.update(|_, cx| {
        let mut chosen = rsharp_app::settings::settings(cx).clone();
        chosen.xaml_format.attribute_style = rsharp_app::settings::AttributeStyle::OnDifferentLines;
        chosen.xaml_format.attribute_indent = rsharp_app::settings::AttributeIndent::SingleIndent;
        rsharp_app::settings::apply(chosen, cx);
    });
    main.update_in(cx, |m, window_, cx| m.open_file(&window, Some((0, 0)), window_, cx));
    cx.simulate_keystrokes(reformat);
    wait(cx, "the reformat by Settings' code style", |cx| text(cx) != untidy);
    let styled = text(cx);
    assert!(
        styled
            .contains("<Button\n            Grid.Row=\"1\"\n            Content=\"Go\"\n            x:Name=\"Pay\"\n"),
        "Settings' code style was not followed:\n{styled}"
    );

    // XAML Styler's rule groups as the .editorconfig writes them, their letter case kept: Content, then x:Name, then
    // the attributes no group names in the order written.
    std::fs::write(
        run.folder.join(".editorconfig"),
        "root = true\n\n[*.{axaml,xaml}]\nxaml_enable_attribute_reordering = true\nxaml_attribute_ordering_rule_groups = Content; x:Name\n",
    )
    .unwrap();
    buffer.update(cx, |b, cx| b.replace_all(untidy.clone(), cx));
    main.update_in(cx, |m, window_, cx| m.open_file(&window, Some((0, 0)), window_, cx));
    cx.simulate_keystrokes(reformat);
    wait(cx, "the reformat by the .editorconfig's rule groups", |cx| text(cx) != untidy);
    let ordered = text(cx);
    assert!(
        ordered
            .contains("<Button\n            Content=\"Go\"\n            x:Name=\"Pay\"\n            Grid.Row=\"1\"\n"),
        "the rule groups lost their letter case:\n{ordered}"
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

    // The file edited while the dialog is open: its edit, made from the file as it was, is refused.
    let (line, column) = after(&text(cx), "Text=\"Hel");
    main.update_in(cx, |m, window_, cx| m.open_file(&window, Some((line, column)), window_, cx));
    cx.dispatch_action(ExtractResource);
    wait(cx, "Extract Resource's dialog on the title", |cx| dialog(cx).is_some());
    let resource = dialog(cx).unwrap();
    let edited = text(cx).replace("Spacing=\"8\"", "Spacing=\"16\"");
    buffer.update(cx, |b, cx| b.replace_all(edited, cx));
    resource.update(cx, |d, cx| d.accept(cx));
    wait(cx, "the stale extraction refused", |cx| resource.read_with(cx, |d, _| d.problem().is_some()));
    assert_eq!(
        resource.read_with(cx, |d, _| d.problem().map(str::to_string)).as_deref(),
        Some("Extract Resource was not made: the file was edited meanwhile")
    );
    assert!(!text(cx).contains("StaticResource TextBlockText"), "the stale edit was applied:\n{}", text(cx));
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

// WPF's XAML (Passenger's WpfXamlTests), on Passenger's WPF gallery: a WinExe on net10.0-windows with UseWPF, which
// EnableWindowsTargeting restores and builds off Windows.

const WPF_PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <OutputType>WinExe</OutputType>\n    <TargetFramework>net10.0-windows</TargetFramework>\n    <UseWPF>true</UseWPF>\n    <EnableWindowsTargeting>true</EnableWindowsTargeting>\n    <Nullable>enable</Nullable>\n    <ImplicitUsings>enable</ImplicitUsings>\n  </PropertyGroup>\n</Project>\n";
const WPF_APP_CODE: &str =
    "using System.Windows;\n\nnamespace Gallery;\n\npublic partial class App : Application\n{\n}\n";
const WPF_WINDOW: &str = "<Window x:Class=\"Gallery.MainWindow\"\n        xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n        xmlns:d=\"http://schemas.microsoft.com/expression/blend/2008\"\n        xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"\n        xmlns:vm=\"clr-namespace:Gallery.ViewModels\"\n        mc:Ignorable=\"d\"\n        d:DataContext=\"{d:DesignInstance Type=vm:MainViewModel}\"\n        Title=\"Gallery\" Height=\"450\" Width=\"800\">\n    <Window.Resources>\n        <Style TargetType=\"Button\">\n            <Setter Property=\"Template\">\n                <Setter.Value>\n                    <ControlTemplate TargetType=\"Button\">\n                        <Border x:Name=\"Chrome\" Background=\"{TemplateBinding Background}\" CornerRadius=\"4\">\n                            <ContentPresenter HorizontalAlignment=\"Center\" />\n                        </Border>\n                        <ControlTemplate.Triggers>\n                            <Trigger Property=\"IsMouseOver\" Value=\"True\">\n                                <Setter TargetName=\"Chrome\" Property=\"Background\" Value=\"{DynamicResource Muted}\" />\n                            </Trigger>\n                        </ControlTemplate.Triggers>\n                    </ControlTemplate>\n                </Setter.Value>\n            </Setter>\n        </Style>\n    </Window.Resources>\n    <StackPanel Margin=\"12\">\n        <TextBlock Style=\"{StaticResource Heading}\" Text=\"{Binding Greeting}\" />\n        <Button x:Name=\"Go\" Content=\"Go\" Click=\"OnGo\" Background=\"{StaticResource Accent}\" />\n        <TextBlock Text=\"{Binding Content, ElementName=Go}\" />\n        <ListBox ItemsSource=\"{Binding People}\" Background=\"{x:Static SystemColors.ControlBrush}\">\n            <ListBox.ItemTemplate>\n                <DataTemplate>\n                    <TextBlock Text=\"{Binding Name}\" />\n                </DataTemplate>\n            </ListBox.ItemTemplate>\n        </ListBox>\n    </StackPanel>\n</Window>\n";
const WPF_APP: &str = "<Application x:Class=\"Gallery.App\"\n             xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\"\n             xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n             StartupUri=\"MainWindow.xaml\">\n    <Application.Resources>\n        <ResourceDictionary>\n            <ResourceDictionary.MergedDictionaries>\n                <ResourceDictionary Source=\"Themes/Brushes.xaml\" />\n            </ResourceDictionary.MergedDictionaries>\n            <Style x:Key=\"Heading\" TargetType=\"TextBlock\">\n                <Setter Property=\"FontSize\" Value=\"20\" />\n                <Setter Property=\"Foreground\" Value=\"{StaticResource Accent}\" />\n            </Style>\n        </ResourceDictionary>\n    </Application.Resources>\n</Application>\n";
const WPF_BRUSHES: &str = "<ResourceDictionary xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\"\n                    xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n                    xmlns:sys=\"clr-namespace:System;assembly=mscorlib\">\n    <SolidColorBrush x:Key=\"Accent\" Color=\"#3574F0\" />\n    <SolidColorBrush x:Key=\"Muted\" Color=\"#868A91\" />\n    <sys:Double x:Key=\"Gap\">8</sys:Double>\n</ResourceDictionary>\n";
const WPF_MAIN_VIEW_MODEL: &str = "using System.Collections.ObjectModel;\n\nnamespace Gallery.ViewModels;\n\npublic class MainViewModel\n{\n    public string Greeting { get; set; } = \"Hello\";\n\n    public Person Owner { get; } = new();\n\n    public ObservableCollection<Person> People { get; } = [];\n\n    public bool IsBusy { get; set; }\n}\n\npublic class Person\n{\n    public string Name { get; set; } = \"Ada\";\n\n    public int Age { get; set; }\n}\n";
const WPF_CODE_BEHIND: &str = "using System.Windows;\n\nnamespace Gallery;\n\npublic partial class MainWindow : Window\n{\n    public MainWindow()\n    {\n        InitializeComponent();\n        Go.Content = \"Start\";\n    }\n\n    private void OnGo(object sender, RoutedEventArgs e)\n    {\n    }\n}\n";
const WPF_LISTS_VIEW_MODEL: &str = "using System.Collections.ObjectModel;\n\nnamespace Gallery.ViewModels;\n\npublic class Node\n{\n    public string Title { get; set; } = \"\";\n\n    public ObservableCollection<Node> Children { get; } = [];\n}\n\npublic interface INamed\n{\n    string Name { get; }\n}\n\npublic interface IShopper : INamed\n{\n    int Visits { get; }\n}\n\npublic class ListsViewModel\n{\n    public ObservableCollection<Node> Roots { get; } = [];\n\n    public ObservableCollection<Person> People { get; } = [];\n\n    public IShopper? Shopper { get; set; }\n}\n";
const WPF_LISTS: &str = "<Window x:Class=\"Gallery.ListsWindow\"\n        xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n        xmlns:d=\"http://schemas.microsoft.com/expression/blend/2008\"\n        xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"\n        xmlns:vm=\"clr-namespace:Gallery.ViewModels\"\n        xmlns:sys=\"clr-namespace:System;assembly=mscorlib\"\n        mc:Ignorable=\"d\"\n        d:DataContext=\"{d:DesignInstance vm:ListsViewModel}\"\n        Title=\"Lists\" Height=\"450\" Width=\"800\">\n    <Window.Resources>\n        <ResourceDictionary>\n            <ResourceDictionary.MergedDictionaries>\n                <ResourceDictionary Source=\"pack://application:,,,/PresentationFramework.Aero2;component/themes/aero2.normalcolor.xaml\" />\n            </ResourceDictionary.MergedDictionaries>\n            <x:Array x:Key=\"Sizes\" Type=\"sys:Double\">\n                <sys:Double>10</sys:Double>\n                <sys:Double>12</sys:Double>\n            </x:Array>\n            <XmlDataProvider x:Key=\"Books\" XPath=\"Books\">\n                <x:XData>\n                    <Books xmlns=\"\">\n                        <Book Title=\"Dune\" />\n                    </Books>\n                </x:XData>\n            </XmlDataProvider>\n        </ResourceDictionary>\n    </Window.Resources>\n    <UniformGrid Columns=\"3\">\n        <TreeView ItemsSource=\"{Binding Roots}\">\n            <TreeView.ItemTemplate>\n                <HierarchicalDataTemplate ItemsSource=\"{Binding Children}\">\n                    <TextBlock Text=\"{Binding Title}\" />\n                </HierarchicalDataTemplate>\n            </TreeView.ItemTemplate>\n        </TreeView>\n        <DataGrid ItemsSource=\"{Binding People}\" AutoGenerateColumns=\"False\" Background=\"{StaticResource {x:Static SystemColors.WindowBrushKey}}\">\n            <DataGrid.Columns>\n                <DataGridTextColumn Header=\"Name\" Binding=\"{Binding Name}\" />\n                <DataGridTemplateColumn Header=\"Age\">\n                    <DataGridTemplateColumn.CellTemplate>\n                        <DataTemplate>\n                            <TextBlock Text=\"{Binding Age}\" />\n                        </DataTemplate>\n                    </DataGridTemplateColumn.CellTemplate>\n                </DataGridTemplateColumn>\n            </DataGrid.Columns>\n        </DataGrid>\n        <StackPanel>\n            <ListView ItemsSource=\"{Binding People}\">\n                <ListView.View>\n                    <GridView>\n                        <GridViewColumn Header=\"Age\" DisplayMemberBinding=\"{Binding Age}\" />\n                    </GridView>\n                </ListView.View>\n            </ListView>\n            <TextBlock Text=\"{Binding Shopper.Name}\" />\n            <ComboBox ItemsSource=\"{StaticResource Sizes}\" />\n            <ListBox ItemsSource=\"{Binding Source={StaticResource Books}, XPath=Book}\" DisplayMemberPath=\"@Title\" />\n        </StackPanel>\n    </UniformGrid>\n</Window>\n";
const WPF_LISTS_CODE_BEHIND: &str = "using System.Windows;\n\nnamespace Gallery;\n\npublic partial class ListsWindow : Window\n{\n    public ListsWindow()\n    {\n        InitializeComponent();\n    }\n}\n";

/// Passenger's WPF gallery, restored; None, said, without the XAML host or the WindowsDesktop reference pack.
fn wpf_gallery(name: &str) -> Option<Run> {
    restored(
        name,
        &[
            ("Gallery.csproj", WPF_PROJECT),
            ("App.xaml", WPF_APP),
            ("App.xaml.cs", WPF_APP_CODE),
            ("Themes/Brushes.xaml", WPF_BRUSHES),
            ("MainWindow.xaml", WPF_WINDOW),
            ("MainWindow.xaml.cs", WPF_CODE_BEHIND),
            ("ViewModels/MainViewModel.cs", WPF_MAIN_VIEW_MODEL),
            ("ViewModels/ListsViewModel.cs", WPF_LISTS_VIEW_MODEL),
            ("ListsWindow.xaml", WPF_LISTS),
            ("ListsWindow.xaml.cs", WPF_LISTS_CODE_BEHIND),
        ],
    )
}

/// The window opened on `file` of a restored WPF gallery, servers on, the editor's buffer and a reader of its text.
fn open_wpf<'a>(
    run: &Run,
    file: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<MainWindow>, &'a mut VisualTestContext, Entity<rsharp_app::buffer::Buffer>) {
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let path = run.folder.join(file);
    main.update_in(cx, |m, window, cx| {
        m.open_workspace(&run.folder, window, cx);
        m.open_file(&path, None, window, cx);
    });
    let buffer = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).buffer.clone());
    (main, cx, buffer)
}

/// The caret put just after `needle` in the open file `path`.
fn place_after(main: &Entity<MainWindow>, path: &std::path::Path, needle: &str, cx: &mut VisualTestContext) {
    let buffer = cx.update(|_, cx| rsharp_app::buffer::find(&path.to_path_buf(), cx)).unwrap();
    let place = after(&buffer.read_with(cx, |b, _| b.document.text().to_string()), needle);
    main.update_in(cx, |m, window, cx| m.open_file(path, Some(place), window, cx));
}

#[gpui::test]
fn typing_a_tag_offers_wpfs_controls_then_a_controls_properties_and_events_and_then_the_values_of_the_one_chosen(
    cx: &mut TestAppContext,
) {
    let Some(run) = wpf_gallery("wpf-complete") else { return };
    let (main, cx, _) = open_wpf(&run, "MainWindow.xaml", cx);
    let window = run.folder.join("MainWindow.xaml");
    place_after(&main, &window, "<StackPanel Margin=\"12\">", cx);
    cx.simulate_keystrokes("enter");
    cx.simulate_input("<");
    offered(&main, cx, "WPF's controls after a tag's '<'", "Button");
    assert!(shown(&main, cx).iter().any(|l| l == "Border"), "{:?}", shown(&main, cx));
    cx.simulate_input("Butt");
    cx.run_until_parked();
    assert_eq!(shown(&main, cx).first().map(String::as_str), Some("Button"));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(caret_line(&main, cx).ends_with("<Button|"), "{}", caret_line(&main, cx));

    cx.simulate_input(" ");
    cx.simulate_input("C");
    offered(&main, cx, "a Button's events among its attributes", "Click");
    for label in ["Command", "Content", "Canvas.Left"] {
        assert!(shown(&main, cx).iter().any(|l| l == label), "{label} not offered: {:?}", shown(&main, cx));
    }
    cx.simulate_keystrokes("escape backspace");
    cx.simulate_input("G");
    offered(&main, cx, "Grid's attached properties by their owner", "Grid.Row");
    assert!(!shown(&main, cx).iter().any(|l| l == "IsMouseOver"), "a property only a trigger reads was offered to set");
    cx.simulate_keystrokes("escape backspace");
    cx.simulate_input("H");
    offered(&main, cx, "a Button's dependency properties, its bases' included", "HorizontalAlignment");
    cx.simulate_input("orizontalA");
    cx.run_until_parked();
    assert_eq!(shown(&main, cx).first().map(String::as_str), Some("HorizontalAlignment"));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(caret_line(&main, cx).ends_with("<Button HorizontalAlignment=\"|\""), "{}", caret_line(&main, cx));
    offered(&main, cx, "the enum's members inside the new value's quotes", "Center");
    let mut values = shown(&main, cx);
    values.sort();
    assert_eq!(values, ["Center", "Left", "Right", "Stretch"]);
    cx.simulate_input("Ce");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(caret_line(&main, cx).ends_with("<Button HorizontalAlignment=\"Center|\""), "{}", caret_line(&main, cx));

    // A trigger's property may be one WPF sets itself; its setter's, one of the element its TargetName names.
    place_after(&main, &window, "<Trigger Property=\"", cx);
    cx.dispatch_action(rsharp_app::editor::Complete);
    offered(&main, cx, "the read-only dependency properties a trigger watches", "IsMouseOver");
    cx.simulate_keystrokes("escape");
    place_after(&main, &window, "<Setter TargetName=\"Chrome\" Property=\"", cx);
    cx.dispatch_action(rsharp_app::editor::Complete);
    offered(&main, cx, "the Border's properties, as TargetName names it", "CornerRadius");
}

#[gpui::test]
fn resources_complete_from_what_app_xaml_merges_in_and_a_binding_from_its_design_time_data_context_through_a_path(
    cx: &mut TestAppContext,
) {
    use rsharp_app::editor::Complete;
    let Some(run) = wpf_gallery("wpf-resources") else { return };
    let (main, cx, buffer) = open_wpf(&run, "MainWindow.xaml", cx);
    let window = run.folder.join("MainWindow.xaml");
    let text = |cx: &mut VisualTestContext| buffer.read_with(cx, |b, _| b.document.text().to_string());
    let replace = |from: &str, to: &str, cx: &mut VisualTestContext| {
        let replaced = text(cx).replacen(from, to, 1);
        buffer.update(cx, |b, cx| b.replace_all(replaced, cx));
    };
    let has = |label: &str, cx: &mut VisualTestContext| shown(&main, cx).iter().any(|l| l == label);

    place_after(&main, &window, "<Button x:Name=\"Go\" Content=\"Go\" Click=\"OnGo\"", cx);
    cx.simulate_input(" Foreground=");
    cx.simulate_input("\"");
    offered(&main, cx, "the named colours a brush takes", "Red");
    // RSharp's editor pairs no quotes (Passenger's does): the value is closed by hand, the caret back inside it.
    cx.simulate_keystrokes("escape");
    cx.simulate_input("\"");
    cx.simulate_keystrokes("left");
    cx.simulate_input("{");
    offered(&main, cx, "the markup extensions after a value's brace", "StaticResource");
    for label in ["DynamicResource", "Binding", "RelativeSource", "x:Static", "x:Type"] {
        assert!(has(label, cx), "{label} not offered: {:?}", shown(&main, cx));
    }
    cx.simulate_input("StaticRes");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.simulate_input(" ");
    offered(&main, cx, "the brushes' keys, merged in by App.xaml", "Accent");
    assert!(has("Heading", cx) && has("Gap", cx), "{:?}", shown(&main, cx));
    cx.simulate_keystrokes("escape");

    place_after(&main, &window, "Text=\"{Binding ", cx);
    cx.dispatch_action(Complete);
    offered(&main, cx, "the view model's properties, d:DataContext's type's", "Greeting");
    assert!(has("Owner", cx) && has("Mode=", cx), "{:?}", shown(&main, cx));
    cx.simulate_keystrokes("escape");
    replace("{Binding Greeting}", "{Binding Owner.}", cx);
    place_after(&main, &window, "{Binding Owner.", cx);
    cx.dispatch_action(Complete);
    offered(&main, cx, "the path's next step's properties, Person's after Owner.", "Name");
    assert!(has("Age", cx) && !has("Greeting", cx), "{:?}", shown(&main, cx));
    cx.simulate_keystrokes("escape");

    // An item template's bindings resolve against the items its list's ItemsSource binds.
    replace(
        "<TextBlock Text=\"{Binding Name}\" />",
        "<TextBlock Text=\"{Binding Name}\" /><TextBlock Text=\"{Binding }\" />",
        cx,
    );
    place_after(&main, &window, "<TextBlock Text=\"{Binding Name}\" /><TextBlock Text=\"{Binding ", cx);
    cx.dispatch_action(Complete);
    offered(&main, cx, "Person's properties, People's items'", "Age");
    cx.simulate_keystrokes("escape");

    replace("Background=\"{x:Static SystemColors.ControlBrush}\"", "Background=\"{x:Static SystemColors.}\"", cx);
    place_after(&main, &window, "Background=\"{x:Static SystemColors.", cx);
    cx.dispatch_action(Complete);
    offered(&main, cx, "SystemColors' static members", "ControlBrushKey");
    cx.simulate_keystrokes("escape");

    // A namespace as WPF writes it: the project's own by its name, another assembly's with that assembly's.
    for (typed, wanted) in [
        ("http", "http://schemas.microsoft.com/winfx/2006/xaml/presentation"),
        ("clr-namespace:Gal", "clr-namespace:Gallery.ViewModels"),
    ] {
        replace(
            "xmlns:vm=\"clr-namespace:Gallery.ViewModels\"",
            &format!("xmlns:vm=\"clr-namespace:Gallery.ViewModels\" xmlns:items=\"{typed}\""),
            cx,
        );
        place_after(&main, &window, &format!("xmlns:items=\"{typed}"), cx);
        cx.dispatch_action(Complete);
        offered(&main, cx, &format!("{wanted} offered for an xmlns"), wanted);
        cx.simulate_keystrokes("escape");
        replace(&format!(" xmlns:items=\"{typed}\""), "", cx);
    }
    replace(
        "xmlns:vm=\"clr-namespace:Gallery.ViewModels\"",
        "xmlns:vm=\"clr-namespace:Gallery.ViewModels\" xmlns:items=\"clr-namespace:System.Collections.Obj\"",
        cx,
    );
    place_after(&main, &window, "xmlns:items=\"clr-namespace:System.Collections.Obj", cx);
    cx.dispatch_action(Complete);
    wait(cx, "another assembly's namespace, with its assembly", |cx| {
        shown(&main, cx).iter().any(|l| l.starts_with("clr-namespace:System.Collections.ObjectModel;assembly="))
    });
    cx.simulate_keystrokes("escape");

    // A grid column's binding, and a hierarchical template's own, resolve against the items.
    let lists = run.folder.join("ListsWindow.xaml");
    main.update_in(cx, |m, window_, cx| m.open_file(&lists, None, window_, cx));
    place_after(&main, &lists, "<DataGridTextColumn Header=\"Name\" Binding=\"{Binding ", cx);
    cx.dispatch_action(Complete);
    offered(&main, cx, "Person's properties, the grid's items'", "Age");
    assert!(!has("Roots", cx), "the window's data context offered in a grid column: {:?}", shown(&main, cx));
    cx.simulate_keystrokes("escape");
    place_after(&main, &lists, "<HierarchicalDataTemplate ItemsSource=\"{Binding ", cx);
    cx.dispatch_action(Complete);
    offered(&main, cx, "Node's properties, the tree's items'", "Children");
}

#[gpui::test]
fn a_wpf_window_that_compiles_is_marked_with_nothing_and_a_misspelt_property_value_resource_and_binding_path_are_marked(
    cx: &mut TestAppContext,
) {
    let Some(run) = wpf_gallery("wpf-marked") else { return };
    let (main, cx, buffer) = open_wpf(&run, "MainWindow.xaml", cx);
    // The gallery compiles: none of its files is marked once read.
    for file in ["App.xaml", "Themes/Brushes.xaml", "ListsWindow.xaml", "MainWindow.xaml"] {
        let path = run.folder.join(file);
        main.update_in(cx, |m, window, cx| m.open_file(&path, None, window, cx));
        pump(cx, Duration::from_secs(3));
        let marked = main.read_with(cx, |m, cx| {
            m.editor()
                .unwrap()
                .read(cx)
                .buffer
                .read(cx)
                .diagnostics
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
        });
        assert!(marked.is_empty(), "{file}, which compiles, is marked: {marked:?}");
    }
    let text = buffer
        .read_with(cx, |b, _| b.document.text().to_string())
        .replace(
            "<Button x:Name=\"Go\" Content=\"Go\" Click=\"OnGo\"",
            "<Button x:Name=\"Go\" Content=\"Go\" Click=\"OnGo\" Foregrund=\"Red\" HorizontalAlignment=\"Centre\"",
        )
        .replace("{Binding Greeting}", "{Binding Greting}")
        .replace("{StaticResource Accent}", "{StaticResource Acent}");
    buffer.update(cx, |b, cx| b.replace_all(text, cx));
    wait(cx, "the four mistakes marked", |cx| buffer.read_with(cx, |b, _| b.diagnostics.len() == 4));
    let mut marked: Vec<(String, bool)> =
        buffer.read_with(cx, |b, _| b.diagnostics.iter().map(|d| (d.message.clone(), d.error)).collect());
    marked.sort();
    assert_eq!(
        marked,
        [
            ("'Centre' is not a value of HorizontalAlignment".to_string(), true),
            ("Cannot resolve property 'Foregrund' on Button".to_string(), true),
            // WPF's bindings are read as the app runs: a path its design-time data context lacks is a warning.
            ("Cannot resolve property 'Greting' in data context of type MainViewModel".to_string(), false),
            ("Resource 'Acent' is not found".to_string(), true),
        ]
    );
}

#[gpui::test]
fn after_a_build_markup_compiles_error_is_marked_on_its_line_with_its_code(cx: &mut TestAppContext) {
    let Some(run) = wpf_gallery("wpf-built") else { return };
    let (_main, cx, buffer) = open_wpf(&run, "MainWindow.xaml", cx);
    // A button given two children: MarkupCompile's to find, not the live check's.
    let text = buffer.read_with(cx, |b, _| b.document.text().to_string()).replace(
        "<StackPanel Margin=\"12\">",
        "<StackPanel Margin=\"12\">\n<Button><TextBlock Text=\"a\" /><TextBlock Text=\"b\" /></Button>",
    );
    buffer.update(cx, |b, cx| b.replace_all(text.clone(), cx));
    cx.simulate_keystrokes("secondary-shift-b");
    wait(cx, "MarkupCompile's MC3089 marked", |cx| {
        buffer.read_with(cx, |b, _| b.diagnostics.iter().any(|d| d.message.starts_with("MC3089")))
    });
    let (message, error, line) = buffer.read_with(cx, |b, _| {
        let d = b.diagnostics.iter().find(|d| d.message.starts_with("MC3089")).unwrap();
        (d.message.clone(), d.error, b.document.text().byte_to_line(d.range.start))
    });
    assert!(error, "MarkupCompile's error is not marked as an error");
    assert_eq!(
        message,
        "MC3089: The object 'Button' already has a child and cannot add 'TextBlock'. 'Button' can accept only one child."
    );
    assert_eq!(
        line,
        text[..text.find("<Button><TextBlock").unwrap()].matches('\n').count(),
        "not on the button's line"
    );
}

#[gpui::test]
fn go_to_declaration_from_wpf_xaml_opens_a_controls_class_a_resources_definition_a_bound_property_and_a_handler(
    cx: &mut TestAppContext,
) {
    let Some(run) = wpf_gallery("wpf-declaration") else { return };
    let (main, cx, _) = open_wpf(&run, "MainWindow.xaml", cx);
    let window = run.folder.join("MainWindow.xaml");
    let go_to = if cfg!(target_os = "macos") { "cmd-b" } else { "f12" };
    let opened = |name: &str, cx: &mut VisualTestContext| {
        main.read_with(cx, |m, cx| m.editor().is_some_and(|e| e.read(cx).path.ends_with(name)))
    };
    for (needle, file, landed) in [
        ("x:Class=\"Gallery.Main", "MainWindow.xaml.cs", "public partial class |MainWindow"),
        ("{StaticResource Acc", "Themes/Brushes.xaml", "<SolidColorBrush x:Key=\"|Accent\""),
        ("{Binding Gree", "ViewModels/MainViewModel.cs", "public string |Greeting"),
        ("Click=\"OnG", "MainWindow.xaml.cs", "private void |OnGo("),
    ] {
        main.update_in(cx, |m, window_, cx| m.open_file(&window, None, window_, cx));
        place_after(&main, &window, needle, cx);
        cx.simulate_keystrokes(go_to);
        wait(cx, &format!("{file} opened from {needle}"), |cx| opened(file, cx));
        assert!(caret_line(&main, cx).contains(landed), "from {needle}: {}", caret_line(&main, cx));
    }

    // The card on a tag names WPF's type and its base.
    main.update_in(cx, |m, window_, cx| m.open_file(&window, None, window_, cx));
    place_after(&main, &window, "<StackPanel Margin=\"12\">\n        <TextBl", cx);
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "f1" } else { "ctrl-shift-f1" });
    wait(cx, "the card describing the tag's type", |cx| editor.read_with(cx, |e, _| e.popup_text().is_some()));
    let card = editor.read_with(cx, |e, _| e.popup_text()).unwrap();
    assert!(card.contains("class System.Windows.Controls.TextBlock : FrameworkElement"), "{card}");
}

#[gpui::test]
fn a_wpf_elements_x_name_renames_with_its_code_behinds_uses_and_the_code_goes_to_it(cx: &mut TestAppContext) {
    let Some(run) = wpf_gallery("wpf-name") else { return };
    let (main, cx, _) = open_wpf(&run, "MainWindow.xaml.cs", cx);
    let (xaml, behind) = (run.folder.join("MainWindow.xaml"), run.folder.join("MainWindow.xaml.cs"));
    let active = |cx: &mut VisualTestContext| main.read_with(cx, |m, cx| m.editor().map(|e| e.read(cx).path.clone()));

    // The field MarkupCompile makes for the button leads to its name in the XAML, once Roslyn has the project.
    let deadline = Instant::now() + Duration::from_secs(120);
    while active(cx).as_ref() != Some(&xaml) {
        assert!(
            Instant::now() < deadline,
            "Go to Declaration on the field never reached the XAML's name: {:?}",
            active(cx)
        );
        place_after(&main, &behind, "        G", cx);
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-b" } else { "f12" });
        pump(cx, Duration::from_secs(2));
    }
    assert!(caret_line(&main, cx).contains("<Button x:Name=\"|Go\""), "not on the name: {}", caret_line(&main, cx));

    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "alt-f7" } else { "shift-f12" });
    wait(cx, "Find Usages listing", |cx| main.read_with(cx, |m, _| m.found().is_some()));
    let found = main.read_with(cx, |m, _| m.found()).unwrap();
    let line_of = |path: &std::path::PathBuf, needle: &str| {
        let text = std::fs::read_to_string(path).unwrap();
        format!(
            "{}:{}",
            path.file_name().unwrap().to_string_lossy(),
            text[..text.find(needle).unwrap()].matches('\n').count() + 1
        )
    };
    for place in [line_of(&behind, "Go.Content"), line_of(&xaml, "ElementName=Go")] {
        assert!(found.contains(&place), "Find Usages did not list {place}: {found:?}");
    }

    place_after(&main, &xaml, "<Button x:Name=\"G", cx);
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
    assert!(renamed.contains("<Button x:Name=\"Start\"") && renamed.contains("ElementName=Start"), "{renamed}");
    assert!(!renamed.contains("x:Name=\"Go\""), "{renamed}");
}

#[gpui::test]
fn a_wpf_user_control_with_no_code_behind_is_a_class_unmarked_and_go_to_declaration_reaches_its_xaml(
    cx: &mut TestAppContext,
) {
    let badge = "<UserControl x:Class=\"Gallery.Badge\"\n             xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\"\n             xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\">\n    <Border Background=\"{StaticResource Accent}\" Padding=\"4\">\n        <TextBlock Text=\"New\" />\n    </Border>\n</UserControl>\n";
    let badges = "<Window x:Class=\"Gallery.Badges\"\n        xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n        xmlns:local=\"clr-namespace:Gallery\"\n        Title=\"Badges\" Height=\"200\" Width=\"300\">\n    <StackPanel>\n        <local:Badge Margin=\"4\" HorizontalAlignment=\"Left\" />\n    </StackPanel>\n</Window>\n";
    let Some(run) = wpf_gallery("wpf-badge") else { return };
    std::fs::write(run.folder.join("Badge.xaml"), badge).unwrap();
    std::fs::write(run.folder.join("Badges.xaml"), badges).unwrap();
    let (main, cx, buffer) = open_wpf(&run, "Badge.xaml", cx);
    pump(cx, Duration::from_secs(3));
    let marked = buffer.read_with(cx, |b, _| b.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
    assert!(marked.is_empty(), "the XAML-only control is marked: {marked:?}");

    let uses = run.folder.join("Badges.xaml");
    main.update_in(cx, |m, window, cx| m.open_file(&uses, None, window, cx));
    pump(cx, Duration::from_secs(3));
    let marked = main.read_with(cx, |m, cx| {
        m.editor().unwrap().read(cx).buffer.read(cx).diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>()
    });
    assert!(marked.is_empty(), "its use, with a UserControl's Margin, is marked: {marked:?}");
    place_after(&main, &uses, "<local:Bad", cx);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-b" } else { "f12" });
    wait(cx, "Badge.xaml opened", |cx| {
        main.read_with(cx, |m, cx| m.editor().is_some_and(|e| e.read(cx).path.ends_with("Badge.xaml")))
    });
    assert!(caret_line(&main, cx).contains("x:Class=\"|Gallery.Badge\""), "{}", caret_line(&main, cx));
}

const WPF_SHOP_VIEW: &str = "<UserControl x:Class=\"Gallery.ShopView\"\n             xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\"\n             xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n             xmlns:d=\"http://schemas.microsoft.com/expression/blend/2008\"\n             xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"\n             xmlns:vm=\"clr-namespace:Gallery.ViewModels\"\n             mc:Ignorable=\"d\"\n             d:DataContext=\"{d:DesignInstance Type=vm:ShopViewModel}\">\n    <StackPanel>\n        <TextBlock Text=\"{Binding Name}\" />\n    </StackPanel>\n</UserControl>\n";
const WPF_BUTTON: &str =
    "<Button x:Name=\"Go\" Content=\"Go\" Click=\"OnGo\" Background=\"{StaticResource Accent}\" />";
const WPF_TITLE: &str = "<TextBlock Style=\"{StaticResource Heading}\" Text=\"{Binding Greeting}\" />";

#[gpui::test]
fn wpf_reformat_code_lays_xaml_out_by_riders_code_style_and_the_editorconfig(cx: &mut TestAppContext) {
    let Some(run) = wpf_gallery("wpf-format") else { return };
    let (main, cx, buffer) = open_wpf(&run, "MainWindow.xaml", cx);
    let window = run.folder.join("MainWindow.xaml");
    let reformat = if cfg!(target_os = "macos") { "cmd-alt-l" } else { "ctrl-alt-enter" };
    let said = |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.said().map(str::to_string));
    let text = |cx: &mut VisualTestContext| buffer.read_with(cx, |b, _| b.document.text().to_string());
    wait(cx, "the window, laid out as Rider lays it out, left as it is", |cx| {
        cx.simulate_keystrokes(reformat);
        said(cx).as_deref() == Some("Already formatted")
    });
    let tidy = text(cx);
    let (start, end) = (tidy.find("<StackPanel").unwrap(), tidy.find("</StackPanel>").unwrap() + "</StackPanel>".len());
    let untidy = format!("{}{UNTIDY}{}", &tidy[..start], &tidy[end..]);
    buffer.update(cx, |b, cx| b.replace_all(untidy.clone(), cx));
    main.update_in(cx, |m, window_, cx| m.open_file(&window, Some((0, 0)), window_, cx));
    cx.simulate_keystrokes(reformat);
    wait(cx, "the reformat", |cx| said(cx).as_deref() == Some("Reformatted"));
    assert_eq!(
        text(cx),
        format!(
            "{}{}{}",
            &tidy[..start],
            "<StackPanel Margin=\"12\">\n        <!-- the\n             greeting -->\n        <TextBlock Text='Fish &amp; chips' />\n        <Button Grid.Row=\"1\" Content=\"Go\"\n                x:Name=\"Pay\"\n                HorizontalAlignment=\"Center\" />\n\n\n        <Border></Border>\n    </StackPanel>",
            &tidy[end..]
        )
    );
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert_eq!(text(cx), untidy, "one Undo did not take the whole reformat back");

    std::fs::write(
        run.folder.join(".editorconfig"),
        "root = true\n\n[*.{axaml,xaml}]\nxml_attribute_style = on_different_lines\nxml_attribute_indent = single_indent\nxaml_enable_attribute_reordering = true\nxaml_remove_ending_tag_of_empty_element = true\n",
    )
    .unwrap();
    cx.simulate_keystrokes(reformat);
    wait(cx, "the reformat by the .editorconfig", |cx| text(cx) != untidy);
    let configured = text(cx);
    assert!(
        configured.contains("        <Button\n            x:Name=\"Pay\"\n            Grid.Row=\"1\"\n            HorizontalAlignment=\"Center\"\n            Content=\"Go\" />\n\n\n        <Border />\n    </StackPanel>"),
        "the .editorconfig's style was not followed:\n{configured}"
    );
    assert!(configured.starts_with("<Window\n    x:Class=\"Gallery.MainWindow\"\n    xmlns=\""), "{configured}");
}

#[gpui::test]
fn wpf_surround_with_wraps_the_element_at_the_caret_or_the_elements_chosen(cx: &mut TestAppContext) {
    let Some(run) = wpf_gallery("wpf-surround") else { return };
    let (main, cx, buffer) = open_wpf(&run, "MainWindow.xaml", cx);
    let window = run.folder.join("MainWindow.xaml");
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    let surround = if cfg!(target_os = "macos") { "cmd-alt-t" } else { "ctrl-k ctrl-s" };
    let listed = |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.surrounds_shown());
    let text = |cx: &mut VisualTestContext| buffer.read_with(cx, |b, _| b.document.text().to_string());
    let before = text(cx);
    place_after(&main, &window, "<Butt", cx);
    cx.simulate_keystrokes(surround);
    wait(cx, "Surround With's templates", |cx| listed(cx).is_some());
    // Avalonia's Panel is no WPF element.
    assert_eq!(
        listed(cx).unwrap(),
        ["tag", "Grid", "StackPanel", "Border", "ScrollViewer", "DockPanel", "WrapPanel", "Canvas", "Viewbox"]
    );
    cx.simulate_input("4");
    cx.run_until_parked();
    assert!(
        text(cx).contains(&format!("        <Border>\n            {WPF_BUTTON}\n        </Border>\n")),
        "{}",
        text(cx)
    );
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert_eq!(text(cx), before, "one Undo did not take the surround back");

    // The title and the button chosen, from inside one to inside the other: Rider's tag around both.
    let (line, column) = after(&before, &WPF_TITLE[..3]);
    main.update_in(cx, |m, window_, cx| m.open_file(&window, Some((line, column)), window_, cx));
    cx.simulate_keystrokes("shift-down shift-right");
    cx.simulate_keystrokes(surround);
    wait(cx, "Surround With's templates for two elements", |cx| listed(cx).is_some());
    assert!(!listed(cx).unwrap().iter().any(|o| o == "Border" || o == "ScrollViewer"), "{:?}", listed(cx));
    cx.simulate_input("1");
    cx.run_until_parked();
    cx.simulate_input("DockPanel");
    cx.run_until_parked();
    assert!(
        text(cx).contains(&format!(
            "        <DockPanel>\n            {WPF_TITLE}\n            {WPF_BUTTON}\n        </DockPanel>\n"
        )),
        "{}",
        text(cx)
    );
}

#[gpui::test]
fn wpf_extract_resource_and_extract_style_make_a_keyed_resource_and_a_keyed_style(cx: &mut TestAppContext) {
    use rsharp_app::editor::{ExtractResource, ExtractStyle};
    let Some(run) = wpf_gallery("wpf-extract") else { return };
    let (main, cx, buffer) = open_wpf(&run, "MainWindow.xaml", cx);
    let (window, app) = (run.folder.join("MainWindow.xaml"), run.folder.join("App.xaml"));
    let text = |cx: &mut VisualTestContext| buffer.read_with(cx, |b, _| b.document.text().to_string());
    let said = |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.said().map(str::to_string));
    let dialog = |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.extracting());
    let button = "<Button x:Name=\"Go\" Content=\"Go\" Click=\"OnGo\"";
    let with_margin = text(cx).replace(button, &format!("{button} Margin=\"12\""));
    buffer.update(cx, |b, cx| b.replace_all(with_margin, cx));

    place_after(&main, &window, "<StackPanel Margin=\"1", cx);
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
        ["MainWindow.xaml <Window.Resources>", "App.xaml <Application.Resources>", "Brushes.xaml <ResourceDictionary>"]
    );
    resource.update(cx, |d, cx| {
        d.set_name("PanelMargin", cx);
        d.accept(cx);
    });
    wait(cx, "the resource extracted", |cx| dialog(cx).is_none());
    assert!(
        text(cx).contains("        </Style>\n        <Thickness x:Key=\"PanelMargin\">12</Thickness>\n    </Window.Resources>\n    <StackPanel Margin=\"{StaticResource PanelMargin}\">"),
        "{}",
        text(cx)
    );
    assert!(text(cx).contains(&format!("{button} Margin=\"{{StaticResource PanelMargin}}\"")), "{}", text(cx));
    assert_eq!(
        said(cx).as_deref(),
        Some("Extracted resource 'PanelMargin' to MainWindow.xaml <Window.Resources>, used in 2 places")
    );
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();

    // A brush to the application's resources.
    let with_brush = text(cx).replace(
        "<TextBlock Style=\"{StaticResource Heading}\"",
        "<TextBlock Style=\"{StaticResource Heading}\" Foreground=\"#E2A33F\"",
    );
    buffer.update(cx, |b, cx| b.replace_all(with_brush, cx));
    place_after(&main, &window, "Foreground=\"#E2", cx);
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
        app_text.contains("            </Style>\n            <SolidColorBrush x:Key=\"ForegroundBrush\" Color=\"#E2A33F\" />\n        </ResourceDictionary>"),
        "{app_text}"
    );
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();

    // The button's margin and background to WPF's keyed style, which it names.
    place_after(&main, &window, "<Butt", cx);
    cx.dispatch_action(ExtractStyle);
    wait(cx, "Extract Style's dialog", |cx| dialog(cx).is_some());
    let style = dialog(cx).unwrap();
    assert_eq!(style.read_with(cx, |d, cx| d.name(cx)), "ButtonStyle");
    let properties: Vec<String> = style.read_with(cx, |d, _| d.properties()).into_iter().map(|(p, _)| p).collect();
    assert_eq!(properties, ["Content = \"Go\"", "Margin = \"12\"", "Background = \"{StaticResource Accent}\""]);
    style.update(cx, |d, cx| {
        d.toggle_property(0, cx);
        d.accept(cx);
    });
    wait(cx, "the style extracted", |cx| dialog(cx).is_none());
    let made = text(cx);
    assert!(
        made.contains("        </Style>\n        <Style x:Key=\"ButtonStyle\" TargetType=\"Button\">\n            <Setter Property=\"Margin\" Value=\"12\" />\n            <Setter Property=\"Background\" Value=\"{StaticResource Accent}\" />\n        </Style>\n    </Window.Resources>"),
        "{made}"
    );
    assert!(
        made.contains("<Button x:Name=\"Go\" Content=\"Go\" Click=\"OnGo\" Style=\"{StaticResource ButtonStyle}\" />"),
        "{made}"
    );
}

#[gpui::test]
fn wpf_a_binding_path_and_a_converter_nothing_has_offer_to_create_them(cx: &mut TestAppContext) {
    let Some(run) = wpf_gallery("wpf-create") else { return };
    let shop_model = "using CommunityToolkit.Mvvm.ComponentModel;\n\nnamespace Gallery.ViewModels;\n\npublic class ShopViewModel : ObservableObject\n{\n    [ObservableProperty]\n    private string? _name;\n}\n";
    for (file, text) in [
        ("ShopView.xaml", WPF_SHOP_VIEW),
        (
            "ShopView.xaml.cs",
            "using System.Windows.Controls;\n\nnamespace Gallery;\n\npublic partial class ShopView : UserControl\n{\n}\n",
        ),
        ("ViewModels/ShopViewModel.cs", shop_model),
        ("Mvvm.cs", MVVM),
    ] {
        std::fs::write(run.folder.join(file), text).unwrap();
    }
    let (main, cx, buffer) = open_wpf(&run, "MainWindow.xaml", cx);
    let (window, model) = (run.folder.join("MainWindow.xaml"), run.folder.join("ViewModels/MainViewModel.cs"));
    let quick_fix = if cfg!(target_os = "macos") { "alt-enter" } else { "ctrl-." };
    let fix = |path: &std::path::PathBuf, needle: &str, offered: &str, cx: &mut VisualTestContext| {
        place_after(&main, path, needle, cx);
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
    let marked = |text: &str, cx: &mut VisualTestContext| {
        buffer.read_with(cx, |b, _| b.diagnostics.iter().any(|d| d.message.contains(text)))
    };
    let edit = |from: &str, to: &str, cx: &mut VisualTestContext| {
        let text = buffer.read_with(cx, |b, _| b.document.text().to_string()).replacen(from, to, 1);
        buffer.update(cx, |b, cx| b.replace_all(text, cx));
    };
    let disk = |path: &std::path::PathBuf| std::fs::read_to_string(path).unwrap_or_default();

    edit("{Binding Greeting}", "{Binding Subtitle}", cx);
    wait(cx, "the binding path the view model lacks marked", |cx| marked("'Subtitle'", cx));
    fix(&window, "{Binding Subt", "Create property 'Subtitle'", cx);
    wait(cx, "the property written", |_| {
        disk(&model).contains("    public bool IsBusy { get; set; }\n\n    public string? Subtitle { get; set; }\n}")
    });
    wait(cx, "the binding resolved once the property is there", |cx| !marked("'Subtitle'", cx));

    edit(
        "<Button x:Name=\"Go\" Content=\"Go\" Click=\"OnGo\"",
        "<Button x:Name=\"Go\" Content=\"Go\" Click=\"OnGo\" Command=\"{Binding SaveCommand}\"",
        cx,
    );
    wait(cx, "the command the view model lacks marked", |cx| marked("'SaveCommand'", cx));
    fix(&window, "{Binding Save", "Create property 'SaveCommand'", cx);
    wait(cx, "the command written", |_| {
        disk(&model)
            .contains("    public string? Subtitle { get; set; }\n\n    public ICommand? SaveCommand { get; set; }\n}")
    });
    assert!(
        disk(&model).contains("using System.Collections.ObjectModel;\nusing System.Windows.Input;\n"),
        "{}",
        disk(&model)
    );

    edit(
        "<TextBlock Text=\"{Binding Content, ElementName=Go}\" />",
        "<TextBlock Text=\"{Binding Content, ElementName=Go}\" />\n        <TextBlock Text=\"{Binding Greeting, Converter={StaticResource ShoutConverter}}\" />",
        cx,
    );
    wait(cx, "the converter nothing defines marked", |cx| marked("Resource 'ShoutConverter' is not found", cx));
    fix(&window, "{StaticResource Shout", "Create converter 'ShoutConverter'", cx);
    let converter = run.folder.join("ShoutConverter.cs");
    wait(cx, "ShoutConverter.cs made", |_| converter.exists());
    let made = disk(&converter);
    assert!(made.contains("namespace Gallery;\n\npublic class ShoutConverter : IValueConverter\n{\n"), "{made}");
    assert!(made.contains("using System.Windows.Data;"), "{made}");
    assert!(
        made.contains("    public object Convert(object value, Type targetType, object parameter, CultureInfo culture)\n    {\n        throw new NotImplementedException();\n    }\n"),
        "{made}"
    );
    let view = buffer.read_with(cx, |b, _| b.document.text().to_string());
    assert!(
        view.contains("xmlns:vm=\"clr-namespace:Gallery.ViewModels\"\n        xmlns:local=\"clr-namespace:Gallery\""),
        "{view}"
    );
    assert!(view.contains("<local:ShoutConverter x:Key=\"ShoutConverter\" />"), "{view}");

    // With the MVVM Toolkit, an [ObservableProperty] field and a [RelayCommand] method, the class made partial.
    let (shop, shop_model_path) = (run.folder.join("ShopView.xaml"), run.folder.join("ViewModels/ShopViewModel.cs"));
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
        "using CommunityToolkit.Mvvm.ComponentModel;\nusing CommunityToolkit.Mvvm.Input;\n\nnamespace Gallery.ViewModels;\n\npublic partial class ShopViewModel : ObservableObject\n{\n    [ObservableProperty]\n    private string? _name;\n\n    [ObservableProperty]\n    private string? _title;\n\n    [RelayCommand]\n    private void Buy()\n    {\n    }\n}\n"
    );
}

#[gpui::test]
fn colours_show_as_gutter_swatches_and_one_chosen_in_the_picker_is_written_back_as_it_was_written(
    cx: &mut TestAppContext,
) {
    let window_text = "<Window xmlns=\"https://github.com/avaloniaui\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n        x:Class=\"Demo.MainWindow\"\n        Title=\"Gallery\">\n    <StackPanel Margin=\"12\" Spacing=\"8\">\n        <TextBlock Name=\"Title\" Text=\"Hello\" />\n        <Button Name=\"Go\" Content=\"Go\" Background=\"{StaticResource Accent}\" />\n    </StackPanel>\n</Window>\n";
    let code = "namespace Demo;\n\npublic partial class MainWindow : Avalonia.Controls.Window\n{\n    public MainWindow()\n    {\n        InitializeComponent();\n        Go.Content = \"Start\";\n    }\n}\n";
    let Some(run) = restored(
        "xaml-colours",
        &[
            ("Demo.csproj", PROJECT),
            ("App.axaml", APP),
            ("App.axaml.cs", "namespace Demo;\n\npublic partial class App : Avalonia.Application { }\n"),
            ("Styles/Palette.axaml", PALETTE),
            ("MainWindow.axaml", window_text),
            ("MainWindow.axaml.cs", code),
        ],
    ) else {
        return;
    };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let (window, palette, behind) = (
        run.folder.join("MainWindow.axaml"),
        run.folder.join("Styles/Palette.axaml"),
        run.folder.join("MainWindow.axaml.cs"),
    );
    main.update_in(cx, |m, window_, cx| {
        m.open_workspace(&run.folder, window_, cx);
        m.open_file(&window, None, window_, cx);
    });
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    let colour_on = |editor: &Entity<rsharp_app::editor::CodeEditor>, line: usize, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, cx| e.colour_marks(cx).into_iter().find(|(l, _)| *l == line).map(|(_, c)| c))
    };

    // The button's background names the palette's Accent: its swatch is the Dark theme's, which RSharp shows.
    let button = 6;
    wait(cx, "the swatch of the brush the button's background names", |cx| colour_on(&editor, button, cx).is_some());
    assert_eq!(colour_on(&editor, button, cx).as_deref(), Some("#FF3D7BF5"));
    let swatch =
        editor.read_with(cx, |e, cx| e.swatch_point(button, cx)).expect("no swatch was drawn on the button's line");
    cx.simulate_click(swatch, gpui::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, cx| e.picking_colour(cx)).as_deref(), Some("#3D7BF5"));
    cx.simulate_input("#E2A33F");
    assert_eq!(editor.read_with(cx, |e, cx| e.picking_colour(cx)).as_deref(), Some("#E2A33F"));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, cx| e.picking_colour(cx)), None, "the picker stayed open");
    // Written where the brush is defined, in the palette's Dark dictionary; the Light one keeps its own.
    wait(cx, "the colour written into the palette", |_| {
        std::fs::read_to_string(&palette).unwrap_or_default().contains("Color=\"#E2A33F\"")
    });
    let written = std::fs::read_to_string(&palette).unwrap();
    assert!(written.contains("<SolidColorBrush x:Key=\"Accent\" Color=\"#E2A33F\" />"), "{written}");
    assert!(written.contains("<SolidColorBrush x:Key=\"Accent\" Color=\"#3574F0\" />"), "{written}");
    wait(cx, "the swatch following the palette", |cx| colour_on(&editor, button, cx).as_deref() == Some("#FFE2A33F"));

    // A closed resource file can change while its picker is open too.
    let swatch = editor.read_with(cx, |e, cx| e.swatch_point(button, cx)).unwrap();
    cx.simulate_click(swatch, gpui::Modifiers::default());
    cx.run_until_parked();
    cx.simulate_input("#AA0000");
    let changed = written.replace("#E2A33F", "#112233");
    std::fs::write(&palette, &changed).unwrap();
    cx.simulate_keystrokes("enter");
    wait(cx, "the changed resource's colour edit refused", |cx| {
        main.read_with(cx, |m, _| m.said().is_some_and(|s| s.contains("edited meanwhile")))
    });
    assert_eq!(std::fs::read_to_string(&palette).unwrap(), changed, "the picker overwrote the changed resource");

    // C#'s colours too, written back in the form they were written.
    main.update_in(cx, |m, window_, cx| m.open_file(&behind, Some((7, 29)), window_, cx));
    let code_editor = main.read_with(cx, |m, _| m.editor().unwrap());
    cx.simulate_keystrokes("enter");
    cx.simulate_input(
        "Go.Background = new Avalonia.Media.SolidColorBrush(Avalonia.Media.Color.FromRgb(0x33, 0x66, 0x99));",
    );
    let line = code_editor.read_with(cx, |e, _| e.caret().line);
    wait(cx, "the swatch of the colour FromRgb makes", |cx| colour_on(&code_editor, line, cx).is_some());
    assert_eq!(colour_on(&code_editor, line, cx).as_deref(), Some("#FF336699"));
    let swatch = code_editor.read_with(cx, |e, cx| e.swatch_point(line, cx)).expect("no swatch on the call's line");
    cx.simulate_click(swatch, gpui::Modifiers::default());
    cx.run_until_parked();
    cx.simulate_input("#AA0000");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    wait(cx, "the call written back as a call", |cx| {
        code_editor.read_with(cx, |e, _| e.text().to_string()).contains("Avalonia.Media.Color.FromRgb(170, 0, 0));")
    });
}

fn colour_picker<'a>(
    name: &str,
    cx: &'a mut TestAppContext,
) -> Option<(Run, Entity<MainWindow>, Entity<rsharp_app::editor::CodeEditor>, &'a mut VisualTestContext)> {
    let Some(host) = rsharp_lsp::xaml::XamlHost::carried() else {
        eprintln!("skipped: the XAML host was not built");
        return None;
    };
    let run = Run::new(name, &[("Colours.cs", "var a = Color.FromRgb(1, 2, 3);\nvar b = Color.FromRgb(4, 5, 6);\n")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |m, window, cx| {
        m.open_workspace(&run.folder, window, cx);
        m.open_file(&run.folder.join("Colours.cs"), None, window, cx);
    });
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    let buffer = cx.update(|_, cx| rsharp_app::buffer::find(&run.folder.join("Colours.cs"), cx)).unwrap();
    buffer.update(cx, |b, cx| b.attach_colours(host, run.folder.join("Demo.csproj"), cx));
    wait(cx, "the two C# colour swatches", |cx| editor.read_with(cx, |e, cx| e.colour_marks(cx).len() == 2));
    let swatch = editor.read_with(cx, |e, cx| e.swatch_point(0, cx)).expect("the first colour has no swatch");
    cx.simulate_click(swatch, gpui::Modifiers::default());
    cx.run_until_parked();
    cx.simulate_input("#AA0000");
    Some((run, main, editor, cx))
}

#[gpui::test]
fn the_colour_picker_does_not_switch_to_another_colour_when_the_swatches_refresh(cx: &mut TestAppContext) {
    let Some((run, main, editor, cx)) = colour_picker("colour-picker-target", cx) else { return };
    let changed = "// removed\nvar b = Color.FromRgb(4, 5, 6);\n";
    let buffer = cx.update(|_, cx| rsharp_app::buffer::find(&run.folder.join("Colours.cs"), cx)).unwrap();
    buffer.update(cx, |b, cx| b.replace_all(changed.into(), cx));
    wait(cx, "only the second colour's swatch", |cx| editor.read_with(cx, |e, cx| e.colour_marks(cx).len() == 1));
    cx.simulate_keystrokes("enter");
    wait(cx, "the colour edit applied or refused", |cx| {
        main.read_with(cx, |m, _| m.said().is_some_and(|s| s.contains("edited meanwhile")))
            || editor.read_with(cx, |e, _| e.text().to_string() != changed)
    });
    assert_eq!(editor.read_with(cx, |e, _| e.text().to_string()), changed, "the picker edited a different colour");
}

#[gpui::test]
fn the_colour_picker_does_not_write_at_a_stale_range_before_the_swatches_refresh(cx: &mut TestAppContext) {
    let Some((run, main, editor, cx)) = colour_picker("colour-picker-range", cx) else { return };
    let changed = format!("    {}", editor.read_with(cx, |e, _| e.text().to_string()));
    let buffer = cx.update(|_, cx| rsharp_app::buffer::find(&run.folder.join("Colours.cs"), cx)).unwrap();
    buffer.update(cx, |b, cx| b.replace_all(changed.clone(), cx));
    cx.simulate_keystrokes("enter");
    wait(cx, "the stale colour edit applied or refused", |cx| {
        main.read_with(cx, |m, _| m.said().is_some_and(|s| s.contains("edited meanwhile")))
            || editor.read_with(cx, |e, _| e.text().to_string() != changed)
    });
    assert_eq!(editor.read_with(cx, |e, _| e.text().to_string()), changed, "the stale colour range overwrote code");
}

#[gpui::test]
fn the_colour_picker_checks_disk_changes_before_the_watcher_reports_them(cx: &mut TestAppContext) {
    let Some((run, main, editor, cx)) = colour_picker("colour-picker-disk", cx) else { return };
    let changed = format!("    {}", editor.read_with(cx, |e, _| e.text().to_string()));
    std::fs::write(run.folder.join("Colours.cs"), &changed).unwrap();
    cx.simulate_keystrokes("enter");
    wait(cx, "the external colour edit seen or overwritten", |cx| {
        main.read_with(cx, |m, _| m.said().is_some_and(|s| s.contains("edited meanwhile")))
            || editor.read_with(cx, |e, cx| e.is_modified(cx))
    });
    assert_eq!(editor.read_with(cx, |e, _| e.text().to_string()), changed, "the picker overwrote an external edit");
    assert!(!editor.read_with(cx, |e, cx| e.is_modified(cx)), "the refused picker edit left the file modified");
}

#[gpui::test]
fn the_preview_draws_a_window_from_the_last_build_follows_its_text_and_draws_styles_as_their_preview_with(
    cx: &mut TestAppContext,
) {
    let project = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <OutputType>WinExe</OutputType>\n    <TargetFramework>net10.0</TargetFramework>\n    <Nullable>enable</Nullable>\n  </PropertyGroup>\n  <ItemGroup>\n    <PackageReference Include=\"Avalonia\" Version=\"12.1.4\" />\n    <PackageReference Include=\"Avalonia.Desktop\" Version=\"12.1.4\" />\n    <PackageReference Include=\"Avalonia.Themes.Fluent\" Version=\"12.1.4\" />\n  </ItemGroup>\n</Project>\n";
    let program = "using Avalonia;\n\nnamespace Demo;\n\npublic static class Program\n{\n    [System.STAThread]\n    public static void Main(string[] args) => BuildAvaloniaApp().StartWithClassicDesktopLifetime(args);\n\n    public static AppBuilder BuildAvaloniaApp() => AppBuilder.Configure<App>().UsePlatformDetect();\n}\n";
    let app = "<Application xmlns=\"https://github.com/avaloniaui\"\n             xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n             x:Class=\"Demo.App\">\n    <Application.Styles>\n        <FluentTheme />\n    </Application.Styles>\n</Application>\n";
    let app_code = "using Avalonia;\nusing Avalonia.Markup.Xaml;\n\nnamespace Demo;\n\npublic partial class App : Application\n{\n    public override void Initialize() => AvaloniaXamlLoader.Load(this);\n}\n";
    let window_text = "<Window xmlns=\"https://github.com/avaloniaui\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n        xmlns:d=\"http://schemas.microsoft.com/expression/blend/2008\"\n        xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"\n        mc:Ignorable=\"d\" d:DesignWidth=\"800\" d:DesignHeight=\"450\"\n        x:Class=\"Demo.MainWindow\" Title=\"Demo\">\n    <StackPanel Margin=\"12\">\n        <TextBlock Text=\"Hello\" />\n    </StackPanel>\n</Window>\n";
    let window_code = "using Avalonia.Controls;\n\nnamespace Demo;\n\npublic partial class MainWindow : Window\n{\n    public MainWindow()\n    {\n        InitializeComponent();\n    }\n}\n";
    let buttons = "<Styles xmlns=\"https://github.com/avaloniaui\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\">\n    <Design.PreviewWith>\n        <Border Padding=\"20\">\n            <Button Classes=\"orange\" Content=\"Orange\" Width=\"120\" Height=\"40\" />\n        </Border>\n    </Design.PreviewWith>\n    <Style Selector=\"Button.orange\">\n        <Setter Property=\"Background\" Value=\"#E2A33F\" />\n    </Style>\n    <Style Selector=\"Button.orange /template/ ContentPresenter\">\n        <Setter Property=\"Background\" Value=\"#E2A33F\" />\n    </Style>\n</Styles>\n";
    let Some(run) = restored(
        "xaml-preview",
        &[
            ("Demo.csproj", project),
            ("Program.cs", program),
            ("App.axaml", app),
            ("App.axaml.cs", app_code),
            ("MainWindow.axaml", window_text),
            ("MainWindow.axaml.cs", window_code),
        ],
    ) else {
        return;
    };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let window = run.folder.join("MainWindow.axaml");
    main.update_in(cx, |m, window_, cx| {
        m.open_workspace(&run.folder, window_, cx);
        m.open_file(&window, None, window_, cx);
    });
    // The previewer draws the last build.
    cx.simulate_keystrokes("secondary-shift-b");
    wait(cx, "the build", |cx| main.read_with(cx, |m, _| !m.is_building() && m.build_outcome().is_some()));
    cx.dispatch_action(rsharp_app::window::XamlPreview);
    let frame = |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.preview_frame());
    let status = |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.preview_status()).unwrap_or_default();
    wait(cx, "the previewer's first frame", |cx| {
        assert!(!status(cx).contains("could not start") && !status(cx).starts_with("Build"), "{}", status(cx));
        frame(cx).is_some()
    });
    assert_eq!(main.read_with(cx, |m, _| m.preview_error()), None);
    // Drawn at the size the window's design-time width and height give it.
    assert_eq!(frame(cx), Some((800, 450)));
    // A point of the panel below its children, where its background shows.
    let below = |cx: &mut VisualTestContext| main.read_with(cx, |m, _| m.preview_pixel(400, 300));
    let orange = Some([0x3F, 0xA3, 0xE2, 0xFF]);
    assert_ne!(below(cx), orange);

    // The file's elements come with their boxes: the caret in the TextBlock's tag outlines it, under the panel's margin.
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    let offset_of = |what: &str, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.text().to_string()).find(what).unwrap_or_else(|| panic!("no {what} in the file"))
    };
    let caret = offset_of("<TextBlock", cx) + 3;
    editor.update(cx, |e, cx| e.select(rsharp_text::Selection::caret(caret), cx));
    let outlined = |cx: &mut VisualTestContext| main.read_with(cx, |m, cx| m.preview_outlined(cx));
    wait(cx, "the TextBlock at the caret outlined", |cx| outlined(cx).is_some_and(|[_, _, _, h]| h < 100.));
    assert_eq!(
        outlined(cx).map(|[x, y, w, _]| (x, y, w)),
        Some((12., 12., 776.)),
        "the TextBlock is not in the margin"
    );
    let tags: Vec<String> = main.read_with(cx, |m, cx| m.preview_elements(cx)).into_iter().map(|(t, _)| t).collect();
    assert_eq!(
        tags.iter().map(|t| t.split_whitespace().next().unwrap_or_default()).collect::<Vec<_>>(),
        ["<Window", "<StackPanel", "<TextBlock"],
        "the drawn elements are not the file's, parents first: {tags:?}"
    );

    // A click below the TextBlock is on the panel: its start tag is selected, the caret at its start, and it is outlined.
    let frame_bounds = cx.debug_bounds("xaml preview frame").expect("the preview's frame is on screen");
    let fit = f32::from(frame_bounds.size.width) / 800.;
    cx.simulate_click(
        frame_bounds.origin + gpui::point(gpui::px(400. * fit), gpui::px(300. * fit)),
        Default::default(),
    );
    cx.run_until_parked();
    let (start, end) = (offset_of("<StackPanel", cx), offset_of("<TextBlock", cx));
    let tag_end = start + editor.read_with(cx, |e, _| e.text().to_string())[start..end].find('>').unwrap() + 1;
    assert_eq!(
        editor.read_with(cx, |e, _| e.primary()),
        rsharp_text::Selection { anchor: tag_end, head: start },
        "the click did not select the panel's start tag"
    );
    assert_eq!(outlined(cx), Some([12., 12., 776., 426.]), "the panel is not outlined");

    let buffer = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).buffer.clone());
    let replace = |from: &str, to: &str, cx: &mut VisualTestContext| {
        let text = buffer.read_with(cx, |b, _| b.document.text().to_string()).replacen(from, to, 1);
        buffer.update(cx, |b, cx| b.replace_all(text, cx));
    };
    replace("<StackPanel", "<StackPanel Background=\"#E2A33F\"", cx);
    wait(cx, "the preview drawing the panel's new background once typing paused", |cx| below(cx) == orange);

    // XAML that does not load leaves the last frame up, the previewer's reason over it.
    replace("</StackPanel>", "</StackPane>", cx);
    wait(cx, "the previewer saying why the XAML did not load", |cx| {
        main.read_with(cx, |m, _| m.preview_error()).is_some()
    });
    let error = main.read_with(cx, |m, _| m.preview_error()).unwrap();
    assert!(error.contains("line 9"), "the error names no line: {error}");
    assert_eq!(below(cx), orange, "the last frame went");
    replace("</StackPane>", "</StackPanel>", cx);
    wait(cx, "the error gone once the XAML loads again", |cx| main.read_with(cx, |m, _| m.preview_error()).is_none());

    // Styles preview as their Design.PreviewWith: the file shown is the one previewed.
    let styles = run.folder.join("Buttons.axaml");
    std::fs::write(&styles, buttons).unwrap();
    main.update_in(cx, |m, window_, cx| m.open_file(&styles, None, window_, cx));
    wait(cx, "the button Design.PreviewWith holds drawn in the style's colour", |cx| {
        main.read_with(cx, |m, _| {
            m.preview_frame()
                .is_some_and(|(width, height)| (0..width).step_by(4).any(|x| m.preview_pixel(x, height / 2) == orange))
        })
    });

    // Back to the window's tab, open already: the preview follows it.
    main.update_in(cx, |m, window_, cx| m.open_file(&window, None, window_, cx));
    wait(cx, "the preview back on the window", |cx| {
        main.read_with(cx, |m, _| m.preview_frame() == Some((800, 450)) && m.preview_pixel(400, 300) == orange)
    });

    // Closed, the preview ends, its previewer with it.
    main.update_in(cx, |m, window_, cx| m.toggle_tool(rsharp_app::dock::Tool::XamlPreview, window_, cx));
    assert_eq!(status(cx), "", "the preview went on with its tool window closed");
}
