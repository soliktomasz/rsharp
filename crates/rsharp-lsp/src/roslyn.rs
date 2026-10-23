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
            "semanticTokens": { "refreshSupport": true },
            "inlayHint": { "refreshSupport": true },
            "diagnostics": { "refreshSupport": true },
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

/// Whether Roslyn says the projects are loaded: by projectInitializationComplete, or by the end of its load's
/// progress ("Loaded 1 project(s)"), which it at times sends without the other.
pub fn says_projects_loaded(event: &ServerEvent) -> bool {
    match event {
        ServerEvent::Notification { method, params } => {
            method == "workspace/projectInitializationComplete"
                || (method == "$/progress"
                    && params["value"]["kind"] == "end"
                    && params["value"]["message"].as_str().is_some_and(|m| m.starts_with("Loaded ")))
        }
        _ => false,
    }
}

/// The options that turn on Rider's solution-wide analysis: the compiler's and the analyzers' problems for every file,
/// not only the open ones, so `workspace/diagnostic` answers for the whole solution.
pub const SOLUTION_WIDE: &[&str] = &["dotnet_compiler_diagnostics_scope", "dotnet_analyzer_diagnostics_scope"];

/// Roslyn's option for a configuration section ("csharp|inlay_hints.csharp_enable_inlay_hints_for_types"), by
/// its last name: the inlay hints Passenger turns on, as Rider shows them, and a project's files kept in the project;
/// null leaves the rest at Roslyn's default.
pub fn option(section: &str) -> Value {
    let name = section.rsplit(['.', '|']).next().unwrap_or(section);
    match name {
        "dotnet_enable_inlay_hints_for_parameters"
        | "dotnet_enable_inlay_hints_for_literal_parameters"
        | "dotnet_enable_inlay_hints_for_indexer_parameters"
        | "dotnet_suppress_inlay_hints_for_parameters_that_differ_only_by_suffix"
        | "dotnet_suppress_inlay_hints_for_parameters_that_match_method_intent"
        | "dotnet_suppress_inlay_hints_for_parameters_that_match_argument_name"
        | "csharp_enable_inlay_hints_for_types"
        | "csharp_enable_inlay_hints_for_implicit_variable_types"
        | "csharp_enable_inlay_hints_for_lambda_parameter_types"
        | "csharp_enable_inlay_hints_for_implicit_object_creation" => Value::Bool(true),
        "dotnet_enable_inlay_hints_for_object_creation_parameters"
        | "dotnet_enable_inlay_hints_for_other_parameters"
        | "csharp_enable_inlay_hints_for_collection_expressions" => Value::Bool(false),
        // A .cs file a project could hold is the project's, never a file-based program of its own (.NET 10's `dotnet
        // run file.cs`): opened while its project loads, Roslyn otherwise makes it one, cut off from the project.
        "dotnet_enable_file_based_programs_when_ambiguous" => Value::Bool(false),
        _ => Value::Null,
    }
}

/// A project Roslyn could not load whole, from a line of its log ("Warning while loading /a/B.csproj: …"): the
/// project, whether it is an error, and what was wrong.
pub fn load_problem(line: &str) -> Option<(PathBuf, bool, String)> {
    // Roslyn's log lines may lead with bracketed tags ("[LanguageServerProjectLoader]").
    let mut line = line.trim_start();
    while let Some(rest) = line.strip_prefix('[').and_then(|l| l.split_once(']')).map(|(_, rest)| rest) {
        line = rest.trim_start();
    }
    let (error, rest) = if let Some(rest) = line.strip_prefix("Error while loading ") {
        (true, rest)
    } else {
        (false, line.strip_prefix("Warning while loading ")?)
    };
    let (path, message) = rest.split_once(": ")?;
    let path = path.trim();
    if !path.ends_with("proj") && !path.ends_with(".cs") {
        return None;
    }
    // MSBuild ends a problem with the project it came from, which the line named already.
    let message = message.trim().strip_suffix(path).unwrap_or(message).trim();
    Some((PathBuf::from(path), error, message.split_whitespace().collect::<Vec<_>>().join(" ")))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_load_problem_is_read_from_roslyns_log_line_without_its_tags_or_its_project_again() {
        let line = "[LanguageServerProjectLoader] Error while loading /a/App.csproj: The SDK 'Nope' was not found.  /a/App.csproj";
        assert_eq!(
            load_problem(line),
            Some((PathBuf::from("/a/App.csproj"), true, "The SDK 'Nope' was not found.".to_string()))
        );
        assert_eq!(load_problem("Warning while loading /a/B.fsproj: old").map(|p| p.1), Some(false));
        assert_eq!(load_problem("Successfully completed load of /a/App.csproj"), None);
    }

    #[test]
    fn roslyns_inlay_hint_options_are_answered_and_the_rest_left_to_it() {
        assert_eq!(option("csharp|inlay_hints.csharp_enable_inlay_hints_for_types"), Value::Bool(true));
        assert_eq!(
            option("visual_basic|inlay_hints.dotnet_enable_inlay_hints_for_other_parameters"),
            Value::Bool(false)
        );
        assert_eq!(option("projects.dotnet_binary_log_path"), Value::Null);
    }
}
