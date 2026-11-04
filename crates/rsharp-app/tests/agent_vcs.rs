//! An agent working with Git and NuGet through the window: the status, a diff, the log, a commit and a new branch, and a
//! package search that fails cleanly where there are no feeds.

mod support;

use gpui::{TestAppContext, VisualTestContext};
use serde_json::{Value, json};
use std::io::Cursor;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use support::Run;

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
fn an_agent_reads_status_and_diff_commits_and_branches(cx: &mut TestAppContext) {
    let run = Run::new("agent-vcs", &[("a.cs", "class A\n{\n    int n;\n}\n")]);
    init(&run.folder);
    git(&run.folder, &["add", "."]);
    git(&run.folder, &["commit", "-m", "initial"]);
    std::fs::write(run.folder.join("a.cs"), "class A\n{\n    int n;\n    int m;\n}\n").unwrap();
    unsafe { std::env::set_var("RSHARP_AGENTS", std::env::temp_dir().join(format!("rsa-{}", std::process::id()))) };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| {
        let mut on = rsharp_app::settings::settings(cx).clone();
        on.agents_enabled = true;
        rsharp_app::settings::apply(on, cx);
    });
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
            call(1, "vcs_status", json!({})),
            call(2, "vcs_diff", json!({ "file": "a.cs" })),
            call(3, "vcs_commit", json!({ "message": "add m" })),
            call(4, "vcs_log", json!({ "limit": 5 })),
            call(5, "vcs_checkout", json!({ "branch": "feature", "create": true })),
            call(6, "vcs_status", json!({})),
            call(7, "nuget_install", json!({ "project": "none.csproj", "package": "X", "version": "1.0.0" })),
        ],
    );
    let status: Value = serde_json::from_str(&text(&r[0])).unwrap_or_else(|_| panic!("vcs_status: {}", r[0]));
    assert_eq!(status["branch"], "main");
    assert_eq!(status["changes"][0]["file"], "a.cs", "status: {status}");
    assert!(text(&r[1]).contains("+4:     int m;"), "the diff misses the added line: {}", r[1]);
    assert_eq!(r[2]["result"]["isError"], false, "vcs_commit: {}", r[2]);
    assert!(text(&r[3]).contains("add m") && text(&r[3]).contains("initial"), "log: {}", r[3]);
    assert_eq!(r[4]["result"]["isError"], false, "vcs_checkout: {}", r[4]);
    assert_eq!(
        serde_json::from_str::<Value>(&text(&r[5])).unwrap()["branch"],
        "feature",
        "status after the checkout: {}",
        r[5]
    );
    assert_eq!(git(&run.folder, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "feature");
    assert_eq!(r[6]["result"]["isError"], true, "nuget_install on a project that is not there");
}
