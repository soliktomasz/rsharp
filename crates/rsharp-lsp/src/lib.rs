//! The Language Server Protocol client (JSON-RPC over a server's standard input and output) and the Roslyn
//! language server it runs for C#. No UI: events come on the reader's thread, and a request's answer is waited
//! for off the UI thread.

mod protocol;
mod roslyn;

pub use protocol::*;
pub use roslyn::*;

use rsharp_core::process::{self, Child};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Why a request got no answer: the server refused it, or stopped. The one failure callers catch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerFailed(pub String);

impl std::fmt::Display for ServerFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ServerFailed {}

pub type Answer = Result<Value, ServerFailed>;

/// What the server tells the client unasked.
#[derive(Debug)]
pub enum ServerEvent {
    Notification {
        method: String,
        params: Value,
    },
    /// A line the server wrote to its error output: how it says why it cannot start, or crashed.
    Stderr(String),
    /// The server's output closed: it ended or crashed.
    Ended,
}

type Pending = Arc<Mutex<HashMap<i64, Sender<Answer>>>>;

pub struct LanguageServer {
    input: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    next_id: AtomicI64,
    /// What the server said it can do, answering `initialize`.
    pub capabilities: Value,
    _child: Child,
}

impl LanguageServer {
    /// Starts the server and reads it on a thread of its own, telling `events` what it says unasked.
    pub fn start(mut command: Command, events: impl Fn(ServerEvent) + Send + Sync + 'static) -> io::Result<Self> {
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = process::spawn(command)?;
        let input = Arc::new(Mutex::new(child.process.stdin.take().expect("stdin was piped")));
        let output = child.process.stdout.take().expect("stdout was piped");
        let errors = child.process.stderr.take().expect("stderr was piped");
        let events = Arc::new(events);
        let error_events = events.clone();
        std::thread::Builder::new().name("lsp-stderr".into()).spawn(move || {
            for line in BufReader::new(errors).lines().map_while(Result::ok) {
                error_events(ServerEvent::Stderr(line));
            }
        })?;
        let pending: Pending = Arc::default();
        let reader_input = input.clone();
        let reader_pending = pending.clone();
        std::thread::Builder::new()
            .name("lsp-reader".into())
            .spawn(move || read(BufReader::new(output), &reader_input, &reader_pending, &*events))?;
        Ok(Self { input, pending, next_id: AtomicI64::new(1), capabilities: Value::Null, _child: child })
    }

    /// Sends a request; the answer arrives on the returned channel.
    pub fn request(&self, method: &str, params: Value) -> Receiver<Answer> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, sender);
        if let Err(error) =
            write(&self.input, &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
        {
            if let Some(sender) = self.pending.lock().unwrap().remove(&id) {
                let _ = sender.send(Err(ServerFailed(format!("the server could not be told {method}: {error}"))));
            }
        }
        receiver
    }

    pub fn notify(&self, method: &str, params: Value) {
        // A notification lost because the server ended is reported by its Ended event.
        let _ = write(&self.input, &json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }
}

fn write(input: &Mutex<ChildStdin>, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message)?;
    let mut input = input.lock().unwrap();
    write!(input, "Content-Length: {}\r\n\r\n", body.len())?;
    input.write_all(&body)?;
    input.flush()
}

fn read(mut output: impl BufRead, input: &Mutex<ChildStdin>, pending: &Pending, events: &dyn Fn(ServerEvent)) {
    while let Some(message) = next_message(&mut output) {
        let id = message.get("id").and_then(Value::as_i64);
        match (message.get("method").and_then(Value::as_str), id) {
            (Some(method), Some(id)) => {
                log::info!(target: "lsp", "server asks {method}: {}", message["params"]);
                let _ = write(
                    input,
                    &json!({ "jsonrpc": "2.0", "id": id, "result": answer_for(method, &message["params"]) }),
                );
            }
            (Some(method), None) => events(ServerEvent::Notification {
                method: method.to_string(),
                params: message.get("params").cloned().unwrap_or(Value::Null),
            }),
            (None, Some(id)) => {
                if let Some(sender) = pending.lock().unwrap().remove(&id) {
                    let answer = match message.get("error") {
                        Some(error) => Err(ServerFailed(error["message"].as_str().unwrap_or("refused").to_string())),
                        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                    };
                    let _ = sender.send(answer);
                }
            }
            (None, None) => {}
        }
    }
    for (_, sender) in pending.lock().unwrap().drain() {
        let _ = sender.send(Err(ServerFailed("the language server stopped".into())));
    }
    events(ServerEvent::Ended);
}

/// What the client answers a server's own request: Passenger's settings are the server's defaults, so each
/// configuration item is null, and every registration or progress token is simply accepted.
fn answer_for(method: &str, params: &Value) -> Value {
    match method {
        "workspace/configuration" => Value::Array(vec![Value::Null; params["items"].as_array().map_or(0, Vec::len)]),
        _ => Value::Null,
    }
}

/// One framed message, or `None` once the output closes or stops making sense.
fn next_message(output: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if output.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = value.trim().parse().ok();
        }
    }
    let mut body = vec![0; length?];
    output.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roslyn_answers_initialize_and_says_it_has_loaded_a_project() {
        let Some(_) = roslyn_server() else {
            eprintln!("skipped: Roslyn's language server is not installed");
            return;
        };
        let folder = std::env::temp_dir().join(format!("rsharp-lsp-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("App.csproj"), r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework><OutputType>Exe</OutputType></PropertyGroup></Project>"#).unwrap();
        std::fs::write(folder.join("Program.cs"), "System.Console.WriteLine(1);\n").unwrap();
        let folder = rsharp_core::paths::canonical(&folder);

        let (sender, events) = mpsc::channel();
        let server = start_roslyn(&folder, None, &folder.join("logs"), move |event| {
            let _ = sender.send(event);
        })
        .expect("Roslyn started and answered initialize");
        // What Roslyn said on the way, so a failure names why it never loaded.
        let mut said = Vec::new();
        let loaded =
            std::iter::from_fn(|| events.recv_timeout(std::time::Duration::from_secs(180)).ok()).any(|event| {
                match event {
                    ServerEvent::Notification { method, .. } if method == "workspace/projectInitializationComplete" => {
                        return true;
                    }
                    // Every message, information included: where a stalled load stopped is only said at that level.
                    ServerEvent::Notification { method, params } => {
                        let text = params["message"].as_str().or(params["value"]["message"].as_str());
                        said.push(format!("{method}: {}", text.unwrap_or_default()));
                    }
                    ServerEvent::Stderr(line) => said.push(format!("stderr: {line}")),
                    ServerEvent::Ended => said.push("the server ended".into()),
                }
                false
            });
        drop(server);
        let _ = std::fs::remove_dir_all(&folder);
        assert!(loaded, "Roslyn never said the folder's project had loaded; it said:\n{}", said.join("\n"));
    }
}
