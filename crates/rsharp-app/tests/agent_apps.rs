//! An agent using the terminal, the profiler and a debugged Avalonia app through the window.

mod support;

use gpui::{TestAppContext, VisualTestContext};
use serde_json::{Value, json};
use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, Instant};
use support::Run;

const EXE_PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>";

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

fn write(root: &Path, name: &str, text: &str) {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Pin the installed SDK and make restore use only this fixture's package directory and no feeds, as the run tests do.
fn offline(run: &Run) -> bool {
    let Some(dotnet) = rsharp_core::programs::dotnet() else {
        eprintln!("skipped: dotnet is not installed");
        return false;
    };
    let command = rsharp_core::dotnet::command(&dotnet, ["--list-sdks"], &run.folder);
    let mut listed = String::new();
    let _ = rsharp_core::build::run(command, &std::sync::atomic::AtomicBool::new(false), |text, error| {
        if !error {
            listed.push_str(text);
        }
    });
    let Some(sdk) = listed.lines().find_map(|line| {
        line.split_whitespace().next().filter(|version| version.starts_with("10.0.1")).map(str::to_string)
    }) else {
        eprintln!("skipped: no installed SDK 10.0.1xx for the offline debugger fixture");
        return false;
    };
    write(&run.folder, "global.json", &format!(r#"{{"sdk":{{"version":"{sdk}","rollForward":"disable"}}}}"#));
    write(
        &run.folder,
        "nuget.config",
        "<configuration><packageSources><clear /></packageSources><config><add key=\"globalPackagesFolder\" value=\".packages\" /></config></configuration>",
    );
    write(
        &run.folder,
        "Directory.Build.props",
        "<Project><PropertyGroup><RestorePackagesPath>$(MSBuildThisFileDirectory).packages</RestorePackagesPath><NuGetAudit>false</NuGetAudit></PropertyGroup></Project>",
    );
    true
}

/// Plays an agent's session of `requests` through `rsharp_mcp::attach` on a thread of its own while the window runs, and
/// answers each reply.
fn agent(cx: &mut VisualTestContext, folder: &std::path::Path, requests: &[Value]) -> Vec<Value> {
    let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
    let folder = folder.to_path_buf();
    let session = std::thread::spawn(move || {
        let mut output = Vec::new();
        rsharp_mcp::attach(&folder, Cursor::new(input), &mut output).unwrap();
        output
    });
    let deadline = Instant::now() + Duration::from_secs(240);
    while !session.is_finished() {
        assert!(Instant::now() < deadline, "the agent's session never ended");
        cx.executor().advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(20));
    }
    String::from_utf8(session.join().unwrap()).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn call(id: i64, tool: &str, arguments: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": tool, "arguments": arguments } })
}

fn text(reply: &Value) -> String {
    reply["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

fn enable(main_cx: &mut VisualTestContext) {
    main_cx.update(|_, cx| {
        let mut on = rsharp_app::settings::settings(cx).clone();
        on.agents_enabled = true;
        rsharp_app::settings::apply(on, cx);
    });
}

#[gpui::test]
fn an_agent_runs_a_command_in_a_terminal_tab_and_reads_its_screen(cx: &mut TestAppContext) {
    let run = Run::new("agent-terminal", &[("note.txt", "x\n")]);
    unsafe { std::env::set_var("RSHARP_AGENTS", std::env::temp_dir().join(format!("rsa-{}", std::process::id()))) };
    let (main, cx) = run.window(cx);
    enable(cx);
    main.update_in(cx, |m, w, cx| {
        m.open_workspace(&run.folder, w, cx);
        m.open_agent_socket(cx);
    });
    let r = agent(
        cx,
        &run.folder,
        &[call(1, "terminal_run", json!({ "command": "echo from the agent; exit 3", "waitMs": 20000 }))],
    );
    let seen: Value = serde_json::from_str(&text(&r[0])).unwrap_or_else(|_| panic!("terminal_run: {}", r[0]));
    assert_eq!(seen["exited"], true, "the command did not end: {seen}");
    assert_eq!(seen["exitCode"], 3, "its exit code: {seen}");
    assert!(
        seen["screen"].as_str().unwrap_or_default().contains("from the agent"),
        "its output is not on the screen: {seen}"
    );
}

#[gpui::test]
fn an_agent_profiles_the_program_and_reads_its_hot_spots(cx: &mut TestAppContext) {
    if !rsharp_lsp::profiler::available() {
        eprintln!("skipped: RSharp's build carries no profiler host");
        return;
    }
    let run = Run::new("agent-profile", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "var end = System.Diagnostics.Stopwatch.StartNew();\nlong n = 0;\nwhile (end.ElapsedMilliseconds < 3000) { n = Spin(n); }\nstatic long Spin(long n) { for (int i = 0; i < 100000; i++) { n += i; } return n; }\n",
    );
    if !offline(&run) {
        return;
    }
    unsafe { std::env::set_var("RSHARP_AGENTS", std::env::temp_dir().join(format!("rsa-{}", std::process::id()))) };
    let (main, cx) = run.window(cx);
    enable(cx);
    main.update_in(cx, |m, w, cx| {
        m.open_workspace(&run.folder, w, cx);
        m.open_agent_socket(cx);
    });
    cx.run_until_parked();
    let r = agent(cx, &run.folder, &[call(1, "profile", json!({ "seconds": 30 }))]);
    assert!(text(&r[0]).contains("Spin"), "the method the program spent its time in is not a hot spot: {}", r[0]);
}

#[gpui::test]
fn an_agent_drives_a_debugged_avalonia_app_through_its_inspector(cx: &mut TestAppContext) {
    let Some(run) = app("agent-app") else { return };
    unsafe { std::env::set_var("RSHARP_AGENTS", std::env::temp_dir().join(format!("rsa-{}", std::process::id()))) };
    let (main, cx) = run.window(cx);
    enable(cx);
    main.update_in(cx, |m, w, cx| {
        m.open_workspace(&run.folder, w, cx);
        m.open_agent_socket(cx);
    });
    cx.run_until_parked();
    let started = agent(cx, &run.folder, &[call(1, "debug_start", json!({ "waitMs": 1000 }))]);
    assert_eq!(started[0]["result"]["isError"], false, "debug_start: {}", started[0]);
    let deadline = Instant::now() + Duration::from_secs(180);
    while !main.read_with(cx, |m, _| m.live_status()).unwrap_or_default().starts_with("Avalonia") {
        assert!(Instant::now() < deadline, "the app's inspector never said hello");
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(20));
    }
    let r = agent(
        cx,
        &run.folder,
        &[
            call(1, "app_windows", json!({})),
            call(2, "app_tree", json!({})),
            call(3, "app_details", json!({ "control": "Button#Save" })),
            call(4, "app_capture", json!({ "control": "Button#Save" })),
            call(5, "app_binding_errors", json!({})),
            call(6, "debug_stop", json!({})),
        ],
    );
    assert!(text(&r[0]).contains("MainWindow"), "app_windows: {}", r[0]);
    assert!(text(&r[1]).contains("Button#Save"), "app_tree has no Save button: {}", r[1]);
    assert!(text(&r[2]).contains("MainWindow.axaml"), "app_details names no XAML place: {}", r[2]);
    assert_eq!(r[3]["result"]["content"][0]["type"], "image", "app_capture is not a picture: {}", r[3]);
    assert!(text(&r[4]).contains("Nmae"), "the failed binding is not listed: {}", r[4]);
}
