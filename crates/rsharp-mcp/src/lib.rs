//! RSharp as an MCP server: `rsharp --mcp [FOLDER]` speaks the Model Context Protocol (JSON-RPC, a message a line)
//! on its standard input and output, with Roslyn on the folder behind its tools, so an agent sees the C# as the IDE
//! does: its problems, its symbols, an outline, usages, a rename across files, and an edit checked as it lands. No UI.
//! A place is a file with a line and column (from 1, as a person counts them), or a symbol's name.

use rsharp_core::encodings::{Charset, decode, encode};
use rsharp_core::paths;
use rsharp_lsp::{self as lsp, LanguageServer, Point, ServerEvent, WorkspaceEdit};
use rsharp_text::{Document, Edit, EditPlan, offset_of_utf16};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// How long a tool waits for Roslyn to load the projects before it answers without them.
const LOAD_WAIT: Duration = Duration::from_secs(180);

/// What Roslyn has said about the folder's projects.
#[derive(Default)]
struct Load {
    loaded: bool,
    failed: Option<String>,
    problems: Vec<(PathBuf, bool, String)>,
}

struct Session {
    folder: PathBuf,
    solution: Option<PathBuf>,
    server: Option<LanguageServer>,
    load: Arc<(Mutex<Load>, Condvar)>,
    /// The files opened in Roslyn, with the version and the text it was last told.
    open: HashMap<PathBuf, (i64, String)>,
    solution_wide: bool,
}

/// Serves MCP on `input` and `output` until the input ends, Roslyn started on `folder` meanwhile.
pub fn serve(folder: &Path, input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut session = Session::start(folder);
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            write(
                &mut output,
                &json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "not JSON" } }),
            )?;
            continue;
        };
        // A notification (initialized, cancelled) needs no answer.
        let Some(id) = message.get("id").cloned() else { continue };
        let answer = match message["method"].as_str().unwrap_or_default() {
            "initialize" => Ok(json!({
                "protocolVersion": message["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "rsharp", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => {
                let params = &message["params"];
                let name = params["name"].as_str().unwrap_or_default();
                let result = session.call(name, &params["arguments"]);
                Ok(json!({
                    "content": [{ "type": "text", "text": result.as_ref().unwrap_or_else(|e| e) }],
                    "isError": result.is_err(),
                }))
            }
            method => Err(json!({ "code": -32601, "message": format!("no method {method}") })),
        };
        let reply = match answer {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        };
        write(&mut output, &reply)?;
    }
    Ok(())
}

fn write(output: &mut impl Write, message: &Value) -> io::Result<()> {
    writeln!(output, "{message}")?;
    output.flush()
}

const INSTRUCTIONS: &str = "RSharp's view of this folder's C#, from the Roslyn language server: use `diagnostics` to \
see what does not compile, `find_symbol` and `outline` to find your way, `usages` before changing a symbol, `rename` \
to rename one across every file (a type's file renamed with it), and `edit` to change a file and see its problems at \
once. Lines and columns count from 1.";

/// Each tool: its name, what it does, and its arguments' schema.
fn tools() -> Value {
    let place = json!({
        "file": { "type": "string", "description": "The file, absolute or from the folder" },
        "line": { "type": "integer", "description": "The line, from 1" },
        "column": { "type": "integer", "description": "The column, from 1 (characters)" },
        "symbol": { "type": "string", "description": "A symbol's name, instead of a file, line and column" },
    });
    let with = |extra: Value| {
        let mut all = place.clone();
        all.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        all
    };
    json!([
        { "name": "workspace", "annotations": { "readOnlyHint": true }, "description": "The folder, its solution and projects, and whether Roslyn has loaded them (with the projects that would not load).",
          "inputSchema": { "type": "object", "properties": {} } },
        { "name": "diagnostics", "annotations": { "readOnlyHint": true }, "description": "Errors and warnings: a file's, or with no file the whole solution's.",
          "inputSchema": { "type": "object", "properties": { "file": { "type": "string" } } } },
        { "name": "find_symbol", "annotations": { "readOnlyHint": true }, "description": "Types and members by name (their letters in order), with where each is declared.",
          "inputSchema": { "type": "object", "properties": { "query": { "type": "string" } }, "required": ["query"] } },
        { "name": "outline", "annotations": { "readOnlyHint": true }, "description": "A file's namespaces, types and members, nested, each with its line.",
          "inputSchema": { "type": "object", "properties": { "file": { "type": "string" } }, "required": ["file"] } },
        { "name": "usages", "annotations": { "readOnlyHint": true }, "description": "Every place a symbol is used, its declaration first, each with its line's text.",
          "inputSchema": { "type": "object", "properties": place } },
        { "name": "rename", "annotations": { "readOnlyHint": false, "destructiveHint": true }, "description": "Renames a symbol across every file and writes them; a type's file named after it is renamed too.",
          "inputSchema": { "type": "object", "properties": with(json!({ "newName": { "type": "string" } })), "required": ["newName"] } },
        { "name": "edit", "annotations": { "readOnlyHint": false, "destructiveHint": true }, "description": "Replaces text in a file (exactly once, unless all is true), writes it, and answers with the file's problems after.",
          "inputSchema": { "type": "object", "properties": {
              "file": { "type": "string" }, "oldText": { "type": "string" }, "newText": { "type": "string" },
              "all": { "type": "boolean", "description": "Replace every copy" } },
            "required": ["file", "oldText", "newText"] } },
    ])
}

/// A file's text as it is on disk, and how it is stored, so a write keeps its encoding and line separator.
fn read(path: &Path) -> Result<(Document, Charset), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{} could not be read: {e}", path.display()))?;
    let (text, charset) = decode(&bytes, None);
    Ok((Document::new(&text), charset))
}

impl Session {
    fn start(folder: &Path) -> Self {
        let folder = paths::canonical(folder);
        let solution = std::fs::read_dir(&folder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("slnx" | "sln")))
            .min_by_key(|p| (p.extension().and_then(|e| e.to_str()) != Some("slnx"), p.clone()));
        let load: Arc<(Mutex<Load>, Condvar)> = Arc::default();
        let heard = load.clone();
        let logs = std::env::temp_dir().join(format!("rsharp-mcp-{}", std::process::id()));
        let started = lsp::start_roslyn(&folder, solution.as_deref(), &logs, move |event| {
            let (state, changed) = &*heard;
            let mut state = state.lock().unwrap();
            if lsp::says_projects_loaded(&event) {
                state.loaded = true;
            }
            match &event {
                ServerEvent::Notification { method, params } if method == "window/logMessage" => {
                    let line = params["message"].as_str().unwrap_or_default();
                    if let Some(problem) = lsp::load_problem(line).filter(|_| !line.contains("Canonical.csproj")) {
                        state.problems.push(problem);
                    }
                }
                ServerEvent::Ended => state.failed = Some("Roslyn stopped".into()),
                _ => {}
            }
            changed.notify_all();
        });
        let server = match started {
            Ok(server) => Some(server),
            Err(failed) => {
                load.0.lock().unwrap().failed = Some(failed.0);
                None
            }
        };
        Self { folder, solution, server, load, open: HashMap::new(), solution_wide: false }
    }

    /// Roslyn, once it has loaded the projects (or said it cannot), waiting at most LOAD_WAIT.
    fn ready(&self) -> Result<&LanguageServer, String> {
        let (state, changed) = &*self.load;
        let state = changed
            .wait_timeout_while(state.lock().unwrap(), LOAD_WAIT, |s| !s.loaded && s.failed.is_none())
            .unwrap()
            .0;
        if let Some(why) = &state.failed {
            return Err(format!("Roslyn is not running: {why}"));
        }
        self.server.as_ref().ok_or_else(|| "Roslyn is not running".into())
    }

    /// Asks Roslyn, asking again a few times when it cancelled the request because a file changed under it.
    fn ask(&self, method: &str, params: Value) -> Result<Value, String> {
        let server = self.ready()?;
        for attempt in 0.. {
            let answer = server
                .request(method, params.clone())
                .recv_timeout(Duration::from_secs(120))
                .map_err(|_| format!("Roslyn did not answer {method}"))?;
            match answer {
                Err(e) if e.0.contains("cancel") && attempt < 10 => std::thread::sleep(Duration::from_millis(200)),
                answer => return answer.map_err(|e| format!("Roslyn refused {method}: {e}")),
            }
        }
        unreachable!("the loop returns by its tenth attempt")
    }

    fn path(&self, given: &str) -> Result<PathBuf, String> {
        let path = Path::new(given);
        let path = if path.is_absolute() { path.to_path_buf() } else { self.folder.join(path) };
        if !path.is_file() {
            return Err(format!("{given} is not a file"));
        }
        Ok(paths::canonical(&path))
    }

    fn shown(&self, path: &Path) -> String {
        path.strip_prefix(&self.folder).unwrap_or(path).to_string_lossy().into_owned()
    }

    /// A file opened in Roslyn with its text on disk, and its URI; read again each time, so a write the agent made
    /// with its own tools reaches Roslyn.
    fn opened(&mut self, path: &Path) -> Result<String, String> {
        let uri = paths::to_uri(path);
        let (document, _) = read(path)?;
        let text = document.text().to_string();
        match self.open.get(path) {
            None => {
                self.ready()?.notify(
                    "textDocument/didOpen",
                    json!({ "textDocument": { "uri": uri, "languageId": "csharp", "version": 1, "text": text } }),
                );
                self.open.insert(path.to_path_buf(), (1, text));
            }
            Some((_, told)) if *told != text => self.written(path, &text)?,
            Some(_) => {}
        }
        Ok(uri)
    }

    /// Roslyn told a file's new text: an open file's as a change, another's as changed on disk.
    fn written(&mut self, path: &Path, text: &str) -> Result<(), String> {
        let uri = paths::to_uri(path);
        let (method, params) = match self.open.get_mut(path) {
            // The whole text replaced, its range named: Roslyn takes no change without one.
            Some((version, told)) => {
                *version += 1;
                let lines: Vec<&str> = told.split('\n').collect();
                let end = Point::new(lines.len() - 1, lines.last().map_or(0, |l| l.encode_utf16().count()));
                let whole = lsp::Span { start: Point::new(0, 0), end };
                let change = json!({ "textDocument": { "uri": uri, "version": *version },
                    "contentChanges": [{ "range": whole.to_json(), "text": text }] });
                *told = text.to_string();
                ("textDocument/didChange", change)
            }
            None => ("workspace/didChangeWatchedFiles", json!({ "changes": [{ "uri": uri, "type": 2 }] })),
        };
        self.ready()?.notify(method, params);
        Ok(())
    }

    /// The place a tool's arguments name: a file, line and column, or a symbol found by its name.
    fn place(&mut self, args: &Value) -> Result<(PathBuf, Point), String> {
        if let Some(file) = args["file"].as_str() {
            let path = self.path(file)?;
            let line = args["line"].as_u64().filter(|l| *l >= 1).ok_or("a place needs line, from 1")? as usize - 1;
            let column =
                args["column"].as_u64().filter(|c| *c >= 1).ok_or("a place needs column, from 1")? as usize - 1;
            let (document, _) = read(&path)?;
            let text = document.text();
            if line >= text.len_lines() {
                return Err(format!("{file} has no line {}", line + 1));
            }
            let utf16 = text.line(line).chars().take(column).map(char::len_utf16).sum();
            return Ok((path, Point::new(line, utf16)));
        }
        let name = args["symbol"].as_str().ok_or("name a place: file, line and column, or symbol")?;
        let found = lsp::symbols(&self.ask("workspace/symbol", json!({ "query": name }))?);
        let bare = |s: &str| s.split(['(', '<']).next().unwrap_or(s).to_string();
        let named: Vec<_> = found.iter().filter(|s| bare(&s.name) == name).collect();
        match named[..] {
            [symbol] => Ok((symbol.location.path.clone(), symbol.location.span.start)),
            [] if found.is_empty() => Err(format!("no symbol named {name}")),
            _ => {
                let candidates = if named.is_empty() { found.iter().collect() } else { named };
                let listed: Vec<String> = candidates
                    .iter()
                    .take(20)
                    .map(|s| format!("{} at {}", s.name, self.at(&s.location.path, s.location.span.start)))
                    .collect();
                Err(format!(
                    "{name} names no one symbol; give the file, line and column of one of:\n{}",
                    listed.join("\n")
                ))
            }
        }
    }

    fn call(&mut self, tool: &str, args: &Value) -> Result<String, String> {
        match tool {
            // Waits for the load like the others, so what it says of Roslyn is settled; a failure is said, not refused.
            "workspace" => {
                let _ = self.ready();
                Ok(self.workspace())
            }
            "diagnostics" => match args["file"].as_str() {
                Some(file) => {
                    let path = self.path(file)?;
                    self.file_problems(&path)
                }
                None => self.solution_problems(),
            },
            "find_symbol" => {
                let query = args["query"].as_str().ok_or("find_symbol needs query")?;
                let found = lsp::symbols(&self.ask("workspace/symbol", json!({ "query": query }))?);
                if found.is_empty() {
                    return Ok(format!("No symbol matches {query}."));
                }
                Ok(found
                    .iter()
                    .take(100)
                    .map(|s| {
                        let within =
                            if s.container.is_empty() { String::new() } else { format!(" in {}", s.container) };
                        format!(
                            "{} {}{within}: {}",
                            lsp::kind_name(s.kind),
                            s.name,
                            self.at(&s.location.path, s.location.span.start)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "outline" => {
                let path = self.path(args["file"].as_str().ok_or("outline needs file")?)?;
                let uri = self.opened(&path)?;
                let shape =
                    lsp::outline(&self.ask("textDocument/documentSymbol", json!({ "textDocument": { "uri": uri } }))?);
                Ok(shape
                    .iter()
                    .map(|(depth, name, kind, at)| {
                        format!("{}{} {name} (line {})", "  ".repeat(*depth), lsp::kind_name(*kind), at.start.line + 1)
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "usages" => {
                let (path, at) = self.place(args)?;
                let uri = self.opened(&path)?;
                let mut params = lsp::at(&uri, at);
                params["context"] = json!({ "includeDeclaration": true });
                let found = lsp::locations(&self.ask("textDocument/references", params)?);
                if found.is_empty() {
                    return Ok("No usages found.".into());
                }
                let mut lines: HashMap<PathBuf, Vec<String>> = HashMap::new();
                Ok(found
                    .iter()
                    .map(|l| {
                        let text = lines.entry(l.path.clone()).or_insert_with(|| {
                            read(&l.path)
                                .map(|(d, _)| d.text().lines().map(|l| l.to_string()).collect())
                                .unwrap_or_default()
                        });
                        let line = text.get(l.span.start.line).map(|t| t.trim().to_string()).unwrap_or_default();
                        format!("{}: {line}", self.at(&l.path, l.span.start))
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "rename" => {
                let new_name = args["newName"].as_str().filter(|n| !n.is_empty()).ok_or("rename needs newName")?;
                let (path, at) = self.place(args)?;
                let uri = self.opened(&path)?;
                let here = lsp::at(&uri, at);
                let types = lsp::hierarchy_items(&self.ask("textDocument/prepareTypeHierarchy", here.clone())?);
                let declared: Vec<PathBuf> = lsp::locations(&self.ask("textDocument/definition", here.clone())?)
                    .into_iter()
                    .map(|l| l.path)
                    .collect();
                let (document, _) = read(&path)?;
                let old = word_at(&document, at);
                let mut params = here;
                params["newName"] = json!(new_name);
                let edit = lsp::workspace_edit(&self.ask("textDocument/rename", params)?);
                if edit.is_empty() {
                    return Err(format!("{old} cannot be renamed to {new_name}"));
                }
                // The files' new names checked before any file is written, so a clash changes nothing.
                let moves = if types.first().is_some_and(|t| t.name == old) {
                    lsp::files_named_after(&declared, &old, new_name)
                } else {
                    Vec::new()
                };
                if let Some((from, to)) = moves.iter().find(|(_, to)| to.exists()) {
                    return Err(format!("{} was not renamed: {} is there already", self.shown(from), self.shown(to)));
                }
                let mut said = self.apply(&edit)?;
                for (from, to) in moves {
                    std::fs::rename(&from, &to)
                        .map_err(|e| format!("{} could not be renamed: {e}", self.shown(&from)))?;
                    if self.open.remove(&from).is_some() {
                        self.ready()?.notify(
                            "textDocument/didClose",
                            json!({ "textDocument": { "uri": paths::to_uri(&from) } }),
                        );
                    }
                    self.ready()?.notify(
                        "workspace/didChangeWatchedFiles",
                        json!({ "changes": [
                            { "uri": paths::to_uri(&from), "type": 3 }, { "uri": paths::to_uri(&to), "type": 1 } ] }),
                    );
                    said.push(format!("{} renamed to {}", self.shown(&from), self.shown(&to)));
                }
                Ok(format!("Renamed {old} to {new_name}:\n{}", said.join("\n")))
            }
            "edit" => {
                let path = self.path(args["file"].as_str().ok_or("edit needs file")?)?;
                let old = args["oldText"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .ok_or("edit needs oldText")?
                    .replace("\r\n", "\n");
                let new = args["newText"].as_str().ok_or("edit needs newText")?.replace("\r\n", "\n");
                let (mut document, charset) = read(&path)?;
                let text = document.text().to_string();
                let found: Vec<usize> = text.match_indices(&old).map(|(at, _)| at).collect();
                let all = args["all"].as_bool().unwrap_or(false);
                match found.len() {
                    0 => return Err(format!("oldText is not in {}", self.shown(&path))),
                    n if n > 1 && !all => {
                        return Err(format!(
                            "oldText is in {} {n} times; give more of it, or all: true",
                            self.shown(&path)
                        ));
                    }
                    _ => {}
                }
                let edits: Vec<Edit> =
                    found.iter().map(|at| Edit { range: *at..*at + old.len(), text: new.clone() }).collect();
                document.apply(EditPlan { edits, ..Default::default() });
                self.store(&path, &document, charset)?;
                // The edit is written either way: a failure to read its problems is said, not made the edit's.
                let problems =
                    self.file_problems(&path).unwrap_or_else(|e| format!("Its problems could not be read: {e}"));
                Ok(format!("Edited {}: {} replaced.\n{problems}", self.shown(&path), found.len()))
            }
            _ => Err(format!("no tool {tool}")),
        }
    }

    fn at(&self, path: &Path, at: Point) -> String {
        format!("{}:{}:{}", self.shown(path), at.line + 1, at.character + 1)
    }

    fn workspace(&self) -> String {
        let (state, _) = &*self.load;
        let state = state.lock().unwrap();
        let mut projects: Vec<String> = Vec::new();
        let mut folders = vec![self.folder.clone()];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if path.is_dir() && !name.starts_with('.') && !matches!(name.as_str(), "bin" | "obj" | "node_modules") {
                    folders.push(path);
                } else if name.ends_with("proj") {
                    projects.push(self.shown(&path));
                }
            }
        }
        projects.sort();
        let roslyn = match (&state.failed, state.loaded) {
            (Some(why), _) => format!("not running: {why}"),
            (None, true) => "loaded".into(),
            (None, false) => "loading".into(),
        };
        let mut said = vec![
            format!("Folder: {}", self.folder.display()),
            format!("Solution: {}", self.solution.as_deref().map_or("none".into(), |s| self.shown(s))),
            format!("Projects: {}", if projects.is_empty() { "none".into() } else { projects.join(", ") }),
            format!("Roslyn: {roslyn}"),
        ];
        for (project, error, message) in &state.problems {
            said.push(format!(
                "{} {}: {message}",
                if *error { "Error loading" } else { "Warning loading" },
                self.shown(project)
            ));
        }
        said.join("\n")
    }

    fn file_problems(&mut self, path: &Path) -> Result<String, String> {
        let uri = self.opened(path)?;
        let report = self.ask("textDocument/diagnostic", json!({ "textDocument": { "uri": uri } }))?;
        let problems = lsp::problems(&report["items"]);
        if problems.is_empty() {
            return Ok(format!("No problems in {}.", self.shown(path)));
        }
        Ok(problems.iter().map(|p| self.problem(path, p)).collect::<Vec<_>>().join("\n"))
    }

    fn solution_problems(&mut self) -> Result<String, String> {
        if !self.solution_wide {
            let server = self.ready()?;
            for option in lsp::SOLUTION_WIDE {
                server.set_option(option, json!("fullSolution"));
            }
            self.solution_wide = true;
        }
        let found = lsp::workspace_problems(&self.ask("workspace/diagnostic", json!({ "previousResultIds": [] }))?);
        if found.is_empty() {
            return Ok("No problems in the solution.".into());
        }
        Ok(found
            .iter()
            .flat_map(|(path, problems)| problems.iter().map(move |p| (path, p)))
            .map(|(path, p)| self.problem(path, p))
            .collect::<Vec<_>>()
            .join("\n"))
    }

    fn problem(&self, path: &Path, problem: &lsp::Problem) -> String {
        let kind = if problem.error { "error" } else { "warning" };
        format!("{} {kind}: {}", self.at(path, problem.span.start), problem.message)
    }

    /// A workspace edit written to disk, each file in its own encoding and line separator, Roslyn told; says each file.
    fn apply(&mut self, edit: &WorkspaceEdit) -> Result<Vec<String>, String> {
        let mut said = Vec::new();
        for (path, edits) in &edit.files {
            let (mut document, charset) = read(path)?;
            let text = document.text().clone();
            let mut made: Vec<Edit> = edits
                .iter()
                .map(|e| Edit {
                    range: offset_of_utf16(&text, e.span.start.line, e.span.start.character)
                        ..offset_of_utf16(&text, e.span.end.line, e.span.end.character),
                    text: e.text.replace("\r\n", "\n"),
                })
                .collect();
            made.sort_by_key(|e| e.range.start);
            document.apply(EditPlan { edits: made, ..Default::default() });
            self.store(path, &document, charset)?;
            said.push(format!("{}: {} changed", self.shown(path), edits.len()));
        }
        Ok(said)
    }

    fn store(&mut self, path: &Path, document: &Document, charset: Charset) -> Result<(), String> {
        let bytes = encode(&document.contents(), charset)
            .map_err(|c| format!("'{c}' cannot be written in {}'s encoding", self.shown(path)))?;
        rsharp_core::files::write(path, &bytes)
            .map_err(|e| format!("{} could not be written: {e}", self.shown(path)))?;
        self.written(path, &document.text().to_string())
    }
}

/// The identifier at a place in a document.
fn word_at(document: &Document, at: Point) -> String {
    let text = document.text();
    let offset = offset_of_utf16(text, at.line, at.character);
    let word = rsharp_text::word_at(text, offset).range();
    text.byte_slice(word).to_string()
}
