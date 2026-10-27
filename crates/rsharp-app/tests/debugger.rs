//! Debugging from the window: a line breakpoint set with F9, F5 building and starting the program under the debugger,
//! the Debug window's frames and values, a step with F10, and the program resumed to its end.

mod support;

use gpui::{Entity, Focusable, Modifiers, TestAppContext, VisualTestContext};
use rsharp_app::debug_view::Phase;
use rsharp_app::window::MainWindow;
use std::path::{Path, PathBuf};
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

/// One debugging test at a time, as Passenger runs its debugger window tests alone: each starts a build, the adapter
/// and a program, and several at once on a CI runner's few cores pass their limits waiting for one another.
fn alone() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
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
    let _alone = alone();
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
    let _alone = alone();
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

/// Opens Rider's Breakpoints dialog with its key.
fn open_breakpoints(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-shift-f8" } else { "ctrl-shift-f8" });
    cx.run_until_parked();
    assert!(cx.debug_bounds("breakpoints dialog").is_some(), "Cmd+Shift+F8 did not open the Breakpoints dialog");
}

/// Drives the window until the session the Debug started has ended, saying whether it paused on the way.
fn run_to_end(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> bool {
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut started = false;
    let mut paused = false;
    loop {
        let phase = main.read_with(cx, |main, _| main.debug_phase());
        started |= phase != Phase::Idle;
        paused |= phase == Phase::Paused;
        if started && phase == Phase::Idle {
            return paused;
        }
        if Instant::now() >= deadline {
            let said = main.read_with(cx, |main, _| main.said().map(str::to_string));
            panic!("the program never finished; it is at {phase:?}: {said:?}");
        }
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_breakpoints_dialog_keeps_a_condition_and_mutes_every_breakpoint() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-dialog", &[]);
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

    open_breakpoints(cx);
    assert!(
        cx.debug_bounds("breakpoint row: Program.cs:7").is_some(),
        "the dialog lists the line breakpoint as file:line, counted from 1"
    );
    click(cx, "breakpoint field: Condition");
    cx.simulate_input("a > 40");
    cx.run_until_parked();
    let shown = main.read_with(cx, |main, cx| {
        main.breakpoints_dialog().map(|dialog| (dialog.read(cx).chosen(), dialog.read(cx).condition_text(cx)))
    });
    click(cx, "button: Done");
    assert_eq!(shown, Some(("Program.cs:7".to_string(), "a > 40".to_string())), "the dialog field took the typing");
    assert_eq!(
        main.read_with(cx, |main, _| main.breakpoints().of(&source)[0].condition.clone()),
        "a > 40",
        "the condition typed in the dialog is kept with the breakpoint"
    );

    open_breakpoints(cx);
    click(cx, "check: Mute Breakpoints");
    assert!(main.read_with(cx, |main, _| main.breakpoints_muted()), "Mute Breakpoints is kept");
    click(cx, "check: Mute Breakpoints");
    assert!(!main.read_with(cx, |main, _| main.breakpoints_muted()), "and turned off again");
    click(cx, "button: Done");
}

#[test]
fn a_condition_the_debugger_is_false_stops_nothing_and_a_logpoint_logs() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-condition", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", PROGRAM);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((7, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    open_breakpoints(cx);
    click(cx, "breakpoint field: Condition");
    cx.simulate_input("a > 100");
    cx.run_until_parked();
    click(cx, "button: Done");
    assert_eq!(
        main.read_with(cx, |main, _| main.breakpoints().of(&source)[0].condition.clone()),
        "a > 100",
        "the condition reached the folder's breakpoint"
    );

    cx.simulate_keystrokes("f5");
    let paused = run_to_end(&main, cx);
    assert!(!paused, "a breakpoint whose condition is false stopped the program");
    let output = main.read_with(cx, |main, cx| main.debug_console().read(cx).text());
    let built = main.read_with(cx, |main, cx| main.build_output(cx));
    assert!(output.contains("answer 42"), "the program ran to its end: {output:?}; its build said: {built:?}");

    // A logpoint on the statement that writes the answer logs it and goes on.
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((8, 0)), window, cx));
    cx.simulate_keystrokes("f9");
    open_breakpoints(cx);
    click(cx, "breakpoint row: Program.cs:9");
    click(cx, "breakpoint field: Log message");
    cx.simulate_input("b is {b}");
    cx.run_until_parked();
    click(cx, "button: Done");
    let logged = main.read_with(cx, |main, _| {
        main.breakpoints().of(&source).into_iter().find(|breakpoint| breakpoint.line == 8).map(|b| b.log_message)
    });
    assert_eq!(logged.as_deref(), Some("b is {b}"), "the log message reached the second breakpoint");
    cx.simulate_keystrokes("f5");
    run_to_end(&main, cx);
    let output = main.read_with(cx, |main, cx| main.debug_console().read(cx).text());
    assert!(output.contains("b is 42"), "the logpoint's message reached the Debug console: {output:?}");
}

#[test]
fn any_exception_off_lets_an_unhandled_exception_run_to_its_end() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-exception", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        Console.WriteLine(\"before\");\n        throw new InvalidOperationException(\"boom\");\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((6, 0)), window, cx));
    cx.run_until_parked();

    // Any exception is every folder's, and on until it is turned off here.
    open_breakpoints(cx);
    click(cx, "breakpoint row: Any exception");
    click(cx, "check: Enabled");
    click(cx, "button: Done");
    assert!(!main.read_with(cx, |main, _| main.breakpoints().any_exception().enabled), "Any exception was turned off");

    cx.simulate_keystrokes("f5");
    let paused = run_to_end(&main, cx);
    assert!(!paused, "an unhandled exception stopped the program with Any exception off");
    let output = main.read_with(cx, |main, cx| main.debug_console().read(cx).text());
    assert!(output.contains("before"), "the program ran: {output:?}");
}

/// Builds a program that loops until it is killed, starts it, and waits for its debugger transport.
fn running_program(run: &Run) -> Option<(rsharp_core::process::Child, PathBuf)> {
    let dotnet = rsharp_core::programs::dotnet()?;
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\nusing System.Threading;\n\nclass Program\n{\n    static void Main()\n    {\n        var total = 0;\n        while (true)\n        {\n            total++;\n            Thread.Sleep(50);\n        }\n    }\n}\n",
    );
    // A name of its own, so Attach to Process's list can tell it from another test's program running beside it.
    write(
        &run.folder,
        "App/App.csproj",
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework><AssemblyName>AttachApp</AssemblyName></PropertyGroup></Project>",
    );
    if !offline(run) {
        return None;
    }
    let built = run.folder.join("bin");
    let output = std::process::Command::new(&dotnet)
        .args(["build", "-c", "Debug", "-o"])
        .arg(&built)
        .arg("App/App.csproj")
        .arg("--nologo")
        .arg("-v")
        .arg("q")
        .current_dir(&run.folder)
        .output()
        .expect("dotnet build runs");
    assert!(
        output.status.success(),
        "the attaching program did not build: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut command = std::process::Command::new(&dotnet);
    command
        .arg(built.join("AttachApp.dll"))
        .current_dir(&built)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let program = rsharp_core::process::spawn(command).expect("the program starts");
    // The runtime opens its debugger transport a moment after start: Attach to Process finds programs by it.
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if transport_ready(program.id()) {
            return Some((program, rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"))));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the program opened no debugger transport within twenty seconds");
}

/// Whether the runtime's debugger transport is open for a process: its temporary-folder pipe on macOS and Linux, its
/// diagnostic port's named pipe on Windows.
fn transport_ready(id: u32) -> bool {
    let (folder, name) = if cfg!(windows) {
        (r"\\.\pipe\".to_string(), format!("dotnet-diagnostic-{id}"))
    } else {
        (std::env::temp_dir().to_string_lossy().into_owned(), format!("clr-debug-pipe-{id}-"))
    };
    std::fs::read_dir(folder).ok().into_iter().flatten().flatten().any(|entry| {
        let found = entry.file_name().to_string_lossy().into_owned();
        if cfg!(windows) { found == name } else { found.starts_with(&name) }
    })
}

#[test]
fn attach_to_a_running_program_stops_at_its_breakpoint_and_detaching_lets_it_run_on() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-attach", &[]);
    let Some((mut program, source)) = running_program(&run) else { return };
    let id = program.id();
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((10, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.run_until_parked();

    cx.simulate_keystrokes("secondary-shift-a");
    cx.simulate_input("attach to process");
    cx.simulate_keystrokes("enter");
    let opened = pump(cx, |cx| cx.debug_bounds("attach dialog").is_some());
    assert!(opened, "Find Action did not open Attach to Process");
    click(cx, "attach field: Search processes");
    cx.simulate_input(&id.to_string());
    let listed = pump(cx, |cx| cx.debug_bounds("attach row: AttachApp").is_some());
    assert!(listed, "Attach to Process did not list the running program");
    click(cx, "attach row: AttachApp");
    click(cx, "button: Attach with .NET Debugger");

    let paused = pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused));
    assert!(paused, "the attached program did not stop at the breakpoint");
    let frames = main.read_with(cx, |main, _| main.debug_frames().to_vec());
    assert_eq!(frames[0].path().as_deref(), Some(source.as_path()), "the frame is in the attached program");
    assert_eq!(main.read_with(cx, |main, _| main.attached_process().map(|process| process.id)), Some(id));

    click(cx, "debug: Stop");
    assert!(!main.read_with(cx, |main, _| main.is_debugging()), "Stop ended the session");
    assert!(main.read_with(cx, |main, _| main.attached_process().is_none()));
    std::thread::sleep(Duration::from_millis(200));
    assert!(program.process.try_wait().ok().flatten().is_none(), "Detach let the program go on rather than ending it");
    program.kill();
}

#[test]
fn evaluate_expression_answers_in_the_paused_frame_and_set_next_statement_moves_the_thread() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-evaluate", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", PROGRAM);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    // The breakpoint is on "int b = a + 1;", where a is 41.
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((7, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    let paused = pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused));
    assert!(paused, "the program did not stop at its breakpoint");

    cx.simulate_keystrokes("alt-f8");
    let opened = pump(cx, |cx| cx.debug_bounds("evaluate dialog").is_some());
    assert!(opened, "Alt+F8 did not open Evaluate Expression");
    click(cx, "evaluate field: Expression");
    cx.simulate_input("a + 1");
    let typed = main.read_with(cx, |main, cx| main.evaluate_dialog().map(|dialog| dialog.read(cx).expression(cx)));
    assert_eq!(typed.as_deref(), Some("a + 1"), "the expression field took the typing");
    cx.simulate_keystrokes("enter");
    let answered = pump(cx, |cx| {
        main.read_with(cx, |main, cx| {
            main.evaluate_dialog()
                .and_then(|dialog| dialog.read(cx).result().map(|result| result.contains("42")))
                .unwrap_or(false)
        })
    });
    let result = main.read_with(cx, |main, cx| {
        main.evaluate_dialog().and_then(|dialog| dialog.read(cx).result().map(str::to_string))
    });
    assert!(answered, "a + 1 was not answered 42 in the paused frame: {result:?}");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(main.read_with(cx, |main, _| main.evaluate_dialog().is_none()), "Escape closed Evaluate Expression");

    // Step to the next statement, then move the execution point back to the assignment's line.
    cx.simulate_keystrokes("f10");
    let stepped = pump(cx, |cx| {
        main.read_with(cx, |main, _| {
            main.debug_phase() == Phase::Paused && main.debug_frames().first().is_some_and(|frame| frame.line == 8)
        })
    });
    assert!(stepped, "Step Over did not reach the WriteLine's line");
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((6, 0)), window, cx));
    cx.simulate_keystrokes("ctrl-shift-f10");
    let moved =
        pump(cx, |cx| main.read_with(cx, |main, _| main.debug_frames().first().is_some_and(|frame| frame.line == 6)));
    assert!(moved, "Set Next Statement did not move the thread to the assignment's line");
    let marked = main
        .read_with(cx, |_, cx| rsharp_app::buffer::find(&source, cx).and_then(|buffer| buffer.read(cx).execution_line));
    assert_eq!(marked, Some(6), "the editor marks the line Set Next Statement put the thread on");
    cx.simulate_keystrokes("shift-f5");
}

#[test]
fn set_value_changes_a_local_and_the_program_goes_on_with_it() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-set-value", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", PROGRAM);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((7, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    let paused = pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused));
    assert!(paused, "the program did not stop at its breakpoint");

    click(cx, "debug variable a");
    let offered = pump(cx, |cx| cx.debug_bounds("debug field: Set value").is_some());
    assert!(offered, "choosing a local did not offer Set value");
    click(cx, "debug field: Set value");
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("10");
    click(cx, "debug: Set Value");
    let changed = pump(cx, |cx| {
        main.read_with(cx, |main, cx| main.debug_view().read(cx).variable_value("a").as_deref() == Some("10"))
    });
    assert!(changed, "the Variables tree did not show a = 10 after Set value");

    cx.simulate_keystrokes("f5");
    let ended = pump(cx, |cx| main.read_with(cx, |main, _| !main.is_debugging()));
    assert!(ended, "the program did not run to its end");
    let output = main.read_with(cx, |main, cx| main.debug_console().read(cx).text());
    assert!(output.contains("answer 11"), "the program went on with a = 10: {output:?}");
}

const THREADS_PROGRAM: &str = "using System;\nusing System.Threading;\n\nclass Program\n{\n    static void Main()\n    {\n        var worker = new Thread(Work) { Name = \"worker\" };\n        worker.Start();\n        worker.Join();\n    }\n\n    static void Work()\n    {\n        var total = 0;\n        for (var i = 0; i < 100; i++)\n        {\n            total += i;\n            Thread.Sleep(50);\n        }\n    }\n}\n";

#[test]
fn the_debug_window_lists_a_paused_programs_threads_and_choosing_one_shows_its_frames() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-threads", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", THREADS_PROGRAM);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((17, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    let paused = pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused));
    assert!(paused, "the worker thread did not reach the breakpoint");

    let threads: Vec<i64> = main.read_with(cx, |main, _| main.debug_threads().iter().map(|thread| thread.id).collect());
    assert!(threads.len() >= 2, "the paused program's Main and worker threads are listed: {threads:?}");
    assert!(cx.debug_bounds("debug thread 0").is_some(), "the Debug window shows the Threads list");
    let chosen = main.read_with(cx, |main, _| main.debug_thread());
    let other = threads.iter().position(|id| Some(*id) != chosen).expect("a thread other than the stopped one");
    click(cx, if other == 0 { "debug thread 0" } else { "debug thread 1" });
    let picked = pump(cx, |cx| main.read_with(cx, |main, _| main.debug_thread() == Some(threads[other])));
    assert!(picked, "choosing a thread did not make it the one shown");
    assert!(!main.read_with(cx, |main, _| main.debug_frames().is_empty()), "the chosen thread's frames are listed");
    assert_eq!(
        main.read_with(cx, |main, cx| main.debug_view().read(cx).chosen_thread()),
        Some(threads[other]),
        "the Threads list shows the chosen thread"
    );
    cx.simulate_keystrokes("shift-f5");
}

const SMART_STEP_PROGRAM: &str = "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        var total = Sum(1, Double(5));\n        Console.WriteLine($\"total {total}\");\n    }\n\n    static int Double(int x)\n    {\n        return x * 2;\n    }\n\n    static int Sum(int a, int b)\n    {\n        return a + b;\n    }\n}\n";

#[test]
fn smart_step_into_steps_into_the_call_chosen() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-smart-step", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", SMART_STEP_PROGRAM);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((6, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    let paused = pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused));
    assert!(paused, "the program did not stop at its breakpoint");

    cx.simulate_keystrokes("shift-alt-f11");
    let listed = pump(cx, |cx| cx.debug_bounds("smart step into dialog").is_some());
    assert!(listed, "Shift+Alt+F11 did not offer the statement's calls");
    let labels: Vec<String> =
        main.read_with(cx, |main, _| main.step_targets().iter().map(|target| target.label.clone()).collect());
    let sum = labels
        .iter()
        .position(|label| label.contains("Sum"))
        .unwrap_or_else(|| panic!("Sum is not among the calls offered: {labels:?}"));
    click(cx, if sum == 0 { "smart step into target 0" } else { "smart step into target 1" });

    let stepped = pump(cx, |cx| {
        main.read_with(cx, |main, _| {
            main.debug_phase() == Phase::Paused
                && main
                    .debug_frames()
                    .first()
                    .is_some_and(|frame| frame.name.contains("Sum") && (16..=17).contains(&frame.line))
        })
    });
    let frame = main.read_with(cx, |main, _| main.debug_frames().first().cloned());
    assert!(stepped, "Smart Step Into did not stop in Sum's body: {:?}", frame.map(|frame| (frame.name, frame.line)));
    assert_eq!(
        main.read_with(cx, |main, _| main.debug_frames()[0].path()),
        Some(source.clone()),
        "the step went into the program's own file"
    );
    cx.simulate_keystrokes("shift-f5");
}

#[test]
fn a_debug_configuration_that_runs_in_a_terminal_starts_its_program_there() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-terminal", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\nusing System.Threading;\n\nclass Program\n{\n    static void Main()\n    {\n        Console.WriteLine(\"hello from the terminal\");\n        Thread.Sleep(300);\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    // The folder's own configuration, running the program in a terminal.
    let store = rsharp_core::state::StateStore::open(run.state.clone());
    store.write_for(
        &run.folder,
        &rsharp_core::run::configuration::RunConfigurations {
            configurations: vec![rsharp_core::run::RunConfiguration {
                name: "App".into(),
                project: run.folder.join("App/App.csproj"),
                use_terminal: true,
                ..rsharp_core::run::RunConfiguration::default()
            }],
            selected: Some("App".into()),
        },
    );
    store.flush();

    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    assert_eq!(
        main.read_with(cx, |main, _| main.chosen_run_configuration().map(|configuration| configuration.use_terminal)),
        Some(true),
        "the folder's own configuration runs the program in a terminal"
    );
    cx.simulate_keystrokes("f5");
    let started = pump(cx, |cx| main.read_with(cx, |main, _| main.debug_terminal().is_some()));
    assert!(
        started,
        "the program's Terminal tab did not start: {:?}",
        main.read_with(cx, |main, _| main.said().map(str::to_string))
    );
    let text = |main: &Entity<MainWindow>, cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| {
            let Some(id) = main.debug_terminal() else { return String::new() };
            let panel = main.terminal().read(cx);
            panel.session(id).map(|session| session.screen.lock().unwrap().visible_text()).unwrap_or_default()
        })
    };
    let wrote = pump(cx, |cx| text(&main, cx).contains("hello from the terminal"));
    assert!(wrote, "the program's output reached its Terminal tab: {:?}", text(&main, cx));
    let ended = pump(cx, |cx| main.read_with(cx, |main, _| !main.is_debugging()));
    assert!(ended, "the session ended when the program did");
}

#[test]
fn a_step_over_shows_what_its_call_returned() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-return-value", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static int Double(int x)\n    {\n        return x * 2;\n    }\n\n    static void Main()\n    {\n        var value = Double(21);\n        Console.WriteLine($\"value {value}\");\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((11, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    assert!(pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused)));
    cx.simulate_keystrokes("f10");
    assert!(
        pump(cx, |cx| {
            main.read_with(cx, |main, _| {
                main.debug_phase() == Phase::Paused && main.debug_frames().first().is_some_and(|frame| frame.line == 12)
            })
        }),
        "Step Over did not reach the WriteLine's line"
    );

    let returned = main.read_with(cx, |main, cx| {
        main.debug_view()
            .read(cx)
            .return_values()
            .iter()
            .map(|row| (row.name.clone(), row.value.clone()))
            .collect::<Vec<_>>()
    });
    assert_eq!(returned.len(), 1, "the step left one returned value: {returned:?}");
    assert!(returned[0].0.contains("Double"), "it names the method that returned it: {returned:?}");
    assert_eq!(returned[0].1, "42", "and carries what it returned");
    assert!(cx.debug_bounds("debug return value 0").is_some(), "the Debug window shows the Return values group");
    cx.simulate_keystrokes("shift-f5");
}

#[test]
fn evaluate_expression_shows_the_values_members() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-evaluate-members", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\nusing System.Collections.Generic;\n\nclass Program\n{\n    static void Main()\n    {\n        var numbers = new List<int> { 3, 7, 11 };\n        Console.WriteLine(numbers.Count);\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    // The WriteLine's line: numbers is assigned by then.
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((8, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    assert!(pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused)));

    cx.simulate_keystrokes("alt-f8");
    assert!(pump(cx, |cx| cx.debug_bounds("evaluate dialog").is_some()));
    click(cx, "evaluate field: Expression");
    cx.simulate_input("numbers");
    cx.simulate_keystrokes("enter");
    assert!(
        pump(cx, |cx| {
            main.read_with(cx, |main, cx| {
                main.evaluate_dialog().is_some_and(|dialog| dialog.read(cx).result().is_some())
            })
        }),
        "Evaluate Expression never answered"
    );
    let (result, members) = main.read_with(cx, |main, cx| {
        main.evaluate_dialog()
            .map(|dialog| (dialog.read(cx).result().map(str::to_string), dialog.read(cx).members().to_vec()))
            .unwrap_or_default()
    });
    assert!(
        result.as_deref().is_some_and(|result| result.contains("Count = 3")),
        "the list's count is the result: {result:?}"
    );
    assert!(
        members.iter().any(|(name, value, _)| name == "[2]" && value == "11"),
        "the evaluated list's members were not shown; result {result:?}, members {members:?}"
    );
    assert!(cx.debug_bounds("evaluate member 0").is_some(), "the members are rows in the dialog");
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("shift-f5");
}

#[test]
fn stepping_over_an_await_stops_after_it() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-async-step", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\nusing System.Threading.Tasks;\n\nclass Program\n{\n    static async Task<int> Work()\n    {\n        var value = await Compute();\n        return value + 1;\n    }\n\n    static async Task<int> Compute()\n    {\n        await Task.Delay(10);\n        return 41;\n    }\n\n    static async Task Main()\n    {\n        var result = await Work();\n        Console.WriteLine($\"result {result}\");\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((7, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    assert!(pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused)));

    cx.simulate_keystrokes("f10");
    let stepped = pump(cx, |cx| {
        main.read_with(cx, |main, _| {
            main.debug_phase() == Phase::Paused && main.debug_frames().first().is_some_and(|frame| frame.line == 8)
        })
    });
    let frame = main.read_with(cx, |main, _| main.debug_frames().first().map(|frame| (frame.name.clone(), frame.line)));
    assert!(stepped, "Step Over did not stop after the await: {frame:?}");
    assert_eq!(
        main.read_with(cx, |main, _| main.debug_frames()[0].path()),
        Some(source.clone()),
        "the stop is in the method that awaited"
    );
    cx.simulate_keystrokes("shift-f5");
}

#[test]
fn a_stop_sets_the_frame_values_in_at_the_end_of_its_line() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-inline-values", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", PROGRAM);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    // The WriteLine's line: a and b are both assigned by then.
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((8, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    assert!(pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused)));

    let inline = main.read_with(cx, |_, cx| {
        rsharp_app::buffer::find(&source, cx).and_then(|buffer| buffer.read(cx).inline_values.clone())
    });
    let text = inline.as_ref().map(|(_, text)| text.clone()).unwrap_or_default();
    assert_eq!(
        inline.as_ref().map(|(line, _)| *line),
        Some(8),
        "the values are set in on the stopped line: {inline:?}"
    );
    assert!(text.contains("a = 41") && text.contains("b = 42"), "the frame's values are set in: {text:?}");

    cx.simulate_keystrokes("f5");
    assert!(pump(cx, |cx| main.read_with(cx, |main, _| !main.is_debugging())));
    let cleared = main.read_with(cx, |_, cx| {
        rsharp_app::buffer::find(&source, cx).and_then(|buffer| buffer.read(cx).inline_values.clone())
    });
    assert!(cleared.is_none(), "running on cleared the values: {cleared:?}");
}

#[test]
fn a_function_breakpoint_stops_the_program_in_its_method() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-function-breakpoint", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static int Double(int x)\n    {\n        return x * 2;\n    }\n\n    static void Main()\n    {\n        Console.WriteLine(Double(21));\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);

    // Rider's Add Function Breakpoint, and its name typed in the form.
    open_breakpoints(cx);
    click(cx, "button: Add Function Breakpoint");
    click(cx, "breakpoint field: Function name");
    cx.simulate_input("Program.Double");
    assert!(
        pump(cx, |cx| cx.debug_bounds("breakpoint row: Program.Double").is_some()),
        "the dialog lists the function breakpoint by its name"
    );
    click(cx, "button: Done");

    cx.simulate_keystrokes("f5");
    assert!(pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused)));
    let frame =
        main.read_with(cx, |main, _| main.debug_frames().first().map(|frame| (frame.name.clone(), frame.path())));
    let (name, path) = frame.clone().unwrap_or_default();
    assert!(name.contains("Double"), "the program stopped in the method the breakpoint names: {frame:?}");
    assert_eq!(path, Some(source), "and in its own file");
    cx.simulate_keystrokes("shift-f5");
}

/// The program the Hot Reload test edits: it greets every tick, forever, so an edit can reach it while it runs. The
/// greeting is a method of its own, called on every tick: a method on the stack would wait for a restart, as Roslyn warns.
const GREETING_PROGRAM: &str = "using System;\nusing System.Threading;\n\nfor (int i = 1; ; i++)\n{\n    Greeter.Greet(i);\n    Thread.Sleep(100);\n}\n";

const GREETER: &str = "using System;\n\nstatic class Greeter\n{\n    public static void Greet(int i)\n    {\n        Console.WriteLine($\"Hello {i}\");\n    }\n}\n";

#[test]
fn a_saved_edit_reaches_the_program_the_debugger_runs() {
    let _alone = alone();
    saved_edit_reaches_program(false);
}

#[test]
fn a_saved_edit_in_a_project_beside_the_solution_reaches_the_program() {
    let _alone = alone();
    saved_edit_reaches_program(true);
}

fn saved_edit_reaches_program(project_beside_solution: bool) {
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let run =
        Run::new(if project_beside_solution { "debugger-hot-reload-sibling" } else { "debugger-hot-reload" }, &[]);
    let project = if project_beside_solution { run.folder.join("App") } else { run.folder.clone() };
    let workspace = if project_beside_solution {
        write(&run.folder, "Solution/App.slnx", "<Solution><Project Path=\"../App/App.csproj\" /></Solution>");
        run.folder.join("Solution/App.slnx")
    } else {
        run.folder.clone()
    };
    write(&project, "App.csproj", EXE_PROJECT);
    write(&project, "Program.cs", GREETING_PROGRAM);
    write(&project, "Greeter.cs", GREETER);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&project.join("Greeter.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = run.window(&mut cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&workspace, window, cx);
        // The caret at the start of `Hello`, on the line that greets.
        main.open_file(&source, Some((6, 28)), window, cx);
    });
    let loaded = pump(cx, |cx| main.read_with(cx, |main, _| main.server_ready() || main.server_failed().is_some()));
    assert!(loaded, "Roslyn loaded the project: {:?}", main.read_with(cx, |main, _| main.server_status()));
    assert_eq!(main.read_with(cx, |main, _| main.server_failed().map(str::to_string)), None, "Roslyn failed");

    // A breakpoint on the greeting line: the program stops in the method the edit changes, on its stack.
    cx.simulate_keystrokes("f9");
    cx.run_until_parked();
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused)),
        "the program stopped at the greeting's breakpoint: {:?}",
        output(&main, cx)
    );

    // The stop put the caret where the program paused; the edit is made at the greeting.
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((6, 28)), window, cx));
    cx.run_until_parked();

    // `Hello` replaced by `Howdy`: the file saved while the program is paused is what Hot Reload applies.
    cx.simulate_keystrokes("shift-right shift-right shift-right shift-right shift-right");
    cx.simulate_input("Howdy");
    cx.run_until_parked();
    let typed = main
        .read_with(cx, |main, cx| main.editor().map(|editor| editor.read(cx).text().to_string()).unwrap_or_default());
    assert!(typed.contains("Console.WriteLine($\"Howdy {i}\");"), "the edit lands on the greeting: {typed}");
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-s" } else { "ctrl-s" });
    let applied = |cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| main.debug_console().read(cx).text().contains("C# changes applied"))
    };
    assert!(pump(cx, applied), "the Debug console says Hot Reload applied the save: {:?}", output(&main, cx));
    assert_eq!(
        main.read_with(cx, |main, _| main.breakpoint_verified(&source, 6)),
        Some(true),
        "the breakpoint was bound again in the code the update replaced"
    );

    // The breakpoint taken away, going on runs the edited statement the program was paused in.
    cx.simulate_keystrokes("f9");
    cx.run_until_parked();
    cx.simulate_keystrokes("f5");
    let edited = |cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| main.debug_console().read(cx).text().contains("Howdy "))
    };
    assert!(pump(cx, edited), "the saved edit ran in the program: {:?}", output(&main, cx));

    // Shift+F5's Stop ends the session, its program with it.
    cx.simulate_keystrokes("shift-f5");
    let stopped = pump(cx, |cx| main.read_with(cx, |main, _| !main.is_debugging()));
    assert!(
        stopped,
        "the session stopped: {:?}",
        main.read_with(cx, |main, _| (main.debug_phase(), main.said().map(str::to_string)))
    );
}

/// The greeting with a static field, whose type needs a new program to change.
const RUDE_GREETER: &str = "using System;\n\nstatic class Greeter\n{\n    static int calls;\n\n    public static void Greet(int i)\n    {\n        Console.WriteLine($\"Hello {i} {++calls}\");\n    }\n}\n";

#[test]
fn an_edit_the_runtime_cannot_take_restarts_the_program() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let run = Run::new("debugger-hot-reload-restart", &[]);
    write(&run.folder, "App.csproj", EXE_PROJECT);
    write(&run.folder, "Program.cs", GREETING_PROGRAM);
    write(&run.folder, "Greeter.cs", RUDE_GREETER);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("Greeter.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = run.window(&mut cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        // The caret at the start of `int`, on the field whose type changes.
        main.open_file(&source, Some((4, 11)), window, cx);
    });
    let loaded = pump(cx, |cx| main.read_with(cx, |main, _| main.server_ready() || main.server_failed().is_some()));
    assert!(loaded, "Roslyn loaded the project: {:?}", main.read_with(cx, |main, _| main.server_status()));
    assert_eq!(main.read_with(cx, |main, _| main.server_failed().map(str::to_string)), None, "Roslyn failed");

    cx.simulate_keystrokes("f5");
    let greeting = |cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| main.debug_console().read(cx).text().contains("Hello 2"))
    };
    assert!(pump(cx, greeting), "the debugged program greets: {:?}", output(&main, cx));

    // `int` becomes `long`: a field's type is an edit only a new program runs.
    cx.simulate_keystrokes("shift-right shift-right shift-right");
    cx.simulate_input("long");
    cx.run_until_parked();
    let typed = main
        .read_with(cx, |main, cx| main.editor().map(|editor| editor.read(cx).text().to_string()).unwrap_or_default());
    assert!(typed.contains("static long calls;"), "the edit lands on the field: {typed}");
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-s" } else { "ctrl-s" });
    let restarted = |cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| {
            main.debug_console().read(cx).text().contains("restarting the program to apply the changes")
        })
    };
    assert!(pump(cx, restarted), "the rude edit restarted the program: {:?}", output(&main, cx));
    assert!(pump(cx, greeting), "the restarted program greets: {:?}", output(&main, cx));
    assert!(main.read_with(cx, |main, _| main.is_debugging()), "a new session runs");

    // Shift+F5's Stop ends it.
    cx.simulate_keystrokes("shift-f5");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| !main.is_debugging())),
        "the session stopped: {:?}",
        output(&main, cx)
    );
}

fn output(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> String {
    main.read_with(cx, |main, cx| main.debug_console().read(cx).text())
}

#[test]
fn an_unsaved_edit_is_applied_before_resuming() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let run = Run::new("debugger-hot-reload-resume", &[]);
    write(&run.folder, "App.csproj", EXE_PROJECT);
    write(&run.folder, "Program.cs", GREETING_PROGRAM);
    write(&run.folder, "Greeter.cs", GREETER);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("Greeter.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = run.window(&mut cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&source, Some((6, 28)), window, cx);
    });
    let loaded = pump(cx, |cx| main.read_with(cx, |main, _| main.server_ready() || main.server_failed().is_some()));
    assert!(loaded, "Roslyn loaded the project: {:?}", main.read_with(cx, |main, _| main.server_status()));
    assert_eq!(main.read_with(cx, |main, _| main.server_failed().map(str::to_string)), None, "Roslyn failed");

    cx.simulate_keystrokes("f9");
    cx.run_until_parked();
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused)),
        "the program stopped at the greeting's breakpoint: {:?}",
        output(&main, cx)
    );

    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((6, 28)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("shift-right shift-right shift-right shift-right shift-right");
    cx.simulate_input("Howdy");
    cx.run_until_parked();
    assert!(
        std::fs::read_to_string(&source).unwrap().contains("Hello {i}"),
        "the edit is not saved: resuming is what applies it"
    );

    // Rider's "Changes will be applied on the next step or resume": the program runs the edit it never saved.
    cx.simulate_keystrokes("f5");
    let edited = |cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| main.debug_console().read(cx).text().contains("Howdy "))
    };
    assert!(pump(cx, edited), "the unsaved edit ran when the program resumed: {:?}", output(&main, cx));

    cx.simulate_keystrokes("shift-f5");
    assert!(pump(cx, |cx| main.read_with(cx, |main, _| !main.is_debugging())), "the session stopped");
}

#[test]
fn apply_hot_reload_changes_applies_a_debugged_programs_edit() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let run = Run::new("debugger-hot-reload-action", &[]);
    write(&run.folder, "App.csproj", EXE_PROJECT);
    write(&run.folder, "Program.cs", GREETING_PROGRAM);
    write(&run.folder, "Greeter.cs", GREETER);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("Greeter.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = run.window(&mut cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&source, Some((6, 28)), window, cx);
    });
    let loaded = pump(cx, |cx| main.read_with(cx, |main, _| main.server_ready() || main.server_failed().is_some()));
    assert!(loaded, "Roslyn loaded the project: {:?}", main.read_with(cx, |main, _| main.server_status()));
    assert_eq!(main.read_with(cx, |main, _| main.server_failed().map(str::to_string)), None, "Roslyn failed");

    cx.simulate_keystrokes("f5");
    let greeting = |cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| main.debug_console().read(cx).text().contains("Hello 2"))
    };
    assert!(pump(cx, greeting), "the debugged program greets: {:?}", output(&main, cx));

    cx.simulate_keystrokes("shift-right shift-right shift-right shift-right shift-right");
    cx.simulate_input("Howdy");
    cx.run_until_parked();
    assert!(
        std::fs::read_to_string(&source).unwrap().contains("Hello {i}"),
        "the edit is not saved: Apply Hot Reload Changes applies the editors' text"
    );

    // Rider's Apply Hot Reload Changes (Alt+F10) for the debugged program, the file unsaved.
    cx.simulate_keystrokes("alt-f10");
    let applied = |cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| main.debug_console().read(cx).text().contains("C# changes applied"))
    };
    assert!(pump(cx, applied), "Alt+F10 applied the unsaved edit: {:?}", output(&main, cx));
    let edited = |cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| main.debug_console().read(cx).text().contains("Howdy "))
    };
    assert!(pump(cx, edited), "the program printed the edit Alt+F10 applied: {:?}", output(&main, cx));
    assert!(main.read_with(cx, |main, _| main.debug_phase() == Phase::Running), "the program ran on");

    cx.simulate_keystrokes("shift-f5");
    assert!(pump(cx, |cx| main.read_with(cx, |main, _| !main.is_debugging())), "the session stopped");
}

#[test]
fn closing_the_last_tab_releases_its_buffer() {
    let _alone = alone();
    let run = Run::new("debugger-closed-buffer", &[("Program.cs", "class Program {}\n")]);
    let source = rsharp_core::paths::canonical(&run.folder.join("Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, None, window, cx));
    cx.run_until_parked();
    let (buffer, editor) = main.read_with(cx, |main, cx| {
        let editor = main.editor().unwrap();
        (editor.read(cx).buffer.downgrade(), editor.downgrade())
    });

    cx.simulate_keystrokes("secondary-w");
    cx.run_until_parked();
    // The frame dropped its editor; the next effect cycle releases the editor's buffer too.
    cx.update(|_, _| {});
    assert!(main.read_with(cx, |main, _| main.editor().is_none()), "the last tab closed");
    editor.assert_released();
    buffer.assert_released();
}

#[test]
fn a_breakpoint_follows_its_code_through_an_edit() {
    let _alone = alone();
    let run = Run::new(
        "debugger-breakpoint-follow",
        &[(
            "Program.cs",
            "class Program\n{\n    static void Main()\n    {\n        System.Console.WriteLine(1);\n    }\n}\n",
        )],
    );
    let source = rsharp_core::paths::canonical(&run.folder.join("Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((4, 8)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.run_until_parked();
    assert_eq!(main.read_with(cx, |main, _| main.breakpoints().lines(&source)), vec![4], "F9 set the breakpoint");

    // A line inserted at the file's start carries the statement, and its breakpoint, down one.
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((0, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        main.read_with(cx, |main, _| main.breakpoints().lines(&source)),
        vec![5],
        "the breakpoint moved with its code"
    );
    let marks = main.read_with(cx, |_, cx| {
        rsharp_app::buffer::find(&source, cx).map(|buffer| buffer.read(cx).breakpoints.clone()).unwrap_or_default()
    });
    assert_eq!(marks.iter().map(|mark| mark.line).collect::<Vec<_>>(), vec![5], "the gutter shows it at its new line");

    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-s" } else { "ctrl-s" });
    cx.simulate_keystrokes("secondary-w");
    cx.update(|_, _| {});
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((0, 0)), window, cx));
    cx.simulate_keystrokes("enter");
    assert_eq!(
        main.read_with(cx, |main, _| main.breakpoints().lines(&source)),
        vec![6],
        "the reopened buffer's first edit is not mistaken for its previous buffer's edit"
    );
}

#[test]
fn an_edit_the_runtime_cannot_take_asks_when_the_restart_setting_is_off() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let run = Run::new("debugger-hot-reload-ask", &[]);
    write(&run.folder, "App.csproj", EXE_PROJECT);
    write(&run.folder, "Program.cs", GREETING_PROGRAM);
    write(&run.folder, "Greeter.cs", RUDE_GREETER);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("Greeter.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = run.window(&mut cx);
    cx.update(|_, cx| {
        cx.global_mut::<rsharp_app::state::Ide>().servers = true;
        let mut settings = rsharp_app::settings::settings(cx).clone();
        settings.hot_reload_restart_on_rude_edit = false;
        rsharp_app::settings::apply(settings, cx);
    });
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&source, Some((4, 11)), window, cx);
    });
    let loaded = pump(cx, |cx| main.read_with(cx, |main, _| main.server_ready() || main.server_failed().is_some()));
    assert!(loaded, "Roslyn loaded the project: {:?}", main.read_with(cx, |main, _| main.server_status()));
    assert_eq!(main.read_with(cx, |main, _| main.server_failed().map(str::to_string)), None, "Roslyn failed");

    cx.simulate_keystrokes("f5");
    let greeting = |cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| main.debug_console().read(cx).text().contains("Hello 2"))
    };
    assert!(pump(cx, greeting), "the debugged program greets: {:?}", output(&main, cx));

    cx.simulate_keystrokes("shift-right shift-right shift-right");
    cx.simulate_input("long");
    cx.run_until_parked();
    assert!(main.read_with(cx, |main, cx| {
        main.editor()
            .map(|editor| editor.read(cx).text().to_string())
            .unwrap_or_default()
            .contains("static long calls;")
    }));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-s" } else { "ctrl-s" });
    assert!(
        pump(cx, |cx| cx.debug_bounds("choice dialog").is_some()),
        "the edit the runtime cannot take asks whether to restart: {:?}",
        output(&main, cx)
    );
    assert!(
        main.read_with(cx, |main, cx| main.choice_dialog().map(|dialog| dialog.read(cx).chosen().map(str::to_string)))
            == Some(Some("Yes".into())),
        "Yes is the answer Enter would take"
    );

    click(cx, "choice: Yes");
    let restarted = |cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| {
            main.debug_console().read(cx).text().contains("restarting the program to apply the changes")
        })
    };
    assert!(pump(cx, restarted), "the answer restarted the program: {:?}", output(&main, cx));
    assert!(pump(cx, greeting), "the restarted program greets: {:?}", output(&main, cx));
    assert!(main.read_with(cx, |main, _| main.choice_dialog().is_none()), "the question is gone");

    cx.simulate_keystrokes("shift-f5");
    assert!(pump(cx, |cx| main.read_with(cx, |main, _| !main.is_debugging())), "the session stopped");
}

/// A program whose stop passes through System.Linq, an assembly that ships no symbols: its frame is decompiled.
const LINQ_PROGRAM: &str = "using System;\nusing System.Linq;\n\nclass Program\n{\n    static int Selector(int x) => x * 2;\n\n    static void Main()\n    {\n        int[] numbers = { 1, 2, 3 };\n        int[] doubled = numbers.Select(Selector).ToArray();\n        Console.WriteLine(string.Join(\",\", doubled));\n    }\n}\n";

#[test]
fn choosing_a_frame_without_symbols_opens_its_decompiled_source() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-decompiled", &[]);
    write(&run.folder, "App.csproj", EXE_PROJECT);
    write(&run.folder, "Program.cs", LINQ_PROGRAM);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((5, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused)),
        "the program stopped in the selector: {:?}",
        output(&main, cx)
    );
    let names: Vec<String> =
        main.read_with(cx, |main, _| main.debug_frames().iter().map(rsharp_dap::protocol::Frame::short_name).collect());
    assert!(
        names.get(1).is_some_and(|name| name.starts_with("System.Linq.dll!")),
        "System.Linq's frame is below the selector's: {names:?}"
    );

    // The library's frame has no source of its own: choosing it decompiles System.Linq and opens the source read-only.
    click(cx, "debug frame 1");
    let decompiled = |cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| {
            main.editor().is_some_and(|editor| {
                let editor = editor.read(cx);
                editor.path != source
                    && editor.path.to_string_lossy().contains("decompiled")
                    && editor.text().to_string().contains("namespace System.Linq")
            })
        })
    };
    assert!(pump(cx, &decompiled), "the decompiled source opened at the frame: {:?}", output(&main, cx));
    let read_only = main.read_with(cx, |main, cx| {
        main.editor().map(|editor| editor.read(cx).buffer.read(cx).read_only).unwrap_or(false)
    });
    assert!(read_only, "decompiled code opens read-only");

    // A frame already opened has its materialized path rather than the PDB's relative document name.
    click(cx, "debug frame 0");
    click(cx, "debug frame 1");
    assert!(pump(cx, &decompiled), "the same decompiled frame opened again");

    // Another call stops in the same library: its cached decompiled source must still open.
    click(cx, "debug frame 0");
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused)),
        "the next selector call stopped: {:?}",
        output(&main, cx)
    );
    click(cx, "debug frame 1");
    assert!(
        pump(cx, &decompiled),
        "the decompiled source opened again after the next stop: {:?}",
        main.read_with(cx, |main, _| main.said().map(str::to_string))
    );

    cx.simulate_keystrokes("shift-f5");
    assert!(pump(cx, |cx| main.read_with(cx, |main, _| !main.is_debugging())), "the session stopped");
}

/// Whether the system still has a process with this id running.
#[cfg(unix)]
fn process_running(id: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(id.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Waits for a process to go, as a stopped program does when its debugger ends.
#[cfg(unix)]
fn gone(id: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if !process_running(id) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    !process_running(id)
}

#[test]
fn pause_stops_a_running_program_and_shows_where_it_is_in_its_loop() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-pause", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        long turns = 0;\n        Console.WriteLine(\"looping\");\n        while (true)\n        {\n            turns++;\n        }\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, None, window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |cx| output(&main, cx).contains("looping")),
        "the program did not reach its loop: {:?}",
        output(&main, cx)
    );
    click(cx, "debug: Pause");
    let paused = pump(cx, |cx| {
        main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused && !main.debug_frames().is_empty())
    });
    let at = main.read_with(cx, |main, _| main.debug_frames().first().map(|frame| frame.line));
    assert!(paused, "Pause did not stop the program");
    assert!(at.is_some_and(|line| (8..=11).contains(&line)), "the pause is at {at:?}, not in the loop");
    let marked = main
        .read_with(cx, |_, cx| rsharp_app::buffer::find(&source, cx).and_then(|buffer| buffer.read(cx).execution_line));
    assert!(marked.is_some_and(|line| (8..=11).contains(&line)), "the editor marks the pause in the loop: {marked:?}");
    cx.simulate_keystrokes("shift-f5");
}

#[test]
fn the_breakpoints_dialog_lists_every_breakpoint_and_turns_one_off() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-breakpoints-list", &[]);
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
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((8, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");

    open_breakpoints(cx);
    assert!(cx.debug_bounds("breakpoint row: Program.cs:7").is_some(), "the dialog lists the first line breakpoint");
    assert!(cx.debug_bounds("breakpoint row: Program.cs:9").is_some(), "the dialog lists the second line breakpoint");
    assert!(cx.debug_bounds("breakpoint row: Any exception").is_some(), "the dialog lists Any exception");

    click(cx, "breakpoint row: Program.cs:7");
    click(cx, "check: Enabled");
    let first = main.read_with(cx, |main, _| main.breakpoints().of(&source)[0].enabled);
    assert!(!first, "the box turned the first line breakpoint off");
    let marks = main.read_with(cx, |_, cx| {
        rsharp_app::buffer::find(&source, cx).map(|buffer| buffer.read(cx).breakpoints.clone()).unwrap_or_default()
    });
    assert!(marks.iter().any(|mark| mark.line == 6 && !mark.enabled), "the gutter shows it turned off: {marks:?}");
    click(cx, "button: Done");
    assert!(main.read_with(cx, |main, _| main.breakpoints_dialog().is_none()), "Done closed the dialog");
}

#[test]
fn the_breakpoints_dialog_adds_and_removes_exception_breakpoints_and_the_folder_keeps_them() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-breakpoints-kept", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", PROGRAM);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, None, window, cx));
    cx.run_until_parked();
    open_breakpoints(cx);
    click(cx, "button: Add Exception Breakpoint");
    click(cx, "breakpoint field: Exception types");
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("System.InvalidOperationException");
    cx.run_until_parked();
    click(cx, "button: Done");
    assert_eq!(
        main.read_with(cx, |main, _| main.breakpoints().exceptions().first().map(|e| e.types.clone())),
        Some(vec!["System.InvalidOperationException".to_string()]),
        "the type typed in the dialog is the exception breakpoint's"
    );
    cx.update(|_, cx| rsharp_app::state::store(cx).flush());
    let kept = {
        let store = rsharp_core::state::StateStore::open(run.state.clone());
        store.read_for::<rsharp_core::debug::Breakpoints>(&run.folder).resolved(&run.folder)
    };
    assert_eq!(
        kept.exceptions().first().map(|e| e.types.clone()),
        Some(vec!["System.InvalidOperationException".to_string()]),
        "the folder keeps the exception breakpoint"
    );

    // Remove takes the chosen one away; Any exception stays, as Rider's cannot be removed.
    open_breakpoints(cx);
    click(cx, "breakpoint row: System.InvalidOperationException");
    click(cx, "button: Remove");
    assert!(
        main.read_with(cx, |main, _| main.breakpoints().exceptions().is_empty()),
        "Remove took the exception breakpoint away"
    );
    click(cx, "breakpoint row: Any exception");
    click(cx, "button: Remove");
    assert!(main.read_with(cx, |main, _| main.breakpoints().exceptions().is_empty()), "Any exception is never removed");
    click(cx, "button: Done");
}

#[test]
fn mute_breakpoints_lets_a_run_pass_a_line_breakpoint_and_an_exception_breakpoint_and_unmuting_stops_it_again() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-mute", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        for (int turn = 1; turn <= 2; turn++)\n        {\n            Console.WriteLine($\"turn {turn}\");\n            try\n            {\n                throw new InvalidOperationException($\"turn {turn}\");\n            }\n            catch (InvalidOperationException)\n            {\n            }\n            Console.WriteLine($\"passed {turn}\");\n            Console.ReadLine();\n        }\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    // The breakpoint on `Console.WriteLine($"turn {turn}");`, line 8 from 0.
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((8, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    open_breakpoints(cx);
    click(cx, "button: Add Exception Breakpoint");
    click(cx, "breakpoint field: Exception types");
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("System.InvalidOperationException");
    cx.run_until_parked();
    click(cx, "button: Done");

    // Mute Breakpoints: both stay, greyed, and the program runs past them both to its first read.
    main.update_in(cx, |main, _, cx| main.set_breakpoints_muted(true, cx));
    cx.run_until_parked();
    assert!(main.read_with(cx, |main, _| main.breakpoints_muted()), "Mute Breakpoints is on");
    cx.simulate_keystrokes("f5");
    let passed = pump(cx, |cx| output(&main, cx).contains("passed 1"));
    assert!(passed, "the muted program ran past both breakpoints: {:?}", output(&main, cx));
    assert!(
        main.read_with(cx, |main, _| main.debug_phase() == Phase::Running),
        "a muted breakpoint stopped the program: {:?}",
        main.read_with(cx, |main, _| main.said().map(str::to_string))
    );

    // Unmuted while it waits for its line: the next turn stops at the line, then at the exception.
    main.update_in(cx, |main, _, cx| main.set_breakpoints_muted(false, cx));
    // The adapter is told again in the background: the line breakpoint bound says the exception rule is in too.
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| main.breakpoint_verified(&source, 8) == Some(true))),
        "the line breakpoint bound again once unmuted"
    );
    main.update_in(cx, |main, _, cx| main.debug_input("\n", cx));
    let stopped = pump(cx, |cx| {
        main.read_with(cx, |main, cx| {
            main.debug_phase() == Phase::Paused
                && main.debug_view().read(cx).variable_value("turn").as_deref() == Some("2")
        })
    });
    assert!(stopped, "the unmuted line breakpoint did not stop the second turn: {:?}", output(&main, cx));
    assert_eq!(
        main.read_with(cx, |main, _| main.debug_frames().first().map(|frame| frame.line)),
        Some(8),
        "the stop is on the line breakpoint's statement"
    );
    cx.simulate_keystrokes("f5");
    let exception = pump(cx, |cx| {
        main.read_with(cx, |main, _| {
            main.debug_phase() == Phase::Paused && main.debug_frames().first().is_some_and(|frame| frame.line == 11)
        })
    });
    assert!(exception, "the unmuted exception breakpoint did not stop the next throw: {:?}", output(&main, cx));
    cx.simulate_keystrokes("shift-f5");
}

#[test]
fn an_exception_stop_lists_the_exception_as_exception_and_a_watch_reads_its_message() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-exception-watch", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        int index = 5;\n        throw new InvalidOperationException($\"bad index {index}\");\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, None, window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f5");
    let stopped = pump(cx, |cx| {
        main.read_with(cx, |main, cx| {
            main.debug_phase() == Phase::Paused
                && main.debug_view().read(cx).variable_names().iter().any(|name| name == "$exception")
        })
    });
    let (phase, names, said) = main.read_with(cx, |main, cx| {
        (main.debug_phase(), main.debug_view().read(cx).variable_names(), main.said().map(str::to_string))
    });
    assert!(
        stopped,
        "the exception stop does not list $exception: phase {phase:?}, values {names:?}, said {said:?}; console {:?}",
        output(&main, cx)
    );
    let value = main.read_with(cx, |main, cx| main.debug_view().read(cx).variable_value("$exception"));
    assert!(
        value.as_deref().is_some_and(|value| value.contains("InvalidOperationException")),
        "the exception's own value names its type: {value:?}"
    );

    scroll_to_row(cx, "debug field: Watch");
    click(cx, "debug field: Watch");
    cx.simulate_input("$exception.Message");
    let watched = pump(cx, |cx| {
        main.read_with(cx, |main, cx| {
            main.debug_view().read(cx).watches().iter().any(|(_, answer)| answer.contains("bad index 5"))
        })
    });
    let watches = main.read_with(cx, |main, cx| main.debug_view().read(cx).watches().to_vec());
    assert!(watched, "a watch of $exception.Message did not answer: {watches:?}");
    cx.simulate_keystrokes("shift-f5");
}

#[test]
fn what_is_typed_into_the_debug_console_is_the_programs_input() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-console-input", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        Console.Write(\"Name? \");\n        string? name = Console.ReadLine();\n        Console.WriteLine($\"read: {name}\");\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, None, window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f5");
    assert!(pump(cx, |cx| output(&main, cx).contains("Name? ")), "the program did not ask for its line");
    main.update_in(cx, |main, window, cx| {
        let editor = main.debug_console().read(cx).editor().clone();
        window.focus(&editor.focus_handle(cx));
    });
    cx.run_until_parked();
    cx.simulate_input("Ada");
    cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(" Lovelace".into())));
    cx.dispatch_action(rsharp_app::editor::Paste);
    cx.simulate_keystrokes("enter");
    let read = pump(cx, |cx| output(&main, cx).contains("read: Ada Lovelace"));
    assert!(read, "what was typed and pasted did not reach the program: {:?}", output(&main, cx));
    assert!(pump(cx, |cx| main.read_with(cx, |main, _| !main.is_debugging())), "the program read the line and ended");
}

#[test]
fn debugging_a_program_that_prints_100000_coloured_lines_leaves_the_console_free_of_colour_codes() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-coloured-output", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        for (int i = 0; i < 100_000; i++)\n        {\n            Console.WriteLine($\"\\u001b[3{i % 7 + 1}mline {i}\\u001b[0m\");\n        }\n        Console.WriteLine(\"printed\");\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, None, window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f5");
    let printed = pump(cx, |cx| output(&main, cx).contains("printed"));
    let text = output(&main, cx);
    assert!(printed, "the program did not print its last line; the console has {} characters", text.len());
    assert!(!text.contains('\u{1b}'), "the console kept the program's colour codes");
    assert!(!text.contains("[0m"), "the console kept the program's colour codes");
    assert!(text.contains("line 99999"), "the console kept the program's last numbered line");
    assert!(
        text.len() <= rsharp_app::console::OUTPUT_LIMIT,
        "the console kept {} characters, past its limit",
        text.len()
    );
}

#[test]
fn attach_to_process_lists_dotnet_programs_alone() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-attach-list", &[]);
    let Some((mut program, _source)) = running_program(&run) else { return };
    let id = program.id();
    // A program with no .NET in it, running beside the one a debugger can attach to.
    let mut other = std::process::Command::new("sleep")
        .arg("30")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("sleep starts");
    let other_id = other.id();
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);

    cx.simulate_keystrokes("secondary-shift-a");
    cx.simulate_input("attach to process");
    cx.simulate_keystrokes("enter");
    assert!(pump(cx, |cx| cx.debug_bounds("attach dialog").is_some()), "Find Action did not open Attach to Process");
    click(cx, "attach field: Search processes");
    cx.simulate_input(&id.to_string());
    assert!(pump(cx, |cx| cx.debug_bounds("attach row: AttachApp").is_some()), "the .NET program is not listed");
    let shown =
        main.read_with(cx, |main, cx| main.attach_dialog().map(|dialog| dialog.read(cx).shown(cx)).unwrap_or_default());
    assert!(shown.iter().any(|process| process.id == id && process.program() == "AttachApp"), "the program is listed");
    assert!(!shown.iter().any(|process| process.id == other_id), "a program with no .NET was listed");
    assert!(!shown.iter().any(|process| process.id == std::process::id()), "RSharp itself was listed");

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(main.read_with(cx, |main, _| main.attach_dialog().is_none()), "Escape closed the dialog");
    program.kill();
    let _ = other.kill();
}

#[cfg(unix)]
#[test]
fn closing_the_window_while_debugging_leaves_no_program_or_debugger_running() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-window-close", &[]);
    let started = run.folder.join("program.pid");
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        &format!(
            "using System;\nusing System.IO;\nusing System.Threading;\n\nclass Program\n{{\n    static void Main()\n    {{\n        File.WriteAllText(@\"{}\", Environment.ProcessId.ToString());\n        Thread.Sleep(Timeout.Infinite);\n    }}\n}}\n",
            started.display()
        ),
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, None, window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |_| std::fs::read_to_string(&started).is_ok()),
        "the program did not start: {:?}",
        output(&main, cx)
    );
    let program: u32 = std::fs::read_to_string(&started).unwrap().trim().parse().expect("the program wrote its id");
    let adapter = main.read_with(cx, |main, _| main.debug_adapter_process());
    assert!(process_running(program), "the program was not running under the debugger");

    assert!(cx.simulate_close(), "the window closes as a person closes it");
    assert!(gone(program), "after the window closed, the program being debugged went on running");
    if let Some(adapter) = adapter {
        assert!(gone(adapter), "after the window closed, the debugger went on running");
    }
}

#[cfg(unix)]
#[test]
fn a_debugger_killed_while_its_program_is_paused_takes_the_program_with_it() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-killed", &[]);
    let started = run.folder.join("program.pid");
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        &format!(
            "using System;\nusing System.IO;\nusing System.Threading;\n\nclass Program\n{{\n    static void Main()\n    {{\n        File.WriteAllText(@\"{}\", Environment.ProcessId.ToString());\n        Console.WriteLine(\"stopped here\");\n        Thread.Sleep(Timeout.Infinite);\n    }}\n}}\n",
            started.display()
        ),
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    // The breakpoint on the WriteLine after the file is written, so the program's id is there when it stops.
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((9, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused)),
        "the program did not stop at its breakpoint: {:?}",
        output(&main, cx)
    );
    let program: u32 = std::fs::read_to_string(&started).unwrap().trim().parse().expect("the program wrote its id");
    let adapter = main.read_with(cx, |main, _| main.debug_adapter_process()).expect("the debugger's process");
    assert!(process_running(program), "the program was not running while paused");

    // The debugger alone, as a crash ends it: what it started is left to whatever ends it next.
    let _ = std::process::Command::new("kill").arg("-9").arg(adapter.to_string()).status();
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| !main.is_debugging())),
        "the session did not end with its debugger"
    );
    assert!(gone(program), "the debugger died and the program it launched was left running");
}

#[test]
fn an_exception_stop_names_the_exception_at_the_line_that_threw_it() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-exception-line", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        int[] values = { 1, 2 };\n        int index = 5;\n        Console.WriteLine(values[index]);\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, None, window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f5");
    let stopped = pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused));
    assert!(stopped, "the unhandled exception stopped the program: {:?}", output(&main, cx));
    let (exception, inline) = main.read_with(cx, |_, cx| {
        rsharp_app::buffer::find(&source, cx)
            .map(|buffer| (buffer.read(cx).exception.clone(), buffer.read(cx).inline_values.clone()))
            .unwrap_or_default()
    });
    let (line, said) = exception.clone().unwrap_or_default();
    assert_eq!(line, 8, "the exception is named at the line that threw it: {exception:?}");
    assert!(said.contains("IndexOutOfRangeException"), "the exception's annotation names its type: {said:?}");
    assert!(
        inline.as_ref().is_some_and(|(at, values)| *at == 8 && values.contains("index = 5")),
        "the frame's values are set in on the throwing line: {inline:?}"
    );
    let status = main.read_with(cx, |main, _| main.said().map(str::to_string)).unwrap_or_default();
    assert!(status.contains("IndexOutOfRangeException"), "the status bar names the exception: {status:?}");
    cx.simulate_keystrokes("shift-f5");
}

/// Right-clicks what a debug selector names, opening its menu.
fn right_click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let at = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing on screen is {selector}")).center();
    cx.simulate_mouse_down(at, gpui::MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(at, gpui::MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
}

#[test]
fn the_variables_and_frames_menus_copy_values_add_watches_and_copy_the_stack() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-menus", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", PROGRAM);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    // The breakpoint on "int b = a + 1;", where a is 41.
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((7, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, cx| {
            main.debug_phase() == Phase::Paused
                && main.debug_view().read(cx).variable_value("a").as_deref() == Some("41")
        })),
        "the program stopped where a is 41: {:?}",
        output(&main, cx)
    );

    right_click(cx, "debug variable a");
    assert!(main.read_with(cx, |main, cx| main.debug_view().read(cx).menu_open()), "a variable's menu opens");
    click(cx, "debug menu: Copy Value");
    let copied = cx.update(|_, cx| cx.read_from_clipboard()).and_then(|item| item.text()).unwrap_or_default();
    assert_eq!(copied, "41", "the variable's menu copied its value");

    right_click(cx, "debug variable a");
    assert!(main.read_with(cx, |main, cx| main.debug_view().read(cx).menu_open()), "the variable's menu opens again");
    assert!(
        cx.debug_bounds("debug menu: Set Data Breakpoint").is_none(),
        "a top-level local has no parent object for a data breakpoint"
    );
    click(cx, "debug menu: Add to Watches");
    let watched = pump(cx, |cx| {
        main.read_with(cx, |main, cx| {
            main.debug_view()
                .read(cx)
                .watches()
                .iter()
                .any(|(expression, answer)| expression == "a" && answer.starts_with("41"))
        })
    });
    let watches = main.read_with(cx, |main, cx| main.debug_view().read(cx).watches().to_vec());
    assert!(watched, "the variable's menu watched it: {watches:?}");

    right_click(cx, "debug frame 0");
    click(cx, "debug menu: Copy Stack");
    let stack = cx.update(|_, cx| cx.read_from_clipboard()).and_then(|item| item.text()).unwrap_or_default();
    assert!(stack.contains("Program.Main"), "the frames' menu copied the stack: {stack:?}");
    assert!(stack.contains("Program.cs:8"), "the stack names where each frame is: {stack:?}");
    cx.simulate_keystrokes("shift-f5");
}

/// The program with a loop, whose second breakpoint is after the first's statement.
const LOOP_PROGRAM: &str = "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        var total = 0;\n        for (int i = 1; i <= 3; i++)\n        {\n            total += i;\n        }\n        Console.WriteLine($\"total {total}\");\n    }\n}\n";

#[test]
fn a_hit_count_the_debugger_cannot_read_is_refused_and_the_loop_goes_on_to_the_next_breakpoint() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-hit-count", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", LOOP_PROGRAM);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((9, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((11, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");

    open_breakpoints(cx);
    click(cx, "breakpoint row: Program.cs:10");
    click(cx, "breakpoint field: Hit count");
    cx.simulate_input("3 times");
    cx.run_until_parked();
    click(cx, "button: Done");
    assert_eq!(
        main.read_with(cx, |main, _| main.breakpoints().of(&source)[0].hit_count.clone()),
        "3 times",
        "the hit count typed in the dialog is kept with the breakpoint"
    );

    cx.simulate_keystrokes("f5");
    let stopped = pump(cx, |cx| {
        main.read_with(cx, |main, _| {
            main.debug_phase() == Phase::Paused && main.debug_frames().first().is_some_and(|frame| frame.line == 11)
        })
    });
    assert!(stopped, "the loop ran past the unreadable hit count to the next breakpoint: {:?}", output(&main, cx));
    assert_eq!(
        main.read_with(cx, |main, _| main.breakpoint_verified(&source, 9)),
        Some(false),
        "the breakpoint with the unreadable hit count is refused"
    );
    let said = main.read_with(cx, |main, _| main.breakpoint_message(&source, 9)).unwrap_or_default();
    assert!(
        said.starts_with("The hit count \"3 times\" is not one the debugger can read"),
        "the debugger's words say what it could not read: {said:?}"
    );
    cx.simulate_keystrokes("shift-f5");
}

/// ICorDebug offers data breakpoints on Windows x64 alone, so this is the one place they can be proven.
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn a_data_breakpoint_stops_the_program_when_the_value_it_watches_is_written() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-data-breakpoint", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        // As Passenger's DataBreakpointTests: the field written in a method, a statement after the write, since the
        // runtime stops only where the thread can be.
        "using System;\n\nclass Counter\n{\n    public int Count;\n\n    public void Set(int value)\n    {\n        Count = value;\n        Console.WriteLine($\"set {value}\");\n    }\n}\n\nclass Program\n{\n    static void Main()\n    {\n        var counter = new Counter();\n        Console.WriteLine(\"ready\");\n        counter.Set(2);\n        counter.Set(3);\n        Console.WriteLine(counter.Count);\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    // Paused on `Console.WriteLine("ready");`, counter made and its field not yet written.
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((18, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, cx| {
            main.debug_phase() == Phase::Paused && main.debug_view().read(cx).variable_value("counter").is_some()
        })),
        "the program stopped where counter is: {:?}",
        output(&main, cx)
    );

    // counter's own row opens to its fields; Count's row sets the data breakpoint.
    click(cx, "debug variable counter");
    assert!(pump(cx, |cx| cx.debug_bounds("debug variable Count").is_some()), "counter's members are listed");
    right_click(cx, "debug variable Count");
    click(cx, "debug menu: Set Data Breakpoint");
    let watching = pump(cx, |cx| {
        main.read_with(cx, |main, cx| {
            main.debug_view().read(cx).data_breakpoints().iter().any(|location| location.contains("Count"))
        })
    });
    let listed = main.read_with(cx, |main, cx| main.debug_view().read(cx).data_breakpoints());
    assert!(
        watching,
        "the debugger watches counter.Count: {listed:?}; it said {:?}",
        main.read_with(cx, |main, _| main.said().map(str::to_string))
    );
    // As Passenger's test waits for its watch to be bound before the program goes on.
    let bound = |main: &Entity<MainWindow>, cx: &mut VisualTestContext| {
        main.read_with(cx, |main, _| main.data_breakpoints_bound().to_vec())
    };
    assert!(
        pump(cx, |cx| bound(&main, cx).first().is_some_and(|(verified, _)| *verified)),
        "the debugger did not take the data breakpoint: {:?}",
        bound(&main, cx)
    );

    // The line breakpoint cleared, as Passenger's test has none: only the watched write stops the program, and the
    // runtime stops it at the statement after the write, which a line breakpoint there would also claim.
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((18, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    assert!(main.read_with(cx, |main, _| main.breakpoints().lines(&source).is_empty()), "the line breakpoint stayed");
    cx.simulate_keystrokes("f5");
    let stopped = pump(cx, |cx| {
        main.read_with(cx, |main, _| {
            main.debug_phase() == Phase::Paused
                && main.debug_frames().first().is_some_and(|frame| (8..=9).contains(&frame.line))
                && main.said().is_some_and(|said| said.contains("data breakpoint"))
        })
    });
    let frame = main.read_with(cx, |main, _| main.debug_frames().first().map(|frame| frame.line));
    assert!(
        stopped,
        "the write to counter.Count stopped the program: frame {frame:?}, said {:?}",
        main.read_with(cx, |main, _| main.said().map(str::to_string))
    );
    cx.simulate_keystrokes("shift-f5");
}

/// The exception a file's stop names at its line, if any: the line from 0 and the words.
fn thrown(source: &Path, main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> Option<(usize, String)> {
    main.read_with(cx, |_, cx| {
        rsharp_app::buffer::find(&source.to_path_buf(), cx).and_then(|b| b.read(cx).exception.clone())
    })
}

/// Adds an exception breakpoint on a type through the Breakpoints dialog, leaving the dialog open on it.
fn add_exception_breakpoint(cx: &mut VisualTestContext, type_name: &str) {
    click(cx, "button: Add Exception Breakpoint");
    click(cx, "breakpoint field: Exception types");
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input(type_name);
    cx.run_until_parked();
}

#[test]
fn an_exception_breakpoint_added_by_its_type_stops_where_that_type_is_thrown_as_its_condition_allows_and_at_no_other_type()
 {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-exception-type", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        for (int turn = 1; turn <= 3; turn++)\n        {\n            try\n            {\n                throw new ArgumentException($\"argument {turn}\");\n            }\n            catch (ArgumentException)\n            {\n            }\n            try\n            {\n                throw new InvalidOperationException($\"operation {turn}\");\n            }\n            catch (InvalidOperationException)\n            {\n            }\n        }\n        Console.WriteLine(\"done\");\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, None, window, cx));
    cx.run_until_parked();
    open_breakpoints(cx);
    add_exception_breakpoint(cx, "System.InvalidOperationException");
    click(cx, "breakpoint field: Condition");
    cx.simulate_input("turn == 2");
    cx.run_until_parked();
    click(cx, "button: Done");
    let added = main.read_with(cx, |main, _| main.breakpoints().exceptions().first().cloned()).expect("the breakpoint");
    assert_eq!(added.condition, "turn == 2", "the condition typed is the breakpoint's");
    assert_eq!(
        added.scenarios.len(),
        5,
        "Rider's new breakpoint on a type stops however it is handled: {:?}",
        added.scenarios
    );

    // The argument exceptions and the first turn's pass: the stop is the second turn's, at the line that threw it.
    cx.simulate_keystrokes("f5");
    let stopped =
        pump(cx, |cx| thrown(&source, &main, cx).is_some() || !main.read_with(cx, |main, _| main.is_debugging()));
    let at = thrown(&source, &main, cx);
    assert!(stopped && at.is_some(), "the program did not stop at the exception breakpoint: {:?}", output(&main, cx));
    let (line, said) = at.unwrap();
    assert_eq!(line, 17, "the stop is not at the line that threw it: {said:?}");
    assert!(said.starts_with("InvalidOperationException: operation 2"), "the stop names another exception: {said:?}");
    assert_eq!(
        main.read_with(cx, |main, cx| main.debug_view().read(cx).variable_value("turn")),
        Some("2".into()),
        "the condition let another turn's exception stop the program"
    );

    // Resumed, nothing else stops it: not the third turn's, which the condition rules out, nor any argument exception.
    cx.simulate_keystrokes("f5");
    assert!(!run_to_end(&main, cx), "the program stopped again after the second turn's exception");
    assert!(output(&main, cx).contains("done"), "the program did not run to its end: {:?}", output(&main, cx));
}

#[test]
fn an_exception_breakpoints_handler_options_stop_it_where_the_users_code_leaves_the_exception_or_where_it_handles_it() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-exception-handlers", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    // The first is handled where it is thrown; List.Sort catches the comparer's and throws its own, which the program handles.
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\nusing System.Collections.Generic;\n\nclass Program\n{\n    static void Main()\n    {\n        try\n        {\n            throw new InvalidOperationException(\"caught by the program\");\n        }\n        catch (InvalidOperationException)\n        {\n        }\n        List<int> numbers = new List<int> { 3, 1, 2 };\n        try\n        {\n            numbers.Sort((_, _) => throw new InvalidOperationException(\"left to List.Sort\"));\n        }\n        catch (InvalidOperationException)\n        {\n        }\n        Console.WriteLine(\"done\");\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, None, window, cx));
    cx.run_until_parked();
    open_breakpoints(cx);
    // "Any exception" off, so only the breakpoint on the type stops the program.
    click(cx, "breakpoint row: Any exception");
    click(cx, "check: Enabled");
    assert!(!main.read_with(cx, |main, _| main.breakpoints().any_exception().enabled), "Any exception stayed on");
    add_exception_breakpoint(cx, "System.InvalidOperationException");
    // Rider's user-unhandled: handled outside the user's code, or not at all.
    click(cx, "check: Handled in user code");
    click(cx, "button: Done");
    let scenarios = main.read_with(cx, |main, _| main.breakpoints().exceptions()[0].scenarios.clone());
    assert!(!scenarios.iter().any(|s| s == "handledInUserCode"), "User code stayed among its handlers: {scenarios:?}");

    let stop = |main: &Entity<MainWindow>, cx: &mut VisualTestContext| {
        pump(cx, |cx| thrown(&source, main, cx).is_some() || !main.read_with(cx, |main, _| main.is_debugging()));
        thrown(&source, main, cx)
    };
    cx.simulate_keystrokes("f5");
    let first = stop(&main, cx);
    assert!(
        first.as_ref().is_some_and(|(line, said)| *line == 17 && said.starts_with("InvalidOperationException: left to List.Sort")),
        "the user-unhandled breakpoint did not stop where List.Sort catches the comparer's exception: {first:?}; {:?}",
        output(&main, cx)
    );
    cx.simulate_keystrokes("f5");
    assert!(!run_to_end(&main, cx), "the program stopped at an exception its own code handles");

    // Handled in the user's code only: the program's own catches stop it, the comparer's exception does not.
    open_breakpoints(cx);
    click(cx, "breakpoint row: System.InvalidOperationException");
    click(cx, "check: Handled in user code");
    click(cx, "check: Handled in external code");
    click(cx, "check: Unhandled");
    click(cx, "button: Done");
    let mut scenarios = main.read_with(cx, |main, _| main.breakpoints().exceptions()[0].scenarios.clone());
    scenarios.sort();
    assert_eq!(
        scenarios,
        ["handledInUserCode", "thrownInExternalCode", "thrownInUserCode"],
        "the handler options taken"
    );
    cx.simulate_keystrokes("f5");
    let first = stop(&main, cx);
    assert!(
        first.as_ref().is_some_and(
            |(line, said)| *line == 9 && said.starts_with("InvalidOperationException: caught by the program")
        ),
        "the breakpoint did not stop at the exception the program catches: {first:?}; {:?}",
        output(&main, cx)
    );
    cx.simulate_keystrokes("f5");
    // List.Sort's own, thrown outside the user's code and caught by the program, shown where the program called it.
    let second = pump(cx, |cx| {
        thrown(&source, &main, cx).is_some_and(|(line, said)| line == 17 && said.contains("Failed to compare"))
            || !main.read_with(cx, |main, _| main.is_debugging())
    });
    let at = thrown(&source, &main, cx);
    assert!(
        second && at.as_ref().is_some_and(|(_, said)| said.starts_with("InvalidOperationException: Failed to compare")),
        "the second stop was not at List.Sort's own exception but {at:?}"
    );
    cx.simulate_keystrokes("f5");
    assert!(!run_to_end(&main, cx), "the program stopped a third time");
}

/// Clicks a row of the Variables tree and waits for its members, as a person opens a value.
fn open_row(main: &Entity<MainWindow>, cx: &mut VisualTestContext, name: &str, child: &str) {
    let row: &'static str = Box::leak(format!("debug variable {name}").into_boxed_str());
    let shown: &'static str = Box::leak(format!("debug variable {child}").into_boxed_str());
    scroll_to_row(cx, row);
    click(cx, row);
    let opened = pump(cx, |cx| {
        main.read_with(cx, |main, cx| main.debug_view().read(cx).variable_names().iter().any(|n| n == child))
    });
    let _ = shown;
    let (names, said) =
        main.read_with(cx, |main, cx| (main.debug_view().read(cx).variable_names(), main.said().map(str::to_string)));
    assert!(opened, "{name} did not open to {child}: Variables shows {names:?}; the window said {said:?}");
}

/// Scrolls the Debug window's values with the wheel until what a selector names is inside them, as a person looks for it.
fn scroll_to_row(cx: &mut VisualTestContext, row: &'static str) {
    for _ in 0..200 {
        cx.run_until_parked();
        let list = cx.debug_bounds("debug variables").expect("the Variables list is on screen");
        let lines = match cx.debug_bounds(row) {
            Some(at) if at.top() < list.top() => 3.,
            Some(at) if at.bottom() > list.bottom() => -3.,
            Some(_) => return,
            None => -3.,
        };
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: list.center(),
            delta: gpui::ScrollDelta::Lines(gpui::point(0., lines)),
            ..Default::default()
        });
    }
    panic!("scrolling the Debug window's values never showed {row}");
}

/// Set Value on a row of the tree: chosen, its value typed over, Set.
fn set_row(cx: &mut VisualTestContext, name: &str, value: &str) {
    let row: &'static str = Box::leak(format!("debug variable {name}").into_boxed_str());
    scroll_to_row(cx, row);
    click(cx, row);
    assert!(
        pump(cx, |cx| cx.debug_bounds("debug field: Set value").is_some()),
        "choosing {name} did not offer Set value"
    );
    scroll_to_row(cx, "debug field: Set value");
    click(cx, "debug field: Set value");
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input(value);
    scroll_to_row(cx, "debug: Set Value");
    click(cx, "debug: Set Value");
}

#[test]
fn a_value_set_on_an_element_listed_under_a_range_is_the_one_the_program_goes_on_with() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-set-range", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        int[] numbers = new int[250];\n        int[,] grid = new int[2, 2];\n        Console.WriteLine($\"set {numbers[150]} {grid[1, 0]}\");\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((8, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, cx| main.debug_view().read(cx).variable_value("grid").is_some())),
        "the program did not stop before the line that writes them: {:?}",
        output(&main, cx)
    );

    // An element listed under a range takes a value as any element does: the range's row is no step of its expression.
    open_row(&main, cx, "numbers", "[100..199]");
    open_row(&main, cx, "[100..199]", "[150]");
    set_row(cx, "[150]", "7");
    pump(cx, |cx| main.read_with(cx, |main, _| main.said().is_some_and(|said| !said.starts_with("Paused"))));
    let said = main.read_with(cx, |main, _| main.said().map(str::to_string)).unwrap_or_default();
    assert!(said.starts_with("Set "), "Set Value on numbers[150] was refused: {said:?}");
    open_row(&main, cx, "grid", "[1, ...]");
    open_row(&main, cx, "[1, ...]", "[1, 0]");
    set_row(cx, "[1, 0]", "5");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| main.said().is_some_and(|said| said.contains("to 5")))),
        "Set Value on grid[1, 0] was refused: {:?}",
        main.read_with(cx, |main, _| main.said().map(str::to_string))
    );

    cx.simulate_keystrokes("f5");
    assert!(pump(cx, |cx| output(&main, cx).contains("set ")), "the program did not write its values");
    assert!(output(&main, cx).contains("set 7 5"), "the program went on with other values: {:?}", output(&main, cx));
}

/// Evaluate Expression on an expression, answered or given up on.
fn evaluate(main: &Entity<MainWindow>, cx: &mut VisualTestContext, expression: &str) -> String {
    if main.read_with(cx, |main, _| main.evaluate_dialog().is_none()) {
        cx.simulate_keystrokes("alt-f8");
        assert!(pump(cx, |cx| cx.debug_bounds("evaluate dialog").is_some()), "Alt+F8 did not open Evaluate Expression");
    }
    click(cx, "evaluate field: Expression");
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input(expression);
    // The answer before is set aside, so only this expression's answer is read.
    main.update(cx, |main, cx| {
        if let Some(dialog) = main.evaluate_dialog() {
            dialog.update(cx, |dialog, cx| dialog.set_result(String::new(), false, cx));
        }
    });
    cx.simulate_keystrokes("enter");
    let result = |cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| {
            main.evaluate_dialog().and_then(|dialog| dialog.read(cx).result().map(str::to_string))
        })
        .filter(|result| !result.is_empty())
    };
    assert!(pump(cx, |cx| result(cx).is_some()), "Evaluate Expression never answered {expression}");
    result(cx).unwrap()
}

#[test]
fn an_evaluation_that_could_not_be_stopped_leaves_the_program_paused_and_a_step_goes_on_once_it_returns() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-unstoppable", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    // A getter that reads input the program has not been given: an evaluation of it cannot be stopped, and runs on.
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        Counted counted = new Counted();\n        int total = 0;\n        for (int i = 1; i <= 3; i++)\n        {\n            total += i;\n        }\n        Console.WriteLine($\"total {total} {counted.Count}\");\n    }\n}\n\nstatic class Waits\n{\n    public static string Input => Console.ReadLine() ?? \"none\";\n}\n\nsealed class Counted\n{\n    public int Count => 1;\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((10, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    let value = |main: &Entity<MainWindow>, cx: &mut VisualTestContext, name: &str| {
        main.read_with(cx, |main, cx| main.debug_view().read(cx).variable_value(name))
    };
    assert!(pump(cx, |cx| value(&main, cx, "i").as_deref() == Some("1")), "the program did not stop in the loop");

    let said = evaluate(&main, cx, "Waits.Input");
    assert!(said.contains("could not be stopped"), "the debugger did not give up on the evaluation: {said:?}");
    assert_eq!(main.read_with(cx, |main, _| main.debug_phase()), Phase::Paused, "the program was left running");
    let marked =
        main.read_with(cx, |_, cx| rsharp_app::buffer::find(&source, cx).and_then(|b| b.read(cx).execution_line));
    assert_eq!(marked, Some(10), "the editor no longer marks the stop");
    assert!(!main.read_with(cx, |main, _| main.debug_frames().is_empty()), "the stop's frames went");
    assert_eq!(value(&main, cx, "i").as_deref(), Some("1"), "the evaluation's frames were taken for the stop's");
    // No getter runs while it does, though what needs no code run in the program evaluates.
    open_row(&main, cx, "counted", "Count");
    let count = value(&main, cx, "Count").unwrap_or_default();
    assert!(
        count.starts_with("Implicit evaluation is disabled"),
        "a getter ran while the evaluation still ran: {count:?}"
    );
    let sum = evaluate(&main, cx, "i + 1");
    assert!(sum.starts_with('2'), "i + 1 did not evaluate: {sum:?}");
    cx.simulate_keystrokes("escape");

    // The step is taken, and ends at the loop body's closing brace once the read the evaluation waits on has its line.
    cx.simulate_keystrokes("f10");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Running)),
        "the step was refused: {:?}",
        main.read_with(cx, |main, _| main.said().map(str::to_string))
    );
    main.update_in(cx, |main, _, cx| main.debug_input("typed\n", cx));
    let stepped = pump(cx, |cx| {
        main.read_with(cx, |main, _| {
            main.debug_phase() == Phase::Paused && main.debug_frames().first().is_some_and(|frame| frame.line == 11)
        })
    });
    assert!(
        stepped,
        "the step did not end at the next line: {:?}",
        main.read_with(cx, |m, _| m.said().map(str::to_string))
    );
    assert!(pump(cx, |cx| value(&main, cx, "total").as_deref() == Some("1")), "the stop's locals");
    // The program has gone on since, so getters run again.
    open_row(&main, cx, "counted", "Count");
    assert_eq!(value(&main, cx, "Count").as_deref(), Some("1"), "the getter did not run once the program went on");
    cx.simulate_keystrokes("shift-f5");
}

#[test]
fn an_exception_the_debugger_cannot_describe_still_shows_where_the_program_stopped() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-undescribed", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    // Its Source takes longer than the debugger waits for code it runs, so the description of it, which reads Source, is
    // given up; the program is stopped again once Source returns, its frames there to read.
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\nusing System.Threading;\n\nclass Program\n{\n    static void Main()\n    {\n        int index = 5;\n        throw new Slow();\n    }\n}\n\nsealed class Slow : Exception\n{\n    public override string Source { get { Thread.Sleep(8000); return \"late\"; } set { } }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, None, window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| !main.debug_frames().is_empty())),
        "the stop's frames never came, though its exception is not described: {:?}",
        main.read_with(cx, |main, _| main.said().map(str::to_string))
    );
    assert_eq!(main.read_with(cx, |main, _| main.debug_frames()[0].line), 8, "the stop is not at the throw");
    let marked =
        main.read_with(cx, |_, cx| rsharp_app::buffer::find(&source, cx).and_then(|b| b.read(cx).execution_line));
    assert_eq!(marked, Some(8), "the editor does not mark the throw");
    assert!(
        pump(cx, |cx| main
            .read_with(cx, |main, cx| main.debug_view().read(cx).variable_value("index").as_deref() == Some("5"))),
        "the stop's locals are not shown"
    );
    assert!(
        pump(cx, |cx| main
            .read_with(cx, |main, _| main.said().is_some_and(|said| said.contains("could not describe")))),
        "the status bar does not say why the exception is not named: {:?}",
        main.read_with(cx, |main, _| main.said().map(str::to_string))
    );
    cx.simulate_keystrokes("shift-f5");
}

#[test]
fn closing_the_window_while_attached_lets_go_of_the_program_which_runs_on() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-attach-close", &[]);
    let Some((mut program, _)) = running_program(&run) else { return };
    let id = program.id();
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    cx.simulate_keystrokes("secondary-shift-a");
    cx.simulate_input("attach to process");
    cx.simulate_keystrokes("enter");
    assert!(pump(cx, |cx| cx.debug_bounds("attach dialog").is_some()), "Find Action did not open Attach to Process");
    click(cx, "attach field: Search processes");
    cx.simulate_input(&id.to_string());
    assert!(pump(cx, |cx| cx.debug_bounds("attach row: AttachApp").is_some()), "the running program is not listed");
    click(cx, "attach row: AttachApp");
    click(cx, "button: Attach with .NET Debugger");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Running)),
        "the debugger did not attach: {:?}",
        main.read_with(cx, |main, _| main.said().map(str::to_string))
    );
    let adapter = main.read_with(cx, |main, _| main.debug_adapter_process()).expect("the debugger's process");

    assert!(cx.simulate_close(), "the window closes as a person closes it");
    assert!(adapter_gone(adapter), "after the window closed, the debugger went on running");
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        program.process.try_wait().ok().flatten().is_none(),
        "closing the window ended a program it had only attached to"
    );
    program.kill();
}

/// Whether a process has ended within a minute, by its id.
fn adapter_gone(id: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut system = sysinfo_like_running(id);
    while system && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        system = sysinfo_like_running(id);
    }
    !system
}

/// Whether a process with this id runs, as the system's own tools tell.
fn sysinfo_like_running(id: u32) -> bool {
    let output = if cfg!(windows) {
        std::process::Command::new("tasklist").args(["/FI", &format!("PID eq {id}"), "/NH"]).output()
    } else {
        std::process::Command::new("kill").args(["-0", &id.to_string()]).output()
    };
    match output {
        Ok(output) if cfg!(windows) => String::from_utf8_lossy(&output.stdout).contains(&id.to_string()),
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

#[test]
fn a_configuration_is_run_and_debugged_with_the_same_arguments_variable_and_working_folder() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-parity", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static void Main(string[] args)\n    {\n        Console.WriteLine($\"seen [{string.Join(\"|\", args)}] {Environment.GetEnvironmentVariable(\"RSHARP_SEEN\")} in {Environment.CurrentDirectory}\");\n    }\n}\n",
    );
    write(
        &run.folder,
        "App/Properties/launchSettings.json",
        r#"{ "profiles": { "Seen": { "commandName": "Project", "commandLineArgs": "first \"second word\"", "workingDirectory": "work dir", "environmentVariables": { "RSHARP_SEEN": "yes, seen" } } } }"#,
    );
    write(&run.folder, "App/work dir/.keep", "");
    if !offline(&run) {
        return;
    }
    let work = rsharp_core::paths::canonical(&run.folder.join("App/work dir"));
    let seen = format!("seen [first|second word] yes, seen in {}", work.display());
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    let chosen = main.read_with(cx, |main, _| main.chosen_run_configuration().map(|c| c.name.clone()));
    assert!(
        chosen.as_deref().is_some_and(|name| name.contains("Seen")),
        "the launch profile is not chosen: {chosen:?}"
    );

    cx.simulate_keystrokes("ctrl-f5");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| !main.is_running() && main.run_outcome().is_some())),
        "the run did not end"
    );
    let ran = main.read_with(cx, |main, cx| main.run_output(cx));
    assert!(ran.contains(&seen), "Run did not give the program what Debug gives it, {seen}:\n{ran}");

    cx.simulate_keystrokes("f5");
    assert!(run_to_end(&main, cx) || true, "the debugged program did not end");
    let debugged = output(&main, cx);
    assert!(debugged.contains(&seen), "Debug did not give the program what Run gives it, {seen}:\n{debugged}");
    // shortcut: Passenger also profiles it with the same values; RSharp's profiler is Phase 11, when this checks Profile too.
}

/// TerminalConsoleApp's own project, which needs a real terminal: top-level statements and implicit usings.
const TERMINAL_PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework><ImplicitUsings>enable</ImplicitUsings><Nullable>enable</Nullable></PropertyGroup></Project>";

/// A folder with Passenger's TerminalConsoleApp and its configuration "App in the terminal", run with these arguments in
/// a Terminal tab; none when the machine cannot debug it.
fn terminal_app(run: &Run, arguments: &str) -> Option<PathBuf> {
    let app = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/passenger/tests/Debuggees/TerminalConsoleApp/Program.cs");
    let Ok(source) = std::fs::read_to_string(&app) else {
        eprintln!("skipped: vendor/passenger is not checked out");
        return None;
    };
    write(&run.folder, "App/App.csproj", TERMINAL_PROJECT);
    write(&run.folder, "App/Program.cs", &source);
    if !offline(run) {
        return None;
    }
    let store = rsharp_core::state::StateStore::open(run.state.clone());
    store.write_for(
        &run.folder,
        &rsharp_core::run::configuration::RunConfigurations {
            configurations: vec![rsharp_core::run::RunConfiguration {
                name: "App in the terminal".into(),
                project: run.folder.join("App/App.csproj"),
                arguments: arguments.into(),
                use_terminal: true,
                ..rsharp_core::run::RunConfiguration::default()
            }],
            selected: Some("App in the terminal".into()),
        },
    );
    store.flush();
    Some(rsharp_core::paths::canonical(&run.folder.join("App/Program.cs")))
}

/// The program's Terminal tab as it reads, and how it ended.
fn terminal_text(main: &Entity<MainWindow>, cx: &mut VisualTestContext, id: usize) -> (String, Option<u32>) {
    main.read_with(cx, |main, cx| {
        main.terminal().read(cx).session(id).map(|session| (session.text(), session.exit_code())).unwrap_or_default()
    })
}

#[test]
fn debugged_in_the_terminal_a_program_stops_at_a_breakpoint_on_mains_first_line_and_reads_what_is_typed_there() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-terminal-first-line", &[]);
    let Some(source) = terminal_app(&run, "") else { return };
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((0, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    let stopped = pump(cx, |cx| {
        main.read_with(cx, |main, _| {
            main.debug_phase() == Phase::Paused || main.said().is_some_and(|s| s.contains("exited"))
        })
    });
    assert!(
        stopped && main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused),
        "debugged in the terminal, the program ended without stopping at the breakpoint on Main's first line: {:?}",
        main.read_with(cx, |main, _| main.said().map(str::to_string))
    );
    assert_eq!(
        main.read_with(cx, |main, _| main.debug_frames()[0].line),
        0,
        "the program stopped past Main's first line"
    );
    let id = main.read_with(cx, |main, _| main.debug_terminal()).expect("the program runs in a Terminal tab");
    assert!(!terminal_text(&main, cx, id).0.contains("started"), "the program ran before the debugger had hold of it");

    // Let go, it reads what is typed into its tab, and ends with its own exit code.
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |cx| terminal_text(&main, cx, id).0.contains("key?")),
        "the program did not ask for a key in its tab"
    );
    main.update_in(cx, |main, window, cx| {
        let view = main.terminal().read(cx).view().clone();
        window.focus(&view.focus_handle(cx));
    });
    cx.run_until_parked();
    cx.simulate_input("q");
    cx.simulate_input("hello");
    cx.simulate_keystrokes("enter");
    assert!(
        pump(cx, |cx| !main.read_with(cx, |main, _| main.is_debugging()) && terminal_text(&main, cx, id).1.is_some()),
        "the program did not end: {:?}",
        terminal_text(&main, cx, id)
    );
    let (shown, code) = terminal_text(&main, cx, id);
    assert!(shown.contains("key q") && shown.contains("line hello"), "the program's tab shows:\n{shown}");
    assert_eq!(code, Some(3), "the tab does not carry the program's own exit code");
    let said = main.read_with(cx, |main, _| main.said().map(str::to_string)).unwrap_or_default();
    // The adapter says the exit itself where it can read it (Windows), else the tab does: either way, the code is said.
    assert!(
        said.contains("exit code 3") || said.contains("exited with code 3"),
        "the status bar does not say how it ended: {said:?}"
    );
}

#[cfg(unix)]
#[test]
fn stop_ends_a_program_debugged_in_the_terminal_and_what_it_started() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-terminal-stop", &[]);
    let Some(_) = terminal_app(&run, "child") else { return };
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    cx.simulate_keystrokes("f5");
    let asked = pump(cx, |cx| {
        main.read_with(cx, |main, _| main.debug_terminal())
            .is_some_and(|id| terminal_text(&main, cx, id).0.contains("key?"))
    });
    assert!(asked, "the program did not start its child and ask for a key: {:?}", output(&main, cx));
    let id = main.read_with(cx, |main, _| main.debug_terminal()).unwrap();
    let (shown, _) = terminal_text(&main, cx, id);
    let child: u32 = shown
        .lines()
        .find_map(|line| line.trim().strip_prefix("child ").and_then(|id| id.trim().parse().ok()))
        .unwrap_or_else(|| panic!("the program did not say its child's id:\n{shown}"));
    let program = main.read_with(cx, |main, cx| main.terminal().read(cx).session(id).map(|s| s.pid)).unwrap();
    assert!(process_running(child), "the program's child is not running");

    cx.simulate_keystrokes("shift-f5");
    assert!(
        pump(cx, |cx| !main.read_with(cx, |main, _| main.is_debugging()) && terminal_text(&main, cx, id).1.is_some()),
        "Stop did not end the session and its tab's program"
    );
    assert!(gone(program), "Stop left the program running");
    assert!(gone(child), "Stop left the program's child running");
    assert!(main.read_with(cx, |main, cx| main.terminal().read(cx).has_tab(id)), "the program's tab went with it");
}

/// Passenger's StartHold: the program's runtime held at its start by a diagnostic port the test owns, so the debugger
/// waits for it as it waits for one slow to start.
// shortcut: a Unix socket only; the DAP suite holds a Windows runtime by named pipe, and this window case can take that
// helper when the window's Windows run needs it.
#[cfg(unix)]
struct StartHold {
    path: PathBuf,
    listener: std::os::unix::net::UnixListener,
}

#[cfg(unix)]
impl StartHold {
    fn new() -> Option<Self> {
        let path = std::env::temp_dir().join(format!("rsharp-whold-{}", std::process::id()));
        if path.as_os_str().len() > 100 {
            return None;
        }
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).ok()?;
        Some(Self { path, listener })
    }

    /// The held runtime's process id once it has connected, its connection kept open so it never goes on.
    fn held(self) -> std::sync::mpsc::Receiver<u32> {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::Read;
            let Ok((mut stream, _)) = self.listener.accept() else { return };
            // "ADVR_V1\0", the cookie, its process's id and two bytes kept for later.
            let mut advertised = [0u8; 34];
            if stream.read_exact(&mut advertised).is_ok() {
                let _ = sender.send(u64::from_le_bytes(advertised[24..32].try_into().unwrap()) as u32);
            }
            let mut rest = [0u8; 1];
            let _ = stream.read(&mut rest);
            let _ = std::fs::remove_file(&self.path);
        });
        receiver
    }
}

#[cfg(unix)]
#[test]
fn stop_ends_a_debug_session_whose_program_is_still_starting_and_the_program_with_it() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let Some(hold) = StartHold::new() else {
        eprintln!("skipped: the temp folder's path leaves no room for a diagnostic socket");
        return;
    };
    let run = Run::new("debugger-held-start", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", PROGRAM);
    if !offline(&run) {
        return;
    }
    let store = rsharp_core::state::StateStore::open(run.state.clone());
    store.write_for(
        &run.folder,
        &rsharp_core::run::configuration::RunConfigurations {
            configurations: vec![rsharp_core::run::RunConfiguration {
                name: "App held".into(),
                project: run.folder.join("App/App.csproj"),
                environment: std::collections::BTreeMap::from([(
                    "DOTNET_DiagnosticPorts".to_string(),
                    format!("{},connect,suspend", hold.path.display()),
                )]),
                ..rsharp_core::run::RunConfiguration::default()
            }],
            selected: Some("App held".into()),
        },
    );
    store.flush();
    let held = hold.held();
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    cx.simulate_keystrokes("f5");
    let program = std::cell::Cell::new(None);
    assert!(
        pump(cx, |_| {
            if program.get().is_none() {
                program.set(held.try_recv().ok());
            }
            program.get().is_some()
        }),
        "the build, then the program's runtime never came to the hold: {:?}",
        main.read_with(cx, |main, _| main.said().map(str::to_string))
    );
    let program = program.get().unwrap();
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| main.debug_adapter_process().is_some())),
        "the debugger waited for the program to start with no session for Stop to end"
    );
    let adapter = main.read_with(cx, |main, _| main.debug_adapter_process()).unwrap();
    assert_eq!(main.read_with(cx, |main, _| main.debug_phase()), Phase::Starting, "the session is not starting");
    assert!(cx.debug_bounds("debug: Stop").is_some(), "the Debug window has no Stop while the program starts");

    click(cx, "debug: Stop");
    assert!(
        pump(cx, |cx| !main.read_with(cx, |main, _| main.is_debugging()) && !process_running(program)),
        "Stop did not end the starting session and its program"
    );
    assert!(gone(adapter), "Stop left the debugger running");
    let said = main.read_with(cx, |main, _| main.said().map(str::to_string)).unwrap_or_default();
    assert_eq!(said, "Debugging App held was stopped as it started", "the status bar does not say it was stopped");
    assert!(
        output(&main, cx).contains("Debugging App held was stopped as it started"),
        "the Debug console does not say so"
    );
    // Nothing the session said once it was let go stands: the window stays as Stop left it.
    for _ in 0..10 {
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!main.read_with(cx, |main, _| main.is_debugging()), "the stopped start came back as a session");
}

/// Passenger's test of a program the debugger cannot end: stopped where it stands, it answers neither Stop nor the
/// window's close, which end it and the debugger once their limit has passed, leaving no request waiting on a window
/// that is gone.
#[cfg(unix)]
#[test]
fn stop_and_the_windows_close_on_a_program_the_debugger_cannot_end_leave_nothing_running() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-unendable", &[]);
    let started = run.folder.join("program.pid");
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        &format!(
            "using System;\nusing System.IO;\nusing System.Threading;\n\nclass Program\n{{\n    static void Main()\n    {{\n        File.WriteAllText(@\"{}\", Environment.ProcessId.ToString());\n        Thread.Sleep(Timeout.Infinite);\n    }}\n}}\n",
            started.display()
        ),
    );
    if !offline(&run) {
        return;
    }
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |_| std::fs::read_to_string(&started).is_ok_and(|id| !id.is_empty())),
        "the program did not start"
    );
    let program: u32 = std::fs::read_to_string(&started).unwrap().trim().parse().expect("the program wrote its id");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Running)),
        "the session does not say the program runs"
    );
    let adapter = main.read_with(cx, |main, _| main.debug_adapter_process()).expect("the debugger's process");
    let _ = std::process::Command::new("kill").args(["-STOP", &program.to_string()]).status();

    cx.simulate_keystrokes("shift-f5");
    assert!(!main.read_with(cx, |main, _| main.is_debugging()), "Stop left the session going");
    assert!(cx.simulate_close(), "the window closes as a person closes it");
    assert!(gone(adapter), "after the window closed, the debugger went on running");
    assert!(gone(program), "after the window closed, the program being debugged went on running");
    let log = std::fs::read_to_string(run.state.join("logs").join("rsharp.log")).unwrap_or_default();
    assert!(!log.contains("panicked"), "a request to the debugger failed once the window was gone:\n{log}");
}

#[test]
fn a_value_from_the_stop_before_a_step_opens_to_an_error_never_to_another_values_members() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-stale-value", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nstatic class Program\n{\n    static void Main()\n    {\n        Inner inner = new Inner();\n        Visited visited = new Visited();\n        Visit(visited);\n        Console.WriteLine(Console.ReadLine());\n    }\n\n    static void Visit(Visited visited)\n    {\n        Console.WriteLine(visited.VisitedField);\n    }\n}\n\nsealed class Inner\n{\n    public int InnerField = 1;\n}\n\nsealed class Visited\n{\n    public string VisitedField = \"v\";\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((8, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    let names = |main: &Entity<MainWindow>, cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| main.debug_view().read(cx).variable_names())
    };
    assert!(pump(cx, |cx| names(&main, cx).iter().any(|n| n == "inner")), "the program did not stop before the call");
    // The row as the last stop showed it, its click held past the step as one in flight would be.
    let stale = main.read_with(cx, |main, cx| main.debug_view().read(cx).open_request("inner")).expect("inner opens");

    cx.simulate_keystrokes("f11");
    assert!(
        pump(cx, |cx| {
            let shown = names(&main, cx);
            !shown.iter().any(|n| n == "inner") && shown.iter().any(|n| n == "visited")
        }),
        "the step did not stop in the method called"
    );
    main.update(cx, |main, cx| main.debug_view().update(cx, |_, cx| cx.emit(stale)));
    for _ in 0..20 {
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(20));
    }
    let shown = names(&main, cx);
    assert!(
        !shown.iter().any(|n| n == "VisitedField" || n == "InnerField"),
        "a value of the stop before opened to another value's members: {shown:?}"
    );
    let status = main.read_with(cx, |main, cx| main.debug_view().read(cx).status().to_string());
    assert!(
        status.contains("earlier stop"),
        "the Debug window does not say the value is gone with its stop: {status:?}"
    );

    // Run on to a read of input, the Debug window lets the stop's frames and values go, as Rider's does.
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, cx| main.debug_phase() == Phase::Running
            && main.debug_view().read(cx).frames().is_empty()
            && main.debug_view().read(cx).variable_names().is_empty())),
        "the last stop's frames and values were not cleared while the program runs"
    );
    cx.simulate_keystrokes("shift-f5");
}

/// Where a line's breakpoint dot and execution arrow are, in the window.
fn gutter(main: &Entity<MainWindow>, cx: &mut VisualTestContext, line: usize) -> gpui::Point<gpui::Pixels> {
    cx.run_until_parked();
    main.read_with(cx, |main, cx| main.editor().and_then(|editor| editor.read(cx).gutter_point(line)))
        .unwrap_or_else(|| panic!("line {line} is not on screen"))
}

/// The breakpoint popup's focused field, if the popup is open.
fn popup_field(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> Option<rsharp_app::breakpoint_popup::Field> {
    main.update_in(cx, |main, window, cx| main.breakpoint_popup().and_then(|popup| popup.read(cx).focused(window, cx)))
}

#[test]
fn dragging_the_execution_arrow_past_the_loop_goes_on_from_there_and_skips_it() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-arrow-drag", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", LOOP_PROGRAM);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((9, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |cx| main
            .read_with(cx, |main, cx| main.debug_view().read(cx).variable_value("i").as_deref() == Some("1"))),
        "the program did not stop in the loop: {:?}",
        output(&main, cx)
    );

    // From the arrow beside `total += i;` to the Console.WriteLine after the loop.
    let (from, to) = (gutter(&main, cx, 9), gutter(&main, cx, 11));
    cx.simulate_mouse_down(from, gpui::MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(to, Some(gpui::MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(to, gpui::MouseButton::Left, Modifiers::default());
    let marked = |cx: &mut VisualTestContext| {
        main.read_with(cx, |_, cx| rsharp_app::buffer::find(&source, cx).and_then(|b| b.read(cx).execution_line))
    };
    assert!(
        pump(cx, |cx| marked(cx) == Some(11)),
        "the arrow does not stand at the line after the loop: {:?}",
        marked(cx)
    );
    assert_eq!(
        main.read_with(cx, |main, _| main.breakpoints().lines(&source)),
        vec![9],
        "the drag changed the breakpoints"
    );

    cx.simulate_keystrokes("f5");
    assert!(pump(cx, |cx| output(&main, cx).contains("total ")), "the program did not go on");
    assert!(output(&main, cx).contains("total 0"), "the loop was not skipped: {:?}", output(&main, cx));
    assert!(pump(cx, |cx| !main.read_with(cx, |main, _| main.is_debugging())), "the program did not end");
}

#[test]
fn a_condition_typed_into_a_breakpoints_popup_decides_where_it_stops() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-popup-condition", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", LOOP_PROGRAM);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((9, 12)), window, cx));
    cx.run_until_parked();
    // Alt+F9 on a line with no breakpoint sets one and opens its properties, its condition ready to type.
    cx.simulate_keystrokes("alt-f9");
    assert!(cx.debug_bounds("breakpoint popup").is_some(), "Alt+F9 did not open the breakpoint's popup");
    assert_eq!(
        popup_field(&main, cx),
        Some(rsharp_app::breakpoint_popup::Field::Condition),
        "the condition is not ready"
    );
    cx.simulate_input("i == 2");
    cx.simulate_keystrokes("enter");
    assert!(main.read_with(cx, |main, _| main.breakpoint_popup().is_none()), "Enter did not close the popup");
    let breakpoints = main.read_with(cx, |main, _| main.breakpoints().of(&source));
    assert_eq!(breakpoints.len(), 1, "Alt+F9 did not set one breakpoint: {breakpoints:?}");
    assert_eq!(breakpoints[0].condition, "i == 2", "the condition typed is not the breakpoint's");

    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, cx| main.debug_phase() == Phase::Paused
            && main.debug_view().read(cx).variable_value("i").is_some())),
        "the program did not stop where the condition allows: {:?}",
        output(&main, cx)
    );
    assert_eq!(
        main.read_with(cx, |main, cx| main.debug_view().read(cx).variable_value("i")),
        Some("2".into()),
        "the condition did not decide where it stopped"
    );
    cx.simulate_keystrokes("shift-f5");
}

#[test]
fn a_tracepoint_set_with_shift_click_logs_its_message_and_the_program_runs_on() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-tracepoint", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", LOOP_PROGRAM);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, None, window, cx));
    cx.run_until_parked();
    let at = gutter(&main, cx, 9);
    let shift = Modifiers { shift: true, ..Modifiers::default() };
    cx.simulate_mouse_down(at, gpui::MouseButton::Left, shift);
    cx.simulate_mouse_up(at, gpui::MouseButton::Left, shift);
    cx.run_until_parked();
    let set = main.read_with(cx, |main, _| main.breakpoints().of(&source));
    assert!(set.len() == 1 && !set[0].log_message.is_empty(), "Shift+click did not set a tracepoint: {set:?}");

    // A right-click on it opens its properties; the log message is two fields on from the condition.
    cx.simulate_mouse_down(at, gpui::MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(at, gpui::MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("breakpoint popup").is_some(), "a right-click did not open the tracepoint's popup");
    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("tab");
    assert_eq!(popup_field(&main, cx), Some(rsharp_app::breakpoint_popup::Field::LogMessage), "Tab did not reach it");
    cx.simulate_input("i is {i}, total {total}");
    cx.simulate_keystrokes("enter");
    let logged = main.read_with(cx, |main, _| main.breakpoints().of(&source)[0].log_message.clone());
    assert_eq!(logged, "i is {i}, total {total}", "the message typed is not the tracepoint's");

    cx.simulate_keystrokes("f5");
    assert!(!run_to_end(&main, cx), "the tracepoint stopped the program");
    let console = output(&main, cx);
    for line in ["i is 1, total 0", "i is 2, total 1", "i is 3, total 3", "total 6"] {
        assert!(console.contains(line), "the console lacks {line:?}:\n{console}");
    }
}

#[test]
fn a_string_of_megabytes_shows_its_start_in_variables_and_a_hover_without_holding_the_window() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-megabytes", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        string big = new string('x', 4_000_000);\n        int n = big.Length;\n        Console.WriteLine(n);\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((8, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    let big = |main: &Entity<MainWindow>, cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| main.debug_view().read(cx).variable_value("big"))
    };
    assert!(pump(cx, |cx| big(&main, cx).is_some()), "the string's row never came: {:?}", output(&main, cx));
    let drawn = Instant::now();
    cx.run_until_parked();
    assert!(cx.debug_bounds("debug variable big").is_some(), "Variables does not draw the string's row");
    assert!(
        drawn.elapsed() < Duration::from_secs(5),
        "drawing the row took {:?}: the string was laid out whole",
        drawn.elapsed()
    );
    assert_eq!(big(&main, cx).map(|value| value.len()), Some(4_000_002), "the row does not keep the whole value");
    right_click(cx, "debug variable big");
    click(cx, "debug menu: Copy Value");
    let copied = cx.update(|_, cx| cx.read_from_clipboard()).and_then(|item| item.text()).unwrap_or_default();
    assert_eq!(copied.len(), 4_000_002, "Copy Value did not copy all of the string");

    let offset =
        main.read_with(cx, |main, cx| main.editor().unwrap().read(cx).text().to_string()).find("big.Length").unwrap()
            + 1;
    let at = main
        .read_with(cx, |main, cx| main.editor().and_then(|editor| editor.read(cx).text_point(offset)))
        .expect("the hovered word is on screen");
    cx.simulate_mouse_move(at, None, Modifiers::default());
    let card = |cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| main.editor().and_then(|editor| editor.read(cx).card_text().map(str::to_string)))
    };
    assert!(
        pump(cx, |cx| card(cx).is_some_and(|card| card.starts_with("big ="))),
        "no hover card on big: {:?}",
        card(cx)
    );
    let card = card(cx).unwrap();
    assert!(
        card.starts_with("big = {string} \"xxx") && card.len() < 2000,
        "the hover card says {} characters, beginning {:?}",
        card.len(),
        &card[..card.len().min(40)]
    );
    cx.simulate_keystrokes("shift-f5");
}

#[test]
fn a_line_typed_above_a_bound_breakpoint_while_paused_leaves_it_stopping_at_its_statement() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    // Hot Reload carries the moved lines into the program as it goes on, which takes Roslyn's Edit and Continue.
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let run = Run::new("debugger-live-follow", &[]);
    write(&run.folder, "App.csproj", EXE_PROJECT);
    write(&run.folder, "Program.cs", LOOP_PROGRAM);
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = run.window(&mut cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let folder = run.folder.clone();
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&folder, window, cx);
        main.open_file(&source, Some((9, 0)), window, cx);
    });
    let loaded = pump(cx, |cx| main.read_with(cx, |main, _| main.server_ready() || main.server_failed().is_some()));
    assert!(loaded, "Roslyn did not load the project: {:?}", main.read_with(cx, |main, _| main.server_status()));
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    let value = |main: &Entity<MainWindow>, cx: &mut VisualTestContext, name: &str| {
        main.read_with(cx, |main, cx| main.debug_view().read(cx).variable_value(name))
    };
    assert!(pump(cx, |cx| value(&main, cx, "i").as_deref() == Some("1")), "the program did not make its first stop");
    let marks = |main: &Entity<MainWindow>, cx: &mut VisualTestContext| {
        main.read_with(cx, |_, cx| {
            rsharp_app::buffer::find(&source, cx)
                .map(|b| (b.read(cx).execution_line, b.read(cx).inline_values.as_ref().map(|(line, _)| *line)))
                .unwrap_or_default()
        })
    };

    // A line broken at the top while paused: the breakpoint, the arrow and the values go down a line with their code.
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((0, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        main.read_with(cx, |main, _| main.breakpoints().lines(&source)),
        vec![10],
        "the breakpoint left its code"
    );
    assert_eq!(marks(&main, cx), (Some(10), Some(10)), "the execution point and its values left their code");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| main.breakpoint_verified(&source, 10) == Some(true))),
        "the moved breakpoint is no longer bound to its statement"
    );

    // The new line has no code in the program being debugged: a breakpoint there is refused, not bound to the next.
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((0, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| main.breakpoint_verified(&source, 0) == Some(false))),
        "the new line's breakpoint was not refused"
    );
    let said = main.read_with(cx, |main, _| main.breakpoint_message(&source, 0)).unwrap_or_default();
    assert!(
        said.contains("written after the program being debugged was built"),
        "the refusal does not say why: {said:?}"
    );

    // Resumed, the program stops on the loop's next turn at the statement the breakpoint was bound to, shown where it
    // now is: Hot Reload applies the moved lines as it goes on.
    cx.simulate_keystrokes("f5");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused)
            && value(&main, cx, "i").as_deref() == Some("2")),
        "the loop's next turn did not stop at the breakpoint's statement: {:?}; {:?}",
        main.read_with(cx, |main, _| main.said().map(str::to_string)),
        output(&main, cx)
    );
    assert_eq!(main.read_with(cx, |main, _| main.debug_frames()[0].line), 10, "the debugger names another line");
    assert_eq!(marks(&main, cx).0, Some(10), "the execution point is not where the statement now is");
    cx.simulate_keystrokes("shift-f5");
}

/// The hover card on a word, once the debugger has answered for it; none while none is shown.
fn hover(main: &Entity<MainWindow>, cx: &mut VisualTestContext, word: &str) -> Option<String> {
    let offset = main.read_with(cx, |main, cx| main.editor().unwrap().read(cx).text().to_string()).find(word)? + 1;
    let at = main.read_with(cx, |main, cx| main.editor().and_then(|editor| editor.read(cx).text_point(offset)))?;
    // Away first, so a rest on the word is a new one.
    cx.simulate_mouse_move(gpui::point(gpui::px(1.), gpui::px(1.)), None, Modifiers::default());
    cx.run_until_parked();
    cx.simulate_mouse_move(at, None, Modifiers::default());
    pump(cx, |cx| card(main, cx).is_some_and(|card| card.contains(" = ")));
    card(main, cx)
}

fn card(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> Option<String> {
    main.read_with(cx, |main, cx| main.editor().and_then(|editor| editor.read(cx).card_text().map(str::to_string)))
}

#[test]
fn a_hover_reads_the_chosen_frame_and_goes_when_the_program_leaves_its_stop() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-hover-frame", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        int value = 41;\n        Show(99);\n        value++;\n        Console.WriteLine(value);\n    }\n\n    static void Show(int value)\n    {\n        Console.WriteLine(value);\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((14, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    let value = |main: &Entity<MainWindow>, cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| main.debug_view().read(cx).variable_value("value"))
    };
    assert!(pump(cx, |cx| value(&main, cx).as_deref() == Some("99")), "the program did not stop in Show");

    // The caller chosen: Variables shows its value, and so does a hover on its word.
    click(cx, "debug frame 1");
    assert!(pump(cx, |cx| value(&main, cx).as_deref() == Some("41")), "choosing Main did not show its values");
    let said = hover(&main, cx, "value = 41").unwrap_or_default();
    assert!(said.starts_with("value = {int} 41"), "the hover read another frame than the one chosen: {said:?}");

    // Stepped on past `value++`, the card read at the stop before goes with it, rather than showing 41 beside 42.
    click(cx, "debug frame 0");
    cx.simulate_keystrokes("shift-f11");
    assert!(
        pump(cx, |cx| main.read_with(cx, |main, _| main.debug_phase() == Phase::Paused
            && main.debug_frames().first().is_some_and(|frame| frame.name.contains("Main")))),
        "Step Out did not come back to Main"
    );
    let said = hover(&main, cx, "value = 41").unwrap_or_default();
    assert!(said.starts_with("value = {int} 41"), "the hover in Main does not read its value: {said:?}");
    // Each step taken once the one before has stopped, until the stop is past `value++`.
    for _ in 0..3 {
        if value(&main, cx).as_deref() == Some("42") {
            break;
        }
        let line = main.read_with(cx, |main, _| main.debug_frames().first().map(|frame| frame.line));
        cx.simulate_keystrokes("f10");
        pump(cx, |cx| {
            main.read_with(cx, |main, _| {
                main.debug_phase() == Phase::Paused && main.debug_frames().first().map(|frame| frame.line) != line
            })
        });
    }
    assert!(pump(cx, |cx| value(&main, cx).as_deref() == Some("42")), "the steps did not run value++");
    let left = card(&main, cx);
    assert!(
        left.as_ref().is_none_or(|card| !card.contains("41")),
        "the card read at an earlier stop still shows its value: {left:?}"
    );
    cx.simulate_keystrokes("shift-f5");
}

#[test]
fn a_watch_reads_the_chosen_frame_and_is_asked_again_at_each_stop() {
    let _alone = alone();
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("debugger-watch-frame", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(
        &run.folder,
        "App/Program.cs",
        "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        int value = 41;\n        Show(99);\n        value++;\n        Console.WriteLine(value);\n    }\n\n    static void Show(int value)\n    {\n        Console.WriteLine(value);\n    }\n}\n",
    );
    if !offline(&run) {
        return;
    }
    let source = rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"));
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((14, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f9");
    cx.simulate_keystrokes("f5");
    let value = |main: &Entity<MainWindow>, cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| main.debug_view().read(cx).variable_value("value"))
    };
    let watched = |main: &Entity<MainWindow>, cx: &mut VisualTestContext, answer: &str| {
        let answer = answer.to_string();
        pump(cx, |cx| {
            main.read_with(cx, |main, cx| {
                main.debug_view()
                    .read(cx)
                    .watches()
                    .iter()
                    .any(|(asked, said)| asked == "value" && said.starts_with(&answer))
            })
        })
    };
    let watches = |main: &Entity<MainWindow>, cx: &mut VisualTestContext| {
        main.read_with(cx, |main, cx| main.debug_view().read(cx).watches().to_vec())
    };
    assert!(pump(cx, |cx| value(&main, cx).as_deref() == Some("99")), "the program did not stop in Show");
    right_click(cx, "debug variable value");
    click(cx, "debug menu: Add to Watches");
    assert!(watched(&main, cx, "99"), "the watch does not read Show's value: {:?}", watches(&main, cx));

    // The caller chosen, the watch is its value; the callee chosen again, the callee's.
    click(cx, "debug frame 1");
    assert!(watched(&main, cx, "41"), "the watch does not follow the frame chosen: {:?}", watches(&main, cx));
    click(cx, "debug frame 0");
    assert!(watched(&main, cx, "99"), "the watch did not come back to the frame chosen: {:?}", watches(&main, cx));

    // Each stop asks it again, so past `value++` it reads what the program now holds.
    cx.simulate_keystrokes("shift-f11");
    assert!(watched(&main, cx, "41"), "the watch was not asked again at the next stop: {:?}", watches(&main, cx));
    for _ in 0..3 {
        if value(&main, cx).as_deref() == Some("42") {
            break;
        }
        let line = main.read_with(cx, |main, _| main.debug_frames().first().map(|frame| frame.line));
        cx.simulate_keystrokes("f10");
        pump(cx, |cx| {
            main.read_with(cx, |main, _| {
                main.debug_phase() == Phase::Paused && main.debug_frames().first().map(|frame| frame.line) != line
            })
        });
    }
    assert!(watched(&main, cx, "42"), "the watch kept an earlier stop's value: {:?}", watches(&main, cx));
    cx.simulate_keystrokes("shift-f5");
}
