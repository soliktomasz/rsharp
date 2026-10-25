//! Debugging from the window: a line breakpoint set with F9, F5 building and starting the program under the debugger,
//! the Debug window's frames and values, a step with F10, and the program resumed to its end.

mod support;

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use rsharp_app::debug_view::Phase;
use rsharp_app::window::MainWindow;
use std::path::Path;
use std::time::{Duration, Instant};
use support::Run;

const EXE_PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>";

/// The program the test debugs: line 7 sets `a`, line 8 adds to it, line 9 writes the answer.
const PROGRAM: &str = "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        int a = 41;\n        int b = a + 1;\n        Console.WriteLine($\"answer {b}\");\n    }\n}\n";

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

fn open<'a>(run: &Run, cx: &'a mut TestAppContext) -> (Entity<MainWindow>, &'a mut VisualTestContext) {
    let (main, cx) = run.window(cx);
    let folder = run.folder.clone();
    main.update_in(cx, |main, window, cx| main.open_workspace(&folder, window, cx));
    cx.run_until_parked();
    (main, cx)
}

/// Clicks what a debug selector names, as a person would.
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let at = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing on screen is {selector}")).center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

/// Drives the window's clock until `done` says so or two minutes pass, saying whether it came true.
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
fn a_breakpoint_stops_the_program_and_the_debug_window_shows_it() {
    if !cfg!(target_os = "macos") {
        // The debugger's engine is Passenger's, and macOS is where this suite runs; Windows and Linux are CI's.
    }
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-stop", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", PROGRAM);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((6, 0)), window, cx));
    cx.run_until_parked();

    cx.simulate_keystrokes("f9");
    cx.run_until_parked();
    assert_eq!(
        main.read_with(cx, |main, _| main.breakpoints().lines(&source)),
        vec![6],
        "F9 at the caret sets a breakpoint on its line"
    );

    cx.simulate_keystrokes("f5");
    let paused = pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused));
    assert!(paused, "the program reached its breakpoint within two minutes");
    let frames = main.read_with(cx, |main, _| main.debug_frames().to_vec());
    assert_eq!(frames[0].path().as_deref(), Some(source.as_path()), "the innermost frame is the breakpoint's file");
    assert_eq!(frames[0].line, 6, "the frame stands at the breakpoint's line, counted from 0");
    assert!(cx.debug_bounds("debug frame 0").is_some(), "the Debug window lists the stopped thread's frames");
    assert!(cx.debug_bounds("debug variable a").is_some(), "the Debug window lists the frame's locals");
    let marked = main
        .read_with(cx, |_, cx| rsharp_app::buffer::find(&source, cx).and_then(|buffer| buffer.read(cx).execution_line));
    assert_eq!(marked, Some(6), "the editor marks the line the program is stopped at");

    click(cx, "debug: Step Over");
    let stepped = pump(cx, |cx| {
        main.read_with(cx, |main, _| {
            main.debug_phase() == Phase::Paused && main.debug_frames().first().is_some_and(|frame| frame.line == 7)
        })
    });
    assert!(stepped, "Step Over moved the program to its next statement");

    click(cx, "debug frame 0");
    click(cx, "debug: Resume");
    let ended = pump(cx, |cx| main.read_with(cx, |main, _| !main.is_debugging()));
    assert!(ended, "the program ran to its end");
    let output = main.read_with(cx, |main, cx| main.debug_console().read(cx).text());
    assert!(output.contains("answer 42"), "the program's output is in the Debug console: {output:?}");
}

#[test]
fn a_breakpoint_is_kept_with_the_folder_and_read_back() {
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-kept", &[("App/App.csproj", EXE_PROJECT), ("App/Program.cs", PROGRAM)]);
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((6, 0)), window, cx));
    cx.simulate_keystrokes("f9");
    cx.run_until_parked();
    cx.update(|_, cx| rsharp_app::state::store(cx).flush());
    let kept = {
        let store = rsharp_core::state::StateStore::open(run.state.clone());
        store.read_for::<rsharp_core::debug::Breakpoints>(&run.folder).resolved(&run.folder)
    };
    assert_eq!(kept.lines(&source), vec![6], "the breakpoint is kept with the folder as breakpoints.json");
}
