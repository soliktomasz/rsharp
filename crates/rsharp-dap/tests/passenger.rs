//! Passenger's own debugger tests, translated: the same `tests/Debuggees` programs, the same questions, asked through
//! this client. The first batch is what Passenger's `PauseTests` asks of `LoopingConsoleApp` — a program that never ends
//! on its own — including the values a pause in the runtime's sleep leaves readable.

use rsharp_dap::{Event, Launch, Session, SourceBreakpoint, Start, adapter};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// The parity tests run one at a time: each builds a debuggee and debugs a real program, and ten at once starved this
/// machine's SDK until one never stopped.
fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

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
            hot_reload: true,
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

/// Starts a debuggee with exception breakpoints, as a case that stops at a throw asks.
fn started_filtering(
    program: &Path,
    arguments: &[&str],
    breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)>,
    exceptions: Vec<rsharp_dap::ExceptionFilter>,
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
            stop_at_entry: false,
            hot_reload: true,
        }),
        &std::env::temp_dir().join("rsharp-passenger-symbols"),
        breakpoints,
        Vec::new(),
        exceptions,
        None,
        move |event| drop(sender.send(event)),
    )
    .expect("the session starts");
    (session, events)
}

/// One of Passenger's debuggees, built for the case that asks; skipped when the tools or the debuggees are absent.
fn debuggee_program(name: &str) -> Option<PathBuf> {
    if adapter().is_none() || rsharp_core::programs::dotnet().is_none() || debuggees().is_none() {
        eprintln!("skipped: dotnet, the adapter or Passenger's debuggees are not there");
        return None;
    }
    build(name)
}

/// Passenger's `LoopingConsoleApp`, built once per test as its case asks.
fn looping_program() -> Option<PathBuf> {
    if adapter().is_none() || rsharp_core::programs::dotnet().is_none() || debuggees().is_none() {
        eprintln!("skipped: dotnet, the adapter or Passenger's debuggees are not there");
        return None;
    }
    build("LoopingConsoleApp")
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
    let _serial = one_at_a_time();
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
    let _serial = one_at_a_time();
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
    let _serial = one_at_a_time();
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
    let _serial = one_at_a_time();
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
    let _serial = one_at_a_time();
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
    let _serial = one_at_a_time();
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
    let _serial = one_at_a_time();
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
    let _serial = one_at_a_time();
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

/// Passenger's `SyntheticVariablesTests`: the debugger's own names, `$ReturnValue…` and `$exception`.
fn synthetic_line(code: &str) -> usize {
    line_of("SteppingConsoleApp", code)
}

fn evaluate(session: &Session, frame: i64, expression: &str) -> String {
    let answer =
        session.evaluate(expression, frame).unwrap_or_else(|why| panic!("{expression} did not evaluate: {why}"));
    assert!(!answer.result.contains("error"), "{expression} did not evaluate: {}", answer.result);
    answer.result
}

#[test]
fn a_step_over_lists_what_each_call_returned_in_the_order_they_returned_and_names_them_for_an_expression() {
    let _serial = one_at_a_time();
    let Some(program) = stepping_program() else { return };
    let source = stepping_source();
    let line = synthetic_line("string described = Describe(Count());");
    let (session, events) = started(
        &program,
        &[],
        vec![(source, vec![SourceBreakpoint { line: line - 1, ..SourceBreakpoint::default() }])],
        false,
    );

    let stopped = stop(&events, "breakpoint");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let locals = locals_of(&session, frames.first().expect("the stop has a frame"));
    assert!(
        !locals.iter().any(|variable| variable.name.ends_with(" returned")),
        "nothing has returned yet: {locals:?}"
    );

    session.step_over(stopped.thread).expect("a step over is asked");
    let over = stop(&events, "step");
    let frames = session.stack_trace(over.thread).expect("the stack reads");
    let top = frames.first().expect("the stop has a frame");
    let locals = locals_of(&session, top);
    assert!(locals.len() >= 2, "the step's values are listed: {locals:?}");
    let named = |index: usize| {
        let variable = &locals[index];
        (
            variable.name.clone(),
            variable.value.clone(),
            variable.r#type.clone().unwrap_or_default(),
            variable.evaluate_name.clone().unwrap_or_default(),
        )
    };
    assert_eq!(named(0), ("Count() returned".into(), "42".into(), "int".into(), "$ReturnValue1".into()));
    assert_eq!(named(1), ("Describe() returned".into(), "\"count 42\"".into(), "string".into(), "$ReturnValue".into()));
    assert_eq!(
        locals.iter().find(|variable| variable.name == "described").map(|variable| variable.value.as_str()),
        Some("\"count 42\"")
    );
    assert_eq!(evaluate(&session, top.id, "$returnvalue"), "\"count 42\"");
    assert_eq!(evaluate(&session, top.id, "$ReturnValue1"), "42");
    assert_eq!(evaluate(&session, top.id, "$ReturnValue.Length"), "8");
    assert_eq!(evaluate(&session, top.id, "$ReturnValue1 + 1"), "43");

    // The next step's values replace the last's: a double, kept boxed as the int was.
    session.step_over(over.thread).expect("a step over is asked");
    let over = stop(&events, "step");
    let frames = session.stack_trace(over.thread).expect("the stack reads");
    let top = frames.first().expect("the stop has a frame");
    let locals = locals_of(&session, top);
    // The adapter prints a double in the machine's culture, so its decimal mark may be a comma, as this one's is.
    let ratio = named_value(&locals, 0);
    assert_eq!(
        (ratio.0.as_str(), ratio.1.replace(',', ".").as_str(), ratio.2.as_str(), ratio.3.as_str()),
        ("Ratio() returned", "0.75", "double", "$ReturnValue")
    );
    assert!(
        !locals.iter().any(|variable| variable.name == "Count() returned" || variable.name == "Describe() returned")
    );
    assert_eq!(evaluate(&session, top.id, "$ReturnValue * 4"), "3");

    // A struct is a value the runtime does not give back: nothing is listed for it.
    session.step_over(over.thread).expect("a step over is asked");
    let over = stop(&events, "step");
    let frames = session.stack_trace(over.thread).expect("the stack reads");
    let locals = locals_of(&session, frames.first().expect("the stop has a frame"));
    assert!(
        !locals.iter().any(|variable| variable.name.ends_with(" returned")),
        "a struct is not given back: {locals:?}"
    );
    session.disconnect(true);
}

fn named_value(locals: &[rsharp_dap::protocol::Variable], index: usize) -> (String, String, String, String) {
    let variable = &locals[index];
    (
        variable.name.clone(),
        variable.value.clone(),
        variable.r#type.clone().unwrap_or_default(),
        variable.evaluate_name.clone().unwrap_or_default(),
    )
}

#[test]
fn a_step_out_lists_what_the_method_stepped_out_of_returned() {
    let _serial = one_at_a_time();
    let Some(program) = stepping_program() else { return };
    let source = stepping_source();
    let line = synthetic_line("string described = Describe(Count());");
    let (session, events) = started(
        &program,
        &[],
        vec![(source, vec![SourceBreakpoint { line: line - 1, ..SourceBreakpoint::default() }])],
        false,
    );

    let stopped = stop(&events, "breakpoint");
    let thread = stopped.thread;
    session.step_into(thread).expect("a step into is asked");
    let into = stop(&events, "step");
    assert_eq!(line_at(&session, &into), synthetic_line("static int Count() => 42;"), "into Count()");
    session.step_out(thread).expect("a step out is asked");
    let out = stop(&events, "step");
    assert_eq!(line_at(&session, &out), line, "back at the statement");
    let frames = session.stack_trace(out.thread).expect("the stack reads");
    let top = frames.first().expect("the stop has a frame");
    let locals = locals_of(&session, top);
    let returned = locals.first().unwrap_or_else(|| panic!("the returned value is listed: {locals:?}"));
    assert_eq!((returned.name.as_str(), returned.value.as_str()), ("Count() returned", "42"));
    assert_eq!(evaluate(&session, top.id, "$ReturnValue"), "42");
    session.disconnect(true);
}

#[test]
fn a_stop_at_an_exception_lists_it_as_exception_and_an_expression_reads_its_message() {
    let _serial = one_at_a_time();
    let Some(program) = stepping_program() else { return };
    let source = stepping_source();
    let line = line_of("SteppingConsoleApp", "Throws.Run();");
    let (session, events) = started_filtering(
        &program,
        &[],
        vec![(source, vec![SourceBreakpoint { line: line - 1, ..SourceBreakpoint::default() }])],
        // Every exception, wherever it is thrown and whether it is handled, as Passenger's ["all"] does: the client's
        // empty-path filter with every scenario is breakMode "always".
        vec![rsharp_dap::ExceptionFilter {
            scenarios: [
                "thrownInUserCode",
                "thrownInExternalCode",
                "handledInUserCode",
                "handledInExternalCode",
                "unhandled",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            ..rsharp_dap::ExceptionFilter::default()
        }],
    );

    let stopped = stop(&events, "breakpoint");
    session.resume(stopped.thread).expect("the program runs on");
    let thrown = stop(&events, "exception");
    let throw_line = line_of(
        "SteppingConsoleApp",
        "static void Fail(string reason) => throw new InvalidOperationException(reason);",
    );
    assert_eq!(line_at(&session, &thrown), throw_line, "the throw's own line");

    let frames = session.stack_trace(thrown.thread).expect("the stack reads");
    let top = frames.first().expect("the stop has a frame");
    let locals = locals_of(&session, top);
    let exception = locals
        .iter()
        .find(|variable| variable.name == "$exception")
        .unwrap_or_else(|| panic!("$exception is listed: {locals:?}"));
    assert_eq!(exception.r#type.as_deref(), Some("System.InvalidOperationException"));
    assert_eq!(evaluate(&session, top.id, "$exception.Message"), "\"bad input\"");
    session.disconnect(true);
}

#[test]
fn a_lambda_on_the_line_is_stepped_into_when_what_it_is_handed_to_runs_it() {
    let _serial = one_at_a_time();
    let Some(program) = stepping_program() else { return };
    let source = stepping_source();
    let statement = "int[] doubled = [.. Numbers().Select(n => Twice(n))];";
    let line = line_of("SteppingConsoleApp", statement);
    // At the statement's column: a breakpoint on the line alone binds to the lambda's body, which shares it.
    let column = source_text("SteppingConsoleApp", line).find(statement).expect("the statement is written there");
    let (session, events) = started(
        &program,
        &[],
        vec![(source, vec![SourceBreakpoint { line: line - 1, column: Some(column), ..SourceBreakpoint::default() }])],
        false,
    );

    let stopped = stop(&events, "breakpoint");
    assert_eq!(line_at(&session, &stopped), line, "the stop is on the statement's line");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let targets = session.step_in_targets(frames[0].id).expect("the calls are listed");
    let labels: Vec<&str> = targets.iter().map(|target| target.label.as_str()).collect();
    assert_eq!(labels, ["Numbers()", "n => Twice(n)"], "the calls in the order they run: {targets:?}");

    // Select is the framework's; the lambda's body is the program's own.
    session.step_into_target(stopped.thread, targets[1].id).expect("a step into the lambda is asked");
    let into = stop(&events, "step");
    assert_eq!(line_at(&session, &into), line, "the lambda's body shares the statement's line");
    let frames = session.stack_trace(into.thread).expect("the stack reads");
    let top = frames.first().expect("the stop has a frame");
    assert!(top.name.contains(">b__"), "the frame is the lambda's own method: {top:?}");
    let locals = locals_of(&session, top);
    assert_eq!(locals.iter().find(|variable| variable.name == "n").map(|variable| variable.value.as_str()), Some("1"));
    session.disconnect(true);
}

#[test]
fn an_evaluation_that_waits_on_another_thread_is_stopped_and_says_why() {
    let _serial = one_at_a_time();
    let Some(program) = looping_program() else { return };
    let source = rsharp_core::paths::canonical(
        &debuggees().expect("the debuggees are there").join("LoopingConsoleApp/Program.cs"),
    );
    let line = line_of("LoopingConsoleApp", "stop while counting");
    let (session, events) = started(
        &program,
        &["count"],
        vec![(source, vec![SourceBreakpoint { line: line - 1, ..SourceBreakpoint::default() }])],
        false,
    );

    let stopped = stop(&events, "breakpoint");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let top = frames.first().expect("the stop has a frame");
    let waited = session.evaluate("Counter.Next", top.id).expect("the evaluation answers");
    assert!(
        waited.result.contains("another thread"),
        "the evaluation that waits on the paused program's other thread is stopped and says why: {}",
        waited.result
    );
    let after = session.evaluate("1 + 1", top.id).expect("the next evaluation answers");
    assert_eq!(after.result, "2", "the session goes on answering after it");
    session.disconnect(true);
}

#[test]
fn a_condition_that_calls_a_method_stops_a_parallel_loop_on_its_matching_iteration() {
    let _serial = one_at_a_time();
    let Some(program) = looping_program() else { return };
    let source = rsharp_core::paths::canonical(
        &debuggees().expect("the debuggees are there").join("LoopingConsoleApp/Program.cs"),
    );
    let line = line_of("LoopingConsoleApp", "each iteration");
    let (session, events) = started(
        &program,
        &["parallel"],
        vec![(
            source,
            vec![SourceBreakpoint {
                line: line - 1,
                condition: Some("i.ToString() == \"377\"".into()),
                ..SourceBreakpoint::default()
            }],
        )],
        false,
    );

    let stopped = stop(&events, "breakpoint");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let locals = locals_of(&session, frames.first().expect("the stop has a frame"));
    assert_eq!(
        locals.iter().find(|variable| variable.name == "i").map(|variable| variable.value.as_str()),
        Some("377"),
        "the parallel loop stopped on the iteration its condition asked for: {locals:?}"
    );
    session.disconnect(true);
}

#[test]
fn evaluation_answers_in_a_paused_frame_with_members_properties_and_methods() {
    let _serial = one_at_a_time();
    if adapter().is_none() || rsharp_core::programs::dotnet().is_none() || debuggees().is_none() {
        eprintln!("skipped: dotnet, the adapter or Passenger's debuggees are not there");
        return;
    }
    let Some(program) = build("DebuggableConsoleApp") else { return };
    let source = rsharp_core::paths::canonical(
        &debuggees().expect("the debuggees are there").join("DebuggableConsoleApp/MyClass.cs"),
    );
    let (session, events) = started(
        &program,
        &[],
        vec![(source, vec![SourceBreakpoint { line: 21, ..SourceBreakpoint::default() }])],
        false,
    );

    let stopped = stop(&events, "breakpoint");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let top = frames.first().expect("the stop has a frame");

    // Passenger's EvalTests asks each of these of the paused frame; private members and calls included.
    for (expression, expected) in [
        ("myInt + 10", "14"),
        ("myInt + myInt", "8"),
        ("myIntParam + 4", "10"),
        ("_instanceField + 4", "9"),
        ("_instanceStaticField + 4", "10"),
        ("IntProperty + 4", "14"),
        ("IntStaticProperty + 4", "14"),
        ("ClassProperty.IntField + 4", "10"),
        ("this.Get14() + 4", "18"),
        ("MyClass.IntStaticProperty + 4", "14"),
        ("DebuggableConsoleApp.MyClass.IntStaticProperty + 4", "14"),
        ("Namespace1.AnotherClass.IntStaticProperty + 4", "14"),
        ("this.DoubleNumber(4)", "8"),
        ("this.DoubleNumber(4f)", "8"),
        ("Get14()", "14"),
        ("GetObject().IntProperty", "6"),
    ] {
        assert_eq!(evaluate(&session, top.id, expression), expected, "{expression}");
    }

    let locals = locals_of(&session, top);
    let value =
        |name: &str| locals.iter().find(|variable| variable.name == name).map(|variable| variable.value.clone());
    assert_eq!(value("myParam").as_deref(), Some("13"));
    assert_eq!(value("myInt").as_deref(), Some("4"));
    assert_eq!(value("anotherVar").as_deref(), Some("\"asdf\""));
    assert_eq!(value("nullableInt").as_deref(), Some("null"));
    assert_eq!(value("enumVar").as_deref(), Some("SecondValue"));
    session.disconnect(true);
}

#[test]
fn a_paused_frame_lists_its_locals_and_their_members() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let source = rsharp_core::paths::canonical(
        &debuggees().expect("the debuggees are there").join("DebuggableConsoleApp/MyClass.cs"),
    );
    let (session, events) = started(
        &program,
        &[],
        vec![(source, vec![SourceBreakpoint { line: 21, ..SourceBreakpoint::default() }])],
        false,
    );

    let stopped = stop(&events, "breakpoint");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let top = frames.first().expect("the stop has a frame");
    let locals = locals_of(&session, top);
    let named = |name: &str| locals.iter().find(|variable| variable.name == name).cloned();

    // Passenger's VariablesTests lists these eleven for MyClass.MyMethod.
    assert_eq!(locals.len(), 11, "the frame's locals: {locals:?}");
    let expected = [
        ("myParam", "13", "long"),
        ("myIntParam", "6", "int"),
        ("myInt", "4", "int"),
        ("enumVar", "SecondValue", "DebuggableConsoleApp.MyEnum"),
        ("enumWithFlagsVar", "FlagValue1 | FlagValue3", "DebuggableConsoleApp.MyEnumWithFlags"),
        ("nullableInt", "null", "int?"),
        ("structVar", "{DebuggableConsoleApp.MyStruct}", "DebuggableConsoleApp.MyStruct"),
        ("nullableIntWithVal", "4", "int?"),
        ("nullableRefType", "null", "DebuggableConsoleApp.MyClass"),
        ("anotherVar", "\"asdf\"", "string"),
    ];
    for (name, value, kind) in expected {
        let variable = named(name).unwrap_or_else(|| panic!("{name} is a local: {locals:?}"));
        assert_eq!((variable.value.as_str(), variable.r#type.as_deref()), (value, Some(kind)), "{name}");
    }
    let this = named("this").unwrap_or_else(|| panic!("this is a local: {locals:?}"));
    assert_eq!(this.value, "{DebuggableConsoleApp.MyClass}");

    // The struct's own members, and the object's, as the tree opens them.
    let struct_var = named("structVar").expect("structVar is a local");
    let members = session.variables(struct_var.variables_reference).expect("the struct's members read");
    let member = |members: &[rsharp_dap::protocol::Variable], name: &str| {
        members.iter().find(|variable| variable.name == name).map(|variable| variable.value.clone())
    };
    assert_eq!(member(&members, "Id").as_deref(), Some("5"), "the struct's Id: {members:?}");
    assert_eq!(member(&members, "Name").as_deref(), Some("\"StructName\""), "the struct's Name: {members:?}");

    let this_members = session.variables(this.variables_reference).expect("the object's members read");
    assert_eq!(member(&this_members, "IntProperty").as_deref(), Some("10"));
    assert_eq!(member(&this_members, "_instanceField").as_deref(), Some("5"));
    assert_eq!(member(&this_members, "_name").as_deref(), Some("\"TestName\""));

    // An enum: its value and the group holding its members.
    let enum_var = named("enumVar").expect("enumVar is a local");
    let enum_members = session.variables(enum_var.variables_reference).expect("the enum's members read");
    assert_eq!(member(&enum_members, "value__").as_deref(), Some("1"), "the enum's value: {enum_members:?}");
    let statics = enum_members
        .iter()
        .find(|variable| variable.name == "Static members")
        .unwrap_or_else(|| panic!("the enum has its Static members: {enum_members:?}"));
    let values = session.variables(statics.variables_reference).expect("the enum's values read");
    let names: Vec<&str> = values.iter().map(|variable| variable.name.as_str()).collect();
    assert_eq!(names, ["FirstValue", "SecondValue", "ThirdValue"], "the enum's values: {values:?}");
    session.disconnect(true);
}

#[test]
fn an_exception_in_a_parallel_loop_whose_iterations_test_a_condition_stops_where_it_was_thrown() {
    let _serial = one_at_a_time();
    let Some(program) = looping_program() else { return };
    let source = rsharp_core::paths::canonical(
        &debuggees().expect("the debuggees are there").join("LoopingConsoleApp/Program.cs"),
    );
    let line = line_of("LoopingConsoleApp", "each iteration");
    let (session, events) = started_filtering(
        &program,
        &["parallel", "throws"],
        vec![(
            source,
            vec![SourceBreakpoint {
                line: line - 1,
                // Never true, but each iteration's test runs code in the program while the other threads wait.
                condition: Some("i.ToString() == \"none\"".into()),
                ..SourceBreakpoint::default()
            }],
        )],
        vec![rsharp_dap::ExceptionFilter {
            scenarios: [
                "thrownInUserCode",
                "thrownInExternalCode",
                "handledInUserCode",
                "handledInExternalCode",
                "unhandled",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            ..rsharp_dap::ExceptionFilter::default()
        }],
    );

    let stopped = stop(&events, "exception");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let locals = locals_of(&session, frames.first().expect("the stop has a frame"));
    assert_eq!(
        locals.iter().find(|variable| variable.name == "i").map(|variable| variable.value.as_str()),
        Some("200"),
        "the iteration that threw: {locals:?}"
    );
    let info = session.exception_info(stopped.thread).expect("the exception is described");
    assert_eq!(
        info.details.as_ref().and_then(|details| details.message.as_deref()),
        Some("iteration 200"),
        "the exception's message: {info:?}"
    );
    session.disconnect(true);
}

#[test]
fn a_stack_lists_the_programs_frames_and_the_libraries_between_them() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let debuggees = debuggees().expect("the debuggees are there");
    let with_bcl = rsharp_core::paths::canonical(&debuggees.join("DebuggableConsoleApp/ClassWithBclCall.cs"));
    let (session, events) = started(
        &program,
        &[],
        vec![(with_bcl.clone(), vec![SourceBreakpoint { line: 11, ..SourceBreakpoint::default() }])],
        false,
    );

    let stopped = stop(&events, "breakpoint");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let names: Vec<String> = frames.iter().map(|frame| frame.short_name()).collect();
    let paths: Vec<Option<PathBuf>> = frames.iter().map(|frame| frame.path()).collect();
    let lines: Vec<usize> = frames.iter().map(|frame| frame.line + 1).collect();

    // Passenger's StackTraceTests lists five, the library's frames among the program's.
    assert_eq!(frames.len(), 5, "the stack: {names:?}");
    assert!(names[0].ends_with("ClassWithBclCall.Selector(int x)"), "the top frame: {names:?}");
    assert_eq!(paths[0].as_deref(), Some(with_bcl.as_path()));
    assert_eq!(lines[0], 12, "the selector's own line");
    assert_eq!(frames[0].column, 2, "on return x * 2's column, from 0");
    assert!(names[1].contains("RangeSelectIterator"), "the library's fill: {names:?}");
    assert!(names[2].contains("ToArray"), "and its ToArray: {names:?}");
    assert_eq!((paths[1].clone(), paths[2].clone()), (None, None), "no source for the library's frames");
    assert_eq!(
        (frames[1].presentation_hint.as_deref(), frames[2].presentation_hint.as_deref()),
        (Some("subtle"), Some("subtle")),
        "the library's frames are outside the program's code"
    );
    assert!(names[3].ends_with("ClassWithBclCall.Test(int myParam)"), "{names:?}");
    assert_eq!((lines[3], paths[3].clone()), (7, Some(with_bcl)), "the call's own line");
    assert!(names[4].ends_with("Program.Main(string[] args)"), "{names:?}");
    assert_eq!(lines[4], 31, "the loop's call of Test");

    session.disconnect(true);
}

#[test]
fn a_stop_inside_a_lambda_lists_the_values_it_closes_over() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let lambda = rsharp_core::paths::canonical(
        &debuggees().expect("the debuggees are there").join("DebuggableConsoleApp/Lambdas/MyLambdaClass.cs"),
    );
    let (session, events) = started(
        &program,
        &[],
        vec![(lambda, vec![SourceBreakpoint { line: 27, ..SourceBreakpoint::default() }])],
        false,
    );

    let stopped = stop(&events, "breakpoint");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let top = frames.first().expect("the stop has a frame");
    assert!(top.short_name().contains(">b__"), "the frame is a lambda's own method: {top:?}");
    let locals = locals_of(&session, top);

    // Passenger's LambdaVariablesTests lists these eleven inside the inner lambda, the captured ones among them.
    assert_eq!(locals.len(), 11, "the lambda's locals: {locals:?}");
    let expected = [
        ("capturedString", "\"captured\"", "string"),
        ("innerLocalFromOuterLocalInt1", "4", "int"),
        ("innerLocalFromOuterLocalInt2", "15", "int"),
        ("innerLocalFromRootLocalInt", "10", "int"),
        ("innerLocalFromRootLocalString", "\"captured\"", "string"),
        ("local", "10", "int"),
        ("outerLocalFromCapturedField", "4", "int"),
        ("outerLocalFromLocal", "15", "int"),
        ("result", "25", "int"),
        ("y", "5", "int"),
    ];
    for (name, value, kind) in expected {
        let variable = locals
            .iter()
            .find(|variable| variable.name == name)
            .unwrap_or_else(|| panic!("{name} is a local of the lambda: {locals:?}"));
        assert_eq!((variable.value.as_str(), variable.r#type.as_deref()), (value, Some(kind)), "{name}");
    }
    let this =
        locals.iter().find(|variable| variable.name == "this").unwrap_or_else(|| panic!("this is a local: {locals:?}"));
    assert_eq!(this.value, "{DebuggableConsoleApp.Lambdas.MyLambdaClass}");
    session.disconnect(true);
}

/// The stop's top frame: the file it is in (empty when none) and its line, counted from 1.
fn top_of(session: &Session, stop: &rsharp_dap::Stop) -> (String, usize) {
    let frames = session.stack_trace(stop.thread).expect("the stack reads");
    let top = frames.first().expect("the stop has a frame");
    (top.path().map(|path| path.to_string_lossy().into_owned()).unwrap_or_default(), top.line + 1)
}

#[test]
fn stepping_through_an_async_method_and_the_method_it_awaits() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let debuggees = debuggees().expect("the debuggees are there");
    let async_class = rsharp_core::paths::canonical(&debuggees.join("DebuggableConsoleApp/MyAsyncClass.cs"));
    let (session, events) = started(
        &program,
        &[],
        vec![(async_class, vec![SourceBreakpoint { line: 8, ..SourceBreakpoint::default() }])],
        false,
    );

    let mut stopped = stop(&events, "breakpoint");
    assert_eq!(top_of(&session, &stopped).1, 9, "the first statement");

    // Passenger's AsyncStepTests: over the sync statement, to the await, over the await, to the next
    // await. The thread may change after an await, so each step names the stop's own.
    for expected in [10, 11, 12, 13] {
        session.step_over(stopped.thread).expect("a step over is asked");
        stopped = stop(&events, "step");
        let (file, line) = top_of(&session, &stopped);
        assert!(file.ends_with("MyAsyncClass.cs"), "still in the async method: {file}");
        assert_eq!(line, expected);
    }

    // Into the awaited method, then into its statement, then out to where the await was.
    session.step_into(stopped.thread).expect("a step into is asked");
    let into = stop(&events, "step");
    let (file, line) = top_of(&session, &into);
    assert!(file.ends_with("AnotherClass.cs"), "in the awaited method: {file}");
    assert_eq!(line, 18, "its own line");
    session.step_into(into.thread).expect("a step into is asked again");
    let inside = stop(&events, "step");
    assert_eq!(top_of(&session, &inside), (top_of(&session, &into).0, 19), "its first statement");
    session.step_out(inside.thread).expect("a step out is asked");
    let out = stop(&events, "step");
    let (file, line) = top_of(&session, &out);
    assert!(file.ends_with("MyAsyncClass.cs"), "back in the async method: {file}");
    assert_eq!(line, 14, "where the await was");
    session.disconnect(true);
}

#[test]
fn a_launched_program_says_what_hot_reload_can_change() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let source = rsharp_core::paths::canonical(
        &debuggees().expect("the debuggees are there").join("DebuggableConsoleApp/MyClass.cs"),
    );
    let (session, events) = started(
        &program,
        &[],
        vec![(source, vec![SourceBreakpoint { line: 21, ..SourceBreakpoint::default() }])],
        false,
    );
    let _stopped = stop(&events, "breakpoint");

    let state = session.hot_reload_state().expect("the program says what it can take");
    assert!(
        state.capabilities.as_ref().is_some_and(|capabilities| !capabilities.is_empty()),
        "the runtime's Edit and Continue capabilities: {state:?}"
    );
    assert!(state.capabilities_unknown.is_none(), "the runtime said them: {state:?}");
    let module = state
        .modules
        .iter()
        .find(|module| module.name.eq_ignore_ascii_case("DebuggableConsoleApp.dll"))
        .unwrap_or_else(|| panic!("the program's own module is listed: {state:?}"));
    assert!(module.editable, "its code can be edited as it runs: {module:?}");
    assert!(module.path.is_some(), "and its file named: {module:?}");
    session.disconnect(true);
}
