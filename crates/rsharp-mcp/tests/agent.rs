//! An agent's session with RSharp's MCP server on a fresh project, against the real Roslyn: it lists the tools,
//! finds a symbol, reads a file's problems, renames a type across files (its file renamed with it) and edits a
//! file, seeing the problem its edit made. Skips, saying why, when Roslyn's server is not installed.

use serde_json::{Value, json};

const GREETER: &str =
    "namespace Demo;\n\npublic class Greeter\n{\n    public string Greet(string name) => \"Hello \" + name;\n}\n";
const PROGRAM: &str = "namespace Demo;\n\npublic static class Program\n{\n    public static void Main() => System.Console.WriteLine(new Greeter().Greet(\"you\"));\n}\n";

#[test]
fn an_agent_finds_renames_and_edits_the_code_through_the_tools() {
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    let folder = std::env::temp_dir().join(format!("rsharp-mcp-agent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("Demo.csproj"), r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework><OutputType>Exe</OutputType></PropertyGroup></Project>"#).unwrap();
    std::fs::write(folder.join("Greeter.cs"), GREETER).unwrap();
    std::fs::write(folder.join("Program.cs"), PROGRAM.replace('\n', "\r\n")).unwrap();

    let call = |id: usize, tool: &str, arguments: Value| json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": tool, "arguments": arguments } });
    let session = [
        json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } } }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        call(2, "workspace", json!({})),
        call(3, "find_symbol", json!({ "query": "Greeter" })),
        call(4, "diagnostics", json!({ "file": "Program.cs" })),
        call(5, "usages", json!({ "symbol": "Greet" })),
        call(6, "rename", json!({ "file": "Greeter.cs", "line": 3, "column": 14, "newName": "Welcomer" })),
        call(7, "edit", json!({ "file": "Program.cs", "oldText": "\"you\"", "newText": "42" })),
        call(8, "outline", json!({ "file": "Welcomer.cs" })),
    ];
    let input: String = session.iter().map(|m| format!("{m}\n")).collect();
    let mut output = Vec::new();
    rsharp_mcp::serve(&folder, input.as_bytes(), &mut output).expect("the session ran");
    let answers: Vec<Value> =
        String::from_utf8(output).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let text = |id: usize| {
        let answer = answers.iter().find(|a| a["id"] == id).unwrap_or_else(|| panic!("no answer to {id}"));
        answer["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
    };

    let names: Vec<&str> =
        answers[1]["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert_eq!(names, ["workspace", "diagnostics", "find_symbol", "outline", "usages", "rename", "edit"]);
    assert!(
        text(2).contains("Projects: Demo.csproj") && text(2).contains("Roslyn: loaded"),
        "workspace said {}",
        text(2)
    );
    assert!(text(3).contains("class Greeter") && text(3).contains("Greeter.cs:3:14"), "find_symbol said {}", text(3));
    assert_eq!(text(4), "No problems in Program.cs.");
    assert!(text(5).contains("Greeter.cs:5:19") && text(5).contains("Program.cs:5:"), "usages said {}", text(5));
    assert!(text(6).contains("Greeter.cs renamed to Welcomer.cs"), "rename said {}", text(6));
    assert!(
        !folder.join("Greeter.cs").exists() && folder.join("Welcomer.cs").is_file(),
        "the type's file was not renamed"
    );
    let program = std::fs::read_to_string(folder.join("Program.cs")).unwrap();
    assert!(
        program.contains("new Welcomer()") && program.contains("\r\n") && !program.contains("\r\r"),
        "the rename wrote {program:?}"
    );
    assert!(
        text(7).contains("Program.cs:5:") && text(7).contains("error"),
        "the edit's problem was not said: {}",
        text(7)
    );
    assert!(text(8).contains("class Welcomer (line 3)"), "outline said {}", text(8));

    let _ = std::fs::remove_dir_all(&folder);
}
