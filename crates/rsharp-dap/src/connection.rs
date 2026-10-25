//! One adapter's process and the conversation with it: the protocol's messages, a `Content-Length` frame at a time, read
//! on a thread of its own and written under a lock, so a request waits off the UI thread for its answer. Every program
//! goes through `rsharp_core::process::spawn`, so the adapter and the program it starts end with the IDE.

use rsharp_core::process::{self, Child};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the adapter says unasked, or its process's end.
#[derive(Debug)]
pub(crate) enum Incoming {
    Event {
        name: String,
        body: Value,
    },
    /// A line the adapter wrote to its error output.
    Stderr(String),
    /// The adapter's output closed: it ended, or crashed.
    Ended,
}

type Pending = Mutex<HashMap<i64, mpsc::Sender<Result<Value, String>>>>;

/// A request the client answers itself, as the adapter's runInTerminal: its body, or why it refused.
pub(crate) type Requests = Arc<dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync>;

pub(crate) struct Connection {
    input: Mutex<ChildStdin>,
    pending: Pending,
    next: AtomicI64,
    child: Mutex<Option<Child>>,
}

impl Connection {
    /// Starts the adapter and reads it on threads of its own, telling `notify` what it says and `requests` what it
    /// asks the client itself.
    pub fn start(
        mut command: Command,
        notify: Arc<dyn Fn(Incoming) + Send + Sync>,
        requests: Requests,
    ) -> io::Result<Arc<Connection>> {
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = process::spawn(command)?;
        let input = child.process.stdin.take().expect("stdin was piped");
        let output = child.process.stdout.take().expect("stdout was piped");
        let errors = child.process.stderr.take().expect("stderr was piped");
        let connection = Arc::new(Connection {
            input: Mutex::new(input),
            pending: Mutex::default(),
            next: AtomicI64::new(1),
            child: Mutex::new(Some(child)),
        });
        let reader = connection.clone();
        let noisy = notify.clone();
        std::thread::Builder::new()
            .name("dap-reader".into())
            .spawn(move || reader.read(BufReader::new(output), &*notify, &*requests))?;
        std::thread::Builder::new().name("dap-stderr".into()).spawn(move || {
            for line in BufReader::new(errors).lines().map_while(Result::ok) {
                noisy(Incoming::Stderr(line));
            }
        })?;
        Ok(connection)
    }

    /// Asks, and waits up to `timeout` for the answer. Called from a thread that may wait: never the UI's.
    pub fn request(&self, command: &str, arguments: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver): (_, Receiver<Result<Value, String>>) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, sender);
        let body = json!({ "seq": id, "type": "request", "command": command, "arguments": arguments });
        if let Err(error) = self.write(&body) {
            self.pending.lock().unwrap().remove(&id);
            return Err(format!("the debugger could not be told {command}: {error}"));
        }
        match receiver.recv_timeout(timeout) {
            Ok(answer) => answer,
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(format!("the debugger did not answer {command} within {} s", timeout.as_secs()))
            }
        }
    }

    /// Ends the adapter and everything it started.
    pub fn kill(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            child.kill();
        }
    }

    /// Whether the adapter's process has ended, without ending it.
    pub fn ended(&self) -> bool {
        let mut child = self.child.lock().unwrap();
        match child.as_mut() {
            Some(child) => matches!(child.process.try_wait(), Ok(Some(_)) | Err(_)),
            None => true,
        }
    }

    fn write(&self, message: &Value) -> io::Result<()> {
        let body = serde_json::to_vec(message)?;
        let mut input = self.input.lock().unwrap();
        write!(input, "Content-Length: {}\r\n\r\n", body.len())?;
        input.write_all(&body)?;
        input.flush()
    }

    fn read(
        self: Arc<Self>,
        mut output: impl BufRead,
        notify: &(dyn Fn(Incoming) + Send + Sync),
        requests: &(dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync),
    ) {
        while let Some(message) = next_message(&mut output) {
            match message.get("type").and_then(Value::as_str) {
                Some("response") => {
                    let id = message.get("request_seq").and_then(Value::as_i64).unwrap_or_default();
                    let answered = self.pending.lock().unwrap().remove(&id);
                    if let Some(answered) = answered {
                        let answer = match message.get("success").and_then(Value::as_bool) {
                            Some(false) => Err(message
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("the debugger refused it")
                                .to_string()),
                            _ => Ok(message.get("body").cloned().unwrap_or(Value::Null)),
                        };
                        let _ = answered.send(answer);
                    }
                }
                Some("event") => {
                    let name = message.get("event").and_then(Value::as_str).unwrap_or_default().to_string();
                    let body = message.get("body").cloned().unwrap_or(Value::Null);
                    notify(Incoming::Event { name, body });
                }
                Some("request") => {
                    // The adapter's own request, as a program in the client's terminal: what the client does not
                    // offer is refused, so the adapter goes on rather than waiting for an answer that never comes.
                    let id = message.get("seq").and_then(Value::as_i64).unwrap_or_default();
                    let what = message.get("command").and_then(Value::as_str).unwrap_or("a request");
                    let arguments = message.get("arguments").cloned().unwrap_or(Value::Null);
                    let mut answer = json!({
                        "seq": self.next.fetch_add(1, Ordering::Relaxed),
                        "type": "response",
                        "request_seq": id,
                        "command": what,
                    });
                    match requests(what, &arguments) {
                        Ok(body) => {
                            answer["success"] = json!(true);
                            answer["body"] = body;
                        }
                        Err(message) => {
                            answer["success"] = json!(false);
                            answer["message"] = json!(message);
                        }
                    }
                    let _ = self.write(&answer);
                }
                _ => {}
            }
        }
        let unanswered: Vec<_> = self.pending.lock().unwrap().drain().map(|(_, answered)| answered).collect();
        for answered in unanswered {
            let _ = answered.send(Err("the debugger stopped".into()));
        }
        notify(Incoming::Ended);
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.kill();
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
    fn a_message_is_a_length_and_json() {
        let body = br#"{"seq":1,"type":"request","command":"initialize"}"#;
        let framed = [format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes(), body.to_vec()].concat();
        let mut input = framed.as_slice();
        let message = next_message(&mut input).expect("one message");
        assert_eq!(message["command"], "initialize");
        assert!(next_message(&mut input).is_none(), "the message ends there");
    }
}
