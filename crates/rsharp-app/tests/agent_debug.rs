//! An agent debugging through the window: it sets a breakpoint, starts the program under the debugger, reads where it
//! stopped, evaluates an expression there, steps, and lets the program run to its end.

mod support;

use gpui::{TestAppContext, VisualTestContext};
use serde_json::{Value, json};
use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, Instant};
use support::Run;

const EXE_PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>";

/// Line 7 sets `a`, line 8 adds to it, line 9 writes the answer.
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

#[gpui::test]
fn an_agent_debugs_a_program_stopping_reading_evaluating_stepping_and_running_on(cx: &mut TestAppContext) {
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new("agent-debug", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", PROGRAM);
    if !offline(&run) {
        return;
    }
    unsafe { std::env::set_var("RSHARP_AGENTS", std::env::temp_dir().join(format!("rsa-{}", std::process::id()))) };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| {
        let mut on = rsharp_app::settings::settings(cx).clone();
        on.agents_enabled = true;
        rsharp_app::settings::apply(on, cx);
    });
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_agent_socket(cx);
    });
    cx.run_until_parked();
    let r = agent(
        cx,
        &run.folder,
        &[
            call(1, "breakpoint_set", json!({ "file": "App/Program.cs", "line": 8 })),
            call(2, "breakpoints", json!({})),
            call(3, "debug_start", json!({})),
            call(4, "debug_evaluate", json!({ "expression": "a + 1" })),
            call(5, "debug_step", json!({ "how": "over" })),
            call(6, "debug_variables", json!({})),
            call(7, "debug_continue", json!({})),
            call(8, "debug_output", json!({})),
            call(9, "debug_continue", json!({})),
        ],
    );
    assert!(text(&r[1]).contains("\"line\":8"), "breakpoints: {}", r[1]);
    let stopped: Value = serde_json::from_str(&text(&r[2])).unwrap_or_else(|_| panic!("debug_start: {}", r[2]));
    assert_eq!(stopped["phase"], "paused", "the program did not stop at the breakpoint: {stopped}");
    assert!(text(&r[2]).contains("Program.cs:8"), "the stop is not at line 8: {stopped}");
    assert!(text(&r[3]).contains("42"), "a + 1 is not 42: {}", r[3]);
    assert!(text(&r[5]).contains("\"b\"") && text(&r[5]).contains("42"), "after the step b is not 42: {}", r[5]);
    assert!(text(&r[7]).contains("answer 42"), "the program's output: {}", r[7]);
    assert_eq!(r[8]["result"]["isError"], true, "a program that has ended was continued: {}", r[8]);
}
