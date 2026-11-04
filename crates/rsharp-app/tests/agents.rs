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
    agent_answering(cx, folder, requests, None)
}

/// The same, the person answering the window's question about letting the agent in with `answer` when it is asked.
fn agent_answering(
    cx: &mut VisualTestContext,
    folder: &std::path::Path,
    requests: &[Value],
    answer: Option<&'static str>,
) -> Vec<Value> {
    unsafe { std::env::set_var("RSHARP_NO_START", "1") };
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
        if let (Some(answer), Some(at)) = (answer, cx.debug_bounds("choice dialog")) {
            let _ = at;
            let selector = match answer {
                "Allow" => "choice: Allow",
                "Always for this folder" => "choice: Always for this folder",
                _ => "choice: No",
            };
            if let Some(button) = cx.debug_bounds(selector) {
                cx.simulate_click(button.center(), gpui::Modifiers::default());
            }
        }
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

    // Nobody has enabled agents: the person is asked, and a No is the call's answer.
    let refused = agent_answering(cx, &run.folder, &[call(1, "workspace", json!({}))], Some("No"));
    assert!(
        refused[0]["result"]["isError"] == true && text(&refused[0]).contains("did not let"),
        "an agent was let in against a No, or not told why not: {}",
        refused[0]
    );
    // Always for this folder lets it in, and the folder is kept so the next agent is not asked.
    let always = agent_answering(cx, &run.folder, &[call(1, "workspace", json!({}))], Some("Always for this folder"));
    assert_eq!(always[0]["result"]["isError"], false, "Always did not let the agent in: {}", always[0]);
    let kept = cx.update(|_, cx| rsharp_app::settings::settings(cx).agent_folders.clone());
    assert!(
        kept.contains(run.folder.to_str().unwrap().trim_start_matches("/private")) || !kept.is_empty(),
        "the folder was not kept: {kept:?}"
    );
    let again = agent(cx, &run.folder, &[call(1, "workspace", json!({}))]);
    assert_eq!(
        again[0]["result"]["isError"], false,
        "an agent was asked again for a folder always let in: {}",
        again[0]
    );
    cx.update(|_, cx| {
        let mut off = rsharp_app::settings::settings(cx).clone();
        off.agent_folders = String::new();
        // Enable MCP Server lets every agent in unasked.
        off.agents_enabled = true;
        rsharp_app::settings::apply(off, cx);
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

#[gpui::test]
fn an_agent_reads_edits_and_searches_the_files_and_changes_a_setting(cx: &mut TestAppContext) {
    let run = Run::new(
        "agents-files",
        &[("A.cs", "class Alpha { int one = 1; int two = 1; }\n"), ("B.cs", "class Beta { }\n")],
    );
    unsafe { std::env::set_var("RSHARP_AGENTS", std::env::temp_dir().join(format!("rsa-{}", std::process::id()))) };
    let (main, cx) = run.window(cx);
    let folder = run.folder.clone();
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&folder, window, cx);
        main.open_agent_socket(cx);
    });
    cx.update(|_, cx| {
        let mut on = rsharp_app::settings::settings(cx).clone();
        on.agents_enabled = true;
        rsharp_app::settings::apply(on, cx);
    });
    let replies = agent(
        cx,
        &run.folder,
        &[
            call(1, "editor_text", json!({ "file": "A.cs" })),
            call(2, "edit", json!({ "file": "A.cs", "oldText": "= 1;", "newText": "= 2;" })),
            call(3, "edit", json!({ "file": "A.cs", "oldText": "int one = 1;", "newText": "int one = 5;" })),
            call(4, "find_in_files", json!({ "text": "class", "mask": "*.cs" })),
            call(5, "replace_in_files", json!({ "text": "Beta", "replacement": "Gamma" })),
            call(6, "settings_set", json!({ "changes": { "noSuchSetting": 1 } })),
            call(7, "settings_set", json!({ "changes": { "editorFontSize": 15 } })),
            call(8, "settings_set", json!({ "changes": { "agentsEnabled": false } })),
            call(9, "editor_text", json!({ "file": "B.cs" })),
        ],
    );
    assert!(text(&replies[0]).starts_with("1: class Alpha"), "editor_text: {}", replies[0]);
    assert_eq!(replies[1]["result"]["isError"], true, "an edit matching twice was made: {}", replies[1]);
    assert!(text(&replies[1]).contains("2 times"), "the refusal does not say why: {}", replies[1]);
    assert_eq!(replies[2]["result"]["isError"], false, "the unique edit failed: {}", replies[2]);
    assert_eq!(
        std::fs::read_to_string(run.folder.join("A.cs")).unwrap(),
        "class Alpha { int one = 5; int two = 1; }\n",
        "the edit was not saved"
    );
    let found: Value = serde_json::from_str(&text(&replies[3])).unwrap();
    assert_eq!(found["count"], 2, "find_in_files: {found}");
    assert!(text(&replies[4]).contains("Replaced 1 matches in 1 files"), "replace_in_files: {}", replies[4]);
    assert!(text(&replies[8]).contains("Gamma"), "the replaced text is not in the editor's: {}", replies[8]);
    assert_eq!(replies[5]["result"]["isError"], true, "an unknown setting was accepted");
    assert!(text(&replies[6]).contains("15"), "editorFontSize: {}", replies[6]);
    assert_eq!(replies[7]["result"]["isError"], true, "an agent turned off its own admission");
    assert_eq!(cx.update(|_, cx| rsharp_app::settings::settings(cx).editor_font_size), 15.0);
}

const PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework><OutputType>Exe</OutputType></PropertyGroup></Project>"#;

#[gpui::test]
fn an_agent_asks_the_windows_roslyn_for_problems_symbols_usages_and_a_rename(cx: &mut TestAppContext) {
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let run = Run::new(
        "agents-language",
        &[
            ("Demo.csproj", PROJECT),
            (
                "Greeter.cs",
                "namespace Demo;\n\npublic class Greeter\n{\n    public string Greet(string name) => \"Hello \" + name;\n}\n",
            ),
            (
                "Program.cs",
                "namespace Demo;\n\npublic static class Program\n{\n    public static void Main()\n    {\n        var greeter = new Greeter();\n        System.Console.WriteLine(greeter.Greet(\"you\"));\n    }\n}\n",
            ),
            ("Broken.cs", "namespace Demo;\n\nclass Broken\n{\n    int M() => \"no\";\n}\n"),
        ],
    );
    unsafe { std::env::set_var("RSHARP_AGENTS", std::env::temp_dir().join(format!("rsa-{}", std::process::id()))) };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| {
        cx.global_mut::<rsharp_app::state::Ide>().servers = true;
        let mut on = rsharp_app::settings::settings(cx).clone();
        on.agents_enabled = true;
        rsharp_app::settings::apply(on, cx);
    });
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_agent_socket(cx);
    });
    let replies = agent(
        cx,
        &run.folder,
        &[
            call(1, "diagnostics", json!({ "file": "Broken.cs" })),
            call(2, "find_symbol", json!({ "query": "Greeter" })),
            call(3, "outline", json!({ "file": "Greeter.cs" })),
            call(4, "usages", json!({ "file": "Greeter.cs", "line": 5, "column": 19 })),
            call(5, "definition", json!({ "file": "Program.cs", "line": 7, "column": 28 })),
            call(6, "rename", json!({ "file": "Greeter.cs", "line": 5, "column": 19, "newName": "Welcome" })),
            call(7, "editor_text", json!({ "file": "Program.cs" })),
        ],
    );
    assert!(text(&replies[0]).contains("error"), "Broken.cs has no error: {}", replies[0]);
    assert!(text(&replies[1]).contains("Greeter.cs"), "find_symbol: {}", replies[1]);
    assert!(text(&replies[2]).contains("Greet"), "outline: {}", replies[2]);
    assert!(text(&replies[3]).contains("Program.cs"), "usages of Greet miss its call: {}", replies[3]);
    assert!(text(&replies[4]).contains("Greeter.cs"), "definition: {}", replies[4]);
    assert!(text(&replies[5]).contains("Renamed Greet to Welcome"), "rename: {}", replies[5]);
    assert!(
        text(&replies[6]).contains("greeter.Welcome("),
        "the rename did not reach the editor's text: {}",
        replies[6]
    );
}

#[gpui::test]
fn an_agent_formats_walks_a_hierarchy_and_asks_for_code_actions(cx: &mut TestAppContext) {
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let run = Run::new(
        "agents-actions",
        &[
            ("Demo.csproj", PROJECT),
            ("Shapes.cs", "namespace Demo;\n\npublic abstract class Shape { }\n\npublic class Circle : Shape { }\n"),
            ("Loose.cs", "namespace Demo;\nclass Loose\n{\nint   x;\n}\n"),
        ],
    );
    unsafe { std::env::set_var("RSHARP_AGENTS", std::env::temp_dir().join(format!("rsa-{}", std::process::id()))) };
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| {
        cx.global_mut::<rsharp_app::state::Ide>().servers = true;
        let mut on = rsharp_app::settings::settings(cx).clone();
        on.agents_enabled = true;
        rsharp_app::settings::apply(on, cx);
    });
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_agent_socket(cx);
    });
    let replies = agent(
        cx,
        &run.folder,
        &[
            call(1, "hierarchy", json!({ "file": "Shapes.cs", "line": 3, "column": 23 })),
            call(2, "format", json!({ "file": "Loose.cs" })),
            call(3, "editor_text", json!({ "file": "Loose.cs" })),
            call(4, "code_actions", json!({ "file": "Shapes.cs", "line": 5, "column": 15 })),
            call(
                5,
                "apply_code_action",
                json!({ "file": "Shapes.cs", "line": 5, "column": 15, "title": "No such action" }),
            ),
        ],
    );
    assert!(text(&replies[0]).contains("Circle"), "Shape's subtypes miss Circle: {}", replies[0]);
    assert!(text(&replies[1]).contains("Formatted"), "format: {}", replies[1]);
    assert!(text(&replies[2]).contains("5:     int x;"), "the file was not laid out: {}", replies[2]);
    assert_eq!(replies[3]["result"]["isError"], false, "code_actions: {}", replies[3]);
    assert_eq!(replies[4]["result"]["isError"], true, "an action that is not offered was made: {}", replies[4]);
}
