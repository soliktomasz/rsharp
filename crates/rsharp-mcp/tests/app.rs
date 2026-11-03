//! The MCP server's app tools as an agent uses them (Passenger's McpTests.AvaloniaApp): Passenger's LiveTreeApp, headless,
//! built and started by app_run, its tree read, a picture of its window drawn, a click given, and the app stopped.

use serde_json::{Value, json};
use std::path::Path;

#[test]
fn an_agent_runs_an_avalonia_app_reads_its_tree_draws_its_window_clicks_and_stops_it() {
    if rsharp_dap::inspector::carried().is_none() || !cfg!(unix) {
        eprintln!("skipped: no inspector, or no Unix socket for it");
        return;
    }
    let Some(dotnet) = rsharp_core::programs::dotnet() else {
        eprintln!("skipped: dotnet is not installed");
        return;
    };
    let folder = std::env::temp_dir().join(format!("rsharp-mcp-app-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let debuggee = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/passenger/tests/Debuggees/LiveTreeApp");
    for file in ["App.axaml", "App.axaml.cs", "MainWindow.axaml", "MainWindow.axaml.cs", "Program.cs"] {
        std::fs::copy(debuggee.join(file), folder.join(file)).unwrap();
    }
    let packages: String = [
        "Avalonia",
        "Avalonia.Themes.Fluent",
        "Avalonia.Fonts.Inter",
        "Avalonia.Headless",
        "Avalonia.HarfBuzz",
        "Avalonia.Skia",
    ]
    .iter()
    .map(|p| format!("<PackageReference Include=\"{p}\" Version=\"12.1.4\" />"))
    .collect();
    std::fs::write(folder.join("LiveTreeApp.csproj"), format!("<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings></PropertyGroup><ItemGroup>{packages}</ItemGroup></Project>")).unwrap();
    let restore = rsharp_core::dotnet::command(&dotnet, ["restore", "-v", "q"], &folder).output().unwrap();
    if !restore.status.success() {
        eprintln!("skipped: Avalonia could not be restored");
        return;
    }
    let call = |id: i64, name: &str, arguments: Value| {
        json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": name, "arguments": arguments } }).to_string()
    };
    let lines = [
        call(1, "app_run", json!({})),
        call(2, "app_tree", json!({})),
        call(3, "app_windows", json!({})),
        call(9, "app_stop", json!({})),
    ];
    let input = lines.join("\n") + "\n";
    let mut output = Vec::new();
    rsharp_mcp::serve(&folder, input.as_bytes(), &mut output).unwrap();
    let answers: Vec<Value> =
        String::from_utf8(output).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let text = |id: i64| {
        let answer = answers.iter().find(|a| a["id"] == id).unwrap_or_else(|| panic!("no answer {id}"));
        assert_eq!(answer["result"]["isError"], false, "tool {id} failed: {answer}");
        answer["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
    };
    assert!(text(1).contains("Running on Avalonia 12.1.4"), "app_run did not say the app runs: {}", text(1));
    let tree = text(2);
    assert!(
        tree.contains("Button#Save") && tree.contains("\"Save\""),
        "app_tree lacks the Save button and its text:\n{tree}"
    );
    assert!(text(3).contains("Live Tree App"), "app_windows lacks the window's title: {}", text(3));
    assert_eq!(text(9), "The app was ended.");
    let _ = std::fs::remove_dir_all(&folder);
}
