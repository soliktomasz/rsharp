//! The agents' side of a window (Passenger's `AgentSocket` and `PassengerTools`): each window listens on a socket of its
//! own (`rsharp_mcp::AgentSocket`), an agent started with `rsharp --mcp-attach FOLDER` joins the window that has its
//! folder, and each tool call it makes is answered here on the UI thread, as the window's own commands are. An agent is
//! let in only while Settings' Enable MCP Server is on.

use super::*;
use crate::settings::settings;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use futures::{FutureExt, StreamExt};
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

/// The tool name an admission question travels as, which no tool has.
const ADMIT: &str = "\0admit";

impl Ide for Bridge {
    fn admit(&self, agent: &str) -> Result<(), String> {
        self.call(agent, ADMIT, &Value::Null).map(drop)
    }

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
    /// Whether agents were let in without asking at the last change of settings: turning it off ends their connections.
    let_in: bool,
    _settings: Option<gpui::Subscription>,
}

/// What the person may answer when an agent asks to work in the window.
const ANSWERS: [&str; 3] = ["Allow", "Always for this folder", "No"];

pub(super) fn said(text: impl Into<String>) -> Value {
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
        self.agents.let_in = crate::settings::agents_let_in(cx, self.folder());
        self.agents._settings = Some(cx.observe_global::<crate::settings::Settings>(|w, cx| {
            let now = crate::settings::agents_let_in(cx, w.folder());
            if w.agents.let_in
                && !now
                && let Some(socket) = &w.agents.socket
            {
                socket.shut_out();
            }
            w.agents.let_in = now;
        }));
        self.describe_agents();
    }

    /// Lets an agent in, or asks the person: Allow for this connection, Always for this folder (kept in Settings), or No.
    /// Nobody answering within a minute is a no.
    fn agent_admit(&mut self, agent: &str, cx: &mut Context<Self>) -> Task<Result<Value, String>> {
        if crate::settings::agents_let_in(cx, self.folder()) {
            return Task::ready(Ok(said("Let in")));
        }
        if self.choice_dialog.is_some() {
            return Task::ready(Err("The window is asking the person something else: call again in a moment".into()));
        }
        let (sender, answer) = futures::channel::oneshot::channel::<bool>();
        let (handle, folder) = (self.handle, self.folder().map(Path::to_path_buf));
        let message = format!(
            "{agent} wants to work in this window{}.\n\nIt can read and change your files, run and debug your program, and use Git.",
            folder.as_ref().map(|f| format!(" ({})", f.display())).unwrap_or_default()
        );
        let mut sender = Some(sender);
        // Asked from the task, after the update that carried the call has ended: a window is not updated twice at once.
        let ask = move |cx: &mut gpui::AsyncApp| {
            handle.update(cx, |m, window, cx| {
                let before = window.focused(cx);
                let dialog =
                    cx.new(|cx| crate::choice_dialog::ChoiceDialog::new("Agent access", &message, &ANSWERS, 2, cx));
                cx.subscribe_in(&dialog, window, move |m, _, ended: &crate::choice_dialog::Ended, window, cx| {
                    m.choice_dialog = None;
                    if let Some(before) = &before {
                        window.focus(before);
                    }
                    let allowed = match ended {
                        crate::choice_dialog::Ended::Chosen("Allow") => true,
                        crate::choice_dialog::Ended::Chosen("Always for this folder") => {
                            if let Some(folder) = m.folder().map(|f| f.display().to_string()) {
                                let mut settings = settings(cx).clone();
                                if !settings.agent_folders.split(';').any(|f| f.trim() == folder) {
                                    if !settings.agent_folders.trim().is_empty() {
                                        settings.agent_folders.push(';');
                                    }
                                    settings.agent_folders.push_str(&folder);
                                    crate::settings::apply(settings, cx);
                                }
                            }
                            true
                        }
                        _ => false,
                    };
                    if let Some(sender) = sender.take() {
                        let _ = sender.send(allowed);
                    }
                })
                .detach();
                window.focus(&dialog.focus_handle(cx));
                m.choice_dialog = Some(dialog);
                cx.notify();
            })
        };
        let place =
            self.folder().map_or("the RSharp window".to_string(), |f| format!("the RSharp window on {}", f.display()));
        let agent = agent.to_string();
        cx.spawn(async move |this, cx| {
            if ask(cx).is_err() {
                return Err("The window has closed".to_string());
            }
            let timeout = cx.background_executor().timer(Duration::from_secs(60));
            let said_yes = futures::select_biased! {
                answered = answer.fuse() => answered.ok(),
                () = timeout.fuse() => None,
            };
            match said_yes {
                Some(true) => Ok(said("Let in")),
                Some(false) => Err(format!("The person at {place} did not let {agent} work there. Ask them, or do without it; a later call asks them again.")),
                None => {
                    // The question is taken back, so a late answer finds nothing to settle.
                    let _ = this.update(cx, |w, cx| {
                        w.choice_dialog = None;
                        cx.notify();
                    });
                    Err(format!("Nobody at {place} answered within a minute whether {agent} may work there. A later call asks again."))
                }
            }
        })
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
        if tool == ADMIT {
            return self.agent_admit(agent, cx);
        }
        if let Some(reply) = self.agent_editor_tool(tool, arguments, cx) {
            return reply;
        }
        if let Some(reply) = self.agent_language_tool(tool, arguments, cx) {
            return reply;
        }
        if let Some(reply) = self.agent_debug_tool(tool, arguments, cx) {
            return reply;
        }
        if let Some(reply) = self.agent_vcs_tool(tool, arguments, cx) {
            return reply;
        }
        if let Some(reply) = self.agent_apps_tool(tool, arguments, cx) {
            return reply;
        }
        if let Some(reply) = self.agent_workflow_tool(tool, arguments, cx) {
            return reply;
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
