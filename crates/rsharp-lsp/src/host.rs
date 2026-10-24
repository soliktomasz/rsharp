//! Passenger's project Host: one reader routes replies and one writer sends complete JSON lines, so Roslyn,
//! project evaluation and an external build's hold share the folder's one process without blocking each other.

use crate::ServerFailed;
use rsharp_core::msbuild::insight::ProjectInsight;
use rsharp_core::process::{self, Child};
use rsharp_core::project::EvaluatedProject;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::Duration;

const POLL: Duration = Duration::from_millis(20);
/// A Host silent for this long has wedged. Its start holds the folder's one connection, so every other caller waits on
/// it: an unbounded wait would leave them waiting for ever, and a build's hold unable to begin.
const READY_TIMEOUT: Duration = Duration::from_secs(120);
type Reply = Result<Value, ServerFailed>;

/// A project's design-time builds, or why there are none, as Roslyn's server takes them from the extension.
pub struct Built {
    pub infos: Option<String>,
    pub problem: Option<String>,
    pub diagnostics: Value,
}

#[derive(Default)]
struct Replies {
    next_id: i64,
    pending: HashMap<i64, Sender<Reply>>,
    failed: Option<ServerFailed>,
}

impl Replies {
    fn fail(&mut self, error: ServerFailed) {
        if self.failed.is_none() {
            self.failed = Some(error.clone());
            for (_, sender) in self.pending.drain() {
                let _ = sender.send(Err(error.clone()));
            }
        }
    }
}

struct Connection {
    input: Sender<Value>,
    replies: Arc<Mutex<Replies>>,
    pins: AtomicUsize,
    child: Mutex<Child>,
}

struct Request {
    id: i64,
    replies: Receiver<Reply>,
}

impl Connection {
    fn failed(&self) -> Option<ServerFailed> {
        self.replies.lock().unwrap().failed.clone()
    }

    fn request(&self, method: &str, params: Value) -> Result<Request, ServerFailed> {
        let (sender, replies) = mpsc::channel();
        let id = {
            let mut state = self.replies.lock().unwrap();
            if let Some(error) = &state.failed {
                return Err(error.clone());
            }
            state.next_id += 1;
            let id = state.next_id;
            state.pending.insert(id, sender);
            id
        };
        let request = json!({ "id": id, "method": method, "params": params });
        if let Err(error) = self.input.send(request) {
            let error = ServerFailed(format!("the project host could not be asked: {error}"));
            self.replies.lock().unwrap().fail(error.clone());
            return Err(error);
        }
        Ok(Request { id, replies })
    }

    fn ask(&self, method: &str, params: Value, mut partial: impl FnMut(&Value)) -> Result<Value, String> {
        let request = self.request(method, params).map_err(|e| e.0)?;
        loop {
            let reply = request.replies.recv().map_err(|_| "the project host stopped".to_string())?.map_err(|e| e.0)?;
            if let Some(error) = reply["error"].as_str() {
                return Err(error.to_string());
            }
            if let Some(value) = reply.get("partial") {
                partial(value);
            }
            if let Some(result) = reply.get("result") {
                return Ok(result.clone());
            }
        }
    }
}

#[derive(Clone)]
pub struct ProjectHost {
    connection: Arc<Connection>,
}

impl ProjectHost {
    /// Starts the Host on `folder` and reads its first line, the SDK it loaded or why it loaded none.
    pub fn start(dotnet: &Path, host: &Path, folder: &Path) -> Result<Self, ServerFailed> {
        Self::start_cancellable(dotnet, host, folder, None)
    }

    fn start_cancellable(
        dotnet: &Path,
        host: &Path,
        folder: &Path,
        cancel: Option<&AtomicBool>,
    ) -> Result<Self, ServerFailed> {
        let mut command = Command::new(dotnet);
        command
            .arg(host)
            .arg("--folder")
            .arg(folder)
            .current_dir(folder)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        Self::start_command(command, cancel)
    }

    fn start_command(mut command: Command, cancel: Option<&AtomicBool>) -> Result<Self, ServerFailed> {
        rsharp_core::dotnet::prepare(&mut command);
        let mut child =
            process::spawn(command).map_err(|e| ServerFailed(format!("the project host did not start: {e}")))?;
        let mut input = child.process.stdin.take().expect("stdin was piped");
        let output = BufReader::new(child.process.stdout.take().expect("stdout was piped"));
        let (sender, ready) = mpsc::channel();
        let replies = Arc::new(Mutex::new(Replies { pending: HashMap::from([(0, sender)]), ..Replies::default() }));
        let (input_tx, requests) = mpsc::channel();
        let connection = Arc::new(Connection {
            input: input_tx,
            replies: replies.clone(),
            pins: AtomicUsize::new(0),
            child: Mutex::new(child),
        });
        let writing_replies = replies.clone();
        std::thread::spawn(move || {
            for request in requests {
                if let Err(error) = writeln!(input, "{request}").and_then(|_| input.flush()) {
                    writing_replies
                        .lock()
                        .unwrap()
                        .fail(ServerFailed(format!("the project host could not be asked: {error}")));
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let read = || -> Result<(), ServerFailed> {
                for line in output.lines() {
                    let line = line.map_err(|e| ServerFailed(format!("the project host could not be read: {e}")))?;
                    let reply: Value = serde_json::from_str(&line)
                        .map_err(|e| ServerFailed(format!("the project host said what is not JSON: {e}")))?;
                    let id = reply["id"]
                        .as_i64()
                        .ok_or_else(|| ServerFailed("the project host's reply has no request id".into()))?;
                    let terminal = reply.get("result").is_some() || reply["error"].is_string();
                    if !terminal && reply.get("partial").is_none() {
                        return Err(ServerFailed("the project host's reply has no result".into()));
                    }
                    let mut state = replies.lock().unwrap();
                    if terminal {
                        if let Some(sender) = state.pending.remove(&id) {
                            let _ = sender.send(Ok(reply));
                        }
                    } else if let Some(sender) = state.pending.get(&id) {
                        let _ = sender.send(Ok(reply));
                    }
                }
                Err(ServerFailed("the project host stopped".into()))
            };
            if let Err(error) = read() {
                replies.lock().unwrap().fail(error);
            }
        });
        let began = std::time::Instant::now();
        loop {
            if cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
                return Err(ServerFailed("cancelled".into()));
            }
            if began.elapsed() > READY_TIMEOUT {
                return Err(ServerFailed(format!(
                    "the project host did not name its SDK within {} seconds",
                    READY_TIMEOUT.as_secs()
                )));
            }
            match ready.recv_timeout(POLL) {
                Ok(Ok(reply)) => {
                    if let Some(error) = reply["error"].as_str() {
                        return Err(ServerFailed(format!("the project host loaded no SDK: {error}")));
                    }
                    return Ok(Self { connection });
                }
                Ok(Err(error)) => return Err(error),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(ServerFailed("the project host stopped".into())),
            }
        }
    }

    /// One project's design-time builds, with the global properties a build of its solution gives it.
    pub fn build(&mut self, project: &Path, properties: &Value) -> Result<Built, ServerFailed> {
        let mut built = None;
        self.connection
            .ask("build", json!({ "projects": [project], "properties": properties }), |partial| {
                built = Some(Built {
                    infos: partial["infos"].as_str().map(str::to_string),
                    problem: partial["problem"].as_str().map(str::to_string),
                    diagnostics: partial.get("diagnostics").cloned().unwrap_or(Value::Null),
                });
            })
            .map_err(|e| ServerFailed(format!("the project host could not build {}: {e}", project.display())))?;
        built.ok_or_else(|| ServerFailed(format!("the project host built nothing of {}", project.display())))
    }

    /// One project as MSBuild evaluates it, every framework of it, with the solution's global properties.
    pub fn evaluate(&mut self, project: &Path, properties: &Value) -> Result<EvaluatedProject, ServerFailed> {
        let mut evaluated: Option<EvaluatedProject> = None;
        let mut unread = None;
        self.connection
            .ask("evaluate", json!({ "projects": [project], "properties": properties }), |partial| {
                match serde_json::from_value::<EvaluatedProject>(partial.clone()) {
                    Ok(project) if project.complete || evaluated.is_none() => evaluated = Some(project),
                    Ok(_) => {}
                    Err(error) => unread = Some(error),
                }
            })
            .map_err(|e| ServerFailed(format!("the project host could not evaluate {}: {e}", project.display())))?;
        evaluated.ok_or_else(|| match unread {
            Some(error) => {
                ServerFailed(format!("the project host's evaluation of {} did not read: {error}", project.display()))
            }
            None => ServerFailed(format!("the project host evaluated nothing of {}", project.display())),
        })
    }

    /// One project evaluated for code help, with the open files' unsaved text in place of the disk's.
    pub fn inspect(
        &mut self,
        project: &Path,
        properties: &Value,
        texts: &HashMap<PathBuf, String>,
    ) -> Result<ProjectInsight, ServerFailed> {
        let texts: serde_json::Map<String, Value> =
            texts.iter().map(|(p, t)| (p.to_string_lossy().into_owned(), Value::String(t.clone()))).collect();
        let result = self
            .connection
            .ask("inspect", json!({ "project": project, "properties": properties, "texts": texts }), |_| {})
            .map_err(|e| ServerFailed(format!("the project host could not inspect {}: {e}", project.display())))?;
        serde_json::from_value(result).map_err(|e| {
            ServerFailed(format!("the project host's inspection of {} did not read: {e}", project.display()))
        })
    }
}

/// Holds generating design-time builds off. Drop releases it without waiting on the caller's thread.
pub struct BuildHold {
    connection: Arc<Connection>,
    request: Option<Request>,
}

impl BuildHold {
    /// Connection loss means the external build must stop before another Host may start.
    pub fn failed(&self) -> Option<ServerFailed> {
        self.connection.failed()
    }
}

impl Drop for BuildHold {
    fn drop(&mut self) {
        let Some(request) = self.request.take() else { return };
        let connection = self.connection.clone();
        std::thread::spawn(move || {
            let _ = connection.request("cancel", json!({ "request": request.id }));
            // The cancel acknowledgement is separate: only the hold's terminal reply releases the pin.
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                match request.replies.recv_timeout(remaining) {
                    Ok(Ok(reply)) if reply.get("result").is_some() || reply["error"].is_string() => break,
                    Ok(Ok(_)) => {}
                    Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {
                        connection
                            .replies
                            .lock()
                            .unwrap()
                            .fail(ServerFailed("the project host did not release its build hold".into()));
                        connection.child.lock().unwrap().kill();
                        break;
                    }
                }
            }
            connection.pins.fetch_sub(1, Ordering::AcqRel);
        });
    }
}

/// The folder's one Host, started on first use and replaced after transport loss once every hold has ended.
#[derive(Clone)]
pub struct SharedHost {
    running: Arc<Mutex<Option<ProjectHost>>>,
    dotnet: PathBuf,
    host: PathBuf,
    folder: PathBuf,
}

impl SharedHost {
    /// None without Passenger's sidecars or dotnet: Roslyn then finds the projects itself.
    pub fn carried(folder: &Path) -> Option<Self> {
        let host = crate::roslyn::carried_host()?;
        let dotnet = rsharp_core::programs::dotnet()?;
        Some(Self { running: Arc::default(), dotnet, host, folder: folder.to_path_buf() })
    }

    fn connection(&self, cancel: Option<&AtomicBool>, pin: bool) -> Result<ProjectHost, ServerFailed> {
        let mut running = loop {
            if cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
                return Err(ServerFailed("cancelled".into()));
            }
            if cancel.is_none() {
                break self.running.lock().unwrap();
            }
            match self.running.try_lock() {
                Ok(running) => break running,
                Err(TryLockError::WouldBlock) => std::thread::sleep(POLL),
                Err(TryLockError::Poisoned(error)) => panic!("project Host lock poisoned: {error}"),
            }
        };
        if let Some(host) = running.as_ref()
            && let Some(error) = host.connection.failed()
        {
            if host.connection.pins.load(Ordering::Acquire) > 0 {
                return Err(error);
            }
            *running = None;
        }
        if running.is_none() {
            *running = Some(match cancel {
                Some(_) => ProjectHost::start_cancellable(&self.dotnet, &self.host, &self.folder, cancel)?,
                None => ProjectHost::start(&self.dotnet, &self.host, &self.folder)?,
            });
        }
        let host = running.as_ref().expect("started above").clone();
        if pin {
            host.connection.pins.fetch_add(1, Ordering::AcqRel);
        }
        Ok(host)
    }

    fn with<T>(&self, ask: impl FnOnce(&mut ProjectHost) -> Result<T, ServerFailed>) -> Result<T, ServerFailed> {
        ask(&mut self.connection(None, false)?)
    }

    /// Waits off the UI thread until generating design-time builds have drained, or Stop is requested.
    pub fn hold(&self, cancel: &AtomicBool, mut waiting: impl FnMut(&[PathBuf])) -> Result<BuildHold, ServerFailed> {
        let connection = self.connection(Some(cancel), true)?.connection;
        let request = match connection.request("hold", json!({})) {
            Ok(request) => request,
            Err(error) => {
                connection.pins.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let guard = BuildHold { connection, request: Some(request) };
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err(ServerFailed("cancelled".into()));
            }
            match guard.request.as_ref().unwrap().replies.recv_timeout(POLL) {
                Ok(Ok(reply)) => {
                    if let Some(error) = reply["error"].as_str() {
                        return Err(ServerFailed(format!("the project host could not hold builds: {error}")));
                    }
                    if let Some(partial) = reply.get("partial") {
                        if partial["held"].as_bool() == Some(true) {
                            if cancel.load(Ordering::Acquire) {
                                return Err(ServerFailed("cancelled".into()));
                            }
                            return Ok(guard);
                        }
                        let paths: Vec<PathBuf> = serde_json::from_value(partial["running"].clone()).map_err(|e| {
                            ServerFailed(format!("the project host's waiting projects did not read: {e}"))
                        })?;
                        waiting(&paths);
                    }
                    if reply.get("result").is_some() {
                        return Err(ServerFailed("the project host ended its build hold before acquiring it".into()));
                    }
                }
                Ok(Err(error)) => return Err(error),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(ServerFailed("the project host stopped".into())),
            }
        }
    }

    /// Blocks for this request's replies, so call it off the UI thread.
    pub fn build(&self, project: &Path, properties: &Value) -> Result<Built, ServerFailed> {
        self.with(|host| host.build(project, properties))
    }

    /// Blocks for this request's replies, so call it off the UI thread.
    pub fn evaluate(&self, project: &Path, solution: Option<&Path>) -> Result<EvaluatedProject, ServerFailed> {
        let properties = crate::roslyn::solution_properties(solution);
        self.with(|host| host.evaluate(project, &properties))
    }

    /// Blocks for this request's replies, so call it off the UI thread.
    pub fn inspect(
        &self,
        project: &Path,
        solution: Option<&Path>,
        texts: &HashMap<PathBuf, String>,
    ) -> Result<ProjectInsight, ServerFailed> {
        let properties = crate::roslyn::solution_properties(solution);
        self.with(|host| host.inspect(project, &properties, texts))
    }
}

#[cfg(test)]
mod live_tests;

#[cfg(all(test, unix))]
mod tests;
