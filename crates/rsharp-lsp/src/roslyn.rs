use crate::{LanguageServer, ServerEvent, ServerFailed};
use rsharp_core::{paths, programs};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Passenger's sidecars as RSharp's build made them (build.rs): the Roslyn server version Passenger pins, its extension
/// compiled against that server, and its project Host.
// shortcut: found where the build put them (compiled in), so a copy of the executable elsewhere runs the installed tool
// instead; carry them beside the app when it ships (phase 13).
struct Carried {
    server: PathBuf,
    extension: PathBuf,
    host: PathBuf,
}

/// Passenger's Host as RSharp's build published it, when it did.
pub(crate) fn carried_host() -> Option<PathBuf> {
    carried().map(|c| c.host)
}

/// RSharp's XAML host (Passenger.Xaml behind JSON lines) as the build published it, when it did.
pub(crate) fn carried_xaml_host() -> Option<PathBuf> {
    let sidecars = env!("RSHARP_SIDECARS");
    let host = Path::new(sidecars).join("xaml").join("RSharp.XamlHost.dll");
    (!sidecars.is_empty() && host.is_file()).then_some(host)
}

fn carried() -> Option<Carried> {
    let (sidecars, server) = (env!("RSHARP_SIDECARS"), env!("RSHARP_ROSLYN_SERVER"));
    if sidecars.is_empty() {
        return None;
    }
    let carried = Carried {
        server: Path::new(server).join("Microsoft.CodeAnalysis.LanguageServer.dll"),
        extension: Path::new(sidecars).join("roslyn-extension").join("Passenger.Roslyn.Extension.dll"),
        host: Path::new(sidecars).join("host").join("Passenger.Host.dll"),
    };
    (carried.server.is_file() && carried.extension.is_file() && carried.host.is_file()).then_some(carried)
}

/// Roslyn's language server: the one Passenger pins, built with its sidecars, else the person's own
/// `dotnet tool install -g roslyn-language-server`.
pub fn roslyn_server() -> Option<PathBuf> {
    carried().map(|c| c.server).or_else(|| {
        let tools = programs::home().join(".dotnet").join("tools");
        programs::find("roslyn-language-server", std::slice::from_ref(&tools))
            .or_else(|| programs::find("Microsoft.CodeAnalysis.LanguageServer", &[tools]))
    })
}

/// The projects to load: the solution's (a .slnx's or a .sln's), else every project under the folder.
pub fn projects_of(folder: &Path, solution: Option<&Path>) -> Vec<PathBuf> {
    if let Some(solution) = solution {
        let Some(solution) = rsharp_core::solution::read(solution) else { return Vec::new() };
        return solution
            .projects()
            .into_iter()
            .filter(|p| p.exists && p.path.extension().is_some_and(|e| e.to_string_lossy().ends_with("proj")))
            .map(|p| p.path.clone())
            .collect();
    }
    let mut found = Vec::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let (path, name) = (entry.path(), entry.file_name().to_string_lossy().into_owned());
            if path.is_dir() && !name.starts_with('.') && !matches!(name.as_str(), "bin" | "obj" | "node_modules") {
                folders.push(path);
            } else if name.ends_with(".csproj") || name.ends_with(".fsproj") || name.ends_with(".vbproj") {
                found.push(paths::canonical(&path));
            }
        }
    }
    found.sort();
    found
}

/// The project a file is in: the nearest folder up from it holding a project file.
pub fn project_of(file: &Path) -> Option<PathBuf> {
    file.ancestors().skip(1).find_map(|folder| {
        let mut found: Vec<PathBuf> = std::fs::read_dir(folder)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.to_string_lossy().ends_with("proj")))
            .collect();
        found.sort();
        found.into_iter().next()
    })
}

/// Files made or renamed outside the server's sight (a refactoring's new file, a type's file renamed): their projects
/// loaded again, so each one's design-time build lists them, as Passenger's folder watcher has them built again.
// shortcut: told by the edits RSharp makes only; the folder watcher (phase 4) tells of any program's.
pub fn reload_projects_of(server: &LanguageServer, files: &[PathBuf]) {
    let mut projects: Vec<PathBuf> = files.iter().filter_map(|f| project_of(f)).collect();
    projects.sort();
    projects.dedup();
    if !projects.is_empty() {
        let projects: Vec<Value> = projects.iter().map(|p| json!({ "path": p })).collect();
        server.notify("passenger/loadProjects", json!({ "projects": projects }));
    }
}

/// The global properties a build of the solution gives its projects (a Directory.Build.props may read them), and the
/// Razor design-time targets Roslyn's server gives its own build host.
/// The global properties a build of the solution gives each project.
pub(crate) fn solution_properties(solution: Option<&Path>) -> Value {
    let mut properties = serde_json::Map::new();
    if let Some(solution) = solution {
        let folder = solution.parent().unwrap_or(solution);
        properties.insert("SolutionDir".into(), json!(format!("{}{}", folder.display(), std::path::MAIN_SEPARATOR)));
        properties.insert("SolutionPath".into(), json!(solution));
        properties.insert("SolutionName".into(), json!(solution.file_stem().map(|s| s.to_string_lossy())));
        properties.insert("SolutionFileName".into(), json!(solution.file_name().map(|s| s.to_string_lossy())));
        properties
            .insert("SolutionExt".into(), json!(solution.extension().map(|e| format!(".{}", e.to_string_lossy()))));
    }
    Value::Object(properties)
}

/// A design-time build's properties: the solution's, and Roslyn's own Razor targets when it carries them.
fn build_properties(solution: Option<&Path>, server: &Path) -> Value {
    let Value::Object(mut properties) = solution_properties(solution) else { unreachable!("an object above") };
    let razor = server.parent().unwrap_or(server).join("Targets").join("Microsoft.NET.Sdk.Razor.DesignTime.targets");
    if razor.is_file() {
        properties.insert("RazorDesignTimeTargets".into(), json!(razor));
    }
    Value::Object(properties)
}

/// Starts Roslyn over `folder`, initializes it and loads `solution` (else the folder's projects). With Passenger's
/// sidecars, the server loads them as Passenger's does: the extension named the projects, each project's design-time
/// build asked of Passenger's Host. Blocks until the server answers, so call it off the UI thread.
pub fn start_roslyn(
    folder: &Path,
    solution: Option<&Path>,
    log_folder: &Path,
    host: Option<crate::SharedHost>,
    events: impl Fn(ServerEvent) + Send + Sync + 'static,
) -> Result<LanguageServer, ServerFailed> {
    let dotnet = programs::dotnet();
    let carried = carried().filter(|_| dotnet.is_some());
    let program = match &carried {
        Some(_) => dotnet.clone().expect("carried needs dotnet"),
        None => roslyn_server().ok_or_else(|| {
            ServerFailed("Roslyn's language server was not found: dotnet tool install -g roslyn-language-server".into())
        })?,
    };
    let mut command = Command::new(program);
    if let Some(carried) = &carried {
        command.arg(&carried.server);
    }
    command
        .args(["--stdio", "--logLevel", "Information", "--telemetryLevel", "off"])
        .arg("--extensionLogDirectory")
        .arg(log_folder)
        .args(["--clientProcessId", &std::process::id().to_string()])
        .current_dir(folder);
    let mut answers: crate::Answers = std::sync::Arc::new(|_: &str, _: &Value| None);
    match &carried {
        Some(carried) => {
            rsharp_core::dotnet::prepare(&mut command);
            command.arg("--extension").arg(&carried.extension);
            // Each server's MSBuild nodes its own, as Passenger keeps them: a node another's restore left is never
            // taken up, and ends with the server.
            static STARTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let started = STARTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            command.env("MSBUILDNODEHANDSHAKESALT", format!("rsharp-{}-{started}", std::process::id()));
            // The server's own restore takes no properties: off Windows a WPF or Windows Forms project restores only with
            // EnableWindowsTargeting, which a project with no Windows-only framework never reads.
            if !cfg!(windows) {
                command.env("EnableWindowsTargeting", "true");
            }
            let host = host
                .or_else(|| crate::SharedHost::carried(folder))
                .ok_or_else(|| ServerFailed("dotnet was not found for Passenger's Host".into()))?;
            let properties = build_properties(solution, &carried.server);
            let shared = std::sync::Arc::new((host, properties));
            answers = std::sync::Arc::new(move |method: &str, params: &Value| {
                if method != "passenger/projectInfo" {
                    return None;
                }
                let project = PathBuf::from(params["path"].as_str().unwrap_or_default());
                let shared = shared.clone();
                Some(Box::new(move || {
                    let (host, properties) = &*shared;
                    let built = host.build(&project, properties);
                    match built {
                        Ok(built) => {
                            // A build's problem (an SDK not found) goes to the server as an error of the project, which
                            // it then reports as it reports its own build host's (Error while loading …).
                            let mut diagnostics = built.diagnostics.as_array().cloned().unwrap_or_default();
                            if let Some(problem) = &built.problem {
                                diagnostics.push(json!({ "error": true, "message": problem, "project": project }));
                            }
                            json!({ "infos": built.infos, "problem": built.problem, "diagnostics": diagnostics })
                        }
                        // A host that failed is started again for the next project.
                        Err(failed) => {
                            json!({ "infos": null, "problem": failed.0 })
                        }
                    }
                }) as Box<dyn FnOnce() -> Value + Send>)
            });
        }
        None if solution.is_none() => {
            command.arg("--autoLoadProjects");
        }
        None => {}
    }
    if let Some(dotnet) = dotnet.as_ref().and_then(|d| d.parent().map(Path::to_path_buf)) {
        command.env("DOTNET_ROOT", dotnet);
    }
    let mut server = LanguageServer::start_answering(command, events, answers)
        .map_err(|e| ServerFailed(format!("Roslyn did not start: {e}")))?;

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
    match &carried {
        Some(carried) => {
            // The extension's message handlers (the refactorings with options), then the projects through its loader.
            let activated = server
                .request("server/_vs_activateExtension", json!({ "assemblyFilePath": carried.extension }))
                .recv_timeout(Duration::from_secs(60));
            if !matches!(activated, Ok(Ok(_))) {
                log::warn!(target: "roslyn", "Passenger's extension was not activated: {activated:?}");
            }
            let projects: Vec<Value> = projects_of(folder, solution).iter().map(|p| json!({ "path": p })).collect();
            server.notify("passenger/loadProjects", json!({ "projects": projects, "solution": solution }));
        }
        None => {
            if let Some(solution) = solution {
                server.notify("solution/open", json!({ "solution": paths::to_uri(solution) }));
            }
        }
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
            "typeHierarchy": {},
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

/// The files named after a type, renamed with it as Rider renames them: a C# file whose name up to its first dot is
/// the type's (Foo.cs, Foo.Designer.cs) gets the new name before that dot; one named otherwise (Shapes.cs) keeps its own.
pub fn files_named_after(files: &[PathBuf], old: &str, new: &str) -> Vec<(PathBuf, PathBuf)> {
    let mut moves: Vec<(PathBuf, PathBuf)> = Vec::new();
    for file in files {
        let Some(file_name) = file.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        let (stem, rest) = file_name.split_once('.').unwrap_or((&file_name, ""));
        if stem == old && file_name.ends_with(".cs") && moves.iter().all(|(f, _)| f != file) {
            moves.push((file.clone(), file.with_file_name(format!("{new}.{rest}"))));
        }
    }
    moves
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
    fn a_types_files_are_renamed_before_their_first_dot_and_a_file_named_otherwise_kept() {
        let files = [PathBuf::from("/a/Foo.cs"), PathBuf::from("/a/Foo.Designer.cs"), PathBuf::from("/a/Shapes.cs")];
        assert_eq!(
            files_named_after(&files, "Foo", "Bar"),
            [
                (PathBuf::from("/a/Foo.cs"), PathBuf::from("/a/Bar.cs")),
                (PathBuf::from("/a/Foo.Designer.cs"), PathBuf::from("/a/Bar.Designer.cs")),
            ]
        );
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
