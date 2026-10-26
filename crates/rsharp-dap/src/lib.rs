//! The Debug Adapter Protocol client and Passenger's debug adapter, which this runs as Passenger does: its engine behind
//! the protocol in a process of its own. No UI: events come on the adapter's threads, and a request's answer is waited
//! for off the UI thread.

mod connection;
pub mod protocol;

use connection::{Connection, Incoming};
pub use protocol::Capabilities;
use protocol::{Breakpoint, Evaluated, Frame, Scope, Thread, Variable};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// The adapter Passenger built from vendor/passenger, at build time; beside the executable when it was not built.
pub fn adapter() -> Option<PathBuf> {
    let built = env!("RSHARP_DEBUG_ADAPTER");
    let path = if built.is_empty() {
        std::env::current_exe().ok()?.parent()?.join("debugger").join("Passenger.DebugAdapter.dll")
    } else {
        PathBuf::from(built)
    };
    path.is_file().then_some(path)
}

pub use protocol::RunInTerminal;

/// How the client starts a program the adapter asks for in a terminal of its own.
pub type TerminalRun = Arc<dyn Fn(RunInTerminal) -> Result<RunInTerminalAnswer, String> + Send + Sync>;

/// What the adapter says while a session runs.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The program stopped: at a breakpoint, a step, an exception, or a pause.
    Stopped(Stop),
    Continued,
    Exited(Option<i64>),
    Terminated,
    /// What the program wrote, or what the adapter said of itself on its error output.
    Output {
        text: String,
        error: bool,
    },
    /// A breakpoint as the adapter last made it: bound and will be hit, or not, with why not.
    Breakpoint {
        path: Option<PathBuf>,
        line: Option<usize>,
        verified: bool,
        message: Option<String>,
    },
    /// The adapter's process ended; nothing more is asked of it.
    Ended,
}

/// Where and why a program stopped.
#[derive(Clone, Debug, PartialEq)]
pub struct Stop {
    /// breakpoint, step, exception, pause, entry, data breakpoint, function breakpoint.
    pub reason: String,
    pub thread: i64,
    pub path: Option<PathBuf>,
    /// From 0.
    pub line: Option<usize>,
}

impl Stop {
    /// The stop in the person's words: "Paused on a breakpoint", "Paused on an exception".
    pub fn said(&self) -> String {
        match self.reason.as_str() {
            "breakpoint" => "Paused on a breakpoint".into(),
            "step" => "Paused after a step".into(),
            "exception" => "Paused on an exception".into(),
            "pause" => "Paused".into(),
            "entry" => "Paused at the program's entry".into(),
            "data breakpoint" => "Paused on a data breakpoint".into(),
            "function breakpoint" => "Paused on a breakpoint".into(),
            other => format!("Paused ({other})"),
        }
    }
}

/// A line breakpoint as the debugger is told of it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceBreakpoint {
    /// From 0.
    pub line: usize,
    /// From 0, where a breakpoint is for one call on a line several share (a lambda's body, say); usually absent.
    pub column: Option<usize>,
    pub condition: Option<String>,
    /// Rider's hit count: `5`, `> 2`, `% 3` and the like.
    pub hit_condition: Option<String>,
    pub log_message: Option<String>,
}

/// Rider's function breakpoint as the debugger is told of it: the method's name and what a hit asks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FunctionBreakpoint {
    /// `Namespace.Type.Method`, with `*` at its end for a suffix that matches any.
    pub name: String,
    pub condition: Option<String>,
    pub hit_condition: Option<String>,
    pub log_message: Option<String>,
}

/// A CLR exception breakpoint as the debugger is told of it: the types it is about (none for every exception), the
/// cases that stop the program, and what a hit asks besides.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExceptionFilter {
    /// Full names as CLR spells them; empty for every exception.
    pub types: Vec<String>,
    /// Rider's "Suspend if": thrownInUserCode, thrownInExternalCode, handledInUserCode, handledInExternalCode, unhandled.
    pub scenarios: Vec<String>,
    pub condition: Option<String>,
    pub hit_condition: Option<String>,
    pub log_message: Option<String>,
}

impl ExceptionFilter {
    /// The path the protocol takes: CLR, then the types; CLR alone is every exception.
    fn path(&self) -> Vec<String> {
        let mut path = vec!["CLR".to_string()];
        path.extend(self.types.iter().filter(|kind| !kind.trim().is_empty()).cloned());
        path
    }
}

/// What a program is started with: the assembly or executable, its arguments, folder and environment.
#[derive(Clone, Debug)]
pub struct Launch {
    pub program: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub stop_at_entry: bool,
    /// The program's code may be changed as it runs (Hot Reload): the adapter turns the runtime's Edit and Continue on.
    pub hot_reload: bool,
    /// Just My Code: the debugger passes over library code, as Rider's setting does.
    pub just_my_code: bool,
}

/// What the client's terminal answers the adapter with: the program's process, once it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunInTerminalAnswer {
    pub process_id: u32,
}

/// How a session starts: a program of its own, or a running one attached to.
#[derive(Clone, Debug)]
pub enum Start {
    Launch(Launch),
    Attach { process_id: u32, just_my_code: bool },
}

const INITIALIZE: Duration = Duration::from_secs(20);
const LAUNCH: Duration = Duration::from_secs(30);
const INITIALIZED: Duration = Duration::from_secs(60);
const CONFIGURED: Duration = Duration::from_secs(120);
const REQUEST: Duration = Duration::from_secs(20);
/// A frame's decompile: CoreLib's takes a minute or more, off the engine's lock.
const DECOMPILE: Duration = Duration::from_secs(300);

/// One program under the debugger: the adapter's process, the conversation with it, and the requests the debug window
/// makes. Every method is safe to call off the UI thread and none needs the UI.
pub struct Session {
    connection: Arc<Connection>,
    /// The process the adapter started, once it said which it is.
    process_id: Arc<AtomicI64>,
    /// What the adapter made of each file's breakpoints as the session opened.
    verified: Mutex<Vec<(PathBuf, Vec<Breakpoint>)>>,
    pub capabilities: Capabilities,
    /// What the program is, as the Debug window names it.
    pub program: String,
    /// Where the adapter keeps the symbols it decompiles (`--symbolCache`).
    symbols: PathBuf,
    /// The embedded sources of each decompiled PDB read, by its path.
    decompiled: Mutex<HashMap<PathBuf, Arc<Vec<(String, String)>>>>,
}

/// A session's connection while it starts, for a client that stops a launch whose runtime has not answered yet.
pub struct Starting {
    connection: Arc<Connection>,
}

impl Starting {
    /// Stops the launch: the adapter ends the program and answers configurationDone saying the session ended.
    pub fn stop(&self) -> Result<(), String> {
        self.connection.request("disconnect", json!({ "terminateDebuggee": true }), REQUEST).map(drop)
    }
}

impl Session {
    /// Starts the adapter and the program (or attaches it), with the folder and its lines as breakpoints. The session is
    /// ready once the adapter's `configurationDone` has been answered, which waits for the program's runtime.
    pub fn start(
        adapter: &Path,
        start: Start,
        symbols: &Path,
        breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)>,
        functions: Vec<FunctionBreakpoint>,
        exceptions: Vec<ExceptionFilter>,
        filters: Vec<(String, Option<String>)>,
        terminal: Option<TerminalRun>,
        events: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<Session, String> {
        Self::start_watched(
            adapter,
            start,
            symbols,
            breakpoints,
            functions,
            exceptions,
            filters,
            terminal,
            events,
            |_| {},
        )
    }

    /// As `start`, with the starting session given to `watch` before its runtime is waited for.
    #[allow(clippy::too_many_arguments)]
    pub fn start_watched(
        adapter: &Path,
        start: Start,
        symbols: &Path,
        breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)>,
        functions: Vec<FunctionBreakpoint>,
        exceptions: Vec<ExceptionFilter>,
        filters: Vec<(String, Option<String>)>,
        terminal: Option<TerminalRun>,
        events: impl Fn(Event) + Send + Sync + 'static,
        watch: impl FnOnce(Starting),
    ) -> Result<Session, String> {
        let dotnet = rsharp_core::programs::dotnet()
            .ok_or_else(|| "No .NET command line found: install the .NET SDK, or set DOTNET_ROOT".to_string())?;
        let mut command = Command::new(&dotnet);
        command.arg(adapter).args(["--interpreter=vscode"]).arg(format!("--symbolCache={}", symbols.display()));
        let initialized = Arc::new((Mutex::new(false), Condvar::new()));
        let process_id = Arc::new(AtomicI64::new(0));
        let ids = process_id.clone();
        let ready = initialized.clone();
        let said = Arc::new(events);
        let reporting = said.clone();
        let notify = Arc::new(move |incoming: Incoming| match incoming {
            Incoming::Event { name, body } => {
                let event = match name.as_str() {
                    "initialized" => {
                        let (ready, woken) = &*ready;
                        *ready.lock().unwrap() = true;
                        woken.notify_all();
                        None
                    }
                    "stopped" => Some(Event::Stopped(Stop {
                        reason: text(&body, "reason").unwrap_or_else(|| "unknown".into()),
                        thread: number(&body, "threadId").unwrap_or_default(),
                        path: body["source"]["path"].as_str().map(PathBuf::from),
                        line: number(&body, "line").map(|line| line.saturating_sub(1) as usize),
                    })),
                    "continued" => Some(Event::Continued),
                    "exited" => Some(Event::Exited(number(&body, "exitCode"))),
                    "terminated" => Some(Event::Terminated),
                    "output" => Some(Event::Output {
                        text: body["output"].as_str().unwrap_or_default().to_string(),
                        error: body["category"].as_str() == Some("stderr"),
                    }),
                    "breakpoint" => {
                        let breakpoint = &body["breakpoint"];
                        Some(Event::Breakpoint {
                            path: breakpoint["source"]["path"].as_str().map(PathBuf::from),
                            line: number(breakpoint, "line").map(|line| line.saturating_sub(1) as usize),
                            verified: breakpoint["verified"].as_bool().unwrap_or(false),
                            message: breakpoint["message"].as_str().map(str::to_string),
                        })
                    }
                    "process" => {
                        ids.store(number(&body, "systemProcessId").unwrap_or_default(), Ordering::Release);
                        None
                    }
                    _ => None,
                };
                if let Some(event) = event {
                    reporting(event);
                }
            }
            Incoming::Stderr(line) => reporting(Event::Output { text: format!("{line}\n"), error: true }),
            Incoming::Ended => reporting(Event::Ended),
        });
        // The adapter's own request, a program in the terminal: it is answered by the client's terminal when one is
        // given, and refused otherwise so the adapter goes on rather than waiting.
        let requests: connection::Requests = {
            let terminal = terminal.clone();
            Arc::new(move |command: &str, arguments: &Value| {
                if command != "runInTerminal" {
                    return Err(format!("{command} is not supported by this debugger client"));
                }
                let Some(terminal) = &terminal else {
                    return Err("A program in the terminal is not supported by this session".into());
                };
                let request: RunInTerminal = serde_json::from_value(arguments.clone())
                    .map_err(|error| format!("The debugger's runInTerminal did not read: {error}"))?;
                let answer = terminal(request)?;
                Ok(json!({ "processId": answer.process_id }))
            })
        };
        let connection = Connection::start(command, notify, requests)
            .map_err(|error| format!("The debugger did not start: {error}"))?;
        watch(Starting { connection: Arc::clone(&connection) });
        let mut session = Session {
            connection,
            process_id,
            verified: Mutex::new(Vec::new()),
            capabilities: Capabilities::default(),
            program: match &start {
                Start::Launch(launch) => launch.program.file_stem().unwrap_or_default().to_string_lossy().into_owned(),
                Start::Attach { process_id, .. } => format!("process {process_id}"),
            },
            symbols: symbols.to_path_buf(),
            decompiled: Mutex::new(HashMap::new()),
        };
        match session.begin(start, breakpoints, functions, exceptions, filters, terminal.is_some(), &initialized) {
            Ok(capabilities) => {
                session.capabilities = capabilities;
                Ok(session)
            }
            Err(why) => {
                session.connection.kill();
                Err(why)
            }
        }
    }

    /// The conversation's opening, as the protocol orders it: initialize, the program or the process told of, the
    /// breakpoints sent once the adapter is ready for them, then configurationDone, whose answer starts the program.
    fn begin(
        &self,
        start: Start,
        breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)>,
        functions: Vec<FunctionBreakpoint>,
        exceptions: Vec<ExceptionFilter>,
        filters: Vec<(String, Option<String>)>,
        runs_in_terminal: bool,
        initialized: &Arc<(Mutex<bool>, Condvar)>,
    ) -> Result<Capabilities, String> {
        let body = self.connection.request(
            "initialize",
            json!({
                "clientID": "rsharp",
                "clientName": "RSharp",
                "adapterID": "coreclr",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "pathFormat": "path",
                "supportsVariableType": true,
                "supportsRunInTerminalRequest": runs_in_terminal,
            }),
            INITIALIZE,
        )?;
        let capabilities: Capabilities =
            serde_json::from_value(body).map_err(|error| format!("The debugger's abilities did not read: {error}"))?;
        match start {
            Start::Launch(launch) => self.connection.request(
                "launch",
                json!({
                    "program": launch.program,
                    "args": launch.arguments,
                    "cwd": launch.working_directory,
                    "env": launch.environment,
                    "console": if runs_in_terminal { "integratedTerminal" } else { "internalConsole" },
                    "stopAtEntry": launch.stop_at_entry,
                    "justMyCode": launch.just_my_code,
                    "hotReload": launch.hot_reload,
                }),
                LAUNCH,
            )?,
            Start::Attach { process_id, just_my_code } => self.connection.request(
                "attach",
                json!({ "processId": process_id, "justMyCode": just_my_code }),
                LAUNCH,
            )?,
        };
        let (ready, woken) = &**initialized;
        let mut done = ready.lock().unwrap();
        while !*done {
            let (now, timeout) = woken.wait_timeout(done, INITIALIZED).unwrap();
            done = now;
            if timeout.timed_out() && !*done {
                return Err("The debugger did not finish starting".into());
            }
        }
        drop(done);
        let mut verified = Vec::new();
        for (path, lines) in breakpoints {
            let answer = self.set_breakpoints(&path, &lines)?;
            verified.push((path, answer));
        }
        *self.verified.lock().unwrap() = verified;
        self.set_function_breakpoints(&functions)?;
        // A filter-only launch keeps Passenger's default stop for unhandled exceptions.
        let rules = if exceptions.is_empty() && !filters.is_empty() { None } else { Some(exceptions.as_slice()) };
        self.configure_exception_breakpoints(&filters, rules)?;
        self.connection.request("configurationDone", json!({}), CONFIGURED)?;
        Ok(capabilities)
    }

    pub fn process_id(&self) -> Option<u32> {
        match self.process_id.load(Ordering::Acquire) {
            0 => None,
            id => Some(id as u32),
        }
    }

    /// Sends a file's breakpoints, replacing what the debugger had there; its answer says what it made of each.
    /// Stops at methods by name, wherever they are: Rider's function breakpoints.
    pub fn set_function_breakpoints(&self, breakpoints: &[FunctionBreakpoint]) -> Result<Vec<Breakpoint>, String> {
        let list: Vec<Value> = breakpoints
            .iter()
            .map(|breakpoint| {
                let mut sent = json!({ "name": breakpoint.name });
                if let Some(condition) = &breakpoint.condition {
                    sent["condition"] = json!(condition);
                }
                if let Some(hit_condition) = &breakpoint.hit_condition {
                    sent["hitCondition"] = json!(hit_condition);
                }
                if let Some(log_message) = &breakpoint.log_message {
                    sent["logMessage"] = json!(log_message);
                }
                sent
            })
            .collect();
        let body = self.connection.request("setFunctionBreakpoints", json!({ "breakpoints": list }), REQUEST)?;
        let answered: protocol::Breakpoints = serde_json::from_value(body)
            .map_err(|error| format!("The debugger's function breakpoints did not read: {error}"))?;
        Ok(answered.breakpoints)
    }

    pub fn set_breakpoints(&self, path: &Path, breakpoints: &[SourceBreakpoint]) -> Result<Vec<Breakpoint>, String> {
        let list: Vec<Value> = breakpoints
            .iter()
            .map(|breakpoint| {
                let mut sent = json!({ "line": breakpoint.line + 1 });
                if let Some(column) = breakpoint.column {
                    sent["column"] = json!(column + 1);
                }
                if let Some(condition) = &breakpoint.condition {
                    sent["condition"] = json!(condition);
                }
                if let Some(hit_condition) = &breakpoint.hit_condition {
                    sent["hitCondition"] = json!(hit_condition);
                }
                if let Some(log_message) = &breakpoint.log_message {
                    sent["logMessage"] = json!(log_message);
                }
                sent
            })
            .collect();
        let body = self.connection.request(
            "setBreakpoints",
            json!({ "source": { "path": path }, "breakpoints": list }),
            REQUEST,
        )?;
        let answered: protocol::Breakpoints = serde_json::from_value(body)
            .map_err(|error| format!("The debugger's breakpoints did not read: {error}"))?;
        Ok(answered.breakpoints)
    }

    /// `resolveStackFrame`: a frame with no source, its assembly decompiled by the adapter into the symbol cache. The
    /// first call for an assembly takes as long as the decompile, off the engine's lock, while the session answers.
    pub fn resolve_stack_frame(&self, id: i64) -> Result<Frame, String> {
        let body = self.connection.request("resolveStackFrame", json!({ "stackFrameId": id }), DECOMPILE)?;
        let answered: protocol::ResolveStackFrame = serde_json::from_value(body)
            .map_err(|error| format!("The debugger's resolved frame did not read: {error}"))?;
        Ok(placed(answered.stack_frame))
    }

    /// A resolved frame's source, from the embedded text of the decompiled PDB the adapter wrote: the read-only file
    /// RSharp writes it to under the cache, for a window to open, and its text.
    pub fn decompiled_source(&self, frame: &Frame) -> Result<(PathBuf, String), String> {
        let info = frame.decompiled_source_info.as_ref().ok_or("The frame's source was not decompiled")?;
        let source = frame.path().ok_or("The frame has no decompiled source")?;
        let name = info
            .assembly
            .assembly_path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{} names no assembly", info.assembly.assembly_path.display()))?;
        let mvid: String =
            info.assembly.mvid.chars().filter(char::is_ascii_hexdigit).collect::<String>().to_ascii_lowercase();
        let folder = self.symbols.join(&name).join(&mvid);
        let pdb = std::fs::read_dir(&folder)
            .map_err(|error| format!("The decompiled symbols for {name} are not in {}: {error}", folder.display()))?
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|extension| extension == "pdb"))
            .ok_or_else(|| format!("The decompiled symbols for {name} are not in {}", folder.display()))?;
        let sources = self.decompiled_sources(&pdb)?;
        let wanted = comparable(&source);
        let root = self.symbols.parent().map(Path::to_path_buf).unwrap_or_else(|| self.symbols.clone());
        let (_, text) = sources
            .iter()
            .find(|(path, _)| comparable(Path::new(path)) == wanted || comparable(&root.join(path)) == wanted)
            .ok_or_else(|| format!("{} is not a document of {}", source.display(), pdb.display()))?;
        let target = if source.is_absolute() { source.clone() } else { root.join(&source) };
        write_decompiled(&target, text)?;
        Ok((target, text.clone()))
    }

    /// The embedded sources of a decompiled PDB, read once: each document's path as the PDB names it, and its text.
    fn decompiled_sources(&self, pdb: &Path) -> Result<Arc<Vec<(String, String)>>, String> {
        let mut cache = self.decompiled.lock().unwrap();
        if let Some(sources) = cache.get(pdb) {
            return Ok(sources.clone());
        }
        let bytes = std::fs::read(pdb).map_err(|error| format!("{} did not read: {error}", pdb.display()))?;
        let parsed = symbolic_ppdb::PortablePdb::parse(&bytes)
            .map_err(|error| format!("{} is not a portable PDB: {error}", pdb.display()))?;
        let mut sources = Vec::new();
        for source in parsed
            .get_embedded_sources()
            .map_err(|error| format!("{}'s embedded sources did not read: {error}", pdb.display()))?
        {
            let source =
                source.map_err(|error| format!("{}'s embedded source did not read: {error}", pdb.display()))?;
            let text = source
                .get_contents(Some(64 << 20))
                .map_err(|error| format!("{}'s embedded source did not read: {error}", pdb.display()))?;
            sources.push((source.get_path().to_string(), String::from_utf8_lossy(&text).into_owned()));
        }
        let sources = Arc::new(sources);
        cache.insert(pdb.to_path_buf(), sources.clone());
        Ok(sources)
    }

    /// The DAP exception filters (`all`, `user-unhandled`, each with a condition or none) and the folder's own exception
    /// breakpoints, in one request as the adapter takes them. Answers what the adapter made of the filters.
    /// An empty exception list clears the rules; filter-only callers use `set_exception_filters`.
    pub fn set_exception_breakpoints(
        &self,
        filters: &[(String, Option<String>)],
        exceptions: &[ExceptionFilter],
    ) -> Result<Vec<Breakpoint>, String> {
        self.configure_exception_breakpoints(filters, Some(exceptions))
    }

    fn configure_exception_breakpoints(
        &self,
        filters: &[(String, Option<String>)],
        exceptions: Option<&[ExceptionFilter]>,
    ) -> Result<Vec<Breakpoint>, String> {
        let plain: Vec<&str> =
            filters.iter().filter(|(_, condition)| condition.is_none()).map(|(filter, _)| filter.as_str()).collect();
        let options: Vec<Value> = filters
            .iter()
            .filter_map(|(filter, condition)| {
                condition.as_ref().map(|condition| json!({ "filterId": filter, "condition": condition }))
            })
            .collect();
        let rules: Vec<Value> = exceptions
            .unwrap_or_default()
            .iter()
            .map(|exception| {
                let mut option = json!({
                    "path": [{ "names": exception.path() }],
                    "breakMode": if exception.scenarios.is_empty() { "never" } else { "always" },
                    "scenarios": exception.scenarios,
                });
                if let Some(condition) = &exception.condition {
                    option["condition"] = json!(condition);
                }
                if let Some(hit_condition) = &exception.hit_condition {
                    option["hitCondition"] = json!(hit_condition);
                }
                if let Some(log_message) = &exception.log_message {
                    option["logMessage"] = json!(log_message);
                }
                option
            })
            .collect();
        // The adapter takes filters and rules apart: an empty `exceptionOptions` is a list of no rules, not absent.
        let mut request = json!({ "filters": plain });
        if !options.is_empty() {
            request["filterOptions"] = json!(options);
        }
        if exceptions.is_some() {
            request["exceptionOptions"] = json!(rules);
        }
        let body = self.connection.request("setExceptionBreakpoints", request, REQUEST)?;
        let answered: protocol::Breakpoints = serde_json::from_value(body)
            .map_err(|error| format!("The debugger's exception breakpoints did not read: {error}"))?;
        Ok(answered.breakpoints)
    }

    /// The folder's own exception breakpoints alone, as the window's Settings keep them.
    pub fn set_exceptions(&self, exceptions: &[ExceptionFilter]) -> Result<(), String> {
        self.set_exception_breakpoints(&[], exceptions).map(drop)
    }

    /// The DAP exception filters alone, as Passenger's tests ask them.
    pub fn set_exception_filters(&self, filters: &[(&str, Option<&str>)]) -> Result<Vec<Breakpoint>, String> {
        let filters: Vec<(String, Option<String>)> =
            filters.iter().map(|(filter, condition)| ((*filter).to_string(), condition.map(str::to_string))).collect();
        self.configure_exception_breakpoints(&filters, None)
    }

    /// What the adapter made of each file's breakpoints as the session opened: whether each will be hit, and why not.
    pub fn verified(&self) -> Vec<(PathBuf, Vec<Breakpoint>)> {
        self.verified.lock().unwrap().clone()
    }

    pub fn threads(&self) -> Result<Vec<Thread>, String> {
        let body = self.connection.request("threads", json!({}), REQUEST)?;
        let answered: protocol::Threads =
            serde_json::from_value(body).map_err(|error| format!("The debugger's threads did not read: {error}"))?;
        Ok(answered.threads)
    }

    /// The thread's frames, innermost first, each with its place from 0.
    pub fn stack_trace(&self, thread: i64) -> Result<Vec<Frame>, String> {
        let body = self.connection.request("stackTrace", json!({ "threadId": thread, "startFrame": 0 }), REQUEST)?;
        let answered: protocol::StackTrace =
            serde_json::from_value(body).map_err(|error| format!("The debugger's stack did not read: {error}"))?;
        Ok(answered.stack_frames.into_iter().map(placed).collect())
    }

    /// What the program can take of Hot Reload: the runtime's capabilities and each loaded module, editable or why not.
    pub fn hot_reload_state(&self) -> Result<protocol::HotReloadState, String> {
        let body = self.connection.request("hotReloadState", json!({}), REQUEST)?;
        serde_json::from_value(body).map_err(|error| format!("The debugger's hot reload state did not read: {error}"))
    }

    /// Hot Reload: applies the updates Roslyn's Edit and Continue emitted, as the extension answered them (each module's
    /// metadata, IL and PDB deltas are opaque here, already base64 on the wire), and answers what became of each.
    pub fn apply_changes(&self, updates: Value) -> Result<Vec<protocol::ModuleApplied>, String> {
        let body = self.connection.request("applyChanges", json!({ "updates": updates }), REQUEST)?;
        let answered: protocol::ApplyChanges = serde_json::from_value(body)
            .map_err(|error| format!("The debugger's applied changes did not read: {error}"))?;
        Ok(answered.modules)
    }

    /// The exception a stopped thread is at, as the adapter describes it: its type, message and place.
    pub fn exception_info(&self, thread: i64) -> Result<protocol::ExceptionInfo, String> {
        let body = self.connection.request("exceptionInfo", json!({ "threadId": thread }), REQUEST)?;
        serde_json::from_value(body).map_err(|error| format!("The debugger's exception did not read: {error}"))
    }

    pub fn scopes(&self, frame: i64) -> Result<Vec<Scope>, String> {
        let body = self.connection.request("scopes", json!({ "frameId": frame }), REQUEST)?;
        let answered: protocol::Scopes =
            serde_json::from_value(body).map_err(|error| format!("The debugger's scopes did not read: {error}"))?;
        Ok(answered.scopes)
    }

    pub fn variables(&self, reference: i64) -> Result<Vec<Variable>, String> {
        let body = self.connection.request("variables", json!({ "variablesReference": reference }), REQUEST)?;
        let answered: protocol::Variables =
            serde_json::from_value(body).map_err(|error| format!("The debugger's values did not read: {error}"))?;
        Ok(answered.variables)
    }

    /// What an expression is in a frame: Evaluate, and what a watch asks as it is typed. The adapter evaluates only
    /// where the program is paused, so a frame is always named.
    pub fn evaluate(&self, expression: &str, frame: i64) -> Result<Evaluated, String> {
        self.evaluate_within(expression, frame, REQUEST)
    }

    /// As `evaluate`, waiting as long as `within` says: an expression the debugger has to stop itself takes longer to
    /// answer than a request does.
    pub fn evaluate_within(&self, expression: &str, frame: i64, within: Duration) -> Result<Evaluated, String> {
        let body = self.connection.request(
            "evaluate",
            json!({ "expression": expression, "frameId": frame, "context": "watch" }),
            within,
        )?;
        serde_json::from_value(body).map_err(|error| format!("The debugger's answer did not read: {error}"))
    }

    /// Set Value: an expression's value in a frame, as a local, a member or an element is changed there.
    pub fn set_expression(&self, expression: &str, value: &str, frame: i64) -> Result<protocol::SetValue, String> {
        let body = self.connection.request(
            "setExpression",
            json!({ "expression": expression, "value": value, "frameId": frame }),
            REQUEST,
        )?;
        serde_json::from_value(body).map_err(|error| format!("The debugger's set value did not read: {error}"))
    }

    /// The places a paused thread could go on from in a file at a line (Set Next Statement's question), from 0.
    pub fn goto_targets(&self, path: &Path, line: usize) -> Result<Vec<protocol::GotoTarget>, String> {
        let body =
            self.connection.request("gotoTargets", json!({ "source": { "path": path }, "line": line + 1 }), REQUEST)?;
        let answered: protocol::GotoTargets = serde_json::from_value(body)
            .map_err(|error| format!("The debugger's goto targets did not read: {error}"))?;
        Ok(answered
            .targets
            .into_iter()
            .map(|target| protocol::GotoTarget {
                line: target.line.saturating_sub(1),
                column: target.column.saturating_sub(1),
                ..target
            })
            .collect())
    }

    /// Set Next Statement: the thread goes on from a target [`Session::goto_targets`] gave.
    pub fn set_next_statement(&self, thread: i64, target: i64) -> Result<(), String> {
        self.connection.request("goto", json!({ "threadId": thread, "targetId": target }), REQUEST).map(drop)
    }

    /// Sends the program a line on its own console input, as the internal console does.
    pub fn input(&self, text: &str) -> Result<(), String> {
        self.input_with(text, false)
    }

    /// Sends `command` as it is, as the protocol's own test does: the adapter's refusal is the error.
    pub fn request_raw(&self, command: &str, arguments: serde_json::Value) -> Result<serde_json::Value, String> {
        self.connection.request(command, arguments, REQUEST)
    }

    /// Sends a line and, when `close` says, closes the program's input, as the internal console does at its end.
    pub fn input_with(&self, text: &str, close: bool) -> Result<(), String> {
        self.connection.request("input", json!({ "text": text, "close": close }), REQUEST).map(drop)
    }

    pub fn resume(&self, thread: i64) -> Result<(), String> {
        self.connection.request("continue", json!({ "threadId": thread }), REQUEST).map(drop)
    }

    pub fn step_over(&self, thread: i64) -> Result<(), String> {
        self.connection.request("next", json!({ "threadId": thread }), REQUEST).map(drop)
    }

    pub fn step_into(&self, thread: i64) -> Result<(), String> {
        self.connection.request("stepIn", json!({ "threadId": thread }), REQUEST).map(drop)
    }

    /// The calls a frame's statement can step into, in the order they run (Smart Step Into's question), from 0.
    pub fn step_in_targets(&self, frame: i64) -> Result<Vec<protocol::StepInTarget>, String> {
        let body = self.connection.request("stepInTargets", json!({ "frameId": frame }), REQUEST)?;
        let answered: protocol::StepInTargets = serde_json::from_value(body)
            .map_err(|error| format!("The debugger's step in targets did not read: {error}"))?;
        Ok(answered
            .targets
            .into_iter()
            .map(|target| protocol::StepInTarget {
                line: target.line.saturating_sub(1),
                column: target.column.saturating_sub(1),
                ..target
            })
            .collect())
    }

    /// Smart Step Into: a step into one of the calls [`Session::step_in_targets`] gave.
    pub fn step_into_target(&self, thread: i64, target: i64) -> Result<(), String> {
        self.connection.request("stepIn", json!({ "threadId": thread, "targetId": target }), REQUEST).map(drop)
    }

    pub fn step_out(&self, thread: i64) -> Result<(), String> {
        self.connection.request("stepOut", json!({ "threadId": thread }), REQUEST).map(drop)
    }

    /// Breaks a running program at its next statement.
    pub fn pause(&self, thread: i64) -> Result<(), String> {
        self.connection.request("pause", json!({ "threadId": thread }), REQUEST).map(drop)
    }

    /// Ends the session: a launched program ends, one attached to is let go.
    pub fn disconnect(&self, terminate: bool) {
        let _ =
            self.connection.request("disconnect", json!({ "terminateDebuggee": terminate }), Duration::from_secs(3));
        self.connection.kill();
    }

    /// Whether the adapter's process has ended, without ending it.
    pub fn ended(&self) -> bool {
        self.connection.ended()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.connection.kill();
    }
}

/// A frame as RSharp counts places: lines and columns from 0, the ends exclusive.
fn placed(frame: Frame) -> Frame {
    Frame {
        line: frame.line.saturating_sub(1),
        column: frame.column.saturating_sub(1),
        end_line: frame.end_line.map(|line| line.saturating_sub(1)),
        end_column: frame.end_column.map(|column| column.saturating_sub(1)),
        ..frame
    }
}

/// A path of a decompiled PDB's document as both the adapter and the PDB name it, whatever separator that platform used.
fn comparable(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

/// Writes a decompiled source where a window can open it, read-only, as Passenger shows decompiled code: written again
/// when the decompiler's text changed, its read-only mark taken off for the write.
fn write_decompiled(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("{} was not made: {error}", parent.display()))?;
    }
    if !std::fs::read_to_string(path).is_ok_and(|kept| kept == text) {
        if let Ok(metadata) = std::fs::metadata(path) {
            let mut permissions = metadata.permissions();
            // The file is the person's: make it writable for its owner alone, as it was made read-only for everyone.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                permissions.set_mode(permissions.mode() | 0o200);
            }
            #[cfg(not(unix))]
            {
                #[allow(clippy::permissions_set_readonly_false)]
                permissions.set_readonly(false);
            }
            let _ = std::fs::set_permissions(path, permissions);
        }
        std::fs::write(path, text).map_err(|error| format!("{} was not written: {error}", path.display()))?;
    }
    let mut permissions =
        std::fs::metadata(path).map_err(|error| format!("{} did not read: {error}", path.display()))?.permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(path, permissions)
        .map_err(|error| format!("{} was not made read-only: {error}", path.display()))
}

fn text(body: &Value, name: &str) -> Option<String> {
    body.get(name).and_then(Value::as_str).map(str::to_string)
}

fn number(body: &Value, name: &str) -> Option<i64> {
    body.get(name).and_then(Value::as_i64)
}
