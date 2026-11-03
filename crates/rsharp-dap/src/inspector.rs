//! RSharp's end of Passenger's inspector inside an Avalonia app it debugs (Passenger's LiveApp, `src/Passenger.Inspector`
//! carried under `sidecars/inspector`): a socket of the launch's own named in the app's environment with the inspector
//! as its startup hook; from before Main the app tells of failed bindings, and once its UI is up says hello and answers
//! requests (JSON lines, protocol version 5) on its UI thread, which a paused app does not. Requests and their answers
//! are the protocol's own JSON; what the app says unasked comes as [`Told`] on a thread of the connection's own.

use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The protocol both ends speak; an app saying another is refused.
pub const VERSION: i64 = 5;

/// How long an answer is waited for: an app paused under the debugger answers nothing until it runs again.
const ANSWER: Duration = Duration::from_secs(10);

/// What the app says unasked.
#[derive(Clone, Debug, PartialEq)]
pub enum Told {
    /// Its UI is up: its process, its Avalonia, and why the inspector cannot serve it when it cannot.
    Hello { process: Option<i64>, avalonia: Option<String>, problem: Option<String> },
    /// Controls came or went since its tree was last asked for.
    TreeChanged,
    /// Select Element ended: the element pointed at, or none for Escape.
    Picked(Option<i64>),
    /// A binding failed, as the protocol's BindingFailure says it, first or again (its count).
    BindingFailed(Value),
    /// A property whose binding the last values read showed changed since.
    ValuesChanged,
    /// The app went.
    Ended,
}

/// The inspector RSharp's build carries, if it does.
pub fn carried() -> Option<PathBuf> {
    Some(PathBuf::from(env!("RSHARP_INSPECTOR"))).filter(|path| path.is_file())
}

/// Whether a program is an Avalonia app the inspector can serve: Avalonia.Base beside its assembly.
pub fn inspects(built: &Path) -> bool {
    built.parent().is_some_and(|folder| folder.join("Avalonia.Base.dll").is_file()) && carried().is_some()
}

/// One launch's inspector: the socket it listens on, the app once connected, and the answers waited for.
pub struct LiveApp {
    socket: PathBuf,
    writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    waiting: Arc<Mutex<HashMap<i64, Sender<Value>>>>,
    next: AtomicI64,
}

static MADE: AtomicUsize = AtomicUsize::new(0);

impl LiveApp {
    /// Listens for the app, `told` called on the connection's thread with what it says unasked; the variables the launch
    /// sets so the app loads the inspector and connects (after any startup hooks `environment` names already).
    #[cfg(unix)]
    pub fn listen(
        environment: &BTreeMap<String, String>,
        told: impl Fn(Told) + Send + 'static,
    ) -> std::io::Result<(Self, BTreeMap<String, String>)> {
        use std::os::unix::net::UnixListener;
        let inspector = carried().ok_or_else(|| std::io::Error::other("RSharp's build carries no inspector"))?;
        let made = MADE.fetch_add(1, Ordering::Relaxed);
        // A short name: macOS takes no socket path over 104 bytes.
        let socket = std::env::temp_dir().join(format!("rsharp-{}-{made}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket)?;
        let writer: Arc<Mutex<Option<Box<dyn Write + Send>>>> = Arc::default();
        let waiting: Arc<Mutex<HashMap<i64, Sender<Value>>>> = Arc::default();
        let (to, answers) = (writer.clone(), waiting.clone());
        std::thread::Builder::new().name("inspector".into()).spawn(move || {
            let Ok((stream, _)) = listener.accept() else { return };
            if let Ok(clone) = stream.try_clone() {
                *to.lock().unwrap() = Some(Box::new(clone));
            }
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                let Ok(reply) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(id) = reply["id"].as_i64() {
                    if let Some(answer) = answers.lock().unwrap().remove(&id) {
                        let _ = answer.send(reply);
                    }
                    continue;
                }
                match reply["event"].as_str() {
                    Some("hello") => {
                        let hello = &reply["hello"];
                        let refused = (hello["protocol"].as_i64() != Some(VERSION)).then(|| {
                            format!("The app's inspector speaks protocol {}, RSharp {VERSION}", hello["protocol"])
                        });
                        told(Told::Hello {
                            process: hello["processId"].as_i64(),
                            avalonia: hello["avalonia"].as_str().map(str::to_string),
                            problem: hello["problem"].as_str().map(str::to_string).or(refused),
                        })
                    }
                    Some("treeChanged") => told(Told::TreeChanged),
                    Some("picked") => told(Told::Picked(reply["element"].as_i64())),
                    Some("pickEnded") => told(Told::Picked(None)),
                    Some("bindingFailed") => told(Told::BindingFailed(reply["binding"].clone())),
                    Some("valuesChanged") => told(Told::ValuesChanged),
                    _ => {}
                }
            }
            *to.lock().unwrap() = None;
            answers.lock().unwrap().clear();
            told(Told::Ended);
        })?;
        let hooks = match environment
            .get("DOTNET_STARTUP_HOOKS")
            .cloned()
            .or_else(|| std::env::var("DOTNET_STARTUP_HOOKS").ok())
        {
            Some(others) if !others.is_empty() => {
                format!("{}{}{others}", inspector.display(), if cfg!(windows) { ';' } else { ':' })
            }
            _ => inspector.display().to_string(),
        };
        let variables = BTreeMap::from([
            ("PASSENGER_INSPECTOR".to_string(), socket.display().to_string()),
            ("DOTNET_STARTUP_HOOKS".to_string(), hooks),
        ]);
        Ok((Self { socket, writer, waiting, next: AtomicI64::new(1) }, variables))
    }

    // shortcut: no Unix domain socket in Rust's standard library on Windows, so no Live Visual Tree there; listen through
    // a crate for Windows' AF_UNIX (or a named pipe the inspector also speaks) once Windows users debug Avalonia apps.
    #[cfg(not(unix))]
    pub fn listen(
        _: &BTreeMap<String, String>,
        _: impl Fn(Told) + Send + 'static,
    ) -> std::io::Result<(Self, BTreeMap<String, String>)> {
        Err(std::io::Error::other("the Live Visual Tree is not available on Windows yet"))
    }

    /// A request by the protocol's method and its other fields, and its answer; an error says why there is none (the
    /// app not connected, gone, paused, or what it said). Blocks, so ask off the UI thread.
    pub fn ask(&self, method: &str, mut fields: Value) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        fields["id"] = json!(id);
        fields["method"] = json!(method);
        let (send, receive) = channel();
        self.waiting.lock().unwrap().insert(id, send);
        let written = match self.writer.lock().unwrap().as_mut() {
            Some(writer) => {
                writeln!(writer, "{fields}").and_then(|_| writer.flush()).map_err(|e| format!("The app has gone: {e}"))
            }
            None => Err("The app has not shown its UI yet".into()),
        };
        if let Err(why) = written {
            self.waiting.lock().unwrap().remove(&id);
            return Err(why);
        }
        let reply = receive.recv_timeout(ANSWER).map_err(|_| {
            self.waiting.lock().unwrap().remove(&id);
            "The app did not answer: it may be paused".to_string()
        })?;
        match reply["error"].as_str() {
            Some(error) => Err(error.to_string()),
            None => Ok(reply),
        }
    }
}

impl Drop for LiveApp {
    fn drop(&mut self) {
        if let Some(mut writer) = self.writer.lock().unwrap().take() {
            let _ = writer.flush();
        }
        let _ = std::fs::remove_file(&self.socket);
    }
}
