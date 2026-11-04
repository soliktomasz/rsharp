//! An agent in the window: `--mcp-attach`'s server joins the window that has its folder over the window's own socket,
//! is refused until Enable MCP Server is on, and then works the editors as the person does.

mod support;

use gpui::{TestAppContext, VisualTestContext};
use serde_json::{Value, json};
use std::io::Cursor;
use std::time::{Duration, Instant};
use support::Run;

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
    let deadline = Instant::now() + Duration::from_secs(30);
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
fn an_agent_is_refused_until_the_person_enables_mcp_then_opens_files_in_the_window(cx: &mut TestAppContext) {
    let run = Run::new("agents", &[("Program.cs", "class P { }\n")]);
    // A socket folder of this test's own, so a person's RSharp never answers it.
    // Short, as macOS allows a socket's path 104 bytes.
    unsafe { std::env::set_var("RSHARP_AGENTS", std::env::temp_dir().join(format!("rsa-{}", std::process::id()))) };
    let (main, cx) = run.window(cx);
    let folder = run.folder.clone();
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&folder, window, cx);
        main.open_agent_socket(cx);
    });

    let listed = agent(cx, &run.folder, &[json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })]);
    let names: Vec<&str> =
        listed[0]["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(
        names.contains(&"open_file") && names.contains(&"diagnostics"),
        "the tool list is missing tools: {names:?}"
    );

    let refused = agent(cx, &run.folder, &[call(1, "workspace", json!({}))]);
    assert!(
        refused[0]["result"]["isError"] == true && text(&refused[0]).contains("Enable MCP Server"),
        "an agent was let in with Enable MCP Server off, or not told why not: {}",
        refused[0]
    );

    cx.update(|_, cx| {
        let mut on = rsharp_app::settings::settings(cx).clone();
        on.agents_enabled = true;
        rsharp_app::settings::apply(on, cx);
    });
    let worked = agent(
        cx,
        &run.folder,
        &[call(1, "workspace", json!({})), call(2, "open_file", json!({ "file": "Program.cs", "line": 1 }))],
    );
    assert!(
        text(&worked[0]).contains("Program.cs") || text(&worked[0]).contains(run.folder.to_str().unwrap()),
        "workspace: {}",
        worked[0]
    );
    assert_eq!(worked[1]["result"]["isError"], false, "open_file failed: {}", worked[1]);
    let open = main.read_with(cx, |m, cx| m.editor().map(|e| e.read(cx).path.clone()));
    assert_eq!(open.as_deref().and_then(|p| p.file_name()).and_then(|n| n.to_str()), Some("Program.cs"));

    let elsewhere = tempfile_folder();
    let none = agent(cx, &elsewhere, &[call(1, "workspace", json!({}))]);
    assert!(text(&none[0]).contains("No RSharp window has"), "a folder no window has: {}", none[0]);
}

fn tempfile_folder() -> std::path::PathBuf {
    let folder = std::env::temp_dir().join(format!("rsharp-agents-elsewhere-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    folder
}
