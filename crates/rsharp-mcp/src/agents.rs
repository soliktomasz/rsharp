//! The agents' way into a window: each window listens on a socket of its own in the user's own folder
//! ([`AgentSocket`]), and `rsharp --mcp-attach [FOLDER]` ([`attach`]) is the agent's MCP server for its session, which
//! joins the window whose folder holds FOLDER at its first tool call and passes each call to it. The socket speaks JSON
//! lines: `{id, agent, tool, arguments}` answered `{id, content}` or `{id, error}`; what the window does with a call is
//! the app's ([`Ide`]), run on its UI thread.

use serde_json::{Value, json};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};

/// What a window does with an agent's tool call: its content (MCP's content array) or why not. Called on a thread of the
/// connection's own, so it may wait for the UI thread.
pub trait Ide: Send + Sync + 'static {
    fn call(&self, agent: &str, tool: &str, arguments: &Value) -> Result<Value, String>;
}

/// Where windows listen: a folder of the user's own in the temporary folder, kept short because macOS allows a socket's
/// path 104 bytes. `RSHARP_AGENTS` names another, for a test's own.
pub fn socket_folder() -> PathBuf {
    std::env::var_os("RSHARP_AGENTS").map(PathBuf::from).unwrap_or_else(|| {
        let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
        std::env::temp_dir().join(format!("rsharp-agents-{}", user.chars().take(16).collect::<String>()))
    })
}

fn own_folder() -> io::Result<PathBuf> {
    let folder = socket_folder();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&folder)?;
        // Never listen or connect in a link, or in a folder anyone else may write to.
        let meta = std::fs::symlink_metadata(&folder)?;
        let open = std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o077 != 0;
        if !meta.is_dir() || open {
            return Err(io::Error::other(format!("{} is not a folder of the user's own", folder.display())));
        }
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(&folder)?;
    Ok(folder)
}

static OPENED: AtomicUsize = AtomicUsize::new(0);

/// A window's socket, open until it is dropped, which ends every connection.
pub struct AgentSocket {
    path: PathBuf,
    closed: Arc<AtomicBool>,
}

impl AgentSocket {
    pub fn open(ide: Arc<dyn Ide>) -> io::Result<Self> {
        let folder = own_folder()?;
        let path = folder.join(format!("{}-{}.sock", std::process::id(), OPENED.fetch_add(1, Ordering::Relaxed) + 1));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        let closed = Arc::new(AtomicBool::new(false));
        let stopping = closed.clone();
        std::thread::Builder::new().name("agent socket".into()).spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (ide, stopping) = (ide.clone(), stopping.clone());
                        std::thread::spawn(move || serve(stream, ide.as_ref(), &stopping));
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
                    Err(_) => break,
                }
            }
        })?;
        Ok(Self { path, closed })
    }

    /// Writes the window's folder beside the socket, as it opens and changes, for `--mcp-attach` to choose by.
    pub fn describe(&self, folder: Option<&Path>) {
        let text = folder.map(|f| f.display().to_string()).unwrap_or_default();
        let _ = std::fs::write(self.path.with_extension("folder"), text);
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for AgentSocket {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(self.path.with_extension("folder"));
    }
}

fn serve(stream: UnixStream, ide: &dyn Ide, closed: &AtomicBool) {
    let _ = stream.set_nonblocking(false);
    let Ok(mut writer) = stream.try_clone() else { return };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        if closed.load(Ordering::Acquire) {
            break;
        }
        let Ok(request) = serde_json::from_str::<Value>(&line) else { continue };
        let reply = match ide.call(
            request["agent"].as_str().unwrap_or("An agent"),
            request["tool"].as_str().unwrap_or_default(),
            &request["arguments"],
        ) {
            Ok(content) => json!({ "id": request["id"], "content": content }),
            Err(why) => json!({ "id": request["id"], "error": why }),
        };
        if writeln!(writer, "{reply}").and_then(|()| writer.flush()).is_err() {
            break;
        }
    }
}

/// The window listening whose folder holds `folder`: its socket, dead ones taken away as they are found.
fn window_for(folder: &Path) -> Option<UnixStream> {
    let wanted = rsharp_core::paths::canonical(folder);
    let mut found = None;
    for entry in std::fs::read_dir(own_folder().ok()?).ok()?.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "sock") {
            continue;
        }
        let Ok(stream) = UnixStream::connect(&path) else {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(path.with_extension("folder"));
            continue;
        };
        let theirs = std::fs::read_to_string(path.with_extension("folder")).unwrap_or_default();
        if !theirs.is_empty()
            && wanted.starts_with(rsharp_core::paths::canonical(Path::new(&theirs)))
            && found.is_none()
        {
            found = Some(stream);
        }
    }
    found
}

/// `--mcp-attach [FOLDER]`: MCP on `input` and `output` for the agent's whole session. `initialize` and `tools/list` are
/// answered here; a tool call goes to the window that has the folder, found at the call (the person may open it after the
/// agent starts), and says so when there is none.
pub fn attach(folder: &Path, input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut window: Option<(UnixStream, BufReader<UnixStream>)> = None;
    let mut agent = String::from("An agent");
    let mut next = 0;
    for line in input.lines() {
        let line = line?;
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = message.get("id").cloned() else { continue };
        let result: Result<Value, Value> = match message["method"].as_str().unwrap_or_default() {
            "initialize" => {
                let client = &message["params"]["clientInfo"];
                if let Some(name) = client["title"].as_str().or(client["name"].as_str()).filter(|n| !n.is_empty()) {
                    agent = name.to_string();
                }
                Ok(crate::initialize(&message["params"]))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": crate::all_tools() })),
            "tools/call" => {
                if window.is_none() {
                    window = window_for(folder).and_then(|s| Some((s.try_clone().ok()?, BufReader::new(s))));
                }
                let params = &message["params"];
                Ok(match window.as_mut() {
                    None => crate::failed(&format!(
                        "No RSharp window has {} open: open it in RSharp, or start an agent with `rsharp --mcp {}`.",
                        folder.display(),
                        folder.display()
                    )),
                    Some((stream, replies)) => {
                        next += 1;
                        let request = json!({ "id": next, "agent": agent, "tool": params["name"], "arguments": params["arguments"] });
                        let mut answer = String::new();
                        let asked = writeln!(stream, "{request}")
                            .and_then(|()| stream.flush())
                            .and_then(|()| replies.read_line(&mut answer));
                        match asked {
                            Ok(n) if n > 0 => {
                                let reply: Value = serde_json::from_str(&answer).unwrap_or_default();
                                match reply["error"].as_str() {
                                    Some(why) => crate::failed(why),
                                    None => json!({ "content": reply["content"], "isError": false }),
                                }
                            }
                            _ => {
                                window = None;
                                crate::failed("The RSharp window went away: call again once it is open")
                            }
                        }
                    }
                })
            }
            method => Err(json!({ "code": -32601, "message": format!("no method {method}") })),
        };
        let reply = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        };
        writeln!(output, "{reply}")?;
        output.flush()?;
    }
    Ok(())
}
