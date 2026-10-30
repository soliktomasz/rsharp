//! The window's side of the web languages: one server per kind (TypeScript and JavaScript, HTML, CSS, JSON) for the
//! folder, started on Node.js with the first file of its kind, every open file of its languages kept in step with it,
//! and its pushed problems marked in the file they name. Without Node.js or the servers a file keeps its colours, and
//! the status bar says why.

use super::*;
use rsharp_lsp::web::{WebServer, start_web, web_language};
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
}

impl MainWindow {
    /// Gives a web file's buffer its server, which starts with the first file of its kind.
    pub(super) fn attach_web(&mut self, buffer: &Entity<Buffer>, cx: &mut Context<Self>) {
        let Some((kind, language)) = web_language(&buffer.read(cx).path) else { return };
        if !cx.global::<crate::state::Ide>().servers {
            return;
        }
        match self.web.slots.get(&kind) {
            Some(WebSlot::Running(server)) => {
                let server = server.clone();
                buffer.update(cx, |b, cx| b.attach_server_as(server, language, cx));
            }
            Some(_) => {}
            None => self.start_web(kind, cx),
        }
    }

    fn start_web(&mut self, kind: WebServer, cx: &mut Context<Self>) {
        let Some(folder) = self.workspace.as_ref().map(|w| w.folder.clone()) else { return };
        let Some(node) = rsharp_core::programs::node() else {
            return self.web_unavailable(kind, "Node.js was not found".into(), cx);
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
                for buffer in w.workspace_buffers(cx) {
                    if web_language(&buffer.read(cx).path).is_some_and(|(k, _)| k == kind) {
                        w.attach_web(&buffer, cx);
                    }
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

    /// A server that cannot run: its files keep their colours, and the status bar says why.
    fn web_unavailable(&mut self, kind: WebServer, why: String, cx: &mut Context<Self>) {
        log::warn!(target: "web", "the {} language server cannot run: {why}", kind.name());
        self.web.slots.insert(kind, WebSlot::Unavailable);
        self.said = Some(format!("{} has its colours only: {why}", kind.name()).into());
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
                    self.web_unavailable(kind, format!("the {} language server ended", kind.name()), cx);
                }
            }
        }
    }
}
