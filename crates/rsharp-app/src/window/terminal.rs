//! The Terminal window's place in the window: the toggle Rider's key and the View menu use, Open In > Terminal
//! from the Solution view, the links the terminal's output carries, and an agent's explicitly supplied command.

use super::*;

impl MainWindow {
    /// A link clicked in a terminal opens where the Run console's do: a URL in the browser, a place in the editor.
    pub(super) fn follow_terminal_link(
        &mut self,
        link: rsharp_core::run::link::Link,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = match link {
            rsharp_core::run::link::Link::Url { url, .. } => editor::ConsoleTarget::Url(url),
            rsharp_core::run::link::Link::File { path, line, .. } => {
                editor::ConsoleTarget::File { path, line, column: 0 }
            }
        };
        self.follow_console_link(target, window, cx);
    }
    /// The one Terminal window of this window, as the tests read it.
    pub fn terminal(&self) -> &Entity<crate::terminal::TerminalPanel> {
        &self.terminal
    }

    /// Where a new shell starts, and the history file it keeps: a test's and a screenshot's own never touch the
    /// person's.
    fn configure_terminal(&self, cx: &mut Context<Self>) {
        let folder = self.workspace.as_ref().map(|w| w.folder.clone()).unwrap_or_else(rsharp_core::programs::home);
        let isolated = !cx.global::<crate::state::Ide>().servers || std::env::var_os("RSHARP_SCREENSHOT").is_some();
        let history = isolated.then(|| store(cx).root().join("terminal/history"));
        self.terminal.update(cx, |p, _| {
            p.configure(folder, history);
        });
    }
    /// Starts the first shell if the window has none: the Terminal is opened lazily, so nothing runs until it is.
    pub(super) fn ensure_terminal(&self, cx: &mut Context<Self>) {
        self.configure_terminal(cx);
        self.terminal.update(cx, |p, cx| p.ensure_shell(cx));
    }
    /// Rider's `alt-f12`: opens the window, takes the keys when it is open but not focused, and hides it otherwise.
    pub(super) fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dock.is_open(Tool::Terminal) && !self.terminal.focus_handle(cx).contains_focused(window, cx) {
            window.focus(&self.terminal.focus_handle(cx));
        } else {
            self.toggle_tool(Tool::Terminal, window, cx);
        }
    }
    /// A new shell at `folder`: the Solution view's Open In > Terminal, at a folder's or a file's parent.
    pub fn open_terminal_at(&mut self, folder: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.configure_terminal(cx);
        self.terminal.update(cx, |p, cx| p.new_shell(Some(folder), cx));
        self.dock.show(Tool::Terminal);
        window.focus(&self.terminal.focus_handle(cx));
        self.save_state(cx);
        cx.notify();
    }
    /// An explicitly supplied command — `--agent` — in an Agent tab of its own, which keeps its output and status.
    /// `arguments` names a program the launch step resolved; nothing is assumed or launched by opening Terminal.
    pub fn run_agent(&mut self, arguments: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> usize {
        let folder = self.workspace.as_ref().map(|w| w.folder.clone()).unwrap_or_else(rsharp_core::programs::home);
        let mut command = rsharp_terminal::CommandBuilder::new(&arguments[0]);
        command.args(&arguments[1..]);
        command.cwd(&folder);
        let id = self.terminal.update(cx, |p, cx| p.start("Agent".into(), command, folder, true, cx));
        self.dock.show(Tool::Terminal);
        window.focus(&self.terminal.focus_handle(cx));
        cx.notify();
        id
    }
}
