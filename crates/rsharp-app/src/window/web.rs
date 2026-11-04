//! The window's side of the web languages: one server per kind (TypeScript and JavaScript, HTML, CSS, JSON) for the
//! folder, started on Node.js with the first file of its kind, every open file of its languages kept in step with it,
//! and its pushed problems marked in the file they name; ESLint's server beside them for a file the project's own
//! eslint checks. Without Node.js or the servers a file keeps its colours, and the status bar says why.

use super::*;
use rsharp_lsp::web::{WebServer, eslint_covers, start_web, web_language};
use std::collections::HashMap;

/// A web server of the folder: being started, running, or unable to run.
pub(super) enum WebSlot {
    Starting,
    Running(Arc<LanguageServer>),
    Unavailable,
}

/// The folder's web servers, and the task that starts each and then listens to it, which ends with this.
#[derive(Default)]
pub(super) struct WebServers {
    slots: HashMap<WebServer, WebSlot>,
    listening: HashMap<WebServer, Task<()>>,
    /// How many times each ended server was started again, and when it last started: one that lived a minute has paid
    /// for its restarts.
    restarts: HashMap<WebServer, (u32, std::time::Instant)>,
}

/// A server that keeps ending is left alone after this many starts in quick succession.
const RESTARTS: u32 = 3;

impl MainWindow {
    /// Gives a web file's buffer its server, and ESLint's where the project's eslint checks it; each starts with the
    /// first file that wants it.
    pub(super) fn attach_web(&mut self, buffer: &Entity<Buffer>, cx: &mut Context<Self>) {
        let path = buffer.read(cx).path.clone();
        // A Razor file is Roslyn's, but its HTML is the HTML server's, which Razor asks through the client.
        if rsharp_lsp::razor::is_razor(&path) && cx.global::<crate::state::Ide>().servers {
            if !self.web.slots.contains_key(&WebServer::Html) {
                self.start_web(WebServer::Html, cx);
            }
            return;
        }
        let Some((kind, language)) = web_language(&path) else { return };
        if !cx.global::<crate::state::Ide>().servers {
            return;
        }
        let lint = eslint_covers(&path) && crate::settings::package(cx, &path, "eslint").is_some();
        for wanted in [kind].into_iter().chain(lint.then_some(WebServer::Eslint)) {
            match self.web.slots.get(&wanted) {
                Some(WebSlot::Running(server)) => {
                    let server = server.clone();
                    buffer.update(cx, |b, cx| match wanted {
                        WebServer::Eslint => b.attach_lint(server, language, cx),
                        _ => b.attach_server_as(server, language, cx),
                    });
                }
                Some(_) => {}
                None => self.start_web(wanted, cx),
            }
        }
    }

    fn start_web(&mut self, kind: WebServer, cx: &mut Context<Self>) {
        let Some(folder) = self.workspace.as_ref().map(|w| w.folder.clone()) else { return };
        let node = match crate::settings::node(cx) {
            Ok(node) => node,
            Err(why) => return self.web_unavailable(kind, why, cx),
        };
        let (sender, mut events) = futures::channel::mpsc::unbounded();
        let task = cx.spawn(async move |this, cx| {
            let started = cx
                .background_executor()
                .spawn(async move { start_web(kind, &folder, &node, move |e| drop(sender.unbounded_send(e))) })
                .await;
            let server = match started {
                Ok(server) => Arc::new(server),
                Err(failed) => {
                    let _ = this.update(cx, |w, cx| w.web_unavailable(kind, failed.0, cx));
                    return;
                }
            };
            let alive = this.update(cx, |w, cx| {
                log::info!(target: "web", "the {} language server started", kind.name());
                w.web.slots.insert(kind, WebSlot::Running(server.clone()));
                w.connect_razor_html();
                // Attaching is once per server, so every web file can be given whatever it wants now running.
                for buffer in w.workspace_buffers(cx) {
                    w.attach_web(&buffer, cx);
                }
            });
            if alive.is_err() {
                return;
            }
            while let Some(event) = events.next().await {
                if this.update(cx, |w, cx| w.on_web_event(kind, &server, event, cx)).is_err() {
                    break;
                }
            }
        });
        self.web.slots.insert(kind, WebSlot::Starting);
        self.web.listening.insert(kind, task);
    }

    /// Razor's questions about its files' HTML go to the folder's HTML server, once both it and Roslyn run.
    pub(super) fn connect_razor_html(&self) {
        if let (Some(roslyn), Some(WebSlot::Running(html))) = (&self.server, self.web.slots.get(&WebServer::Html)) {
            roslyn.razor.set_html(Some(html.clone()));
        }
    }

    /// A server that ended is started again, its files given the new one, up to [`RESTARTS`] times in a row; then its files
    /// keep their colours and the status bar says it ended.
    fn restart_web(&mut self, kind: WebServer, ended: &Arc<LanguageServer>, cx: &mut Context<Self>) {
        let (count, since) = self.web.restarts.get(&kind).copied().unwrap_or((0, std::time::Instant::now()));
        let count = if since.elapsed() > std::time::Duration::from_secs(60) { 0 } else { count };
        if count >= RESTARTS {
            return self.web_unavailable(kind, format!("the {} language server ended", kind.name()), cx);
        }
        log::warn!(target: "web", "the {} language server ended; starting it again", kind.name());
        self.web.restarts.insert(kind, (count + 1, std::time::Instant::now()));
        for buffer in self.workspace_buffers(cx) {
            buffer.update(cx, |b, cx| b.drop_ended_server(ended, cx));
        }
        self.web.slots.remove(&kind);
        self.start_web(kind, cx);
    }

    /// A server that cannot run: its files keep their colours, and the status bar says why.
    fn web_unavailable(&mut self, kind: WebServer, why: String, cx: &mut Context<Self>) {
        log::warn!(target: "web", "the {} language server cannot run: {why}", kind.name());
        self.web.slots.insert(kind, WebSlot::Unavailable);
        self.said = Some(match kind {
            WebServer::Eslint => format!("ESLint cannot run: {why}").into(),
            _ => format!("{} has its colours only: {why}", kind.name()).into(),
        });
        cx.notify();
    }

    fn on_web_event(
        &mut self,
        kind: WebServer,
        server: &Arc<LanguageServer>,
        event: ServerEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            ServerEvent::Notification { method, params } if method == "textDocument/publishDiagnostics" => {
                let Some(path) = params["uri"].as_str().and_then(rsharp_core::paths::from_uri) else { return };
                if let Some(buffer) = crate::buffer::find(&path, cx)
                    && buffer.read(cx).server().is_some_and(|(s, _)| Arc::ptr_eq(&s, server))
                {
                    buffer.update(cx, |b, cx| b.show_problems(&params["diagnostics"], cx));
                }
            }
            ServerEvent::Notification { method, params } if method == "workspace/applyEdit" => {
                let edit = rsharp_lsp::workspace_edit(&params["edit"]);
                if let Err(why) = crate::language::apply_workspace_edit(&edit, None, cx) {
                    self.said = Some(format!("{}'s edit could not be made: {why}", kind.name()).into());
                    cx.notify();
                }
            }
            ServerEvent::Notification { method, params } if method == "window/logMessage" => {
                if params["type"].as_u64().is_some_and(|t| t <= 2) {
                    log::warn!(target: "web", "{}: {}", kind.name(), params["message"].as_str().unwrap_or_default());
                }
            }
            ServerEvent::Notification { .. } => {}
            ServerEvent::Stderr(line) => log::warn!(target: "web", "{}: {line}", kind.name()),
            ServerEvent::Ended => {
                if self.web.slots.get(&kind).is_some_and(|s| matches!(s, WebSlot::Running(r) if Arc::ptr_eq(r, server)))
                {
                    self.restart_web(kind, server, cx);
                }
            }
        }
    }
}
