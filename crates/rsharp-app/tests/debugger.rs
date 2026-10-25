//! Debugging from the window: a line breakpoint set with F9, F5 building and starting the program under the debugger,
//! the Debug window's frames and values, a step with F10, and the program resumed to its end.

mod support;

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
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
    loop {
        let pipe = std::fs::read_dir(std::env::temp_dir()).ok().into_iter().flatten().flatten().find(|entry| {
            entry.file_name().to_string_lossy().starts_with(&format!("clr-debug-pipe-{}-", program.id()))
        });
        if pipe.is_some() {
            return Some((program, rsharp_core::paths::canonical(&run.folder.join("App/Program.cs"))));
        }
        assert!(Instant::now() < deadline, "the program opened no debugger transport within twenty seconds");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn attach_to_a_running_program_stops_at_its_breakpoint_and_detaching_lets_it_run_on() {
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    if cfg!(windows) {
        eprintln!("skipped: Attach to Process lists no programs on Windows yet");
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
