//! The web language servers as the editor uses them: a file opened in its server gets its problems pushed, and
//! completion and Reformat Code answer in it. Skips, saying why, without Node.js or the servers.

use rsharp_lsp::ServerEvent;
use rsharp_lsp::web::{WebServer, start_web, web_language, web_servers};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

struct Opened {
    server: rsharp_lsp::LanguageServer,
    events: mpsc::Receiver<ServerEvent>,
    uri: String,
}

/// `file` written with `text` in a folder of its own and opened in its server; none when the servers cannot run here.
fn open(name: &str, file: &str, text: &str) -> Option<Opened> {
    let Some(node) = rsharp_core::programs::node() else {
        eprintln!("skipped: Node.js was not found");
        return None;
    };
    if let Err(why) = web_servers() {
        eprintln!("skipped: {why}");
        return None;
    }
    let folder = std::env::temp_dir().join(format!("rsharp-web-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let folder = rsharp_core::paths::canonical(&folder);
    let path: PathBuf = folder.join(file);
    std::fs::write(&path, text).unwrap();
    let (kind, language) = web_language(&path).expect("the file has a web server");
    let (sender, events) = mpsc::channel();
    let server = start_web(kind, &folder, &node, move |e| drop(sender.send(e)))
        .unwrap_or_else(|e| panic!("the {} server did not start: {e}", kind.name()));
    let uri = rsharp_core::paths::to_uri(&path);
    server.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": language, "version": 1, "text": text } }),
    );
    Some(Opened { server, events, uri })
}

impl Opened {
    /// The first problems the server pushes for the file that are not empty.
    fn problems(&self) -> Vec<String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            match self.events.recv_timeout(left) {
                Ok(ServerEvent::Notification { method, params })
                    if method == "textDocument/publishDiagnostics"
                        && params["uri"] == self.uri
                        && params["diagnostics"].as_array().is_some_and(|d| !d.is_empty()) =>
                {
                    return params["diagnostics"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|d| d["message"].as_str().unwrap_or_default().to_string())
                        .collect();
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        Vec::new()
    }

    fn ask(&self, method: &str, params: Value) -> Value {
        self.server
            .request(method, params)
            .recv_timeout(Duration::from_secs(60))
            .unwrap_or_else(|_| panic!("{method} was never answered"))
            .unwrap_or_else(|e| panic!("{method} was refused: {e}"))
    }

    fn completion(&self, line: u32, character: u32) -> Vec<String> {
        let answer = self.ask(
            "textDocument/completion",
            json!({ "textDocument": { "uri": self.uri }, "position": { "line": line, "character": character } }),
        );
        let items = answer.get("items").unwrap_or(&answer);
        items.as_array().into_iter().flatten().map(|i| i["label"].as_str().unwrap_or_default().to_string()).collect()
    }

    fn formatting(&self) -> Vec<String> {
        let answer = self.ask(
            "textDocument/formatting",
            json!({ "textDocument": { "uri": self.uri }, "options": { "tabSize": 4, "insertSpaces": true } }),
        );
        answer.as_array().into_iter().flatten().map(|e| e["newText"].as_str().unwrap_or_default().to_string()).collect()
    }
}

fn kind_of(file: &str) -> WebServer {
    web_language(Path::new(file)).unwrap().0
}

#[test]
fn typescript_is_checked_completed_and_formatted() {
    assert_eq!(kind_of("site.tsx"), WebServer::TypeScript);
    let text = "const count: number = \"three\";\nconsole.\nfunction  f( ){return 1}\nf();\n";
    let Some(file) = open("ts", "site.ts", text) else { return };
    let problems = file.problems();
    assert!(
        problems.iter().any(|p| p.contains("not assignable to type 'number'")),
        "a string given to a number was not marked: {problems:?}"
    );
    let offered = file.completion(1, 8);
    assert!(offered.iter().any(|l| l == "log"), "console. did not offer log: {offered:?}");
    assert!(!file.formatting().is_empty(), "Reformat Code changed nothing in badly spaced TypeScript");
    let at = |line: u32, character: u32| json!({ "textDocument": { "uri": file.uri }, "position": { "line": line, "character": character } });
    let card = file.ask("textDocument/hover", at(0, 8));
    assert!(card["contents"].to_string().contains("count: number"), "hover on count did not name its type: {card}");
    let found = file.ask("textDocument/definition", at(3, 0));
    assert!(found.to_string().contains("\"line\":2"), "Go to Declaration on f() did not find its function: {found}");
}

#[test]
fn css_html_and_json_are_checked_completed_and_formatted() {
    let Some(css) = open("css", "site.css", ".card { colr: red; }\n") else { return };
    let problems = css.problems();
    assert!(
        problems.iter().any(|p| p.contains("Unknown property")),
        "a misspelt property was not marked: {problems:?}"
    );
    let offered = css.completion(0, 8);
    assert!(offered.iter().any(|l| l == "color"), "a property name did not offer color: {offered:?}");
    assert!(!css.formatting().is_empty(), "Reformat Code laid out nothing in one-line CSS");

    let Some(html) = open("html", "index.html", "<html><body><di</body></html>\n") else { return };
    let offered = html.completion(0, 15);
    assert!(offered.iter().any(|l| l == "div"), "<di did not offer div: {offered:?}");
    assert!(!html.formatting().is_empty(), "Reformat Code laid out nothing in one-line HTML");

    let Some(json) = open("json", "data.json", "{ \"a\": 1 \"b\": 2 }\n") else { return };
    let problems = json.problems();
    assert!(!problems.is_empty(), "a missing comma in JSON was not marked");
    assert!(!json.formatting().is_empty(), "Reformat Code laid out nothing in one-line JSON");
}
