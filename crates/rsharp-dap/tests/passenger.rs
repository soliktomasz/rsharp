//! Passenger's own debugger tests, translated: the same `tests/Debuggees` programs, the same questions, asked through
//! this client. The first batch is what Passenger's `PauseTests` asks of `LoopingConsoleApp` — a program that never ends
//! on its own — including the values a pause in the runtime's sleep leaves readable.

use rsharp_dap::{Event, Launch, Session, SourceBreakpoint, Start, adapter};
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
fn started(
    program: &Path,
    arguments: &[&str],
    breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)>,
    at_entry: bool,
) -> (Session, Receiver<Event>) {
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
            stop_at_entry: at_entry,
        }),
        &std::env::temp_dir().join("rsharp-passenger-symbols"),
        breakpoints,
        Vec::new(),
        Vec::new(),
        None,
        move |event| drop(sender.send(event)),
    )
    .expect("the session starts");
    (session, events)
}

fn launched(program: &Path, arguments: &[&str]) -> (Session, Receiver<Event>) {
    started(program, arguments, Vec::new(), true)
}

/// The 1-based line of the stop's top frame, as the debuggee's source counts it.
fn line_at(session: &Session, stop: &rsharp_dap::Stop) -> usize {
    let frames = session.stack_trace(stop.thread).expect("the stack reads");
    let top = frames.first().unwrap_or_else(|| panic!("the stop has a frame: {stop:?}"));
    assert!(
        top.path().is_some_and(|path| path.ends_with("Program.cs")),
        "the top frame is in the debuggee's own source: {top:?}"
    );
    top.line + 1
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

/// Passenger's `SteppingConsoleApp` cases, found by the code on their lines as its tests find them: `Calls.Run`,
/// `Awaits.RunAsync` and `Returns.Run`.
fn stepping_source() -> PathBuf {
    let root = debuggees().expect("the debuggees are there");
    rsharp_core::paths::canonical(&root.join("SteppingConsoleApp/Program.cs"))
}

fn stepping_program() -> Option<PathBuf> {
    if adapter().is_none() || rsharp_core::programs::dotnet().is_none() || debuggees().is_none() {
        eprintln!("skipped: dotnet, the adapter or Passenger's debuggees are not there");
        return None;
    }
    build("SteppingConsoleApp")
}

#[test]
fn stepping_into_a_call_and_out_of_it_comes_back_to_the_statement() {
    let Some(program) = stepping_program() else { return };
    let source = stepping_source();
    let (session, events) = started(
        &program,
        &[],
        vec![(source.clone(), vec![SourceBreakpoint { line: 12, ..SourceBreakpoint::default() }])],
        false,
    );

    let stopped = stop(&events, "breakpoint");
    assert_eq!(line_at(&session, &stopped), 13, "the breakpoint is on Outer(Inner())");
    let thread = stopped.thread;

    session.step_into(thread).expect("a step into is asked");
    let into = stop(&events, "step");
    assert_eq!(line_at(&session, &into), 18, "the step goes into Inner()");

    session.step_out(thread).expect("a step out is asked");
    let out = stop(&events, "step");
    assert_eq!(line_at(&session, &out), 13, "and back to the statement it was called from");

    session.step_over(thread).expect("a step over is asked");
    let over = stop(&events, "step");
    assert_eq!(line_at(&session, &over), 14, "a step over goes to the next statement");
    session.disconnect(true);
}

#[test]
fn a_step_over_an_await_goes_on_after_it() {
    let Some(program) = stepping_program() else { return };
    let source = stepping_source();
    let (session, events) = started(
        &program,
        &[],
        vec![(source.clone(), vec![SourceBreakpoint { line: 50, ..SourceBreakpoint::default() }])],
        false,
    );

    let stopped = stop(&events, "breakpoint");
    assert_eq!(line_at(&session, &stopped), 51, "the breakpoint is on the await");
    session.step_over(stopped.thread).expect("a step over is asked");
    let over = stop(&events, "step");
    assert_eq!(line_at(&session, &over), 52, "the step ends after the await");
    session.disconnect(true);
}

#[test]
fn a_step_over_shows_what_the_call_returned() {
    let Some(program) = stepping_program() else { return };
    let source = stepping_source();
    let (session, events) = started(
        &program,
        &[],
        vec![(source.clone(), vec![SourceBreakpoint { line: 67, ..SourceBreakpoint::default() }])],
        false,
    );

    let stopped = stop(&events, "breakpoint");
    assert_eq!(line_at(&session, &stopped), 68, "the breakpoint is on Describe(Count())");
    session.step_over(stopped.thread).expect("a step over is asked");
    let over = stop(&events, "step");
    let frames = session.stack_trace(over.thread).expect("the stack reads");
    let top = frames.first().expect("the stop has a frame");
    let scope = session.scopes(top.id).expect("the frame's scopes read").into_iter().next().expect("a scope");
    let locals = session.variables(scope.variables_reference).expect("the locals read");
    let returned: Vec<_> = locals.iter().filter(|variable| variable.name.contains("returned")).collect();
    assert!(
        returned.iter().any(|variable| variable.name.contains("Describe") && variable.value.contains("count 42")),
        "what Describe returned is among the locals: {locals:?}"
    );
    assert!(
        returned.iter().any(|variable| variable.name.contains("Count") && variable.value == "42"),
        "and the value the call inside it returned: {locals:?}"
    );
    session.disconnect(true);
}

/// The source line a case is written on, counted from 1, as it reads.
fn source_text(name: &str, line: usize) -> String {
    let root = debuggees().expect("the debuggees are there");
    std::fs::read_to_string(root.join(name).join("Program.cs"))
        .expect("the debuggee's source reads")
        .lines()
        .nth(line - 1)
        .unwrap_or_default()
        .to_string()
}

/// The frame's locals, as Passenger's `host.Locals(stop)` reads them.
fn locals_of(session: &Session, frame: &rsharp_dap::protocol::Frame) -> Vec<rsharp_dap::protocol::Variable> {
    let scope =
        session.scopes(frame.id).expect("the frame's scopes read").into_iter().next().expect("the frame has a scope");
    session.variables(scope.variables_reference).expect("the locals read")
}

#[test]
fn smart_step_into_lists_the_calls_in_the_order_they_run_and_steps_into_the_one_chosen() {
    let Some(program) = stepping_program() else { return };
    let source = stepping_source();
    let (session, events) = started(
        &program,
        &[],
        vec![(source, vec![SourceBreakpoint { line: 12, ..SourceBreakpoint::default() }])],
        false,
    );
    let stopped = stop(&events, "breakpoint");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let targets = session.step_in_targets(frames[0].id).expect("the calls are listed");
    let labels: Vec<&str> = targets.iter().map(|target| target.label.as_str()).collect();
    assert_eq!(labels, ["Inner()", "Outer(int)"], "the calls in the order they run: {targets:?}");

    // Each is placed on its name, as Rider marks the calls on the line.
    let text = source_text("SteppingConsoleApp", 13);
    assert_eq!(targets[0].column, text.find("Inner").expect("Inner is written there"), "the inner call's place");
    assert_eq!(targets[1].column, text.find("Outer").expect("Outer is written there"), "the outer call's place");

    // Straight into Outer, past Inner, which has run by then: Outer is given what it returned.
    session.step_into_target(stopped.thread, targets[1].id).expect("a step into Outer is asked");
    let into = stop(&events, "step");
    assert_eq!(line_at(&session, &into), 21, "Outer's own line");
    let frames = session.stack_trace(into.thread).expect("the stack reads");
    let top = frames.first().expect("the stop has a frame");
    assert!(top.name.contains("Outer"), "the frame is Outer's: {top:?}");
    let locals = locals_of(&session, top);
    assert_eq!(
        locals.iter().find(|variable| variable.name == "value").map(|variable| variable.value.as_str()),
        Some("20"),
        "Outer got what Inner returned: {locals:?}"
    );
    session.disconnect(true);
}

#[test]
fn of_two_calls_of_the_same_method_the_second_chosen_is_the_one_stepped_into() {
    let Some(program) = stepping_program() else { return };
    let source = stepping_source();
    let (session, events) = started(
        &program,
        &[],
        vec![(source, vec![SourceBreakpoint { line: 13, ..SourceBreakpoint::default() }])],
        false,
    );
    let stopped = stop(&events, "breakpoint");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let targets = session.step_in_targets(frames[0].id).expect("the calls are listed");
    let labels: Vec<&str> = targets.iter().map(|target| target.label.as_str()).collect();
    assert_eq!(labels, ["Add(int, int)", "Add(int, int)"], "both calls are Add: {targets:?}");
    assert!(targets[0].column > targets[1].column, "the inner call, which runs first, is listed first: {targets:?}");

    session.step_into_target(stopped.thread, targets[1].id).expect("a step into the outer Add is asked");
    let into = stop(&events, "step");
    let frames = session.stack_trace(into.thread).expect("the stack reads");
    let top = frames.first().expect("the stop has a frame");
    let locals = locals_of(&session, top);
    let value =
        |name: &str| locals.iter().find(|variable| variable.name == name).map(|variable| variable.value.clone());
    assert_eq!(
        (value("a"), value("b")),
        (Some("3".into()), Some("3".into())),
        "the outer call's arguments: {locals:?}"
    );
    assert_eq!(line_at(&session, &into), 26, "Add's own line");
    session.disconnect(true);
}
