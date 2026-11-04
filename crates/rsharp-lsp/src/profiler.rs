//! RSharp's profiler host (`profiler-host/`, Passenger.Profiler in a process of its own): a program started under it, or
//! one already running attached to, its output as it comes and, when the run ends, its snapshot as the host's JSON (the
//! call tree of each thread with samples in and under each node, hot spots, collections, allocated types).

use crate::ServerFailed;
use crate::host::Connection;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

/// A program to profile, as the host starts it.
pub struct Target {
    pub program: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: PathBuf,
    pub environment: BTreeMap<String, String>,
}

/// How a run began: the program's process, the trace it writes, and what the snapshot is named for.
#[derive(Clone, Debug, PartialEq)]
pub struct Started {
    pub pid: i64,
    pub trace: PathBuf,
    pub name: String,
    pub attached: bool,
}

/// How a run ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Ended {
    /// The program ended by itself: the run's snapshot.
    Exited(Value),
    /// [`Profiler::stop`] took the snapshot while the program went on.
    Stopped,
}

/// One profiling run's host. It ends with the run, or when this is dropped, the program it started with it.
#[derive(Clone)]
pub struct Profiler {
    connection: Arc<Connection>,
}

/// The host RSharp's build publishes, if it did.
fn carried() -> Option<PathBuf> {
    let sidecars = env!("RSHARP_SIDECARS");
    let host = Path::new(sidecars).join("profiler").join("RSharp.ProfilerHost.dll");
    (!sidecars.is_empty() && host.is_file()).then_some(host)
}

impl Profiler {
    /// Starts the host; its own failure (no dotnet, no host) says why. Blocks until it is ready: call it off the UI thread.
    pub fn start() -> Result<Self, ServerFailed> {
        let host = carried().ok_or_else(|| ServerFailed("RSharp's build carries no profiler".into()))?;
        let dotnet = rsharp_core::programs::dotnet().ok_or_else(|| ServerFailed("dotnet was not found".into()))?;
        let mut command = Command::new(dotnet);
        command.arg(host).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        Ok(Self { connection: Connection::start(command, "the profiler", None)? })
    }

    /// Starts `target` held at the run's diagnostic port, its trace written into `folder`.
    pub fn profile(&self, target: &Target, folder: &Path) -> Result<Started, String> {
        self.begin(
            "start",
            json!({
                "program": target.program, "arguments": target.arguments, "workingDirectory": target.working_directory,
                "environment": target.environment, "folder": folder,
            }),
        )
    }

    /// Attaches to the running process `pid`; the program goes on as it was after.
    pub fn attach(&self, pid: u32, name: &str, folder: &Path) -> Result<Started, String> {
        self.begin("attach", json!({ "pid": pid, "name": name, "folder": folder }))
    }

    fn begin(&self, method: &str, params: Value) -> Result<Started, String> {
        let started = self.connection.ask(method, params, |_| {})?;
        Ok(Started {
            pid: started["pid"].as_i64().unwrap_or_default(),
            trace: PathBuf::from(started["trace"].as_str().unwrap_or_default()),
            name: started["name"].as_str().unwrap_or_default().to_string(),
            attached: started["attached"] == true,
        })
    }

    /// Follows the run to its end, the program's output given to `output` as it comes (with whether it is on standard
    /// error). Blocks for the whole run: call it off the UI thread.
    pub fn run(&self, mut output: impl FnMut(&str, bool)) -> Result<Ended, String> {
        let ended = self.connection.ask("run", json!({}), |partial| {
            output(partial["text"].as_str().unwrap_or_default(), partial["error"] == true)
        })?;
        Ok(if ended["stopped"] == true { Ended::Stopped } else { Ended::Exited(ended) })
    }

    /// Ends the run, the program going on, and reads what it found.
    pub fn stop(&self) -> Result<Value, String> {
        self.connection.ask("stop", json!({}), |_| {})
    }

    /// What the program's heap holds now (the host's JSON: number, bytes, objects and the types by size); the run goes on.
    pub fn memory(&self) -> Result<Value, String> {
        self.connection.ask("memory", json!({}), |_| {})
    }

    /// Two memory snapshots of the run compared object by object, by their numbers.
    pub fn compare(&self, older: i64, newer: i64) -> Result<Value, String> {
        self.connection.ask("compare", json!({ "older": older, "newer": newer }), |_| {})
    }

    /// Types to the program's standard input.
    pub fn input(&self, text: &str) -> Result<(), String> {
        self.connection.ask("input", json!({ "text": text }), |_| {}).map(drop)
    }

    /// Ends the program, unless it was attached to.
    pub fn kill(&self) -> Result<(), String> {
        self.connection.ask("kill", json!({}), |_| {}).map(drop)
    }
}

/// Whether RSharp's build carries the profiler host.
pub fn available() -> bool {
    carried().is_some()
}
