//! An agent building, running, testing, stashing and running to a line through the window.

mod support;

use gpui::{TestAppContext, VisualTestContext};
use serde_json::{Value, json};
use std::io::Cursor;
use std::path::Path;
use std::process::{Command, Stdio};
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

fn git(folder: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command
        .current_dir(folder)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = rsharp_core::process::spawn(command).unwrap();
    let mut stdout = String::new();
    std::io::Read::read_to_string(&mut child.process.stdout.take().unwrap(), &mut stdout).unwrap();
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.process.stderr.take().unwrap(), &mut stderr).unwrap();
    assert!(child.wait().unwrap().success(), "git {args:?}: {stderr}");
    stdout
}

/// A repository whose local configuration keeps commits independent of the person's Git setup.
fn init(folder: &Path) {
    git(folder, &["init", "-b", "main"]);
    git(folder, &["config", "user.name", "Test"]);
    git(folder, &["config", "user.email", "test@example.invalid"]);
    git(folder, &["config", "commit.gpgsign", "false"]);
    git(folder, &["config", "core.hooksPath", "disabled-hooks"]);
    // The fixtures' bytes are the files' bytes on every system: Windows' default would check them out as CRLF.
    git(folder, &["config", "core.autocrlf", "false"]);
}

fn enable(cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        let mut on = rsharp_app::settings::settings(cx).clone();
        on.agents_enabled = true;
        rsharp_app::settings::apply(on, cx);
    });
}

#[gpui::test]
fn an_agent_builds_reads_errors_runs_a_program_and_reads_its_output(cx: &mut TestAppContext) {
    let run = Run::new("agent-build-run", &[]);
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", "System.Console.WriteLine(\"built and run\");\n");
    if !offline(&run) {
        return;
    }
    unsafe { std::env::set_var("RSHARP_AGENTS", std::env::temp_dir().join(format!("rsa-{}", std::process::id()))) };
    let (main, cx) = run.window(cx);
    enable(cx);
    main.update_in(cx, |m, w, cx| {
        m.open_workspace(&run.folder, w, cx);
        m.open_agent_socket(cx);
    });
    cx.run_until_parked();
    let ok = agent(
        cx,
        &run.folder,
        &[call(1, "build", json!({})), call(2, "run", json!({ "waitMs": 60000 })), call(3, "run_output", json!({}))],
    );
    let built: Value = serde_json::from_str(&text(&ok[0])).unwrap_or_else(|_| panic!("build: {}", ok[0]));
    assert_eq!(built["status"], "Succeeded", "the build: {built}");
    assert!(text(&ok[1]).contains("built and run"), "run: {}", ok[1]);
    assert!(text(&ok[2]).contains("built and run"), "run_output: {}", ok[2]);

    write(&run.folder, "App/Program.cs", "System.Console.WriteLine(\"x\")\n");
    let bad = agent(cx, &run.folder, &[call(1, "build", json!({}))]);
    let failed: Value = serde_json::from_str(&text(&bad[0])).unwrap_or_else(|_| panic!("build: {}", bad[0]));
    assert_ne!(failed["status"], "Succeeded", "a build of a syntax error succeeded: {failed}");
    assert!(
        failed["problems"].as_array().is_some_and(|p| p.iter().any(|p| p["code"] == "CS1002")),
        "the missing semicolon is not among the problems: {failed}"
    );
}

#[gpui::test]
fn an_agent_stops_a_running_program_and_stashes_changes(cx: &mut TestAppContext) {
    let run = Run::new("agent-stop-stash", &[("a.txt", "one\n")]);
    init(&run.folder);
    git(&run.folder, &["add", "."]);
    git(&run.folder, &["commit", "-m", "initial"]);
    std::fs::write(run.folder.join("a.txt"), "two\n").unwrap();
    unsafe { std::env::set_var("RSHARP_AGENTS", std::env::temp_dir().join(format!("rsa-{}", std::process::id()))) };
    let (main, cx) = run.window(cx);
    enable(cx);
    main.update_in(cx, |m, w, cx| {
        m.open_workspace(&run.folder, w, cx);
        m.open_agent_socket(cx);
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    while main.read_with(cx, |m, _| m.git_status().is_none()) {
        assert!(Instant::now() < deadline, "the window never read the repository");
        cx.executor().advance_clock(Duration::from_millis(150));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
    let r = agent(
        cx,
        &run.folder,
        &[
            call(1, "vcs_stash", json!({ "message": "set aside" })),
            call(2, "vcs_stash", json!({ "action": "list" })),
            call(3, "vcs_stash", json!({ "action": "pop" })),
            call(4, "stop", json!({})),
        ],
    );
    assert_eq!(r[0]["result"]["isError"], false, "stash push: {}", r[0]);
    assert!(text(&r[1]).contains("set aside"), "the stash is not listed: {}", r[1]);
    assert_eq!(r[2]["result"]["isError"], false, "stash pop: {}", r[2]);
    assert_eq!(
        std::fs::read_to_string(run.folder.join("a.txt")).unwrap(),
        "two\n",
        "the popped stash did not bring the change back"
    );
    assert!(text(&r[3]).starts_with("Stopped"), "stop: {}", r[3]);
}

#[gpui::test]
fn an_agent_lists_source_tests_and_runs_to_a_line(cx: &mut TestAppContext) {
    if rsharp_dap::adapter().is_none() {
        eprintln!("skipped: Passenger's debug adapter is not built");
        return;
    }
    let run = Run::new(
        "agent-run-to",
        &[
            (
                "Shop.slnx",
                "<Solution><Project Path=\"App/App.csproj\" /><Project Path=\"Tests/Tests.csproj\" /></Solution>",
            ),
            ("App/App.csproj", EXE_PROJECT),
            ("App/Program.cs", PROGRAM),
            (
                "Tests/Tests.csproj",
                "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework><IsTestProject>true</IsTestProject></PropertyGroup></Project>",
            ),
            (
                "Tests/Orders.cs",
                "namespace Shop;\npublic class Orders\n{\n    [Fact]\n    public void Keeps() { }\n}\n",
            ),
        ],
    );
    if !offline(&run) {
        return;
    }
    unsafe { std::env::set_var("RSHARP_AGENTS", std::env::temp_dir().join(format!("rsa-{}", std::process::id()))) };
    let (main, cx) = run.window(cx);
    enable(cx);
    main.update_in(cx, |m, w, cx| {
        m.open_workspace(&run.folder, w, cx);
        m.set_evaluated(
            rsharp_core::project::EvaluatedProject {
                path: run.folder.join("Tests/Tests.csproj"),
                is_test_project: true,
                ..Default::default()
            },
            cx,
        );
        m.open_agent_socket(cx);
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    while main.read_with(cx, |m, cx| m.test_view().read(cx).tests().is_empty()) {
        assert!(Instant::now() < deadline, "the window never found the source tests");
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
    let r = agent(
        cx,
        &run.folder,
        &[
            call(1, "tests_list", json!({ "filter": "keeps" })),
            call(2, "breakpoint_set", json!({ "file": "App/Program.cs", "line": 7 })),
            call(3, "debug_start", json!({ "configuration": "App" })),
            call(4, "debug_run_to", json!({ "file": "App/Program.cs", "line": 9 })),
            call(5, "breakpoints", json!({})),
            call(6, "debug_stop", json!({})),
        ],
    );
    assert!(text(&r[0]).contains("Shop.Orders.Keeps") && text(&r[0]).contains("notrun"), "tests_list: {}", r[0]);
    assert!(text(&r[3]).contains("Program.cs:9"), "debug_run_to did not stop at line 9: {}", r[3]);
    assert!(!text(&r[4]).contains("\"line\":9"), "the temporary breakpoint was left behind: {}", r[4]);
}
