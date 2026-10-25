//! Passenger's own debugger tests, translated: the same `tests/Debuggees` programs, the same questions, asked through
//! this client. The first batch is what Passenger's `PauseTests` asks of `LoopingConsoleApp` — a program that never ends
//! on its own — including the values a pause in the runtime's sleep leaves readable.

use rsharp_dap::{Event, Launch, Session, Start, adapter};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// Where Passenger's debuggees are, beside this client's crate.
fn debuggees() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/passenger/tests/Debuggees");
    root.is_dir().then_some(root)
}

/// Builds one debuggee and answers its assembly; skips (None) when dotnet is not there, as Passenger's tests do.
fn build(name: &str) -> Option<PathBuf> {
    let dotnet = rsharp_core::programs::dotnet()?;
    let root = debuggees()?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let output = std::env::temp_dir().join(format!("rsharp-passenger-{}-{stamp}", std::process::id()));
    let status = std::process::Command::new(&dotnet)
        .arg("build")
        .arg(root.join(name).join(format!("{name}.csproj")))
        .args(["-c", "Debug", "-o"])
        .arg(&output)
        .args(["--nologo", "-v", "q", "-p:NuGetAudit=false"])
        .output()
        .expect("dotnet build runs");
    assert!(
        status.status.success(),
        "{name} did not build:\n{}{}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );
    Some(output.join(format!("{name}.dll")))
}

/// A line as the debuggee's source has it, counted from 1, as Passenger's `Debuggees.Line` finds it.
fn line_of(name: &str, needle: &str) -> usize {
    let root = debuggees().expect("the debuggees are there");
    let source = std::fs::read_to_string(root.join(name).join("Program.cs")).expect("the debuggee's source reads");
    source
        .lines()
        .position(|line| line.contains(needle))
        .map(|line| line + 1)
        .unwrap_or_else(|| panic!("{name}'s source has no line with {needle}"))
}

/// The stop the program comes to next, waiting up to a minute; the events it passed are kept for a failure's sake.
fn stop(events: &Receiver<Event>, expected: &str) -> rsharp_dap::Stop {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "the program never stopped with reason {expected}; saw {seen:?}");
        match events.recv_timeout(left) {
            Ok(Event::Stopped(stop)) => {
                assert_eq!(stop.reason, expected, "the stop's reason; saw {seen:?}");
                return stop;
            }
            Ok(Event::Ended) => panic!("the debugger ended before a stop with reason {expected}; saw {seen:?}"),
            Ok(event) => seen.push(format!("{event:?}")),
            Err(error) => panic!("the program never stopped with reason {expected}: {error}; saw {seen:?}"),
        }
    }
}

/// Waits for the program's own word that it is in its loop, as Passenger's launch does before it pauses.
fn wait_for_looping(events: &Receiver<Event>) {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut wrote = String::new();
    while Instant::now() < deadline && !wrote.contains("looping") {
        if let Ok(Event::Output { text, .. }) = events.recv_timeout(Duration::from_millis(200)) {
            wrote.push_str(&text);
        }
    }
    assert!(wrote.contains("looping"), "the program never said it is in its loop: {wrote:?}");
}

/// Starts a debuggee at its entry, so the thread pausing it later is known; Passenger launches without a stop and takes
/// the thread its pause answers with, which this client's pause asks for up front.
fn launched(program: &Path, arguments: &[&str]) -> (Session, Receiver<Event>) {
    let adapter = adapter().expect("Passenger's debug adapter is built");
    let (sender, events) = mpsc::channel();
    let working_directory = program.parent().expect("the assembly is in a folder").to_path_buf();
    let session = Session::start(
        &adapter,
        Start::Launch(Launch {
            program: program.to_path_buf(),
            arguments: arguments.iter().map(|argument| argument.to_string()).collect(),
            working_directory,
            environment: BTreeMap::new(),
            stop_at_entry: true,
        }),
        &std::env::temp_dir().join("rsharp-passenger-symbols"),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        None,
        move |event| drop(sender.send(event)),
    )
    .expect("the session starts");
    (session, events)
}

fn in_loop(session: &Session, stop: &rsharp_dap::Stop, name: &str) {
    // Passenger's own check reads the top frame, as a stopped event names only the thread.
    let frames = session.stack_trace(stop.thread).expect("the stack reads");
    let top = frames.first().unwrap_or_else(|| panic!("the stop has a frame: {stop:?}"));
    let path = top.path().unwrap_or_else(|| panic!("the top frame names a file: {top:?}"));
    assert!(path.ends_with("Program.cs"), "the stop is in the debuggee's own source: {path:?}");
    let line = top.line + 1;
    let first = line_of(name, "turns++").saturating_sub(2);
    let last = line_of(name, "Thread.Sleep(nap)") + 2;
    assert!(
        (first..=last).contains(&line),
        "the stop is in the program's loop, line {line} between {first} and {last}"
    );
}

#[test]
fn a_pause_stops_a_looping_program_in_its_loop_and_a_second_pause_after_continuing_does_too() {
    if adapter().is_none() || rsharp_core::programs::dotnet().is_none() || debuggees().is_none() {
        eprintln!("skipped: dotnet, the adapter or Passenger's debuggees are not there");
        return;
    }
    let Some(program) = build("LoopingConsoleApp") else { return };
    let (session, events) = launched(&program, &[]);
    let entry = stop(&events, "entry");
    let thread = entry.thread;
    session.resume(thread).expect("the program runs on");
    wait_for_looping(&events);

    let paused = session.pause(thread);
    assert!(paused.is_ok(), "the pause is asked of a running program: {paused:?}");
    let stopped = stop(&events, "pause");
    in_loop(&session, &stopped, "LoopingConsoleApp");

    session.resume(thread).expect("the program runs on");
    session.pause(thread).expect("the pause is asked again");
    let stopped = stop(&events, "pause");
    in_loop(&session, &stopped, "LoopingConsoleApp");
    session.disconnect(true);
}

#[test]
fn a_step_from_a_pause_inside_a_sleep_comes_back_to_the_loop() {
    if adapter().is_none() || rsharp_core::programs::dotnet().is_none() || debuggees().is_none() {
        eprintln!("skipped: dotnet, the adapter or Passenger's debuggees are not there");
        return;
    }
    let Some(program) = build("LoopingConsoleApp") else { return };
    let (session, events) = launched(&program, &["sleep"]);
    let entry = stop(&events, "entry");
    let thread = entry.thread;
    session.resume(thread).expect("the program runs on");
    wait_for_looping(&events);
    // Let the loop sleep, so the pause catches its thread in the runtime's own code.
    std::thread::sleep(Duration::from_millis(300));
    session.pause(thread).expect("the pause is asked of a running program");
    let paused = stop(&events, "pause");
    session.step_over(thread).expect("a step over is asked from the pause");
    let stepped = stop(&events, "step");
    in_loop(&session, &stepped, "LoopingConsoleApp");
    assert_ne!(paused.thread, 0);
    session.disconnect(true);
}

#[test]
fn a_value_paused_in_a_sleep_shows_its_type_and_fields_where_its_tostring_cannot_run() {
    if adapter().is_none() || rsharp_core::programs::dotnet().is_none() || debuggees().is_none() {
        eprintln!("skipped: dotnet, the adapter or Passenger's debuggees are not there");
        return;
    }
    let Some(program) = build("LoopingConsoleApp") else { return };
    let (session, events) = launched(&program, &["sleep"]);
    let entry = stop(&events, "entry");
    let thread = entry.thread;
    session.resume(thread).expect("the program runs on");
    wait_for_looping(&events);
    std::thread::sleep(Duration::from_millis(300));
    session.pause(thread).expect("the pause is asked of a running program");
    let stopped = stop(&events, "pause");

    // The frame in the program's own source, as Passenger's test finds it, and its locals.
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let main = frames
        .iter()
        .find(|frame| frame.path().is_some_and(|path| path.ends_with("Program.cs")))
        .unwrap_or_else(|| panic!("a frame is in the program's source: {frames:?}"));
    let scope =
        session.scopes(main.id).expect("the frame's scopes read").into_iter().next().expect("the frame has a scope");
    let locals = session.variables(scope.variables_reference).expect("the locals read");
    let nap =
        locals.iter().find(|variable| variable.name == "nap").unwrap_or_else(|| panic!("nap is a local: {locals:?}"));
    assert!(
        nap.value == "00:00:00.0200000" || nap.value == "{System.TimeSpan}",
        "a TimeSpan read where the pause caught its thread shows as {}",
        nap.value
    );
    let fields = session.variables(nap.variables_reference).expect("the TimeSpan's fields read");
    let ticks =
        fields.iter().find(|field| field.name == "_ticks").unwrap_or_else(|| panic!("_ticks is a field: {fields:?}"));
    assert_eq!(ticks.value, "200000", "the nap's ticks");
    session.disconnect(true);
}
