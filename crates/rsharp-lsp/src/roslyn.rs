use crate::{LanguageServer, ServerEvent, ServerFailed};
use rsharp_core::{paths, programs};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Roslyn's language server as `dotnet tool install -g roslyn-language-server` puts it in place.
// shortcut: found on the machine rather than carried beside the app (Passenger downloads it at build), so a
// person installs the tool first; carrying it pins one version.
pub fn roslyn_server() -> Option<PathBuf> {
    let tools = programs::home().join(".dotnet").join("tools");
    programs::find("roslyn-language-server", std::slice::from_ref(&tools))
        .or_else(|| programs::find("Microsoft.CodeAnalysis.LanguageServer", &[tools]))
}

/// Starts Roslyn over `folder`, initializes it and opens `solution` (else the folder's projects, found by the
/// server itself). Blocks until the server answers, so call it off the UI thread.
pub fn start_roslyn(
    folder: &Path,
    solution: Option<&Path>,
    log_folder: &Path,
    events: impl Fn(ServerEvent) + Send + Sync + 'static,
) -> Result<LanguageServer, ServerFailed> {
    let program = roslyn_server().ok_or_else(|| {
        ServerFailed("Roslyn's language server was not found: dotnet tool install -g roslyn-language-server".into())
    })?;
    let mut command = Command::new(program);
    command
        .args(["--stdio", "--logLevel", "Information", "--telemetryLevel", "off"])
        .arg("--extensionLogDirectory")
        .arg(log_folder)
        .args(["--clientProcessId", &std::process::id().to_string()])
        .current_dir(folder);
    if solution.is_none() {
        command.arg("--autoLoadProjects");
    }
    if let Some(dotnet) = programs::dotnet().and_then(|d| d.parent().map(Path::to_path_buf)) {
        command.env("DOTNET_ROOT", dotnet);
    }
    let mut server =
        LanguageServer::start(command, events).map_err(|e| ServerFailed(format!("Roslyn did not start: {e}")))?;

    let root = paths::to_uri(folder);
    let answer = server
        .request("initialize", json!({
            "processId": std::process::id(),
            "rootUri": root,
            "workspaceFolders": [{ "uri": root, "name": folder.file_name().map(|n| n.to_string_lossy()).unwrap_or_default() }],
            "capabilities": client_capabilities(),
        }))
        .recv_timeout(Duration::from_secs(120))
        .map_err(|_| ServerFailed("Roslyn did not answer initialize".into()))??;
    match answer.get("capabilities") {
        Some(capabilities) => server.capabilities = capabilities.clone(),
        None => return Err(ServerFailed("Roslyn's answer to initialize named no capabilities".into())),
    }
    server.notify("initialized", json!({}));
    if let Some(solution) = solution {
        server.notify("solution/open", json!({ "solution": paths::to_uri(solution) }));
    }
    Ok(server)
}

/// What RSharp tells Roslyn it can do, as Passenger tells it: completion and code actions with resolve, rename,
/// semantic tokens over ranges, inlay hints, and UTF-16 positions, which are what the editor's columns already are.
fn client_capabilities() -> Value {
    json!({
        "general": { "positionEncodings": ["utf-16"] },
        "window": { "workDoneProgress": true },
        "workspace": {
            "configuration": true,
            "workspaceFolders": true,
            "workspaceEdit": { "documentChanges": true, "resourceOperations": ["create", "rename", "delete"] },
            "didChangeWatchedFiles": { "dynamicRegistration": false },
            "symbol": {},
        },
        "textDocument": {
            "synchronization": { "didSave": true },
            "completion": {
                "completionItem": {
                    "snippetSupport": false,
                    "documentationFormat": ["markdown", "plaintext"],
                    "labelDetailsSupport": true,
                    "resolveSupport": { "properties": ["documentation", "detail", "additionalTextEdits"] },
                },
                "completionList": { "itemDefaults": ["commitCharacters", "editRange", "data"] },
                "contextSupport": true,
            },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "signatureHelp": {
                "signatureInformation": {
                    "documentationFormat": ["markdown", "plaintext"],
                    "parameterInformation": { "labelOffsetSupport": true },
                    "activeParameterSupport": true,
                },
                "contextSupport": true,
            },
            "definition": { "linkSupport": true },
            "typeDefinition": { "linkSupport": true },
            "implementation": { "linkSupport": true },
            "references": {},
            "documentHighlight": {},
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
            "codeAction": {
                "codeActionLiteralSupport": { "codeActionKind": { "valueSet": [
                    "", "quickfix", "refactor", "refactor.extract", "refactor.inline", "refactor.rewrite", "source",
                    "source.organizeImports",
                ] } },
                "isPreferredSupport": true,
                "dataSupport": true,
                "resolveSupport": { "properties": ["edit"] },
            },
            "formatting": {},
            "rangeFormatting": {},
            "rename": { "prepareSupport": true },
            "semanticTokens": {
                "requests": { "range": true, "full": { "delta": false } },
                "tokenTypes": [],
                "tokenModifiers": [],
                "formats": ["relative"],
                "multilineTokenSupport": false,
                "overlappingTokenSupport": false,
            },
            "inlayHint": {},
            "diagnostic": {},
            "publishDiagnostics": {},
        },
    })
}

/// The kinds Roslyn names its semantic tokens by, in its legend's order.
pub fn token_legend(server: &LanguageServer) -> Vec<String> {
    server.capabilities["semanticTokensProvider"]["legend"]["tokenTypes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_str().map(str::to_string))
        .collect()
}
