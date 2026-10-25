//! The client against the real adapter and a real program: a breakpoint stops it, its frame and values are read, a step
//! moves it on, evaluate answers, and the program writes its output.

use rsharp_dap::{Event, ExceptionFilter, Launch, Session, SourceBreakpoint, Start, adapter};
use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const PROGRAM: &str = "using System;\n\nclass Program\n{\n    static void Main()\n    {\n        int a = 41;\n        int b = a + 1;\n        Console.WriteLine($\"answer {b}\");\n    }\n}\n";

#[test]
fn a_breakpoint_stops_the_program_and_its_values_are_read() {
    let Some(adapter) = adapter() else {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    };
    let Some(dotnet) = rsharp_core::programs::dotnet() else {
        eprintln!("skipped: dotnet is not installed");
        return;
    };
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let folder = std::env::temp_dir().join(format!("rsharp-dap-test-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("App.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework><OutputType>Exe</OutputType></PropertyGroup></Project>",
    )
    .unwrap();
    let source = folder.join("Program.cs");
    std::fs::write(&source, PROGRAM).unwrap();
    let built = folder.join("bin");
    let output = std::process::Command::new(&dotnet)
        .args(["build", "-c", "Debug", "-o"])
        .arg(&built)
        .arg("--nologo")
        .arg("-v")
        .arg("q")
        .current_dir(&folder)
        .output()
        .expect("dotnet build runs");
    assert!(
        output.status.success(),
        "the test's program did not build:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let assembly = built.join("App.dll");
    let source = rsharp_core::paths::canonical(&source);
    let symbols = folder.join("cache").join("symbols");
    let (sender, events) = mpsc::channel();
    let session = Session::start(
        &adapter,
        Start::Launch(Launch {
            program: assembly,
            arguments: Vec::new(),
            working_directory: built,
            environment: BTreeMap::new(),
            stop_at_entry: false,
        }),
        &symbols,
        vec![(
            source.clone(),
            vec![SourceBreakpoint { line: 7, hit_condition: Some("1".into()), ..SourceBreakpoint::default() }],
        )],
        // Every exception nothing handles stops the program, as Rider's Any exception does.
        vec![ExceptionFilter { scenarios: vec!["unhandled".into()], ..ExceptionFilter::default() }],
        move |event| drop(sender.send(event)),
    )
    .expect("the session starts");

    let (stop, seen) = wait_for_stop(&events);
    assert_eq!(stop.reason, "breakpoint", "the program stopped at its breakpoint");
    assert!(
        seen.iter().any(|event| matches!(event, Event::Breakpoint { verified: true, line: Some(7), .. })),
        "the adapter said the breakpoint was bound before it stopped there: {seen:?}"
    );
    assert_eq!(stop.path.as_deref(), Some(source.as_path()), "the stop names the file");
    assert_eq!(stop.line, Some(7), "the breakpoint's line, counted from 0");

    let frames = session.stack_trace(stop.thread).expect("the stack is read");
    assert_eq!(frames[0].path().as_deref(), Some(source.as_path()), "the innermost frame is the breakpoint's file");
    assert!(frames[0].short_name().contains("Main"), "the frame is Program.Main: {:?}", frames[0].short_name());

    let scopes = session.scopes(frames[0].id).expect("the scopes are read");
    let locals = scopes.iter().find(|scope| scope.name == "Locals").expect("a Locals scope");
    let values = session.variables(locals.variables_reference).expect("the locals are read");
    let a = values.iter().find(|variable| variable.name == "a").expect("the local a");
    assert_eq!(a.value, "41", "a is 41 at the breakpoint");
    assert_eq!(session.evaluate("a * 2", frames[0].id).expect("evaluate answers").result, "82");

    let changed = session.set_expression("a", "10", frames[0].id).expect("a is set");
    assert_eq!(changed.value, "10", "the debugger says what it set");
    assert_eq!(session.evaluate("a", frames[0].id).expect("evaluate answers").result, "10", "a is 10 now");
    session.set_expression("a", "41", frames[0].id).expect("a is set back");

    session.step_over(stop.thread).expect("the step is taken");
    let (after, _) = wait_for_stop(&events);
    assert_eq!(after.reason, "step", "the program stopped after the step");
    assert_eq!(after.line, Some(8), "the step moved to the next statement");
    let here = session.stack_trace(after.thread).expect("the stack is read after the step");
    assert_eq!(session.evaluate("b", here[0].id).expect("evaluate answers").result, "42");

    let targets = session.goto_targets(&source, 6).expect("the goto targets answer");
    assert!(!targets.is_empty(), "Set Next Statement has a place to go to on the assignment's line");
    session.set_next_statement(after.thread, targets[0].id).expect("the thread goes to it");
    let moved = session.stack_trace(after.thread).expect("the stack is read after Set Next Statement");
    assert_eq!(moved[0].line, 6, "Set Next Statement moved the thread back to the assignment's line");

    session.resume(stop.thread).expect("the program goes on");
    let wrote = wait_for_output(&events);
    assert!(wrote.contains("answer 42"), "the program's output is told of: {wrote:?}");

    session.disconnect(true);
    let _ = std::fs::remove_dir_all(&folder);
}

fn wait_for_stop(events: &Receiver<Event>) -> (rsharp_dap::Stop, Vec<Event>) {
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "the program never stopped");
        match events.recv_timeout(left) {
            Ok(Event::Stopped(stop)) => return (stop, seen),
            Ok(Event::Ended) => panic!("the debugger ended before the program stopped"),
            Ok(event) => seen.push(event),
            Err(error) => panic!("the program never stopped: {error}"),
        }
    }
}

fn wait_for_output(events: &Receiver<Event>) -> String {
    let mut wrote = String::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        match events.recv_timeout(Duration::from_millis(500)) {
            Ok(Event::Output { text, .. }) => wrote.push_str(&text),
            Ok(Event::Ended) => break,
            Ok(_) => {}
            Err(_) => {}
        }
        if wrote.contains("answer 42") {
            break;
        }
    }
    wrote
}
