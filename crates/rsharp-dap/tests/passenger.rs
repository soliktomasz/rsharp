//! Passenger's own debugger tests, translated: the same `tests/Debuggees` programs, the same questions, asked through
//! this client. The first batch is what Passenger's `PauseTests` asks of `LoopingConsoleApp` — a program that never ends
//! on its own — including the values a pause in the runtime's sleep leaves readable.

use rsharp_dap::{Event, FunctionBreakpoint, Launch, Session, SourceBreakpoint, Start, adapter};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

/// Builds one debuggee and answers its assembly, built once per process; skips (None) when dotnet is not there, as
/// Passenger's tests do.
fn build(name: &str) -> Option<PathBuf> {
    use std::sync::{Mutex, OnceLock};
    static BUILT: OnceLock<Mutex<BTreeMap<String, PathBuf>>> = OnceLock::new();
    let built = BUILT.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Some(assembly) = built.lock().unwrap().get(name) {
        return Some(assembly.clone());
    }
    let assembly = build_once(name)?;
    built.lock().unwrap().insert(name.to_string(), assembly.clone());
    Some(assembly)
}

fn build_once(name: &str) -> Option<PathBuf> {
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
    started_with_symbols(
        program,
        arguments,
        breakpoints,
        at_entry,
        &std::env::temp_dir().join("rsharp-passenger-symbols"),
    )
}

/// `started`, the adapter's decompiled symbols kept in `symbols` (a folder of its own for a test that decompiles).
fn started_with_symbols(
    program: &Path,
    arguments: &[&str],
    breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)>,
    at_entry: bool,
    symbols: &Path,
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
            just_my_code: true,
        }),
        symbols,
        breakpoints,
        Vec::new(),
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
            just_my_code: true,
        }),
        &std::env::temp_dir().join("rsharp-passenger-symbols"),
        breakpoints,
        Vec::new(),
        exceptions,
        Vec::new(),
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
fn a_library_frame_is_resolved_from_the_decompiled_symbols() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let with_bcl = rsharp_core::paths::canonical(
        &debuggees().expect("the debuggees are there").join("DebuggableConsoleApp/ClassWithBclCall.cs"),
    );
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let symbols = std::env::temp_dir().join(format!("rsharp-decompiled-{}-{stamp}", std::process::id()));
    let (session, events) = started_with_symbols(
        &program,
        &[],
        vec![(with_bcl, vec![SourceBreakpoint { line: 11, ..SourceBreakpoint::default() }])],
        false,
        &symbols,
    );

    let stopped = stop(&events, "breakpoint");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let unresolved = frames
        .iter()
        .find(|frame| frame.is_resolved == Some(false))
        .expect("a library frame with no source of its own");
    assert_eq!(unresolved.path(), None, "it has no source before its assembly is decompiled");
    assert!(unresolved.decompiled_source_info.is_none(), "nothing is decompiled until it is asked for");

    // The adapter decompiles System.Linq into the cache, and the client reads the text its PDB embeds.
    let resolved = session.resolve_stack_frame(unresolved.id).expect("the frame resolves");
    assert_eq!(resolved.is_resolved, Some(true), "the frame has source now");
    assert!(resolved.path().is_some_and(|path| path.to_string_lossy().ends_with(".cs")), "and a source file");
    assert!(resolved.line > 0, "its line in the decompiled source");
    let info = resolved.decompiled_source_info.as_ref().expect("what the source was decompiled from");
    assert!(info.type_full_name.contains("RangeSelectIterator"), "the type: {}", info.type_full_name);
    assert!(info.assembly.assembly_path.to_string_lossy().ends_with("System.Linq.dll"), "{:?}", info.assembly);
    let (path, text) = session.decompiled_source(&resolved).expect("the embedded source reads");
    assert!(
        text.contains("RangeSelectIterator") || text.contains("class "),
        "the source: {:?}",
        text.chars().take(200).collect::<String>()
    );
    assert!(
        path.to_string_lossy().contains("decompiled"),
        "kept under the cache's decompiled folder: {}",
        path.display()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "the file a window opens is the text");
    assert!(std::fs::metadata(&path).unwrap().permissions().readonly(), "decompiled code opens read-only");
    session.disconnect(true);
    let _ = std::fs::remove_dir_all(&symbols);
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

/// A line of a debuggee's file, counted from 1, that carries `marker`.
fn line_in(name: &str, file: &str, marker: &str) -> usize {
    let root = debuggees().expect("the debuggees are there");
    std::fs::read_to_string(root.join(name).join(file))
        .expect("the debuggee's source reads")
        .lines()
        .position(|line| line.contains(marker))
        .map(|line| line + 1)
        .unwrap_or_else(|| panic!("{file} has no line with {marker}"))
}

/// A column of `marker` on a line (from 1) of a debuggee's file, counted from 1.
fn column_in(name: &str, file: &str, line: usize, marker: &str) -> usize {
    let root = debuggees().expect("the debuggees are there");
    let text = std::fs::read_to_string(root.join(name).join(file)).expect("the debuggee's source reads");
    let text = text.lines().nth(line - 1).unwrap_or_else(|| panic!("{file} has no line {line}"));
    text.find(marker).unwrap_or_else(|| panic!("{file}:{line} has no {marker}")) + 1
}

/// A debuggee's file as breakpoints name it.
fn source_of(name: &str, file: &str) -> PathBuf {
    rsharp_core::paths::canonical(&debuggees().expect("the debuggees are there").join(name).join(file))
}

/// The top frame of a stop, as Passenger's `GetTopStackFrame` reads it.
fn top_frame(session: &Session, thread: i64) -> rsharp_dap::protocol::Frame {
    session.stack_trace(thread).expect("the stack reads").into_iter().next().expect("the stop has a frame")
}

/// Whether the frame is at Passenger's place: its file's ending, line, end line, column and end column, counted from 1.
fn at(
    frame: &rsharp_dap::protocol::Frame,
    file: &str,
    line: usize,
    end_line: usize,
    column: usize,
    end_column: usize,
) -> bool {
    frame.path().is_some_and(|path| path.ends_with(file))
        && frame.line + 1 == line
        && frame.end_line.map(|end| end + 1) == Some(end_line)
        && frame.column + 1 == column
        && frame.end_column.map(|end| end + 1) == Some(end_column)
}

/// What the program wrote next, from an output that starts with `prefix`.
fn wait_for_output(events: &Receiver<Event>, prefix: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "the program never wrote {prefix:?}; saw {seen:?}");
        match events.recv_timeout(left) {
            Ok(Event::Output { text, .. }) if text.starts_with(prefix) => return text,
            Ok(event) => seen.push(format!("{event:?}")),
            Err(error) => panic!("the program never wrote {prefix:?}: {error}; saw {seen:?}"),
        }
    }
}

/// The debugger's word on a breakpoint whose message carries `needle`, as it binds or says why not.
fn wait_for_breakpoint_message(events: &Receiver<Event>, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "no breakpoint message named {needle:?}; saw {seen:?}");
        match events.recv_timeout(left) {
            Ok(Event::Breakpoint { message: Some(message), verified: true, .. }) if message.contains(needle) => {
                return message;
            }
            Ok(event) => seen.push(format!("{event:?}")),
            Err(error) => panic!("no breakpoint message named {needle:?}: {error}; saw {seen:?}"),
        }
    }
}

/// Stops at each of `lines` in turn, resuming between, and answers the last stop's thread.
fn stops_at(session: &Session, events: &Receiver<Event>, file: &str, lines: &[usize]) -> i64 {
    let mut thread = 0;
    for (index, line) in lines.iter().enumerate() {
        let stopped = stop(events, "breakpoint");
        thread = stopped.thread;
        let frame = top_frame(session, thread);
        assert!(at(&frame, file, *line, *line, 2, 3), "stop {} of {}: {frame:?}", index + 1, lines.len());
        if index + 1 < lines.len() {
            session.resume(thread).expect("the program goes on");
        }
    }
    thread
}

#[test]
fn a_launch_stopped_at_entry_stops_in_main() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = started(&program, &[], Vec::new(), true);
    let stopped = stop(&events, "entry");
    let frame = top_frame(&session, stopped.thread);
    assert!(frame.name.contains("Main"), "the entry frame is Main: {frame:?}");
    assert!(frame.path().is_some_and(|path| path.ends_with("Program.cs")), "{frame:?}");
    session.disconnect(true);
}

#[test]
fn a_launch_stopped_at_entry_stops_in_a_top_level_statements_main() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("TopLevelStatementsConsoleApp") else { return };
    let (session, events) = started(&program, &[], Vec::new(), true);
    let stopped = stop(&events, "entry");
    let frame = top_frame(&session, stopped.thread);
    assert!(frame.name.contains("Main"), "{frame:?}");
    assert!(frame.path().is_some_and(|path| path.ends_with("Program.cs")), "{frame:?}");
    assert_eq!(frame.line + 1, 2, "the first statement, as Passenger's launch finds it");
    assert!(locals_of(&session, &frame).iter().any(|variable| variable.name == "test"));
    session.disconnect(true);
}

#[test]
fn a_launch_stopped_at_entry_stops_in_an_async_main() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("AsyncEntryConsoleApp") else { return };
    let (session, events) = started(&program, &[], Vec::new(), true);
    let stopped = stop(&events, "entry");
    let frame = top_frame(&session, stopped.thread);
    assert!(frame.name.contains("Main"), "{frame:?}");
    assert!(frame.path().is_some_and(|path| path.ends_with("Program.cs")), "{frame:?}");
    assert_eq!(frame.line + 1, 5);
    assert!(locals_of(&session, &frame).iter().any(|variable| variable.name == "test"));
    session.disconnect(true);
}

#[test]
fn a_launch_stopped_at_entry_stops_in_an_async_top_level_statements_main() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("AsyncTopLevelStatementsConsoleApp") else { return };
    let (session, events) = started(&program, &[], Vec::new(), true);
    let stopped = stop(&events, "entry");
    let frame = top_frame(&session, stopped.thread);
    assert!(frame.name.contains("Main"), "{frame:?}");
    assert!(frame.path().is_some_and(|path| path.ends_with("Program.cs")), "{frame:?}");
    assert_eq!(frame.line + 1, 2);
    assert!(locals_of(&session, &frame).iter().any(|variable| variable.name == "test"));
    session.disconnect(true);
}

#[test]
fn a_breakpoint_with_a_column_stops_at_that_statement() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (name, file) = ("DebuggableConsoleApp", "ColumnBreakpointClass.cs");
    let line = line_in(name, file, "column-breakpoint-line");
    let column = column_in(name, file, line, "var third");
    let end_column = column + "var third = second + 1;".len();
    let (session, events) = started(
        &program,
        &[],
        vec![(
            source_of(name, file),
            vec![SourceBreakpoint { line: line - 1, column: Some(column - 1), ..SourceBreakpoint::default() }],
        )],
        false,
    );
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert!(frame.path().is_some_and(|path| path.ends_with(file)), "{frame:?}");
    assert_eq!((frame.line + 1, frame.column + 1), (line, column), "{frame:?}");
    assert_eq!(frame.end_column.map(|column| column + 1), Some(end_column), "{frame:?}");
    session.disconnect(true);
}

#[test]
fn a_breakpoint_without_a_column_stops_at_the_lines_first_statement() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (name, file) = ("DebuggableConsoleApp", "ColumnBreakpointClass.cs");
    let line = line_in(name, file, "column-breakpoint-line");
    let column = column_in(name, file, line, "var first");
    let (session, events) = started(
        &program,
        &[],
        vec![(source_of(name, file), vec![SourceBreakpoint { line: line - 1, ..SourceBreakpoint::default() }])],
        false,
    );
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert_eq!((frame.line + 1, frame.column + 1), (line, column), "{frame:?}");
    session.disconnect(true);
}

#[test]
fn a_breakpoint_with_a_column_on_a_multiline_statement_stops_at_its_start() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (name, file) = ("DebuggableConsoleApp", "ColumnBreakpointClass.cs");
    let start = line_in(name, file, "var multi =");
    let requested_line = start + 1;
    let requested_column = column_in(name, file, requested_line, "1 + 25");
    let expected_column = column_in(name, file, start, "var multi");
    let (session, events) = started(
        &program,
        &[],
        vec![(
            source_of(name, file),
            vec![SourceBreakpoint {
                line: requested_line - 1,
                column: Some(requested_column - 1),
                ..SourceBreakpoint::default()
            }],
        )],
        false,
    );
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert_eq!((frame.line + 1, frame.column + 1), (start, expected_column), "{frame:?}");
    session.disconnect(true);
}

#[test]
fn a_function_breakpoint_names_a_method_and_stops_in_it() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = started(&program, &[], Vec::new(), false);
    session
        .set_function_breakpoints(&[FunctionBreakpoint {
            name: "DebuggableConsoleApp.MyClass.MyMethod".into(),
            ..FunctionBreakpoint::default()
        }])
        .expect("the function breakpoint is set");
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert!(at(&frame, "MyClass.cs", 10, 10, 2, 3), "{frame:?}");
    session.disconnect(true);
}

#[test]
fn a_function_breakpoint_of_a_generic_class_stops_in_its_nested_method() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = started(&program, &[], Vec::new(), false);
    session
        .set_function_breakpoints(&[FunctionBreakpoint {
            name:
                "DebuggableConsoleApp.MyGenericClassContainingAnotherGenericClass<T, U>.MyNestedGenericClass<T, U>.Test"
                    .into(),
            ..FunctionBreakpoint::default()
        }])
        .expect("the function breakpoint is set");
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert!(at(&frame, "MyClassContainingAnotherClass.cs", 18, 18, 3, 4), "{frame:?}");
    session.disconnect(true);
}

#[test]
fn a_function_breakpoint_of_a_name_in_two_namespaces_stops_in_both() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = started(&program, &[], Vec::new(), false);
    session
        .set_function_breakpoints(&[FunctionBreakpoint {
            name: "SameNamedClass.Test".into(),
            ..FunctionBreakpoint::default()
        }])
        .expect("the function breakpoint is set");

    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert!(at(&frame, "SameNamedClass.cs", 6, 6, 3, 4), "{frame:?}");
    session.resume(stopped.thread).expect("the program goes on");

    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert!(at(&frame, "SameNamedClass2.cs", 6, 6, 3, 4), "{frame:?}");
    session.disconnect(true);
}

#[test]
fn a_function_breakpoint_without_parameters_stops_at_every_overload() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = started(&program, &[], Vec::new(), false);
    let overloads = [14, 18, 22, 26, 30];
    session
        .set_function_breakpoints(&[FunctionBreakpoint {
            name: "DebuggableConsoleApp.OverloadedMethodsClass.OverloadedMethod".into(),
            ..FunctionBreakpoint::default()
        }])
        .expect("the function breakpoint is set");
    let thread = stops_at(&session, &events, "OverloadedMethodsClass.cs", &overloads);

    // The classless name finds the same methods; the stop at the last overload goes on to the next iteration's first.
    session
        .set_function_breakpoints(&[FunctionBreakpoint {
            name: "OverloadedMethod".into(),
            ..FunctionBreakpoint::default()
        }])
        .expect("the function breakpoint is set");
    session.resume(thread).expect("the program goes on");
    stops_at(&session, &events, "OverloadedMethodsClass.cs", &overloads);
    session.disconnect(true);
}

#[test]
fn a_function_breakpoint_with_its_parameters_stops_at_that_overload_only() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = started(&program, &[], Vec::new(), false);
    session
        .set_function_breakpoints(&[FunctionBreakpoint {
            name: "DebuggableConsoleApp.OverloadedMethodsClass.OverloadedMethod(int, MyClass)".into(),
            ..FunctionBreakpoint::default()
        }])
        .expect("the function breakpoint is set");
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert!(at(&frame, "OverloadedMethodsClass.cs", 30, 30, 2, 3), "{frame:?}");
    session.resume(stopped.thread).expect("the program goes on");
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert!(at(&frame, "OverloadedMethodsClass.cs", 30, 30, 2, 3), "the same overload on its next call: {frame:?}");
    session.disconnect(true);
}

#[test]
fn a_breakpoint_with_a_true_condition_stops() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (name, file) = ("DebuggableConsoleApp", "MyClass.cs");
    let line = line_in(name, file, "Console.WriteLine($\"Log_MyMethod\")");
    let (session, events) = started(
        &program,
        &[],
        vec![(
            source_of(name, file),
            vec![SourceBreakpoint {
                line: line - 1,
                condition: Some("myInt == 4".into()),
                ..SourceBreakpoint::default()
            }],
        )],
        false,
    );
    let stopped = stop(&events, "breakpoint");
    assert_eq!(stopped.line, Some(line - 1));
    session.disconnect(true);
}

#[test]
fn a_breakpoint_with_a_false_condition_does_not_stop() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (name, file) = ("DebuggableConsoleApp", "MyClass.cs");
    let conditional = line_in(name, file, "Console.WriteLine($\"Log_MyMethod\")");
    let unconditional = line_in(name, file, "var anotherVar = \"asdf\";") + 1;
    let (session, events) = started(
        &program,
        &[],
        vec![(
            source_of(name, file),
            vec![
                SourceBreakpoint {
                    line: conditional - 1,
                    condition: Some("myInt == 999".into()),
                    ..SourceBreakpoint::default()
                },
                SourceBreakpoint { line: unconditional - 1, ..SourceBreakpoint::default() },
            ],
        )],
        false,
    );
    let stopped = stop(&events, "breakpoint");
    assert_eq!(stopped.line, Some(unconditional - 1), "the false condition's line is passed over");
    session.disconnect(true);
}

#[test]
fn a_breakpoint_whose_condition_cannot_be_told_stops_and_says_why() {
    let _serial = one_at_a_time();
    for condition in ["noSuchName", "myInt"] {
        let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
        let (name, file) = ("DebuggableConsoleApp", "MyClass.cs");
        let line = line_in(name, file, "Console.WriteLine($\"Log_MyMethod\")");
        let (session, events) = started(
            &program,
            &[],
            vec![(
                source_of(name, file),
                vec![SourceBreakpoint {
                    line: line - 1,
                    condition: Some(condition.into()),
                    ..SourceBreakpoint::default()
                }],
            )],
            false,
        );
        let message = wait_for_breakpoint_message(&events, condition);
        assert!(message.contains(condition), "the message names the condition: {message}");
        let stopped = stop(&events, "breakpoint");
        assert_eq!(stopped.line, Some(line - 1), "it stops rather than going missing unseen");
        session.disconnect(true);
    }
}

#[test]
fn a_tracepoint_logs_its_message_and_never_stops() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (name, file) = ("DebuggableConsoleApp", "MyClass.cs");
    let traced = line_in(name, file, "Console.WriteLine($\"Log_MyMethod\")");
    let after = line_in(name, file, "var anotherVar = \"asdf\";") + 1;
    let (session, events) = started(
        &program,
        &[],
        vec![(
            source_of(name, file),
            vec![
                SourceBreakpoint {
                    line: traced - 1,
                    log_message: Some("myInt is {myInt}, and the list has {_intList.Count} \\{literally\\}".into()),
                    ..SourceBreakpoint::default()
                },
                SourceBreakpoint { line: after - 1, ..SourceBreakpoint::default() },
            ],
        )],
        false,
    );
    let output = wait_for_output(&events, "myInt is");
    assert_eq!(output, "myInt is 4, and the list has 4 {literally}\n");
    let stopped = stop(&events, "breakpoint");
    assert_eq!(stopped.line, Some(after - 1), "the program went on past the tracepoint");
    session.disconnect(true);
}

#[test]
fn a_hit_condition_of_two_stops_on_the_second_hit() {
    let _serial = one_at_a_time();
    for form in ["==2", "2", "= 2"] {
        let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
        let (name, file) = ("DebuggableConsoleApp", "HitConditionClass.cs");
        let line = line_in(name, file, "breakpoint here");
        let (session, events) = started(
            &program,
            &[],
            vec![(
                source_of(name, file),
                vec![SourceBreakpoint {
                    line: line - 1,
                    hit_condition: Some(form.into()),
                    ..SourceBreakpoint::default()
                }],
            )],
            false,
        );
        let stopped = stop(&events, "breakpoint");
        let frame = top_frame(&session, stopped.thread);
        let count = session.evaluate("_count", frame.id).expect("_count evaluates");
        assert_eq!(count.result, "2", "the second hit, for {form:?}");
        session.disconnect(true);
    }
}

/// The stop a program comes to next, whatever its reason: a step or a goto stops as a goto or a step.
fn next_stop(events: &Receiver<Event>) -> rsharp_dap::Stop {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "the program never stopped; saw {seen:?}");
        match events.recv_timeout(left) {
            Ok(Event::Stopped(stop)) => return stop,
            Ok(Event::Ended) => panic!("the debugger ended before a stop; saw {seen:?}"),
            Ok(event) => seen.push(format!("{event:?}")),
            Err(error) => panic!("the program never stopped: {error}; saw {seen:?}"),
        }
    }
}

#[test]
fn a_stack_appends_the_synthetic_async_callers() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (name, file) = ("DebuggableConsoleApp", "AsyncStackTraceClass.cs");
    let line = line_in(name, file, "await Task.Delay(10);") + 1;
    let (session, events) = started(
        &program,
        &[],
        vec![(source_of(name, file), vec![SourceBreakpoint { line: line - 1, ..SourceBreakpoint::default() }])],
        false,
    );
    let stopped = stop(&events, "breakpoint");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let tail: Vec<(String, usize)> =
        frames.iter().rev().take(2).rev().map(|frame| (frame.name.clone(), frame.line + 1)).collect();
    assert_eq!(
        tail,
        vec![
            ("DebuggableConsoleApp.dll!DebuggableConsoleApp.AsyncStackTraceClass.MiddleAsync()".to_string(), 13),
            ("DebuggableConsoleApp.dll!DebuggableConsoleApp.AsyncStackTraceClass.TestAsync()".to_string(), 7),
        ],
        "the async callers the adapter adds: {:?}",
        frames.iter().map(|frame| frame.short_name()).collect::<Vec<_>>()
    );
    session.disconnect(true);
}

#[test]
fn set_next_statement_skips_ahead_and_goes_back_within_the_method() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let source =
        rsharp_core::paths::canonical(&debuggees().expect("the debuggees are there").join("PassengerCases/Program.cs"));
    let first = line_of("PassengerCases", "jumps += 1;");
    let second = line_of("PassengerCases", "jumps += 10;");
    let third = line_of("PassengerCases", "jumps += 100;");
    let (session, events) = started(
        &program,
        &[],
        vec![(source.clone(), vec![SourceBreakpoint { line: first - 1, ..SourceBreakpoint::default() }])],
        false,
    );
    let stopped = stop(&events, "breakpoint");
    let thread = stopped.thread;
    assert_eq!(stopped.line, Some(first - 1));

    // Past the first two additions, so only the hundred is added.
    let target = session.goto_targets(&source, third - 1).expect("the targets read");
    let target = target.first().expect("the third line is in the method");
    assert_eq!(target.line, third - 1);
    session.set_next_statement(thread, target.id).expect("the thread goes on from there");
    let moved = stop(&events, "goto");
    let frame = top_frame(&session, moved.thread);
    assert_eq!(frame.line + 1, third, "the thread went on from the third line: {frame:?}");
    session.step_over(moved.thread).expect("the step is taken");
    let stepped = next_stop(&events);
    let frame = top_frame(&session, stepped.thread);
    assert_eq!(session.evaluate("jumps", frame.id).expect("jumps evaluates").result, "100");

    // Back to the second, which then adds the ten and the hundred again.
    let target = session.goto_targets(&source, second - 1).expect("the targets read");
    let target = target.first().expect("the second line is in the method");
    session.set_next_statement(thread, target.id).expect("the thread goes on from there");
    let moved = stop(&events, "goto");
    let frame = top_frame(&session, moved.thread);
    assert_eq!(frame.line + 1, second, "the thread went back to the second line: {frame:?}");
    for _ in 0..2 {
        session.step_over(moved.thread).expect("the step is taken");
        next_stop(&events);
    }
    let frame = top_frame(&session, moved.thread);
    assert_eq!(session.evaluate("jumps", frame.id).expect("jumps evaluates").result, "210");

    // A line of another method is no place to go on from.
    let elsewhere = line_of("PassengerCases", "Console.WriteLine(\"interpreter\");");
    assert!(session.goto_targets(&source, elsewhere - 1).expect("the targets read").is_empty());
    session.disconnect(true);
}

/// Starts a debuggee at entry with a Just My Code choice, the breakpoints it stops at.
fn started_just_my_code(
    program: &Path,
    breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)>,
    just_my_code: bool,
) -> (Session, Receiver<Event>) {
    started_with_filters(program, breakpoints, just_my_code, Vec::new())
}

/// `started_just_my_code`, the DAP exception filters set before the program's first line, as Passenger's launches set them.
fn started_with_filters(
    program: &Path,
    breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)>,
    just_my_code: bool,
    filters: Vec<(String, Option<String>)>,
) -> (Session, Receiver<Event>) {
    let adapter = adapter().expect("Passenger's debug adapter is built");
    let (sender, events) = mpsc::channel();
    let working_directory = program.parent().expect("the assembly is in a folder").to_path_buf();
    let session = Session::start(
        &adapter,
        Start::Launch(Launch {
            program: program.to_path_buf(),
            arguments: Vec::new(),
            working_directory,
            environment: BTreeMap::new(),
            stop_at_entry: false,
            hot_reload: true,
            just_my_code,
        }),
        &std::env::temp_dir().join("rsharp-passenger-symbols"),
        breakpoints,
        Vec::new(),
        Vec::new(),
        filters,
        None,
        move |event| drop(sender.send(event)),
    )
    .expect("the session starts");
    (session, events)
}

/// The debuggee stopped where it chooses its exception, the filters set and the exception chosen, and the breakpoint after
/// the throw set; the stop that follows is the exception or the breakpoint, as the filters say.
fn threw(
    program: &Path,
    filters: &[(&str, Option<&str>)],
    exception: &str,
    just_my_code: bool,
) -> (Session, Receiver<Event>) {
    let file = source_of("DebuggableConsoleApp", "Program.cs");
    let setup = line_in("DebuggableConsoleApp", "Program.cs", "myLambdaClass.Test();");
    let completion =
        line_in("DebuggableConsoleApp", "Program.cs", "MyGenericClassContainingAnotherGenericClass<int, string>");
    let (session, events) = started_just_my_code(
        program,
        vec![(file.clone(), vec![SourceBreakpoint { line: setup - 1, ..SourceBreakpoint::default() }])],
        just_my_code,
    );
    let stopped = stop(&events, "breakpoint");
    assert_eq!(stopped.line, Some(setup - 1), "the setup breakpoint");
    let frame = top_frame(&session, stopped.thread);
    let set = session
        .evaluate(&format!("exceptionToThrow = ExceptionToThrow.{exception}"), frame.id)
        .expect("the exception is chosen");
    assert_eq!(set.result, exception);
    session.set_exception_filters(filters).expect("the filters are set");
    session
        .set_breakpoints(&file, &[SourceBreakpoint { line: completion - 1, ..SourceBreakpoint::default() }])
        .expect("the completion breakpoint is set");
    session.resume(stopped.thread).expect("the program goes on");
    (session, events)
}

/// The stop's exception, as the adapter describes it.
fn exception_at(session: &Session, thread: i64) -> rsharp_dap::protocol::ExceptionInfo {
    session.exception_info(thread).expect("the exception reads")
}

#[test]
fn an_exception_filter_of_all_stops_on_a_normal_exception() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = threw(&program, &[("all", None)], "Normal", true);
    let stopped = next_stop(&events);
    assert_eq!(stopped.reason, "exception");
    let info = exception_at(&session, stopped.thread);
    assert_eq!(
        info.details.as_ref().and_then(|details| details.full_type_name.as_deref()),
        Some("System.InvalidOperationException")
    );
    assert_eq!(info.break_mode, "always");
    session.disconnect(true);
}

#[test]
fn an_exception_filter_of_all_passes_an_exception_handled_in_external_code_with_just_my_code() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = threw(&program, &[("all", None)], "HandledWithinExternalCode", true);
    let stopped = next_stop(&events);
    let completion =
        line_in("DebuggableConsoleApp", "Program.cs", "MyGenericClassContainingAnotherGenericClass<int, string>");
    assert_eq!(
        (stopped.reason.as_str(), stopped.line),
        ("breakpoint", Some(completion - 1)),
        "the handled exception is passed over"
    );
    session.disconnect(true);
}

#[test]
fn an_exception_filter_of_all_stops_on_an_exception_handled_in_external_code_without_just_my_code() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = threw(&program, &[("all", None)], "HandledWithinExternalCode", false);
    let stopped = next_stop(&events);
    assert_eq!(stopped.reason, "exception");
    let info = exception_at(&session, stopped.thread);
    assert_eq!(
        info.details.as_ref().and_then(|details| details.full_type_name.as_deref()),
        Some("System.ArgumentException")
    );
    session.disconnect(true);
}

#[test]
fn an_exception_filter_of_all_stops_on_one_thrown_in_external_code_and_returned_to_the_user() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = threw(&program, &[("all", None)], "ExternalCode", true);
    let stopped = next_stop(&events);
    assert_eq!(stopped.reason, "exception");
    let info = exception_at(&session, stopped.thread);
    assert_eq!(
        info.details.as_ref().and_then(|details| details.full_type_name.as_deref()),
        Some("System.FormatException")
    );
    session.disconnect(true);
}

#[test]
fn a_user_unhandled_filter_stops_where_the_user_left_the_exception() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = threw(&program, &[("user-unhandled", None)], "UserUnhandled", true);
    let stopped = next_stop(&events);
    assert_eq!(stopped.reason, "exception");
    let info = exception_at(&session, stopped.thread);
    assert_eq!(
        info.details.as_ref().and_then(|details| details.full_type_name.as_deref()),
        Some("System.InvalidOperationException")
    );
    assert_eq!(info.break_mode, "userUnhandled");
    session.disconnect(true);
}

#[test]
fn a_user_unhandled_filter_passes_an_exception_the_user_handled() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = threw(&program, &[("user-unhandled", None)], "Normal", true);
    let stopped = next_stop(&events);
    assert_eq!(stopped.reason, "breakpoint");
    session.disconnect(true);
}

#[test]
fn no_exception_filter_passes_a_handled_exception() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = threw(&program, &[], "Normal", true);
    let stopped = next_stop(&events);
    assert_eq!(stopped.reason, "breakpoint");
    session.disconnect(true);
}

#[test]
fn an_exception_filters_condition_picks_the_types_it_names() {
    let _serial = one_at_a_time();
    for (condition, stops) in [("System.InvalidOperationException", true), ("System.FormatException", false)] {
        let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
        let (session, events) = threw(&program, &[("user-unhandled", Some(condition))], "UserUnhandled", true);
        let stopped = next_stop(&events);
        assert_eq!(stopped.reason == "exception", stops, "the condition {condition}");
        session.disconnect(true);
    }
}

#[test]
fn step_into_passes_over_methods_the_code_marks_as_not_the_users() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let source = source_of("PassengerCases", "Program.cs");
    let start = line_in("PassengerCases", "Program.cs", "Attributed.StepThrough();");
    let expected = [
        line_in("PassengerCases", "Program.cs", "Attributed.NonUser();"),
        line_in("PassengerCases", "Program.cs", "Attributed.Hidden();"),
        line_in("PassengerCases", "Program.cs", "Console.WriteLine(\"end\");"),
    ];
    let (session, events) = started_just_my_code(
        &program,
        vec![(source, vec![SourceBreakpoint { line: start - 1, ..SourceBreakpoint::default() }])],
        true,
    );
    let mut stopped = stop(&events, "breakpoint");
    for line in expected {
        session.step_into(stopped.thread).expect("the step is taken");
        stopped = next_stop(&events);
        let frame = top_frame(&session, stopped.thread);
        assert_eq!(frame.line + 1, line, "the step landed at {frame:?}");
    }
    session.disconnect(true);
}

#[test]
fn a_breakpoint_in_a_method_marked_not_the_users_is_not_hit_and_says_why() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let source = source_of("PassengerCases", "Program.cs");
    let marked = line_in("PassengerCases", "Program.cs", "[DebuggerStepThrough] public static void StepThrough()");
    let end = line_in("PassengerCases", "Program.cs", "Console.WriteLine(\"end\");");
    let (session, events) = started_just_my_code(
        &program,
        vec![(
            source.clone(),
            vec![
                SourceBreakpoint { line: marked - 1, ..SourceBreakpoint::default() },
                SourceBreakpoint { line: end - 1, ..SourceBreakpoint::default() },
            ],
        )],
        true,
    );
    let stopped = stop(&events, "breakpoint");
    assert_eq!(stopped.line, Some(end - 1), "the marked method's breakpoint is passed over");

    let answers = session
        .set_breakpoints(&source, &[SourceBreakpoint { line: marked - 1, ..SourceBreakpoint::default() }])
        .expect("the breakpoint is sent again");
    let answer = answers.first().expect("the breakpoint's answer");
    assert!(!answer.verified, "the debugger refuses it: {answer:?}");
    assert!(
        answer.message.as_deref().is_some_and(|message| message.contains("Just My Code")),
        "the answer says why: {answer:?}"
    );
    session.disconnect(true);
}

#[test]
fn an_evaluation_that_never_returns_is_stopped_and_the_debugger_goes_on() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let source = source_of("PassengerCases", "Program.cs");
    let start = line_in("PassengerCases", "Program.cs", "Attributed.Hidden();");
    let end = line_in("PassengerCases", "Program.cs", "Console.WriteLine(\"end\");");
    let (session, events) = started_just_my_code(
        &program,
        vec![(source, vec![SourceBreakpoint { line: start - 1, ..SourceBreakpoint::default() }])],
        true,
    );
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    let timed_out = session.evaluate("Slow.Forever", frame.id).expect("the engine answers the timeout");
    assert!(timed_out.result.contains("was stopped"), "the evaluation was stopped: {timed_out:?}");
    assert_eq!(session.evaluate("1 + 1", frame.id).expect("an evaluation after it").result, "2");
    session.step_over(stopped.thread).expect("the step is taken");
    let stepped = next_stop(&events);
    let frame = top_frame(&session, stepped.thread);
    assert_eq!(frame.line + 1, end);
    session.disconnect(true);
}

#[test]
fn every_thread_listed_has_frames_to_show() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let source = source_of("PassengerCases", "Program.cs");
    let line = line_in("PassengerCases", "Program.cs", "Console.WriteLine(\"both workers are waiting\");");
    let (session, events) = started_just_my_code(
        &program,
        vec![(source, vec![SourceBreakpoint { line: line - 1, ..SourceBreakpoint::default() }])],
        true,
    );
    let stopped = stop(&events, "breakpoint");
    let threads = session.threads().expect("the threads read");
    let main = threads.iter().find(|thread| thread.id == stopped.thread).expect("the stopped thread is listed");
    assert_eq!(main.name, "Main Thread");
    assert!(threads.iter().any(|thread| thread.name == "worker-named"), "{threads:?}");
    for thread in &threads {
        assert!(!session.stack_trace(thread.id).expect("a stack reads").is_empty(), "{thread:?}");
    }
    session.disconnect(true);
}

/// An expression's value and type in a frame, as an evaluation answers.
fn eval(session: &Session, frame: i64, expression: &str) -> rsharp_dap::protocol::Evaluated {
    session.evaluate(expression, frame).unwrap_or_else(|why| panic!("{expression} evaluates: {why}"))
}

/// A local by its name, as the Locals scope lists it.
fn local<'a>(locals: &'a [rsharp_dap::protocol::Variable], name: &str) -> &'a rsharp_dap::protocol::Variable {
    locals.iter().find(|variable| variable.name == name).unwrap_or_else(|| {
        panic!("{name} is among the locals: {:?}", locals.iter().map(|v| &v.name).collect::<Vec<_>>())
    })
}

/// Stops a debuggee of `name` at the line carrying `marker`, and answers its top frame.
fn stopped_in_frame(
    program: &Path,
    name: &str,
    file: &str,
    marker: &str,
) -> (Session, Receiver<Event>, rsharp_dap::protocol::Frame) {
    let source = source_of(name, file);
    let line = line_in(name, file, marker);
    let (session, events) = started_just_my_code(
        program,
        vec![(source, vec![SourceBreakpoint { line: line - 1, ..SourceBreakpoint::default() }])],
        true,
    );
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert_eq!((frame.line + 1, frame.path().is_some_and(|path| path.ends_with(file))), (line, true), "{frame:?}");
    (session, events, frame)
}

/// `stopped_in_frame`, and the frame's id to evaluate in.
fn stopped_at(program: &Path, name: &str, file: &str, marker: &str) -> (Session, Receiver<Event>, i64) {
    let (session, events, frame) = stopped_in_frame(program, name, file, marker);
    (session, events, frame.id)
}

#[test]
fn integers_enums_conversions_and_unboxing_evaluate_as_the_language_says() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let (session, _events, frame) =
        stopped_at(&program, "PassengerCases", "Program.cs", "Console.WriteLine(\"interpreter\");");

    let complement = eval(&session, frame, "~myInt");
    assert_eq!((complement.result.as_str(), complement.r#type.as_deref()), ("-6", Some("int")));
    let converted = eval(&session, frame, "checked((int)myUInt)");
    assert_eq!((converted.result.as_str(), converted.r#type.as_deref()), ("7", Some("int")));
    assert_eq!(eval(&session, frame, "(int)minusOne").result, "-1");
    assert_eq!(eval(&session, frame, "(int)wide").result, "65535");
    assert_eq!(eval(&session, frame, "new Counter[2].Length").result, "2");
    assert!(eval(&session, frame, "((Pair<string>)boxedPair).Value").result.contains("InvalidCast"));

    eval(&session, frame, "counter.Increment()");
    assert_eq!(eval(&session, frame, "counter.Count").result, "1");
    assert_eq!(eval(&session, frame, "price = 10m").result, "10");
    assert_eq!(eval(&session, frame, "price").result, "10");
    session.disconnect(true);
}

#[test]
fn set_value_assigns_what_an_expression_names_and_answers_with_it() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let (session, _events, frame) =
        stopped_at(&program, "PassengerCases", "Program.cs", "Console.WriteLine(\"interpreter\");");

    let set = session.set_expression("pairOfInt.Value", "40 + 2", frame).expect("the value is set");
    assert_eq!(set.value, "42");
    assert_eq!(eval(&session, frame, "pairOfInt.Value").result, "42");
    let refused = session.set_expression("myInt", "\"text\"", frame).expect_err("the type is refused");
    assert!(!refused.is_empty());
    assert_eq!(eval(&session, frame, "myInt").result, "5");
    session.disconnect(true);
}

#[test]
fn a_multi_dimensional_arrays_elements_evaluate_and_take_a_value_set() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let (session, _events, frame) =
        stopped_at(&program, "PassengerCases", "Program.cs", "Console.WriteLine(\"interpreter\");");

    let element = eval(&session, frame, "grid[1, 1]");
    assert_eq!((element.result.as_str(), element.r#type.as_deref()), ("4", Some("int")));
    assert_eq!(eval(&session, frame, "grid[2, 0] + grid[2, 1]").result, "11");
    let set = session.set_expression("grid[0, 1]", "40 + 2", frame).expect("the element is set");
    assert_eq!(set.value, "42");
    assert_eq!(eval(&session, frame, "grid[0, 1]").result, "42");
    assert_eq!(eval(&session, frame, "pairGrid[1, 0].Value = 7").result, "7");
    assert_eq!(eval(&session, frame, "pairGrid[1, 0].Value").result, "7");
    assert!(eval(&session, frame, "grid[3, 0]").result.contains("IndexOutOfRange"));
    session.disconnect(true);
}

#[test]
fn a_constrained_call_on_a_struct_argument_changes_the_argument() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let (session, _events, frame) =
        stopped_at(&program, "PassengerCases", "Program.cs", "Console.WriteLine(\"bumper\");");

    eval(&session, frame, "item.Bump()");
    assert_eq!(eval(&session, frame, "((Bumped)(object)item).N").result, "1");
    session.disconnect(true);
}

#[test]
fn a_checked_conversion_out_of_range_throws_the_overflow_the_program_would() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let (session, _events, frame) =
        stopped_at(&program, "PassengerCases", "Program.cs", "Console.WriteLine(\"interpreter\");");

    for expression in [
        "checked((ulong)(myInt - 6))",
        "checked((nuint)(myInt - 6))",
        "checked((byte)(myInt + 251))",
        "checked((sbyte)(myUInt + 121))",
        "checked((nint)(myInt * 1e30))",
        "checked((ulong)(myInt - 6.5))",
    ] {
        let result = eval(&session, frame, expression).result;
        assert!(
            result.contains("System.OverflowException") && !result.contains("CIL execution failed"),
            "{expression} gave {result:?}, where the program would throw an OverflowException"
        );
    }
    assert_eq!(eval(&session, frame, "checked((ulong)(myInt - 5))").result, "0");
    assert_eq!(eval(&session, frame, "myUInt = uint.MaxValue").result, "4294967295");
    assert_eq!(eval(&session, frame, "myUInt").result, "4294967295");
    session.disconnect(true);
}

#[test]
fn a_parameter_a_lambda_captures_is_listed_once_with_its_live_value() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let (session, _events, frame) =
        stopped_in_frame(&program, "PassengerCases", "Program.cs", "Console.WriteLine(f());");
    let locals = locals_of(&session, &frame);
    assert_eq!(local(&locals, "captured").value, "41");
    assert_eq!(locals.iter().filter(|variable| variable.name == "captured").count(), 1, "listed once");
    assert_eq!(locals.iter().filter(|variable| variable.name == "f").count(), 1);
    session.disconnect(true);
}

#[test]
fn an_exception_wrapping_another_reports_the_inner_one() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let source = source_of("PassengerCases", "Program.cs");
    let start = line_in("PassengerCases", "Program.cs", "Console.WriteLine(f());");
    let (session, events) = started_with_filters(
        &program,
        vec![(source, vec![SourceBreakpoint { line: start - 1, ..SourceBreakpoint::default() }])],
        true,
        vec![("all".into(), None)],
    );
    // Every exception stops, the earlier cases' too: run on to the one that wraps another.
    let mut stop = next_stop(&events);
    let mut found = None;
    for _ in 0..10 {
        if stop.reason == "exception" {
            let info = exception_at(&session, stop.thread);
            if info
                .details
                .as_ref()
                .and_then(|details| details.inner_exception.as_ref())
                .is_some_and(|inner| !inner.is_empty())
            {
                found = Some(info);
                break;
            }
        }
        session.resume(stop.thread).expect("the program goes on");
        stop = next_stop(&events);
    }
    let info = found.unwrap_or_else(|| panic!("the wrapping exception never stopped; last stop {stop:?}"));
    assert_eq!(
        info.details.as_ref().and_then(|details| details.full_type_name.as_deref()),
        Some("System.InvalidOperationException")
    );
    let inner = info
        .details
        .and_then(|details| details.inner_exception)
        .and_then(|inner| inner.into_iter().next())
        .expect("the exception it wraps");
    assert_eq!(
        (inner.full_type_name.as_deref(), inner.message.as_deref(), inner.evaluate_name.as_deref()),
        (Some("System.Exception"), Some("Message1"), Some("$exception.InnerException"))
    );
    assert!(inner.inner_exception.is_none_or(|inner| inner.is_empty()));
    assert!(
        info.description
            .as_deref()
            .is_some_and(|description| description.contains("Innermost exception: System.Exception"))
    );
    session.disconnect(true);
}

#[test]
fn debugger_display_renders_as_visual_studio_does() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let (session, _events, frame) =
        stopped_in_frame(&program, "PassengerCases", "Program.cs", "Console.WriteLine(\"displays\");");
    let locals = locals_of(&session, &frame);

    assert_eq!(local(&locals, "money").value, "12.5 USD");
    assert_eq!(local(&locals, "quoted").value, "12.5 \"USD\"");
    assert_eq!(local(&locals, "braced").value, "{3}");
    assert!(
        local(&locals, "missing").value.starts_with("Value=2 Bad=<error: The name 'NoSuchMember' does not exist"),
        "{:?}",
        local(&locals, "missing").value
    );
    assert_eq!(local(&locals, "head").value, "head -> tail");
    assert!(local(&locals, "tail").value.starts_with("tail -> <error:"));
    assert_eq!(
        (local(&locals, "labelled").value.as_str(), local(&locals, "labelled").r#type.as_deref()),
        ("11", Some("RetypedThing"))
    );
    let holder = local(&locals, "holder");
    let holder_members = session.variables(holder.variables_reference).expect("the holder's member reads");
    let item = holder_members.first().expect("one member");
    assert_eq!(
        (item.name.as_str(), item.value.as_str(), item.r#type.as_deref()),
        ("renamed", "11", Some("RetypedThing"))
    );
    assert_eq!(local(&locals, "derived").value, "base b");
    assert_eq!(local(&locals, "box").value, "box of 1 EUR");
    let by_name = session.variables(local(&locals, "byName").variables_reference).expect("the entries read");
    let entry = by_name.iter().find(|variable| variable.name != "Raw View").expect("an entry besides Raw View");
    assert_eq!((entry.name.as_str(), entry.value.as_str()), ("[\"origin\"]", "(0, 0)"));
    let lifted = session.variables(local(&locals, "lifted").variables_reference).expect("the lifted members read");
    let mut names: Vec<&str> = lifted.iter().map(|variable| variable.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["A", "B"]);
    let angry = session.variables(local(&locals, "angry").variables_reference).expect("the value's own members read");
    assert_eq!(local(&angry, "Real").value, "1");
    assert!(!angry.iter().any(|variable| variable.name == "_message" || variable.name == "StackTrace"));
    session.disconnect(true);
}

#[test]
fn a_value_whose_tostring_throws_shows_its_type_and_opens_to_its_members() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let (session, _events, frame) =
        stopped_in_frame(&program, "PassengerCases", "Program.cs", "Console.WriteLine(\"displays\");");
    let locals = locals_of(&session, &frame);
    let unprintable = local(&locals, "unprintable");
    assert_eq!(unprintable.value, "{Unprintable}");
    assert!(unprintable.variables_reference > 0, "it has members to open");
    let members = session.variables(unprintable.variables_reference).expect("its members read");
    assert_eq!(local(&members, "Kept").value, "5");
    assert_eq!(eval(&session, frame.id, "unprintable").result, "{Unprintable}");
    session.disconnect(true);
}

#[test]
fn an_async_method_evaluates_its_locals_before_and_after_a_suspension() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let file = source_of("DebuggableConsoleApp", "AsyncMethodEvalClass.cs");
    let before = line_in("DebuggableConsoleApp", "AsyncMethodEvalClass.cs", "bp before suspension");
    let after = line_in("DebuggableConsoleApp", "AsyncMethodEvalClass.cs", "bp after suspension");
    let (session, events) = started_just_my_code(
        &program,
        vec![(
            file,
            vec![
                SourceBreakpoint { line: before - 1, ..SourceBreakpoint::default() },
                SourceBreakpoint { line: after - 1, ..SourceBreakpoint::default() },
            ],
        )],
        true,
    );
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert_eq!(frame.line + 1, before);
    assert_eq!(eval(&session, frame.id, "localInt").result, "4");
    assert_eq!(eval(&session, frame.id, "IntField").result, "10");
    session.resume(stopped.thread).expect("the program goes on");
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert_eq!(frame.line + 1, after, "after the await");
    assert_eq!(eval(&session, frame.id, "localInt").result, "4");
    assert_eq!(eval(&session, frame.id, "IntField").result, "10");
    session.disconnect(true);
}

#[test]
fn an_async_methods_locals_are_listed_before_and_after_its_await() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let file = source_of("DebuggableConsoleApp", "MyAsyncClass.cs");
    let line = line_in("DebuggableConsoleApp", "MyAsyncClass.cs", "var result = await");
    let (session, events) = started_just_my_code(
        &program,
        vec![(file, vec![SourceBreakpoint { line: line - 1, ..SourceBreakpoint::default() }])],
        true,
    );
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    let locals = locals_of(&session, &frame);
    assert_eq!(locals.len(), 6, "{:?}", locals.iter().map(|v| &v.name).collect::<Vec<_>>());
    assert_eq!(local(&locals, "this").value, "{DebuggableConsoleApp.MyAsyncClass}");
    assert_eq!(local(&locals, "myParam").value, "4");
    assert_eq!(local(&locals, "intVar").value, "10");
    for name in ["result", "result2", "result3"] {
        assert_eq!(local(&locals, name).value, "0");
    }

    // Into the awaited method, whose own frame has its own local.
    session.step_into(stopped.thread).expect("the step is taken");
    let stepped = next_stop(&events);
    let frame = top_frame(&session, stepped.thread);
    assert!(frame.path().is_some_and(|path| path.ends_with("AnotherClass.cs")), "{frame:?}");
    assert_eq!(frame.line + 1, 18, "the awaited method's first stop, as Passenger finds it: {frame:?}");
    let inner = locals_of(&session, &frame);
    assert_eq!(local(&inner, "test").value, "0");

    // Back at the breakpoint, the same locals as before.
    session.resume(stepped.thread).expect("the program goes on");
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    let again = locals_of(&session, &frame);
    assert_eq!(again.len(), 6);
    assert_eq!(local(&again, "intVar").value, "10");
    session.disconnect(true);
}

/// A launch's choices, for a case that changes them from the usual (no arguments, Just My Code on).
struct Launcher {
    arguments: Vec<String>,
    environment: BTreeMap<String, String>,
    stop_at_entry: bool,
    just_my_code: bool,
    filters: Vec<(String, Option<String>)>,
    terminal: Option<rsharp_dap::TerminalRun>,
}

impl Default for Launcher {
    fn default() -> Self {
        Self {
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            stop_at_entry: false,
            just_my_code: true,
            filters: Vec::new(),
            terminal: None,
        }
    }
}

/// Starts a debuggee as `how` asks, the session's failure kept for a case that expects one.
fn started_how(
    program: &Path,
    breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)>,
    how: Launcher,
) -> (Result<Session, String>, Receiver<Event>) {
    let adapter = adapter().expect("Passenger's debug adapter is built");
    let (sender, events) = mpsc::channel();
    let working_directory = program.parent().expect("the assembly is in a folder").to_path_buf();
    let session = Session::start(
        &adapter,
        Start::Launch(Launch {
            program: program.to_path_buf(),
            arguments: how.arguments,
            working_directory,
            environment: how.environment,
            stop_at_entry: how.stop_at_entry,
            hot_reload: true,
            just_my_code: how.just_my_code,
        }),
        &std::env::temp_dir().join("rsharp-passenger-symbols"),
        breakpoints,
        Vec::new(),
        Vec::new(),
        how.filters,
        how.terminal,
        move |event| drop(sender.send(event)),
    );
    (session, events)
}

/// Attaches to a running program, as the IDE's Debug Test does.
fn attached_how(
    process_id: u32,
    breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)>,
    just_my_code: bool,
) -> (Result<Session, String>, Receiver<Event>) {
    let adapter = adapter().expect("Passenger's debug adapter is built");
    let (sender, events) = mpsc::channel();
    let session = Session::start(
        &adapter,
        Start::Attach { process_id, just_my_code },
        &std::env::temp_dir().join("rsharp-passenger-symbols"),
        breakpoints,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        None,
        move |event| drop(sender.send(event)),
    );
    (session, events)
}

/// A debuggee's apphost, the executable its build puts beside its assembly, as Passenger's tests start one.
fn apphost(program: &Path) -> PathBuf {
    if cfg!(windows) { program.with_extension("exe") } else { program.with_extension("") }
}

/// Starts a debuggee the way a person would, its apphost run directly, so an attach has a runtime to take. Its output is
/// read and dropped, as a test's is: a program writing more than a pipe holds would otherwise wait on its own output.
fn run_debuggee(program: &Path, arguments: &[&str], environment: &[(&str, &str)]) -> rsharp_core::process::Child {
    let mut command = std::process::Command::new(apphost(program));
    command
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (name, value) in environment {
        command.env(name, value);
    }
    let mut child = rsharp_core::process::spawn(command).expect("the debuggee starts");
    for stream in [
        child.process.stdout.take().map(|stream| Box::new(stream) as Box<dyn std::io::Read + Send>),
        child.process.stderr.take().map(|stream| Box::new(stream) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        std::thread::spawn(move || {
            let mut stream = stream;
            let _ = std::io::copy(&mut stream, &mut std::io::sink());
        });
    }
    child
}

/// Whether the runtime's diagnostic socket name fits in the temp folder's path: a long path truncates the name, and the
/// pid with it, so no debugger can find the port an attach waits on.
fn diagnostic_sockets_fit() -> bool {
    std::env::temp_dir().to_string_lossy().len() <= 70
}

/// Skips an attach case whose debuggee's port a long temp path would hide.
fn skip_if_diagnostic_sockets_do_not_fit() -> bool {
    if diagnostic_sockets_fit() {
        return false;
    }
    eprintln!("skipped: the temp folder's path is too long for the runtime's diagnostic socket name");
    true
}

/// The client's terminal: it starts what the adapter asks for and keeps the child until the test ends.
fn terminal_runner() -> (rsharp_dap::TerminalRun, Arc<std::sync::Mutex<Vec<rsharp_core::process::Child>>>) {
    let kept = Arc::new(std::sync::Mutex::new(Vec::new()));
    let children = kept.clone();
    let runner: rsharp_dap::TerminalRun = Arc::new(move |request: rsharp_dap::RunInTerminal| {
        let program = request.args.first().ok_or("the terminal was asked for no program")?;
        let program = if program == "dotnet" {
            rsharp_core::programs::dotnet().unwrap_or_else(|| PathBuf::from("dotnet"))
        } else {
            PathBuf::from(program)
        };
        let mut command = std::process::Command::new(program);
        command.args(request.args.iter().skip(1));
        if let Some(cwd) = &request.cwd {
            command.current_dir(cwd);
        }
        for (name, value) in request.env.iter().flatten() {
            command.env(name, value);
        }
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let child = rsharp_core::process::spawn(command).map_err(|error| error.to_string())?;
        let process_id = child.process.id();
        children.lock().unwrap().push(child);
        Ok(rsharp_dap::RunInTerminalAnswer { process_id })
    });
    (runner, kept)
}

/// The program's output and exit code, waited for until it ends.
fn wait_for_end(events: &Receiver<Event>) -> (String, Option<i64>) {
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut output = String::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "the program never ended; it wrote {} characters", output.len());
        match events.recv_timeout(left) {
            Ok(Event::Output { text, .. }) => output.push_str(&text),
            Ok(Event::Exited(code)) => return (output, code),
            Ok(Event::Terminated) => return (output, None),
            Ok(_) => {}
            Err(error) => panic!("the program never ended: {error}; it wrote {} characters", output.len()),
        }
    }
}

#[test]
fn a_launched_program_reads_its_own_input_never_the_adapters() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("ReadsInputConsoleApp") else { return };
    let source = source_of("ReadsInputConsoleApp", "Program.cs");
    let line = line_in("ReadsInputConsoleApp", "Program.cs", "Console.WriteLine($\"read:");
    let (session, events) = started_how(
        &program,
        vec![(source, vec![SourceBreakpoint { line: line - 1, ..SourceBreakpoint::default() }])],
        Launcher::default(),
    );
    let session = session.expect("the session starts");
    // While it waits on its console, the debugger still answers, and the line it read is its own.
    session.input("hello\n").expect("the input is sent");
    let stopped = stop(&events, "breakpoint");
    assert_eq!(stopped.line, Some(line - 1));
    let frame = top_frame(&session, stopped.thread);
    assert_eq!(eval(&session, frame.id, "line").result, "\"hello\"");
    session.disconnect(true);
}

#[test]
fn a_debugged_programs_child_runs_as_it_would_undebugged() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let source = source_of("PassengerCases", "Program.cs");
    let line = line_in("PassengerCases", "Program.cs", "Console.WriteLine($\"child said: {said}\");");
    let (session, events) = started_how(
        &program,
        vec![(source, vec![SourceBreakpoint { line: line - 1, ..SourceBreakpoint::default() }])],
        Launcher::default(),
    );
    let session = session.expect("the session starts");
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert_eq!(eval(&session, frame.id, "said").result, "\"child ran\"");
    assert_eq!(
        eval(&session, frame.id, "Environment.GetEnvironmentVariable(\"DOTNET_DefaultDiagnosticPortSuspend\")").result,
        "null",
        "the hold the debugger uses at a start must not reach the programs the debuggee starts"
    );
    session.disconnect(true);
}

#[test]
fn a_program_writing_64_kb_runs_to_its_end_with_all_it_wrote_and_its_own_exit_code() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let (session, events) =
        started_how(&program, Vec::new(), Launcher { arguments: vec!["flood".into()], ..Launcher::default() });
    let session = session.expect("the session starts");
    let (output, exit_code) = wait_for_end(&events);
    let expected: String = (0..1024usize)
        .map(|i| format!("{i:05} {}\n", std::iter::repeat_n((b'a' + (i % 26) as u8) as char, 57).collect::<String>()))
        .collect();
    assert_eq!(output, expected, "the program wrote all 64 KB");
    assert_eq!(exit_code, Some(42), "the program's own exit code");
    session.disconnect(false);
}

#[test]
fn the_thread_running_main_is_the_one_named_main_thread() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("TopLevelStatementsConsoleApp") else { return };
    let (session, events) = started_how(&program, Vec::new(), Launcher { stop_at_entry: true, ..Launcher::default() });
    let session = session.expect("the session starts");
    let stopped = stop(&events, "entry");
    let threads = session.threads().expect("the threads read");
    let main = threads.iter().find(|thread| thread.id == stopped.thread).expect("the stopped thread is listed");
    assert_eq!(main.name, "Main Thread");
    assert_eq!(threads.iter().filter(|thread| thread.name == "Main Thread").count(), 1, "{threads:?}");
    session.disconnect(true);
}

#[test]
fn a_program_started_in_the_clients_terminal_stops_at_a_breakpoint_on_mains_first_line_every_time() {
    if skip_if_diagnostic_sockets_do_not_fit() {
        return;
    }
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("TerminalConsoleApp") else { return };
    let source = source_of("TerminalConsoleApp", "Program.cs");
    let first = line_in("TerminalConsoleApp", "Program.cs", "// Main's first line");
    for launch in 1..=5 {
        let (runner, kept) = terminal_runner();
        let (session, events) = started_how(
            &program,
            vec![(source.clone(), vec![SourceBreakpoint { line: first - 1, ..SourceBreakpoint::default() }])],
            Launcher { terminal: Some(runner), ..Launcher::default() },
        );
        let session = session.expect("the session starts");
        let stopped = stop(&events, "breakpoint");
        assert_eq!(stopped.line, Some(first - 1), "launch {launch} stopped at Main's first line");
        let frame = top_frame(&session, stopped.thread);
        assert!(frame.name.contains("Main"), "launch {launch} stopped in the terminal's program: {frame:?}");
        session.disconnect(true);
        drop(kept);
    }
}

#[test]
fn a_client_that_cannot_start_the_program_in_its_terminal_fails_the_launch_saying_why() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("TerminalConsoleApp") else { return };
    let terminal: rsharp_dap::TerminalRun = Arc::new(|_| Err("no terminal here".into()));
    let (session, _events) =
        started_how(&program, Vec::new(), Launcher { terminal: Some(terminal), ..Launcher::default() });
    let why = match session {
        Err(why) => why,
        Ok(_) => panic!("the launch fails"),
    };
    assert!(why.contains("no terminal here"), "the failure says why: {why}");
}

#[test]
fn a_program_that_ends_before_its_runtime_starts_fails_configuration_done_saying_how_it_ended() {
    if !cfg!(unix) {
        eprintln!("skipped: the case starts a shell");
        return;
    }
    let _serial = one_at_a_time();
    let (program, arguments) = (Path::new("/bin/sh"), vec!["-c".to_string(), "exit 3".to_string()]);
    let (session, _events) = started_how(program, Vec::new(), Launcher { arguments, ..Launcher::default() });
    let why = match session {
        Err(why) => why,
        Ok(_) => panic!("the launch fails"),
    };
    assert!(
        why.contains("it ended before its runtime told the debugger it had started") && why.contains("exit code 3"),
        "the failure says how it ended: {why}"
    );
}

#[test]
fn a_program_held_at_its_start_is_let_go_by_the_attach_and_its_breakpoints_first_pass_is_seen() {
    if skip_if_diagnostic_sockets_do_not_fit() {
        return;
    }
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let debuggee = run_debuggee(&program, &[], &[("DOTNET_DefaultDiagnosticPortSuspend", "1")]);
    let file = source_of("DebuggableConsoleApp", "HitConditionClass.cs");
    let line = line_in("DebuggableConsoleApp", "HitConditionClass.cs", "breakpoint here");
    let (session, events) = attached_how(
        debuggee.process.id(),
        vec![(file, vec![SourceBreakpoint { line: line - 1, ..SourceBreakpoint::default() }])],
        true,
    );
    let session = session.expect("the attach starts");
    // The attach lets the held program go once the debugger is registered for its runtime: the first pass is ours.
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert_eq!(eval(&session, frame.id, "_count").result, "1", "the first stop is the breakpoint's first pass");
    session.disconnect(true);
}

#[test]
fn a_running_program_is_the_debuggers_once_configuration_done_answers() {
    if skip_if_diagnostic_sockets_do_not_fit() {
        return;
    }
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let debuggee = run_debuggee(&program, &[], &[]);
    let (session, events) = attached_how(debuggee.process.id(), Vec::new(), true);
    let session = session.expect("the attach starts");
    session.pause(0).expect("the pause is sent");
    let stopped = next_stop(&events);
    assert_eq!(stopped.reason, "pause", "a pause right after configurationDone stops the program");
    session.disconnect(false);
}

#[test]
fn an_attached_programs_end_says_its_exit_code_where_the_system_tells_it() {
    if skip_if_diagnostic_sockets_do_not_fit() {
        return;
    }
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let mut debuggee = run_debuggee(&program, &["flood"], &[("DOTNET_DefaultDiagnosticPortSuspend", "1")]);
    let (session, events) = attached_how(debuggee.process.id(), Vec::new(), true);
    let session = session.expect("the attach starts");
    // The runtime is let go by the attach and writes 64 KB; it ends in its own time, and the debugger says so after.
    let deadline = Instant::now() + Duration::from_secs(60);
    while debuggee.process.try_wait().expect("the program's end is checked").is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let (_, exit_code) = wait_for_end(&events);
    let told = if cfg!(windows) { Some(42) } else { None };
    assert_eq!(exit_code, told, "the attached program ended with 42");
    session.disconnect(false);
}

#[test]
fn an_attach_to_a_program_that_is_not_dotnet_fails_saying_why_and_lets_it_run() {
    if !cfg!(unix) {
        eprintln!("skipped: the case starts sleep");
        return;
    }
    let _serial = one_at_a_time();
    let mut command = std::process::Command::new("/bin/sleep");
    command.arg("60");
    let mut other = rsharp_core::process::spawn(command).expect("sleep starts");
    let id = other.process.id();
    let (session, _events) = attached_how(id, Vec::new(), true);
    let why = match session {
        Err(why) => why,
        Ok(_) => panic!("the attach fails"),
    };
    assert!(why.contains(&format!("Could not attach to process {id}")), "the failure names the process: {why}");
    assert!(why.contains("not a .NET program"), "the failure says what it is: {why}");
    assert!(other.process.try_wait().expect("sleep is checked").is_none(), "the program the attach refused runs on");
    let _ = other.kill();
}

/// The debuggee stopped where it chooses its exception, the exception chosen, the setup breakpoint taken away and the
/// program let go: the next stop is the exception, as the filters say.
fn threw_at(program: &Path, exception: &str, filters: &[(&str, Option<&str>)]) -> (Session, Receiver<Event>) {
    let file = source_of("DebuggableConsoleApp", "Program.cs");
    let setup = line_in("DebuggableConsoleApp", "Program.cs", "myLambdaClass.Test();");
    let (session, events) = started_just_my_code(
        program,
        vec![(file.clone(), vec![SourceBreakpoint { line: setup - 1, ..SourceBreakpoint::default() }])],
        true,
    );
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    let set = session
        .evaluate(&format!("exceptionToThrow = ExceptionToThrow.{exception}"), frame.id)
        .expect("the exception is chosen");
    assert_eq!(set.result, exception);
    session.set_exception_filters(filters).expect("the filters are set");
    session.set_breakpoints(&file, &[]).expect("the setup breakpoint is taken away");
    session.resume(stopped.thread).expect("the program goes on");
    (session, events)
}

#[test]
fn a_stop_at_an_exception_lists_the_exception_scope() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = threw_at(&program, "Normal", &[("all", None), ("user-unhandled", None)]);
    let stopped = stop(&events, "exception");
    let frame = top_frame(&session, stopped.thread);
    let locals = locals_of(&session, &frame);
    let exception = local(&locals, "$exception");
    assert_eq!(exception.r#type.as_deref(), Some("System.InvalidOperationException"));
    assert!(
        exception.value.starts_with("System.InvalidOperationException: Test exception"),
        "the exception's own words: {:?}",
        exception.value
    );
    assert_eq!(local(&locals, "exceptionToThrow").value, "Normal");

    let info = exception_at(&session, stopped.thread);
    assert_eq!(info.exception_id, "CLR/System.InvalidOperationException");
    assert_eq!(info.break_mode, "always");
    let details = info.details.as_ref().expect("the exception's details");
    assert_eq!(details.full_type_name.as_deref(), Some("System.InvalidOperationException"));
    assert_eq!(details.message.as_deref(), Some("Test exception"));
    assert_eq!(details.evaluate_name.as_deref(), Some("$exception"));
    assert_eq!(details.h_result, Some(-2146233079));
    assert_eq!(details.source.as_deref(), Some("DebuggableConsoleApp"));
    let trace = details.stack_trace.as_deref().expect("its stack trace");
    assert!(trace.contains("DebuggableConsoleApp.Exceptions.Test(ExceptionToThrow exceptionToThrow)"), "{trace}");
    assert!(trace.contains(":line 14"), "{trace}");
    session.disconnect(true);
}

#[test]
fn an_exception_in_external_code_has_no_source_with_just_my_code() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = threw_at(&program, "ExternalCode", &[("all", None), ("user-unhandled", None)]);
    let stopped = stop(&events, "exception");
    assert_eq!(stopped.path, None, "a throw in external code has no source of its own");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let names: Vec<String> = frames.iter().map(|frame| frame.short_name()).collect();
    assert!(names.first().is_some_and(|name| name.contains("ThrowFormatException")), "{names:?}");
    assert!(names.get(1).is_some_and(|name| name.contains("Int32.Parse")), "{names:?}");
    let own = frames
        .iter()
        .find(|frame| frame.path().is_some_and(|path| path.ends_with("Exceptions.cs")))
        .expect("the program's own frame");
    assert_eq!(own.line + 1, 18);
    let main = frames
        .iter()
        .find(|frame| frame.path().is_some_and(|path| path.ends_with("Program.cs")))
        .expect("Main's frame");
    assert_eq!(main.line + 1, 34);

    let top = top_frame(&session, stopped.thread);
    let locals = locals_of(&session, &top);
    let exception = local(&locals, "$exception");
    assert_eq!(exception.r#type.as_deref(), Some("System.FormatException"));
    assert!(exception.value.starts_with("System.FormatException: The input string 'x' was not in a correct format."));
    let info = exception_at(&session, stopped.thread);
    let details = info.details.as_ref().expect("the exception's details");
    assert_eq!(details.full_type_name.as_deref(), Some("System.FormatException"));
    assert_eq!(details.message.as_deref(), Some("The input string 'x' was not in a correct format."));
    assert_eq!(details.source.as_deref(), Some("System.Private.CoreLib"));

    // The step over from the throw lands in the user's own catch.
    session.step_over(stopped.thread).expect("the step is taken");
    let stepped = next_stop(&events);
    let frame = top_frame(&session, stepped.thread);
    assert!(frame.path().is_some_and(|path| path.ends_with("Exceptions.cs")), "{frame:?}");
    assert_eq!((frame.line + 1, frame.column + 1), (32, 3), "{frame:?}");
    session.disconnect(true);
}

/// Launches `PassengerCases` with its end's breakpoint and the exception filters given, for a case that reads the stop
/// Just My Code makes of the exception in the wrapper outside the user's code.
fn launched_past_the_wrapper(end: usize, filter: &str) -> (Session, Receiver<Event>) {
    let program = debuggee_program("PassengerCases").expect("PassengerCases is built");
    let source = source_of("PassengerCases", "Program.cs");
    let (session, events) = started_with_filters(
        &program,
        vec![(source, vec![SourceBreakpoint { line: end - 1, ..SourceBreakpoint::default() }])],
        true,
        vec![(filter.into(), None)],
    );
    (session, events)
}

#[test]
fn an_exception_caught_in_code_marked_not_the_users_stops_as_user_unhandled() {
    let _serial = one_at_a_time();
    let call = line_in("PassengerCases", "Program.cs", "Wrapper.CallWithCatch(");
    let end = line_in("PassengerCases", "Program.cs", "Console.WriteLine(\"end\");");
    let (session, events) = launched_past_the_wrapper(end, "user-unhandled");
    let stopped = stop(&events, "exception");
    let frame = top_frame(&session, stopped.thread);
    assert_eq!(frame.line + 1, call, "the stop is the user's call into the wrapper: {frame:?}");
    let info = exception_at(&session, stopped.thread);
    assert_eq!(info.break_mode, "userUnhandled");
    assert_eq!(
        info.details.as_ref().and_then(|details| details.full_type_name.as_deref()),
        Some("System.InvalidOperationException")
    );
    session.resume(stopped.thread).expect("the program goes on");
    let stopped = stop(&events, "breakpoint");
    assert_eq!(stopped.line, Some(end - 1), "then the end's own breakpoint");
    session.disconnect(true);
}

#[test]
fn a_step_from_an_exception_the_user_left_unhandled_stops_where_the_users_code_goes_on() {
    let _serial = one_at_a_time();
    let call = line_in("PassengerCases", "Program.cs", "Wrapper.CallWithCatch(");
    let end = line_in("PassengerCases", "Program.cs", "Console.WriteLine(\"end\");");
    let next = line_in("PassengerCases", "Program.cs", "Attributed.StepThrough();");
    let (session, events) = launched_past_the_wrapper(end, "user-unhandled");
    let stopped = stop(&events, "exception");
    assert_eq!(top_frame(&session, stopped.thread).line + 1, call);

    // Past the handler outside the user's code that caught it, back in the code that called it, as a step out does.
    session.step_over(stopped.thread).expect("the step is taken");
    let stepped = next_stop(&events);
    assert_eq!((stepped.reason.as_str(), top_frame(&session, stepped.thread).line + 1), ("step", call));
    session.step_over(stepped.thread).expect("the step is taken");
    let stepped = next_stop(&events);
    assert_eq!((stepped.reason.as_str(), top_frame(&session, stepped.thread).line + 1), ("step", next));
    session.disconnect(true);
}

#[test]
fn a_step_over_a_call_that_throws_and_catches_within_it_ends_at_the_next_line() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let source = source_of("PassengerCases", "Program.cs");
    let call = line_in("PassengerCases", "Program.cs", "Wrapper.CallWithCatch(");
    let next = line_in("PassengerCases", "Program.cs", "Attributed.StepThrough();");
    let (session, events) = started_with_filters(
        &program,
        vec![(source, vec![SourceBreakpoint { line: call - 1, column: Some(0), ..SourceBreakpoint::default() }])],
        true,
        Vec::new(),
    );
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert_eq!(
        (frame.line + 1, frame.column + 1),
        (call, 1),
        "at the call, not the lambda on the same line that throws"
    );
    session.step_over(stopped.thread).expect("the step is taken");
    let stepped = next_stop(&events);
    let frame = top_frame(&session, stepped.thread);
    assert_eq!(
        (stepped.reason.as_str(), frame.line + 1),
        ("step", next),
        "past the throw and catch within the wrapper"
    );
    session.disconnect(true);
}

#[test]
fn a_step_from_where_an_exception_is_thrown_goes_past_a_handler_outside_the_users_code() {
    let _serial = one_at_a_time();
    let call = line_in("PassengerCases", "Program.cs", "Wrapper.CallWithCatch(");
    let end = line_in("PassengerCases", "Program.cs", "Console.WriteLine(\"end\");");
    let (session, events) = launched_past_the_wrapper(end, "all");
    let stopped = stop(&events, "exception");
    assert_eq!(top_frame(&session, stopped.thread).line + 1, call);
    session.step_over(stopped.thread).expect("the step is taken");
    let stepped = next_stop(&events);
    assert_eq!(
        (stepped.reason.as_str(), top_frame(&session, stepped.thread).line + 1),
        ("step", call),
        "the step from the throw went past the handler outside the user's code"
    );
    session.disconnect(true);
}

#[test]
fn a_step_from_where_an_exception_is_thrown_stops_in_the_users_own_catch() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let source = source_of("PassengerCases", "Program.cs");
    let thrown = line_in("PassengerCases", "Program.cs", "try { throw new Exception(\"Message1\"); }");
    let caught = line_in("PassengerCases", "Program.cs", "catch (Exception e1)");
    let (session, events) = started_just_my_code(
        &program,
        vec![(source, vec![SourceBreakpoint { line: thrown - 1, ..SourceBreakpoint::default() }])],
        true,
    );
    let stopped = stop(&events, "breakpoint");
    session.set_exception_filters(&[("all", None)]).expect("every exception stops");
    session.resume(stopped.thread).expect("the program goes on");
    let stopped = stop(&events, "exception");
    assert_eq!(top_frame(&session, stopped.thread).line + 1, thrown, "the throw's own line");
    session.step_over(stopped.thread).expect("the step is taken");
    let stepped = next_stop(&events);
    assert_eq!(
        (stepped.reason.as_str(), top_frame(&session, stepped.thread).line + 1),
        ("step", caught),
        "the step from the throw stops in the user's own catch"
    );
    session.disconnect(true);
}

/// The stop a program comes to next, waiting as long as `within` says: a step into CoreLib decompiles a minute or more.
fn next_stop_within(events: &Receiver<Event>, within: Duration) -> rsharp_dap::Stop {
    let deadline = Instant::now() + within;
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "the program never stopped; saw {seen:?}");
        match events.recv_timeout(left) {
            Ok(Event::Stopped(stop)) => return stop,
            Ok(event) => seen.push(format!("{event:?}")),
            Err(error) => panic!("the program never stopped: {error}; saw {seen:?}"),
        }
    }
}

#[test]
fn a_step_into_a_module_without_symbols_shows_its_decompiled_code() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let file = source_of("DebuggableConsoleApp", "MyClass.cs");
    let line = line_in("DebuggableConsoleApp", "MyClass.cs", "Console.WriteLine($\"Log_MyMethod\")");
    let (session, events) = started_how(
        &program,
        vec![(file, vec![SourceBreakpoint { line: line - 1, ..SourceBreakpoint::default() }])],
        Launcher { just_my_code: false, ..Launcher::default() },
    );
    let session = session.expect("the session starts");
    let stopped = stop(&events, "breakpoint");
    assert_eq!(stopped.line, Some(line - 1));
    session.step_into(stopped.thread).expect("the step is taken");
    let stepped = next_stop_within(&events, Duration::from_secs(300));
    let frame = top_frame(&session, stepped.thread);
    assert!(frame.path().is_some_and(|path| path.ends_with("Console.cs")), "into Console's decompiled code: {frame:?}");
    assert_eq!(frame.line + 1, 1219, "the decompiled line Passenger's test names: {frame:?}");
    session.disconnect(true);
}

#[test]
fn a_breakpoint_naming_a_same_named_file_in_another_folder_does_not_bind_to_the_real_one() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let folder = debuggees().expect("the debuggees are there").join("DebuggableConsoleApp");
    let root = rsharp_core::paths::canonical(&folder.join("MyClass.cs"));
    // MyClass.cs is in the project's root; SomeFolder holds other files, none of them this one.
    let wrong = folder.join("SomeFolder").join("MyClass.cs");
    let dotted = folder.join("Namespace1").join("..").join("MyClass.cs");
    let (session, events) = started_how(
        &program,
        vec![
            (wrong.clone(), vec![SourceBreakpoint { line: 21, ..SourceBreakpoint::default() }]),
            (dotted, vec![SourceBreakpoint { line: 21, ..SourceBreakpoint::default() }]),
        ],
        Launcher::default(),
    );
    let session = session.expect("the session starts");
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    assert_eq!(
        (frame.line + 1, frame.path().as_deref()),
        (22, Some(root.as_path())),
        "the stop names the root's MyClass.cs"
    );
    let answers = session
        .set_breakpoints(&wrong, &[SourceBreakpoint { line: 21, ..SourceBreakpoint::default() }])
        .expect("the breakpoint is sent again");
    let answer = answers.first().expect("its answer");
    assert!(!answer.verified, "the other folder's name binds to nothing: {answer:?}");
    session.disconnect(true);
}

#[test]
fn a_breakpoints_hit_count_and_id_outlast_its_files_breakpoints_being_sent_again() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let file = source_of("DebuggableConsoleApp", "HitConditionClass.cs");
    let breakpoint = |line: usize, hits: Option<&str>| SourceBreakpoint {
        line: line - 1,
        hit_condition: hits.map(str::to_string),
        ..SourceBreakpoint::default()
    };
    let (session, events) = started_how(
        &program,
        vec![(file.clone(), vec![breakpoint(9, Some("==3")), breakpoint(10, Some("==5"))])],
        Launcher::default(),
    );
    let session = session.expect("the session starts");
    let first = stop(&events, "breakpoint");
    assert_eq!(first.line, Some(8), "the third pass stops at line 9");
    let frame = top_frame(&session, first.thread);
    assert_eq!(eval(&session, frame.id, "_count").result, "2", "line 10's breakpoint is two of its five hits in");

    let added = session
        .set_breakpoints(&file, &[breakpoint(9, Some("==3")), breakpoint(10, Some("==5")), breakpoint(11, None)])
        .expect("the file's breakpoints are sent again");
    session.resume(first.thread).expect("the program goes on");
    let after = stop(&events, "breakpoint");
    assert_eq!(after.line, Some(10), "the breakpoint added at line 11");
    let removed =
        session.set_breakpoints(&file, &[breakpoint(10, Some("==5"))]).expect("line 9's and 11's are taken away");
    session.resume(after.thread).expect("the program goes on");
    let fifth = stop(&events, "breakpoint");
    assert_eq!(fifth.line, Some(9), "line 10's fifth pass");
    let frame = top_frame(&session, fifth.thread);
    assert_eq!(eval(&session, frame.id, "_count").result, "5");
    assert_eq!(added.get(1).and_then(|answer| answer.id), removed.first().and_then(|answer| answer.id));
    assert!(added.first().and_then(|answer| answer.id).is_some());
    assert!(added.get(2).is_some_and(|answer| answer.verified), "the breakpoint added while paused binds");
    session.disconnect(true);
}

#[test]
fn stack_frames_are_numbered_on_from_stop_to_stop_with_no_symbols_frames_subtle() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let first = source_of("DebuggableConsoleApp", "ClassWithBclCall.cs");
    let second = source_of("DebuggableConsoleApp", "MyClassContainingAnotherClass.cs");
    let third = source_of("DebuggableConsoleApp", "Namespace1").join("AnotherClass.cs");
    let (session, events) = started_how(
        &program,
        vec![(first.clone(), vec![SourceBreakpoint { line: 11, ..SourceBreakpoint::default() }])],
        Launcher::default(),
    );
    let session = session.expect("the session starts");
    let stopped = stop(&events, "breakpoint");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let names: Vec<&str> = frames.iter().map(|frame| frame.name.as_str()).collect();
    assert_eq!(names.len(), 5, "{names:?}");
    assert_eq!(
        names,
        [
            "DebuggableConsoleApp.dll!DebuggableConsoleApp.ClassWithBclCall.Selector(int x)",
            "System.Linq.dll!System.Linq.Enumerable.RangeSelectIterator<int, int>.Fill(System.Span<int> results, int start, System.Func<int, int> func)",
            "System.Linq.dll!System.Linq.Enumerable.RangeSelectIterator<int, int>.ToArray()",
            "DebuggableConsoleApp.dll!DebuggableConsoleApp.ClassWithBclCall.Test(int myParam)",
            "DebuggableConsoleApp.dll!DebuggableConsoleApp.Program.Main(string[] args)",
        ]
    );
    assert_eq!(frames.iter().map(|frame| frame.id).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
    assert_eq!(frames.iter().map(|frame| frame.line).collect::<Vec<_>>(), [11, 0, 0, 6, 30]);
    assert_eq!(
        frames.iter().map(|frame| frame.is_resolved == Some(true)).collect::<Vec<_>>(),
        [true, false, false, true, true]
    );
    assert_eq!(
        frames.iter().map(|frame| frame.presentation_hint.as_deref()).collect::<Vec<_>>(),
        [None, Some("subtle"), Some("subtle"), None, None]
    );
    assert!(frames[1..3].iter().all(|frame| frame.path().is_none()), "the library's frames have no source");

    session.set_breakpoints(&first, &[]).expect("the first file's are taken away");
    session
        .set_breakpoints(&second, &[SourceBreakpoint { line: 18, ..SourceBreakpoint::default() }])
        .expect("the next file's are set");
    session.resume(stopped.thread).expect("the program goes on");
    let stopped = stop(&events, "breakpoint");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    assert_eq!(frames.iter().map(|frame| frame.id).collect::<Vec<_>>(), [6, 7], "numbered on from the first stop");
    assert_eq!(frames[0].line + 1, 19);

    session.set_breakpoints(&second, &[]).expect("the second file's are taken away");
    session
        .set_breakpoints(&third, &[SourceBreakpoint { line: 19, ..SourceBreakpoint::default() }])
        .expect("the async case's are set");
    session.resume(stopped.thread).expect("the program goes on");
    let stopped = stop(&events, "breakpoint");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let user: Vec<&str> = frames
        .iter()
        .map(|frame| frame.name.as_str())
        .filter(|name| name.starts_with("DebuggableConsoleApp.dll!"))
        .collect();
    assert!(
        user.contains(&"DebuggableConsoleApp.dll!DebuggableConsoleApp.Namespace1.AnotherClass.AnotherMethodAsync()"),
        "{user:?}"
    );
    assert!(
        user.contains(&"DebuggableConsoleApp.dll!DebuggableConsoleApp.MyAsyncClass.MyMethodAsync(int myParam)"),
        "{user:?}"
    );
    assert!(!user.iter().any(|name| name.contains("MoveNext")), "{user:?}");
    session.disconnect(true);
}

#[test]
fn a_stop_in_a_method_emitted_code_calls_has_its_stack_down_to_main_through_the_emitted_frame() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let source = source_of("PassengerCases", "Program.cs");
    let called_back = line_in("PassengerCases", "Program.cs", "Console.WriteLine($\"called back with {value}\");");
    let (session, events) = started_how(
        &program,
        vec![(source, vec![SourceBreakpoint { line: called_back - 1, ..SourceBreakpoint::default() }])],
        Launcher { arguments: vec!["emitted".into()], ..Launcher::default() },
    );
    let session = session.expect("the session starts");
    let stopped = stop(&events, "breakpoint");
    let frames = session.stack_trace(stopped.thread).expect("the stack reads");
    let names: Vec<&str> = frames.iter().map(|frame| frame.name.as_str()).collect();
    assert!(names.contains(&"PassengerCases.dll!Program.<Main>$(string[] args)"), "the stack reaches Main: {names:?}");
    assert_eq!(names[0], "PassengerCases.dll!Emitted.CalledBack(int value)");
    assert!(names[1].ends_with("!EmittedCaller.CallBack(int)"), "{names:?}");
    assert_eq!(names[2], "PassengerCases.dll!Emitted.Run()");

    let arguments = locals_of(&session, &frames[1]);
    assert_eq!(arguments.len(), 1, "the emitted frame's argument, without a name: {arguments:?}");
    assert_eq!((arguments[0].name.as_str(), arguments[0].value.as_str()), ("arg_0", "7"));
    session.step_out(stopped.thread).expect("the step out is taken");
    let stepped = next_stop(&events);
    let frame = top_frame(&session, stepped.thread);
    assert_eq!(
        (stepped.reason.as_str(), frame.line + 1),
        ("step", line_in("PassengerCases", "Program.cs", "callBack(7);"))
    );
    session.disconnect(true);
}

#[test]
fn a_lambda_watch_evaluates_as_quickly_with_a_large_lambda_as_with_a_small_one() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, _events, frame) =
        stopped_at(&program, "DebuggableConsoleApp", "MyClass.cs", "Console.WriteLine($\"Log_MyMethod\")");
    assert_eq!(evaluate(&session, frame, "_intList.Where(x => x > 2).Count()"), "3");
    let started = Instant::now();
    assert_eq!(evaluate(&session, frame, "_intList.Where(x => x > myInt).Count()"), "2");
    let small = started.elapsed();
    let text = "x".repeat(32 * 1024);
    let started = Instant::now();
    assert_eq!(
        evaluate(&session, frame, &format!("_intList.Where(x => \"{text}\".Length > x).Count()")),
        "4",
        "a 32 KB string 64 KB larger than the small lambda still answers"
    );
    let large = started.elapsed();
    assert!(
        large < small + Duration::from_secs(1),
        "a lambda 64 KB larger took {large:?}, where a small one took {small:?}: its code went into the program a byte at a time"
    );
    session.disconnect(true);
}

#[test]
fn an_expression_that_loops_without_end_is_stopped_saying_it_took_too_long() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, _events, frame) =
        stopped_at(&program, "DebuggableConsoleApp", "MyClass.cs", "Console.WriteLine($\"Log_MyMethod\")");
    // The first LINQ call loads LINQ into the program, as the watch that comes before this one does.
    assert_eq!(evaluate(&session, frame, "_intList.Where(x => x > 2).Count()"), "3");
    let evaluating = session.evaluate_within(
        "Array.IndexOf<long>([.. System.Linq.Enumerable.Repeat(myInt, int.MaxValue)], 6L)",
        frame,
        Duration::from_secs(60),
    );
    let stuck = evaluating.expect("the debugger answers even when it could not stop the expression");
    assert!(
        stuck.result.contains("The expression took too long to evaluate"),
        "an expression that never ends is stopped, not waited for: {:?}",
        stuck.result
    );
    assert_eq!(evaluate(&session, frame, "myInt + 1"), "5", "the program is as it was");
    session.disconnect(true);
}

/// A frame's one scope's variables, as the debugger lists them.
fn scope_variables(session: &Session, frame: i64) -> Vec<rsharp_dap::protocol::Variable> {
    let scopes = session.scopes(frame).expect("the scopes read");
    assert_eq!(scopes.len(), 1, "one scope: {scopes:?}");
    session.variables(scopes[0].variables_reference).expect("the variables read")
}

/// What a list of variables says, as (name, value, type) triples.
fn variables_as(actual: &[rsharp_dap::protocol::Variable]) -> Vec<(String, String, String)> {
    actual
        .iter()
        .map(|variable| (variable.name.clone(), variable.value.clone(), variable.r#type.clone().unwrap_or_default()))
        .collect()
}

fn assert_variables(actual: &[rsharp_dap::protocol::Variable], expected: &[(&str, &str, &str)]) {
    // The debugger lists a method's locals in its own order; what it says of each is what matters.
    let mut got = variables_as(actual);
    let mut want: Vec<(String, String, String)> = expected
        .iter()
        .map(|(name, value, r#type)| ((*name).to_string(), (*value).to_string(), (*r#type).to_string()))
        .collect();
    got.sort();
    want.sort();
    assert_eq!(got, want);
}

fn member<'a>(variables: &'a [rsharp_dap::protocol::Variable], name: &str) -> &'a rsharp_dap::protocol::Variable {
    variables.iter().find(|variable| variable.name == name).unwrap_or_else(|| panic!("{name} is listed: {variables:?}"))
}

fn nested(
    session: &Session,
    variables: &[rsharp_dap::protocol::Variable],
    name: &str,
) -> Vec<rsharp_dap::protocol::Variable> {
    let parent = member(variables, name);
    session.variables(parent.variables_reference).unwrap_or_else(|why| panic!("{name}'s members read: {why}"))
}

#[test]
fn a_sync_methods_variables_are_its_eleven_locals_and_their_members() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let file = source_of("DebuggableConsoleApp", "MyClass.cs");
    let (session, events) = started_how(
        &program,
        vec![(file, vec![SourceBreakpoint { line: 21, ..SourceBreakpoint::default() }])],
        Launcher::default(),
    );
    let session = session.expect("the session starts");
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    let locals = scope_variables(&session, frame.id);
    let expected = [
        ("this", "{DebuggableConsoleApp.MyClass}", "DebuggableConsoleApp.MyClass"),
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
    assert_variables(&locals, &expected);
    for variable in &locals {
        assert_eq!(variable.evaluate_name.as_deref(), Some(variable.name.as_str()), "each local names itself");
    }

    assert_variables(
        &nested(&session, &locals, "structVar"),
        &[("Id", "5", "int"), ("Name", "\"StructName\"", "string")],
    );

    let instance = nested(&session, &locals, "this");
    assert_variables(
        &instance,
        &[
            ("_name", "\"TestName\"", "string"),
            ("_classField", "{DebuggableConsoleApp.MyClass3}", "DebuggableConsoleApp.MyClass3"),
            ("_recordField", "MyRecord1 { X = 1, Y = 2 }", "DebuggableConsoleApp.MyRecord1"),
            ("ClassProperty", "{DebuggableConsoleApp.MyClass2}", "DebuggableConsoleApp.MyClass2"),
            ("ClassProperty2", "{DebuggableConsoleApp.MyClass2}", "DebuggableConsoleApp.MyClass2"),
            ("_intList", "Count = 4", "System.Collections.Generic.List<int>"),
            ("_intArray", "int[4]", "int[]"),
            ("_instanceField", "5", "int"),
            ("_structField", "{DebuggableConsoleApp.MyStruct}", "DebuggableConsoleApp.MyStruct"),
            ("IntProperty", "10", "int"),
            ("_classWithDebugDisplay", "IntProperty = 14", "DebuggableConsoleApp.ClassWithDebugDisplay"),
            ("_classWithDebugDisplay2", "Test = stringValue1", "DebuggableConsoleApp.ClassWithDebugDisplay2"),
            ("_classWithDebugDisplay3", "Test = stringValue2", "DebuggableConsoleApp.ClassWithDebugDisplay3"),
            (
                "_myClassWithGeneric",
                "{DebuggableConsoleApp.MyClassWithGeneric<int>}",
                "DebuggableConsoleApp.MyClassWithGeneric<int>",
            ),
            ("_intDictionary", "Count = 3", "System.Collections.Generic.Dictionary<int, int>"),
            ("FieldFromBase", "42", "int"),
            ("PropertyFromBase", "84", "int"),
            ("_doubler", "{System.Func<int, int>}", "System.Func<int, int>"),
            ("Static members", "", ""),
        ],
    );
    assert_variables(
        &nested(&session, &instance, "_intArray"),
        &[("[0]", "2", "int"), ("[1]", "3", "int"), ("[2]", "5", "int"), ("[3]", "7", "int")],
    );
    let static_members = nested(&session, &instance, "Static members");
    assert_eq!(static_members.len(), 10, "the statics: {static_members:?}");
    let without_times: Vec<rsharp_dap::protocol::Variable> = static_members
        .iter()
        .filter(|variable| variable.name != "_utcNow" && variable.name != "_nullableUtcNow")
        .cloned()
        .collect();
    assert_variables(
        &without_times,
        &[
            ("_counter", "1", "int"),
            ("IntStaticProperty", "10", "int"),
            ("StaticClassProperty", "{DebuggableConsoleApp.MyClass2}", "DebuggableConsoleApp.MyClass2"),
            ("_staticClassField", "{DebuggableConsoleApp.MyClass2}", "DebuggableConsoleApp.MyClass2"),
            ("_staticIntList", "Count = 4", "System.Collections.Generic.List<int>"),
            (
                "_fieldDictionary",
                "Count = 0",
                "System.Collections.Generic.Dictionary<DebuggableConsoleApp.MyClass2, DebuggableConsoleApp.MyClass>",
            ),
            ("_instanceStaticField", "6", "int"),
            ("StaticFieldFromBase", "168", "int"),
        ],
    );
    for (name, r#type) in [("_utcNow", "System.DateTime"), ("_nullableUtcNow", "System.DateTime?")] {
        let time = member(&static_members, name);
        assert_eq!(time.r#type.as_deref(), Some(r#type), "{name}");
        assert!(!time.value.is_empty(), "{name} shows its time");
    }
    assert_variables(
        &nested(&session, &instance, "_intList"),
        &[("[0]", "1", "int"), ("[1]", "4", "int"), ("[2]", "8", "int"), ("[3]", "25", "int"), ("Raw View", "", "")],
    );
    assert_variables(
        &nested(&session, &instance, "_intDictionary"),
        &[
            ("[5]", "50", "System.Collections.Generic.DebugViewDictionaryItem<int, int>"),
            ("[10]", "100", "System.Collections.Generic.DebugViewDictionaryItem<int, int>"),
            ("[15]", "150", "System.Collections.Generic.DebugViewDictionaryItem<int, int>"),
            ("Raw View", "", ""),
        ],
    );
    assert_variables(
        &nested(&session, &instance, "_classField"),
        &[
            ("IntField", "6", "int"),
            ("IntProperty", "6", "int"),
            ("MyProperty", "\"Hello\"", "string"),
            (
                "NestedClassProperty",
                "{DebuggableConsoleApp.MyClassContainingAnotherClass.MyNestedClass}",
                "DebuggableConsoleApp.MyClassContainingAnotherClass.MyNestedClass",
            ),
            (
                "NestedGenericClassProperty",
                "{DebuggableConsoleApp.MyGenericClassContainingAnotherGenericClass<string, int>.MyNestedGenericClass<long, float>}",
                "DebuggableConsoleApp.MyGenericClassContainingAnotherGenericClass<string, int>.MyNestedGenericClass<long, float>",
            ),
        ],
    );
    assert_variables(
        &nested(&session, &instance, "ClassProperty"),
        &[("IntField", "6", "int"), ("IntProperty", "6", "int"), ("MyProperty", "\"Hello\"", "string")],
    );
    assert_variables(
        &nested(&session, &instance, "_recordField"),
        &[
            ("EqualityContract", "DebuggableConsoleApp.MyRecord1", "System.RuntimeType"),
            ("X", "1", "int"),
            ("Y", "2", "int"),
        ],
    );

    let enum_variables = nested(&session, &locals, "enumVar");
    assert_variables(&enum_variables, &[("Static members", "", ""), ("value__", "1", "int")]);
    assert_variables(
        &nested(&session, &enum_variables, "Static members"),
        &[
            ("FirstValue", "FirstValue", "DebuggableConsoleApp.MyEnum"),
            ("SecondValue", "SecondValue", "DebuggableConsoleApp.MyEnum"),
            ("ThirdValue", "ThirdValue", "DebuggableConsoleApp.MyEnum"),
        ],
    );

    session.resume(stopped.thread).expect("the program goes on");
    let again = stop(&events, "breakpoint");
    let frame = top_frame(&session, again.thread);
    assert_variables(&scope_variables(&session, frame.id), &expected);
    session.disconnect(true);
}

#[test]
fn a_variables_classs_locals_and_its_members_are_listed_with_what_they_hold() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let file = source_of("DebuggableConsoleApp", "VariablesClass.cs");
    let (session, events) = started_how(
        &program,
        vec![(file, vec![SourceBreakpoint { line: 149, ..SourceBreakpoint::default() }])],
        Launcher::default(),
    );
    let session = session.expect("the session starts");
    let stopped = stop(&events, "breakpoint");
    assert_eq!(stopped.line, Some(149));
    let frame = top_frame(&session, stopped.thread);
    assert_eq!((frame.line + 1, frame.column + 1), (150, 3));
    let locals = scope_variables(&session, frame.id);
    assert_eq!(locals.len(), 39, "the method's locals: {locals:?}");

    let date_time = member(&locals, "localDateTime");
    assert_eq!(date_time.r#type.as_deref(), Some("System.DateTime"));
    assert!(date_time.value.contains("2026") && date_time.value.contains("42:39"), "{date_time:?}");
    let unusual = ["localDateTime", "localFloat", "localDouble"];
    let usual: Vec<rsharp_dap::protocol::Variable> =
        locals.iter().filter(|variable| !unusual.contains(&variable.name.as_str())).cloned().collect();
    assert_variables(
        &usual,
        &[
            ("this", "{DebuggableConsoleApp.VariablesClass}", "DebuggableConsoleApp.VariablesClass"),
            ("localBool", "true", "bool"),
            ("localByte", "1", "byte"),
            ("localSByte", "-1", "sbyte"),
            ("localShort", "-2", "short"),
            ("localUShort", "2", "ushort"),
            ("localInt", "3", "int"),
            ("localUInt", "4", "uint"),
            ("localLong", "5", "long"),
            ("localULong", "6", "ulong"),
            ("localChar", "90 'Z'", "char"),
            ("localDecimal", "3.5", "decimal"),
            ("localNullableInt", "123", "int?"),
            ("localNullableIntNull", "null", "int?"),
            ("localNullableDecimal", "2.5", "decimal?"),
            ("localNullableDecimalNull", "null", "decimal?"),
            ("localString", "\"hello\"", "string"),
            ("localNullableString", "null", "string"),
            ("localObject", "{object}", "object"),
            ("localNullableObject", "null", "object"),
            ("localBoxedInt", "42", "int"),
            ("localArray", "int[3]", "int[]"),
            ("localList", "Count = 2", "System.Collections.Generic.List<string>"),
            ("localDictionary", "Count = 1", "System.Collections.Generic.Dictionary<int, string>"),
            ("localStruct", "{DebuggableConsoleApp.TestStruct}", "DebuggableConsoleApp.TestStruct"),
            ("localClass", "{DebuggableConsoleApp.TestClass}", "DebuggableConsoleApp.TestClass"),
            ("localRecord", "TestRecord { Name = record, Age = 1 }", "DebuggableConsoleApp.TestRecord"),
            ("localInterface", "{DebuggableConsoleApp.TestClass}", "DebuggableConsoleApp.TestClass"),
            ("localDelegate", "{System.Func<int, int>}", "System.Func<int, int>"),
            ("localTuple", "(1, \"stringInTuple\")", "System.Tuple<int, string>"),
            ("localValueTuple", "(2, \"stringInValueTuple\")", "System.ValueTuple<int, string>"),
            ("localGeneric", "{DebuggableConsoleApp.GenericBox<int>}", "DebuggableConsoleApp.GenericBox<int>"),
            ("localDynamic", "241", "int"),
            ("localAnonymous", "{ Id = 1, Name = \"Anonymous\" }", "<>f__AnonymousType0<int, string>"),
            ("localGuid", "27de5b68-af24-4e59-a785-dde52e2ea7af", "System.Guid"),
            (
                "localCompositeValues",
                "{DebuggableConsoleApp.CompositeValueFixtures}",
                "DebuggableConsoleApp.CompositeValueFixtures",
            ),
        ],
    );

    // A float and a double show their decimal separator as the machine's culture writes it.
    for (name, r#type, expected) in [("localFloat", "float", "1.5"), ("localDouble", "double", "2.5")] {
        let shown = member(&locals, name);
        assert_eq!(shown.r#type.as_deref(), Some(r#type), "{name}");
        assert_eq!(shown.value.replace(',', "."), expected, "{name}");
    }

    let instance = nested(&session, &locals, "this");
    assert_eq!(instance.len(), 56, "the instance's members: {instance:?}");
    let special = [
        "DateTimeField",
        "DateOnlyField",
        "TimeOnlyField",
        "TimeSpanField",
        "ThrowingProperty",
        "_stringFieldWithNewLine",
        "FloatField",
        "DoubleField",
    ];
    let plain: Vec<rsharp_dap::protocol::Variable> =
        instance.iter().filter(|variable| !special.contains(&variable.name.as_str())).cloned().collect();
    assert_variables(
        &plain,
        &[
            ("BoolField", "true", "bool"),
            ("ByteField", "1", "byte"),
            ("SByteField", "-1", "sbyte"),
            ("ShortField", "-2", "short"),
            ("UShortField", "2", "ushort"),
            ("IntField", "123", "int"),
            ("UIntField", "123", "uint"),
            ("LongField", "123456789", "long"),
            ("ULongField", "123456789", "ulong"),
            ("CharField", "65 'A'", "char"),
            ("DecimalField", "3.45", "decimal"),
            ("NullableIntField", "42", "int?"),
            ("NullableIntNullField", "null", "int?"),
            ("NullableBoolField", "true", "bool?"),
            ("NullableBoolNullField", "null", "bool?"),
            ("NullableGuidField", "f0e1d2c3-b4a5-9687-7869-5a4b3c2d1e0f", "System.Guid?"),
            ("NullableEnumField", "Friday", "System.DayOfWeek?"),
            ("NullableEnumNullField", "null", "System.DayOfWeek?"),
            ("StringField", "\"Hello\"", "string"),
            ("NullableStringField", "null", "string"),
            ("ObjectField", "{object}", "object"),
            ("NullableObjectField", "null", "object"),
            ("IntArrayField", "int[3]", "int[]"),
            ("NullableStringArrayField", "string[3]", "string[]"),
            ("MultiDimArrayField", "int[2, 5]", "int[,]"),
            ("JaggedArrayField", "int[2][]", "int[][]"),
            ("ListField", "Count = 3", "System.Collections.Generic.List<int>"),
            ("DictionaryField", "Count = 2", "System.Collections.Generic.Dictionary<string, int>"),
            ("GuidField", "a1b2c3d4-e5f6-7890-abcd-ef1234567890", "System.Guid"),
            ("EnumField", "Monday", "System.DayOfWeek"),
            ("StructField", "{DebuggableConsoleApp.TestStruct}", "DebuggableConsoleApp.TestStruct"),
            ("ClassField", "{DebuggableConsoleApp.TestClass}", "DebuggableConsoleApp.TestClass"),
            ("RecordField", "TestRecord { Name = Alice, Age = 42 }", "DebuggableConsoleApp.TestRecord"),
            ("RecordStructField", "TestRecordStruct { Value = 7 }", "DebuggableConsoleApp.TestRecordStruct"),
            ("InterfaceField", "{DebuggableConsoleApp.TestClass}", "DebuggableConsoleApp.TestClass"),
            ("DelegateField", "{System.Func<int, int>}", "System.Func<int, int>"),
            ("TupleField", "(1, \"tuple\")", "System.Tuple<int, string>"),
            ("ValueTupleField", "(123, \"value tuple\")", "System.ValueTuple<int, string>"),
            ("GenericField", "{DebuggableConsoleApp.GenericBox<string>}", "DebuggableConsoleApp.GenericBox<string>"),
            ("DynamicField", "\"dynamic value\"", "string"),
            ("ReadonlyField", "\"readonly\"", "string"),
            ("IntProperty", "100", "int"),
            ("NullableStringProperty", "null", "string"),
            (
                "_genericTypeWithStaticField",
                "{DebuggableConsoleApp.GenericTypeWithStaticField<string>}",
                "DebuggableConsoleApp.GenericTypeWithStaticField<string>",
            ),
            ("ClassProperty", "{DebuggableConsoleApp.TestClass}", "DebuggableConsoleApp.TestClass"),
            ("RecordProperty", "TestRecord { Name = InitProperty, Age = 5 }", "DebuggableConsoleApp.TestRecord"),
            ("ComputedProperty", "246", "int"),
            ("Static members", "", ""),
        ],
    );
    for (name, r#type, expected) in [("FloatField", "float", "1.23"), ("DoubleField", "double", "2.34")] {
        let shown = member(&instance, name);
        assert_eq!(shown.r#type.as_deref(), Some(r#type), "{name}");
        assert_eq!(shown.value.replace(',', "."), expected, "{name}");
    }
    for (name, r#type) in [
        ("DateTimeField", "System.DateTime"),
        ("DateOnlyField", "System.DateOnly"),
        ("TimeOnlyField", "System.TimeOnly"),
        ("TimeSpanField", "System.TimeSpan"),
    ] {
        let shown = member(&instance, name);
        assert_eq!(shown.r#type.as_deref(), Some(r#type), "{name}");
        assert!(!shown.value.is_empty(), "{name} shows its value");
    }
    let throwing = member(&instance, "ThrowingProperty");
    assert_eq!(throwing.r#type.as_deref(), Some("System.InvalidOperationException"));
    assert!(throwing.value.contains("ThrowingProperty was accessed"), "{throwing:?}");
    let new_line = member(&instance, "_stringFieldWithNewLine");
    assert!(new_line.value.contains("Test") && new_line.value.contains("quoted"), "{new_line:?}");

    assert_variables(&nested(&session, &instance, "_genericTypeWithStaticField"), &[("Static members", "", "")]);
    let strong = nested(&session, &instance, "_genericTypeWithStaticField");
    let statics = nested(&session, &strong, "Static members");
    assert_variables(&statics, &[("IntValue", "4", "int"), ("Value", "null", "string"), ("IntProperty", "5", "int")]);
    let rows = nested(&session, &instance, "MultiDimArrayField");
    assert_variables(&rows, &[("[0, ...]", "", ""), ("[1, ...]", "", "")]);
    assert_variables(
        &nested(&session, &rows, "[0, ...]"),
        &[
            ("[0, 0]", "1", "int"),
            ("[0, 1]", "2", "int"),
            ("[0, 2]", "3", "int"),
            ("[0, 3]", "4", "int"),
            ("[0, 4]", "5", "int"),
        ],
    );
    let composite = nested(&session, &locals, "localCompositeValues");
    assert_eq!(composite.len(), 12, "the composite values: {composite:?}");
    let ordinary: Vec<rsharp_dap::protocol::Variable> = composite
        .iter()
        .filter(|variable| {
            variable.name != "FailedMember" && variable.name != "EscapedString" && variable.name != "Anonymous"
        })
        .cloned()
        .collect();
    assert_variables(
        &ordinary,
        &[
            ("NullAndEmptyStrings", "(1, null, \"\", \"null\")", "System.Tuple<int, string, string, string>"),
            ("NestedTuple", "(1, (2, \"nested\"))", "System.ValueTuple<int, System.ValueTuple<int, string>>"),
            (
                "LongValueTuple",
                "(1, 2, 3, 4, 5, 6, 7, \"eight\", \"nine\")",
                "System.ValueTuple<int, int, int, int, int, int, int, System.ValueTuple<string, string>>",
            ),
            (
                "LongTuple",
                "(1, 2, 3, 4, 5, 6, 7, \"eight\")",
                "System.Tuple<int, int, int, int, int, int, int, System.Tuple<string>>",
            ),
            ("EmptyTuple", "()", "System.ValueTuple"),
            ("NullableTuple", "(4, \"nullable\")", "System.ValueTuple<int, string>?"),
            ("NullTuple", "null", "System.ValueTuple<int, string>?"),
            ("NullableDebuggerDisplay", "display: 5", "DebuggableConsoleApp.NullableDebuggerDisplay?"),
            ("NullableToString", "to-string: 6", "DebuggableConsoleApp.NullableToString?"),
        ],
    );
    assert!(member(&composite, "FailedMember").value.contains("<error:"), "a display that fails is shown as the error");
    assert!(member(&composite, "EscapedString").value.contains("a\\n\\\"b\\\\c"), "the escaped string is escaped");
    assert!(
        member(&composite, "Anonymous").value.contains("Text =")
            && member(&composite, "Anonymous").value.contains("Nested =")
    );
    session.disconnect(true);
}

#[test]
fn a_continue_runs_every_thread_of_the_program_and_the_evaluations_late_end_is_let_go() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let source = source_of("PassengerCases", "Program.cs");
    let before = line_in("PassengerCases", "Program.cs", "Console.WriteLine($\"before {before}\");");
    let worker = line_in("PassengerCases", "Program.cs", "Console.WriteLine(\"worker ran\");");
    let (session, events) = started_how(
        &program,
        vec![(
            source,
            vec![
                SourceBreakpoint { line: before - 1, ..SourceBreakpoint::default() },
                SourceBreakpoint { line: worker - 1, ..SourceBreakpoint::default() },
            ],
        )],
        Launcher { arguments: vec!["reads".into()], ..Launcher::default() },
    );
    let session = session.expect("the session starts");
    let stopped = stop(&events, "breakpoint");
    let frame = top_frame(&session, stopped.thread);
    let stuck = session
        .evaluate_within("Reads.Input", frame.id, Duration::from_secs(60))
        .expect("the debugger answers an evaluation it could not stop");
    assert!(stuck.result.contains("could not be stopped"), "the getter blocked on input: {stuck:?}");

    // The continue runs the program's other thread, whose own breakpoint stops it; the evaluation's read ends late.
    session.resume(stopped.thread).expect("the program goes on");
    let worker_stop = stop(&events, "breakpoint");
    assert_eq!(worker_stop.line, Some(worker - 1));
    assert_ne!(worker_stop.thread, stopped.thread, "the continue let the worker thread run");
    session.input_with("typed\n", true).expect("the input is sent and closed");
    session.resume(worker_stop.thread).expect("the program goes on");
    let (output, exit_code) = wait_for_end(&events);
    assert!(output.contains("before 1"), "the program said what it did: {output}");
    assert!(output.contains("read none"), "its own read found its input at its end: {output}");
    assert_eq!(exit_code, Some(0));
    session.disconnect(false);
}

#[test]
fn an_exception_stop_is_still_described_once_an_evaluation_there_could_not_be_stopped() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("PassengerCases") else { return };
    let source = source_of("PassengerCases", "Program.cs");
    let end = line_in("PassengerCases", "Program.cs", "Console.WriteLine(\"end\");");
    let (session, events) = started_with_filters(
        &program,
        vec![(source, vec![SourceBreakpoint { line: end - 1, ..SourceBreakpoint::default() }])],
        true,
        vec![("all".into(), None), ("user-unhandled".into(), None)],
    );
    let stopped = stop(&events, "exception");
    let frame = top_frame(&session, stopped.thread);
    let stuck = session
        .evaluate_within("Reads.Input", frame.id, Duration::from_secs(60))
        .expect("the debugger answers an evaluation it could not stop");
    assert!(stuck.result.contains("could not be stopped"), "the getter blocked on input: {stuck:?}");

    // Read from the exception's own fields: no code runs on the thread while the evaluation does.
    let info = exception_at(&session, stopped.thread);
    let details = info.details.as_ref().expect("the exception's details");
    assert_eq!(details.full_type_name.as_deref(), Some("System.InvalidOperationException"));
    assert_eq!(details.message.as_deref(), Some("thrown in user code"));
    assert!(
        info.description.as_deref().is_some_and(|description| description.contains("Implicit evaluation is disabled")),
        "the client is told why the fields were read instead: {:?}",
        info.description
    );
    let locals = locals_of(&session, &frame);
    assert_eq!(locals.iter().filter(|variable| variable.name == "$exception").count(), 1, "{locals:?}");
    session.disconnect(true);
}

#[test]
fn an_unsupported_request_answers_an_error_without_stopping_the_dispatcher() {
    let _serial = one_at_a_time();
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let (session, events) = started_how(&program, Vec::new(), Launcher::default());
    let session = session.expect("the session starts");
    let unimplemented = session
        .request_raw("setVariable", serde_json::json!({ "variablesReference": 0, "name": "value", "value": "1" }))
        .expect_err("setVariable is not one the adapter takes");
    assert_eq!(unimplemented, "Request 'setVariable' is not supported.");
    let unknown =
        session.request_raw("unknownRequest", serde_json::json!({})).expect_err("an unknown request is refused");
    assert_eq!(unknown, "Request 'unknownRequest' is not supported.");

    // The dispatcher goes on: the debugger answers a breakpoint and stops the program at it.
    let file = source_of("DebuggableConsoleApp", "MyClass.cs");
    session
        .set_breakpoints(&file, &[SourceBreakpoint { line: 21, ..SourceBreakpoint::default() }])
        .expect("the debugger answers after the refusals");
    let stopped = stop(&events, "breakpoint");
    assert_eq!(stopped.line, Some(21));
    session.disconnect(true);
}

/// Passenger's StartHold: a runtime held at its start by a diagnostic port the test owns, so a launch waits for it.
/// The port is a Unix socket on macOS and Linux, and a named pipe on Windows.
#[cfg(unix)]
struct StartHold {
    path: PathBuf,
    listener: std::os::unix::net::UnixListener,
}

#[cfg(unix)]
impl StartHold {
    /// A hold at a socket short enough for its path, or none when the temp folder's path leaves no room for one.
    fn new() -> Option<Self> {
        let path = std::env::temp_dir().join(format!("rsharp-hold-{}", std::process::id()));
        if path.as_os_str().len() > 100 {
            return None;
        }
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).ok()?;
        Some(Self { path, listener })
    }

    fn environment(&self) -> BTreeMap<String, String> {
        BTreeMap::from([("DOTNET_DiagnosticPorts".to_string(), format!("{},connect,suspend", self.path.display()))])
    }

    /// The runtime's connection and its process id, once it has come and waits.
    fn accept(&self) -> (Held, i64) {
        let (stream, _) = self.listener.accept().expect("the runtime connects to the hold");
        let mut stream = Held(stream);
        // "ADVR_V1\0", the cookie, its process's id and two bytes kept for later.
        let mut advertised = [0u8; 34];
        stream.read_exact(&mut advertised);
        let pid = u64::from_le_bytes(advertised[24..32].try_into().expect("the id is 8 bytes")) as i64;
        (stream, pid)
    }
}

#[cfg(unix)]
impl Drop for StartHold {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The hold's named pipe on Windows, in the pipe namespace a runtime's diagnostic port understands.
#[cfg(windows)]
struct StartHold {
    name: String,
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
unsafe impl Send for StartHold {}

#[cfg(windows)]
impl StartHold {
    fn new() -> Option<Self> {
        use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
        use windows_sys::Win32::System::Pipes::{
            CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        };
        let name = format!(r"\\.\pipe\rsharp-hold-{}", std::process::id());
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                4096,
                0,
                std::ptr::null(),
            )
        };
        if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            return None;
        }
        Some(Self { name, handle })
    }

    fn environment(&self) -> BTreeMap<String, String> {
        BTreeMap::from([("DOTNET_DiagnosticPorts".to_string(), format!("{},connect,suspend", self.name))])
    }

    /// The runtime's connection and its process id, once it has come and waits.
    fn accept(&self) -> (Held, i64) {
        use windows_sys::Win32::System::Pipes::ConnectNamedPipe;
        unsafe {
            ConnectNamedPipe(self.handle, std::ptr::null_mut());
        }
        let mut held = Held { handle: self.handle };
        let mut advertised = [0u8; 34];
        held.read_exact(&mut advertised);
        let pid = u64::from_le_bytes(advertised[24..32].try_into().expect("the id is 8 bytes")) as i64;
        (held, pid)
    }
}

#[cfg(windows)]
impl Drop for StartHold {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

/// The held runtime's connection, read and written as a stream on either system.
#[cfg(unix)]
struct Held(std::os::unix::net::UnixStream);

#[cfg(unix)]
impl Held {
    fn read_exact(&mut self, into: &mut [u8]) {
        use std::io::Read;
        self.0.read_exact(into).expect("the runtime says what it is told");
    }

    fn read_one(&mut self) {
        use std::io::Read;
        let mut byte = [0u8; 1];
        let _ = self.0.read(&mut byte);
    }

    fn write_all(&mut self, from: &[u8]) {
        use std::io::Write;
        self.0.write_all(from).expect("the runtime is told");
    }
}

#[cfg(windows)]
struct Held {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
unsafe impl Send for Held {}

#[cfg(windows)]
impl Held {
    fn read_exact(&mut self, into: &mut [u8]) {
        use windows_sys::Win32::Storage::FileSystem::ReadFile;
        let mut read = 0u32;
        let ok =
            unsafe { ReadFile(self.handle, into.as_mut_ptr(), into.len() as u32, &mut read, std::ptr::null_mut()) };
        assert!(ok != 0 && read as usize == into.len(), "the runtime says what it is told");
    }

    fn read_one(&mut self) {
        // A closed pipe is the read's end, not a failure: the session ended as it should.
        use windows_sys::Win32::Storage::FileSystem::ReadFile;
        let mut byte = [0u8; 1];
        let mut read = 0u32;
        unsafe {
            ReadFile(self.handle, byte.as_mut_ptr(), 1, &mut read, std::ptr::null_mut());
        }
    }

    fn write_all(&mut self, from: &[u8]) {
        use windows_sys::Win32::Storage::FileSystem::WriteFile;
        let mut written = 0u32;
        let ok =
            unsafe { WriteFile(self.handle, from.as_ptr(), from.len() as u32, &mut written, std::ptr::null_mut()) };
        assert!(ok != 0 && written as usize == from.len(), "the runtime is told");
    }
}

/// Lets the held runtime go: the diagnostic protocol's header and its ResumeRuntime command, then its answer.
fn let_go(held: &mut Held) {
    let mut resume = b"DOTNET_IPC_V1\0".to_vec();
    resume.extend_from_slice(&[20, 0, 0x04, 0x01, 0, 0]);
    held.write_all(&resume);
    let mut answer = [0u8; 24];
    held.read_exact(&mut answer);
}

#[test]
fn a_program_whose_runtime_starts_late_is_still_debugged() {
    let _serial = one_at_a_time();
    let Some(hold) = StartHold::new() else {
        eprintln!("skipped: the temp folder's path leaves no room for a diagnostic socket");
        return;
    };
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let environment = hold.environment();
    let held = std::thread::spawn(move || {
        let (mut stream, _pid) = hold.accept();
        // Six seconds, past the five a debugger once gave a runtime to answer.
        std::thread::sleep(Duration::from_secs(6));
        let_go(&mut stream);
    });
    let file = source_of("DebuggableConsoleApp", "MyClass.cs");
    let began = Instant::now();
    let (session, events) = started_how(
        &program,
        vec![(file, vec![SourceBreakpoint { line: 21, ..SourceBreakpoint::default() }])],
        Launcher { environment, ..Launcher::default() },
    );
    let session = session.expect("the debugger waits for the program's runtime, however late");
    assert!(began.elapsed() >= Duration::from_secs(6), "the launch waited {:?}", began.elapsed());
    let stopped = stop(&events, "breakpoint");
    assert_eq!(stopped.line, Some(21), "the late program stopped at its breakpoint");
    held.join().expect("the hold lets go");
    session.disconnect(true);
}

#[test]
fn a_launch_stopped_while_its_runtime_starts_ends_at_once_with_its_program() {
    let _serial = one_at_a_time();
    let Some(hold) = StartHold::new() else {
        eprintln!("skipped: the temp folder's path leaves no room for a diagnostic socket");
        return;
    };
    let Some(program) = debuggee_program("DebuggableConsoleApp") else { return };
    let Some(adapter) = adapter() else { return };
    let environment = hold.environment();
    let (pid_tx, pid_rx) = mpsc::channel();
    let held = std::thread::spawn(move || {
        let (mut stream, pid) = hold.accept();
        let _ = pid_tx.send(pid);
        // The runtime is never let go: the session's end closes the connection instead.
        stream.read_one();
    });
    let (control_tx, control_rx) = mpsc::channel();
    let starting = std::thread::spawn(move || {
        Session::start_watched(
            &adapter,
            Start::Launch(Launch {
                program: program.clone(),
                arguments: Vec::new(),
                working_directory: program.parent().expect("the program is in a folder").to_path_buf(),
                environment,
                stop_at_entry: false,
                hot_reload: true,
                just_my_code: true,
            }),
            &std::env::temp_dir().join("rsharp-passenger-symbols"),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            |_| {},
            move |control| {
                let _ = control_tx.send(control);
            },
        )
    });
    let control =
        control_rx.recv_timeout(Duration::from_secs(60)).expect("the session hands its handle while starting");
    let pid = pid_rx.recv_timeout(Duration::from_secs(60)).expect("the runtime is held");
    // The adapter may have ended the conversation already; the launch's own answer says how it went.
    let _ = control.stop();
    let why = match starting.join().expect("the start ends") {
        Err(why) => why,
        Ok(_) => panic!("the launch fails when stopped while its runtime starts"),
    };
    assert!(
        why.contains("the session ended before its runtime told the debugger it had started"),
        "the failure says the session ended first: {why}"
    );
    // The program goes with the session that was stopped as it started.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut runs = true;
    while Instant::now() < deadline {
        runs = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .is_ok_and(|status| status.success());
        if !runs {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    held.join().expect("the hold's socket closes with the program");
    assert!(!runs, "the program (process {pid}) outlived the stopped session");
}
