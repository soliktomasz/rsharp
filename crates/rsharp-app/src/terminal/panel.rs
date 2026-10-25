use super::TerminalView;
use crate::{assets::icon, theme};
use gpui::{App, Context, Entity, EventEmitter, FocusHandle, Focusable, Task, Window, div, prelude::*, px};
use rsharp_terminal::{CommandBuilder, Session};
use std::{path::PathBuf, sync::Arc, time::Duration};

pub enum TerminalEvent {
    Empty,
    Link(rsharp_core::run::link::Link),
}
impl EventEmitter<TerminalEvent> for TerminalPanel {}

struct Tab {
    id: usize,
    name: String,
    session: Option<Arc<Session>>,
    folder: PathBuf,
    retained: bool,
    problem: Option<String>,
}

pub struct TerminalPanel {
    view: Entity<TerminalView>,
    tabs: Vec<Tab>,
    active: usize,
    next: usize,
    opened: usize,
    folder: PathBuf,
    history: Option<PathBuf>,
    _poll: Option<Task<()>>,
    /// Which poller `_poll` holds: a poller that outlived its panel's tabs must not clear the one after it.
    poll_gen: u64,
    starters: Vec<Task<()>>,
    focus_requested: bool,
}
impl TerminalPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let view = cx.new(TerminalView::new);
        cx.subscribe(&view, |_, _, link: &super::view::FollowLink, cx| cx.emit(TerminalEvent::Link(link.0.clone())))
            .detach();
        Self {
            view,
            tabs: vec![],
            active: 0,
            next: 0,
            opened: 0,
            folder: rsharp_core::programs::home(),
            history: None,
            _poll: None,
            poll_gen: 0,
            starters: vec![],
            focus_requested: false,
        }
    }
    pub fn tabs_len(&self) -> usize {
        self.tabs.len()
    }
    pub fn view(&self) -> &Entity<TerminalView> {
        &self.view
    }
    pub fn active_session(&self) -> Option<Arc<Session>> {
        self.tabs.get(self.active)?.session.clone()
    }
    pub fn configure(&mut self, folder: PathBuf, history: Option<PathBuf>) {
        self.folder = folder;
        self.history = history;
    }
    pub fn ensure_shell(&mut self, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            self.new_shell(None, cx);
        }
    }
    pub fn new_shell(&mut self, folder: Option<PathBuf>, cx: &mut Context<Self>) {
        let folder = folder.unwrap_or_else(|| self.folder.clone());
        self.opened += 1;
        let name = if self.opened == 1 { "Local".into() } else { format!("Local ({})", self.opened) };
        if let Some(history) = &self.history {
            let _ = std::fs::create_dir_all(history.parent().unwrap());
        }
        let command = rsharp_terminal::shell(&folder, self.history.as_deref());
        self.start(name, command, folder, false, cx);
    }
    pub fn start(
        &mut self,
        name: String,
        command: CommandBuilder,
        folder: PathBuf,
        retained: bool,
        cx: &mut Context<Self>,
    ) -> usize {
        self.tabs.retain(|tab| {
            !(retained && tab.name == name && tab.session.as_ref().is_some_and(|s| s.exit_code().is_some()))
        });
        let id = self.next;
        self.next += 1;
        self.tabs.push(Tab { id, name, session: None, folder, retained, problem: None });
        self.active = self.tabs.len() - 1;
        self.focus_requested = true;
        self.show(cx);
        let starting = cx.background_executor().spawn(async move { Session::start(command, 80, 24).map(Arc::new) });
        self.starters.push(cx.spawn(async move |this, cx| {
            let result = starting.await;
            let _ = this.update(cx, |panel, cx| {
                if let Some(tab) = panel.tabs.iter_mut().find(|t| t.id == id) {
                    match result {
                        Ok(session) => tab.session = Some(session),
                        Err(error) => tab.problem = Some(error),
                    }
                    panel.show(cx);
                }
            });
        }));
        if self._poll.is_none() {
            self.poll_gen += 1;
            let generation = self.poll_gen;
            self._poll = Some(cx.spawn(async move |this, cx| {
                let mut last = None;
                loop {
                    cx.background_executor().timer(Duration::from_millis(16)).await;
                    let keep = this
                        .update(cx, |p, cx| {
                            let ended: Vec<_> = p
                                .tabs
                                .iter()
                                .filter(|t| !t.retained && t.session.as_ref().is_some_and(|s| s.exit_code().is_some()))
                                .map(|t| t.id)
                                .collect();
                            for id in ended {
                                p.close(id, cx);
                            }
                            let revision = p.active_session().map(|s| (s.pid, s.revision()));
                            if revision != last {
                                last = revision;
                                p.view.update(cx, |_, cx| cx.notify());
                                cx.notify();
                            }
                            !p.tabs.is_empty()
                        })
                        .unwrap_or(false);
                    if !keep {
                        break;
                    }
                }
                let _ = this.update(cx, |p, _| {
                    if p.poll_gen == generation {
                        p._poll = None;
                    }
                });
            }));
        }
        cx.notify();
        id
    }
    fn show(&mut self, cx: &mut Context<Self>) {
        let session = self.active_session();
        let folder = self.tabs.get(self.active).map(|t| t.folder.clone()).unwrap_or_else(|| self.folder.clone());
        let program = self.retained_active();
        self.view.update(cx, |view, cx| view.set_session(session, folder, program, cx));
        cx.notify();
    }
    pub fn close_all(&mut self, cx: &mut Context<Self>) {
        self.starters.clear();
        self.tabs.clear();
        // A poller still running is no longer this panel's: the next session's gets a generation of its own.
        self._poll = None;
        self.poll_gen += 1;
        self.show(cx);
    }
    pub fn close(&mut self, id: usize, cx: &mut Context<Self>) {
        if let Some(at) = self.tabs.iter().position(|t| t.id == id) {
            self.tabs.remove(at);
            if at < self.active {
                self.active -= 1;
            }
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
            self.show(cx);
            if self.tabs.is_empty() {
                cx.emit(TerminalEvent::Empty);
            }
        }
    }
    pub fn session(&self, id: usize) -> Option<Arc<Session>> {
        self.tabs.iter().find(|t| t.id == id)?.session.clone()
    }
    pub fn problem(&self, id: usize) -> Option<&str> {
        self.tabs.iter().find(|t| t.id == id)?.problem.as_deref()
    }
    pub fn retained_active(&self) -> bool {
        self.tabs.get(self.active).is_some_and(|t| t.retained)
    }
}
impl Focusable for TerminalPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.view.focus_handle(cx)
    }
}
impl Render for TerminalPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_requested) {
            window.focus(&self.view.focus_handle(cx));
        }
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(at, t)| {
                let (id, name) = (t.id, t.name.clone());
                div()
                    .id(("terminal-tab", id))
                    .debug_selector(move || format!("terminal tab: {name}"))
                    .h(px(28.))
                    .px(px(8.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .bg(if at == self.active { theme::tab_selected() } else { theme::island() })
                    .hover(|s| s.bg(theme::button_hover()))
                    .child(t.name.clone())
                    .when_some(t.session.as_ref().and_then(|s| s.exit_code()), |d, code| d.child(format!("[{code}]")))
                    .on_click(cx.listener(move |p, _, window, cx| {
                        p.active = at;
                        p.show(cx);
                        window.focus(&p.view.focus_handle(cx));
                    }))
                    .child(
                        div()
                            .id(("close-terminal", id))
                            .size(px(16.))
                            .rounded(px(3.))
                            .hover(|d| d.bg(theme::button_hover()))
                            .child(icon("close", 12.))
                            .on_click(cx.listener(move |p, _, _, cx| {
                                cx.stop_propagation();
                                p.close(id, cx);
                            })),
                    )
            })
            .collect();
        let problem = self.tabs.get(self.active).and_then(|t| t.problem.clone());
        div()
            .debug_selector(|| "Terminal panel".into())
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(36.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(px(6.))
                    .gap(px(2.))
                    .overflow_x_hidden()
                    .children(tabs)
                    .child(
                        div()
                            .id("terminal-new")
                            .debug_selector(|| "terminal: New Session".into())
                            .size(px(28.))
                            .rounded(px(4.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(theme::button_hover()))
                            .child(icon("add", 16.))
                            .tooltip(|_, cx| cx.new(|_| crate::window::Tip("New Session".into())).into())
                            .on_click(cx.listener(|p, _, window, cx| {
                                p.new_shell(None, cx);
                                window.focus(&p.view.focus_handle(cx));
                            })),
                    ),
            )
            .when_some(problem, |d, problem| d.child(div().p(px(12.)).text_color(theme::error()).child(problem)))
            .child(self.view.clone())
    }
}
