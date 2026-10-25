//! What the editor asks Roslyn, asked of the real server on a fresh project, each answer read as the editor reads
//! it. Skips, saying why, when the server is not installed.

use rsharp_lsp::*;
use serde_json::json;
use std::sync::mpsc;
use std::time::Duration;

const PROGRAM: &str = r#"using System.Text;

namespace Demo;

public interface IGreeter { string Greet(string name); }

public class Greeter : IGreeter
{
    public string Greet(string name) => "Hello " + name;
}

public static class Program
{
    public static void Main()
    {
        IGreeter greeter = new Greeter();
        var text = greeter.Greet("you");
        System.Console.WriteLine(text);
    }
}
"#;

/// Where `needle` starts in PROGRAM, as a line and column (ASCII, so UTF-16 columns are bytes).
fn find(needle: &str, nth: usize) -> Point {
    let offset = PROGRAM.match_indices(needle).nth(nth).expect("the needle is in the program").0;
    let before = &PROGRAM[..offset];
    let line = before.matches('\n').count();
    Point::new(line, offset - before.rfind('\n').map_or(0, |n| n + 1))
}

fn ask(server: &LanguageServer, method: &str, params: serde_json::Value) -> serde_json::Value {
    server
        .request(method, params)
        .recv_timeout(Duration::from_secs(60))
        .unwrap_or_else(|_| panic!("Roslyn answered {method}"))
        .unwrap_or_else(|e| panic!("Roslyn refused {method}: {e}"))
}

#[test]
fn roslyn_answers_what_the_editor_asks_in_the_form_the_editor_reads() {
    let Some(_) = roslyn_server() else {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    };
    let folder = std::env::temp_dir().join(format!("rsharp-lsp-features-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("Demo.csproj"), r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework><OutputType>Exe</OutputType><Nullable>enable</Nullable></PropertyGroup></Project>"#).unwrap();
    std::fs::write(folder.join("Program.cs"), PROGRAM).unwrap();
    std::fs::write(folder.join("Broken.cs"), "namespace Demo;\n\nclass Broken { int M() => \"no\"; }\n").unwrap();
    let folder = rsharp_core::paths::canonical(&folder);
    let file = folder.join("Program.cs");
    let uri = rsharp_core::paths::to_uri(&file);

    let (sender, events) = mpsc::channel();
    let server = start_roslyn(&folder, None, &folder.join("logs"), None, move |event| {
        let _ = sender.send(event);
    })
    .expect("Roslyn started");
    let loaded =
        std::iter::from_fn(|| events.recv_timeout(Duration::from_secs(180)).ok()).any(|e| says_projects_loaded(&e));
    assert!(loaded, "Roslyn never loaded the project");
    server.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "csharp", "version": 1, "text": PROGRAM } }),
    );

    // Go to Declaration from `new Greeter()` lands on the class's name.
    let found = locations(&ask(&server, "textDocument/definition", at(&uri, find("Greeter()", 0))));
    assert_eq!(
        found.first().map(|l| (&l.path, l.span.start)),
        Some((&file, find("Greeter :", 0))),
        "Go to Declaration found {found:?}"
    );

    // Go to Implementation from the interface finds the class.
    let found = locations(&ask(&server, "textDocument/implementation", at(&uri, find("IGreeter {", 0))));
    assert!(found.iter().any(|l| l.span.start == find("Greeter :", 0)), "Go to Implementation found {found:?}");

    // Find Usages of the interface's method: the call, with the declaration.
    let mut params = at(&uri, find("Greet(string", 0));
    params["context"] = json!({ "includeDeclaration": true });
    let found = locations(&ask(&server, "textDocument/references", params));
    assert!(found.iter().any(|l| l.span.start == find("Greet(\"you\")", 0)), "Find Usages missed the call: {found:?}");

    // The hover card names the method.
    let card = hover(&ask(&server, "textDocument/hover", at(&uri, find("Greet(\"you\")", 0)))).unwrap_or_default();
    assert!(card.contains("IGreeter.Greet(string name)"), "the hover card says {card:?}");

    // Usage highlights of a local: its declaration and its use.
    let found = highlights(&ask(&server, "textDocument/documentHighlight", at(&uri, find("text =", 0))));
    assert_eq!(found.len(), 2, "the local's usages highlighted were {found:?}");

    // Completion after `greeter.` offers the method.
    let mut params = at(&uri, find("Greet(\"you\")", 0));
    params["context"] = json!({ "triggerKind": 1 });
    let list = completion(&ask(&server, "textDocument/completion", params));
    let greet = list.items.iter().find(|i| i.label == "Greet");
    assert!(
        greet.is_some_and(|i| i.edit.is_some()),
        "completion offered {} items, Greet with an edit among them? {greet:?}",
        list.items.len()
    );

    // Parameter info inside the call.
    let mut params = at(&uri, find("\"you\"", 0));
    params["context"] = json!({ "triggerKind": 1, "isRetrigger": false });
    let shown = signature(&ask(&server, "textDocument/signatureHelp", params)).expect("a signature inside the call");
    assert_eq!(shown.parameter.clone().map(|p| &shown.label[p]), Some("name"), "parameter info showed {shown:?}");

    // Rename of the method changes the interface, the class and the call.
    let mut params = at(&uri, find("Greet(string", 0));
    params["newName"] = json!("Welcome");
    let edit = workspace_edit(&ask(&server, "textDocument/rename", params));
    let edits: usize = edit.files.iter().map(|(_, e)| e.iter().filter(|e| e.text == "Welcome").count()).sum();
    assert_eq!(edits, 3, "rename changed {edit:?}");

    // Reformat Code has something to say about a crowded line.
    let crowded = "class  A{void  M( ){}}\n";
    let other = rsharp_core::paths::to_uri(&folder.join("Crowded.cs"));
    server.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": other, "languageId": "csharp", "version": 1, "text": crowded } }),
    );
    let edits = text_edits(&ask(
        &server,
        "textDocument/formatting",
        json!({ "textDocument": { "uri": other }, "options": { "tabSize": 4, "insertSpaces": true } }),
    ));
    assert!(!edits.is_empty(), "Reformat Code changed nothing in {crowded:?}");

    // The colours over the syntax's: the interface named as one.
    let tokens = semantic_tokens(
        &ask(&server, "textDocument/semanticTokens/full", json!({ "textDocument": { "uri": uri } })),
        &token_legend(&server),
    );
    assert!(
        tokens.iter().any(|t| t.at == find("IGreeter greeter", 0) && t.kind.contains("interface")),
        "the semantic tokens were {:?}",
        &tokens[..tokens.len().min(20)]
    );

    // Code actions on the unused using offer to remove it.
    let span = Span { start: Point::new(0, 0), end: Point::new(0, 18) };
    let actions = code_actions(&ask(
        &server,
        "textDocument/codeAction",
        json!({ "textDocument": { "uri": uri }, "range": span.to_json(), "context": { "diagnostics": [] } }),
    ));
    assert!(!actions.is_empty(), "no code actions on the unused using");
    let resolved = ask(&server, "codeAction/resolve", actions[0].raw.clone());
    assert!(
        resolved.get("edit").is_some() || resolved.get("command").is_some(),
        "{} resolved to nothing: {resolved}",
        actions[0].title
    );

    // Go to Symbol finds the class by name.
    // Roslyn indexes the project's symbols after it says it has loaded, so the query is asked again meanwhile.
    let has = |found: &[Symbol]| found.iter().any(|s| s.name == "Greeter" && s.location.path == file);
    let mut found = Vec::new();
    for _ in 0..20 {
        found = symbols(&ask(&server, "workspace/symbol", json!({ "query": "Greeter" })));
        if has(&found) {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(has(&found), "Go to Symbol found {found:?}");

    // The type hierarchy: the class's supertype is the interface, and the interface's subtype the class.
    let class = hierarchy_items(&ask(&server, "textDocument/prepareTypeHierarchy", at(&uri, find("Greeter :", 0))));
    assert_eq!(class.first().map(|t| t.name.as_str()), Some("Greeter"), "prepareTypeHierarchy found {class:?}");
    let up = hierarchy_items(&ask(&server, "typeHierarchy/supertypes", json!({ "item": class[0].raw })));
    assert!(up.iter().any(|t| t.name == "IGreeter"), "the class's supertypes were {up:?}");
    let interface =
        hierarchy_items(&ask(&server, "textDocument/prepareTypeHierarchy", at(&uri, find("IGreeter {", 0))));
    let down = hierarchy_items(&ask(&server, "typeHierarchy/subtypes", json!({ "item": interface[0].raw })));
    assert!(down.iter().any(|t| t.name == "Greeter"), "the interface's subtypes were {down:?}");

    // The file's outline: the namespace's types and their members, nested.
    let shape = outline(&ask(&server, "textDocument/documentSymbol", json!({ "textDocument": { "uri": uri } })));
    assert!(
        shape
            .iter()
            .any(|(depth, name, kind, _)| name.starts_with("Greet(") && *depth > 0 && kind_name(*kind) == "method"),
        "the outline was {shape:?}"
    );

    // Inlay hints: the local's type before its name (`var string? text`), the parameter's name before a literal argument.
    let whole = Span { start: Point::new(0, 0), end: find("}\n}", 0) };
    let hints = inlay_hints(&ask(
        &server,
        "textDocument/inlayHint",
        json!({ "textDocument": { "uri": uri }, "range": whole.to_json() }),
    ));
    let at = |p: Point| hints.iter().filter(|h| h.at == p).map(|h| h.label.trim().to_string()).collect::<Vec<_>>();
    assert_eq!(at(find("text = greeter", 0)), ["string?"], "the local's type hint; all hints: {hints:?}");
    assert_eq!(at(find("\"you\"", 0)), ["name:"], "the literal argument's parameter hint; all hints: {hints:?}");

    // The Solution's problems, with solution-wide analysis on: a file not open, with its error.
    for option in SOLUTION_WIDE {
        server.set_option(option, json!("fullSolution"));
    }
    let found = workspace_problems(&ask(&server, "workspace/diagnostic", json!({ "previousResultIds": [] })));
    let broken = found.iter().find(|(p, _)| p.ends_with("Broken.cs"));
    assert!(broken.is_some_and(|(_, p)| p.iter().any(|p| p.error)), "the solution's problems were {found:?}");

    // Passenger's extension: Change Signature described at the interface's method and its parameter renamed, every
    // use following; Extract Interface described at the class.
    let dispatch = |handler: &str, message: serde_json::Value| {
        let (method, params) = extension::request(handler, &message, &uri);
        extension::answer(&ask(&server, method, params))
    };
    let described = dispatch(extension::DESCRIBE_SIGNATURE, json!({ "at": extension::at(find("Greet(string", 0)) }))
        .expect("the extension described the signature");
    assert_eq!(described["parameters"][0]["name"], "name", "Change Signature described {described}");
    let change = json!({ "at": extension::at(find("Greet(string", 0)), "name": "Greet", "returnType": "string",
        "parameters": [{ "from": 0, "modifier": "", "type": "string", "name": "who", "defaultValue": "", "callValue": "" }] });
    let edit =
        extension::refactored(&dispatch(extension::CHANGE_SIGNATURE, change).expect("Change Signature answered"))
            .expect("Change Signature changed something");
    let renamed: usize = edit.files.iter().map(|(_, e)| e.iter().filter(|e| e.text.contains("who")).count()).sum();
    assert!(
        renamed >= 2,
        "Change Signature's edits did not rename the parameter in the interface and the class: {edit:?}"
    );
    let source = dispatch(extension::DESCRIBE_INTERFACE, json!({ "at": extension::at(find("Greeter :", 0)) }))
        .expect("Extract Interface described");
    assert!(
        source["members"]
            .as_array()
            .is_some_and(|m| m.iter().any(|m| m["text"].as_str().unwrap_or_default().contains("Greet"))),
        "Extract Interface offered {source}"
    );

    // Hot Reload: the extension's Edit and Continue session starts for the solution (none of this program's edits yet),
    // and ends when the debugging session does; a server without the service says why, and the case is skipped.
    let (method, params) =
        extension::workspace_request(extension::START_HOT_RELOAD, &json!({ "capabilities": [], "originals": [] }));
    let started = extension::answer(&ask(&server, method, params)).expect("hot reload answered");
    if let Some(problem) = started["problem"].as_str() {
        eprintln!("skipped: the extension's hot reload did not start: {problem}");
    } else {
        assert!(started["session"].as_i64().is_some(), "the session's number: {started}");
        let (method, params) =
            extension::workspace_request(extension::END_HOT_RELOAD, &json!({ "session": started["session"] }));
        let _ = extension::answer(&ask(&server, method, params));
    }

    drop(server);
    let _ = std::fs::remove_dir_all(&folder);
}
