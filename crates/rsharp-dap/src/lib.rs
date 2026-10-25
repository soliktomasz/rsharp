//! The Debug Adapter Protocol client and Passenger's debug adapter, which this runs as Passenger does: its engine behind
//! the protocol in a process of its own. No UI: events come on the adapter's threads, and a request's answer is waited
//! for off the UI thread.

mod connection;
pub mod protocol;

use connection::{Connection, Incoming};
pub use protocol::Capabilities;
use protocol::{Breakpoint, Evaluated, Frame, Scope, Thread, Variable};
use serde_json::{Value, json};
use std::collections::BTreeMap;
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
    pub condition: Option<String>,
    pub log_message: Option<String>,
}

/// What a program is started with: the assembly or executable, its arguments, folder and environment.
#[derive(Clone, Debug)]
pub struct Launch {
    pub program: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub stop_at_entry: bool,
}

/// How a session starts: a program of its own, or a running one attached to.
#[derive(Clone, Debug)]
pub enum Start {
    Launch(Launch),
    Attach { process_id: u32 },
}

const INITIALIZE: Duration = Duration::from_secs(20);
const LAUNCH: Duration = Duration::from_secs(30);
const INITIALIZED: Duration = Duration::from_secs(60);
const CONFIGURED: Duration = Duration::from_secs(120);
const REQUEST: Duration = Duration::from_secs(20);

/// One program under the debugger: the adapter's process, the conversation with it, and the requests the debug window
/// makes. Every method is safe to call off the UI thread and none needs the UI.
pub struct Session {
    connection: Arc<Connection>,
    /// The process the adapter started, once it said which it is.
    process_id: Arc<AtomicI64>,
    pub capabilities: Capabilities,
    /// What the program is, as the Debug window names it.
    pub program: String,
}

impl Session {
    /// Starts the adapter and the program (or attaches it), with the folder and its lines as breakpoints. The session is
    /// ready once the adapter's `configurationDone` has been answered, which waits for the program's runtime.
    pub fn start(
        adapter: &Path,
        start: Start,
        symbols: &Path,
        breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)>,
        events: impl Fn(Event) + Send + Sync + 'static,
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
        let connection =
            Connection::start(command, notify).map_err(|error| format!("The debugger did not start: {error}"))?;
        let mut session = Session {
            connection,
            process_id,
            capabilities: Capabilities::default(),
            program: match &start {
                Start::Launch(launch) => launch.program.file_stem().unwrap_or_default().to_string_lossy().into_owned(),
                Start::Attach { process_id } => format!("process {process_id}"),
            },
        };
        match session.begin(start, breakpoints, &initialized) {
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
                "supportsRunInTerminalRequest": false,
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
                    "console": "internalConsole",
                    "stopAtEntry": launch.stop_at_entry,
                    "justMyCode": true,
                }),
                LAUNCH,
            )?,
            Start::Attach { process_id } => {
                self.connection.request("attach", json!({ "processId": process_id, "justMyCode": true }), LAUNCH)?
            }
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
        for (path, lines) in breakpoints {
            self.set_breakpoints(&path, &lines)?;
        }
        // No filters: the adapter's own answer, that an exception nothing handles stops the program.
        self.connection.request("setExceptionBreakpoints", json!({}), REQUEST)?;
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
    pub fn set_breakpoints(&self, path: &Path, breakpoints: &[SourceBreakpoint]) -> Result<Vec<Breakpoint>, String> {
        let list: Vec<Value> = breakpoints
            .iter()
            .map(|breakpoint| {
                let mut sent = json!({ "line": breakpoint.line + 1 });
                if let Some(condition) = &breakpoint.condition {
                    sent["condition"] = json!(condition);
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
        Ok(answered
            .stack_frames
            .into_iter()
            .map(|frame| Frame { line: frame.line.saturating_sub(1), column: frame.column.saturating_sub(1), ..frame })
            .collect())
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
        let body = self.connection.request(
            "evaluate",
            json!({ "expression": expression, "frameId": frame, "context": "watch" }),
            REQUEST,
        )?;
        serde_json::from_value(body).map_err(|error| format!("The debugger's answer did not read: {error}"))
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

fn text(body: &Value, name: &str) -> Option<String> {
    body.get(name).and_then(Value::as_str).map(str::to_string)
}

fn number(body: &Value, name: &str) -> Option<i64> {
    body.get(name).and_then(Value::as_i64)
}
