//! A debugger that dies with an unhandled exception as it starts. Its own test binary: the adapter is made to crash by an
//! environment variable it reads, which every other debugging test running beside it in one process would read too.

mod support;

use gpui::{TestAppContext, VisualTestContext};
use rsharp_app::window::MainWindow;
use std::path::Path;
use std::time::{Duration, Instant};
use support::Run;

const EXE_PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>";

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

fn pump(cx: &mut VisualTestContext, done: impl Fn(&mut VisualTestContext) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        if done(cx) {
            return true;
        }
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
    done(cx)
}

#[test]
fn a_debugger_that_dies_with_an_exception_leaves_the_window_up_with_its_words_in_the_console_and_the_log() {
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-crash", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", "System.Console.WriteLine(\"never\");\n");
    if !offline(&run) {
        return;
    }
    // The engine's log is opened as the adapter starts, so a folder in its place kills it with an unhandled exception:
    // its stack trace comes on standard error, read off the UI thread while the session starts.
    let folder_for_log = run.folder.join("engine-log");
    std::fs::create_dir_all(&folder_for_log).unwrap();
    // SAFETY: this binary's one test; nothing else reads the environment while it is set.
    unsafe { std::env::set_var("RSHARP_ENGINE_LOG", &folder_for_log) };
    let mut cx = TestAppContext::single();
    let (main, cx) = run.window(&mut cx);
    let folder = run.folder.clone();
    main.update_in(cx, |main: &mut MainWindow, window, cx| main.open_workspace(&folder, window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f5");
    let log = run.state.join("logs").join("rsharp.log");
    let logged =
        |_: &mut VisualTestContext| std::fs::read_to_string(&log).is_ok_and(|text| text.contains("at Program.<Main>"));
    let ended = pump(cx, |cx| logged(cx) && !main.read_with(cx, |main, _| main.is_debugging()));
    let said = main.read_with(cx, |main, _| main.said().map(str::to_string)).unwrap_or_default();
    let console = main.read_with(cx, |main, cx| main.debug_console().read(cx).text());
    assert!(ended, "the debugger's end never reached the IDE's log or the session never ended: {said:?}\n{console}");
    assert!(said.starts_with("Could not debug"), "the status bar does not say the debugging failed: {said:?}");
    // Every line of the stack trace, to its last frame: a line read on the wrong thread would end the reading.
    assert!(
        console.contains("Unhandled exception. System.UnauthorizedAccessException"),
        "the console lacks it:\n{console}"
    );
    assert!(console.contains("at Program.<Main>"), "the console lacks the stack trace's last frame:\n{console}");
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(text.contains("UnauthorizedAccessException"), "the IDE's log lacks the debugger's exception:\n{text}");
    unsafe { std::env::remove_var("RSHARP_ENGINE_LOG") };
}
