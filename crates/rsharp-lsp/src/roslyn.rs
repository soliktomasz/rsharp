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
    let server =
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
    if answer.get("capabilities").is_none() {
        return Err(ServerFailed("Roslyn's answer to initialize named no capabilities".into()));
    }
    server.notify("initialized", json!({}));
    if let Some(solution) = solution {
        server.notify("solution/open", json!({ "solution": paths::to_uri(solution) }));
    }
    Ok(server)
}

fn client_capabilities() -> Value {
    json!({
        "window": { "workDoneProgress": true },
        "workspace": { "configuration": true, "workspaceFolders": true },
        "textDocument": {
            "synchronization": { "didSave": true },
            "publishDiagnostics": {},
            "diagnostic": {},
            "hover": { "contentFormat": ["markdown", "plaintext"] },
        },
    })
}
