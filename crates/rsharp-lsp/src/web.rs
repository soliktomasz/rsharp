//! The web language servers, as Passenger's WebLanguages runs them: typescript-language-server over TypeScript's own
//! tsserver for TypeScript and JavaScript, and VS Code's HTML, CSS and JSON servers (vscode-langservers-extracted), the
//! npm packages the build put under `sidecars/web` (RSHARP_WEB), each run on the machine's Node.js.
// shortcut: no JSON schemas (Passenger carries SchemaStore's and answers `vscode/content`), so a JSON file's syntax is
// checked but not its keys; carry them when package.json's or appsettings.json's completion is wanted.

use crate::roslyn::client_capabilities;
use crate::{Answers, LanguageServer, ServerEvent, ServerFailed};
use rsharp_core::paths;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WebServer {
    TypeScript,
    Html,
    Css,
    Json,
    /// VS Code's ESLint server, which runs the project's own eslint over the files it covers, beside their own server.
    Eslint,
}

impl WebServer {
    /// The server's name as the status bar says it.
    pub fn name(self) -> &'static str {
        match self {
            WebServer::TypeScript => "TypeScript",
            WebServer::Html => "HTML",
            WebServer::Css => "CSS",
            WebServer::Json => "JSON",
            WebServer::Eslint => "ESLint",
        }
    }

    fn script(self, modules: &Path) -> PathBuf {
        let extracted = modules.join("vscode-langservers-extracted/lib");
        match self {
            WebServer::TypeScript => modules.join("typescript-language-server/lib/cli.mjs"),
            WebServer::Html => extracted.join("html-language-server/node/htmlServerMain.js"),
            WebServer::Css => extracted.join("css-language-server/node/cssServerMain.js"),
            WebServer::Json => extracted.join("json-language-server/node/jsonServerMain.js"),
            WebServer::Eslint => extracted.join("eslint-language-server/eslintServer.js"),
        }
    }
}

/// The server a file is answered by and its language there, by its name; none for a file no web server reads.
pub fn web_language(path: &Path) -> Option<(WebServer, &'static str)> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "ts" | "mts" | "cts" => (WebServer::TypeScript, "typescript"),
        "tsx" => (WebServer::TypeScript, "typescriptreact"),
        "js" | "mjs" | "cjs" => (WebServer::TypeScript, "javascript"),
        "jsx" => (WebServer::TypeScript, "javascriptreact"),
        "html" | "htm" => (WebServer::Html, "html"),
        "css" => (WebServer::Css, "css"),
        "scss" => (WebServer::Css, "scss"),
        "less" => (WebServer::Css, "less"),
        "jsonc" => (WebServer::Json, "jsonc"),
        // The JSON files whose readers take comments, which the server then does not mark.
        "json"
            if name.starts_with("tsconfig")
                || name.starts_with("jsconfig")
                || name.starts_with("appsettings")
                || ["launchsettings.json", "global.json", ".eslintrc.json"].contains(&name.as_str()) =>
        {
            (WebServer::Json, "jsonc")
        }
        "json" => (WebServer::Json, "json"),
        _ => return None,
    })
}

/// Where the build put the servers' packages; none for a build without them.
pub fn web_servers() -> Result<PathBuf, String> {
    let folder = PathBuf::from(env!("RSHARP_WEB"));
    if env!("RSHARP_WEB").is_empty() {
        return Err("this build carries no web language servers (npm was not found when it was built)".into());
    }
    let modules = folder.join("node_modules");
    if modules.is_dir() {
        Ok(modules)
    } else {
        Err(format!("the web language servers are missing from {}", folder.display()))
    }
}

/// Starts `kind`'s server on `node` over `folder`, initialized and told its settings.
pub fn start_web(
    kind: WebServer,
    folder: &Path,
    node: &Path,
    events: impl Fn(ServerEvent) + Send + Sync + 'static,
) -> Result<LanguageServer, ServerFailed> {
    let modules = web_servers().map_err(ServerFailed)?;
    let mut command = Command::new(node);
    command.arg(kind.script(&modules)).arg("--stdio").current_dir(folder);
    let root = paths::to_uri(folder);
    let workspace = json!({ "name": folder.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(), "uri": root });
    let answers: Answers = Arc::new(move |method: &str, params: &Value| {
        (method == "workspace/configuration").then(|| {
            let items: Vec<Value> = params["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| section(kind, i["section"].as_str(), &workspace))
                .collect();
            Box::new(move || Value::Array(items)) as Box<dyn FnOnce() -> Value + Send>
        })
    });
    let mut server = LanguageServer::start_answering(command, events, answers)
        .map_err(|e| ServerFailed(format!("the {} language server did not start: {e}", kind.name())))?;
    let options = match kind {
        // Never installs type packages from npm behind the person's back (automatic type acquisition).
        WebServer::TypeScript => json!({
            "hostInfo": "RSharp",
            "disableAutomaticTypingAcquisition": true,
            "tsserver": { "logVerbosity": "off" },
            "preferences": { "includeCompletionsForModuleExports": true, "includeCompletionsForImportStatements": true },
        }),
        WebServer::Html => {
            json!({ "provideFormatter": true, "embeddedLanguages": { "css": true, "javascript": true } })
        }
        WebServer::Css => json!({ "provideFormatter": true }),
        WebServer::Json => json!({ "provideFormatter": true, "handledSchemaProtocols": ["file"] }),
        WebServer::Eslint => Value::Null,
    };
    let answer = server
        .request("initialize", json!({
            "processId": std::process::id(),
            "rootUri": root,
            "workspaceFolders": [{ "uri": root, "name": folder.file_name().map(|n| n.to_string_lossy()).unwrap_or_default() }],
            "capabilities": capabilities(),
            "initializationOptions": options,
        }))
        .recv_timeout(Duration::from_secs(60))
        .map_err(|_| ServerFailed(format!("the {} language server did not answer initialize", kind.name())))??;
    server.capabilities = answer.get("capabilities").cloned().unwrap_or(Value::Null);
    server.notify("initialized", json!({}));
    // What a server reads from its settings rather than asks for.
    let settings = match kind {
        WebServer::Json => {
            json!({ "json": { "validate": { "enable": true }, "format": { "enable": true }, "schemas": [] } })
        }
        WebServer::Html => json!({
            "html": { "format": { "enable": true }, "validate": { "scripts": true, "styles": true } },
            "css": { "validate": true },
        }),
        WebServer::Css => {
            json!({ "css": { "validate": true }, "scss": { "validate": true }, "less": { "validate": true } })
        }
        WebServer::TypeScript | WebServer::Eslint => json!({}),
    };
    server.notify("workspace/didChangeConfiguration", json!({ "settings": settings }));
    Ok(server)
}

/// The client's abilities as Roslyn is told them, less pulled diagnostics: VS Code's servers then push theirs, as
/// typescript-language-server always does.
fn capabilities() -> Value {
    let mut capabilities = client_capabilities();
    capabilities["textDocument"].as_object_mut().map(|t| t.remove("diagnostic"));
    capabilities["workspace"].as_object_mut().map(|w| w.remove("diagnostics"));
    capabilities
}

/// What a server's `workspace/configuration` is answered: validation on for CSS's languages and the HTML server's
/// styles and scripts; ESLint's whole settings, as VS Code's extension gives them (lint as typed, the project's eslint
/// found from the file up, its own flat or eslintrc configuration as its eslint decides); the rest left to defaults.
fn section(kind: WebServer, section: Option<&str>, workspace: &Value) -> Value {
    match (kind, section) {
        (WebServer::Eslint, None | Some("")) => json!({
            "validate": "on",
            "packageManager": "npm",
            "useESLintClass": false,
            "experimental": {},
            "codeAction": {
                "disableRuleComment": { "enable": true, "location": "separateLine", "commentStyle": "line" },
                "showDocumentation": { "enable": true },
            },
            "codeActionOnSave": { "mode": "all" },
            "format": false,
            "quiet": false,
            "onIgnoredFiles": "off",
            "options": {},
            "rulesCustomizations": [],
            "run": "onType",
            "problems": { "shortenToSingleLine": false },
            "nodePath": null,
            "workspaceFolder": workspace,
        }),
        (WebServer::Css, Some("css" | "scss" | "less")) | (WebServer::Html, Some("css")) => json!({ "validate": true }),
        (WebServer::Html, Some("html")) => {
            json!({ "validate": { "scripts": true, "styles": true }, "format": { "enable": true } })
        }
        _ => Value::Null,
    }
}

/// The package's folder as Node.js finds it from the file (the nearest node_modules above it that has it), as Rider's
/// Automatic ESLint and Prettier configuration finds the project's own.
pub fn installed(file: &Path, name: &str) -> Option<PathBuf> {
    file.ancestors().skip(1).map(|f| f.join("node_modules").join(name)).find(|p| p.join("package.json").is_file())
}

/// Whether ESLint checks the file, by Rider's default "Run for files"; a package's own file never.
pub fn eslint_covers(file: &Path) -> bool {
    covers(file, &["js", "ts", "jsx", "tsx", "cjs", "cts", "mjs", "mts", "html", "vue"])
}

/// Whether Prettier formats the file, by Rider's default "Run for files"; a package's own file never.
pub fn prettier_covers(file: &Path) -> bool {
    covers(file, &["js", "ts", "jsx", "tsx", "cjs", "cts", "mjs", "mts", "json", "vue", "astro"])
}

fn covers(file: &Path, extensions: &[&str]) -> bool {
    !file.components().any(|c| c.as_os_str() == "node_modules")
        && file
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| extensions.contains(&e.to_ascii_lowercase().as_str()))
}

/// What Prettier made of a file: its text and where the caret goes in it; none for a file its .prettierignore names
/// or it has no parser for.
pub struct Prettified {
    pub text: String,
    pub cursor: usize,
}

/// The one Prettier runner (Passenger's `prettier.cjs`, carried as `sidecars/web/prettier.cjs`), kept warm for the
/// process: it loads each project's package once and changes to its folder for each request.
static PRETTIER: std::sync::Mutex<Option<Runner>> = std::sync::Mutex::new(None);

struct Runner {
    input: std::process::ChildStdin,
    lines: std::sync::mpsc::Receiver<String>,
    next: u64,
    _child: rsharp_core::process::Child,
}

/// The file's text formatted by the project's own prettier (`package`), with its .prettierrc, .editorconfig and
/// nearest .prettierignore; `cursor` is a UTF-16 offset in `text`, as JavaScript counts, and so is the answer's.
/// Blocks until Prettier answers: call it off the UI thread.
pub fn prettify(
    node: &Path,
    package: &Path,
    file: &Path,
    text: &str,
    cursor: usize,
) -> Result<Option<Prettified>, String> {
    use std::io::{BufRead, Write};
    let mut runner = PRETTIER.lock().unwrap_or_else(|p| p.into_inner());
    if runner.is_none() {
        let script = PathBuf::from(env!("RSHARP_WEB")).join("prettier.cjs");
        if env!("RSHARP_WEB").is_empty() || !script.is_file() {
            return Err("this build carries no Prettier runner".into());
        }
        let mut command = Command::new(node);
        command.arg(&script).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped());
        let mut child = rsharp_core::process::spawn(command).map_err(|e| format!("Prettier did not start: {e}"))?;
        let input = child.process.stdin.take().expect("stdin was piped");
        let output = child.process.stdout.take().expect("stdout was piped");
        let (sender, lines) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("prettier".into())
            .spawn(move || {
                for line in std::io::BufReader::new(output).lines().map_while(Result::ok) {
                    if sender.send(line).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        *runner = Some(Runner { input, lines, next: 0, _child: child });
    }
    let ignore = file.ancestors().skip(1).map(|f| f.join(".prettierignore")).find(|p| p.is_file());
    let answer = (|| -> Result<Value, String> {
        let running = runner.as_mut().expect("started above");
        running.next += 1;
        let request = json!({
            "id": running.next, "prettier": package, "file": file, "text": text, "cursor": cursor, "ignorePath": ignore,
        });
        writeln!(running.input, "{request}").map_err(|e| format!("Prettier stopped: {e}"))?;
        loop {
            let line = running
                .lines
                .recv_timeout(Duration::from_secs(30))
                .map_err(|_| "Prettier did not answer within 30 seconds".to_string())?;
            let answer: Value = serde_json::from_str(&line).map_err(|e| format!("Prettier said {line:?}: {e}"))?;
            if answer["id"] == running.next {
                return Ok(answer);
            }
        }
    })();
    // A runner that stopped or hung is started again for the next format.
    let answer = answer.inspect_err(|_| *runner = None)?;
    if let Some(error) = answer["error"].as_str() {
        return Err(error.to_string());
    }
    if answer["ignored"] == true || answer["unsupported"] == true {
        return Ok(None);
    }
    Ok(Some(Prettified {
        text: answer["formatted"].as_str().unwrap_or_default().to_string(),
        cursor: answer["cursor"].as_u64().unwrap_or(0) as usize,
    }))
}
