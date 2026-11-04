//! The agents' side of a window (Passenger's `AgentSocket` and `PassengerTools`): each window listens on a socket of its
//! own (`rsharp_mcp::AgentSocket`), an agent started with `rsharp --mcp-attach FOLDER` joins the window that has its
//! folder, and each tool call it makes is answered here on the UI thread, as the window's own commands are. An agent is
//! let in only while Settings' Enable MCP Server is on.

use super::*;
use crate::settings::settings;
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use rsharp_mcp::{AgentSocket, Ide};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static DISABLED: AtomicBool = AtomicBool::new(false);

/// `--no-mcp`: no window of this process listens for agents.
pub fn refuse(no_mcp: bool) {
    DISABLED.store(no_mcp, Ordering::Release);
}

struct Call {
    agent: String,
    tool: String,
    arguments: Value,
    reply: std::sync::mpsc::Sender<Result<Value, String>>,
}

/// The socket's side of the window: a call crosses to the UI thread and its answer comes back.
struct Bridge(UnboundedSender<Call>);

impl Ide for Bridge {
    fn call(&self, agent: &str, tool: &str, arguments: &Value) -> Result<Value, String> {
        let (reply, answer) = std::sync::mpsc::channel();
        let call = Call { agent: agent.into(), tool: tool.into(), arguments: arguments.clone(), reply };
        self.0.unbounded_send(call).map_err(|_| "The RSharp window has closed")?;
        answer.recv_timeout(Duration::from_secs(600)).map_err(|_| "The RSharp window did not answer in time")?
    }
}

#[derive(Default)]
pub(super) struct Agents {
    socket: Option<AgentSocket>,
    _serving: Option<Task<()>>,
}

fn said(text: impl Into<String>) -> Value {
    json!([{ "type": "text", "text": text.into() }])
}

impl MainWindow {
    /// Listens for agents; a window that cannot says so in the log and goes on without.
    pub fn open_agent_socket(&mut self, cx: &mut Context<Self>) {
        if DISABLED.load(Ordering::Acquire) || self.agents.socket.is_some() {
            return;
        }
        let (sender, mut calls) = unbounded();
        match AgentSocket::open(Arc::new(Bridge(sender))) {
            Ok(socket) => self.agents.socket = Some(socket),
            Err(error) => return log::warn!("Agents cannot reach this window: {error}"),
        }
        self.agents._serving = Some(cx.spawn(async move |this, cx| {
            while let Some(call) = calls.next().await {
                let Ok(answer) = this.update(cx, |w, cx| w.agent_tool(&call.agent, &call.tool, &call.arguments, cx))
                else {
                    break;
                };
                // Each call is its own task, so a build an agent waits on does not hold the next call.
                cx.spawn(async move |_| drop(call.reply.send(answer.await))).detach();
            }
        }));
        self.describe_agents();
    }

    pub(super) fn describe_agents(&self) {
        if let Some(socket) = &self.agents.socket {
            socket.describe(self.folder());
        }
    }

    /// The socket of this window, for a test's client.
    pub fn agent_socket(&self) -> Option<PathBuf> {
        self.agents.socket.as_ref().map(|s| s.path().to_path_buf())
    }

    fn agent_tool(
        &mut self,
        agent: &str,
        tool: &str,
        arguments: &Value,
        cx: &mut Context<Self>,
    ) -> Task<Result<Value, String>> {
        if !settings(cx).agents_enabled {
            return Task::ready(Err(format!(
                "{agent} may not work in this RSharp window: the person turns on Settings > MCP Server > Enable MCP Server first."
            )));
        }
        let place = |file: &str| {
            let path = PathBuf::from(file);
            match (&self.workspace, path.is_absolute()) {
                (Some(workspace), false) => workspace.folder.join(path),
                _ => path,
            }
        };
        match tool {
            "workspace" => {
                let open: Vec<String> =
                    self.workspace_buffers(cx).iter().map(|b| b.read(cx).path.display().to_string()).collect();
                let described = json!({
                    "folder": self.folder(),
                    "opened": self.opened_as(),
                    "openFiles": open,
                });
                Task::ready(Ok(said(serde_json::to_string_pretty(&described).unwrap_or_default())))
            }
            "open_file" => {
                let Some(file) = arguments["file"].as_str() else {
                    return Task::ready(Err("open_file needs a file".into()));
                };
                let path = place(file);
                if !path.is_file() {
                    return Task::ready(Err(format!("{} is not a file", path.display())));
                }
                let caret = arguments["line"].as_u64().map(|line| {
                    (
                        line.saturating_sub(1) as usize,
                        arguments["column"].as_u64().unwrap_or(1).saturating_sub(1) as usize,
                    )
                });
                let handle = self.handle;
                cx.spawn(async move |_, cx| {
                    handle
                        .update(cx, |m, window, cx| m.open_file(&path, caret, window, cx))
                        .map(|()| said(format!("Opened {}", path.display())))
                        .map_err(|e| e.to_string())
                })
            }
            "save_all" => Task::ready(self.save_all(cx).map(|()| said("Saved"))),
            other => Task::ready(Err(format!("This RSharp window has no tool {other}"))),
        }
    }
}
