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
}

impl WebServer {
    /// The server's name as the status bar says it.
    pub fn name(self) -> &'static str {
        match self {
            WebServer::TypeScript => "TypeScript",
            WebServer::Html => "HTML",
            WebServer::Css => "CSS",
            WebServer::Json => "JSON",
        }
    }

    fn script(self, modules: &Path) -> PathBuf {
        let extracted = modules.join("vscode-langservers-extracted/lib");
        match self {
            WebServer::TypeScript => modules.join("typescript-language-server/lib/cli.mjs"),
            WebServer::Html => extracted.join("html-language-server/node/htmlServerMain.js"),
            WebServer::Css => extracted.join("css-language-server/node/cssServerMain.js"),
            WebServer::Json => extracted.join("json-language-server/node/jsonServerMain.js"),
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
    let answers: Answers = Arc::new(move |method: &str, params: &Value| {
        (method == "workspace/configuration").then(|| {
            let items: Vec<Value> = params["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| section(kind, i["section"].as_str()))
                .collect();
            Box::new(move || Value::Array(items)) as Box<dyn FnOnce() -> Value + Send>
        })
    });
    let mut server = LanguageServer::start_answering(command, events, answers)
        .map_err(|e| ServerFailed(format!("the {} language server did not start: {e}", kind.name())))?;
    let root = paths::to_uri(folder);
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
        WebServer::TypeScript => json!({}),
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
/// styles and scripts, the rest left to its defaults.
fn section(kind: WebServer, section: Option<&str>) -> Value {
    match (kind, section) {
        (WebServer::Css, Some("css" | "scss" | "less")) | (WebServer::Html, Some("css")) => json!({ "validate": true }),
        (WebServer::Html, Some("html")) => {
            json!({ "validate": { "scripts": true, "styles": true }, "format": { "enable": true } })
        }
        _ => Value::Null,
    }
}
